//! `GET /agent/ws` on an in-process notes-hub: the agent's test hooks (the
//! `test_confirm` tool, script runs; no real providers), clients over
//! tokio-tungstenite, and a `SyncClient` peer.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use notes_agent_wire as w;
use notes_hub::{AgentSection, Config, Server};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use vault_agent::TestHooks;
use vault_core::{new_id, Vault};
use vault_sync::SyncClient;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_agent_{tag}_{}", new_id()));
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

struct Env {
    data: TempDir,
    server: Option<Server>,
    url: String,
}

impl Env {
    fn server(&self) -> &Server {
        self.server.as_ref().unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // The server's runtime is gone by now; its agent is closed on drop.
        drop(self.server.take());
        let _ = &self.data;
    }
}

async fn env_with(prepare: impl FnOnce(&std::path::Path)) -> Env {
    let data = TempDir::new("data");
    prepare(&data.0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        listen: listener.local_addr().unwrap().to_string(),
        data_dir: data.0.clone(),
        trust_proxy: false,
        agent: AgentSection {
            enable_js: true,
            default_model: "scripted/scripted".into(),
            ..AgentSection::default()
        },
        ..Config::default()
    };
    let hooks: notes_hub::AgentTestHooks = Arc::new(|| TestHooks {
        scripted: Some((vec![], "ok".into())),
        confirm_tool: true,
        ..TestHooks::default()
    });
    let server = notes_hub::start_with(listener, config, Some(hooks))
        .await
        .unwrap();
    Env {
        data,
        server: Some(server),
        url,
    }
}

async fn env() -> Env {
    env_with(|_| {}).await
}

/// A JSON-RPC client over one socket.
struct Client {
    tx: mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<w::Response>>>>,
    events: mpsc::UnboundedReceiver<w::AgentEvent>,
    closed: oneshot::Receiver<()>,
    next: u64,
}

fn ws_url(base: &str) -> String {
    format!("{}/agent/ws", base.replacen("http://", "ws://", 1))
}

async fn connect(
    base: &str,
    token: &str,
    device: &str,
) -> Result<Client, tokio_tungstenite::tungstenite::Error> {
    let mut req = ws_url(base).into_client_request().unwrap();
    let h = req.headers_mut();
    h.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    h.insert("X-Device", HeaderValue::from_str(device).unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(req).await?;
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
    Ok(Client {
        tx,
        pending,
        events,
        closed,
        next: 1,
    })
}

impl Client {
    /// Sends a request; the returned receiver gets its response.
    fn start(&mut self, method: &str, params: Value) -> oneshot::Receiver<w::Response> {
        let id = self.next;
        self.next += 1;
        let (s, r) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, s);
        let req = w::Request::new(id, method, params);
        self.tx
            .send(Message::Text(serde_json::to_string(&req).unwrap().into()))
            .unwrap();
        r
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, w::RpcError> {
        let r = self.start(method, params);
        finish(r).await
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

async fn finish(r: oneshot::Receiver<w::Response>) -> Result<Value, w::RpcError> {
    let r = tokio::time::timeout(Duration::from_secs(60), r)
        .await
        .expect("response before timeout")
        .expect("response");
    match r.error {
        Some(e) => Err(e),
        None => Ok(r.result.unwrap_or(Value::Null)),
    }
}

fn get<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

async fn sync(c: &Arc<SyncClient>) {
    let c = c.clone();
    tokio::task::spawn_blocking(move || c.sync_once().map(|_| ()))
        .await
        .unwrap()
        .unwrap();
}

fn titles(v: &Vault) -> Vec<String> {
    v.list_children(None, 0, 200)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn weekly_schedule_is_accepted_over_rpc_and_runs_without_a_provider() {
    let data = TempDir::new("schedules");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = notes_hub::start_on(
        listener,
        Config {
            data_dir: data.0.clone(),
            trust_proxy: false,
            ..Config::default()
        },
    )
    .await
    .unwrap();
    let mut e = Env {
        data,
        server: Some(server),
        url,
    };
    e.server().schedule_tick_at("2026-10-01T00:00:00Z").unwrap();
    let vault = e.server().hub().vault().clone();
    let node = vault_core::node_record_id(&vault.device_id());
    let record: Value =
        serde_json::from_str(&vault.type_data(node.clone(), "node".into()).unwrap()).unwrap();
    assert_eq!(record["available"], true);
    assert_eq!(record["kind"], "hub");
    assert_eq!(record["steps"], json!(["note.from_template"]));
    let parent = vault.create_page(None, "Journal".into()).unwrap();
    let template = vault
        .create_template(parent.clone(), "Weekly template".into())
        .unwrap();
    let page = vault
        .create_page_with_type(None, "Weekly schedule".into(), Some("schedule".into()))
        .unwrap();
    let raw = json!({"version":1,
        "trigger":{"at":"2026-10-05T08:00", "tz":"Europe/Berlin", "missed":"latest", "rrule":"FREQ=WEEKLY;BYDAY=MO"},
        "runner":node,"tools":["create_page"],
        "steps":[{"id":"note","do":"note.from_template","template":template,"parent":parent,"title":"Week {{week}} {{year}}"}],
        "future":"keep me"
    });
    for (key, value) in raw.as_object().unwrap() {
        vault
            .type_data_set(
                page.clone(),
                "schedule".into(),
                vec![key.clone()],
                value.to_string(),
            )
            .unwrap();
    }
    let token = e.server().hub().pair("scheduler client").unwrap();
    let mut client = connect(&e.url, &token, "scheduler-client").await.unwrap();
    let state = client
        .call("schedule_state", json!({"page_id":page}))
        .await
        .unwrap();
    assert_eq!(state["accepted"], false);
    let hash = state["current_hash"].as_str().unwrap();
    assert!(client
        .call("schedule_accept", json!({"page_id":page,"hash":"stale"}))
        .await
        .is_err());
    let accepted = client
        .call("schedule_accept", json!({"page_id":page,"hash":hash}))
        .await
        .unwrap();
    assert_eq!(accepted["hash_matches"], true);
    e.server().schedule_tick_at("2026-10-05T06:00:00Z").unwrap();
    e.server().schedule_tick_at("2026-10-05T06:00:20Z").unwrap();
    let expected = vault_core::schedule_occurrence_id(&page, "2026-10-05T06:00:00Z");
    let note = vault.load_page(expected.clone()).unwrap();
    assert_eq!(note.title, "Week 41 2026");
    assert_eq!(note.parent, Some(parent.clone()));
    assert_eq!(vault.list_children(Some(parent), 0, 100).unwrap().len(), 1);
    let status: Value = serde_json::from_str(
        &vault
            .type_data(page.clone(), "schedule_status".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status["last_run"]["result"], "created");
    assert_eq!(status["last_run"]["page"], expected);
    assert_eq!(status["next_run"], "2026-10-12T06:00:00Z");
    assert_eq!(status["runner_state"], "ok");
    assert!(client
        .call("runner_available", json!({"available":false}))
        .await
        .is_err());
    let preview = client
        .call("schedule_preview", json!({"json":raw.to_string()}))
        .await
        .unwrap();
    assert_eq!(preview["valid"], true);
    assert_eq!(preview["current_hash"], hash);
    assert_eq!(preview["occurrences"].as_array().unwrap().len(), 3);
    let paused = client
        .call("schedule_pause", json!({"page_id":page,"paused":true}))
        .await
        .unwrap();
    assert_eq!(paused["runner_state"], "paused");
    assert!(client
        .call("schedule_run_now", json!({"page_id":page}))
        .await
        .is_err());
    client
        .call("schedule_pause", json!({"page_id":page,"paused":false}))
        .await
        .unwrap();
    let retry = client
        .call("schedule_run_now", json!({"page_id":page}))
        .await
        .unwrap();
    assert_eq!(retry["result"], "existed");
    assert_eq!(retry["page"], expected);
    let after: Value =
        serde_json::from_str(&vault.type_data(page, "schedule".into()).unwrap()).unwrap();
    assert_eq!(after, raw);
    drop(client);
    e.server.take().unwrap().stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_or_bad_tokens_get_401_before_the_upgrade() {
    let e = env().await;
    let http = reqwest::Client::new();
    let r = http
        .get(format!("{}/agent/ws", e.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = http
        .get(format!("{}/agent/ws", e.url))
        .header("Authorization", "Bearer nope")
        .header("X-Device", "dev-1")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    match connect(&e.url, "nope", "dev-1").await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 401),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("connected with a bad token"),
    }
    // A good token for another device is refused as on /sync.
    let token = e.server().hub().pair("phone").unwrap();
    let mine = connect(&e.url, &token, "dev-1").await.unwrap();
    match connect(&e.url, &token, "dev-2").await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 403),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("connected with another device's token"),
    }
    drop(mine);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreadable_schedule_state_keeps_sync_up_and_reports_the_runner_error() {
    let mut e = env_with(|data| {
        fs::create_dir_all(data.join("agent")).unwrap();
        fs::write(data.join("agent/schedules.json"), "broken state").unwrap();
    })
    .await;
    let response = reqwest::get(format!("{}/health", e.url)).await.unwrap();
    assert_eq!(response.status(), 200);
    let token = e.server().hub().pair("scheduler failure client").unwrap();
    let mut client = connect(&e.url, &token, "scheduler-failure-client")
        .await
        .unwrap();
    let error = client
        .call("schedule_state", json!({"page_id":"any"}))
        .await
        .unwrap_err();
    assert!(
        error.message.contains("schedule runner unavailable"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("schedules.json"),
        "{}",
        error.message
    );
    assert!(client
        .call("sessions", Value::Null)
        .await
        .unwrap()
        .is_array());
    assert_eq!(
        fs::read_to_string(e.data.0.join("agent/schedules.json")).unwrap(),
        "broken state"
    );
    drop(client);
    e.server.take().unwrap().stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_scripts_events_and_a_syncing_peer() {
    let e = env().await;
    let dir = TempDir::new("peer");
    let peer = dir.open();
    let device = peer.device_id();
    let token = e.server().hub().pair("laptop").unwrap();
    let mut c = connect(&e.url, &token, &device).await.unwrap();

    let s: w::AgentSession = get(c
        .call("create_session", json!({"title": "Hub chat"}))
        .await
        .unwrap());
    let list: Vec<w::AgentSession> = get(c.call("sessions", Value::Null).await.unwrap());
    assert!(list.iter().any(|x| x.id == s.id && x.title == "Hub chat"));
    let models: Vec<w::AgentModel> = get(c.call("models", Value::Null).await.unwrap());
    assert!(models.iter().any(|m| m.id == "scripted/scripted"));

    c.call("subscribe", Value::Null).await.unwrap();
    let run: w::AgentRunRecord = get(
        c.call(
            "run_script",
            json!({
                "session_id": s.id,
                "label": "[script] create",
                "code": r#"await tools.create_page({title: "From the hub agent", markdown: "Hello"}); return "made";"#,
            }),
        )
        .await
        .unwrap(),
    );
    assert_eq!(run.status, "completed", "{run:?}");

    // Durable events arrive in increasing order, naming this device.
    let mut seqs = Vec::new();
    let mut origin = None;
    loop {
        let ev = c.event(|e| e.session_id == s.id).await;
        if ev.durable {
            seqs.push(ev.sequence);
        }
        if ev.kind == "run_started" {
            origin = ev.origin_id.clone();
        }
        if ev.kind == "run_finished" {
            break;
        }
    }
    assert!(seqs.len() >= 2, "{seqs:?}");
    assert!(seqs.windows(2).all(|p| p[0] < p[1]), "{seqs:?}");
    assert_eq!(origin.as_deref(), Some(device.as_str()));

    // The page reached the hub's vault; a peer pulls it.
    assert!(titles(e.server().hub().vault()).contains(&"From the hub agent".to_string()));
    let client = Arc::new(SyncClient::new(peer.clone(), &e.url, &token));
    sync(&client).await;
    assert!(titles(&peer).contains(&"From the hub agent".to_string()));

    // State, events_after and typed errors.
    let st: w::AgentSessionState = get(c
        .call("session_state", json!({"session_id": s.id}))
        .await
        .unwrap());
    assert_eq!(st.runs.len(), 1);
    let page: w::AgentEvents = get(c
        .call("events_after", json!({"after": 0, "session_id": s.id}))
        .await
        .unwrap());
    assert!(!page.events.is_empty() && !page.resync);
    let err = c
        .call("session_state", json!({"session_id": "nope"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, w::code::AGENT_ERROR);
    assert_eq!(err.kind(), Some(w::ErrorKind::NotFound), "{err:?}");
    let err = c.call("no_such", Value::Null).await.unwrap_err();
    assert_eq!(err.code, w::code::METHOD_NOT_FOUND);
    let err = c.call("rename_session", json!({})).await.unwrap_err();
    assert_eq!(err.code, w::code::INVALID_PARAMS);

    // Several sockets per device.
    let mut c2 = connect(&e.url, &token, &device).await.unwrap();
    let list: Vec<w::AgentSession> = get(c2.call("sessions", Value::Null).await.unwrap());
    assert!(list.iter().any(|x| x.id == s.id));
    assert_eq!(e.server().agent_sockets(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approvals_round_trip_over_the_socket() {
    let e = env().await;
    let token = e.server().hub().pair("laptop").unwrap();
    let mut c = connect(&e.url, &token, "dev-approve").await.unwrap();
    // Lower-level policy/approval RPCs remain functional for explicit callers.
    c.call(w::method::PERMISSION_MODE_SET, json!({"mode":"manual"}))
        .await
        .unwrap();
    let s: w::AgentSession = get(c
        .call("create_session", json!({"title": "Approve"}))
        .await
        .unwrap());
    c.call("subscribe", Value::Null).await.unwrap();
    let running = c.start(
        "run_script",
        json!({
            "session_id": s.id,
            "label": "[script] confirm",
            "code": r#"return await tools.test_confirm({text: "hi"});"#,
        }),
    );
    let asked = c.event(|e| e.kind == "approval_requested").await;
    let approval = asked.approval.expect("approval");
    assert_eq!(approval.tool, "test_confirm");
    let decided: w::AgentApproval = get(c
        .call(
            "decide_approval",
            json!({"approval_id": approval.id, "decision": "approve"}),
        )
        .await
        .unwrap());
    assert_eq!(decided.state, "approved");
    let resolved = c.event(|e| e.kind == "approval_resolved").await;
    assert_eq!(resolved.origin_id.as_deref(), Some("dev-approve"));
    let run: w::AgentRunRecord = get(finish(running).await.unwrap());
    assert_eq!(run.status, "completed", "{run:?}");
    assert!(run.text.unwrap_or_default().contains("confirmed: hi"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sign_out_closes_the_devices_sockets() {
    let e = env().await;
    let token = e.server().hub().pair("laptop").unwrap();
    let mut c = connect(&e.url, &token, "dev-out").await.unwrap();
    c.call("sessions", Value::Null).await.unwrap();
    let r = reqwest::Client::new()
        .post(format!("{}/auth/signout", e.url))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    tokio::time::timeout(Duration::from_secs(5), &mut c.closed)
        .await
        .expect("socket closed after sign-out")
        .unwrap();
    match connect(&e.url, &token, "dev-out").await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 401),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("connected after sign-out"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_that_cannot_open_gives_503_while_sync_works() {
    // A file where the agent's folder should be: the agent cannot open.
    let e = env_with(|data| fs::write(data.join("agent"), b"not a folder").unwrap()).await;
    let dir = TempDir::new("peer");
    let peer = dir.open();
    let token = e.server().hub().pair("laptop").unwrap();
    let r = reqwest::Client::new()
        .get(format!("{}/agent/ws", e.url))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Device", peer.device_id())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    let body = r.text().await.unwrap();
    assert!(body.contains("failed to open"), "{body}");

    e.server()
        .hub()
        .vault()
        .create_page(None, "On the hub".into())
        .unwrap();
    let client = Arc::new(SyncClient::new(peer.clone(), &e.url, &token));
    sync(&client).await;
    assert!(titles(&peer).contains(&"On the hub".to_string()));

    // The next connection retries the open: with the folder fixed it works.
    fs::remove_file(e.data.0.join("agent")).unwrap();
    let mut c = connect(&e.url, &token, &peer.device_id()).await.unwrap();
    c.call("sessions", Value::Null).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_info_reports_the_hubs_base_pack() {
    // The legacy hub config defaults to Ask; neither it nor a saved Ask
    // snapshot may re-enable approvals at startup.
    let e = env_with(|data| {
        let agent = data.join("agent");
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("permission-mode.json"), r#""ask""#).unwrap();
    })
    .await;
    let token = e.server().hub().pair("laptop").unwrap();
    let mut client = connect(&e.url, &token, "dev-host-info").await.unwrap();
    let info: w::HostInfo = get(client
        .call(w::method::HOST_INFO, Value::Null)
        .await
        .unwrap());
    assert_eq!(info.kind, "hub");
    // Settings label the hub's own default with it ("Hub default (...)").
    assert_eq!(info.default_model.as_deref(), Some("scripted/scripted"));
    assert_eq!(info.permission_mode, w::PermissionMode::Bypass);
    assert_eq!(
        fs::read_to_string(e.data.0.join("agent/permission-mode.json")).unwrap(),
        r#""ask""#
    );
    assert!(!info.projects);
    assert_eq!(
        get::<w::PermissionMode>(
            client
                .call(w::method::PERMISSION_MODE_SET, json!({"mode":"bypass"}))
                .await
                .unwrap()
        ),
        w::PermissionMode::Bypass
    );
    let changed: w::HostInfo = get(client
        .call(w::method::HOST_INFO, Value::Null)
        .await
        .unwrap());
    assert_eq!(changed.permission_mode, w::PermissionMode::Bypass);
    for (method, params) in [
        (w::method::PROJECTS_LIST, Value::Null),
        (w::method::PROJECT_ADD, json!({"name":"A","path":"/tmp"})),
        (w::method::PROJECT_REMOVE, json!({"id":"unknown"})),
        (
            w::method::PROJECT_RENAME,
            json!({"id":"unknown","name":"B"}),
        ),
        (
            w::method::SESSION_PROJECT_SET,
            json!({"session_id":"unknown","project_id":"unknown"}),
        ),
    ] {
        let error = client.call(method, params).await.unwrap_err();
        assert!(
            error.message.contains("not supported on this host"),
            "{error:?}"
        );
    }
    assert!(info.packs.is_empty());
    let names: std::collections::BTreeSet<_> = info.tools.iter().map(String::as_str).collect();
    assert_eq!(names.len(), info.tools.len(), "duplicate hub tool names");
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            // The in-app agent's vault tools: every listed registry tool,
            // feedback_submit included. Old and hidden names are not registered here.
            "daily_note",
            "search_notes",
            "memory_read",
            "memory_write",
            "memory_update",
            "memory_delete",
            "read_page",
            "create_page",
            "edit_page",
            "attach_file",
            "update_task",
            "notes_help",
            "upsert_row",
            "trash_page",
            "move_page",
            "feedback_submit",
            "chat_read",
            "ui_open_page",
            "ui_highlight",
            "history_chapters",
            "history_search",
            "history_read",
            "environments",
            "js",
            // env_with enables this fixture-only confirmation tool.
            "test_confirm",
        ])
    );
    for name in ["read", "write", "edit", "search", "list", "exec"] {
        assert!(!names.contains(name), "hub must never register {name}");
    }
    assert!(!info.tools.iter().any(|name| name.starts_with("fs_")));
    assert!(!info.can_run_tasks);
}

/// Integration spec phase 3: a run that names no model uses the vault's
/// default model for `hub` (synced from a peer), over `[agent] default_model`;
/// an explicit model still wins.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runs_without_a_model_use_the_vault_default_for_hub() {
    let mut e = env().await;
    let dir = TempDir::new("defaults_peer");
    let peer = dir.open();
    let device = peer.device_id();
    let token = e.server().hub().pair("laptop").unwrap();
    let mut c = connect(&e.url, &token, &device).await.unwrap();
    c.call("subscribe", Value::Null).await.unwrap();

    async fn run_model(c: &mut Client, model: Option<&str>) -> String {
        let s: w::AgentSession = get(c
            .call("create_session", json!({"title": "Defaults"}))
            .await
            .unwrap());
        let mut params = json!({"session_id": s.id, "text": "hello"});
        if let Some(m) = model {
            params["model"] = json!(m);
        }
        c.call("send", params).await.unwrap();
        loop {
            if c.event(|e| e.session_id == s.id).await.kind == "run_finished" {
                break;
            }
        }
        let st: w::AgentSessionState = get(c
            .call("session_state", json!({"session_id": s.id}))
            .await
            .unwrap());
        st.runs.last().expect("a run").model.clone()
    }

    assert_eq!(run_model(&mut c, None).await, "scripted/scripted", "config default");

    peer.set_agent_default_runner(Some("this_device".into())).unwrap();
    peer.set_agent_default_model("hub".into(), Some("scripted/vault-pick".into()))
        .unwrap();
    peer.set_agent_default_model("this_device".into(), Some("scripted/desktop-only".into()))
        .unwrap();
    let client = Arc::new(SyncClient::new(peer.clone(), &e.url, &token));
    sync(&client).await;
    let hub_defaults = e.server().hub().vault().agent_defaults().unwrap();
    assert_eq!(hub_defaults.model_for("hub"), Some("scripted/vault-pick"));

    assert_eq!(run_model(&mut c, None).await, "scripted/vault-pick", "vault default");
    assert_eq!(
        run_model(&mut c, Some("scripted/scripted")).await,
        "scripted/scripted",
        "an explicit model wins"
    );
    let models: Vec<w::AgentModel> = get(c.call("models", Value::Null).await.unwrap());
    assert!(models.iter().any(|m| m.id == "scripted/vault-pick"), "{models:?}");

    peer.set_agent_default_model("hub".into(), None).unwrap();
    sync(&client).await;
    assert_eq!(run_model(&mut c, None).await, "scripted/scripted", "cleared: config default");
    drop(c);
    e.server.take().unwrap().stop().await;
}
