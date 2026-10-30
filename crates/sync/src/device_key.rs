//! The device's own key: an Ed25519 [`DeviceKey`] from
//! `bubblebuffer-device-key`, kept per vault as a plain key file at
//! `.vault/device-key.json`, beside `.vault/device`. Nothing under `.vault/`
//! is synced, exported or packed into a backup, so the key never leaves the
//! device. It is created on first need.
//!
//! The hub binds the key's public half (and its [`DeviceId`], the key id)
//! to the vault's device id; a proof over a hub nonce
//! ([`session_proof`]) then buys a short-lived session token
//! (`wiki/architecture/device-keys.md`).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use bubblebuffer_device_key::{DeviceId, DeviceKey};
use bubblebuffer_device_key::{relay_proof_hash, KeyFile, RelayRole};

use crate::{hex_encode, now_ms, Result, SyncError};

/// The label of every session proof (`relay_proof_hash`'s protocol label).
pub const PROOF_LABEL: &str = "scratch/session/v1";
/// The proof's role: v0.1.0 knows only `host` and `viewer`.
pub(crate) const PROOF_ROLE: RelayRole = RelayRole::Viewer;

const FILE: &str = "device-key.json";

/// Serializes creation within this process; across processes the file is
/// created with a hard link, which fails if it already exists.
static CREATE: Mutex<()> = Mutex::new(());

fn key_path(vault_root: &str) -> PathBuf {
    PathBuf::from(vault_root).join(".vault").join(FILE)
}

fn key_err(path: &Path, e: impl std::fmt::Display) -> SyncError {
    SyncError::Io(format!("{}: {e}", path.display()))
}

fn read_key(path: &Path) -> Result<Option<DeviceKey>> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let file = KeyFile::parse(&text).map_err(|e| key_err(path, e))?;
            if file.is_protected() {
                return Err(key_err(path, "the device key is passphrase-protected"));
            }
            Ok(Some(file.unlock_device(None).map_err(|e| key_err(path, e))?))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(key_err(path, e)),
    }
}

/// The vault's device key, created (and saved, mode 0600 on Unix) if the
/// vault has none yet.
pub fn load_or_create(vault_root: &str) -> Result<DeviceKey> {
    let path = key_path(vault_root);
    if let Some(key) = read_key(&path)? {
        return Ok(key);
    }
    let _guard = CREATE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(key) = read_key(&path)? {
        return Ok(key);
    }
    let key = DeviceKey::generate().map_err(|e| key_err(&path, e))?;
    let json = KeyFile::from_device(&key, "Scratch device", &iso_now(), None)
        .and_then(|f| f.to_json())
        .map_err(|e| key_err(&path, e))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| key_err(dir, e))?;
    let tmp = dir.join(format!(".{FILE}.{}.{}.tmp", std::process::id(), now_ms()));
    let written = (|| -> io::Result<()> {
        let mut f = private_create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        drop(f);
        // Fails if another process created the file first; then theirs wins.
        match fs::hard_link(&tmp, &path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    })();
    let _ = fs::remove_file(&tmp);
    written.map_err(|e| key_err(&path, e))?;
    read_key(&path)?.ok_or_else(|| key_err(&path, "the device key file vanished"))
}

/// The key id (the library's [`DeviceId`], like `K7MQ-2X9F-D4RT-8WHN`) of
/// the vault's device key, creating the key if needed.
pub fn key_id(vault_root: &str) -> Result<String> {
    Ok(load_or_create(vault_root)?.id().to_string())
}

/// A signed session proof, as `/auth/session` and `/auth/upgrade` take it
/// (hex for the byte fields).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionProof {
    /// The vault device id the key is bound to.
    pub device: String,
    pub key_id: String,
    pub pubkey: String,
    pub origin: String,
    pub nonce: String,
    pub sig: String,
}

/// Signs `relay_proof_hash(PROOF_LABEL, viewer, origin, nonce, key id,
/// public key)` with `key`.
pub fn session_proof(
    key: &DeviceKey,
    device: &str,
    origin: &str,
    nonce: &[u8; 32],
) -> Result<SessionProof> {
    let public = key.public_key();
    let hash = relay_proof_hash(PROOF_LABEL, PROOF_ROLE, origin, nonce, key.id(), &public)
        .map_err(|e| SyncError::Protocol(e.to_string()))?;
    Ok(SessionProof {
        device: device.to_string(),
        key_id: key.id().to_string(),
        pubkey: hex_encode(&public),
        origin: origin.to_string(),
        nonce: hex_encode(nonce),
        sig: hex_encode(&key.sign(&hash)),
    })
}

fn private_create(path: &Path) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.write(true).create_new(true).open(path)
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ` (the key file's `created`).
fn iso_now() -> String {
    let secs = now_ms().div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_created_once_and_reloaded() {
        let dir = std::env::temp_dir().join(format!("vault-sync-key-{}", crate::random_hex().unwrap()));
        let root = dir.to_string_lossy().into_owned();
        let a = load_or_create(&root).unwrap();
        let b = load_or_create(&root).unwrap();
        assert_eq!(a.public_key(), b.public_key());
        assert_eq!(key_id(&root).unwrap(), a.id().to_string());
        let text = fs::read_to_string(dir.join(".vault").join(FILE)).unwrap();
        assert!(KeyFile::parse(&text).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(".vault").join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn iso_now_is_well_formed() {
        let s = iso_now();
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("20") && s.ends_with('Z'));
    }
}
