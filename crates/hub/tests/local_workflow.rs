//! The local mobile-Hub note workflow in one process: a device signs in
//! through a mock OIDC issuer, its vault syncs to the in-process hub, and a
//! chat over `/agent/ws` runs a scripted model that searches, reads and
//! edits the device's note. The edit syncs back to the device.
//!
//! Everything here binds to loopback and uses synthetic tokens. Every
//! server, socket and task started below is stopped by the test.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use notes_agent_wire as w;
use notes_hub::{AgentSection, AgentTestHooks, Config, OidcConfig, Server};
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
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use vault_agent::bubblepaw_core::ToolCall;
use vault_agent::bubblepaw_runner::provider::ScriptedStep;
use vault_agent::TestHooks;
use vault_core::{new_id, Vault};
use vault_sync::SyncClient;

const CLIENT_ID: &str = "notes";
const CLIENT_SECRET: &str = "test-secret";
const APP_REDIRECT: &str = "http://127.0.0.1:53111/done";
const ANN_EMAIL: &str = "ann@example.com";
const ANN_SUB: &str = "ann-sub";

const NOTE: &str = "Pantry plan";
const SEED: &str = "- Eggs\n- Rice";
const ADDED: &str = "- Oat milk (added by the hub agent)";
const FINAL: &str = "Added oat milk to the pantry plan.";

// ---- temp dirs ----

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_flow_{tag}_{}", new_id()));
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

// ---- the mock issuer (one user, Ann) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct NoGroups {
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<String>>,
}
impl AdditionalClaims for NoGroups {}

struct Grant {
    nonce: String,
    challenge: String,
    redirect_uri: String,
}

struct IssuerState {
    url: String,
    key: CoreRsaPrivateSigningKey,
    grants: Mutex<HashMap<String, Grant>>,
}

/// A running issuer: stop it with [`Idp::stop`].
struct Idp {
    url: String,
    stop: oneshot::Sender<()>,
    task: JoinHandle<io::Result<()>>,
}

impl Idp {
    async fn stop(self) {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .expect("the issuer stops")
            .expect("issuer task")
            .expect("issuer serve");
    }
}

fn test_key_pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
        let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string()
    })
}

async fn discovery(State(i): State<Arc<IssuerState>>) -> Json<Value> {
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

async fn jwks(State(i): State<Arc<IssuerState>>) -> Json<Value> {
    Json(json!({ "keys": [i.key.as_verification_key()] }))
}

async fn authorize(
    State(i): State<Arc<IssuerState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    assert_eq!(q["client_id"], CLIENT_ID);
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
    State(i): State<Arc<IssuerState>>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    let basic =
        base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if auth != Some(&format!("Basic {basic}")) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_client"})),
        )
            .into_response();
    }
    let Some(g) = i.grants.lock().unwrap().remove(&f["code"]) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response();
    };
    let verifier_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(f["code_verifier"].as_bytes()));
    if f["grant_type"] != "authorization_code"
        || verifier_hash != g.challenge
        || f["redirect_uri"] != g.redirect_uri
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response();
    }
    let now = chrono::Utc::now();
    let claims = IdTokenClaims::new(
        IssuerUrl::new(i.url.clone()).unwrap(),
        vec![Audience::new(CLIENT_ID.into())],
        now + chrono::Duration::minutes(5),
        now - chrono::Duration::minutes(10),
        StandardClaims::new(SubjectIdentifier::new(ANN_SUB.into()))
            .set_email(Some(EndUserEmail::new(ANN_EMAIL.into()))),
        NoGroups::default(),
    )
    .set_nonce(Some(Nonce::new(g.nonce.clone())));
    let id_token = IdToken::<
        NoGroups,
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

async fn start_idp() -> Idp {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(IssuerState {
        url: format!("http://{addr}"),
        key: CoreRsaPrivateSigningKey::from_pem(
            test_key_pem(),
            Some(JsonWebKeyId::new("test".into())),
        )
        .unwrap(),
        grants: Mutex::new(HashMap::new()),
    });
    let url = state.url.clone();
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(state);
    let (stop, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
    });
    Idp { url, stop, task }
}

// ---- the hub, with the scripted model ----

/// The model's turns, as the `js` tool calls a real model makes (the model
/// sees only `js`; the vault tools are `tools.*` inside it): it searches,
/// reads the note, then appends to it. The script does not see results;
/// the test checks them in the transcript.
fn script() -> Vec<ScriptedStep> {
    let js = |code: String| ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: format!("call-{}", new_id()),
            name: "js".into(),
            arguments: json!({ "code": code }),
        }],
    };
    vec![
        js(r#"return await tools.search_notes({query: "pantry"});"#.into()),
        js(format!(
            r#"return await tools.read_page({{page: "{NOTE}", block_ids: true}});"#
        )),
        js(format!(
            r#"return await tools.edit_page({{page: "{NOTE}", op: "append", markdown: "{ADDED}"}});"#
        )),
    ]
}

struct Hub {
    server: Option<Server>,
    idp: Option<Idp>,
    url: String,
    http: reqwest::Client,
    _dirs: Vec<TempDir>,
}

async fn start_hub() -> Hub {
    let idp = start_idp().await;
    let data = TempDir::new("hub");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");
    let config = Config {
        mcp_agent_enabled: false,
        mcp_agent_default_host: "hub".into(),
        listen: addr.to_string(),
        data_dir: data.0.clone(),
        public_url: url.clone(),
        trust_proxy: false,
        import_source_limit_bytes: Config::default().import_source_limit_bytes,
        schedules: Default::default(),
        mcp_oauth: Default::default(),
        oidc: OidcConfig {
            issuer: idp.url.clone(),
            client_id: CLIENT_ID.into(),
            client_secret: CLIENT_SECRET.into(),
            allowed_users: vec![ANN_EMAIL.into()],
            allowed_groups: vec![],
            scopes: None,
        },
        agent: AgentSection {
            execution_mode: vault_agent::bubblepaw_core::ExecutionMode::Js,
            default_model: "scripted/scripted".into(),
            models: vec!["scripted/scripted".into()],
            ..AgentSection::default()
        },
        ..Config::default()
    };
    let hooks: AgentTestHooks = Arc::new(|| TestHooks {
        scripted: Some((script(), FINAL.into())),
        ..TestHooks::default()
    });
    let server = notes_hub::start_with(listener, config, Some(hooks))
        .await
        .unwrap();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Hub {
        server: Some(server),
        idp: Some(idp),
        url,
        http,
        _dirs: vec![data],
    }
}

impl Hub {
    fn server(&self) -> &Server {
        self.server.as_ref().unwrap()
    }

    async fn location(&self, url: &str) -> Url {
        let r = self.http.get(url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FOUND, "GET {url}");
        Url::parse(r.headers()[header::LOCATION].to_str().unwrap()).unwrap()
    }

    /// The browser part of a login, then `/auth/redeem`: the body the app
    /// gets (token, label and account).
    async fn sign_in(&self, device: &str, name: &str) -> Value {
        let mut start = Url::parse(&format!("{}/auth/start", self.url)).unwrap();
        start
            .query_pairs_mut()
            .append_pair("device", device)
            .append_pair("name", name)
            .append_pair("redirect", APP_REDIRECT)
            .append_pair("state", "app-state-1");
        let authorize = self.location(start.as_str()).await;
        assert!(authorize
            .as_str()
            .starts_with(&self.idp.as_ref().unwrap().url));
        let callback = self.location(authorize.as_str()).await;
        let back = self.location(callback.as_str()).await;
        assert!(back.as_str().starts_with(APP_REDIRECT), "{back}");
        let code = param(&back, "code").expect("a code");
        let r = self
            .http
            .post(format!("{}/auth/redeem", self.url))
            .json(&json!({ "code": code, "device": device }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        r.json().await.unwrap()
    }

    /// Stops the hub (closing its sockets and agent) and the issuer.
    async fn stop(mut self) {
        self.server.take().unwrap().stop().await;
        self.idp.take().unwrap().stop().await;
    }
}

fn param(u: &Url, k: &str) -> Option<String> {
    u.query_pairs()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.into_owned())
}

async fn sync(c: &Arc<SyncClient>) {
    let c = c.clone();
    tokio::task::spawn_blocking(move || c.sync_once().map(|_| ()))
        .await
        .unwrap()
        .unwrap();
}

// ---- the chat socket (as in tests/agent.rs) ----

/// A JSON-RPC client over one socket.
struct Client {
    tx: mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<w::Response>>>>,
    events: mpsc::UnboundedReceiver<w::AgentEvent>,
    closed: oneshot::Receiver<()>,
    next: u64,
}

async fn connect(base: &str, token: &str, device: &str) -> Client {
    let mut req = format!("{}/agent/ws", base.replacen("http://", "ws://", 1))
        .into_client_request()
        .unwrap();
    let h = req.headers_mut();
    h.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    h.insert("X-Device", HeaderValue::from_str(device).unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
    });
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<w::Response>>>> = Arc::default();
    let (ev_tx, events) = mpsc::unbounded_channel();
    let (closed_tx, closed) = oneshot::channel();
    let p = pending.clone();
    tokio::spawn(async move {
        while let Some(Ok(m)) = stream.next().await {
            let Message::Text(t) = m else {
                if matches!(m, Message::Close(_)) {
                    break;
                }
                continue;
            };
            match serde_json::from_str::<w::ServerMessage>(t.as_str()).unwrap() {
                w::ServerMessage::Notification(n) => {
                    assert_eq!(n.method, "event");
                    let _ = ev_tx.send(serde_json::from_value(n.params).unwrap());
                }
                w::ServerMessage::Response(r) => {
                    let id = r.id.as_u64().unwrap();
                    if let Some(s) = p.lock().unwrap().remove(&id) {
                        let _ = s.send(r);
                    }
                }
            }
        }
        let _ = closed_tx.send(());
    });
    Client {
        tx,
        pending,
        events,
        closed,
        next: 1,
    }
}

impl Client {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, w::RpcError> {
        let id = self.next;
        self.next += 1;
        let (s, r) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, s);
        let req = w::Request::new(id, method, params);
        self.tx
            .send(Message::Text(serde_json::to_string(&req).unwrap().into()))
            .unwrap();
        let r = tokio::time::timeout(Duration::from_secs(60), r)
            .await
            .expect("response before timeout")
            .expect("response");
        match r.error {
            Some(e) => Err(e),
            None => Ok(r.result.unwrap_or(Value::Null)),
        }
    }

    async fn event(&mut self, stop: impl Fn(&w::AgentEvent) -> bool) -> w::AgentEvent {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let e = self.events.recv().await.expect("an event");
                if stop(&e) {
                    return e;
                }
            }
        })
        .await
        .expect("event before timeout")
    }
}

fn typed<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

// ---- tests ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_redeem_gives_the_account_beside_token_and_label() {
    let hub = start_hub().await;
    let device = TempDir::new("login").open().device_id();
    let body = hub.sign_in(&device, "phone").await;
    let keys: std::collections::BTreeSet<_> = body.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["account", "label", "token"].map(String::from).into());
    assert_eq!(body["label"], "phone");
    assert_eq!(body["account"], ANN_EMAIL);
    assert!(body["token"].as_str().unwrap().len() >= 32);
    hub.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_note_is_searched_read_and_edited_by_the_hub_agent() {
    let hub = start_hub().await;

    // The device writes a note and syncs it up to the hub.
    let dev_dir = TempDir::new("device");
    let device = dev_dir.open();
    let device_id = device.device_id();
    let page = device.create_page(None, NOTE.into()).unwrap();
    device
        .replace_markdown(page.clone(), vec![], SEED.into())
        .unwrap();
    let body = hub.sign_in(&device_id, "phone").await;
    assert_eq!(body["account"], ANN_EMAIL);
    let token = body["token"].as_str().unwrap().to_string();
    let client = Arc::new(SyncClient::new(device.clone(), &hub.url, &token));
    sync(&client).await;
    assert!(hub
        .server()
        .hub()
        .vault()
        .page_markdown(page.clone())
        .unwrap()
        .contains("Rice"));

    // A chat over /agent/ws; the scripted model runs search, read and edit.
    let mut c = connect(&hub.url, &token, &device_id).await;
    let s: w::AgentSession = typed(
        c.call("create_session", json!({"title": "Pantry"}))
            .await
            .unwrap(),
    );
    c.call("subscribe", Value::Null).await.unwrap();
    let sent: w::AgentSent = typed(
        c.call(
            "send",
            json!({ "session_id": s.id, "text": "Add oat milk to the pantry plan" }),
        )
        .await
        .unwrap(),
    );
    assert!(sent.started, "{sent:?}");

    // Live events until the run finishes: every tool finishes without error
    // (the model's js calls, and the vault tools they run inside), in this
    // order, and the run names this device as its origin.
    let mut tools_finished = Vec::new();
    let mut origin = None;
    let finished = loop {
        let ev = c.event(|e| e.session_id == s.id).await;
        match ev.kind.as_str() {
            "run_started" => origin = ev.origin_id.clone(),
            "tool_finished" => {
                assert!(!ev.is_error, "{ev:?}");
                tools_finished.push(ev.tool_name.clone().unwrap_or_default());
            }
            "run_finished" => break ev,
            _ => {}
        }
    };
    assert_eq!(
        finished.status.as_deref(),
        Some("completed"),
        "{finished:?}"
    );
    assert_eq!(
        tools_finished,
        ["search_notes", "js", "read_page", "js", "edit_page", "js"]
    );
    assert_eq!(origin.as_deref(), Some(device_id.as_str()));

    // The transcript: the run's text, the tool results and the edit record.
    let st: w::AgentSessionState = typed(
        c.call("session_state", json!({ "session_id": s.id }))
            .await
            .unwrap(),
    );
    let run = st.runs.last().expect("a run");
    assert_eq!(run.status, "completed", "{run:?}");
    assert_eq!(finished.text.as_deref(), Some(FINAL), "{finished:?}");
    assert!(
        st.messages
            .iter()
            .any(|m| m.content == FINAL || m.text == FINAL),
        "the final answer is in the transcript"
    );
    let results: Vec<&str> = st
        .messages
        .iter()
        .filter(|m| m.tool_call_id.is_some())
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(
        results[0].contains(NOTE),
        "search found the note: {}",
        results[0]
    );
    assert!(
        results[1].contains("Eggs"),
        "read the seeded text: {}",
        results[1]
    );
    assert!(results[2].contains("Added 1 block(s)"), "{}", results[2]);
    let names: Vec<&str> = st.activity.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["search_notes", "read_page", "edit_page"]);
    assert!(st.activity.iter().all(|a| !a.is_error));
    assert!(st.activity[2].edit_id.is_some());
    assert_eq!(st.edits.len(), 1, "{:?}", st.edits);
    assert_eq!(st.edits[0].page_id, page);
    assert_eq!(st.edits[0].page_title, NOTE);
    assert!(st.edits[0].added >= 1 && !st.edits[0].undone);

    // The edit is on the hub, and the device pulls it back.
    let hub_md = hub
        .server()
        .hub()
        .vault()
        .page_markdown(page.clone())
        .unwrap();
    assert!(
        hub_md.contains("Oat milk (added by the hub agent)"),
        "{hub_md}"
    );
    sync(&client).await;
    let device_md = device.page_markdown(page.clone()).unwrap();
    assert!(device_md.contains("Eggs"), "{device_md}");
    assert!(
        device_md.contains("Oat milk (added by the hub agent)"),
        "{device_md}"
    );
    assert_eq!(device_md, hub_md);

    // Stop: the hub closes the chat socket, then the hub and issuer stop.
    drop(c.tx);
    hub.stop().await;
    tokio::time::timeout(Duration::from_secs(5), &mut c.closed)
        .await
        .expect("the chat socket closed with the hub")
        .unwrap();
}
