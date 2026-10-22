//! notes-hub end to end in one process: a mock OIDC issuer (discovery,
//! JWKS, authorize, token; RS256 ID tokens signed with a test key), the
//! hub on axum, and `SyncClient`s using the tokens a login gives them.

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

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
use url::Url;
use vault_core::{new_id, sha256_hex, Vault};
use vault_sync::{SyncClient, SyncError};

const CLIENT_ID: &str = "notes";
const CLIENT_SECRET: &str = "test-secret";

// ---- temp dirs ----

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.0.to_string_lossy().into_owned()).expect("open"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- the mock issuer ----

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Groups {
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<String>>,
}
impl AdditionalClaims for Groups {}

/// Who signs in next, and how the next ID token is spoiled.
#[derive(Clone)]
struct Knobs {
    sub: String,
    email: String,
    groups: Vec<String>,
    wrong_nonce: bool,
    wrong_aud: bool,
    expired: bool,
}

impl Knobs {
    fn user(sub: &str, email: &str, groups: &[&str]) -> Knobs {
        Knobs {
            sub: sub.into(),
            email: email.into(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
            wrong_nonce: false,
            wrong_aud: false,
            expired: false,
        }
    }
}

struct Grant {
    nonce: String,
    challenge: String,
    redirect_uri: String,
    knobs: Knobs,
}

struct Issuer {
    url: String,
    key: CoreRsaPrivateSigningKey,
    knobs: Mutex<Knobs>,
    grants: Mutex<HashMap<String, Grant>>,
    /// Public mode: the token endpoint takes no client secret, only
    /// `client_id` in the body, and refuses any Authorization header.
    public: bool,
    token_requests: Mutex<Vec<TokenRequest>>,
}

/// What the hub sent to the token endpoint.
#[derive(Clone)]
struct TokenRequest {
    authorization: Option<String>,
    form: HashMap<String, String>,
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
        "token_endpoint_auth_methods_supported": [if i.public { "none" } else { "client_secret_basic" }],
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
    assert!(q["scope"].split(' ').any(|s| s == "openid"));
    let code = new_id();
    i.grants.lock().unwrap().insert(
        code.clone(),
        Grant {
            nonce: q["nonce"].clone(),
            challenge: q["code_challenge"].clone(),
            redirect_uri: q["redirect_uri"].clone(),
            knobs: i.knobs.lock().unwrap().clone(),
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
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    i.token_requests.lock().unwrap().push(TokenRequest {
        authorization: authorization.clone(),
        form: f.clone(),
    });
    let accepted = if i.public {
        authorization.is_none()
            && f.get("client_id").map(String::as_str) == Some(CLIENT_ID)
            && !f.contains_key("client_secret")
    } else {
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
        let expected = format!("Basic {basic}");
        authorization.as_deref() == Some(expected.as_str())
    };
    if !accepted {
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
    let Some(g) = i.grants.lock().unwrap().remove(&f["code"]) else {
        return bad();
    };
    let verifier_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(f["code_verifier"].as_bytes()));
    if f["grant_type"] != "authorization_code"
        || verifier_hash != g.challenge
        || f["redirect_uri"] != g.redirect_uri
    {
        return bad();
    }
    let k = &g.knobs;
    let now = chrono::Utc::now();
    let exp = if k.expired {
        now - chrono::Duration::minutes(5)
    } else {
        now + chrono::Duration::minutes(5)
    };
    let aud = if k.wrong_aud {
        "someone-else"
    } else {
        CLIENT_ID
    };
    let nonce = if k.wrong_nonce {
        "not-the-nonce".to_string()
    } else {
        g.nonce.clone()
    };
    let claims = IdTokenClaims::new(
        IssuerUrl::new(i.url.clone()).unwrap(),
        vec![Audience::new(aud.into())],
        exp,
        now - chrono::Duration::minutes(10),
        StandardClaims::new(SubjectIdentifier::new(k.sub.clone()))
            .set_email(Some(EndUserEmail::new(k.email.clone()))),
        Groups {
            groups: Some(k.groups.clone()),
        },
    )
    .set_nonce(Some(Nonce::new(nonce)));
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

async fn start_issuer(public: bool) -> Arc<Issuer> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let issuer = Arc::new(Issuer {
        url: format!("http://{addr}"),
        key: CoreRsaPrivateSigningKey::from_pem(
            test_key_pem(),
            Some(JsonWebKeyId::new("test".into())),
        )
        .unwrap(),
        knobs: Mutex::new(Knobs::user("ann-sub", "ann@example.com", &[])),
        grants: Mutex::new(HashMap::new()),
        public,
        token_requests: Mutex::new(Vec::new()),
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

const APP_REDIRECT: &str = "http://127.0.0.1:53111/done";

async fn env() -> Env {
    env_with_setup(|_| {}).await
}

/// The hub configured as a public client: no client secret.
async fn env_public() -> Env {
    env_inner(true, |_| {}).await
}

async fn env_with_setup(setup: impl FnOnce(&Config)) -> Env {
    env_inner(false, setup).await
}

async fn env_inner(public: bool, setup: impl FnOnce(&Config)) -> Env {
    let issuer = start_issuer(public).await;
    let dir = TempDir::new("data");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let hub_url = format!("http://{addr}");
    let config = Config {
        mcp_agent_enabled: false,
        mcp_agent_default_host: "hub".into(),
        listen: addr.to_string(),
        data_dir: dir.0.clone(),
        public_url: hub_url.clone(),
        trust_proxy: false,
        import_source_limit_bytes: Config::default().import_source_limit_bytes,
        schedules: Default::default(),
        mcp_oauth: Default::default(),
        oidc: OidcConfig {
            issuer: issuer.url.clone(),
            client_id: CLIENT_ID.into(),
            client_secret: if public {
                String::new()
            } else {
                CLIENT_SECRET.into()
            },
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
    setup(&config);
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
    fn sign_in_as(&self, k: Knobs) {
        *self.issuer.knobs.lock().unwrap() = k;
    }

    async fn location(&self, url: &str) -> Url {
        let r = self.http.get(url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FOUND, "GET {url}");
        let loc = r.headers()[header::LOCATION].to_str().unwrap();
        Url::parse(loc).unwrap()
    }

    /// Runs the browser part of a login; returns where the hub sent the
    /// browser back to the app.
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
        assert_eq!(param(&back, "state").as_deref(), Some("app-state-1"));
        back
    }

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

    /// A full login; returns (token, label).
    async fn token(&self, device: &str, name: &str) -> (String, String) {
        let back = self.login(device, name).await;
        let code = param(&back, "code").expect("a code");
        let (status, body) = self.redeem(&code, device).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        (
            body["token"].as_str().unwrap().to_string(),
            body["label"].as_str().unwrap().to_string(),
        )
    }

    fn client(&self, vault: Arc<Vault>, token: &str) -> Arc<SyncClient> {
        Arc::new(SyncClient::new(vault, &self.hub_url, token))
    }
}

fn param(u: &Url, k: &str) -> Option<String> {
    u.query_pairs()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.into_owned())
}

async fn sync(c: &Arc<SyncClient>) -> Result<(), SyncError> {
    let c = c.clone();
    tokio::task::spawn_blocking(move || c.sync_once().map(|_| ()))
        .await
        .unwrap()
}

// ---- tests ----

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    use std::io::Write;

    #[derive(Clone)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);
    impl Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let writer = LogWriter(logs.clone());
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish(),
        )
        .unwrap();
        logs
    })
    .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_device_tokens_do_not_log_file_values() {
    let logs = captured_logs();
    let e = env().await;
    let path = PathBuf::from(e.server.hub().vault().root()).join(".vault/hub/tokens.json");
    let device_dir = TempDir::new("private_token_file_device");
    let device = device_dir.open().device_id();
    let hash_canary = sha256_hex(b"private-token-file-hash-canary");
    let label_canary = "private-token-file-label-canary";

    for canary in [hash_canary.as_str(), label_canary] {
        let malformed = json!([{
            "hash": hash_canary,
            "label": label_canary,
            "device": null,
            "created_ms": canary,
        }]);
        fs::write(&path, serde_json::to_vec(&malformed).unwrap()).unwrap();
        let hub = e.server.hub().clone();
        let id = device.clone();
        let error = tokio::task::spawn_blocking(move || hub.login_device(&id, "phone"))
            .await
            .unwrap()
            .err()
            .unwrap();
        assert!(
            error.to_string().contains(canary),
            "the parser error must quote the canary"
        );

        let back = e.login(&device, "phone").await;
        assert_eq!(param(&back, "error").as_deref(), Some("device_refused"));
        assert_eq!(param(&back, "code"), None);
        let captured = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        assert!(
            !captured.contains(&hash_canary),
            "logs contain a token file hash"
        );
        assert!(
            !captured.contains(label_canary),
            "logs contain a token file label"
        );
        assert!(
            captured.lines().any(|line| {
                line.contains("login: device token refused")
                    && line.contains(&path.display().to_string())
                    && line.contains("error_kind=\"io\"")
            }),
            "the log must identify the file and error category"
        );
    }
    e.server.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn broken_agent_tokens_only_disable_mcp_and_recover_without_restart() {
    use notes_hub::agent_tokens::{AgentScope, AgentTokens};
    use std::os::unix::fs::PermissionsExt;

    let logs = captured_logs();

    for unreadable in [false, true] {
        let e = env_with_setup(|config| {
            let path = config.vault_dir().join(".vault/hub/agent-tokens.json");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if unreadable {
                fs::write(&path, b"[]").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
                assert!(
                    fs::read(&path).is_err(),
                    "test must run without root privileges"
                );
            } else {
                fs::write(&path, b"garbage credential-canary").unwrap();
            }
        })
        .await;
        let path =
            PathBuf::from(e.server.hub().vault().root()).join(".vault/hub/agent-tokens.json");
        let error_count = || {
            String::from_utf8(logs.lock().unwrap().clone())
                .unwrap()
                .lines()
                .filter(|line| {
                    line.contains(&path.display().to_string())
                        && line.contains("agent token file is unreadable or malformed")
                })
                .count()
        };
        assert_eq!(error_count(), 1, "startup logs one store error");
        let mcp = format!("{}/mcp", e.hub_url);
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"] {
            for origin in [None, Some("https://example.invalid")] {
                let mut req = e.http.request(method.parse().unwrap(), &mcp);
                if let Some(origin) = origin {
                    req = req.header("Origin", origin);
                }
                let response = req.send().await.unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert!(response.headers()["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("text/plain"));
                assert!(response
                    .headers()
                    .keys()
                    .all(|h| !h.as_str().starts_with("access-control-")));
                if method != "HEAD" {
                    assert_eq!(response.text().await.unwrap(), "agent tokens unavailable");
                }
            }
        }
        assert_eq!(error_count(), 1, "requests must not repeat the store error");
        assert_eq!(
            e.http
                .get(format!("{}/health", e.hub_url))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let device_dir = TempDir::new("agent_failure_device");
        let vault = device_dir.open();
        let (device_token, _) = e.token(&vault.device_id(), "phone").await;
        let page = vault
            .create_page(None, "Sync despite broken agent tokens".into())
            .unwrap();
        sync(&e.client(vault, &device_token)).await.unwrap();
        assert_eq!(
            e.server.hub().vault().load_page(page).unwrap().title,
            "Sync despite broken agent tokens"
        );

        if unreadable {
            // Permission repair leaves mtime and size unchanged: failed reads
            // must still be retried rather than caching the error forever.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        } else {
            fs::write(&path, b"[]").unwrap();
        }
        assert_eq!(
            e.http.get(&mcp).send().await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let store = AgentTokens::open(e.server.hub().vault().root()).unwrap();
        let token = store.add("repaired", AgentScope::Read).unwrap();
        let good = fs::read(&path).unwrap();
        let response = e
            .http
            .post(&mcp)
            .bearer_auth(&token)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.json::<Value>().await.unwrap()["result"]["tools"].is_array());

        // Also detect permission changes after successfully caching a file.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        for _ in 0..2 {
            assert_eq!(
                e.http
                    .post(&mcp)
                    .bearer_auth(&token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        assert_eq!(error_count(), 2);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            e.http.get(&mcp).send().await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );

        // These errors quote the canary in their parser diagnostics. This
        // catches accidental logging of raw error text as well as file bytes.
        for (broken, expected) in [
            (r#""credential-canary""#, 3),
            (
                r#"[{"name":"reader","scope":"credential-canary","sha256":"bad","created":1}]"#,
                4,
            ),
        ] {
            fs::write(&path, broken).unwrap();
            let error = AgentTokens::open(e.server.hub().vault().root())
                .err()
                .unwrap();
            assert!(error.to_string().contains("credential-canary"));
            for _ in 0..2 {
                assert_eq!(
                    e.http
                        .post(&mcp)
                        .bearer_auth(&token)
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::SERVICE_UNAVAILABLE
                );
            }
            assert_eq!(error_count(), expected, "one error per changed broken file");
        }
        fs::write(&path, good).unwrap();
        assert_eq!(
            e.http
                .post(&mcp)
                .bearer_auth(&token)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let captured = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        let hash = vault_core::sha256_hex(token.as_bytes());
        for secret in [token.as_str(), hash.as_str(), "credential-canary"] {
            assert!(
                !captured.contains(secret),
                "logs must not contain token material"
            );
        }
        e.server.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn allowed_users_get_tokens_that_sync() {
    let e = env().await;
    let (da, db) = (TempDir::new("a"), TempDir::new("b"));
    let (va, vb) = (da.open(), db.open());

    // Ann is allowed by email (any case), Carol by group.
    let (ta, la) = e.token(&va.device_id(), "phone").await;
    assert_eq!(la, "phone");
    e.sign_in_as(Knobs::user(
        "carol-sub",
        "carol@example.com",
        &["notes-users"],
    ));
    let (tb, lb) = e.token(&vb.device_id(), "laptop").await;
    assert_eq!(lb, "laptop");
    let devices = e.server.hub().devices();
    assert_eq!(devices.len(), 2);
    assert!(devices
        .iter()
        .any(|d| d.label == "phone" && d.device.as_deref() == Some(va.device_id().as_str())));

    // Two clients converge through notes-hub.
    let (a, b) = (e.client(va.clone(), &ta), e.client(vb.clone(), &tb));
    let p = va.create_page(None, "From A".into()).unwrap();
    sync(&a).await.unwrap();
    sync(&b).await.unwrap();
    assert_eq!(vb.load_page(p.clone()).unwrap().title, "From A");
    vb.rename_page(p.clone(), "Edited on B".into()).unwrap();
    let q = vb.create_page(None, "From B".into()).unwrap();
    sync(&b).await.unwrap();
    sync(&a).await.unwrap();
    assert_eq!(va.load_page(p.clone()).unwrap().title, "Edited on B");
    assert_eq!(va.load_page(q.clone()).unwrap().title, "From B");
    let hub_vault = e.server.hub().vault();
    assert_eq!(hub_vault.load_page(q).unwrap().title, "From B");

    // A token is bound to its device.
    let stolen = e.client(vb.clone(), &ta);
    assert!(matches!(
        sync(&stolen).await,
        Err(SyncError::Status(403, _))
    ));
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_logins_make_no_token() {
    let e = env().await;
    let d = TempDir::new("a");
    let device = d.open().device_id();

    // Not listed, not in a listed group.
    e.sign_in_as(Knobs::user("mallory", "mallory@example.com", &["other"]));
    let back = e.login(&device, "phone").await;
    assert_eq!(param(&back, "error").as_deref(), Some("access_denied"));
    assert_eq!(param(&back, "code"), None);

    // Ann, but the ID token is spoiled.
    let ann = Knobs::user("ann-sub", "ann@example.com", &[]);
    for spoil in [
        |k: &mut Knobs| k.wrong_nonce = true,
        |k: &mut Knobs| k.wrong_aud = true,
        |k: &mut Knobs| k.expired = true,
    ] {
        let mut k = ann.clone();
        spoil(&mut k);
        e.sign_in_as(k);
        let back = e.login(&device, "phone").await;
        assert_eq!(param(&back, "error").as_deref(), Some("login_failed"));
        assert_eq!(param(&back, "code"), None);
    }
    assert!(e.server.hub().devices().is_empty());

    // A callback with an unknown state shows the error page.
    let r = e
        .http
        .get(format!("{}/auth/callback?code=x&state=nope", e.hub_url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.text().await.unwrap().contains("Sign-in failed"));
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_redirects_are_refused() {
    let e = env().await;
    for redirect in [
        "https://evil.example/cb",
        "http://127.0.0.1/no-port",
        "http://localhost:8000/done",
        "http://127.0.0.1:1023/done",
        "http://127.0.0.1:65536/done",
        "http://127.0.0.1:8000/other",
        "http://127.0.0.1:8000/done?x=1",
        "http://127.0.0.1:8000/foo/../done",
        "http://127.0.0.1:8000/done\n",
        "http://evil.example:8000/done",
        "notes://other",
        "notes://auth/done",
        "notes://auth?x=1",
    ] {
        let mut u = Url::parse(&format!("{}/auth/start", e.hub_url)).unwrap();
        u.query_pairs_mut()
            .append_pair("device", "abc123")
            .append_pair("name", "phone")
            .append_pair("redirect", redirect);
        let r = e.http.get(u).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{redirect}");
    }
    // Android and both desktop port boundaries are accepted.
    for redirect in [
        "notes://auth",
        "http://127.0.0.1:1024/done",
        "http://127.0.0.1:65535/done",
    ] {
        let mut u = Url::parse(&format!("{}/auth/start", e.hub_url)).unwrap();
        u.query_pairs_mut()
            .append_pair("device", "abc123")
            .append_pair("redirect", redirect);
        assert_eq!(
            e.http.get(u).send().await.unwrap().status(),
            StatusCode::FOUND,
            "{redirect}"
        );
    }
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_time_codes_are_single_use_and_expire() {
    let e = env().await;
    let d = TempDir::new("a");
    let device = d.open().device_id();

    let code = param(&e.login(&device, "phone").await, "code").unwrap();
    // The device must be the one that started the login; a wrong one uses
    // the code up.
    assert_eq!(
        e.redeem(&code, "another-device").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(e.redeem(&code, &device).await.0, StatusCode::BAD_REQUEST);

    let code = param(&e.login(&device, "phone").await, "code").unwrap();
    assert_eq!(e.redeem(&code, &device).await.0, StatusCode::OK);
    let (status, body) = e.redeem(&code, &device).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_code");

    e.server.set_code_ttl(Duration::from_millis(1));
    let code = param(&e.login(&device, "phone").await, "code").unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(e.redeem(&code, &device).await.0, StatusCode::BAD_REQUEST);
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relogin_replaces_the_token_and_sign_out_drops_it() {
    let e = env().await;
    let (da, db) = (TempDir::new("a"), TempDir::new("b"));
    let (va, vb) = (da.open(), db.open());

    let (old, _) = e.token(&va.device_id(), "phone").await;
    let (new, label) = e.token(&va.device_id(), "phone").await;
    assert_eq!(label, "phone");
    assert_ne!(old, new);
    assert_eq!(e.server.hub().devices().len(), 1);
    assert!(matches!(
        sync(&e.client(va.clone(), &old)).await,
        Err(SyncError::Status(401, _))
    ));
    sync(&e.client(va.clone(), &new)).await.unwrap();

    // Another device with the same name gets a suffix.
    let (tb, lb) = e.token(&vb.device_id(), "phone").await;
    assert_eq!(lb, "phone (2)");

    // Sign-out drops the token.
    let signout = |t: String| {
        e.http
            .post(format!("{}/auth/signout", e.hub_url))
            .bearer_auth(t)
            .send()
    };
    assert_eq!(
        signout(new.clone()).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        signout(new.clone()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(matches!(
        sync(&e.client(va.clone(), &new)).await,
        Err(SyncError::Status(401, _))
    ));
    let devices = e.server.hub().devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].label, "phone (2)");
    sync(&e.client(vb.clone(), &tb)).await.unwrap();

    let health = e
        .http
        .get(format!("{}/health", e.hub_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        health
            .headers()
            .get(vault_sync::PURGE_CAPABILITY_HEADER)
            .unwrap(),
        vault_sync::PURGE_CAPABILITY
    );
    assert_eq!(
        health
            .headers()
            .get(vault_sync::PURGE_DEVICE_CAPABILITY_HEADER)
            .unwrap(),
        vault_sync::PURGE_DEVICE_CAPABILITY
    );
    assert_eq!(health.text().await.unwrap(), "notes-hub");
    e.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_client_signs_in_with_pkce_and_no_secret() {
    let e = env_public().await;
    let d = TempDir::new("public");
    let v = d.open();

    let (token, label) = e.token(&v.device_id(), "phone").await;
    assert_eq!(label, "phone");

    // One token request, public-client shaped: no Authorization header,
    // client_id in the body, no secret, and a PKCE verifier (the issuer
    // already checked it against the S256 challenge of the authorize request).
    let reqs = e.issuer.token_requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "one token request per login");
    let r = &reqs[0];
    assert_eq!(r.authorization, None);
    assert_eq!(r.form.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert!(!r.form.contains_key("client_secret"));
    assert!(r.form.get("code_verifier").is_some_and(|v| v.len() >= 43));

    // The token works for sync.
    let c = e.client(v.clone(), &token);
    sync(&c).await.unwrap();
    e.server.stop().await;
}
