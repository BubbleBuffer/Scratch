//! Device keys on the hub: challenges, session proofs, upgrades, pairing
//! codes and the bearer rules (wiki/architecture/device-keys.md). These go
//! through `Hub::handle` directly; `device_key_client.rs` covers a real
//! `SyncClient` over HTTP.

mod support;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use support::TempDir;
use vault_sync::device_key::{session_proof, DeviceKey};
use vault_sync::{Hub, HubRequest, KeyPolicy};

const HOST: &str = "127.0.0.1:8787";
const ORIGIN: &str = "http://127.0.0.1:8787";
const DEVICE: &str = "device-a";

struct Answer {
    status: u16,
    json: serde_json::Value,
}

fn call(hub: &Hub, method: &str, path: &str, bearer: Option<&str>, device: &str, body: &[u8]) -> Answer {
    let mut headers = vec![
        ("Host".to_string(), HOST.to_string()),
        ("X-Device".to_string(), device.to_string()),
    ];
    if let Some(b) = bearer {
        headers.push(("Authorization".into(), format!("Bearer {b}")));
    }
    let r = hub.handle(HubRequest {
        method: method.into(),
        path: path.into(),
        query: String::new(),
        headers,
        body: Box::new(Cursor::new(body.to_vec())),
        body_len: Some(body.len() as u64),
        cancel: None,
    });
    let json = serde_json::from_slice(&r.body).unwrap_or(serde_json::Value::Null);
    Answer { status: r.status, json }
}

fn hub(tag: &str) -> (TempDir, Arc<Hub>) {
    let dir = TempDir::new(tag);
    let hub = Hub::new(dir.open()).unwrap();
    (dir, hub)
}

fn nonce(hub: &Hub) -> [u8; 32] {
    let a = call(hub, "POST", "/auth/challenge", None, DEVICE, b"{}");
    assert_eq!(a.status, 200);
    let hex = a.json["nonce"].as_str().unwrap();
    let bytes: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect();
    bytes.try_into().unwrap()
}

/// A proof by `key` for `device` at `origin` over `nonce`, as JSON.
fn proof(key: &DeviceKey, device: &str, origin: &str, nonce: &[u8; 32]) -> Vec<u8> {
    serde_json::to_vec(&session_proof(key, device, origin, nonce).unwrap()).unwrap()
}

/// `token` upgraded to `key`; returns the session.
fn upgrade(hub: &Hub, token: &str, key: &DeviceKey) -> String {
    let n = nonce(hub);
    let a = call(hub, "POST", "/auth/upgrade", Some(token), DEVICE, &proof(key, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 200, "{:?}", a.json);
    a.json["session"].as_str().unwrap().to_string()
}

fn session(hub: &Hub, key: &DeviceKey) -> Answer {
    let n = nonce(hub);
    call(hub, "POST", "/auth/session", None, DEVICE, &proof(key, DEVICE, ORIGIN, &n))
}

fn changes(hub: &Hub, bearer: &str) -> u16 {
    call(hub, "GET", "/changes", Some(bearer), DEVICE, b"").status
}

#[test]
fn health_advertises_device_keys() {
    let (_d, hub) = hub("dk_health");
    let r = hub.handle(HubRequest {
        method: "GET".into(),
        path: "/health".into(),
        query: String::new(),
        headers: vec![],
        body: Box::new(Cursor::new(Vec::new())),
        body_len: Some(0),
        cancel: None,
    });
    assert!(r.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("X-Device-Keys") && v == "1"));
}

#[test]
fn a_valid_proof_gets_a_session_that_authorizes_requests() {
    let (_d, hub) = hub("dk_valid");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    let s = upgrade(&hub, &token, &key);
    assert_eq!(changes(&hub, &s), 200);
    let a = session(&hub, &key);
    assert_eq!(a.status, 200, "{:?}", a.json);
    assert_eq!(a.json["expires_in"].as_u64(), Some(3600));
    assert_eq!(changes(&hub, a.json["session"].as_str().unwrap()), 200);
    // The session is this device's only.
    assert_eq!(call(&hub, "GET", "/changes", Some(&s), "device-b", b"").status, 403);
}

#[test]
fn a_replayed_nonce_is_rejected() {
    let (_d, hub) = hub("dk_replay");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    let n = nonce(&hub);
    let body = proof(&key, DEVICE, ORIGIN, &n);
    assert_eq!(call(&hub, "POST", "/auth/session", None, DEVICE, &body).status, 200);
    assert_eq!(call(&hub, "POST", "/auth/session", None, DEVICE, &body).status, 401);
}

#[test]
fn an_expired_nonce_is_rejected() {
    let (_d, hub) = hub("dk_expired_nonce");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    hub.set_auth_ttls(Duration::from_millis(20), Duration::from_secs(3600));
    let n = nonce(&hub);
    std::thread::sleep(Duration::from_millis(60));
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 401);
}

#[test]
fn a_nonce_the_hub_never_issued_is_rejected() {
    let (_d, hub) = hub("dk_forged_nonce");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, ORIGIN, &[7; 32]));
    assert_eq!(a.status, 401);
}

#[test]
fn a_wrong_key_is_rejected() {
    let (_d, hub) = hub("dk_wrong_key");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    // Another key, validly signed: not the one bound to this device.
    let other = DeviceKey::generate().unwrap();
    assert_eq!(session(&hub, &other).status, 401);
    // The bound key's id and public key with another key's signature.
    let n = nonce(&hub);
    let mut p = vault_sync::device_key::session_proof(&key, DEVICE, ORIGIN, &n).unwrap();
    p.sig = vault_sync::device_key::session_proof(&other, DEVICE, ORIGIN, &n).unwrap().sig;
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &serde_json::to_vec(&p).unwrap());
    assert_eq!(a.status, 401);
}

#[test]
fn a_wrong_origin_is_rejected() {
    let (_d, hub) = hub("dk_wrong_origin");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    let n = nonce(&hub);
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, "http://evil.example", &n));
    assert_eq!(a.status, 401);
    // With a public URL, only its origin counts, whatever the Host says.
    hub.set_key_policy(KeyPolicy {
        public_url: Some("https://notes.example.org/".into()),
        require_device_keys: false,
    });
    assert_eq!(session(&hub, &key).status, 401);
    let n = nonce(&hub);
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, "https://notes.example.org", &n));
    assert_eq!(a.status, 200, "{:?}", a.json);
}

#[test]
fn a_bare_token_is_rejected_after_upgrade() {
    let (_d, hub) = hub("dk_bare_after_upgrade");
    let token = hub.pair_legacy_token("laptop").unwrap();
    assert_eq!(changes(&hub, &token), 200);
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    assert_eq!(changes(&hub, &token), 401);
    // A second upgrade is refused: the entry is bound already.
    let n = nonce(&hub);
    let a = call(&hub, "POST", "/auth/upgrade", Some(&token), DEVICE, &proof(&key, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 409);
    // Signing out still takes the bare token, and ends the sessions.
    let s = session(&hub, &key).json["session"].as_str().unwrap().to_string();
    assert!(hub.sign_out(&token).unwrap());
    assert_eq!(changes(&hub, &s), 401);
}

#[test]
fn an_unbound_token_is_rejected_when_device_keys_are_required() {
    let (_d, hub) = hub("dk_required");
    let token = hub.pair_legacy_token("laptop").unwrap();
    assert_eq!(changes(&hub, &token), 200);
    hub.set_key_policy(KeyPolicy {
        public_url: None,
        require_device_keys: true,
    });
    assert_eq!(changes(&hub, &token), 401);
    // Nor can it upgrade: the device signs in or pairs again.
    let key = DeviceKey::generate().unwrap();
    let n = nonce(&hub);
    let a = call(&hub, "POST", "/auth/upgrade", Some(&token), DEVICE, &proof(&key, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 401);
}

#[test]
fn an_expired_session_is_rejected() {
    let (_d, hub) = hub("dk_expired_session");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    hub.set_auth_ttls(Duration::from_secs(60), Duration::from_millis(30));
    let s = upgrade(&hub, &token, &key);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(changes(&hub, &s), 401);
}

#[test]
fn unpair_ends_live_sessions() {
    let (_d, hub) = hub("dk_unpair");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    let s = upgrade(&hub, &token, &key);
    assert_eq!(changes(&hub, &s), 200);
    assert_eq!(hub.unpair("laptop").unwrap(), 1);
    assert_eq!(changes(&hub, &s), 401);
    assert_eq!(session(&hub, &key).status, 401);
}

#[test]
fn a_hub_restart_drops_sessions() {
    let (_d, hub) = hub("dk_restart");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    let s = upgrade(&hub, &token, &key);
    hub.drop_sessions();
    assert_eq!(changes(&hub, &s), 401);
    let a = session(&hub, &key);
    assert_eq!(a.status, 200);
}

fn redeem(hub: &Hub, code: &str, key: &DeviceKey) -> Answer {
    let body = serde_json::json!({
        "code": code,
        "device": DEVICE,
        "pubkey": session_proof(key, DEVICE, ORIGIN, &[0; 32]).unwrap().pubkey,
        "key_id": key.id().to_string(),
    });
    call(hub, "POST", "/auth/pair", None, DEVICE, body.to_string().as_bytes())
}

#[test]
fn a_pairing_code_is_single_use_and_gives_a_key_bound_token() {
    let (_d, hub) = hub("dk_pair_code");
    let code = hub.pair("phone").unwrap();
    // The code is no bearer.
    assert_eq!(changes(&hub, &code), 401);
    assert!(hub.devices().iter().any(|d| d.label == "phone"));
    let key = DeviceKey::generate().unwrap();
    let a = redeem(&hub, &code, &key);
    assert_eq!(a.status, 200, "{:?}", a.json);
    assert_eq!(a.json["label"].as_str(), Some("phone"));
    let token = a.json["token"].as_str().unwrap().to_string();
    assert_ne!(token, code);
    // Key-bound from the start: the bare token is refused, a session works.
    assert_eq!(changes(&hub, &token), 401);
    let s = session(&hub, &key);
    assert_eq!(s.status, 200, "{:?}", s.json);
    assert_eq!(changes(&hub, s.json["session"].as_str().unwrap()), 200);
    // Used once.
    assert_eq!(redeem(&hub, &code, &DeviceKey::generate().unwrap()).status, 400);
}

#[test]
fn a_pairing_code_expires() {
    let (_d, hub) = hub("dk_pair_expiry");
    hub.set_pair_code_ttl(Duration::from_millis(30));
    let code = hub.pair("phone").unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(redeem(&hub, &code, &DeviceKey::generate().unwrap()).status, 400);
    assert!(!hub.devices().iter().any(|d| d.label == "phone"));
}

#[test]
fn a_mismatched_key_id_is_rejected() {
    let (_d, hub) = hub("dk_key_id");
    let code = hub.pair("phone").unwrap();
    let key = DeviceKey::generate().unwrap();
    let other = DeviceKey::generate().unwrap();
    let body = serde_json::json!({
        "code": code,
        "device": DEVICE,
        "pubkey": session_proof(&key, DEVICE, ORIGIN, &[0; 32]).unwrap().pubkey,
        "key_id": other.id().to_string(),
    });
    let a = call(&hub, "POST", "/auth/pair", None, DEVICE, body.to_string().as_bytes());
    assert_eq!(a.status, 400);
}

#[test]
fn a_tampered_nonce_is_rejected() {
    let (_d, hub) = hub("dk_tampered_nonce");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    // A real nonce with its issue time moved forward (to stretch its
    // life), and one with a changed random part: the tag no longer fits.
    let mut later = nonce(&hub);
    later[23] = later[23].wrapping_add(1);
    let mut other = nonce(&hub);
    other[0] ^= 0xff;
    for n in [later, other] {
        let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, ORIGIN, &n));
        assert_eq!(a.status, 401);
    }
    assert_eq!(session(&hub, &key).status, 200);
}

#[test]
fn many_open_challenges_never_lock_anyone_out() {
    let (_d, hub) = hub("dk_many_challenges");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    for _ in 0..10_000 {
        assert_eq!(call(&hub, "POST", "/auth/challenge", None, DEVICE, b"{}").status, 200);
    }
    assert_eq!(session(&hub, &key).status, 200);
}

#[test]
fn a_failed_proof_does_not_use_up_its_nonce() {
    let (_d, hub) = hub("dk_failed_proof_nonce");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let key = DeviceKey::generate().unwrap();
    upgrade(&hub, &token, &key);
    // A key nobody bound signs a live nonce: refused, and the nonce stays
    // unused (only authorized proofs are recorded).
    let n = nonce(&hub);
    let stranger = DeviceKey::generate().unwrap();
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&stranger, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 401);
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 200, "{:?}", a.json);
    let a = call(&hub, "POST", "/auth/session", None, DEVICE, &proof(&key, DEVICE, ORIGIN, &n));
    assert_eq!(a.status, 401, "a replay after a valid use");
}

#[test]
fn re_pairing_a_device_with_a_new_key_ends_its_old_sessions() {
    let (dir, hub) = hub("dk_repair_new_key");
    let token = hub.pair_legacy_token("laptop").unwrap();
    let old_key = DeviceKey::generate().unwrap();
    let old = upgrade(&hub, &token, &old_key);
    assert_eq!(changes(&hub, &old), 200);

    // Another process (the CLI) unpairs the device and pairs it again;
    // the device redeems the new code with a new key. The running hub
    // never sees the unpair itself.
    let cli = Hub::new(dir.open()).unwrap();
    assert_eq!(cli.unpair("laptop").unwrap(), 1);
    let code = cli.pair("laptop").unwrap();
    let new_key = DeviceKey::generate().unwrap();
    assert_eq!(redeem(&cli, &code, &new_key).status, 200);

    assert_eq!(changes(&hub, &old), 401, "a session of the old key");
    let s = session(&hub, &new_key);
    assert_eq!(s.status, 200, "{:?}", s.json);
    assert_eq!(changes(&hub, s.json["session"].as_str().unwrap()), 200);
}

