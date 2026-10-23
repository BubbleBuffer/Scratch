//! Execution hosts that connect out to the hub: `GET /host/ws`.
//!
//! A desktop with "Let the hub use this computer" on keeps one socket here,
//! authenticated with its device token exactly like `/agent/ws`. The first
//! connection marks the device as a host in `tokens.json`; it stays one,
//! online or offline, until it signs out. A host's name is its device's
//! current label, so a rename (`POST /device/name`) takes effect at once and
//! the old name stops working. `/mcp/agent` sends host tool calls down the
//! socket (`notes_agent_wire::host`); the desktop runs them and answers.
//!
//! - One connection per device: a new one replaces (closes) the old one.
//! - A host without a connection is listed offline; calls to it fail with
//!   "<name> is offline". After a hub restart every host is offline until
//!   its desktop reconnects (on its own).
//! - Each call has a timeout ([`DEFAULT_CALL_TIMEOUT`]); when it fires the
//!   hub sends `cancel` for that request and the call fails.
//! - Pings, silence and token checks follow `/agent/ws`; signing the device
//!   out closes its socket.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use vault_agent::wire::notes_agent_wire as w;
use vault_core::sha256_hex;
use vault_sync::{Hub, SyncError, DEVICE_HEADER};

use crate::agent::{check, PING_EVERY, SILENCE};
use crate::AppState;

/// How long one relayed call may take, `exec` included.
pub(crate) const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

type Reply = Result<Value, String>;

/// One live desktop socket.
pub(crate) struct HostConn {
    id: u64,
    pub device: String,
    /// The device's label; a rename updates it.
    name: Mutex<String>,
    token_hash: String,
    out: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    next_request: AtomicU64,
    close: CancellationToken,
}

impl HostConn {
    pub fn name(&self) -> String {
        self.name.lock().unwrap().clone()
    }

    fn offline(&self) -> String {
        offline(&self.name())
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
                    self.name(),
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
    /// A host without a connection, by its name.
    Offline(String),
    Unknown,
}

/// `(device, name)` of every host (devices marked in `tokens.json`).
fn host_devices(hub: &Hub) -> Vec<(String, String)> {
    let mut hosts: Vec<(String, String)> = Vec::new();
    for d in hub.devices() {
        if let (true, Some(device)) = (d.host, d.device) {
            if !hosts.iter().any(|(known, _)| *known == device) {
                hosts.push((device, d.label.trim().to_owned()));
            }
        }
    }
    hosts
}

/// The live host connections, by device id.
pub(crate) struct Hosts {
    by_device: Mutex<HashMap<String, Arc<HostConn>>>,
    next_conn: AtomicU64,
    timeout: Mutex<Duration>,
}

impl Hosts {
    pub fn new() -> Self {
        Self {
            by_device: Mutex::new(HashMap::new()),
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

    /// Registers `device`'s connection, replacing (closing) any other.
    fn register(
        &self,
        device: &str,
        name: &str,
        token: &str,
        out: mpsc::UnboundedSender<String>,
    ) -> Arc<HostConn> {
        let conn = Arc::new(HostConn {
            id: self.next_conn.fetch_add(1, Ordering::Relaxed),
            device: device.to_owned(),
            name: Mutex::new(name.to_owned()),
            token_hash: sha256_hex(token.as_bytes()),
            out,
            pending: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            close: CancellationToken::new(),
        });
        let old = self
            .by_device
            .lock()
            .unwrap()
            .insert(device.to_owned(), conn.clone());
        if let Some(old) = old {
            tracing::info!("host socket: a new connection replaced the old one");
            old.close.cancel();
        }
        conn
    }

    /// Drops the connection, unless a newer one replaced it.
    fn unregister(&self, conn: &HostConn) {
        let mut hosts = self.by_device.lock().unwrap();
        if hosts.get(&conn.device).is_some_and(|c| c.id == conn.id) {
            hosts.remove(&conn.device);
        }
    }

    /// `device`'s live connection.
    pub fn connection(&self, device: &str) -> Option<Arc<HostConn>> {
        self.by_device
            .lock()
            .unwrap()
            .get(device)
            .filter(|conn| !conn.close.is_cancelled())
            .cloned()
    }

    /// The host called `name`: an exact match, else the one host whose name
    /// matches ignoring case.
    pub fn lookup(&self, hub: &Hub, name: &str) -> Lookup {
        let hosts = host_devices(hub);
        let found = hosts.iter().find(|(_, known)| known == name).or_else(|| {
            let lower = name.to_lowercase();
            let mut matches = hosts
                .iter()
                .filter(|(_, known)| known.to_lowercase() == lower);
            match (matches.next(), matches.next()) {
                (Some(found), None) => Some(found),
                _ => None,
            }
        });
        match found {
            Some((device, known)) => match self.connection(device) {
                Some(conn) => Lookup::Online(conn),
                None => Lookup::Offline(known.clone()),
            },
            None => Lookup::Unknown,
        }
    }

    /// `(name, online)` for every host, by name.
    pub fn list(&self, hub: &Hub) -> Vec<(String, bool)> {
        let mut hosts: Vec<(String, bool)> = host_devices(hub)
            .into_iter()
            .map(|(device, name)| (name, self.connection(&device).is_some()))
            .collect();
        hosts.sort();
        hosts
    }

    /// A device was renamed: its live connection takes the new name.
    pub fn renamed(&self, device: &str, name: &str) {
        if let Some(conn) = self.by_device.lock().unwrap().get(device) {
            *conn.name.lock().unwrap() = name.to_owned();
        }
    }

    /// Closes the sockets opened with `token` (its device signed out).
    pub fn close_token(&self, token: &str) {
        let hash = sha256_hex(token.as_bytes());
        for conn in self.by_device.lock().unwrap().values() {
            if conn.token_hash == hash {
                conn.close.cancel();
            }
        }
    }

    pub fn shutdown(&self) {
        for conn in self.by_device.lock().unwrap().values() {
            conn.close.cancel();
        }
    }

    /// Live sockets (tests).
    pub fn online(&self) -> usize {
        self.by_device
            .lock()
            .unwrap()
            .values()
            .filter(|conn| !conn.close.is_cancelled())
            .count()
    }
}

/// The bearer token and `X-Device` of a device request, checked.
async fn device_auth(state: &AppState, headers: &HeaderMap) -> Result<(String, String), Response> {
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
        return Err((StatusCode::UNAUTHORIZED, "missing token").into_response());
    }
    if let Err(refused) = check(&state.hub, &token, &device).await {
        return Err(refused.into_response());
    }
    Ok((token, device))
}

#[derive(serde::Deserialize)]
pub(crate) struct RenameBody {
    name: String,
}

/// `POST /device/name` `{"name": ...}`: renames the calling device. The
/// name is trimmed and must be unique among hosts, ignoring case. Answers
/// `{"name": <name used>}`, or `{"error": ...}` with 400 (invalid) or 409
/// ("Another computer is already called X"). Takes effect at once for
/// `/mcp/agent`; the old name stops working.
pub(crate) async fn rename(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Option<Json<RenameBody>>,
) -> Response {
    let (_, device) = match device_auth(&state, &headers).await {
        Ok(auth) => auth,
        Err(refused) => return refused,
    };
    let Some(Json(body)) = body else {
        return rename_error(StatusCode::BAD_REQUEST, "expected {\"name\": ...}");
    };
    let hub = state.hub.clone();
    let renamed = {
        let device = device.clone();
        tokio::task::spawn_blocking(move || hub.rename_device(&device, &body.name)).await
    };
    match renamed {
        Ok(Ok(name)) => {
            state.hosts.renamed(&device, &name);
            tracing::info!("device renamed");
            Json(json!({ "name": name })).into_response()
        }
        Ok(Err(SyncError::Status(status, message))) => rename_error(
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            &message,
        ),
        Ok(Err(error)) => {
            tracing::warn!("device rename failed: {error}");
            rename_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
        }
        Err(_) => rename_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
    }
}

fn rename_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
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
    let (token, device) = match device_auth(&state, &headers).await {
        Ok(auth) => auth,
        Err(refused) => return refused,
    };
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
    let hub = state.hub.clone();
    let marked = {
        let device = device.clone();
        tokio::task::spawn_blocking(move || hub.mark_host(&device)).await
    };
    if !matches!(marked, Ok(Ok(()))) {
        tracing::warn!("host socket: could not mark the device as a host");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not register the host",
        )
            .into_response();
    }
    upgrade.on_upgrade(move |socket| async move {
        let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
        let conn = state.hosts.register(&device, &name, &token, out_tx);
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
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
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
