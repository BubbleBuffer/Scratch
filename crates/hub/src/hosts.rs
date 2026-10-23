//! Execution hosts that connect out to the hub: `GET /host/ws`.
//!
//! A desktop with "Let the hub use this computer" on keeps one socket here,
//! authenticated with its device token exactly like `/agent/ws`. It
//! registers under its device's name (the token's label). `/mcp/agent`
//! sessions that select that host send their host tool calls down the socket
//! (`notes_agent_wire::host`); the desktop runs them and answers.
//!
//! - One connection per name: a new one replaces (closes) the old one.
//! - A closed connection leaves the host listed offline; calls to it fail
//!   with "<name> is offline". Nothing survives a hub restart: desktops
//!   reconnect on their own.
//! - Each call has a timeout ([`DEFAULT_CALL_TIMEOUT`]); when it fires the
//!   hub sends `cancel` for that request and the call fails.
//! - Pings, silence and token checks follow `/agent/ws`; signing the device
//!   out closes its socket.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use vault_agent::wire::notes_agent_wire as w;
use vault_core::sha256_hex;
use vault_sync::DEVICE_HEADER;

use crate::agent::{check, PING_EVERY, SILENCE};
use crate::AppState;

/// How long one relayed call may take, `exec` included.
pub(crate) const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

type Reply = Result<Value, String>;

/// One live desktop socket.
pub(crate) struct HostConn {
    id: u64,
    pub name: String,
    token_hash: String,
    out: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    next_request: AtomicU64,
    close: CancellationToken,
}

impl HostConn {
    fn offline(&self) -> String {
        offline(&self.name)
    }

    /// Sends one request and waits for its answer, the socket closing or
    /// `timeout`. A timeout cancels the request on the desktop.
    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let frame = serde_json::to_string(&w::Request::new(id, method, params))
            .map_err(|e| e.to_string())?;
        if self.close.is_cancelled() || self.out.send(frame).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(self.offline());
        }
        tokio::select! {
            reply = rx => reply.unwrap_or_else(|_| Err(self.offline())),
            _ = self.close.cancelled() => {
                self.pending.lock().unwrap().remove(&id);
                Err(self.offline())
            }
            _ = tokio::time::sleep(timeout) => {
                self.pending.lock().unwrap().remove(&id);
                let cancel = json!({
                    "jsonrpc": w::JSONRPC,
                    "method": w::host::method::CANCEL,
                    "params": w::host::CancelParams { id: Value::from(id) },
                });
                let _ = self.out.send(cancel.to_string());
                Err(format!(
                    "{} did not finish within {} s; the call was cancelled",
                    self.name,
                    timeout.as_secs()
                ))
            }
        }
    }

    /// Routes one answer frame to its waiting request.
    fn answer(&self, response: w::Response) {
        let Some(id) = response.id.as_u64() else {
            return;
        };
        let Some(tx) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let reply = match (response.result, response.error) {
            (_, Some(error)) => Err(error.message),
            (Some(result), None) => Ok(result),
            (None, None) => Ok(Value::Null),
        };
        let _ = tx.send(reply);
    }
}

pub(crate) fn offline(name: &str) -> String {
    format!("{name} is offline")
}

/// What `/mcp/agent` finds for a host name.
pub(crate) enum Lookup {
    Online(Arc<HostConn>),
    Offline,
    Unknown,
}

/// The hosts seen since this hub started, by name.
pub(crate) struct Hosts {
    by_name: Mutex<BTreeMap<String, Option<Arc<HostConn>>>>,
    next_conn: AtomicU64,
    timeout: Mutex<Duration>,
}

impl Hosts {
    pub fn new() -> Self {
        Self {
            by_name: Mutex::new(BTreeMap::new()),
            next_conn: AtomicU64::new(1),
            timeout: Mutex::new(DEFAULT_CALL_TIMEOUT),
        }
    }

    pub fn call_timeout(&self) -> Duration {
        *self.timeout.lock().unwrap()
    }

    pub fn set_call_timeout(&self, timeout: Duration) {
        *self.timeout.lock().unwrap() = timeout;
    }

    /// Registers a connection under `name`, replacing (closing) any other.
    fn register(&self, name: &str, token: &str, out: mpsc::UnboundedSender<String>) -> Arc<HostConn> {
        let conn = Arc::new(HostConn {
            id: self.next_conn.fetch_add(1, Ordering::Relaxed),
            name: name.to_owned(),
            token_hash: sha256_hex(token.as_bytes()),
            out,
            pending: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            close: CancellationToken::new(),
        });
        let old = self
            .by_name
            .lock()
            .unwrap()
            .insert(name.to_owned(), Some(conn.clone()));
        if let Some(Some(old)) = old {
            tracing::info!("host socket: a new connection replaced the old one");
            old.close.cancel();
        }
        conn
    }

    /// Marks the host offline, unless a newer connection replaced this one.
    fn unregister(&self, conn: &HostConn) {
        let mut hosts = self.by_name.lock().unwrap();
        if let Some(slot) = hosts.get_mut(&conn.name) {
            if slot.as_ref().is_some_and(|c| c.id == conn.id) {
                *slot = None;
            }
        }
    }

    /// The host called `name`: an exact match, else the one host whose name
    /// matches ignoring case.
    pub fn lookup(&self, name: &str) -> Lookup {
        let hosts = self.by_name.lock().unwrap();
        let slot = hosts.get(name).or_else(|| {
            let mut matches = hosts
                .iter()
                .filter(|(known, _)| known.to_lowercase() == name.to_lowercase());
            match (matches.next(), matches.next()) {
                (Some((_, slot)), None) => Some(slot),
                _ => None,
            }
        });
        match slot {
            Some(Some(conn)) if !conn.close.is_cancelled() => Lookup::Online(conn.clone()),
            Some(_) => Lookup::Offline,
            None => Lookup::Unknown,
        }
    }

    /// `(name, online)` for every host seen since start, by name.
    pub fn list(&self) -> Vec<(String, bool)> {
        self.by_name
            .lock()
            .unwrap()
            .iter()
            .map(|(name, conn)| {
                (
                    name.clone(),
                    conn.as_ref().is_some_and(|c| !c.close.is_cancelled()),
                )
            })
            .collect()
    }

    /// Closes the sockets opened with `token` (its device signed out).
    pub fn close_token(&self, token: &str) {
        let hash = sha256_hex(token.as_bytes());
        for conn in self.by_name.lock().unwrap().values().flatten() {
            if conn.token_hash == hash {
                conn.close.cancel();
            }
        }
    }

    pub fn shutdown(&self) {
        for conn in self.by_name.lock().unwrap().values().flatten() {
            conn.close.cancel();
        }
    }

    /// Live sockets (tests).
    pub fn online(&self) -> usize {
        self.list().iter().filter(|(_, online)| *online).count()
    }
}

/// `GET /host/ws`.
pub(crate) async fn ws(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    if !state.mcp_agent.enabled {
        return (StatusCode::NOT_FOUND, "the agent gateway is off").into_response();
    }
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
    let name = state
        .hub
        .devices()
        .into_iter()
        .find(|d| d.device.as_deref() == Some(device.as_str()))
        .map(|d| d.label.trim().to_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return (StatusCode::CONFLICT, "this device has no name on the hub").into_response();
    }
    if name.eq_ignore_ascii_case("hub") {
        return (StatusCode::CONFLICT, "a device named hub cannot be a host").into_response();
    }
    let upgrade = match upgrade {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    upgrade.on_upgrade(move |socket| async move {
        let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
        let conn = state.hosts.register(&name, &token, out_tx);
        serve(&state, &conn, &token, &device, socket, out_rx).await;
        conn.close.cancel();
        // Waiting calls see the closed socket and fail as offline.
        conn.pending.lock().unwrap().clear();
        state.hosts.unregister(&conn);
    })
}

async fn serve(
    state: &AppState,
    conn: &HostConn,
    token: &str,
    device: &str,
    socket: WebSocket,
    mut out_rx: mpsc::UnboundedReceiver<String>,
) {
    let (mut tx, mut rx) = socket.split();
    let close = conn.close.clone();
    let writer = async {
        let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        loop {
            let frame = tokio::select! {
                biased;
                _ = close.cancelled() => break,
                Some(m) = out_rx.recv() => Message::Text(m.into()),
                _ = ping.tick() => {
                    if check(&state.hub, token, device).await.is_err() {
                        tracing::info!("host socket: token no longer valid, closing");
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
                    tracing::info!("host socket: silent for {}s, dropping", SILENCE.as_secs());
                    break;
                }
                Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => break,
                Ok(Some(Ok(Message::Text(t)))) => {
                    // The desktop only answers; anything else is ignored.
                    if let Ok(response) = serde_json::from_str::<w::Response>(t.as_str()) {
                        conn.answer(response);
                    }
                }
                Ok(Some(Ok(_))) => {}
            }
        }
        close.cancel();
    };
    tokio::join!(writer, reader);
}
