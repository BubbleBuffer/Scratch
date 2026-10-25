//! "Let the hub use this computer": the host relay. While it runs, this
//! computer keeps one socket open to the hub's `GET /host/ws` and is the
//! host named after this device for `/mcp/agent?host=<name>`
//! (`notes_agent_wire::host`; `wiki/architecture/hub.md`).
//!
//! - The hub sends `tools`, `call` and `close_session` requests and `cancel`
//!   notifications; this side only answers.
//! - Tools are the local agent's own (desktop pack included), as native
//!   tools under Bypass: calls never wait for approval.
//! - Each hub session gets its own local agent session on first call. They
//!   are closed when the hub closes the session or the socket drops.
//! - Drops reconnect with backoff (1 s, doubling, up to 30 s). `stop` (or
//!   drop) closes the socket, cancels running calls and ends the sessions.
//! - [`HostRelay::status`] tells the app where it is, with the last error
//!   (the hub's answer when it refuses the socket).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use notes_agent_wire as w;
use notes_agent_wire::host as h;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::Message;
use vault_agent::bubblepaw_sdk::harness::{CancellationToken, ClientInfo};
use vault_agent::Agent;

use crate::agent_remote::{host_ws_url, open_device_socket, Socket};

const PING_EVERY: Duration = Duration::from_secs(30);
const SILENCE: Duration = Duration::from_secs(90);
const BACKOFF_FIRST: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Where a relay is ([`HostRelay::status`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelayState {
    Off,
    /// The first attempt, not answered yet.
    Connecting,
    Connected,
    /// Waiting to try again after a failure or a drop.
    Retrying,
}

impl RelayState {
    /// `off`, `connecting`, `connected` or `retrying`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RelayState::Off => "off",
            RelayState::Connecting => "connecting",
            RelayState::Connected => "connected",
            RelayState::Retrying => "retrying",
        }
    }
}

/// A relay's state and, while retrying, why the last attempt failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelayStatus {
    pub(crate) state: RelayState,
    pub(crate) error: Option<String>,
}

impl RelayStatus {
    pub(crate) const OFF: RelayStatus = RelayStatus {
        state: RelayState::Off,
        error: None,
    };
    const CONNECTING: RelayStatus = RelayStatus {
        state: RelayState::Connecting,
        error: None,
    };
}

/// What the relay runs calls on: the local agent, or a fake in tests.
pub(crate) trait RelayHost: Send + Sync + 'static {
    fn tools(&self, session: Option<&str>) -> Result<Vec<h::ToolSpec>, String>;
    fn open_session(&self) -> Result<String, String>;
    fn call(
        &self,
        session: &str,
        name: &str,
        arguments: Value,
        caller: h::Caller,
        cancel: CancellationToken,
    ) -> Result<h::CallResult, String>;
    fn close_session(&self, session: &str) -> Result<(), String>;
}

impl RelayHost for Agent {
    fn tools(&self, session: Option<&str>) -> Result<Vec<h::ToolSpec>, String> {
        let tools = self
            .relay_tool_definitions(session)
            .map_err(|e| vault_agent::wire::message(&e))?;
        Ok(tools
            .into_iter()
            .map(|tool| h::ToolSpec {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
                read_only: tool.read_only,
            })
            .collect())
    }

    fn open_session(&self) -> Result<String, String> {
        self.create_session("Hub relay")
            .map(|session| session.id)
            .map_err(|e| vault_agent::wire::message(&e))
    }

    fn call(
        &self,
        session: &str,
        name: &str,
        arguments: Value,
        caller: h::Caller,
        cancel: CancellationToken,
    ) -> Result<h::CallResult, String> {
        let client = ClientInfo {
            id: caller.id,
            name: caller.name,
        };
        self.relay_call(session, name, arguments, client, cancel)
            .map(|outcome| h::CallResult {
                content: outcome.content,
                is_error: outcome.is_error,
                pending: outcome.pending,
            })
            .map_err(|e| vault_agent::wire::message(&e))
    }

    fn close_session(&self, session: &str) -> Result<(), String> {
        self.gateway_close_session(session)
            .map_err(|e| vault_agent::wire::message(&e))
    }
}

struct Shared {
    host: Arc<dyn RelayHost>,
    url: String,
    device: String,
    token: String,
    stopped: AtomicBool,
    stop: Notify,
    status: Mutex<RelayStatus>,
    /// Hub session id → local agent session id.
    sessions: Mutex<HashMap<String, String>>,
    /// Running calls by request id, for `cancel`.
    running: Mutex<HashMap<String, CancellationToken>>,
}

/// A running relay. Dropping it stops it.
pub(crate) struct HostRelay {
    shared: Arc<Shared>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl HostRelay {
    /// Starts connecting to the hub at `base` (its base URL) as `device`
    /// with its device token. Fails only for a URL that is not http(s).
    pub(crate) fn start(
        host: Arc<dyn RelayHost>,
        base: &str,
        device: &str,
        token: &str,
    ) -> Result<HostRelay, String> {
        let shared = Arc::new(Shared {
            host,
            url: host_ws_url(base)?,
            device: device.to_owned(),
            token: token.to_owned(),
            stopped: AtomicBool::new(false),
            stop: Notify::new(),
            status: Mutex::new(RelayStatus::CONNECTING),
            sessions: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
        });
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("host-relay")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.spawn(run(shared.clone()));
        Ok(HostRelay {
            shared,
            runtime: Some(runtime),
        })
    }

    /// Where the relay is now, with the last connection error.
    pub(crate) fn status(&self) -> RelayStatus {
        self.shared.status.lock().unwrap().clone()
    }

    /// Whether the socket to the hub is open now.
    #[cfg(test)]
    pub(crate) fn connected(&self) -> bool {
        self.status().state == RelayState::Connected
    }

    /// Closes the socket, cancels running calls and ends every relay session.
    pub(crate) fn stop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.shared.stop.notify_waiters();
        for (_, cancel) in self.shared.running.lock().unwrap().drain() {
            cancel.cancel();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
        end_sessions_blocking(&self.shared);
        set_status(&self.shared, RelayState::Off, None);
    }
}

impl Drop for HostRelay {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_status(shared: &Shared, state: RelayState, error: Option<String>) {
    *shared.status.lock().unwrap() = RelayStatus { state, error };
}

fn stopped(shared: &Shared) -> bool {
    shared.stopped.load(Ordering::SeqCst)
}

async fn run(shared: Arc<Shared>) {
    let mut backoff = BACKOFF_FIRST;
    while !stopped(&shared) {
        let opened = tokio::select! {
            _ = shared.stop.notified() => break,
            opened = open_device_socket(&shared.url, &shared.device, &shared.token) => opened,
        };
        match opened {
            Ok(socket) => {
                backoff = BACKOFF_FIRST;
                set_status(&shared, RelayState::Connected, None);
                serve(&shared, socket).await;
                let lost = "the connection to the hub was lost".to_string();
                set_status(&shared, RelayState::Retrying, Some(lost));
                let ended = shared.clone();
                let _ = tokio::task::spawn_blocking(move || end_sessions_blocking(&ended)).await;
            }
            // Never logged with the token; the message names only the status
            // and the hub's answer.
            Err(error) => {
                eprintln!("host relay: {error}");
                set_status(&shared, RelayState::Retrying, Some(error));
            }
        }
        tokio::select! {
            _ = shared.stop.notified() => break,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Cancels running calls and closes every local relay session.
fn end_sessions_blocking(shared: &Shared) {
    for (_, cancel) in shared.running.lock().unwrap().drain() {
        cancel.cancel();
    }
    let sessions: Vec<String> = shared
        .sessions
        .lock()
        .unwrap()
        .drain()
        .map(|(_, local)| local)
        .collect();
    for session in sessions {
        if let Err(error) = shared.host.close_session(&session) {
            eprintln!("host relay: closing a session failed: {error}");
        }
    }
}

async fn serve(shared: &Arc<Shared>, socket: Socket) {
    let (mut tx, mut rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = async {
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        loop {
            let frame = tokio::select! {
                biased;
                _ = shared.stop.notified() => break,
                Some(text) = out_rx.recv() => Message::Text(text.into()),
                _ = ping.tick() => Message::Ping(Default::default()),
            };
            if tx.send(frame).await.is_err() {
                return;
            }
        }
        let _ = tx.send(Message::Close(None)).await;
        let _ = tx.close().await;
    };
    let reader = async {
        loop {
            let next = tokio::select! {
                _ = shared.stop.notified() => break,
                next = tokio::time::timeout(SILENCE, rx.next()) => next,
            };
            match next {
                Err(_) | Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => break,
                Ok(Some(Ok(Message::Text(text)))) => receive(shared, text.as_str(), &out_tx),
                Ok(Some(Ok(_))) => {}
            }
        }
    };
    if stopped(shared) {
        return;
    }
    tokio::select! {
        _ = writer => {}
        _ = reader => {}
    }
}

/// One frame from the hub: a request is answered from a blocking task.
fn receive(shared: &Arc<Shared>, text: &str, out: &mpsc::UnboundedSender<String>) {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if frame.get("id").is_none() {
        if frame["method"] == h::method::CANCEL {
            if let Ok(params) = serde_json::from_value::<h::CancelParams>(frame["params"].clone()) {
                if let Some(cancel) = shared.running.lock().unwrap().remove(&params.id.to_string())
                {
                    cancel.cancel();
                }
            }
        }
        return;
    }
    let Ok(request) = serde_json::from_value::<w::Request>(frame) else {
        return;
    };
    let (shared, out) = (shared.clone(), out.clone());
    tokio::spawn(async move {
        let id = request.id.clone();
        let reply = tokio::task::spawn_blocking(move || dispatch(&shared, &request))
            .await
            .unwrap_or_else(|_| {
                w::Response::err(
                    id,
                    w::RpcError::new(w::code::AGENT_ERROR, "the host tool failed"),
                )
            });
        if let Ok(text) = serde_json::to_string(&reply) {
            let _ = out.send(text);
        }
    });
}

fn params<T: DeserializeOwned>(request: &w::Request) -> Result<T, w::RpcError> {
    serde_json::from_value(request.params.clone())
        .map_err(|e| w::RpcError::new(w::code::INVALID_PARAMS, e.to_string()))
}

fn agent_error(message: String) -> w::RpcError {
    w::RpcError::new(w::code::AGENT_ERROR, message)
}

/// Answers one request (blocking: it runs the agent).
fn dispatch(shared: &Shared, request: &w::Request) -> w::Response {
    let id = request.id.clone();
    let answer: Result<Value, w::RpcError> = (|| match request.method.as_str() {
        h::method::TOOLS => {
            let p: h::ToolsParams = params(request)?;
            let local = p
                .session
                .and_then(|session| shared.sessions.lock().unwrap().get(&session).cloned());
            let tools = shared.host.tools(local.as_deref()).map_err(agent_error)?;
            Ok(serde_json::to_value(h::ToolsResult { tools }).unwrap_or_default())
        }
        h::method::CALL => {
            let p: h::CallParams = params(request)?;
            let local = {
                let mut sessions = shared.sessions.lock().unwrap();
                match sessions.get(&p.session) {
                    Some(local) => local.clone(),
                    None => {
                        let local = shared.host.open_session().map_err(agent_error)?;
                        sessions.insert(p.session.clone(), local.clone());
                        local
                    }
                }
            };
            let cancel = CancellationToken::new();
            let key = id.to_string();
            shared
                .running
                .lock()
                .unwrap()
                .insert(key.clone(), cancel.clone());
            let outcome = shared
                .host
                .call(&local, &p.name, p.arguments, p.caller, cancel);
            shared.running.lock().unwrap().remove(&key);
            Ok(serde_json::to_value(outcome.map_err(agent_error)?).unwrap_or_default())
        }
        h::method::CLOSE => {
            let p: h::CloseParams = params(request)?;
            let local = shared.sessions.lock().unwrap().remove(&p.session);
            if let Some(local) = local {
                shared.host.close_session(&local).map_err(agent_error)?;
            }
            Ok(Value::Null)
        }
        other => Err(w::RpcError::new(
            w::code::METHOD_NOT_FOUND,
            format!("unknown method: {other}"),
        )),
    })();
    match answer {
        Ok(result) => w::Response::ok(id, result),
        Err(error) => w::Response::err(id, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    /// A host that records what it is asked and blocks `wait` calls until
    /// they are cancelled.
    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        opened: std::sync::atomic::AtomicUsize,
    }

    impl RelayHost for Fake {
        fn tools(&self, session: Option<&str>) -> Result<Vec<h::ToolSpec>, String> {
            self.log.lock().unwrap().push(format!("tools {session:?}"));
            Ok(vec![h::ToolSpec {
                name: "read".into(),
                description: "Reads a file.".into(),
                input_schema: json!({"type":"object"}),
                read_only: true,
            }])
        }
        fn open_session(&self) -> Result<String, String> {
            let n = self.opened.fetch_add(1, Ordering::SeqCst);
            Ok(format!("local-{n}"))
        }
        fn call(
            &self,
            session: &str,
            name: &str,
            arguments: Value,
            caller: h::Caller,
            cancel: CancellationToken,
        ) -> Result<h::CallResult, String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("call {session} {name} {arguments} {}", caller.id));
            if name == "wait" {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !cancel.is_cancelled() {
                    if Instant::now() > deadline {
                        return Err("never cancelled".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                return Ok(h::CallResult {
                    content: "cancelled".into(),
                    is_error: true,
                    pending: false,
                });
            }
            Ok(h::CallResult {
                content: format!("ran {name}"),
                is_error: false,
                pending: false,
            })
        }
        fn close_session(&self, session: &str) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("close {session}"));
            Ok(())
        }
    }

    fn call(id: u64, session: &str, name: &str) -> w::Request {
        w::Request::new(
            id,
            h::method::CALL,
            json!({"session":session,"name":name,"arguments":{"path":"a"},"caller":{"id":"oauth:ann"}}),
        )
    }

    #[test]
    fn calls_open_one_local_session_per_hub_session_and_close_it() {
        let fake = Arc::new(Fake::default());
        let shared = Shared {
            host: fake.clone(),
            url: String::new(),
            device: String::new(),
            token: String::new(),
            stopped: AtomicBool::new(false),
            stop: Notify::new(),
            status: Mutex::new(RelayStatus::CONNECTING),
            sessions: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
        };
        let tools = dispatch(&shared, &w::Request::new(1, h::method::TOOLS, json!({"session":"s1"})));
        assert_eq!(tools.result.unwrap()["tools"][0]["name"], "read");
        let first = dispatch(&shared, &call(2, "s1", "read"));
        assert_eq!(first.result.unwrap()["content"], "ran read");
        dispatch(&shared, &call(3, "s1", "edit"));
        dispatch(&shared, &call(4, "s2", "read"));
        let listed = dispatch(&shared, &w::Request::new(5, h::method::TOOLS, json!({"session":"s1"})));
        assert!(listed.error.is_none());
        let closed = dispatch(&shared, &w::Request::new(6, h::method::CLOSE, json!({"session":"s1"})));
        assert_eq!(closed.result, Some(Value::Null));
        let unknown = dispatch(&shared, &w::Request::new(7, "steer", json!({})));
        assert_eq!(unknown.error.unwrap().code, w::code::METHOD_NOT_FOUND);
        let bad = dispatch(&shared, &w::Request::new(8, h::method::CALL, json!({"session":1})));
        assert_eq!(bad.error.unwrap().code, w::code::INVALID_PARAMS);
        assert_eq!(
            *fake.log.lock().unwrap(),
            vec![
                "tools None".to_string(),
                "call local-0 read {\"path\":\"a\"} oauth:ann".into(),
                "call local-0 edit {\"path\":\"a\"} oauth:ann".into(),
                "call local-1 read {\"path\":\"a\"} oauth:ann".into(),
                "tools Some(\"local-0\")".into(),
                "close local-0".into(),
            ]
        );
        end_sessions_blocking(&shared);
        assert_eq!(fake.log.lock().unwrap().last().unwrap(), "close local-1");
    }

    /// The relay against a real notes-hub: it registers, a stop takes it off,
    /// and it comes back after the hub restarts its socket.
    #[test]
    fn the_relay_registers_with_a_real_hub_and_stops() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let data = std::env::temp_dir().join(format!("notes_bridge_relay_{}", vault_core::new_id()));
        let (server, url) = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let config = notes_hub::Config {
                listen: listener.local_addr().unwrap().to_string(),
                data_dir: data.clone(),
                trust_proxy: false,
                public_url: format!("{url}/"),
                mcp_agent_enabled: true,
                // Never contacted: nothing here calls /mcp/agent.
                mcp_oauth: notes_hub::McpOAuthConfig {
                    issuer: "http://127.0.0.1:9/issuer/".into(),
                    client_id: "notes-mcp".into(),
                },
                agent: notes_hub::AgentSection {
                    enabled: false,
                    ..notes_hub::AgentSection::default()
                },
                ..notes_hub::Config::default()
            };
            let server = notes_hub::start_with(listener, config, None).await.unwrap();
            (server, url)
        });
        let (token, _) = server.hub().login_device("dev-pc", "work-pc").unwrap();
        let wait = |want: usize| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while server.hosts_online() != want {
                assert!(Instant::now() < deadline, "hosts online: {}", server.hosts_online());
                std::thread::sleep(Duration::from_millis(20));
            }
        };

        let fake = Arc::new(Fake::default());
        let mut relay = HostRelay::start(fake.clone(), &url, "dev-pc", &token).unwrap();
        wait(1);
        assert!(relay.connected());
        relay.stop();
        wait(0);
        assert!(!relay.connected());

        assert_eq!(relay.status(), RelayStatus::OFF);

        // A replaced token is refused; the relay keeps retrying, not connecting,
        // and reports the hub's answer.
        let refused = HostRelay::start(fake, &url, "dev-pc", "not-the-token").unwrap();
        let status = wait_retrying(&refused);
        let error = status.error.unwrap_or_default();
        assert!(error.contains("the hub refused this device"), "{error}");
        assert_eq!(server.hosts_online(), 0);
        assert!(!refused.connected());
        drop(refused);

        runtime.block_on(server.stop());
        let _ = std::fs::remove_dir_all(&data);
    }

    fn wait_retrying(relay: &HostRelay) -> RelayStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = relay.status();
            if status.state == RelayState::Retrying {
                return status;
            }
            assert_ne!(status.state, RelayState::Connected);
            assert!(Instant::now() < deadline, "{status:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_refused_hub_url_retries_with_the_error() {
        // A port that was just free: nothing listens there.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}");
        let mut relay = HostRelay::start(Arc::new(Fake::default()), &url, "pc", "t").unwrap();
        let status = wait_retrying(&relay);
        let error = status.error.unwrap_or_default();
        assert!(error.starts_with("hub unreachable"), "{error}");
        assert_eq!(RelayState::Retrying.as_str(), "retrying");
        relay.stop();
        assert_eq!(relay.status(), RelayStatus::OFF);
    }

    #[test]
    fn a_cancel_stops_a_running_call() {
        let fake = Arc::new(Fake::default());
        let shared = Arc::new(Shared {
            host: fake.clone(),
            url: String::new(),
            device: String::new(),
            token: String::new(),
            stopped: AtomicBool::new(false),
            stop: Notify::new(),
            status: Mutex::new(RelayStatus::CONNECTING),
            sessions: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
        });
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || dispatch(&shared, &call(9, "s1", "wait")))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while shared.running.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (out, _rx) = mpsc::unbounded_channel();
        runtime.block_on(async {
            receive(
                &shared,
                &json!({"jsonrpc":"2.0","method":"cancel","params":{"id":9}}).to_string(),
                &out,
            );
        });
        let reply = worker.join().unwrap();
        assert_eq!(reply.result.unwrap()["content"], "cancelled");
        assert!(shared.running.lock().unwrap().is_empty());
    }
}
