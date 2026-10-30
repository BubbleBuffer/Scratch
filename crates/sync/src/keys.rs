//! The hub's in-memory device-key state: challenge nonces (single use,
//! valid 60 s) and session tokens (32 random bytes, kept as a SHA-256,
//! valid 1 hour). A hub restart drops both; clients handshake again on 401.
//!
//! Nonces are stateless, so an unauthenticated caller cannot fill a table:
//! 16 random bytes, the issue time (u64 ms, big-endian) and the first 8
//! bytes of an HMAC-SHA256 over both under a per-process random key. A
//! nonce is recorded as used only once its proof is verified and
//! authorized; that set is bounded and pruned by age.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use vault_core::sha256_hex;

use crate::{random_hex, Result, SyncError};

const NONCE_TTL: Duration = Duration::from_secs(60);
const SESSION_TTL: Duration = Duration::from_secs(3600);
/// Used nonces remembered at most (after pruning the expired ones). Only
/// authorized proofs add to it.
const MAX_USED_NONCES: usize = 65_536;
/// How far in the future an issue time may be (clock steps).
const NONCE_SKEW_MS: u64 = 5_000;
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
    /// The public key (hex) whose proof bought it.
    pubkey: String,
    expires: Instant,
}

#[derive(Default)]
struct Tables {
    /// Used nonces and their issue time (ms).
    used: HashMap<[u8; 32], u64>,
    sessions: HashMap<String, Session>,
}

pub(crate) struct KeyAuth {
    mac_key: [u8; 32],
    tables: Mutex<Tables>,
    ttls: Mutex<(Duration, Duration)>,
    pub(crate) policy: Mutex<KeyPolicy>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl KeyAuth {
    pub fn new() -> KeyAuth {
        let mut mac_key = [0u8; 32];
        // A failed random source leaves a zero key; challenge() then fails
        // on its own random draw, so no nonce is ever issued under it.
        let _ = getrandom::fill(&mut mac_key);
        KeyAuth {
            mac_key,
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

    fn mac(&self) -> Hmac<Sha256> {
        Hmac::<Sha256>::new_from_slice(&self.mac_key).expect("HMAC takes any key length")
    }

    /// A fresh challenge nonce. Stores nothing.
    pub fn challenge(&self) -> Result<[u8; 32]> {
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce[..16]).map_err(|e| SyncError::Io(format!("random: {e}")))?;
        nonce[16..24].copy_from_slice(&now_ms().to_be_bytes());
        let mut mac = self.mac();
        mac.update(&nonce[..24]);
        nonce[24..].copy_from_slice(&mac.finalize().into_bytes()[..8]);
        Ok(nonce)
    }

    /// Whether `nonce` is one this process issued less than the nonce
    /// lifetime ago (its tag and age; not whether it was used).
    pub fn nonce_is_live(&self, nonce: &[u8; 32]) -> bool {
        let mut mac = self.mac();
        mac.update(&nonce[..24]);
        if mac.verify_truncated_left(&nonce[24..]).is_err() {
            return false;
        }
        let issued = issued_ms(nonce);
        let ttl = lock(&self.ttls).0.as_millis() as u64;
        let now = now_ms();
        issued <= now + NONCE_SKEW_MS && now.saturating_sub(issued) < ttl
    }

    /// Records `nonce` as used, once its proof is verified and authorized:
    /// false if it was used already (a replay).
    pub fn use_nonce(&self, nonce: &[u8; 32]) -> Result<bool> {
        let ttl = lock(&self.ttls).0.as_millis() as u64;
        let now = now_ms();
        let mut t = lock(&self.tables);
        if t.used.contains_key(nonce) {
            return Ok(false);
        }
        t.used.retain(|_, issued| now.saturating_sub(*issued) < ttl + NONCE_SKEW_MS);
        if t.used.len() >= MAX_USED_NONCES {
            return Err(SyncError::Status(503, "too many sign-ins at once; try again".into()));
        }
        t.used.insert(*nonce, issued_ms(nonce));
        Ok(true)
    }

    /// A new session for `device`, bought with the key `pubkey` (hex): the
    /// token and its lifetime in seconds.
    pub fn create_session(&self, device: &str, pubkey: &str) -> Result<(String, u64)> {
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
                pubkey: pubkey.to_string(),
                expires: now + ttl,
            },
        );
        Ok((token, ttl.as_secs().max(1)))
    }

    /// Checks `token` as a session of `device`. None: it is no live
    /// session (perhaps a device token); Some: the verdict, with the public
    /// key (hex) the session was bought with.
    pub fn check_session(&self, token: &str, device: &str) -> Option<Result<String>> {
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
        Some(Ok(s.pubkey.clone()))
    }

    /// Ends every session of `device` (unpair, sign-out, a new sign-in).
    pub fn drop_device(&self, device: &str) {
        lock(&self.tables).sessions.retain(|_, s| s.device != device);
    }

    /// Ends every session (tests: a hub restart).
    pub fn drop_all(&self) {
        let mut t = lock(&self.tables);
        t.sessions.clear();
        t.used.clear();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn issued_ms(nonce: &[u8; 32]) -> u64 {
    u64::from_be_bytes(nonce[16..24].try_into().expect("8 bytes"))
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
        assert!(k.nonce_is_live(&n));
        assert!(k.use_nonce(&n).unwrap());
        assert!(!k.use_nonce(&n).unwrap(), "a replay");
        k.set_ttls(Duration::from_millis(1), SESSION_TTL);
        let n = k.challenge().unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(!k.nonce_is_live(&n));
    }

    #[test]
    fn forged_nonces_are_not_live() {
        let k = KeyAuth::new();
        let mut n = k.challenge().unwrap();
        n[0] ^= 1;
        assert!(!k.nonce_is_live(&n), "a changed random part");
        let mut n = k.challenge().unwrap();
        let moved = (issued_ms(&n) + 1).to_be_bytes();
        n[16..24].copy_from_slice(&moved);
        assert!(!k.nonce_is_live(&n), "a moved issue time");
        assert!(!k.nonce_is_live(&[0u8; 32]));
        let other = KeyAuth::new().challenge().unwrap();
        assert!(!k.nonce_is_live(&other), "another process's nonce");
    }

    #[test]
    fn challenges_store_nothing_and_never_run_out() {
        let k = KeyAuth::new();
        for _ in 0..20_000 {
            k.challenge().unwrap();
        }
        assert!(lock(&k.tables).used.is_empty());
    }

    #[test]
    fn sessions_expire_and_end_with_their_device() {
        let k = KeyAuth::new();
        let (s, ttl) = k.create_session("dev-a", "ab").unwrap();
        assert_eq!(ttl, 3600);
        assert!(matches!(k.check_session(&s, "dev-a"), Some(Ok(p)) if p == "ab"));
        assert!(matches!(k.check_session(&s, "dev-b"), Some(Err(SyncError::Status(403, _)))));
        assert!(k.check_session("other", "dev-a").is_none());
        k.drop_device("dev-a");
        assert!(k.check_session(&s, "dev-a").is_none());
        k.set_ttls(NONCE_TTL, Duration::from_millis(1));
        let (s, _) = k.create_session("dev-a", "ab").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(matches!(k.check_session(&s, "dev-a"), Some(Err(SyncError::Status(401, _)))));
    }
}
