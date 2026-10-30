//! The hub's in-memory device-key state: challenge nonces (32 random bytes,
//! single use, valid 60 s) and session tokens (32 random bytes, kept as a
//! SHA-256, valid 1 hour). Both tables are bounded and prune expired
//! entries. A hub restart drops them; clients handshake again on 401.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use vault_core::sha256_hex;

use crate::{random_hex, Result, SyncError};

const NONCE_TTL: Duration = Duration::from_secs(60);
const SESSION_TTL: Duration = Duration::from_secs(3600);
/// Outstanding challenges at most (after pruning the expired ones).
const MAX_NONCES: usize = 4096;
/// Live sessions at most.
const MAX_SESSIONS: usize = 65_536;

/// Whether device keys are required, and the origin proofs must name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyPolicy {
    /// The hub's public URL (`https://notes.example.org`). When set, a
    /// proof's origin must be its origin; otherwise it must be
    /// `http://<Host>` or `https://<Host>` of the request.
    pub public_url: Option<String>,
    /// Refuse device tokens that are not key-bound yet (default off).
    pub require_device_keys: bool,
}

struct Session {
    device: String,
    expires: Instant,
}

#[derive(Default)]
struct Tables {
    nonces: HashMap<[u8; 32], Instant>,
    sessions: HashMap<String, Session>,
}

pub(crate) struct KeyAuth {
    tables: Mutex<Tables>,
    ttls: Mutex<(Duration, Duration)>,
    pub(crate) policy: Mutex<KeyPolicy>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl KeyAuth {
    pub fn new() -> KeyAuth {
        KeyAuth {
            tables: Mutex::new(Tables::default()),
            ttls: Mutex::new((NONCE_TTL, SESSION_TTL)),
            policy: Mutex::new(KeyPolicy::default()),
        }
    }

    pub fn policy(&self) -> KeyPolicy {
        lock(&self.policy).clone()
    }

    pub fn set_policy(&self, policy: KeyPolicy) {
        *lock(&self.policy) = policy;
    }

    /// Test hook: the nonce and session lifetimes.
    pub fn set_ttls(&self, nonce: Duration, session: Duration) {
        *lock(&self.ttls) = (nonce, session);
    }

    /// A fresh challenge nonce.
    pub fn challenge(&self) -> Result<[u8; 32]> {
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|e| SyncError::Io(format!("random: {e}")))?;
        let ttl = lock(&self.ttls).0;
        let now = Instant::now();
        let mut t = lock(&self.tables);
        t.nonces.retain(|_, exp| *exp > now);
        if t.nonces.len() >= MAX_NONCES {
            return Err(SyncError::Status(503, "too many open challenges; try again".into()));
        }
        t.nonces.insert(nonce, now + ttl);
        Ok(nonce)
    }

    /// Takes `nonce`: true once, and only before it expires.
    pub fn take_nonce(&self, nonce: &[u8; 32]) -> bool {
        let now = Instant::now();
        let mut t = lock(&self.tables);
        matches!(t.nonces.remove(nonce), Some(exp) if exp > now)
    }

    /// A new session for `device`: the token and its lifetime in seconds.
    pub fn create_session(&self, device: &str) -> Result<(String, u64)> {
        let token = random_hex()?;
        let ttl = lock(&self.ttls).1;
        let now = Instant::now();
        let mut t = lock(&self.tables);
        t.sessions.retain(|_, s| s.expires > now);
        if t.sessions.len() >= MAX_SESSIONS {
            return Err(SyncError::Status(503, "too many sessions; try again".into()));
        }
        t.sessions.insert(
            sha256_hex(token.as_bytes()),
            Session {
                device: device.to_string(),
                expires: now + ttl,
            },
        );
        Ok((token, ttl.as_secs().max(1)))
    }

    /// Checks `token` as a session of `device`. None: it is no live
    /// session (perhaps a device token); Some: the verdict.
    pub fn check_session(&self, token: &str, device: &str) -> Option<Result<()>> {
        let hash = sha256_hex(token.as_bytes());
        let mut t = lock(&self.tables);
        let s = t.sessions.get(&hash)?;
        if s.expires <= Instant::now() {
            t.sessions.remove(&hash);
            return Some(Err(SyncError::Status(401, "session expired".into())));
        }
        if !crate::ct_eq(s.device.as_bytes(), device.as_bytes()) {
            return Some(Err(SyncError::Status(
                403,
                "this session belongs to another device".into(),
            )));
        }
        Some(Ok(()))
    }

    /// Ends every session of `device` (unpair, sign-out, a new sign-in).
    pub fn drop_device(&self, device: &str) {
        lock(&self.tables).sessions.retain(|_, s| s.device != device);
    }

    /// Ends every session (tests: a hub restart).
    pub fn drop_all(&self) {
        let mut t = lock(&self.tables);
        t.sessions.clear();
        t.nonces.clear();
    }
}

/// `scheme://host[:port]` of `url`, lowercase, without a default port
/// (80 for http, 443 for https), a path or a trailing slash. None for a
/// URL that is not http(s).
pub fn origin_of(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    host_origin(&scheme, authority)
}

/// The origin for `scheme` and a `Host` header value.
pub(crate) fn host_origin(scheme: &str, host: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let default = match scheme {
        "http" => ":80",
        "https" => ":443",
        _ => return None,
    };
    let host = host.strip_suffix(default).unwrap_or(&host);
    Some(format!("{scheme}://{host}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_normalized() {
        assert_eq!(origin_of("https://Notes.Example.org/").as_deref(), Some("https://notes.example.org"));
        assert_eq!(origin_of("https://notes.example.org:443/x").as_deref(), Some("https://notes.example.org"));
        assert_eq!(origin_of("http://100.1.2.3:8787").as_deref(), Some("http://100.1.2.3:8787"));
        assert_eq!(origin_of("http://[::1]:80").as_deref(), Some("http://[::1]"));
        assert_eq!(origin_of("ftp://x"), None);
    }

    #[test]
    fn nonces_are_single_use_and_expire() {
        let k = KeyAuth::new();
        let n = k.challenge().unwrap();
        assert!(k.take_nonce(&n));
        assert!(!k.take_nonce(&n));
        k.set_ttls(Duration::from_millis(1), SESSION_TTL);
        let n = k.challenge().unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(!k.take_nonce(&n));
    }

    #[test]
    fn sessions_expire_and_end_with_their_device() {
        let k = KeyAuth::new();
        let (s, ttl) = k.create_session("dev-a").unwrap();
        assert_eq!(ttl, 3600);
        assert!(matches!(k.check_session(&s, "dev-a"), Some(Ok(()))));
        assert!(matches!(k.check_session(&s, "dev-b"), Some(Err(SyncError::Status(403, _)))));
        assert!(k.check_session("other", "dev-a").is_none());
        k.drop_device("dev-a");
        assert!(k.check_session(&s, "dev-a").is_none());
        k.set_ttls(NONCE_TTL, Duration::from_millis(1));
        let (s, _) = k.create_session("dev-a").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(matches!(k.check_session(&s, "dev-a"), Some(Err(SyncError::Status(401, _)))));
    }
}
