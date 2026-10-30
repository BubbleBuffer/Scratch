//! The device side of device keys: a [`DeviceSession`] turns the stored
//! device token into a short-lived session bearer for one hub.
//!
//! One provider is shared per (vault, hub origin, token), so the sync
//! client, the agent and host sockets and the app's HTTP calls hold one
//! session between them. On first need it:
//! - asks `/health` whether the hub knows device keys (a hub without them
//!   gets the bare token, as before);
//! - creates the vault's key (`device_key`) if it has none;
//! - upgrades the token (`/auth/upgrade`) unless `.vault/hub-client.json`
//!   says this token is key-bound already, and remembers that it is;
//! - otherwise proves the key for a session (`/auth/session`).
//!
//! It renews the session shortly before it expires, and after a 401
//! ([`DeviceSession::invalidate`]), for example once the hub restarted.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use vault_core::sha256_hex;

use crate::device_key::{self, DeviceKey};
use crate::keys::origin_of;
use crate::tls::Trust;
use crate::{
    hex_decode, now_ms, Result, SyncError, DEVICE_HEADER, DEVICE_KEYS_CAPABILITY,
    DEVICE_KEYS_CAPABILITY_HEADER,
};

/// The hub's 409 for an upgrade of a token that is key-bound already.
pub(crate) const BOUND_ALREADY: &str = "key-bound already";
/// Largest `/auth/*` answer read.
const MAX_ANSWER: u64 = 64 << 10;
/// Renew this long before the session expires (at most a tenth of its life).
const RENEW_MARGIN: Duration = Duration::from_secs(300);

#[derive(Default)]
struct State {
    /// The hub has no device keys: use the bare token. None until asked.
    legacy: Option<bool>,
    /// The session bearer and when to renew it.
    session: Option<(String, Instant)>,
}

/// A session provider for one vault, hub and device token. `Send + Sync`.
pub struct DeviceSession {
    vault_root: String,
    device: String,
    base: String,
    origin: String,
    token: String,
    http: Mutex<(ureq::Agent, Arc<Trust>)>,
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

type Key = (String, String, String);

fn cache() -> &'static Mutex<HashMap<Key, Arc<DeviceSession>>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Arc<DeviceSession>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn agent(trust: &Arc<Trust>) -> ureq::Agent {
    crate::client::agent(Duration::from_secs(30), trust)
}

impl DeviceSession {
    /// The provider for the vault at `vault_root` (device id `device`),
    /// the hub at `hub_url` and the device `token`, created on first use.
    /// `trust`, when given, replaces the TLS trust of an existing provider
    /// (the sync client's pins win over a caller that has none).
    pub fn shared(
        vault_root: &str,
        device: &str,
        hub_url: &str,
        token: &str,
        trust: Option<Arc<Trust>>,
    ) -> Result<Arc<DeviceSession>> {
        let origin = origin_of(hub_url)
            .ok_or_else(|| SyncError::Http(format!("not an http(s) hub URL: {hub_url:?}")))?;
        let key = (
            vault_root.to_string(),
            origin.clone(),
            sha256_hex(token.as_bytes()),
        );
        let mut c = lock(cache());
        if let Some(s) = c.get(&key) {
            if let Some(trust) = trust {
                *lock(&s.http) = (agent(&trust), trust);
            }
            return Ok(s.clone());
        }
        let trust = trust.unwrap_or_else(|| Trust::new(&[]));
        let s = Arc::new(DeviceSession {
            vault_root: vault_root.to_string(),
            device: device.to_string(),
            base: hub_url.trim().trim_end_matches('/').to_string(),
            origin,
            token: token.to_string(),
            http: Mutex::new((agent(&trust), trust)),
            state: Mutex::new(State::default()),
        });
        c.insert(key, s.clone());
        Ok(s)
    }

    /// The device token this provider stands for.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The bearer for a request: a live session, or the bare token for a
    /// hub without device keys. Handshakes (blocking) when needed.
    pub fn bearer(&self) -> Result<String> {
        let mut st = lock(&self.state);
        if st.legacy == Some(true) {
            return Ok(self.token.clone());
        }
        if let Some((s, renew_at)) = &st.session {
            if Instant::now() < *renew_at {
                return Ok(s.clone());
            }
        }
        st.session = None;
        if st.legacy.is_none() {
            let legacy = !self.hub_has_keys()?;
            st.legacy = Some(legacy);
            if legacy {
                return Ok(self.token.clone());
            }
        }
        let key = device_key::load_or_create(&self.vault_root)?;
        let (session, expires_in) = self.handshake(&key)?;
        let life = Duration::from_secs(expires_in);
        let renew_at = Instant::now() + life - RENEW_MARGIN.min(life / 10);
        st.session = Some((session.clone(), renew_at));
        Ok(session)
    }

    /// What the hub's `/health` said about device keys (the sync client
    /// probes it anyway); a change drops the cached session.
    pub fn note_health(&self, device_keys: bool) {
        let mut st = lock(&self.state);
        if st.legacy != Some(!device_keys) {
            st.legacy = Some(!device_keys);
            st.session = None;
        }
    }

    /// Forgets `bearer` if it is the current session (it got a 401), so
    /// the next [`DeviceSession::bearer`] handshakes again.
    pub fn invalidate(&self, bearer: &str) {
        let mut st = lock(&self.state);
        if st.session.as_ref().is_some_and(|(s, _)| s == bearer) {
            st.session = None;
        }
    }

    /// Whether this vault's token is recorded as key-bound at this hub.
    pub fn is_bound(&self) -> bool {
        ClientState::load(&self.vault_root).is_bound(&self.token)
    }

    fn handshake(&self, key: &DeviceKey) -> Result<(String, u64)> {
        if self.is_bound() {
            match self.prove(key, false) {
                // The hub lost the binding (an older tokens.json, say):
                // upgrade once more.
                Err(SyncError::Status(401, m)) if m.contains("not bound") => {
                    ClientState::set_bound(&self.vault_root, &self.token, false)?;
                }
                r => return r,
            }
        }
        match self.prove(key, true) {
            Ok(r) => {
                ClientState::set_bound(&self.vault_root, &self.token, true)?;
                Ok(r)
            }
            // Bound already (at sign-in or pairing): prove for a session.
            // Other 409s (a copy of the hub's vault) stay errors.
            Err(SyncError::Status(409, m)) if m.contains(BOUND_ALREADY) => {
                ClientState::set_bound(&self.vault_root, &self.token, true)?;
                self.prove(key, false)
            }
            Err(e) => Err(e),
        }
    }

    /// A challenge, then `/auth/upgrade` (`upgrade`) or `/auth/session`.
    fn prove(&self, key: &DeviceKey, upgrade: bool) -> Result<(String, u64)> {
        #[derive(Deserialize)]
        struct Challenge {
            nonce: String,
        }
        #[derive(Deserialize)]
        struct Session {
            session: String,
            expires_in: u64,
        }
        let c: Challenge = self.post("/auth/challenge", b"{}", false)?;
        let nonce: [u8; 32] = hex_decode(&c.nonce)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| SyncError::Protocol("bad challenge nonce".into()))?;
        let proof = device_key::session_proof(key, &self.device, &self.origin, &nonce)?;
        let body = serde_json::to_vec(&proof).map_err(|e| SyncError::Protocol(e.to_string()))?;
        let path = if upgrade { "/auth/upgrade" } else { "/auth/session" };
        let s: Session = self.post(path, &body, upgrade)?;
        Ok((s.session, s.expires_in.max(1)))
    }

    fn hub_has_keys(&self) -> Result<bool> {
        let (agent, trust) = lock(&self.http).clone();
        let generation = trust.generation();
        let mut resp = agent
            .get(&format!("{}/health", self.base))
            .call()
            .map_err(|e| transport_error(&trust, generation, e))?;
        let _ = resp.body_mut().with_config().limit(MAX_ANSWER).read_to_vec();
        if resp.status().as_u16() != 200 {
            return Err(SyncError::Status(resp.status().as_u16(), "hub health check failed".into()));
        }
        Ok(resp
            .headers()
            .get(DEVICE_KEYS_CAPABILITY_HEADER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim() == DEVICE_KEYS_CAPABILITY))
    }

    fn post<T: for<'de> Deserialize<'de>>(&self, path: &str, body: &[u8], with_token: bool) -> Result<T> {
        let (agent, trust) = lock(&self.http).clone();
        let generation = trust.generation();
        let mut rb = agent
            .post(&format!("{}{path}", self.base))
            .header("Content-Type", "application/json")
            .header(DEVICE_HEADER, &self.device);
        if with_token {
            rb = rb.header("Authorization", &format!("Bearer {}", self.token));
        }
        let mut resp = rb
            .send(body)
            .map_err(|e| transport_error(&trust, generation, e))?;
        let status = resp.status().as_u16();
        let raw = resp
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_vec()
            .map_err(|e| SyncError::Http(e.to_string()))?;
        if status != 200 {
            return Err(SyncError::Status(status, String::from_utf8_lossy(&raw).into_owned()));
        }
        serde_json::from_slice(&raw).map_err(|e| SyncError::Protocol(format!("{path}: {e}")))
    }
}

/// Redeems a one-time pairing `code` (from a `notes://pair?hub=…&code=…`
/// link) at the hub at `hub_url` for the vault at `vault_root` (device id
/// `device`): sends the vault's device key (created if needed) to
/// `POST /auth/pair` and returns the new device token and its label. The
/// token is key-bound from the start, and recorded so.
pub fn redeem_pairing_code(
    vault_root: &str,
    device: &str,
    hub_url: &str,
    code: &str,
    trust: Arc<Trust>,
) -> Result<(String, String)> {
    #[derive(Deserialize)]
    struct Paired {
        token: String,
        label: String,
    }
    let key = device_key::load_or_create(vault_root)?;
    let body = serde_json::json!({
        "code": code.trim(),
        "device": device,
        "pubkey": crate::hex_encode(&key.public_key()),
        "key_id": key.id().to_string(),
    });
    let base = hub_url.trim().trim_end_matches('/');
    let agent = agent(&trust);
    let generation = trust.generation();
    let mut resp = agent
        .post(&format!("{base}/auth/pair"))
        .header("Content-Type", "application/json")
        .header(DEVICE_HEADER, device)
        .send(body.to_string().as_bytes())
        .map_err(|e| transport_error(&trust, generation, e))?;
    let status = resp.status().as_u16();
    let raw = resp
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|e| SyncError::Http(e.to_string()))?;
    if status != 200 {
        return Err(SyncError::Status(status, String::from_utf8_lossy(&raw).into_owned()));
    }
    let p: Paired =
        serde_json::from_slice(&raw).map_err(|e| SyncError::Protocol(format!("/auth/pair: {e}")))?;
    ClientState::set_bound(vault_root, &p.token, true)?;
    Ok((p.token, p.label))
}

fn transport_error(trust: &Trust, generation: u64, e: ureq::Error) -> SyncError {
    match trust.untrusted_since(generation) {
        Some(sha256) => SyncError::UntrustedCertificate { sha256 },
        None => SyncError::Http(e.to_string()),
    }
}

/// `.vault/hub-client.json`: the device tokens (by SHA-256) this vault has
/// bound its key to, so the next start proves the key instead of upgrading.
#[derive(Default, Serialize, Deserialize)]
struct ClientState {
    #[serde(default)]
    bound: Vec<String>,
}

static STATE_FILE: Mutex<()> = Mutex::new(());

impl ClientState {
    fn path(root: &str) -> PathBuf {
        PathBuf::from(root).join(".vault").join("hub-client.json")
    }

    fn load(root: &str) -> ClientState {
        fs::read(Self::path(root))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn is_bound(&self, token: &str) -> bool {
        let h = sha256_hex(token.as_bytes());
        self.bound.iter().any(|x| *x == h)
    }

    fn set_bound(root: &str, token: &str, bound: bool) -> Result<()> {
        let _guard = lock(&STATE_FILE);
        let mut st = Self::load(root);
        let h = sha256_hex(token.as_bytes());
        st.bound.retain(|x| *x != h);
        if bound {
            st.bound.push(h);
        }
        let path = Self::path(root);
        let dir = path.parent().map(PathBuf::from).unwrap_or_default();
        fs::create_dir_all(&dir)?;
        let tmp = dir.join(format!(".hub-client.json.{}.{}.tmp", std::process::id(), now_ms()));
        let json = serde_json::to_vec_pretty(&st).map_err(|e| SyncError::Io(e.to_string()))?;
        let written = (|| -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, &path)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written.map_err(|e| SyncError::Io(format!("{}: {e}", path.display())))
    }
}
