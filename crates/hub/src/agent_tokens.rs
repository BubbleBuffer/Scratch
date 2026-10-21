//! Agent tokens: `.vault/hub/agent-tokens.json` in the hub's vault. Each
//! entry is `{name, scope, sha256, created}`. `add` shows the plaintext
//! token once and keeps only its SHA-256; nothing else can recover it.
//!
//! The file is re-read when its size or mtime changes, and every mutation
//! re-reads it first. Two [`AgentTokens`] on the same file (the server and
//! the `--agent-token` CLI, say) therefore see each other's writes. A file
//! that cannot be read or parsed is an error, and [`AgentTokens::authenticate`]
//! then refuses every token.
//!
//! Mutations (`add`, `revoke`) also hold an exclusive lock on the stable
//! file `agent-tokens.lock` beside the JSON, across their reload and save,
//! so two processes cannot both reload, then overwrite each other. Reads
//! (`list`, `authenticate`) take no file lock: the JSON is replaced by
//! rename, so a reader sees the old file or the new one.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vault_core::sha256_hex;
use vault_sync::ct_eq;

const FILE: &str = "agent-tokens.json";
/// Never renamed or removed: the lock lives on this inode.
const LOCK_FILE: &str = "agent-tokens.lock";
const MAX_NAME: usize = 40;

/// What a token may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentScope {
    Read,
    Write,
}

impl AgentScope {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentScope::Read => "read",
            AgentScope::Write => "write",
        }
    }
}

impl fmt::Display for AgentScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentScope {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<AgentScope, String> {
        match s {
            "read" => Ok(AgentScope::Read),
            "write" => Ok(AgentScope::Write),
            _ => Err(format!("scope must be read or write, not {s:?}")),
        }
    }
}

/// A token's public identity, as `list` and `authenticate` return it. The
/// hash is not part of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToken {
    pub name: String,
    pub scope: AgentScope,
    /// Unix milliseconds when the token was added.
    pub created: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    scope: AgentScope,
    sha256: String,
    created: i64,
}

impl Entry {
    fn identity(&self) -> AgentToken {
        AgentToken {
            name: self.name.clone(),
            scope: self.scope,
            created: self.created,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTokenError {
    /// The name is not `[a-z0-9-]{1,40}`.
    InvalidName(String),
    /// A token with this name already exists.
    Duplicate(String),
    /// The file exists but cannot be read or parsed, so nothing in it is trusted.
    Malformed(String),
    /// Reading or writing the file failed.
    Io(String),
}

impl fmt::Display for AgentTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentTokenError::InvalidName(n) => write!(
                f,
                "agent token name {n:?}: use 1 to {MAX_NAME} characters of a-z, 0-9 and -"
            ),
            AgentTokenError::Duplicate(n) => write!(f, "an agent token named {n:?} already exists"),
            AgentTokenError::Malformed(m) => write!(f, "agent tokens file is malformed: {m}"),
            AgentTokenError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for AgentTokenError {}

pub type Result<T> = std::result::Result<T, AgentTokenError>;

/// A file's identity for change detection: mtime and size.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    // chmod does not change mtime or size. Notice an unreadable file even
    // when its last successfully read contents are still cached.
    #[cfg(unix)]
    mode: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FailureStamp {
    stamp: Option<Stamp>,
    metadata_error: bool,
}

#[derive(Default)]
struct Cache {
    /// False until a read succeeds, and after this store writes the file.
    loaded: bool,
    /// The file's stamp taken before the last read.
    stamp: Option<Stamp>,
    entries: Vec<Entry>,
    last_failure: Option<FailureStamp>,
}

/// The agent-token store of one vault. Cheap to open; several instances on
/// the same file, in one process or many, stay consistent through the files.
pub struct AgentTokens {
    path: PathBuf,
    lock_path: PathBuf,
    cache: Mutex<Cache>,
    log_failures: bool,
}

impl AgentTokens {
    /// The store in `<vault_root>/.vault/hub/agent-tokens.json`. A missing
    /// file is an empty store, and a malformed one is an error. Nothing is
    /// created until a token is added.
    pub fn open(vault_root: impl AsRef<Path>) -> Result<AgentTokens> {
        let store = Self::new(vault_root.as_ref(), false);
        store.check_available()?;
        Ok(store)
    }

    /// Keeps the hub running when this optional credential file is broken.
    /// Requests retry failed reads, allowing permission repairs as well as
    /// content changes to recover without a restart.
    pub(crate) fn open_for_server(vault_root: impl AsRef<Path>) -> AgentTokens {
        let store = Self::new(vault_root.as_ref(), true);
        let _ = store.check_available();
        store
    }

    fn new(vault_root: &Path, log_failures: bool) -> AgentTokens {
        let dir = vault_root.join(".vault").join("hub");
        AgentTokens {
            path: dir.join(FILE),
            lock_path: dir.join(LOCK_FILE),
            cache: Mutex::new(Cache::default()),
            log_failures,
        }
    }

    pub(crate) fn check_available(&self) -> Result<()> {
        self.refresh(&mut self.lock())
    }

    /// A new token named `name` with `scope`. Returns the plaintext token,
    /// which is not kept anywhere; show it to the user now.
    pub fn add(&self, name: &str, scope: AgentScope) -> Result<String> {
        check_name(name)?;
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        if entries.iter().any(|e| e.name == name) {
            return Err(AgentTokenError::Duplicate(name.to_string()));
        }
        let token = random_hex(32)?;
        entries.push(Entry {
            name: name.to_string(),
            scope,
            sha256: sha256_hex(token.as_bytes()),
            created: now_ms(),
        });
        self.save(&mut cache, entries)?;
        Ok(token)
    }

    /// Every token's identity, in the order they were added.
    pub fn list(&self) -> Result<Vec<AgentToken>> {
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        Ok(cache.entries.iter().map(Entry::identity).collect())
    }

    /// Removes the token named `name`. Returns whether there was one.
    pub fn revoke(&self, name: &str) -> Result<bool> {
        check_name(name)?;
        let _file_lock = self.lock_file()?;
        let mut cache = self.lock();
        self.reload(&mut cache)?;
        let mut entries = cache.entries.clone();
        let before = entries.len();
        entries.retain(|e| e.name != name);
        if entries.len() == before {
            return Ok(false);
        }
        self.save(&mut cache, entries)?;
        Ok(true)
    }

    /// The identity of the token `token` belongs to, or `None`. An `Err`
    /// means the file could not be used, and the caller must refuse.
    pub fn authenticate(&self, token: &str) -> Result<Option<AgentToken>> {
        let mut cache = self.lock();
        self.refresh(&mut cache)?;
        let hash = sha256_hex(token.as_bytes());
        let mut found = None;
        // Every entry is compared, so timing does not show which one matched.
        for e in &cache.entries {
            if ct_eq(e.sha256.as_bytes(), hash.as_bytes()) {
                found = Some(e.identity());
            }
        }
        Ok(found)
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The cross-process mutation lock. Each call opens a fresh handle; the
    /// lock is held while the returned file lives, and closing the handle
    /// releases it. Taken before the cache mutex, and never by reads, so
    /// the two cannot wait on each other.
    fn lock_file(&self) -> Result<fs::File> {
        let lock_err =
            |e: io::Error| AgentTokenError::Io(format!("{}: {e}", self.lock_path.display()));
        if let Some(dir) = self.lock_path.parent() {
            fs::create_dir_all(dir).map_err(lock_err)?;
        }
        let file = open_lock_file(&self.lock_path).map_err(lock_err)?;
        file.lock().map_err(lock_err)?;
        Ok(file)
    }

    fn io_err(&self, e: io::Error) -> AgentTokenError {
        AgentTokenError::Io(format!("{}: {e}", self.path.display()))
    }

    /// Re-reads the file if its stamp changed or the cache is not loaded.
    fn refresh(&self, cache: &mut Cache) -> Result<()> {
        let stamp = self.stamp();
        let failure = FailureStamp {
            stamp: stamp.as_ref().ok().copied().flatten(),
            metadata_error: stamp.is_err(),
        };
        let result = stamp.and_then(|stamp| {
            if cache.loaded && cache.stamp == stamp {
                return Ok(());
            }
            self.load_into(cache, stamp)
        });
        if result.is_err() {
            cache.loaded = false;
            cache.entries.clear();
            if self.log_failures && cache.last_failure != Some(failure) {
                // Parser errors may quote input values. Never log them, hashes
                // or credential material from an unusable file.
                tracing::error!(
                    path = %self.path.display(),
                    "agent token file is unreadable or malformed; /mcp disabled until repaired"
                );
            }
            cache.last_failure = Some(failure);
        } else {
            cache.last_failure = None;
        }
        result
    }

    /// Re-reads the file whatever its stamp says.
    fn reload(&self, cache: &mut Cache) -> Result<()> {
        let stamp = self.stamp()?;
        self.load_into(cache, stamp)
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

    /// Reads the file. The stamp is taken before the read by the caller, so
    /// a write in between makes the next call read again.
    fn load_into(&self, cache: &mut Cache, stamp: Option<Stamp>) -> Result<()> {
        cache.loaded = false;
        let entries = match fs::read(&self.path) {
            Ok(bytes) => parse(&bytes)
                .map_err(|m| AgentTokenError::Malformed(format!("{}: {m}", self.path.display())))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(self.io_err(e)),
        };
        cache.entries = entries;
        cache.stamp = stamp;
        cache.loaded = true;
        Ok(())
    }

    /// Writes `entries` atomically, then makes the cache hold them and
    /// marks it for a reload on the next read.
    fn save(&self, cache: &mut Cache, entries: Vec<Entry>) -> Result<()> {
        let mut json =
            serde_json::to_vec_pretty(&entries).map_err(|e| AgentTokenError::Io(e.to_string()))?;
        json.push(b'\n');
        write_atomic(&self.path, &json).map_err(|e| self.io_err(e))?;
        cache.entries = entries;
        cache.loaded = false;
        Ok(())
    }
}

/// `[a-z0-9-]{1,40}`.
fn check_name(name: &str) -> Result<()> {
    let ok = (1..=MAX_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(AgentTokenError::InvalidName(name.to_string()))
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parses and checks the whole file. Any problem makes the file unusable.
fn parse(bytes: &[u8]) -> std::result::Result<Vec<Entry>, String> {
    let entries: Vec<Entry> = serde_json::from_slice(bytes).map_err(|err| err.to_string())?;
    for (i, e) in entries.iter().enumerate() {
        check_name(&e.name).map_err(|err| err.to_string())?;
        if !is_sha256_hex(&e.sha256) {
            return Err(format!(
                "{:?}: sha256 is not a lowercase hex digest",
                e.name
            ));
        }
        if entries[..i].iter().any(|x| x.name == e.name) {
            return Err(format!("duplicate name {:?}", e.name));
        }
    }
    Ok(entries)
}

/// `n` random bytes, hex-encoded (`2n` characters).
fn random_hex(n: usize) -> Result<String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| AgentTokenError::Io(format!("random: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Writes `bytes` to a new temp file beside `path`, then renames it over
/// `path`, so a reader sees the old file or the new one, never half of
/// either. The temp file is 0600 on Unix from its creation.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{FILE}.{}.{nanos}.tmp", std::process::id()));
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
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> TempRoot {
            let suffix = random_hex(8).unwrap();
            let dir = std::env::temp_dir().join(format!("notes-agent-tokens-{suffix}"));
            fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join(".vault").join("hub").join(FILE)
        }

        fn lock_file(&self) -> PathBuf {
            self.0.join(".vault").join("hub").join(LOCK_FILE)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry_json(name: &str, sha: &str, scope: &str) -> String {
        format!(r#"{{"name":"{name}","scope":"{scope}","sha256":"{sha}","created":1}}"#)
    }

    #[test]
    fn only_the_hash_is_stored() {
        let root = TempRoot::new();
        let store = AgentTokens::open(&root.0).unwrap();
        let token = store.add("ci-bot", AgentScope::Read).unwrap();
        assert_eq!(token.len(), 64);
        let text = fs::read_to_string(root.file()).unwrap();
        assert!(!text.contains(&token));
        assert!(text.contains(&sha256_hex(token.as_bytes())));

        let who = store.authenticate(&token).unwrap().expect("known token");
        assert_eq!(who.name, "ci-bot");
        assert_eq!(who.scope, AgentScope::Read);
        assert!(store.authenticate(&format!("{token}0")).unwrap().is_none());
        assert!(store.authenticate("").unwrap().is_none());

        let other = store.add("other", AgentScope::Write).unwrap();
        assert_ne!(token, other);
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].scope, AgentScope::Write);
    }

    #[test]
    fn writes_are_complete_and_private() {
        let root = TempRoot::new();
        let store = AgentTokens::open(&root.0).unwrap();
        for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
            store.add(name, AgentScope::Read).unwrap();
            let bytes = fs::read(root.file()).unwrap();
            let v: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v.len(), i + 1);
        }
        store.revoke("b").unwrap();
        let v: Vec<serde_json::Value> =
            serde_json::from_slice(&fs::read(root.file()).unwrap()).unwrap();
        assert_eq!(v.len(), 2);

        // The JSON and the stable lock file, and no temp files left behind.
        let mut names: Vec<String> = fs::read_dir(root.file().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![FILE.to_string(), LOCK_FILE.to_string()]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [root.file(), root.lock_file()] {
                let mode = fs::metadata(path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600);
            }
        }
    }

    #[test]
    fn concurrent_stores_keep_every_addition() {
        // Two threads, each with its own store on the same files, each doing
        // a fixed number of additions. Without the file lock, reload+save
        // interleave and some additions are lost.
        const THREADS: usize = 2;
        const PER_THREAD: usize = 8;
        let root = TempRoot::new();
        let added: Vec<(String, String)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..THREADS)
                .map(|t| {
                    let root = &root.0;
                    s.spawn(move || {
                        let store = AgentTokens::open(root).unwrap();
                        (0..PER_THREAD)
                            .map(|i| {
                                let name = format!("t{t}-{i}");
                                let token = store.add(&name, AgentScope::Read).unwrap();
                                (name, token)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });

        let store = AgentTokens::open(&root.0).unwrap();
        assert_eq!(store.list().unwrap().len(), THREADS * PER_THREAD);
        for (name, token) in &added {
            let who = store
                .authenticate(token)
                .unwrap()
                .expect("addition survived");
            assert_eq!(&who.name, name);
        }
    }

    #[test]
    fn same_size_rewrite_with_new_mtime_is_reloaded() {
        // The two files differ only in the name and hash, which have the same
        // lengths, so the size is unchanged. Only the mtime tells them apart.
        // The mtimes are set explicitly, so there is no sleep and no tie.
        use std::time::Duration;

        fn file_with(name: &str, token: &str) -> String {
            let sha = sha256_hex(token.as_bytes());
            format!(r#"[{{"name":"{name}","scope":"read","sha256":"{sha}","created":1}}]"#)
        }

        let root = TempRoot::new();
        fs::create_dir_all(root.file().parent().unwrap()).unwrap();
        let first = file_with("aaaa", "token-one");
        fs::write(root.file(), &first).unwrap();
        let first_mtime = fs::metadata(root.file()).unwrap().modified().unwrap();

        let store = AgentTokens::open(&root.0).unwrap();
        assert!(store.authenticate("token-one").unwrap().is_some());
        assert!(!root.lock_file().exists(), "reads take no file lock");

        let second = file_with("bbbb", "token-two");
        assert_eq!(second.len(), first.len());
        fs::write(root.file(), &second).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(root.file())
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(first_mtime + Duration::from_secs(2)))
            .unwrap();

        assert_eq!(
            store
                .authenticate("token-two")
                .unwrap()
                .expect("reloaded")
                .name,
            "bbbb"
        );
        assert!(store.authenticate("token-one").unwrap().is_none());
    }

    #[test]
    fn name_validation() {
        let long = "a".repeat(40);
        for ok in ["a", "0", "ci-bot-2", long.as_str()] {
            assert!(check_name(ok).is_ok(), "{ok:?} should be valid");
        }
        let too_long = "a".repeat(41);
        for bad in [
            "",
            "Ci",
            "ci_bot",
            "ci bot",
            "ci/bot",
            "ci.bot",
            "é",
            too_long.as_str(),
        ] {
            assert!(check_name(bad).is_err(), "{bad:?} should be invalid");
        }

        let root = TempRoot::new();
        let store = AgentTokens::open(&root.0).unwrap();
        assert!(matches!(
            store.add("Bad", AgentScope::Read),
            Err(AgentTokenError::InvalidName(_))
        ));
        assert!(!root.file().exists());
        store.add("dup", AgentScope::Read).unwrap();
        assert!(matches!(
            store.add("dup", AgentScope::Write),
            Err(AgentTokenError::Duplicate(_))
        ));
        assert_eq!(store.list().unwrap().len(), 1);

        assert_eq!("read".parse::<AgentScope>().unwrap(), AgentScope::Read);
        assert!("admin".parse::<AgentScope>().is_err());
    }

    #[test]
    fn stores_see_each_others_writes() {
        let root = TempRoot::new();
        let a = AgentTokens::open(&root.0).unwrap();
        let b = AgentTokens::open(&root.0).unwrap();

        let laptop = a.add("laptop", AgentScope::Read).unwrap();
        assert_eq!(b.authenticate(&laptop).unwrap().unwrap().name, "laptop");

        // `a` has now read the file; `b` changes it, and `a` must notice.
        assert!(a.authenticate(&laptop).unwrap().is_some());
        let second = b.add("second", AgentScope::Write).unwrap();
        assert_eq!(
            a.authenticate(&second).unwrap().unwrap().scope,
            AgentScope::Write
        );

        // Revoking through `b` takes effect in `a` at once.
        assert!(b.revoke("laptop").unwrap());
        assert!(a.authenticate(&laptop).unwrap().is_none());
        assert!(!b.revoke("laptop").unwrap());

        // A mutation checks the latest file, not its own cache.
        a.add("shared", AgentScope::Read).unwrap();
        assert!(matches!(
            b.add("shared", AgentScope::Write),
            Err(AgentTokenError::Duplicate(_))
        ));
        let names: Vec<String> = b.list().unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["second", "shared"]);
    }

    #[test]
    fn malformed_file_fails_closed() {
        let root = TempRoot::new();
        let store = AgentTokens::open(&root.0).unwrap();
        let token = store.add("ci", AgentScope::Read).unwrap();
        let good = fs::read(root.file()).unwrap();

        let broken = b"[{\"name\": \"ci\"";
        fs::write(root.file(), broken).unwrap();
        assert!(matches!(
            AgentTokens::open(&root.0),
            Err(AgentTokenError::Malformed(_))
        ));
        assert!(matches!(
            store.authenticate(&token),
            Err(AgentTokenError::Malformed(_))
        ));
        assert!(matches!(
            store.add("next", AgentScope::Read),
            Err(AgentTokenError::Malformed(_))
        ));
        assert_eq!(fs::read(root.file()).unwrap(), broken);

        fs::write(root.file(), &good).unwrap();
        assert!(store.authenticate(&token).unwrap().is_some());
    }

    #[test]
    fn file_contents_are_validated() {
        let h = "a".repeat(64);
        let ok = format!("[{}]", entry_json("ok", &h, "read"));
        assert!(parse(ok.as_bytes()).is_ok());
        assert!(parse(b"[]").unwrap().is_empty());

        let dup = format!(
            "[{},{}]",
            entry_json("ok", &h, "read"),
            entry_json("ok", &h, "write")
        );
        assert!(parse(dup.as_bytes()).is_err());
        let bad_name = format!("[{}]", entry_json("Bad", &h, "read"));
        assert!(parse(bad_name.as_bytes()).is_err());
        let bad_hash = format!("[{}]", entry_json("ok", "ABC", "read"));
        assert!(parse(bad_hash.as_bytes()).is_err());
        let bad_scope = format!("[{}]", entry_json("ok", &h, "admin"));
        assert!(parse(bad_scope.as_bytes()).is_err());
        let extra = format!(r#"[{{"name":"ok","scope":"read","sha256":"{h}","created":1,"x":1}}]"#);
        assert!(parse(extra.as_bytes()).is_err());
        assert!(parse(b"").is_err());
    }
}
