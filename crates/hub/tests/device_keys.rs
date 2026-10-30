//! Device keys on notes-hub: a key-bound token is no bearer by itself, and
//! the session its key proves is. Covers the OIDC redeem with a key, the
//! bare-token refusals on `/changes`, `/shares`, `/agent-tokens`, `/agent/ws`
//! and `/mcp`, signout, and `require_device_keys`. The fake OIDC issuer and
//! the login flow are copied from `login.rs`; the proofs come from
//! `vault_sync::device_key`, and the client handshake from `DeviceSession`.

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use notes_hub::{Config, OidcConfig, Server};
use openidconnect::core::{
    CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreRsaPrivateSigningKey,
};
use openidconnect::{
    AdditionalClaims, Audience, EndUserEmail, IdToken, IdTokenClaims, IssuerUrl, JsonWebKeyId,
    Nonce, PrivateSigningKey, StandardClaims, SubjectIdentifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use url::Url;
use vault_core::{new_id, Vault};
use vault_sync::device_key::{session_proof, DeviceKey};
use vault_sync::{origin_of, DeviceSession};

const CLIENT_ID: &str = "notes";
const CLIENT_SECRET: &str = "test-secret";
const APP_REDIRECT: &str = "http://127.0.0.1:53111/done";
/// The device id the bound tokens below are used with.
const DEVICE: &str = "device-a";

// ---- temp dirs ----

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_dk_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.0.to_string_lossy().into_owned()).expect("open"))
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- the fake OIDC issuer (as in login.rs; a confidential client only) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Groups {
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<String>>,
}
impl AdditionalClaims for Groups {}

struct Grant {
    nonce: String,
    challenge: String,
    redirect_uri: String,
}

struct Issuer {
    url: String,
    key: CoreRsaPrivateSigningKey,
    grants: Mutex<HashMap<String, Grant>>,
}

fn test_key_pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
        let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string()
    })
}

async fn discovery(State(i): State<Arc<Issuer>>) -> Json<Value> {
    Json(json!({
        "issuer": i.url,
        "authorization_endpoint": format!("{}/authorize", i.url),
        "token_endpoint": format!("{}/token", i.url),
        "jwks_uri": format!("{}/jwks", i.url),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn jwks(State(i): State<Arc<Issuer>>) -> Json<Value> {
    Json(json!({ "keys": [i.key.as_verification_key()] }))
}

async fn authorize(
    State(i): State<Arc<Issuer>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["code_challenge_method"], "S256");
    let code = new_id();
    i.grants.lock().unwrap().insert(
        code.clone(),
        Grant {
            nonce: q["nonce"].clone(),
            challenge: q["code_challenge"].clone(),
            redirect_uri: q["redirect_uri"].clone(),
        },
    );
    let mut to = Url::parse(&q["redirect_uri"]).unwrap();
    to.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &q["state"]);
    (StatusCode::FOUND, [(header::LOCATION, to.to_string())]).into_response()
}

async fn token(
    State(i): State<Arc<Issuer>>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Basic {basic}"));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_client"})),
        )
            .into_response();
    }
    let bad = || {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response()
    };
    let code = f.get("code").cloned().unwrap_or_default();
    let Some(g) = i.grants.lock().unwrap().remove(&code) else {
        return bad();
    };
    let verifier = f.get("code_verifier").cloned().unwrap_or_default();
    let verifier_hash =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    if f.get("grant_type").map(String::as_str) != Some("authorization_code")
        || verifier_hash != g.challenge
        || f.get("redirect_uri") != Some(&g.redirect_uri)
    {
        return bad();
    }
    let now = chrono::Utc::now();
    let claims = IdTokenClaims::new(
        IssuerUrl::new(i.url.clone()).unwrap(),
        vec![Audience::new(CLIENT_ID.into())],
        now + chrono::Duration::minutes(5),
        now - chrono::Duration::minutes(10),
        StandardClaims::new(SubjectIdentifier::new("ann-sub".into()))
            .set_email(Some(EndUserEmail::new("ann@example.com".into()))),
        Groups {
            groups: Some(Vec::new()),
        },
    )
    .set_nonce(Some(Nonce::new(g.nonce.clone())));
    let id_token = IdToken::<
        Groups,
        CoreGenderClaim,
        CoreJweContentEncryptionAlgorithm,
        CoreJwsSigningAlgorithm,
    >::new(
        claims,
        &i.key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .unwrap();
    Json(json!({
        "access_token": "at",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token.to_string(),
    }))
    .into_response()
}

async fn start_issuer() -> Arc<Issuer> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let issuer = Arc::new(Issuer {
        url: format!("http://{addr}"),
        key: CoreRsaPrivateSigningKey::from_pem(
            test_key_pem(),
            Some(JsonWebKeyId::new("test".into())),
        )
        .unwrap(),
        grants: Mutex::new(HashMap::new()),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(issuer.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    issuer
}

// ---- the hub ----

struct Env {
    _dirs: Vec<TempDir>,
    issuer: Arc<Issuer>,
    server: Server,
    hub_url: String,
    http: reqwest::Client,
}

async fn env() -> Env {
    env_inner(|_| {}).await
}

/// The hub with `require_device_keys` on, and the rest as in `env`.
async fn env_requiring_keys() -> Env {
    env_inner(|c| c.require_device_keys = true).await
}

async fn env_inner(setup: impl FnOnce(&mut Config)) -> Env {
    let issuer = start_issuer().await;
    let dir = TempDir::new("data");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let hub_url = format!("http://{addr}");
    let mut config = Config {
        listen: addr.to_string(),
        data_dir: dir.0.clone(),
        public_url: hub_url.clone(),
        trust_proxy: false,
        oidc: OidcConfig {
            issuer: issuer.url.clone(),
            client_id: CLIENT_ID.into(),
            client_secret: CLIENT_SECRET.into(),
            allowed_users: vec!["ANN@example.com".into(), "bob-sub".into()],
            allowed_groups: vec!["notes-users".into()],
            scopes: None,
        },
        agent: notes_hub::AgentSection {
            enabled: false,
            ..notes_hub::AgentSection::default()
        },
        ..Config::default()
    };
    setup(&mut config);
    let server = notes_hub::start_on(listener, config).await.unwrap();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Env {
        _dirs: vec![dir],
        issuer,
        server,
        hub_url,
        http,
    }
}

impl Env {
    fn origin(&self) -> String {
        origin_of(&self.hub_url).unwrap()
    }

    async fn location(&self, url: &str) -> Url {
        let r = self.http.get(url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FOUND, "GET {url}");
        let loc = r.headers()[header::LOCATION].to_str().unwrap();
        Url::parse(loc).unwrap()
    }

    /// The browser part of a login; returns where the hub sent the browser
    /// back to the app (with the one-time code).
    async fn login(&self, device: &str, name: &str) -> Url {
        let mut start = Url::parse(&format!("{}/auth/start", self.hub_url)).unwrap();
        start
            .query_pairs_mut()
            .append_pair("device", device)
            .append_pair("name", name)
            .append_pair("redirect", APP_REDIRECT)
            .append_pair("state", "app-state-1");
        let authorize = self.location(start.as_str()).await;
        assert!(authorize.as_str().starts_with(&self.issuer.url));
        let callback = self.location(authorize.as_str()).await;
        assert!(callback
            .as_str()
            .starts_with(&format!("{}/auth/callback", self.hub_url)));
        let back = self.location(callback.as_str()).await;
        assert!(back.as_str().starts_with(APP_REDIRECT), "{back}");
        back
    }

    /// `/auth/redeem` without a key: the token is not key-bound.
    async fn redeem(&self, code: &str, device: &str) -> (StatusCode, Value) {
        let r = self
            .http
            .post(format!("{}/auth/redeem", self.hub_url))
            .json(&json!({ "code": code, "device": device }))
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    /// `/auth/redeem` with the device's public key: the token comes key-bound.
    async fn redeem_with_key(&self, code: &str, device: &str, key: &DeviceKey) -> (StatusCode, Value) {
        let r = self
            .http
            .post(format!("{}/auth/redeem", self.hub_url))
            .json(&json!({
                "code": code,
                "device": device,
                "pubkey": hex(&key.public_key()),
                "key_id": key.id().to_string(),
            }))
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    /// A full login without a key; returns the (unbound) token.
    async fn token(&self, device: &str, name: &str) -> String {
        let back = self.login(device, name).await;
        let code = param(&back, "code").expect("a code");
        let (status, body) = self.redeem(&code, device).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["token"].as_str().unwrap().to_string()
    }

    /// `/auth/challenge`: a fresh nonce.
    async fn nonce(&self) -> [u8; 32] {
        let r = self
            .http
            .post(format!("{}/auth/challenge", self.hub_url))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body: Value = r.json().await.unwrap();
        let hex = body["nonce"].as_str().unwrap();
        let bytes: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        bytes.try_into().unwrap()
    }

    /// `/auth/upgrade` (with `bearer`) or `/auth/session` (without) with a
    /// proof by `key` for `device`.
    async fn prove(
        &self,
        path: &str,
        bearer: Option<&str>,
        device: &str,
        key: &DeviceKey,
    ) -> (StatusCode, Value) {
        let n = self.nonce().await;
        let proof = session_proof(key, device, &self.origin(), &n).unwrap();
        let mut req = self
            .http
            .post(format!("{}{path}", self.hub_url))
            .header("X-Device", device)
            .json(&proof);
        if let Some(b) = bearer {
            req = req.bearer_auth(b);
        }
        let r = req.send().await.unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    /// `GET path` with `bearer` and `X-Device`: the status.
    async fn get_status(&self, path: &str, bearer: &str, device: &str) -> StatusCode {
        self.http
            .get(format!("{}{path}", self.hub_url))
            .bearer_auth(bearer)
            .header("X-Device", device)
            .send()
            .await
            .unwrap()
            .status()
    }

    /// `POST /mcp` (a `tools/list` call) with `bearer` and `X-Device`.
    async fn mcp_status(&self, bearer: &str, device: &str) -> StatusCode {
        self.http
            .post(format!("{}/mcp", self.hub_url))
            .bearer_auth(bearer)
            .header("X-Device", device)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
            .send()
            .await
            .unwrap()
            .status()
    }

    /// `GET /agent/ws` with `bearer` and `X-Device`: the upgrade's status,
    /// or 101 when the socket opened.
    async fn ws_status(&self, bearer: &str, device: &str) -> u16 {
        let url = format!("{}/agent/ws", self.hub_url.replacen("http://", "ws://", 1));
        let mut req = url.into_client_request().unwrap();
        let h = req.headers_mut();
        h.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap(),
        );
        h.insert("X-Device", HeaderValue::from_str(device).unwrap());
        match tokio_tungstenite::connect_async(req).await {
            Ok(_) => 101,
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) => r.status().as_u16(),
            Err(other) => panic!("{other}"),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn param(u: &Url, k: &str) -> Option<String> {
    u.query_pairs()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.into_owned())
}

/// The bearer a device client gets from its real handshake: it upgrades
/// the unbound `token` (`/auth/upgrade`), then asks for a session.
async fn bind(e: &Env, dir: &TempDir, device: &str, token: &str) -> String {
    let (root, device, base, token) = (dir.path(), device.to_string(), e.hub_url.clone(), token.to_string());
    tokio::task::spawn_blocking(move || {
        DeviceSession::shared(&root, &device, &base, &token, None)
            .unwrap()
            .bearer()
            .unwrap()
    })
    .await
    .unwrap()
}

// ---- tests ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redeem_with_a_device_key_gives_a_key_bound_token() {
    let e = env().await;
    let d = TempDir::new("redeem_key");
    let v = d.open();
    let device = v.device_id();
    let key = DeviceKey::generate().unwrap();

    let back = e.login(&device, "phone").await;
    let code = param(&back, "code").expect("a code");
    let (status, body) = e.redeem_with_key(&code, &device, &key).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bare = body["token"].as_str().unwrap().to_string();

    // Key-bound: the bare token is no bearer on a device route.
    assert_eq!(
        e.get_status("/changes", &bare, &device).await,
        StatusCode::UNAUTHORIZED
    );
    // The session the key proves is.
    let (status, body) = e.prove("/auth/session", None, &device, &key).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session = body["session"].as_str().unwrap().to_string();
    assert_eq!(e.get_status("/changes", &session, &device).await, StatusCode::OK);

    // Without a key the token stays usable bare, as before.
    let other = TempDir::new("redeem_plain");
    let ov = other.open();
    let device_b = ov.device_id();
    let back = e.login(&device_b, "tablet").await;
    let code = param(&back, "code").expect("a code");
    let (status, body) = e.redeem(&code, &device_b).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plain = body["token"].as_str().unwrap().to_string();
    assert_eq!(
        e.get_status("/changes", &plain, &device_b).await,
        StatusCode::OK
    );
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bound_tokens_bare_bearer_is_refused_on_shares_and_agent_tokens() {
    let e = env().await;
    let token = e.server.hub().pair_legacy_token("laptop").unwrap();
    let d = TempDir::new("bind_shares");
    let _v = d.open();
    let session = bind(&e, &d, DEVICE, &token).await;

    assert_eq!(
        e.get_status("/shares?all=1", &token, DEVICE).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        e.get_status("/agent-tokens", &token, DEVICE).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        e.get_status("/shares?all=1", &session, DEVICE).await,
        StatusCode::OK
    );
    assert_eq!(
        e.get_status("/agent-tokens", &session, DEVICE).await,
        StatusCode::OK
    );
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_ws_accepts_a_session_and_refuses_a_bound_bare_token() {
    let e = env().await;
    let token = e.server.hub().pair_legacy_token("laptop").unwrap();
    let d = TempDir::new("bind_ws");
    let _v = d.open();
    let session = bind(&e, &d, DEVICE, &token).await;

    // The check runs before the agent is opened; the agent is off in this
    // test, so a session that passes the check gets 503, not 401/403.
    let s = e.ws_status(&session, DEVICE).await;
    assert!(s != 401 && s != 403, "the session was refused: {s}");
    assert_eq!(e.ws_status(&token, DEVICE).await, 401);
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_accepts_a_session_and_refuses_a_bound_bare_token() {
    let e = env().await;
    let token = e.server.hub().pair_legacy_token("laptop").unwrap();
    let d = TempDir::new("bind_mcp");
    let _v = d.open();
    let session = bind(&e, &d, DEVICE, &token).await;

    let s = e.mcp_status(&session, DEVICE).await;
    assert!(s.is_success(), "the session was refused: {s}");
    assert_eq!(e.mcp_status(&token, DEVICE).await, StatusCode::UNAUTHORIZED);
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signout_with_the_bare_token_ends_the_sessions() {
    let e = env().await;
    let token = e.server.hub().pair_legacy_token("laptop").unwrap();
    let d = TempDir::new("bind_signout");
    let _v = d.open();
    let session = bind(&e, &d, DEVICE, &token).await;
    assert_eq!(
        e.get_status("/changes", &session, DEVICE).await,
        StatusCode::OK
    );

    let r = e
        .http
        .post(format!("{}/auth/signout", e.hub_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        e.get_status("/changes", &session, DEVICE).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        e.get_status("/changes", &token, DEVICE).await,
        StatusCode::UNAUTHORIZED
    );
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn require_device_keys_refuses_unbound_tokens() {
    let e = env_requiring_keys().await;
    assert!(e.server.hub().key_policy().require_device_keys);

    // A legacy token is refused on a device route, and it cannot be
    // upgraded either.
    let legacy = e.server.hub().pair_legacy_token("laptop").unwrap();
    assert_eq!(
        e.get_status("/changes", &legacy, DEVICE).await,
        StatusCode::UNAUTHORIZED
    );
    let key = DeviceKey::generate().unwrap();
    let (status, body) = e.prove("/auth/upgrade", Some(&legacy), DEVICE, &key).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // A login without a key makes an unbound token: refused too.
    let d = TempDir::new("require_plain");
    let v = d.open();
    let device = v.device_id();
    let unbound = e.token(&device, "phone").await;
    assert_eq!(
        e.get_status("/changes", &unbound, &device).await,
        StatusCode::UNAUTHORIZED
    );

    // A login with a key is bound from the start: its session works.
    let d2 = TempDir::new("require_key");
    let v2 = d2.open();
    let device2 = v2.device_id();
    let key2 = DeviceKey::generate().unwrap();
    let back = e.login(&device2, "tablet").await;
    let code = param(&back, "code").expect("a code");
    let (status, body) = e.redeem_with_key(&code, &device2, &key2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bare = body["token"].as_str().unwrap().to_string();
    assert_eq!(
        e.get_status("/changes", &bare, &device2).await,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.prove("/auth/session", None, &device2, &key2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session = body["session"].as_str().unwrap().to_string();
    assert_eq!(
        e.get_status("/changes", &session, &device2).await,
        StatusCode::OK
    );
    e.server.stop().await;
}
