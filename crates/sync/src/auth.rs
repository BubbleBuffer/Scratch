//! Pairing tokens: `.vault/hub/tokens.json` in the hub's vault. Tokens are
//! stored as SHA-256 hashes. A token is bound to the device id of the
//! first vault that uses it, so one token serves one device, and a device
//! id seen under two tokens (a copied vault) is refused.
//!
//! The file is re-read when its size, its mtime or (on Unix) its permission
//! bits change, and every check and mutation looks at the file first.
//! Several [`Tokens`] on one vault (the running hub and a `vault-hub --pair`
//! run, say) therefore see each
//! other's writes. Mutations, including binding a token on its first use,
//! hold an exclusive lock on the stable file `tokens.lock` beside the JSON,
//! across their reload and save, and they reload before they change
//! anything: a stale copy never writes back a removed entry. A save that
//! fails changes nothing, because the cache is updated only after the file
//! has been replaced.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vault_core::sha256_hex;

use crate::{ct_eq, now_ms, random_hex, Result, SyncError};

const FILE: &str = "tokens.json";
/// Never renamed or removed: the lock lives on this inode.
const LOCK_FILE: &str = "tokens.lock";

/// A paired device, as `Hub::devices` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub label: String,
    /// The device id of the vault that first used the token (none yet if
    /// it has not synced).
    pub device: Option<String>,
    pub created_ms: i64,
    /// The device has connected to `/host/ws` ("Let the hub use this
    /// computer"); it stays a host, online or not, until it signs out.
    #[serde(default)]
    pub host: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    hash: String,
    label: String,
    device: Option<String>,
    created_ms: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    host: bool,
    /// The user named the device ([`Tokens::rename`]); a re-login keeps
    /// the name.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    named: bool,
}

/// The longest device name, in characters (as `/auth/start` allows).
pub const MAX_NAME_CHARS: usize = 64;

/// The entry holding `token`, comparing hashes in constant time and
/// looking at every entry.
fn find(entries: &[Entry], token: &str) -> Option<usize> {
    let hash = sha256_hex(token.as_bytes());
    let mut found = None;
    for (i, x) in entries.iter().enumerate() {
        if ct_eq(x.hash.as_bytes(), hash.as_bytes()) {
            found = Some(i);
        }
    }
    found
}

/// What a check decides from the entries it sees.
enum Verdict {
    /// The token is bound to this device.
    Allowed,
    /// The token is not bound yet: bind the entry at this index.
    Bind(usize),
}

/// The check rules, applied to a file's entries.
fn judge(entries: &[Entry], token: &str, device: &str) -> Result<Verdict> {
    let Some(i) = find(entries, token) else {
        return Err(SyncError::Status(401, "unknown token".into()));
    };
    match &entries[i].device {
        Some(d) if d == device => Ok(Verdict::Allowed),
        Some(_) => Err(SyncError::Status(
            403,
            "this token belongs to another device".into(),
        )),
        None => {
            if let Some(other) = entries.iter().find(|x| x.device.as_deref() == Some(device)) {
                return Err(SyncError::Status(
                    409,
                    format!(
                        "device {device} is already paired as {:?}; unpair it first, or \
                         delete .vault/device if this vault is a copy",
                        other.label
                    ),
                ));
            }
            Ok(Verdict::Bind(i))
        }
    }
}

/// A file's identity for change detection: mtime and size, and on Unix the
/// mode. chmod changes neither mtime nor size, so the mode is what notices
/// an unreadable file while its last good contents are still cached.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    #[cfg(unix)]
    mode: u32,
}

#[derive(Default)]
struct Cache {
    /// False until a read succeeds, and after this store writes the file.
    loaded: bool,
    /// The file's stamp taken before the last read.
    stamp: Option<Stamp>,
    entries: Vec<Entry>,
}

pub(crate) struct Tokens {
    path: PathBuf,
    lock_path: PathBuf,
    cache: Mutex<Cache>,
}

impl Tokens {
    pub fn open(vault_root: &str) -> Result<Tokens> {
        let dir = PathBuf::from(vault_root).join(".vault").join("hub");
        fs::create_dir_all(&dir)?;
        let store = Tokens {
            path: dir.join(FILE),
            lock_path: dir.join(LOCK_FILE),
            cache: Mutex::new(Cache::default()),
        };
        {
            let mut cache = store.lock();
            store.refresh(&mut cache)?;
        }
        Ok(store)
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The cross-process mutation lock. Each call opens a fresh handle; the
    /// lock is held while the returned file lives. Taken before the cache
    /// mutex, and never by a check that does not write, so they cannot wait
    /// on each other.
    fn lock_file(&self) -> Result<fs::File> {
        let lock_err = |e: io::Error| SyncError::Io(format!("{}: {e}", self.lock_path.display()));
        if let Some(dir) = self.lock_path.parent() {
            fs::create_dir_all(dir).map_err(lock_err)?;
        }
        let file = open_lock_file(&self.lock_path).map_err(lock_err)?;
        file.lock().map_err(lock_err)?;
        Ok(file)
    }

    fn io_err(&self, e: io::Error) -> SyncError {
        SyncError::Io(format!("{}: {e}", self.path.display()))
    }

    fn stamp(&self) -> Result<Option<Stamp>> {
        match fs::metadata(&self.path) {
            Ok(m) => Ok(Some(Stamp {
                modified: m.modified().ok(),
                len: m.len(),
                #[cfg(unix)]
                mode: {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode()
                },
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(self.io_err(e)),
        }
    }

    /// Re-reads the file if its stamp changed or the cache is not loaded.
    fn refresh(&self, cache: &mut Cache) -> Result<()> {
        let stamp = self.stamp()?;
        if cache.loaded && cache.stamp == stamp {
            return Ok(());
        }
        self.load_into(cache, stamp)
    }

    /// Re-reads the file whatever its stamp says.
    fn reload(&self, cache: &mut Cache) -> Result<()> {
        let stamp = self.stamp()?;
        self.load_into(cache, stamp)
    }

    /// Reads the file. The stamp is taken before the read by the caller, so
    /// a write in between makes the next call read again. A file that
    /// cannot be read or parsed leaves the cache unloaded, so every later
    /// call fails too.
    fn load_into(&self, cache: &mut Cache, stamp: Option<Stamp>) -> Result<()> {
        cache.loaded = false;
        let entries = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| SyncError::Io(format!("{}: {e}", self.path.display())))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(self.io_err(e)),
        };
        cache.entries = entries;
        cache.stamp = stamp;
        cache.loaded = true;
        Ok(())
    }

    /// Writes `entries` atomically. The cache takes them only after the
    /// write succeeds, and is then marked for a reload on the next read.
    fn save(&self, cache: &mut Cache, entries: Vec<Entry>) -> Result<()> {
        let json = serde_json::to_vec_pretty(&entries).map_err(|e| SyncError::Io(e.to_string()))?;
        write_atomic(&self.path, &json).map_err(|e| self.io_err(e))?;
        cache.entries = entries;
        cache.loaded = false;
        Ok(())
    }

    /// A new token for a device named `label` (shown once; only its hash
    /// is kept).
    pub fn pair(&self, label: &str) -> Result<String> {
        let token = random_hex()?;
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        entries.push(Entry {
            hash: sha256_hex(token.as_bytes()),
            label: label.to_string(),
            device: None,
            created_ms: now_ms(),
            host: false,
            named: false,
        });
        self.save(&mut cache, entries)?;
        Ok(token)
    }

    /// Removes the tokens labelled `label`. Returns how many.
    pub fn unpair(&self, label: &str) -> Result<usize> {
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        let before = entries.len();
        entries.retain(|x| x.label != label);
        let n = before - entries.len();
        if n > 0 {
            self.save(&mut cache, entries)?;
        }
        Ok(n)
    }

    /// The paired devices. If the file cannot be read, there are none: the
    /// signature has no error to return, and a file that cannot be read
    /// authenticates nothing either.
    pub fn devices(&self) -> Vec<PairedDevice> {
        let mut cache = self.lock();
        if self.refresh(&mut cache).is_err() {
            return Vec::new();
        }
        cache
            .entries
            .iter()
            .map(|x| PairedDevice {
                label: x.label.clone(),
                device: x.device.clone(),
                created_ms: x.created_ms,
                host: x.host,
            })
            .collect()
    }

    /// Marks `device` as a host (it connected to `/host/ws`). Writes only
    /// when the mark is new.
    pub fn mark_host(&self, device: &str) -> Result<()> {
        {
            let mut cache = self.lock();
            self.refresh(&mut cache)?;
            if cache
                .entries
                .iter()
                .filter(|x| x.device.as_deref() == Some(device))
                .all(|x| x.host)
            {
                return Ok(());
            }
        }
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        let mut changed = false;
        for x in entries
            .iter_mut()
            .filter(|x| x.device.as_deref() == Some(device))
        {
            changed |= !x.host;
            x.host = true;
        }
        if changed {
            self.save(&mut cache, entries)?;
        }
        Ok(())
    }

    /// Renames `device` to `name` (trimmed). Refused (400) when empty,
    /// too long or `hub`, and (409) when another host already has the
    /// name, ignoring case. Devices that never connected as hosts don't
    /// hold a name. Returns the name used.
    pub fn rename(&self, device: &str, name: &str) -> Result<String> {
        let name: String = name.chars().filter(|c| !c.is_control()).collect();
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(SyncError::Status(400, "Enter a name".into()));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(SyncError::Status(
                400,
                format!("A name has at most {MAX_NAME_CHARS} characters"),
            ));
        }
        if name.eq_ignore_ascii_case("hub") {
            return Err(SyncError::Status(400, "hub is the hub's own name".into()));
        }
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        if !entries.iter().any(|x| x.device.as_deref() == Some(device)) {
            return Err(SyncError::Status(
                404,
                "this device is not on the hub".into(),
            ));
        }
        let lower = name.to_lowercase();
        if entries.iter().any(|x| {
            x.host && x.device.as_deref() != Some(device) && x.label.to_lowercase() == lower
        }) {
            return Err(SyncError::Status(
                409,
                format!("Another computer is already called {name}"),
            ));
        }
        for x in entries
            .iter_mut()
            .filter(|x| x.device.as_deref() == Some(device))
        {
            x.label = name.clone();
            x.named = true;
        }
        self.save(&mut cache, entries)?;
        Ok(name)
    }

    /// A token for `device` after a login: the device's earlier tokens are
    /// dropped (a re-login replaces them), and the label gets a ` (2)`,
    /// ` (3)`, ... suffix if another device already uses `name`. A device
    /// the user renamed keeps its name, and a host stays a host. Returns
    /// the token (shown once) and the label used.
    pub fn login(&self, name: &str, device: &str, hub_device: &str) -> Result<(String, String)> {
        if device.is_empty() {
            return Err(SyncError::Status(400, "missing device id".into()));
        }
        if device == hub_device {
            return Err(SyncError::Status(
                409,
                "this device id is the hub's own".into(),
            ));
        }
        let name = if name.trim().is_empty() {
            "device"
        } else {
            name.trim()
        };
        let token = random_hex()?;
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        let earlier: Vec<Entry> = entries
            .iter()
            .filter(|x| x.device.as_deref() == Some(device))
            .cloned()
            .collect();
        entries.retain(|x| x.device.as_deref() != Some(device));
        let host = earlier.iter().any(|x| x.host);
        let named = earlier.iter().find(|x| x.named).map(|x| x.label.clone());
        let label = match &named {
            Some(label) => label.clone(),
            None => {
                let mut label = name.to_string();
                let mut n = 2;
                while entries.iter().any(|x| x.label == label) {
                    label = format!("{name} ({n})");
                    n += 1;
                }
                label
            }
        };
        entries.push(Entry {
            hash: sha256_hex(token.as_bytes()),
            label: label.clone(),
            device: Some(device.to_string()),
            created_ms: now_ms(),
            host,
            named: named.is_some(),
        });
        self.save(&mut cache, entries)?;
        Ok((token, label))
    }

    /// Drops the entry of `token` (a device signing out). Returns whether
    /// there was one.
    pub fn revoke(&self, token: &str) -> Result<bool> {
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        let Some(i) = find(&entries, token) else {
            return Ok(false);
        };
        entries.remove(i);
        self.save(&mut cache, entries)?;
        Ok(true)
    }

    /// Checks a request's token and device id; binds the token to the
    /// device on first use. Only the first use takes the file lock and
    /// writes; every other check reads the file (or the cache, if the file
    /// has not changed) and returns.
    pub fn check(&self, token: &str, device: &str, hub_device: &str) -> Result<()> {
        if device.is_empty() {
            return Err(SyncError::Status(400, "missing device id".into()));
        }
        if device == hub_device {
            return Err(SyncError::Status(
                409,
                "this vault is a copy of the hub's vault (same device id); \
                 delete .vault/device in the copy"
                    .into(),
            ));
        }
        {
            let mut cache = self.lock();
            self.refresh(&mut cache)?;
            if let Verdict::Allowed = judge(&cache.entries, token, device)? {
                return Ok(());
            }
        }
        // First use: the binding is a read-modify-write, so it is redone
        // under the file lock, against the file as it is now.
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        match judge(&entries, token, device)? {
            Verdict::Allowed => Ok(()),
            Verdict::Bind(i) => {
                entries[i].device = Some(device.to_string());
                self.save(&mut cache, entries)
            }
        }
    }
}

/// Writes `bytes` to a new temp file beside `path`, then renames it over
/// `path`, so a reader sees the old file or the new one, never part of
/// either. The temp name is unique to this process and call, and the file
/// is 0600 on Unix from its creation.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{FILE}.{}.{nanos}.{seq}.tmp", std::process::id()));
    if let Err(e) = write_tmp_then_rename(&tmp, path, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(())
}

fn write_tmp_then_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = create_private(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(tmp, path)
}

/// Options for a file this module creates: mode 0600 on Unix. The mode
/// applies only when the call creates the file.
fn private_options() -> fs::OpenOptions {
    let mut o = fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

fn create_private(path: &Path) -> io::Result<fs::File> {
    private_options().write(true).create_new(true).open(path)
}

/// The lock file, created if missing and never removed. Opened read-write
/// because Windows' `LockFileEx` needs a handle with access to the file.
fn open_lock_file(path: &Path) -> io::Result<fs::File> {
    private_options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Makes the rename durable on Unix. Best effort.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> TempRoot {
            let dir =
                std::env::temp_dir().join(format!("vault-sync-tokens-{}", random_hex().unwrap()));
            fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }

        fn path(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }

        fn dir(&self) -> PathBuf {
            self.0.join(".vault").join("hub")
        }

        fn file(&self) -> PathBuf {
            self.dir().join(FILE)
        }

        fn lock_file(&self) -> PathBuf {
            self.dir().join(LOCK_FILE)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// One entry of the file, as the format stores it.
    fn entry_json(label: &str, token: &str, device: Option<&str>) -> String {
        let device = match device {
            Some(d) => format!("\"{d}\""),
            None => "null".into(),
        };
        format!(
            r#"{{"hash":"{}","label":"{label}","device":{device},"created_ms":1}}"#,
            sha256_hex(token.as_bytes())
        )
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn pair_check_and_refusals() {
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        let laptop = store.pair("laptop").unwrap();
        assert!(!fs::read_to_string(root.file()).unwrap().contains(&laptop));

        store.check(&laptop, "dev-a", "hub").unwrap();
        store.check(&laptop, "dev-a", "hub").unwrap();
        assert!(matches!(
            store.check(&laptop, "dev-b", "hub"),
            Err(SyncError::Status(403, _))
        ));
        assert!(matches!(
            store.check("nope", "dev-a", "hub"),
            Err(SyncError::Status(401, _))
        ));
        assert!(matches!(
            store.check(&laptop, "", "hub"),
            Err(SyncError::Status(400, _))
        ));
        assert!(matches!(
            store.check(&laptop, "hub", "hub"),
            Err(SyncError::Status(409, _))
        ));

        // A second token cannot bind a device that already has one.
        let phone = store.pair("phone").unwrap();
        assert!(matches!(
            store.check(&phone, "dev-a", "hub"),
            Err(SyncError::Status(409, _))
        ));
        assert!(!store.revoke("missing").unwrap());
        assert!(store.revoke(&phone).unwrap());

        let devices = Tokens::open(&root.path()).unwrap().devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].label, "laptop");
        assert_eq!(devices[0].device.as_deref(), Some("dev-a"));
    }

    #[test]
    fn rename_is_unique_among_hosts_only_and_survives_a_relogin() {
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        store.login("pc", "dev-a", "hub").unwrap();
        // A stale sign-in that never ran the relay holds "pc" too.
        let (_, label) = store.login("pc", "dev-b", "hub").unwrap();
        assert_eq!(label, "pc (2)");
        store.login("laptop", "dev-c", "hub").unwrap();
        store.mark_host("dev-b").unwrap();
        store.mark_host("dev-c").unwrap();
        // dev-a is no host, so it does not block the name.
        assert_eq!(store.rename("dev-b", "  PC ").unwrap(), "PC");
        assert!(matches!(
            store.rename("dev-c", "pc"),
            Err(SyncError::Status(409, m)) if m == "Another computer is already called pc"
        ));
        assert!(matches!(
            store.rename("dev-c", " "),
            Err(SyncError::Status(400, _))
        ));
        assert!(matches!(
            store.rename("dev-c", "Hub"),
            Err(SyncError::Status(400, _))
        ));
        assert!(matches!(
            store.rename("dev-x", "x"),
            Err(SyncError::Status(404, _))
        ));
        // Renaming to its own name in another case is fine.
        assert_eq!(store.rename("dev-b", "pc").unwrap(), "pc");
        // A re-login keeps the user's name and the host mark.
        let (_, label) = store.login("Other", "dev-b", "hub").unwrap();
        assert_eq!(label, "pc");
        let b = store
            .devices()
            .into_iter()
            .find(|d| d.device.as_deref() == Some("dev-b"))
            .unwrap();
        assert!(b.host);
        let a = store
            .devices()
            .into_iter()
            .find(|d| d.device.as_deref() == Some("dev-a"))
            .unwrap();
        assert!(!a.host);
    }

    #[test]
    fn login_replaces_the_devices_earlier_token() {
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        let (old, label) = store.login("phone", "dev-p", "hub").unwrap();
        assert_eq!(label, "phone");
        store.check(&old, "dev-p", "hub").unwrap();

        let (new, label) = store.login("phone", "dev-p", "hub").unwrap();
        assert_eq!(label, "phone");
        assert!(matches!(
            store.check(&old, "dev-p", "hub"),
            Err(SyncError::Status(401, _))
        ));
        store.check(&new, "dev-p", "hub").unwrap();

        let (_, label) = store.login("phone", "dev-q", "hub").unwrap();
        assert_eq!(label, "phone (2)");
        assert_eq!(store.devices().len(), 2);
        assert!(matches!(
            store.login("x", "hub", "hub"),
            Err(SyncError::Status(409, _))
        ));
        assert!(matches!(
            store.login("x", "", "hub"),
            Err(SyncError::Status(400, _))
        ));
    }

    #[test]
    fn revoke_through_another_store_is_seen_and_never_resurrected() {
        // `a` holds a cache from before `b`'s change. Neither a later save by
        // `a` nor a first-use binding by `a` may write back the revoked entry.
        let root = TempRoot::new();
        let a = Tokens::open(&root.path()).unwrap();
        let b = Tokens::open(&root.path()).unwrap();
        let old = a.pair("old").unwrap();
        a.check(&old, "dev-old", "hub").unwrap();

        assert_eq!(b.unpair("old").unwrap(), 1);
        assert!(matches!(
            a.check(&old, "dev-old", "hub"),
            Err(SyncError::Status(401, _))
        ));

        let fresh = b.pair("fresh").unwrap();
        a.pair("from-a").unwrap();
        a.check(&fresh, "dev-fresh", "hub").unwrap();

        let text = fs::read_to_string(root.file()).unwrap();
        assert!(!text.contains("\"old\""));
        assert!(!text.contains(&sha256_hex(old.as_bytes())));
        let labels: BTreeSet<String> = Tokens::open(&root.path())
            .unwrap()
            .devices()
            .into_iter()
            .map(|d| d.label)
            .collect();
        assert_eq!(
            labels,
            BTreeSet::from(["fresh".to_string(), "from-a".to_string()])
        );
    }

    #[test]
    fn checks_take_no_file_lock() {
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        // Nothing has been written yet, so there is no lock file either.
        assert!(!root.lock_file().exists());
        assert!(matches!(
            store.check("x", "dev", "hub"),
            Err(SyncError::Status(401, _))
        ));
        assert!(!root.lock_file().exists(), "a refused check takes no lock");
    }

    #[test]
    fn same_size_rewrite_with_new_mtime_is_reloaded() {
        // The two files differ only in the label and the token, which have the
        // same lengths, so the size is unchanged. Only the mtime tells them
        // apart. The mtime is set explicitly, so there is no sleep and no tie.
        let root = TempRoot::new();
        fs::create_dir_all(root.dir()).unwrap();
        let first = format!("[{}]", entry_json("aaaa", "token-one", Some("dev-1")));
        fs::write(root.file(), &first).unwrap();
        let first_mtime = fs::metadata(root.file()).unwrap().modified().unwrap();

        let store = Tokens::open(&root.path()).unwrap();
        store.check("token-one", "dev-1", "hub").unwrap();

        let second = format!("[{}]", entry_json("bbbb", "token-two", Some("dev-1")));
        assert_eq!(second.len(), first.len());
        fs::write(root.file(), &second).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(root.file())
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(first_mtime + Duration::from_secs(2)))
            .unwrap();

        store.check("token-two", "dev-1", "hub").unwrap();
        assert!(matches!(
            store.check("token-one", "dev-1", "hub"),
            Err(SyncError::Status(401, _))
        ));
        assert_eq!(store.devices()[0].label, "bbbb");
    }

    #[test]
    fn concurrent_stores_keep_every_pairing() {
        const THREADS: usize = 2;
        const PER_THREAD: usize = 8;
        let root = TempRoot::new();
        std::thread::scope(|s| {
            for t in 0..THREADS {
                let path = root.path();
                s.spawn(move || {
                    let store = Tokens::open(&path).unwrap();
                    for i in 0..PER_THREAD {
                        store.pair(&format!("t{t}-{i}")).unwrap();
                    }
                });
            }
        });
        let labels: BTreeSet<String> = Tokens::open(&root.path())
            .unwrap()
            .devices()
            .into_iter()
            .map(|d| d.label)
            .collect();
        assert_eq!(labels.len(), THREADS * PER_THREAD);
    }

    #[test]
    fn concurrent_first_use_binds_once() {
        // Two stores race to bind one unbound token to two devices. The file
        // lock makes exactly one of them win; the other is refused.
        for _ in 0..10 {
            let root = TempRoot::new();
            let token = Tokens::open(&root.path()).unwrap().pair("shared").unwrap();
            let barrier = std::sync::Barrier::new(2);
            let results: Vec<Result<()>> = std::thread::scope(|s| {
                let handles: Vec<_> = ["dev-x", "dev-y"]
                    .into_iter()
                    .map(|dev| {
                        let (path, token, barrier) = (root.path(), token.clone(), &barrier);
                        s.spawn(move || {
                            let store = Tokens::open(&path).unwrap();
                            barrier.wait();
                            store.check(&token, dev, "hub")
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert!(results
                .iter()
                .any(|r| matches!(r, Err(SyncError::Status(403, _)))));
            let devices = Tokens::open(&root.path()).unwrap().devices();
            assert_eq!(devices.len(), 1);
            assert!(devices[0].device.is_some());
        }
    }

    #[test]
    fn malformed_file_fails_closed() {
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        let token = store.pair("ci").unwrap();
        let good = fs::read(root.file()).unwrap();

        let broken = b"[{\"hash\": \"x\"";
        fs::write(root.file(), broken).unwrap();
        assert!(matches!(
            store.check(&token, "dev-a", "hub"),
            Err(SyncError::Io(_))
        ));
        assert!(store.pair("next").is_err());
        assert!(store.revoke(&token).is_err());
        assert!(store.devices().is_empty());
        assert!(Tokens::open(&root.path()).is_err());
        assert_eq!(fs::read(root.file()).unwrap(), broken);

        fs::write(root.file(), &good).unwrap();
        store.check(&token, "dev-a", "hub").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private_complete_and_alone_in_the_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        for i in 0..3 {
            let token = store.pair(&format!("d{i}")).unwrap();
            store.check(&token, &format!("dev-{i}"), "hub").unwrap();
            let v: Vec<serde_json::Value> =
                serde_json::from_slice(&fs::read(root.file()).unwrap()).unwrap();
            assert_eq!(v.len(), i + 1);
        }
        store.unpair("d1").unwrap();
        store.login("phone", "dev-p", "hub").unwrap();
        assert_eq!(
            names(&root.dir()),
            vec![FILE.to_string(), LOCK_FILE.to_string()]
        );
        for path in [root.file(), root.lock_file()] {
            let mode = fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    /// Restores a directory's write permission when dropped, so the test
    /// directory can always be removed.
    #[cfg(unix)]
    struct Writable(PathBuf);

    #[cfg(unix)]
    impl Drop for Writable {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_save_changes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        let keep = store.pair("keep").unwrap();
        store.check(&keep, "dev-k", "hub").unwrap();

        // The lock file already exists, so only the temp file's creation
        // fails: the save is what fails.
        let dir = root.dir();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let _restore = Writable(dir.clone());
        if fs::File::create(dir.join("probe")).is_ok() {
            // Root ignores directory permissions, so the failure cannot be made.
            let _ = fs::remove_file(dir.join("probe"));
            eprintln!("failed_save_changes_nothing: skipped, directory stays writable");
            return;
        }

        assert!(store.pair("late").is_err());
        assert!(store.revoke(&keep).is_err());
        // The unsaved revoke did not happen: the cache still holds the token.
        store.check(&keep, "dev-k", "hub").unwrap();
        assert_eq!(store.devices().len(), 1);
        assert_eq!(names(&dir), vec![FILE.to_string(), LOCK_FILE.to_string()]);
        assert_eq!(Tokens::open(&root.path()).unwrap().devices().len(), 1);

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        store.pair("after").unwrap();
        assert_eq!(store.devices().len(), 2);
    }

    /// Restores a file's mode when dropped, so a failed assertion or a panic
    /// cannot leave the test's file unreadable.
    #[cfg(unix)]
    struct FileMode {
        path: PathBuf,
        mode: u32,
    }

    #[cfg(unix)]
    impl Drop for FileMode {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_refuses_cached_entries_with_same_mtime_and_size() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempRoot::new();
        let store = Tokens::open(&root.path()).unwrap();
        let token = store.pair("ci").unwrap();
        // The first check binds the token and saves. The second reads the file
        // and caches its entries, which then authenticate on their own.
        store.check(&token, "dev-a", "hub").unwrap();
        store.check(&token, "dev-a", "hub").unwrap();

        let before = fs::metadata(root.file()).unwrap();
        let mode = before.permissions().mode() & 0o777;
        // Restores the mode on drop, even if an assertion below panics.
        let _restore = FileMode {
            path: root.file(),
            mode,
        };
        fs::set_permissions(root.file(), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(root.file()).is_ok() {
            // Root reads any file whatever its mode, so the failure cannot be made.
            eprintln!("unreadable_file_refuses_cached_entries: skipped, the file stays readable");
            return;
        }
        let after = fs::metadata(root.file()).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());

        // The cached entries must not authenticate while the file is unreadable.
        assert!(matches!(
            store.check(&token, "dev-a", "hub"),
            Err(SyncError::Io(_))
        ));
        assert!(store.devices().is_empty());

        // Permission restored: the file is read again and the token works.
        fs::set_permissions(root.file(), fs::Permissions::from_mode(mode)).unwrap();
        store.check(&token, "dev-a", "hub").unwrap();
        assert_eq!(store.devices().len(), 1);
    }
}
