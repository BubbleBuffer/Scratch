//! The hub's `/mcp` as an opt-in OAuth resource server, end to end: a real
//! listener, a mock issuer (discovery, JWKS and userinfo, with RS256 access
//! tokens signed by a test key) and the static agent tokens beside it.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, McpOAuthConfig, OidcConfig, Server};
use openidconnect::core::{CoreHmacKey, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey};
use openidconnect::{JsonWebKeyId, PrivateSigningKey};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use vault_core::new_id;

const CLIENT_ID: &str = "notes-mcp";
const KID: &str = "test";
const TIMEOUT: Duration = Duration::from_secs(20);

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- keys and tokens ----

fn test_key_pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
        let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string()
    })
}

fn signing_key() -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(test_key_pem(), Some(JsonWebKeyId::new(KID.into()))).unwrap()
}

fn b64(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn signing_input(alg: &str, claims: &Value) -> String {
    let header = b64(json!({"alg": alg, "typ": "JWT", "kid": KID}).to_string());
    format!("{header}.{}", b64(claims.to_string()))
}

fn sign_with(key: &CoreRsaPrivateSigningKey, claims: &Value) -> String {
    let input = signing_input("RS256", claims);
    let signature = key
        .sign(
            &CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            input.as_bytes(),
        )
        .unwrap();
    format!("{input}.{}", b64(signature))
}

/// Correctly signed RS512: only the RS256 pin can reject it.
fn sign_rs512(key: &CoreRsaPrivateSigningKey, claims: &Value) -> String {
    let input = signing_input("RS512", claims);
    let signature = key
        .sign(
            &CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
            input.as_bytes(),
        )
        .unwrap();
    format!("{input}.{}", b64(signature))
}

fn sign_hs256(claims: &Value) -> String {
    let input = signing_input("HS256", claims);
    let signature = CoreHmacKey::new(b"test-hmac-secret".to_vec())
        .sign(&CoreJwsSigningAlgorithm::HmacSha256, input.as_bytes())
        .unwrap();
    format!("{input}.{}", b64(signature))
}

/// An unsecured token: `alg: none` with an empty signature.
fn unsigned(claims: &Value) -> String {
    format!("{}.", signing_input("none", claims))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Access token claims as Authentik 2026.5 issues them: `aud` and `azp` are
/// the client id, with `uid` and `scope` extras and an unknown claim that
/// the hub must ignore. `email` and `groups` are left out when `None`.
fn user_claims(issuer: &str, sub: &str, email: Option<&str>, groups: Option<Vec<&str>>) -> Value {
    let mut claims = json!({
        "iss": issuer,
        "aud": CLIENT_ID,
        "azp": CLIENT_ID,
        "sub": sub,
        "uid": format!("uid-{sub}"),
        "scope": "openid email profile",
        "iat": now(),
        "exp": now() + 300,
        "x_authentik_unknown": {"ignored": true},
    });
    if let Some(email) = email {
        claims["email"] = json!(email);
    }
    if let Some(groups) = groups {
        claims["groups"] = json!(groups);
    }
    claims
}

fn with(claims: &Value, key: &str, value: Value) -> Value {
    let mut claims = claims.clone();
    claims[key] = value;
    claims
}

/// The token's header and signature with a different payload.
fn with_payload(token: &str, claims: &Value) -> String {
    let parts: Vec<&str> = token.split('.').collect();
    format!("{}.{}.{}", parts[0], b64(claims.to_string()), parts[2])
}

fn with_bad_signature(token: &str) -> String {
    let (head, signature) = token.rsplit_once('.').unwrap();
    let mut bytes = signature.as_bytes().to_vec();
    bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
    format!("{head}.{}", String::from_utf8(bytes).unwrap())
}

// ---- JSON-RPC helpers ----

fn tools_list() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
}

fn tool_names(reply: &Value) -> Vec<String> {
    let mut names: Vec<String> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort_unstable();
    names
}

fn tool_text(reply: &Value) -> &str {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    reply["result"]["content"][0]["text"].as_str().unwrap()
}

/// A 401 whose `WWW-Authenticate` is exactly the metadata challenge, plus
/// `error` when given.
fn assert_challenge(env: &Env, response: &reqwest::Response, error: Option<&str>, label: &str) {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{label}");
    let mut expected = format!("Bearer resource_metadata=\"{}\"", env.metadata());
    if let Some(error) = error {
        expected.push_str(&format!(", error=\"{error}\""));
    }
    let actual = response.headers()["www-authenticate"].to_str().unwrap();
    assert_eq!(actual, expected.as_str(), "{label}");
}

fn assert_invalid_token(env: &Env, response: &reqwest::Response, label: &str) {
    assert_challenge(env, response, Some("invalid_token"), label);
}

/// Every version's device of the page titled `title`, which must have history.
fn devices(env: &Env, title: &str) -> Vec<String> {
    let vault = env.server.hub().vault();
    let page = vault_mcp::tools::resolve_page(vault, title).unwrap();
    let devices: Vec<String> = vault
        .list_versions(page)
        .unwrap()
        .into_iter()
        .map(|v| v.device)
        .collect();
    assert!(!devices.is_empty(), "{title} has no write history");
    devices
}

async fn closed_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/userinfo")
}

// ---- the mock issuer ----

#[derive(Clone)]
struct Knobs {
    discovery: StatusCode,
    jwks: StatusCode,
    userinfo_status: StatusCode,
    userinfo_body: Value,
    /// Overrides the discovered userinfo endpoint, for a network failure.
    userinfo_url: Option<String>,
}

impl Default for Knobs {
    fn default() -> Knobs {
        Knobs {
            discovery: StatusCode::OK,
            jwks: StatusCode::OK,
            userinfo_status: StatusCode::OK,
            userinfo_body: json!({}),
            userinfo_url: None,
        }
    }
}

struct Shared {
    url: String,
    key: CoreRsaPrivateSigningKey,
    knobs: Mutex<Knobs>,
    userinfo_hits: AtomicUsize,
}

struct MockIssuer {
    shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl MockIssuer {
    async fn start() -> MockIssuer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let shared = Arc::new(Shared {
            url,
            key: signing_key(),
            knobs: Mutex::new(Knobs::default()),
            userinfo_hits: AtomicUsize::new(0),
        });
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/userinfo", get(userinfo))
            .with_state(shared.clone());
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        MockIssuer {
            shared,
            stop: Some(stop),
            task: Some(task),
        }
    }

    fn url(&self) -> &str {
        &self.shared.url
    }

    fn knobs(&self) -> MutexGuard<'_, Knobs> {
        self.shared.knobs.lock().unwrap()
    }

    fn hits(&self) -> usize {
        self.shared.userinfo_hits.load(Ordering::SeqCst)
    }

    fn sign(&self, claims: &Value) -> String {
        sign_with(&self.shared.key, claims)
    }

    async fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .expect("mock issuer shutdown hung")
                .unwrap();
        }
    }
}

async fn discovery(State(shared): State<Arc<Shared>>) -> Response {
    let knobs = shared.knobs.lock().unwrap().clone();
    if knobs.discovery != StatusCode::OK {
        return knobs.discovery.into_response();
    }
    let userinfo = knobs
        .userinfo_url
        .unwrap_or_else(|| format!("{}/userinfo", shared.url));
    Json(json!({
        "issuer": shared.url,
        "authorization_endpoint": format!("{}/authorize", shared.url),
        "token_endpoint": format!("{}/token", shared.url),
        "userinfo_endpoint": userinfo,
        "jwks_uri": format!("{}/jwks", shared.url),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256", "RS512"],
    }))
    .into_response()
}

async fn jwks(State(shared): State<Arc<Shared>>) -> Response {
    let status = shared.knobs.lock().unwrap().jwks;
    if status != StatusCode::OK {
        return status.into_response();
    }
    Json(json!({"keys": [shared.key.as_verification_key()]})).into_response()
}

async fn userinfo(State(shared): State<Arc<Shared>>) -> Response {
    shared.userinfo_hits.fetch_add(1, Ordering::SeqCst);
    let knobs = shared.knobs.lock().unwrap().clone();
    if knobs.userinfo_status != StatusCode::OK {
        return knobs.userinfo_status.into_response();
    }
    Json(knobs.userinfo_body).into_response()
}

// ---- the hub ----

const ALLOWED_GROUP: &str = "notes-users";

struct Env {
    server: Server,
    issuer: MockIssuer,
    client: Client,
    base: String,
    url: String,
    read: String,
    write: String,
    data: TempDir,
}

impl Env {
    /// The hub with agents disabled and two static tokens; `oauth` turns
    /// on the MCP resource. The allow lists come from `[oidc]`.
    async fn boot(oauth: bool) -> Env {
        let data = TempDir(std::env::temp_dir().join(format!("notes_hub_mcp_oauth_{}", new_id())));
        let issuer = MockIssuer::start().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let mut config = Config {
            listen: addr.to_string(),
            data_dir: data.0.clone(),
            // A trailing slash: the metadata and resource URLs must still join cleanly.
            public_url: format!("{base}/"),
            trust_proxy: false,
            oidc: OidcConfig {
                allowed_users: vec!["ann@example.com".into(), "bob-sub".into()],
                allowed_groups: vec![ALLOWED_GROUP.into()],
                ..OidcConfig::default()
            },
            agent: AgentSection {
                enabled: false,
                ..AgentSection::default()
            },
            ..Config::default()
        };
        if oauth {
            config.mcp_oauth = McpOAuthConfig {
                issuer: issuer.url().into(),
                client_id: CLIENT_ID.into(),
            };
        }
        let store = AgentTokens::open(config.vault_dir()).unwrap();
        let read = store.add("reader", AgentScope::Read).unwrap();
        let write = store.add("writer", AgentScope::Write).unwrap();
        let server = notes_hub::start_on(listener, config).await.unwrap();
        Env {
            server,
            issuer,
            client: Client::builder().timeout(TIMEOUT).build().unwrap(),
            url: format!("{base}/mcp"),
            base,
            read,
            write,
            data,
        }
    }

    async fn oauth() -> Env {
        Env::boot(true).await
    }

    /// The exact metadata URL the challenge must name.
    fn metadata(&self) -> String {
        format!("{}/.well-known/oauth-protected-resource/mcp", self.base)
    }

    async fn plain() -> Env {
        Env::boot(false).await
    }

    fn token(&self, sub: &str, email: Option<&str>, groups: Option<Vec<&str>>) -> String {
        self.issuer
            .sign(&user_claims(self.issuer.url(), sub, email, groups))
    }

    /// A `tools/list` POST with an optional bearer token and Origin.
    async fn send(&self, token: Option<&str>, origin: Option<&str>) -> reqwest::Response {
        let mut request = self.client.post(&self.url).json(&tools_list());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        request.send().await.unwrap()
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(token)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }

    async fn call(&self, token: &str, name: &str, arguments: Value) -> Value {
        self.rpc(
            token,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
    }

    async fn list(&self, token: &str) -> Vec<String> {
        tool_names(&self.rpc(token, "tools/list", json!({})).await)
    }

    async fn stop(mut self) {
        self.server.stop().await;
        self.issuer.stop().await;
    }
}

#[tokio::test]
async fn metadata_paths_are_exact_and_unauthenticated() {
    let env = Env::oauth().await;
    let expected = json!({
        "resource": env.url,
        "authorization_servers": [env.issuer.url()],
        "scopes_supported": ["openid", "email", "profile", "offline_access"],
        "bearer_methods_supported": ["header"],
    });
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let response = env
            .client
            .get(format!("{}{path}", env.base))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body, expected, "{path}");
    }
    env.stop().await;
}

#[tokio::test]
async fn disabled_oauth_falls_through_to_static_tokens_only() {
    let env = Env::plain().await;
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let body = env
            .client
            .get(format!("{}{path}", env.base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!body.contains("authorization_servers"), "{path}: {body}");
    }
    let jwt = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let response = env.send(Some(jwt.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    for header in ["Bearer", "Bearer "] {
        let response = env
            .client
            .post(&env.url)
            .header("authorization", header)
            .json(&tools_list())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{header:?}");
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }
    let response = env
        .send(Some(env.write.as_str()), Some("https://claude.ai"))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = env.send(Some(env.write.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    env.stop().await;
}

#[tokio::test]
async fn allowed_group_token_lists_the_full_tool_set() {
    let env = Env::oauth().await;
    let full = env.list(&env.write).await;
    assert!(full.contains(&"create_page".to_string()));
    // Gina is listed by group alone: neither her sub nor her email is in the allow list.
    let token = env.token(
        "gina-sub",
        Some("gina@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    assert_eq!(env.list(&token).await, full);
    // The groups came with the token, so no userinfo request was needed.
    assert_eq!(env.issuer.hits(), 0);
    env.stop().await;
}

#[tokio::test]
async fn invalid_tokens_are_401_with_challenge() {
    let env = Env::oauth().await;
    let full = env.list(&env.write).await;
    let good = user_claims(
        env.issuer.url(),
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    assert_eq!(env.list(&env.issuer.sign(&good)).await, full);
    let cases = [
        (
            "wrong issuer",
            env.issuer
                .sign(&with(&good, "iss", json!("https://other.example"))),
        ),
        (
            "wrong audience",
            env.issuer.sign(&with(&good, "aud", json!("other-client"))),
        ),
        (
            "expired",
            env.issuer.sign(&with(&good, "exp", json!(now() - 300))),
        ),
        (
            "tampered payload",
            with_payload(
                &env.issuer.sign(&good),
                &with(&good, "sub", json!("mallory-sub")),
            ),
        ),
        (
            "corrupted signature",
            with_bad_signature(&env.issuer.sign(&good)),
        ),
        ("garbage", "not.a.jwt".to_string()),
        ("alg none", unsigned(&good)),
        ("HS256", sign_hs256(&good)),
        (
            "correctly signed RS512",
            sign_rs512(&env.issuer.shared.key, &good),
        ),
    ];
    for (label, token) in &cases {
        let response = env.send(Some(token.as_str()), None).await;
        assert_invalid_token(&env, &response, label);
    }
    env.stop().await;
}

#[tokio::test]
async fn audience_is_client_id_or_client_id_with_mcp_url() {
    let env = Env::oauth().await;
    let full = env.list(&env.write).await;
    let base = user_claims(
        env.issuer.url(),
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let mcp = env.url.clone();
    let accepted = env
        .issuer
        .sign(&with(&base, "aud", json!([CLIENT_ID, mcp])));
    assert_eq!(env.list(&accepted).await, full);
    let rejected = [
        (
            "client id plus unrelated audience",
            json!([CLIENT_ID, "https://unrelated.example/mcp"]),
        ),
        ("MCP URL alone", json!(mcp)),
    ];
    for (label, aud) in rejected {
        let token = env.issuer.sign(&with(&base, "aud", aud));
        let response = env.send(Some(token.as_str()), None).await;
        assert_invalid_token(&env, &response, label);
    }
    env.stop().await;
}

#[tokio::test]
async fn unlisted_user_is_forbidden() {
    let env = Env::oauth().await;
    let with_groups = env.token(
        "eve-sub",
        Some("eve@example.com"),
        Some(vec!["other-group"]),
    );
    let without_groups = env.token("eve-sub", Some("eve@example.com"), None);
    env.issuer.knobs().userinfo_body =
        json!({"sub": "eve-sub", "email": "eve@example.com", "groups": ["other-group"]});
    for token in [&with_groups, &without_groups] {
        let response = env.send(Some(token.as_str()), None).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    env.stop().await;
}

#[tokio::test]
async fn missing_groups_consult_userinfo_once_and_cache_the_token() {
    let env = Env::oauth().await;
    let full = env.list(&env.write).await;
    env.issuer.knobs().userinfo_body =
        json!({"sub": "carol-sub", "email": "carol@example.com", "groups": [ALLOWED_GROUP]});
    let token = env.token("carol-sub", Some("carol@example.com"), None);
    assert_eq!(env.list(&token).await, full);
    assert_eq!(env.issuer.hits(), 1);
    assert_eq!(env.list(&token).await, full);
    assert_eq!(env.issuer.hits(), 1);
    env.stop().await;
}

#[tokio::test]
async fn missing_email_uses_allowed_users_by_sub() {
    let env = Env::oauth().await;
    let full = env.list(&env.write).await;
    // A listed sub is allowed before userinfo, so it needs no request even without an email.
    let bob = env.token("bob-sub", None, Some(vec![]));
    assert_eq!(env.list(&bob).await, full);
    assert_eq!(env.issuer.hits(), 0);
    // Hank is unlisted by sub and has no email. Userinfo supplies an allowed email in other case.
    env.issuer.knobs().userinfo_body =
        json!({"sub": "hank-sub", "email": "ANN@Example.com", "groups": []});
    let hank = env.token("hank-sub", None, Some(vec![]));
    assert_eq!(env.list(&hank).await, full);
    assert_eq!(env.issuer.hits(), 1);
    assert_eq!(env.list(&hank).await, full);
    assert_eq!(env.issuer.hits(), 1);
    // Dan has no email and no allowed sub, email, or group after userinfo.
    env.issuer.knobs().userinfo_body = json!({"sub": "dan-sub", "groups": []});
    let dan = env.token("dan-sub", None, Some(vec![]));
    let response = env.send(Some(dan.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(env.issuer.hits(), 2);
    env.stop().await;
}

#[tokio::test]
async fn listed_sub_or_email_skips_userinfo() {
    let env = Env::oauth().await;
    let full = env.list(&env.write).await;
    let by_sub = env.token("bob-sub", None, None);
    let by_email = env.token("x-sub", Some("ann@example.com"), None);
    assert_eq!(env.list(&by_sub).await, full);
    assert_eq!(env.list(&by_email).await, full);
    assert_eq!(env.issuer.hits(), 0);
    env.stop().await;
}

#[tokio::test]
async fn userinfo_subject_mismatch_is_401() {
    let env = Env::oauth().await;
    env.issuer.knobs().userinfo_body = json!({"sub": "mallory-sub", "groups": [ALLOWED_GROUP]});
    let token = env.token("carol-sub", Some("carol@example.com"), None);
    let response = env.send(Some(token.as_str()), None).await;
    assert_invalid_token(&env, &response, "userinfo subject mismatch");
    env.stop().await;
}

#[tokio::test]
async fn userinfo_error_status_is_503() {
    let env = Env::oauth().await;
    env.issuer.knobs().userinfo_status = StatusCode::INTERNAL_SERVER_ERROR;
    let token = env.token("dan-sub", None, None);
    let response = env.send(Some(token.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}

#[tokio::test]
async fn userinfo_network_failure_is_503() {
    let env = Env::oauth().await;
    let dead = closed_url().await;
    env.issuer.knobs().userinfo_url = Some(dead);
    let token = env.token("dan-sub", None, None);
    let response = env.send(Some(token.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}

#[tokio::test]
async fn discovery_failure_is_503() {
    let env = Env::oauth().await;
    env.issuer.knobs().discovery = StatusCode::INTERNAL_SERVER_ERROR;
    let garbage = env.send(Some("not.a.jwt"), None).await;
    assert_invalid_token(&env, &garbage, "garbage with unavailable issuer");
    let token = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let response = env.send(Some(token.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}

#[tokio::test]
async fn jwks_failure_is_503() {
    let env = Env::oauth().await;
    env.issuer.knobs().jwks = StatusCode::INTERNAL_SERVER_ERROR;
    let token = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let response = env.send(Some(token.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}

#[tokio::test]
async fn static_tokens_keep_scope_and_identity_and_bypass_a_broken_issuer() {
    let mut env = Env::oauth().await;
    let full = env.list(&env.write).await;
    let read = env.list(&env.read).await;
    assert!(read.contains(&"read_page".to_string()));
    assert!(!read.contains(&"create_page".to_string()));
    assert!(full.contains(&"create_page".to_string()));
    let token = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let reply = env
        .call(
            &env.write,
            "create_page",
            json!({"title": "Static identity", "markdown": "staticneedle"}),
        )
        .await;
    tool_text(&reply);
    let refused = env
        .call(&env.read, "create_page", json!({"title": "Static refused"}))
        .await;
    assert!(refused["error"]["code"].is_number(), "{refused}");
    assert!(refused.get("result").is_none());
    let identity = devices(&env, "Static identity");
    assert!(
        identity.iter().all(|d| d.as_str() == "agent:writer"),
        "{identity:?}"
    );

    env.issuer.stop().await;
    assert_eq!(env.list(&env.read).await, read);
    assert_eq!(env.list(&env.write).await, full);
    let response = env.send(Some(token.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}

#[tokio::test]
async fn oauth_writes_are_attributed_to_email_or_sub() {
    let env = Env::oauth().await;
    let by_email = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let by_sub = env.token("bob-sub", None, None);
    for (token, title) in [(&by_email, "OAuth email note"), (&by_sub, "OAuth sub note")] {
        let reply = env
            .call(
                token,
                "create_page",
                json!({"title": title, "markdown": "oauthactor"}),
            )
            .await;
        tool_text(&reply);
    }
    // `devices` asserts the history is nonempty; every version must name the exact actor.
    let email_devices = devices(&env, "OAuth email note");
    assert!(
        email_devices.iter().all(|d| d == "oauth:ann@example.com"),
        "{email_devices:?}"
    );
    let sub_devices = devices(&env, "OAuth sub note");
    assert!(
        sub_devices.iter().all(|d| d == "oauth:bob-sub"),
        "{sub_devices:?}"
    );
    env.stop().await;
}

#[tokio::test]
async fn cached_token_is_rejected_after_expiry() {
    let env = Env::oauth().await;
    // Frank is allowed only through userinfo, whose result is cached until exp.
    env.issuer.knobs().userinfo_body =
        json!({"sub": "frank-sub", "email": "frank@example.com", "groups": [ALLOWED_GROUP]});
    let claims = user_claims(
        env.issuer.url(),
        "frank-sub",
        Some("frank@example.com"),
        None,
    );
    // `exp` is whole seconds, so this gives at least two seconds of validity.
    let short = env.issuer.sign(&with(&claims, "exp", json!(now() + 3)));
    let response = env.send(Some(short.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(env.issuer.hits(), 1);
    let response = env.send(Some(short.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(env.issuer.hits(), 1);
    tokio::time::sleep(Duration::from_secs(4)).await;
    let response = env.send(Some(short.as_str()), None).await;
    assert_invalid_token(&env, &response, "cached token after expiry");
    // Expiry is checked before the cache, so the cached entry was not used again.
    assert_eq!(env.issuer.hits(), 1);
    env.stop().await;
}

#[tokio::test]
async fn missing_bearer_challenge_has_no_error_when_oauth_is_enabled() {
    let env = Env::oauth().await;
    let response = env.send(None, None).await;
    assert_challenge(&env, &response, None, "missing bearer");
    env.stop().await;
}

#[tokio::test]
async fn origin_and_method_rules_with_oauth_enabled() {
    let env = Env::oauth().await;
    let token = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    for origin in [
        None,
        Some("https://claude.ai"),
        Some("https://claude.com"),
        Some("https://chatgpt.com"),
    ] {
        let response = env.send(Some(token.as_str()), origin).await;
        assert_eq!(response.status(), StatusCode::OK, "{origin:?}");
    }
    for origin in ["https://evil.example", "null"] {
        let response = env.send(Some(token.as_str()), Some(origin)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origin}");
    }
    let response = env.client.get(&env.url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()["allow"], "POST");
    let response = env
        .client
        .get(&env.url)
        .header("Origin", "https://claude.ai")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let response = env
        .client
        .get(&env.url)
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    env.stop().await;
}

#[tokio::test]
async fn broken_agent_store_is_503_before_oauth() {
    let env = Env::oauth().await;
    let store = env
        .data
        .0
        .join("vault")
        .join(".vault")
        .join("hub")
        .join("agent-tokens.json");
    fs::write(&store, "not json").unwrap();
    let token = env.token(
        "ann-sub",
        Some("ann@example.com"),
        Some(vec![ALLOWED_GROUP]),
    );
    let response = env.send(Some(token.as_str()), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = env.send(Some("garbage"), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = env.send(None, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = env
        .client
        .get(&env.url)
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}
