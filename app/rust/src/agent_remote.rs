//! `NotesAgent::connect`: the agent on a notes-hub, over `GET /agent/ws`
//! (JSON-RPC 2.0, one text frame per message; `notes_agent_wire`). Builds
//! on every target, Android included: tokio-tungstenite with rustls (ring,
//! webpki roots).
//!
//! - One socket. It pings every 30 s and is dropped after 90 s of silence.
//! - When it drops: reconnect with backoff (1 s, doubling, up to 30 s),
//!   subscribe again if subscribed, then push a `gap` event (sequence = the
//!   last durable sequence delivered) so the app catches up with
//!   `events_after`.
//! - Calls made while disconnected fail at once with "hub unreachable: ...";
//!   nothing is queued.
//! - `close_events` and drop close the socket at once.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use notes_agent_wire as w;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::api::agent::*;

const PING_EVERY: Duration = Duration::from_secs(30);
const SILENCE: Duration = Duration::from_secs(90);
const BACKOFF_FIRST: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Reply = Result<Value, String>;

/// Receives events; returning false drops it.
pub(crate) type EventSink = Box<dyn FnMut(AgentEvent) -> bool + Send>;

struct Subscription {
    id: u64,
    session_id: Option<String>,
    sink: EventSink,
}

#[derive(Clone, Default)]
struct DisplayedScope {
    session_id: Option<String>,
    revision: u64,
}

/// What a socket is reached with.
struct Target {
    url: String,
    device: String,
    token: String,
}

enum Link {
    Up(mpsc::UnboundedSender<Message>),
    Down(String),
    Closed,
}

struct Shared {
    link: Mutex<Link>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    sinks: Mutex<Vec<Subscription>>,
    subscribed: AtomicBool,
    /// One displayed chat per connection, restored after reconnecting.
    displayed_session: Mutex<DisplayedScope>,
    /// The last durable sequence delivered to the sinks.
    last_durable: AtomicI64,
    next_id: AtomicU64,
    closing: AtomicBool,
    stop: Notify,
    /// Test hook: drop the socket now (the hub going away).
    #[cfg(test)]
    drop_now: Notify,
}

/// The hub's agent seen through one socket.
pub(crate) struct Remote {
    shared: Arc<Shared>,
    runtime: Option<tokio::runtime::Runtime>,
}

/// `https://hub` gives `wss://hub/agent/ws`, `http://` gives `ws://`; a
/// base path is kept.
pub(crate) fn ws_url(base: &str) -> Result<String, String> {
    let base = base.trim().trim_end_matches('/');
    let rest = if let Some(r) = base.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = base.strip_prefix("http://") {
        format!("ws://{r}")
    } else {
        return Err(format!("not an http(s) hub URL: {base:?}"));
    };
    Ok(format!("{rest}/agent/ws"))
}

fn tls() -> tokio_tungstenite::Connector {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("rustls protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    tokio_tungstenite::Connector::Rustls(Arc::new(config))
}

async fn open_socket(t: &Target) -> Result<Socket, String> {
    use tokio_tungstenite::tungstenite::Error as E;
    let mut req = t
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("hub unreachable: {e}"))?;
    let h = req.headers_mut();
    let auth = HeaderValue::from_str(&format!("Bearer {}", t.token))
        .map_err(|_| "the device token is not valid text".to_string())?;
    h.insert("Authorization", auth);
    let device = HeaderValue::from_str(&t.device)
        .map_err(|_| "the device id is not valid text".to_string())?;
    h.insert("X-Device", device);
    let connecting =
        tokio_tungstenite::connect_async_tls_with_config(req, None, false, Some(tls()));
    match tokio::time::timeout(Duration::from_secs(20), connecting).await {
        Err(_) => Err("hub unreachable: connecting timed out".into()),
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(E::Http(r))) => {
            let status = r.status();
            let body = r
                .body()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).trim().to_string())
                .unwrap_or_default();
            Err(match status.as_u16() {
                401 | 403 => format!("the hub refused this device ({status}): {body}"),
                503 => format!("the hub's agent is not available: {body}"),
                _ => format!("the hub answered {status}: {body}"),
            })
        }
        Ok(Err(e)) => Err(format!("hub unreachable: {e}")),
    }
}

impl Shared {
    fn deliver(&self, e: w::AgentEvent) {
        if e.durable && e.kind != w::GAP {
            self.last_durable.fetch_max(e.sequence, Ordering::SeqCst);
        }
        let mut sinks = self.sinks.lock().unwrap();
        sinks.retain_mut(|s| {
            if e.ui_action.is_some() && s.session_id.as_deref() != Some(e.session_id.as_str()) {
                return true;
            }
            (s.sink)(e.clone().into())
        });
        self.clear_unused_scope(&sinks);
    }

    /// Called while holding `sinks`, which serializes scope changes with
    /// their queued requests. Never wait for an RPC from the socket pump.
    fn clear_unused_scope(&self, sinks: &[Subscription]) {
        let mut scope = self.displayed_session.lock().unwrap();
        let Some(session) = scope.session_id.as_deref() else {
            return;
        };
        if sinks
            .iter()
            .any(|sink| sink.session_id.as_deref() == Some(session))
        {
            return;
        }
        scope.session_id = None;
        scope.revision = self.next_id.fetch_add(1, Ordering::Relaxed);
        // A null scope clears only the viewer on the host and preserves the
        // global harness subscription. Reconnect also restores this null scope.
        let _ = self.start(
            w::method::SUBSCRIBE,
            serde_json::json!({"session_id": null}),
        );
    }

    fn on_text(&self, text: &str) {
        let Ok(m) = serde_json::from_str::<w::ServerMessage>(text) else {
            return;
        };
        match m {
            w::ServerMessage::Notification(n) => {
                if n.method == w::method::EVENT {
                    if let Ok(e) = serde_json::from_value::<w::AgentEvent>(n.params) {
                        self.deliver(e);
                    }
                } else if n.method == w::method::UI_ACTION {
                    if let Ok(action) = serde_json::from_value::<w::UiAction>(n.params) {
                        let mut event = w::AgentEvent::blank(w::method::UI_ACTION);
                        event.session_id = action.session_id.clone();
                        event.run_id = action.run_id.clone();
                        event.ui_action = Some(action);
                        self.deliver(event);
                    }
                }
            }
            w::ServerMessage::Response(r) => {
                let Some(id) = r.id.as_u64() else { return };
                let Some(tx) = self.pending.lock().unwrap().remove(&id) else {
                    return;
                };
                let _ = tx.send(match r.error {
                    Some(e) => Err(e.message),
                    None => Ok(r.result.unwrap_or(Value::Null)),
                });
            }
        }
    }

    /// Sends a request; the receiver gets its reply.
    fn start(&self, method: &str, params: Value) -> Result<oneshot::Receiver<Reply>, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        // Registered before the link is read: a disconnect fails it either way.
        self.pending.lock().unwrap().insert(id, tx);
        let link = match &*self.link.lock().unwrap() {
            Link::Up(l) => Ok(l.clone()),
            Link::Down(why) => Err(why.clone()),
            Link::Closed => Err("hub unreachable: the connection is closed".into()),
        };
        let frame =
            serde_json::to_string(&w::Request::new(id, method, params)).map_err(|e| e.to_string());
        let sent = link.and_then(|l| {
            l.send(Message::Text(frame?.into()))
                .map_err(|_| "hub unreachable: the connection dropped".to_string())
        });
        if let Err(e) = sent {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        Ok(rx)
    }

    /// Fails every waiting call.
    fn fail_pending(&self, why: &str) {
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err(why.to_string()));
        }
    }
}

impl Remote {
    pub(crate) fn connect(url: String, device: String, token: String) -> Result<Remote, String> {
        let target = Target {
            url: ws_url(&url)?,
            device,
            token,
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("agent-ws")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let socket = runtime.block_on(open_socket(&target))?;
        let shared = Arc::new(Shared {
            link: Mutex::new(Link::Closed),
            pending: Mutex::new(HashMap::new()),
            sinks: Mutex::new(Vec::new()),
            subscribed: AtomicBool::new(false),
            displayed_session: Mutex::new(DisplayedScope::default()),
            last_durable: AtomicI64::new(0),
            next_id: AtomicU64::new(1),
            closing: AtomicBool::new(false),
            stop: Notify::new(),
            #[cfg(test)]
            drop_now: Notify::new(),
        });
        let link = up(&shared);
        runtime.spawn(run(shared.clone(), target, socket, link));
        Ok(Remote {
            shared,
            runtime: Some(runtime),
        })
    }

    fn call<T: DeserializeOwned>(&self, method: &str, params: impl Serialize) -> Result<T, String> {
        let params = serde_json::to_value(params).map_err(|e| e.to_string())?;
        let rx = self.shared.start(method, params)?;
        let value = rx
            .blocking_recv()
            .unwrap_or_else(|_| Err("hub unreachable: the connection dropped".into()))?;
        serde_json::from_value(value).map_err(|e| format!("unexpected reply to {method}: {e}"))
    }

    pub(crate) fn host_info(&self) -> Result<AgentHostInfo, String> {
        let h: w::HostInfo = self.call(w::method::HOST_INFO, Value::Null)?;
        Ok(h.into())
    }

    pub(crate) fn projects_list(&self) -> Result<Vec<AgentProject>, String> {
        let projects: Vec<w::Project> = self.call(w::method::PROJECTS_LIST, Value::Null)?;
        Ok(projects.into_iter().map(Into::into).collect())
    }

    pub(crate) fn project_add(&self, name: String, path: String) -> Result<AgentProject, String> {
        let project: w::Project =
            self.call(w::method::PROJECT_ADD, w::ProjectAdd { name, path })?;
        Ok(project.into())
    }

    pub(crate) fn project_remove(&self, id: String) -> Result<(), String> {
        let _: Value = self.call(w::method::PROJECT_REMOVE, w::ProjectRef { id })?;
        Ok(())
    }

    pub(crate) fn project_rename(&self, id: String, name: String) -> Result<AgentProject, String> {
        let project: w::Project =
            self.call(w::method::PROJECT_RENAME, w::ProjectRename { id, name })?;
        Ok(project.into())
    }

    pub(crate) fn session_project_set(
        &self,
        session_id: String,
        project_id: String,
    ) -> Result<AgentSession, String> {
        let session: w::AgentSession = self.call(
            w::method::SESSION_PROJECT_SET,
            w::SessionProjectSet {
                session_id,
                project_id,
            },
        )?;
        Ok(session.into())
    }

    pub(crate) fn permission_mode_set(
        &self,
        _mode: AgentPermissionMode,
    ) -> Result<AgentPermissionMode, String> {
        let mode: w::PermissionMode = self.call(
            w::method::PERMISSION_MODE_SET,
            w::PermissionModeSet {
                mode: w::PermissionMode::Bypass,
            },
        )?;
        Ok(mode.into())
    }

    pub(crate) fn models(&self) -> Result<Vec<AgentModel>, String> {
        let v: Vec<w::AgentModel> = self.call(w::method::MODELS, Value::Null)?;
        Ok(v.into_iter().map(Into::into).collect())
    }

    pub(crate) fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        let v: Vec<w::AgentSession> = self.call(w::method::SESSIONS, Value::Null)?;
        Ok(v.into_iter().map(Into::into).collect())
    }

    pub(crate) fn create_session(&self, title: String) -> Result<AgentSession, String> {
        let s: w::AgentSession =
            self.call(w::method::CREATE_SESSION, w::CreateSession { title })?;
        Ok(s.into())
    }

    pub(crate) fn rename_session(&self, id: String, title: String) -> Result<AgentSession, String> {
        let s: w::AgentSession = self.call(
            w::method::RENAME_SESSION,
            w::RenameSession {
                session_id: id,
                title,
            },
        )?;
        Ok(s.into())
    }

    pub(crate) fn delete_session(&self, id: String) -> Result<(), String> {
        let _: Value = self.call(w::method::DELETE_SESSION, w::SessionRef { session_id: id })?;
        Ok(())
    }

    pub(crate) fn session_state(&self, id: String) -> Result<AgentSessionState, String> {
        let s: w::AgentSessionState =
            self.call(w::method::SESSION_STATE, w::SessionRef { session_id: id })?;
        Ok(s.into())
    }

    pub(crate) fn send(
        &self,
        id: String,
        text: String,
        model: Option<String>,
        context: Option<AgentMessageContext>,
    ) -> Result<AgentSent, String> {
        let s: w::AgentSent = self.call(
            w::method::SEND,
            w::Send {
                session_id: id,
                text,
                model,
                context: context.map(Into::into),
            },
        )?;
        Ok(s.into())
    }

    pub(crate) fn edit_undo(&self, edit_id: String) -> Result<AgentEdit, String> {
        let edit: w::AgentEdit = self.call(w::method::EDIT_UNDO, w::EditRef { edit_id })?;
        Ok(edit.into())
    }

    pub(crate) fn steer(&self, id: String, text: String) -> Result<String, String> {
        self.call(
            w::method::STEER,
            w::Steer {
                session_id: id,
                text,
            },
        )
    }

    pub(crate) fn cancel(&self, id: String) -> Result<bool, String> {
        self.call(w::method::CANCEL, w::SessionRef { session_id: id })
    }

    pub(crate) fn decide_approval(
        &self,
        approval_id: String,
        decision: String,
        reason: Option<String>,
    ) -> Result<AgentApproval, String> {
        let a: w::AgentApproval = self.call(
            w::method::DECIDE_APPROVAL,
            w::DecideApproval {
                approval_id,
                decision,
                reason,
            },
        )?;
        Ok(a.into())
    }

    pub(crate) fn events_after(
        &self,
        after: i64,
        session: Option<String>,
    ) -> Result<AgentEvents, String> {
        let p: w::AgentEvents = self.call(
            w::method::EVENTS_AFTER,
            w::EventsAfter {
                after,
                session_id: session,
            },
        )?;
        Ok(p.into())
    }

    pub(crate) fn run_script(
        &self,
        id: String,
        label: String,
        code: String,
    ) -> Result<AgentRunRecord, String> {
        let r: w::AgentRunRecord = self.call(
            w::method::RUN_SCRIPT,
            w::RunScript {
                session_id: id,
                label,
                code,
            },
        )?;
        Ok(r.into())
    }

    /// Adds `sink` to the event stream, subscribing the socket on first use.
    pub(crate) fn schedule_state(&self, page_id: String) -> Result<AgentScheduleState, String> {
        let s: w::ScheduleState =
            self.call(w::method::SCHEDULE_STATE, w::SchedulePageRef { page_id })?;
        Ok(s.into())
    }

    pub(crate) fn schedule_accept(
        &self,
        page_id: String,
        hash: String,
    ) -> Result<AgentScheduleState, String> {
        let s: w::ScheduleState = self.call(
            w::method::SCHEDULE_ACCEPT,
            w::ScheduleAccept { page_id, hash },
        )?;
        Ok(s.into())
    }

    pub(crate) fn schedule_pause(
        &self,
        page_id: String,
        paused: bool,
    ) -> Result<AgentScheduleState, String> {
        let s: w::ScheduleState = self.call(
            w::method::SCHEDULE_PAUSE,
            w::SchedulePause { page_id, paused },
        )?;
        Ok(s.into())
    }

    pub(crate) fn schedule_run_now(&self, page_id: String) -> Result<AgentScheduleRun, String> {
        let r: w::ScheduleRun =
            self.call(w::method::SCHEDULE_RUN_NOW, w::SchedulePageRef { page_id })?;
        Ok(r.into())
    }

    pub(crate) fn schedule_preview(&self, json: String) -> Result<AgentSchedulePreview, String> {
        let p: w::SchedulePreview = self.call(
            w::method::SCHEDULE_PREVIEW,
            w::SchedulePreviewParams { json },
        )?;
        Ok(p.into())
    }

    pub(crate) fn subscribe_with(
        &self,
        session_id: Option<String>,
        sink: EventSink,
    ) -> Result<(), String> {
        let subscription_id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mut sinks = self.shared.sinks.lock().unwrap();
        let already_subscribed = self.shared.subscribed.load(Ordering::SeqCst);
        sinks.push(Subscription {
            id: subscription_id,
            session_id: session_id.clone(),
            sink,
        });
        // A scoped subscription switches the chat this connection is showing.
        // Adding a global listener preserves the existing displayed chat.
        if session_id.is_some() || !already_subscribed {
            let mut scope = self.shared.displayed_session.lock().unwrap();
            let previous_scope = scope.clone();
            let params = serde_json::to_value(w::Subscribe {
                session_id: session_id.clone(),
            })
            .expect("subscribe params");
            let receiver = match self.shared.start(w::method::SUBSCRIBE, params) {
                Ok(receiver) => receiver,
                Err(error) => {
                    sinks.retain(|s| s.id != subscription_id);
                    return Err(error);
                }
            };
            self.shared.subscribed.store(true, Ordering::SeqCst);
            if session_id.is_some() {
                scope.session_id = session_id;
                scope.revision = subscription_id;
            }
            drop(scope);
            drop(sinks);
            let result = receiver
                .blocking_recv()
                .unwrap_or_else(|_| Err("hub unreachable: the connection dropped".into()));
            if let Err(error) = result {
                let mut sinks = self.shared.sinks.lock().unwrap();
                sinks.retain(|s| s.id != subscription_id);
                let mut scope = self.shared.displayed_session.lock().unwrap();
                // A later scoped subscriber owns its scope even if this older
                // request fails. Only restore the scope this request replaced.
                if scope.revision == subscription_id {
                    *scope = previous_scope;
                }
                drop(scope);
                self.shared.clear_unused_scope(&sinks);
                if sinks.is_empty() && !already_subscribed {
                    self.shared.subscribed.store(false, Ordering::SeqCst);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Ends every event stream and closes the socket now.
    pub(crate) fn close_events(&self) -> Result<(), String> {
        self.close();
        Ok(())
    }

    fn close(&self) {
        let s = &self.shared;
        s.closing.store(true, Ordering::SeqCst);
        s.subscribed.store(false, Ordering::SeqCst);
        let mut sinks = s.sinks.lock().unwrap();
        *s.displayed_session.lock().unwrap() = DisplayedScope::default();
        sinks.clear();
        drop(sinks);
        s.stop.notify_one();
    }

    /// Test hook: drops the socket as if the hub had gone away.
    #[cfg(test)]
    pub(crate) fn drop_socket(&self) {
        self.shared.drop_now.notify_one();
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.close();
        if let Some(rt) = self.runtime.take() {
            // Long enough to send the close frame.
            rt.shutdown_timeout(Duration::from_millis(300));
        }
    }
}

/// Marks the link up; calls are queued for the socket from now on.
fn up(shared: &Shared) -> mpsc::UnboundedReceiver<Message> {
    let (tx, rx) = mpsc::unbounded_channel();
    *shared.link.lock().unwrap() = Link::Up(tx);
    rx
}

/// Keeps the socket: drives it, and reconnects with backoff when it drops.
async fn run(
    shared: Arc<Shared>,
    target: Target,
    mut socket: Socket,
    mut link: mpsc::UnboundedReceiver<Message>,
) {
    let mut reconnected = false;
    loop {
        let why = drive(&shared, socket, link, reconnected).await;
        let closing = shared.closing.load(Ordering::SeqCst);
        *shared.link.lock().unwrap() = if closing {
            Link::Closed
        } else {
            Link::Down(format!("hub unreachable: {why}"))
        };
        shared.fail_pending(&format!("hub unreachable: {why}"));
        if closing {
            return;
        }
        let mut backoff = BACKOFF_FIRST;
        socket = loop {
            tokio::select! {
                _ = shared.stop.notified() => {
                    *shared.link.lock().unwrap() = Link::Closed;
                    return;
                }
                _ = tokio::time::sleep(backoff) => {}
            }
            match open_socket(&target).await {
                Ok(s) => break s,
                Err(e) => {
                    *shared.link.lock().unwrap() = Link::Down(e);
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                }
            }
        };
        link = up(&shared);
        reconnected = true;
    }
}

/// Runs one socket until it ends; returns why.
async fn drive(
    shared: &Arc<Shared>,
    socket: Socket,
    mut rx: mpsc::UnboundedReceiver<Message>,
    reconnected: bool,
) -> String {
    let (mut sink, mut stream) = socket.split();
    if reconnected && shared.subscribed.load(Ordering::SeqCst) {
        // Subscribe again, then tell the sinks to catch up.
        let s = shared.clone();
        tokio::spawn(async move {
            let rx = {
                let _sinks = s.sinks.lock().unwrap();
                let params = serde_json::to_value(w::Subscribe {
                    session_id: s.displayed_session.lock().unwrap().session_id.clone(),
                })
                .expect("subscribe params");
                let Ok(rx) = s.start(w::method::SUBSCRIBE, params) else {
                    return;
                };
                rx
            };
            if let Ok(Ok(_)) = rx.await {
                let after = s.last_durable.load(Ordering::SeqCst);
                s.deliver(w::AgentEvent::gap(0, after));
            }
        });
    }
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    let mut heard = tokio::time::Instant::now();
    loop {
        #[cfg(test)]
        let dropped = shared.drop_now.notified();
        #[cfg(not(test))]
        let dropped = std::future::pending::<()>();
        tokio::select! {
            _ = shared.stop.notified() => {
                let _ = sink.send(Message::Close(None)).await;
                let _ = sink.close().await;
                return "the connection is closed".into();
            }
            _ = dropped => return "the connection dropped".into(),
            Some(m) = rx.recv() => {
                if let Err(e) = sink.send(m).await {
                    return e.to_string();
                }
            }
            _ = ping.tick() => {
                if let Err(e) = sink.send(Message::Ping(Default::default())).await {
                    return e.to_string();
                }
            }
            _ = tokio::time::sleep_until(heard + SILENCE) => {
                return format!("no answer for {} s", SILENCE.as_secs());
            }
            next = stream.next() => {
                heard = tokio::time::Instant::now();
                match next {
                    None => return "the connection closed".into(),
                    Some(Err(e)) => return e.to_string(),
                    Some(Ok(Message::Text(t))) => shared.on_text(t.as_str()),
                    Some(Ok(Message::Close(_))) => return "the hub closed the connection".into(),
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

/// A remote `NotesAgent` against an in-process notes-hub.
#[cfg(all(test, any(target_os = "windows", target_os = "linux")))]
mod tests {
    use std::sync::mpsc as std_mpsc;
    use std::time::Instant;

    use super::*;

    struct Hub {
        runtime: tokio::runtime::Runtime,
        server: Option<notes_hub::Server>,
        url: String,
        data: std::path::PathBuf,
    }

    impl Drop for Hub {
        fn drop(&mut self) {
            if let Some(s) = self.server.take() {
                self.runtime.block_on(s.stop());
            }
            let _ = std::fs::remove_dir_all(&self.data);
        }
    }

    fn hub() -> Hub {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let data =
            std::env::temp_dir().join(format!("notes_bridge_remote_{}", vault_core::new_id()));
        let (server, url) = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let config = notes_hub::Config {
                listen: listener.local_addr().unwrap().to_string(),
                data_dir: data.clone(),
                trust_proxy: false,
                agent: notes_hub::AgentSection {
                    default_model: "scripted/scripted".into(),
                    // Scripts are `js` cells; a hub runs native tools by default.
                    execution_mode: vault_agent::bubblepaw_core::ExecutionMode::Js,
                    ..notes_hub::AgentSection::default()
                },
                ..notes_hub::Config::default()
            };
            let hooks: notes_hub::AgentTestHooks = Arc::new(|| vault_agent::TestHooks {
                scripted: Some((vec![], "ok".into())),
                ..vault_agent::TestHooks::default()
            });
            let server = notes_hub::start_with(listener, config, Some(hooks))
                .await
                .unwrap();
            (server, url)
        });
        Hub {
            runtime,
            server: Some(server),
            url,
            data,
        }
    }

    fn next(rx: &std_mpsc::Receiver<AgentEvent>, stop: impl Fn(&AgentEvent) -> bool) -> AgentEvent {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let e = rx.recv_timeout(left).expect("an event before the timeout");
            if stop(&e) {
                return e;
            }
        }
    }

    #[test]
    fn ws_urls_follow_the_scheme() {
        assert_eq!(
            ws_url("https://notes.example.com/").unwrap(),
            "wss://notes.example.com/agent/ws"
        );
        assert_eq!(
            ws_url("http://10.0.0.2:8080").unwrap(),
            "ws://10.0.0.2:8080/agent/ws"
        );
        assert_eq!(ws_url("https://h/base").unwrap(), "wss://h/base/agent/ws");
        assert!(ws_url("ftp://h").is_err());
    }

    #[test]
    fn ui_notifications_reach_only_the_listener_showing_the_chat() {
        let (outgoing, mut requests) = mpsc::unbounded_channel();
        let shared = Shared {
            link: Mutex::new(Link::Up(outgoing)),
            pending: Mutex::new(HashMap::new()),
            sinks: Mutex::new(Vec::new()),
            subscribed: AtomicBool::new(false),
            displayed_session: Mutex::new(DisplayedScope::default()),
            last_durable: AtomicI64::new(7),
            next_id: AtomicU64::new(1),
            closing: AtomicBool::new(false),
            stop: Notify::new(),
            drop_now: Notify::new(),
        };
        let (global_tx, global_rx) = std_mpsc::channel();
        let (other_tx, other_rx) = std_mpsc::channel();
        let (chat_tx, chat_rx) = std_mpsc::channel();
        shared.sinks.lock().unwrap().extend([
            Subscription {
                id: 1,
                session_id: None,
                sink: Box::new(move |e| global_tx.send(e).is_ok()),
            },
            Subscription {
                id: 2,
                session_id: Some("other".into()),
                sink: Box::new(move |e| other_tx.send(e).is_ok()),
            },
            Subscription {
                id: 3,
                session_id: Some("chat".into()),
                sink: Box::new(move |e| chat_tx.send(e).is_ok()),
            },
        ]);
        shared.on_text(
            &serde_json::json!({
                "jsonrpc": "2.0", "method": "ui_action", "params": {
                    "session_id": "chat", "run_id": "run", "action": "highlight",
                    "page_id": "page", "block_ids": ["block"]
                }
            })
            .to_string(),
        );
        let event = chat_rx.try_recv().unwrap();
        assert_eq!(event.kind, "ui_action");
        assert_eq!(event.session_id, "chat");
        assert_eq!(event.ui_action.unwrap().block_ids, ["block"]);
        assert!(global_rx.try_recv().is_err());
        assert!(other_rx.try_recv().is_err());
        assert_eq!(shared.last_durable.load(Ordering::SeqCst), 7);

        // Ordinary harness events still reach the existing global stream.
        shared.deliver(w::AgentEvent::blank("paragraph"));
        assert_eq!(global_rx.try_recv().unwrap().kind, "paragraph");

        // Closing a scoped listener clears the host's viewer while retaining
        // the global stream. This is queued directly from the event pump.
        *shared.displayed_session.lock().unwrap() = DisplayedScope {
            session_id: Some("chat".into()),
            revision: 3,
        };
        drop(chat_rx);
        shared.deliver(w::AgentEvent::blank("paragraph"));
        assert_eq!(shared.displayed_session.lock().unwrap().session_id, None);
        let Message::Text(request) = requests.try_recv().unwrap() else {
            panic!("RPC frame")
        };
        let request: w::Request = serde_json::from_str(request.as_str()).unwrap();
        assert_eq!(request.method, w::method::SUBSCRIBE);
        assert_eq!(request.params, serde_json::json!({"session_id": null}));
        assert_eq!(global_rx.try_recv().unwrap().kind, "paragraph");
        shared.deliver(w::AgentEvent::blank("paragraph"));
        assert_eq!(global_rx.try_recv().unwrap().kind, "paragraph");
        assert!(requests.try_recv().is_err());

        // An older listener ending must not clear a newer displayed chat.
        *shared.displayed_session.lock().unwrap() = DisplayedScope {
            session_id: Some("other".into()),
            revision: 4,
        };
        shared.sinks.lock().unwrap().push(Subscription {
            id: 5,
            session_id: Some("chat".into()),
            sink: Box::new(|_| false),
        });
        shared.deliver(w::AgentEvent::blank("paragraph"));
        assert_eq!(
            shared
                .displayed_session
                .lock()
                .unwrap()
                .session_id
                .as_deref(),
            Some("other")
        );
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn a_remote_agent_lists_runs_streams_and_reconnects_with_a_gap() {
        let h = hub();
        let token = h.server.as_ref().unwrap().hub().pair("laptop").unwrap();

        match Remote::connect(h.url.clone(), "dev-remote".into(), "bad".into()) {
            Err(e) => assert!(e.contains("refused"), "{e}"),
            Ok(_) => panic!("connected with a bad token"),
        }

        let remote = Remote::connect(h.url.clone(), "dev-remote".into(), token).unwrap();
        let info = remote.host_info().unwrap();
        assert_eq!(info.kind, "hub");
        assert!(info.packs.is_empty());
        assert!(!info.tools.iter().any(|name| name == "exec"));
        assert!(!info.can_run_tasks);
        assert!(!info.projects);
        assert!(remote
            .projects_list()
            .unwrap_err()
            .contains("not supported on this host"));
        assert_eq!(
            remote
                .permission_mode_set(AgentPermissionMode::Manual)
                .unwrap(),
            AgentPermissionMode::Bypass,
        );
        assert_eq!(
            remote.host_info().unwrap().permission_mode,
            AgentPermissionMode::Bypass
        );
        assert_eq!(
            remote
                .permission_mode_set(AgentPermissionMode::Auto)
                .unwrap(),
            AgentPermissionMode::Bypass,
        );
        let s = remote.create_session("Remote chat".into()).unwrap();
        assert!(s.project_id.is_none());
        assert!(s.project_name.is_none());
        assert!(remote.sessions().unwrap().iter().any(|x| x.id == s.id));

        let (tx, rx) = std_mpsc::channel();
        remote
            .subscribe_with(None, Box::new(move |e| tx.send(e).is_ok()))
            .unwrap();
        let run = remote
            .run_script(
                s.id.clone(),
                "[script] create".into(),
                r#"await tools.create_page({title: "Remote page", markdown: "x"}); return 1;"#
                    .into(),
            )
            .unwrap();
        assert_eq!(run.status, "completed", "{run:?}");
        let started = next(&rx, |e| e.kind == "run_started" && e.session_id == s.id);
        assert_eq!(started.origin_id.as_deref(), Some("dev-remote"));
        let finished = next(&rx, |e| e.kind == "run_finished");
        assert!(finished.sequence > started.sequence);

        // The hub drops the socket: calls fail fast, then it reconnects and
        // a gap at the last durable sequence follows.
        remote.drop_socket();
        let deadline = Instant::now() + Duration::from_millis(900);
        let mut failed = None;
        while Instant::now() < deadline {
            if let Err(e) = remote.sessions() {
                failed = Some(e);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let failed = failed.expect("a call failed while disconnected");
        assert!(failed.starts_with("hub unreachable"), "{failed}");
        let gap = next(&rx, |e| e.kind == "gap");
        assert!(gap.sequence >= finished.sequence, "{gap:?} {finished:?}");
        let page = remote
            .events_after(gap.sequence, Some(s.id.clone()))
            .unwrap();
        assert!(!page.resync);

        // Events flow again after the reconnect.
        remote
            .run_script(s.id.clone(), "[script] again".into(), "return 2;".into())
            .unwrap();
        next(&rx, |e| {
            e.kind == "run_finished" && e.sequence > finished.sequence
        });

        remote.close_events().unwrap();
        let closed = remote.sessions().unwrap_err();
        assert!(closed.starts_with("hub unreachable"), "{closed}");
    }
}
