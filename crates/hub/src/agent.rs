//! The hub's agent: `vault_agent` over the hub's own vault replica, served
//! at `GET /agent/ws` as JSON-RPC 2.0 over a WebSocket (one text frame per
//! message). Protocol: `wiki/architecture/hub.md` (`/agent/ws`).
//!
//! - Auth: the upgrade request's `Authorization: Bearer <token>` and
//!   `X-Device`, checked by `Hub::handle` itself, exactly as for sync.
//! - The agent opens at start; if that fails, sync keeps working, the socket
//!   route answers 503 with the reason, and the next connection retries.
//! - Every command names the device as its `client` (event origin).
//! - The server pings every 30 s and drops a socket after 90 s of silence.
//!   At each ping it checks the token again; sign-out closes the device's
//!   sockets at once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vault_agent::bubblepaw_core::ClientInfo;
use vault_agent::rpc::Dispatcher;
use vault_agent::wire::notes_agent_wire as w;
use vault_agent::{Agent, AgentConfig, HostKind, PermissionMode, TestHooks};
use vault_core::{sha256_hex, Vault};
use vault_sync::{Hub, HubRequest, DEVICE_HEADER};

use crate::config::Config;
use crate::AppState;

/// How often the server pings (and re-checks the socket's token).
pub(crate) const PING_EVERY: Duration = Duration::from_secs(30);
/// A socket silent this long is dropped.
pub(crate) const SILENCE: Duration = Duration::from_secs(90);

/// Test hook: makes the agent's test hooks (scripted provider, the
/// `test_confirm` tool) for each open.
#[doc(hidden)]
pub type AgentTestHooks = Arc<dyn Fn() -> TestHooks + Send + Sync>;

/// An `Arc<Agent>` that is never dropped for the last time inside the
/// async runtime (the agent owns a Tokio runtime, which must be dropped on
/// a thread that may block).
pub(crate) struct Shared(Option<Arc<Agent>>);

impl Clone for Shared {
    fn clone(&self) -> Shared {
        Shared(self.0.clone())
    }
}

impl std::ops::Deref for Shared {
    type Target = Agent;
    fn deref(&self) -> &Agent {
        self.0.as_ref().expect("agent")
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let Some(agent) = self.0.take().and_then(Arc::into_inner) else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            let _ = std::thread::Builder::new()
                .name("agent-close".into())
                .spawn(move || drop(agent));
        } else {
            drop(agent);
        }
    }
}

struct Socket {
    token_hash: String,
    close: CancellationToken,
}

/// The agent slot and the open sockets.
pub(crate) struct AgentHost {
    enabled: bool,
    data_dir: PathBuf,
    default_model: String,
    execution_mode: vault_agent::bubblepaw_core::ExecutionMode,
    enable_js: bool,
    bun: String,
    claude_code_extension: String,
    vault: Arc<Vault>,
    hooks: Option<AgentTestHooks>,
    slot: tokio::sync::Mutex<Option<Shared>>,
    sockets: Mutex<HashMap<u64, Socket>>,
    next_socket: AtomicU64,
    scheduler: Option<Arc<vault_agent::schedules::Scheduler>>,
    scheduler_error: Option<String>,
}

fn opt_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

impl AgentHost {
    pub(crate) fn new(
        config: &Config,
        vault: Arc<Vault>,
        hooks: Option<AgentTestHooks>,
        scheduler: Option<Arc<vault_agent::schedules::Scheduler>>,
        scheduler_error: Option<String>,
    ) -> Self {
        let a = &config.agent;
        AgentHost {
            enabled: a.enabled,
            data_dir: config.data_dir.clone(),
            default_model: a.default_model.clone(),
            execution_mode: a.execution_mode,
            enable_js: a.enable_js,
            bun: a.bun.clone(),
            claude_code_extension: a.claude_code_extension.clone(),
            vault,
            hooks,
            slot: tokio::sync::Mutex::new(None),
            sockets: Mutex::new(HashMap::new()),
            next_socket: AtomicU64::new(1),
            scheduler,
            scheduler_error,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    fn config(&self) -> AgentConfig {
        AgentConfig {
            // The agent keeps `agent/` (store, workspace) inside it.
            data_dir: self.data_dir.clone(),
            default_model: self.default_model.clone(),
            bun_path: opt_path(&self.bun),
            claude_code_dir: opt_path(&self.claude_code_extension),
            permission_mode: PermissionMode::Bypass,
            host_kind: HostKind::Hub,
            // Hub hosts never register WorkspaceTools or the desktop process pack.
            packs: vec![],
            test: self.hooks.as_ref().map(|h| h()).unwrap_or_default(),
        }
    }

    /// The open agent, opening it if it is not open yet (also after a
    /// failed open). Err: why it is not available.
    pub(crate) async fn get(&self) -> Result<Shared, String> {
        if !self.enabled {
            return Err("the agent is off on this hub ([agent] enabled = false)".into());
        }
        let mut slot = self.slot.lock().await;
        if let Some(a) = &*slot {
            return Ok(a.clone());
        }
        let config = self.config();
        let execution_mode = self.execution_mode;
        let enable_js = self.enable_js;
        let vault = self.vault.clone();
        let opened = tokio::task::spawn_blocking(move || {
            Agent::open_with_execution(vault, config, execution_mode, enable_js)
        })
        .await;
        match opened {
            Ok(Ok(agent)) => {
                if let Some(scheduler) = &self.scheduler {
                    agent.set_scheduler(scheduler.clone());
                } else if let Some(error) = &self.scheduler_error {
                    agent.set_scheduler_error(error.clone());
                }
                let agent = Shared(Some(Arc::new(agent)));
                tracing::info!("agent open (default model {})", agent.default_model());
                *slot = Some(agent.clone());
                Ok(agent)
            }
            Ok(Err(e)) => {
                let m = format!("the agent failed to open: {e:#}");
                tracing::error!("{m}");
                Err(m)
            }
            Err(e) => {
                let m = format!("the agent failed to open: {e}");
                tracing::error!("{m}");
                Err(m)
            }
        }
    }

    fn register(&self, token: &str) -> (u64, CancellationToken) {
        let id = self.next_socket.fetch_add(1, Ordering::Relaxed);
        let close = CancellationToken::new();
        self.sockets.lock().unwrap().insert(
            id,
            Socket {
                token_hash: sha256_hex(token.as_bytes()),
                close: close.clone(),
            },
        );
        (id, close)
    }

    fn unregister(&self, id: u64) {
        self.sockets.lock().unwrap().remove(&id);
    }

    /// Closes the sockets opened with `token` (its device signed out).
    pub(crate) fn close_token(&self, token: &str) {
        let hash = sha256_hex(token.as_bytes());
        for s in self.sockets.lock().unwrap().values() {
            if s.token_hash == hash {
                s.close.cancel();
            }
        }
    }

    /// Open sockets (tests).
    pub(crate) fn socket_count(&self) -> usize {
        self.sockets.lock().unwrap().len()
    }

    /// Closes every socket and shuts the agent down.
    pub(crate) async fn shutdown(&self) {
        for s in self.sockets.lock().unwrap().values() {
            s.close.cancel();
        }
        let agent = self.slot.lock().await.take();
        // Off the runtime, so the agent's own runtime can stop.
        let _ = tokio::task::spawn_blocking(move || {
            let _ = std::thread::spawn(move || drop(agent)).join();
        })
        .await;
    }
}

/// Checks the bearer token and `X-Device` with `Hub::handle` itself, on a
/// path it does not route: an authorized request gets its 404 "no route",
/// every refusal the same status and message as on `/sync`.
fn check_device(hub: &Hub, token: &str, device: &str) -> Result<(), (StatusCode, String)> {
    let reply = hub.handle(HubRequest {
        method: "GET".into(),
        path: "/agent/ws".into(),
        query: String::new(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {token}")),
            (DEVICE_HEADER.into(), device.into()),
        ],
        body: Box::new(std::io::empty()),
        body_len: Some(0),
        cancel: None,
    });
    let body = String::from_utf8_lossy(&reply.body).into_owned();
    if reply.status == 404 && body.starts_with("no route") {
        return Ok(());
    }
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    Err((status, body))
}

async fn check(hub: &Arc<Hub>, token: &str, device: &str) -> Result<(), (StatusCode, String)> {
    let (hub, token, device) = (hub.clone(), token.to_string(), device.to_string());
    tokio::task::spawn_blocking(move || check_device(&hub, &token, &device))
        .await
        .unwrap_or_else(|e| Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())))
}

/// `GET /agent/ws`.
pub(crate) async fn ws(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    let device = headers
        .get(DEVICE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if token.is_empty() {
        return (StatusCode::UNAUTHORIZED, "missing token").into_response();
    }
    if let Err(refused) = check(&state.hub, &token, &device).await {
        return refused.into_response();
    }
    let agent = match state.agent.get().await {
        Ok(a) => a,
        Err(why) => return (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
    };
    let upgrade = match upgrade {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let label = state
        .hub
        .devices()
        .into_iter()
        .find(|d| d.device.as_deref() == Some(device.as_str()))
        .map(|d| d.label);
    let client = ClientInfo {
        id: device.clone(),
        name: label,
    };
    let (id, close) = state.agent.register(&token);
    upgrade.on_upgrade(move |socket| async move {
        let conn = Conn {
            agent,
            client,
            hub: state.hub.clone(),
            token,
            device,
            close,
        };
        conn.serve(socket).await;
        state.agent.unregister(id);
    })
}

struct Conn {
    agent: Shared,
    client: ClientInfo,
    hub: Arc<Hub>,
    token: String,
    device: String,
    close: CancellationToken,
}

/// What a request handler needs, cloned into its task.
#[derive(Clone)]
struct Ctx {
    client: ClientInfo,
    out: mpsc::UnboundedSender<String>,
    dispatcher: Dispatcher,
}

impl Conn {
    async fn serve(self, socket: WebSocket) {
        let (mut tx, mut rx) = socket.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let dispatcher = Dispatcher::new(self.agent.0.as_ref().expect("agent").clone());
        dispatcher.set_client_info(self.client.clone());
        let mut events = dispatcher.subscribe(&self.client.id);
        let ctx = Ctx {
            client: self.client.clone(),
            out: out_tx,
            dispatcher,
        };
        let close = self.close.clone();

        let writer = async {
            let mut ping =
                tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
            loop {
                let frame = tokio::select! {
                    biased;
                    _ = close.cancelled() => break,
                    Some(m) = out_rx.recv() => Message::Text(m.into()),
                    Some(m) = events.next() => Message::Text(m.to_string().into()),
                    _ = ping.tick() => {
                        if check(&self.hub, &self.token, &self.device).await.is_err() {
                            tracing::info!("agent socket: token no longer valid, closing");
                            close.cancel();
                            break;
                        }
                        Message::Ping(Default::default())
                    }
                };
                if tx.send(frame).await.is_err() {
                    close.cancel();
                    break;
                }
            }
            let _ = tx.send(Message::Close(None)).await;
            let _ = tx.close().await;
        };

        let reader = async {
            loop {
                let next = tokio::select! {
                    _ = close.cancelled() => break,
                    n = tokio::time::timeout(SILENCE, rx.next()) => n,
                };
                match next {
                    Err(_) => {
                        tracing::info!("agent socket: silent for {}s, dropping", SILENCE.as_secs());
                        break;
                    }
                    Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => break,
                    Ok(Some(Ok(Message::Text(t)))) => handle(t.as_str(), &ctx),
                    // Pings, pongs and binary frames only show the peer is alive.
                    Ok(Some(Ok(_))) => {}
                }
            }
            close.cancel();
        };

        tokio::join!(writer, reader);
        drop(events);
    }
}

fn reply(out: &mpsc::UnboundedSender<String>, r: w::Response) {
    if let Ok(text) = serde_json::to_string(&r) {
        let _ = out.send(text);
    }
}

/// One request frame: parsed here, answered from its own blocking task.
fn handle(text: &str, ctx: &Ctx) {
    let request: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            return reply(
                &ctx.out,
                w::Response::err(
                    Value::Null,
                    w::RpcError::new(w::code::PARSE_ERROR, error.to_string()),
                ),
            )
        }
    };
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let dispatcher = ctx.dispatcher.clone();
        let client = ctx.client.id.clone();
        match tokio::task::spawn_blocking(move || dispatcher.handle(&client, request)).await {
            Ok(Some(response)) => {
                let _ = ctx.out.send(response.to_string());
            }
            Ok(None) => {}
            Err(error) => reply(
                &ctx.out,
                w::Response::err(
                    id,
                    w::RpcError::new(w::code::AGENT_ERROR, error.to_string()),
                ),
            ),
        }
    });
}
