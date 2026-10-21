//! Transport-free JSON-RPC agent methods and bounded event notifications.
//!
//! Prepare each client's notification stream with [`Dispatcher::subscribe`].
//! The wire `subscribe` method starts event forwarding; `unsubscribe` stops it.
//! Transports parse their frames into JSON and run [`Dispatcher::handle`] on a
//! blocking thread, since agent methods may block. Use a dispatcher per
//! connection when one client can have several independent connections.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_core::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::bubblepaw_core::{CancellationToken, ClientInfo};
use crate::wire::{self, notes_agent_wire as w};
use crate::Agent;

const EVENT_BUFFER: usize = 1024;

// The agent owns a runtime, so its last reference must close off async threads.
struct OwnedAgent(Option<Arc<Agent>>);
impl Drop for OwnedAgent {
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

#[derive(Default)]
struct ClientState {
    viewed_session: Option<String>,
    viewer_token: Option<CancellationToken>,
    name: Option<String>,
    events: Option<mpsc::Sender<Value>>,
    token: Option<CancellationToken>,
    generation: u64,
}

impl Drop for ClientState {
    fn drop(&mut self) {
        if let Some(token) = self.viewer_token.take() {
            token.cancel();
        }
        if let Some(token) = self.token.take() {
            token.cancel();
        }
    }
}

struct Inner {
    clients: Mutex<HashMap<String, Arc<Mutex<ClientState>>>>,
    agent: OwnedAgent,
}

impl Drop for Inner {
    fn drop(&mut self) {
        for state in self.clients.lock().unwrap().values() {
            let mut state = state.lock().unwrap();
            if let Some(token) = state.viewer_token.take() {
                token.cancel();
            }
            state.viewed_session = None;
            if let Some(token) = state.token.take() {
                token.cancel();
            }
        }
    }
}

/// One connection's RPC dispatcher, independent of transport and authentication.
#[derive(Clone)]
pub struct Dispatcher {
    inner: Arc<Inner>,
}

/// Bounded notifications. Dropping the stream stops its event forwarder.
pub struct Notifications {
    receiver: mpsc::Receiver<Value>,
    state: Arc<Mutex<ClientState>>,
    generation: u64,
}

impl Stream for Notifications {
    type Item = Value;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Value>> {
        self.receiver.poll_recv(cx)
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        if state.generation == self.generation {
            if let Some(token) = state.viewer_token.take() {
                token.cancel();
            }
            state.viewed_session = None;
            if let Some(token) = state.token.take() {
                token.cancel();
            }
            state.events = None;
        }
    }
}

impl Dispatcher {
    pub fn new(agent: Arc<Agent>) -> Self {
        Self {
            inner: Arc::new(Inner {
                clients: Mutex::new(HashMap::new()),
                agent: OwnedAgent(Some(agent)),
            }),
        }
    }

    fn state(&self, client: &str) -> Arc<Mutex<ClientState>> {
        self.inner
            .clients
            .lock()
            .unwrap()
            .entry(client.into())
            .or_default()
            .clone()
    }

    /// Preserves an optional display name in event origins.
    pub fn set_client_info(&self, client: ClientInfo) {
        self.state(&client.id).lock().unwrap().name = client.name;
    }

    fn client_info(&self, client: &str) -> ClientInfo {
        ClientInfo {
            id: client.into(),
            name: self.state(client).lock().unwrap().name.clone(),
        }
    }

    /// Prepares the notification channel; RPC `subscribe` enables forwarding.
    /// One stream may be active per client in this dispatcher.
    pub fn subscribe(&self, client: &str) -> Notifications {
        self.notifications(client, EVENT_BUFFER)
    }

    fn notifications(&self, client: &str, capacity: usize) -> Notifications {
        let state = self.state(client);
        let (events, receiver) = mpsc::channel(capacity);
        let generation = {
            let mut slot = state.lock().unwrap();
            if let Some(token) = slot.token.take() {
                token.cancel();
            }
            if let Some(token) = slot.viewer_token.take() {
                token.cancel();
            }
            slot.viewed_session = None;
            slot.events = Some(events);
            slot.generation += 1;
            slot.generation
        };
        Notifications {
            receiver,
            state,
            generation,
        }
    }

    /// Dispatches one JSON-RPC request. Call from a blocking thread.
    pub fn handle(&self, client: &str, request: Value) -> Option<Value> {
        let req: w::Request = match serde_json::from_value(request) {
            Ok(req) => req,
            Err(e) => {
                return Some(
                    serde_json::to_value(w::Response::err(
                        Value::Null,
                        w::RpcError::new(w::code::INVALID_REQUEST, e.to_string()),
                    ))
                    .expect("response"),
                )
            }
        };
        let result = if req.jsonrpc != w::JSONRPC {
            Err(w::RpcError::new(
                w::code::INVALID_REQUEST,
                "jsonrpc must be \"2.0\"",
            ))
        } else {
            self.dispatch(client, &req.method, req.params)
        };
        Some(
            serde_json::to_value(match result {
                Ok(value) => w::Response::ok(req.id, value),
                Err(error) => w::Response::err(req.id, error),
            })
            .expect("response"),
        )
    }

    fn start_subscription(&self, client: &str, session: Option<String>) -> anyhow::Result<()> {
        let state = self.state(client);
        let mut slot = state.lock().unwrap();
        anyhow::ensure!(
            slot.events.is_some(),
            "prepare the notification stream before subscribing"
        );
        let agent = self.inner.agent.0.as_ref().expect("agent");
        if let Some(session) = session {
            let token = CancellationToken::new();
            agent.mark_viewing(session.clone(), token.clone())?;
            if let Some(old) = slot.viewer_token.replace(token) {
                old.cancel();
            }
            slot.viewed_session = Some(session);
        } else {
            if let Some(token) = slot.viewer_token.take() {
                token.cancel();
            }
            slot.viewed_session = None;
        }
        if slot
            .token
            .as_ref()
            .is_some_and(|token| !token.is_cancelled())
        {
            return Ok(());
        }
        let last = agent.events_after(u64::MAX, None)?.last_seq as i64;
        let events = slot
            .events
            .clone()
            .ok_or_else(|| anyhow::anyhow!("prepare the notification stream before subscribing"))?;
        let mut buffer = EventBuffer {
            events,
            last,
            dropped: 0,
        };
        let weak = Arc::downgrade(&state);
        slot.token = Some(agent.forward_events(move |event| {
            if let crate::Forwarded::UiAction(action) = &event {
                let Some(state) = weak.upgrade() else {
                    return false;
                };
                if state.lock().unwrap().viewed_session.as_deref() != Some(&action.session_id) {
                    return true;
                }
            }
            buffer.forward(wire::forwarded(event))
        }));
        Ok(())
    }

    fn stop_subscription(&self, client: &str) {
        let state = self.state(client);
        let mut state = state.lock().unwrap();
        if let Some(token) = state.viewer_token.take() {
            token.cancel();
        }
        state.viewed_session = None;
        if let Some(token) = state.token.take() {
            token.cancel();
        }
    }

    fn dispatch(&self, client_id: &str, method: &str, p: Value) -> Result<Value, w::RpcError> {
        use w::method as m;
        let a = self.inner.agent.0.as_ref().expect("agent");
        let client = || self.client_info(client_id);
        match method {
            m::HOST_INFO => out(a.host_info()),
            m::PROJECTS_LIST => out(a.projects_list().map_err(agent_err)?),
            m::PROJECT_ADD => {
                let x: w::ProjectAdd = params(p)?;
                out(a.project_add(&x.name, &x.path).map_err(agent_err)?)
            }
            m::PROJECT_REMOVE => {
                let x: w::ProjectRef = params(p)?;
                a.project_remove(&x.id).map_err(agent_err)?;
                Ok(Value::Null)
            }
            m::PROJECT_RENAME => {
                let x: w::ProjectRename = params(p)?;
                out(a.project_rename(&x.id, &x.name).map_err(agent_err)?)
            }
            m::SESSION_PROJECT_SET => {
                let x: w::SessionProjectSet = params(p)?;
                out(a
                    .session_project_set(&x.session_id, &x.project_id)
                    .map_err(agent_err)?)
            }
            m::PERMISSION_MODE_SET => {
                let x: w::PermissionModeSet = params(p)?;
                out(a.permission_mode_set(x.mode).map_err(agent_err)?)
            }
            m::MODELS => out(a.models().into_iter().map(wire::model).collect::<Vec<_>>()),
            m::SESSIONS => out(a
                .sessions()
                .map_err(agent_err)?
                .into_iter()
                .map(|session| a.session_dto(session))
                .collect::<anyhow::Result<Vec<_>>>()
                .map_err(agent_err)?),
            m::CREATE_SESSION => {
                let x: w::CreateSession = params(p)?;
                out(a
                    .session_dto(a.create_session(&x.title).map_err(agent_err)?)
                    .map_err(agent_err)?)
            }
            m::RENAME_SESSION => {
                let x: w::RenameSession = params(p)?;
                out(a
                    .session_dto(
                        a.rename_session(&x.session_id, &x.title)
                            .map_err(agent_err)?,
                    )
                    .map_err(agent_err)?)
            }
            m::DELETE_SESSION => {
                let x: w::SessionRef = params(p)?;
                a.delete_session(&x.session_id).map_err(agent_err)?;
                Ok(Value::Null)
            }
            m::SESSION_STATE => {
                let x: w::SessionRef = params(p)?;
                out(wire::session_state(
                    a.session_state(&x.session_id).map_err(agent_err)?,
                ))
            }
            m::SEND => {
                let x: w::Send = params(p)?;
                out(wire::sent(
                    a.send_as_with_context(&x.session_id, &x.text, x.model, x.context, client())
                        .map_err(agent_err)?,
                ))
            }
            m::STEER => {
                let x: w::Steer = params(p)?;
                out(a
                    .steer_as(&x.session_id, &x.text, client())
                    .map_err(agent_err)?
                    .id)
            }
            m::CANCEL => {
                let x: w::SessionRef = params(p)?;
                out(a.cancel_as(&x.session_id, client()))
            }
            m::DECIDE_APPROVAL => {
                let x: w::DecideApproval = params(p)?;
                out(wire::approval(
                    a.decide_approval_as(&x.approval_id, &x.decision, x.reason, client())
                        .map_err(agent_err)?,
                ))
            }
            m::EVENTS_AFTER => {
                let x: w::EventsAfter = params(p)?;
                out(wire::events_page(
                    a.events_after(x.after.max(0) as u64, x.session_id.as_deref())
                        .map_err(agent_err)?,
                ))
            }
            m::RUN_SCRIPT => {
                let x: w::RunScript = params(p)?;
                out(wire::run(
                    a.run_script_as(&x.session_id, &x.label, &x.code, client())
                        .map_err(agent_err)?,
                ))
            }
            m::SCHEDULE_STATE => {
                let x: w::SchedulePageRef = params(p)?;
                out(a
                    .scheduler()
                    .map_err(agent_err)?
                    .state(&x.page_id)
                    .map_err(agent_err)?)
            }
            m::SCHEDULE_ACCEPT => {
                let x: w::ScheduleAccept = params(p)?;
                out(a
                    .scheduler()
                    .map_err(agent_err)?
                    .accept(&x.page_id, &x.hash)
                    .map_err(agent_err)?)
            }
            m::SCHEDULE_PAUSE => {
                let x: w::SchedulePause = params(p)?;
                out(a
                    .scheduler()
                    .map_err(agent_err)?
                    .pause(&x.page_id, x.paused)
                    .map_err(agent_err)?)
            }
            m::SCHEDULE_RUN_NOW => {
                let x: w::SchedulePageRef = params(p)?;
                out(a
                    .scheduler()
                    .map_err(agent_err)?
                    .run_now(&x.page_id)
                    .map_err(agent_err)?)
            }
            m::SCHEDULE_PREVIEW => {
                let x: w::SchedulePreviewParams = params(p)?;
                out(a
                    .scheduler()
                    .map_err(agent_err)?
                    .preview(&x.json)
                    .map_err(agent_err)?)
            }
            m::RUNNER_AVAILABLE => {
                let x: w::RunnerAvailable = params(p)?;
                a.scheduler()
                    .map_err(agent_err)?
                    .set_available(x.available)
                    .map_err(agent_err)?;
                Ok(Value::Null)
            }
            m::SUBSCRIBE => {
                let x: w::Subscribe = if p.is_null() {
                    w::Subscribe::default()
                } else {
                    params(p)?
                };
                self.start_subscription(client_id, x.session_id)
                    .map_err(agent_err)?;
                Ok(Value::Null)
            }
            m::UNSUBSCRIBE => {
                self.stop_subscription(client_id);
                Ok(Value::Null)
            }
            m::EDIT_UNDO => {
                let x: w::EditRef = params(p)?;
                out(a.edit_undo(&x.edit_id).map_err(agent_err)?)
            }
            _ => Err(w::RpcError::new(
                w::code::METHOD_NOT_FOUND,
                format!("unknown method {method:?}"),
            )),
        }
    }
}

fn params<T: DeserializeOwned>(v: Value) -> Result<T, w::RpcError> {
    serde_json::from_value(v).map_err(|e| w::RpcError::new(w::code::INVALID_PARAMS, e.to_string()))
}

fn out<T: Serialize>(v: T) -> Result<Value, w::RpcError> {
    serde_json::to_value(v).map_err(|e| w::RpcError::new(w::code::AGENT_ERROR, e.to_string()))
}

fn agent_err(e: anyhow::Error) -> w::RpcError {
    wire::rpc_error(&e)
}

struct EventBuffer {
    events: mpsc::Sender<Value>,
    last: i64,
    dropped: i64,
}

impl EventBuffer {
    fn forward(&mut self, event: w::AgentEvent) -> bool {
        use mpsc::error::TrySendError;
        if self.dropped > 0 {
            match self
                .events
                .try_send(notification(&w::AgentEvent::gap(self.dropped, self.last)))
            {
                Ok(()) => self.dropped = 0,
                Err(TrySendError::Full(_)) => {
                    self.dropped += 1;
                    return true;
                }
                Err(TrySendError::Closed(_)) => return false,
            }
        }
        let (durable, sequence) = (event.durable && event.kind != w::GAP, event.sequence);
        match self.events.try_send(notification(&event)) {
            Ok(()) => {
                if durable {
                    self.last = self.last.max(sequence);
                }
                true
            }
            Err(TrySendError::Full(_)) => {
                self.dropped += 1;
                true
            }
            Err(TrySendError::Closed(_)) => false,
        }
    }
}

fn notification(event: &w::AgentEvent) -> Value {
    serde_json::to_value(match &event.ui_action {
        Some(action) => w::Notification::ui_action(action),
        None => w::Notification::event(event),
    })
    .expect("notification")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentConfig, TestHooks};
    use serde_json::json;
    use vault_core::Vault;

    #[test]
    fn scoped_ui_notification_and_edit_undo_are_transport_methods() {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let page = vault.create_page(None, "Shown page".into()).unwrap();
        vault
            .replace_markdown(page.clone(), vec![], "before".into())
            .unwrap();
        let agent = Arc::new(
            Agent::open(
                vault.clone(),
                AgentConfig {
                    data_dir: dir.path().join("data"),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let dispatcher = Dispatcher::new(agent.clone());
        let session = agent.create_session("Viewer").unwrap();
        let mut notifications = dispatcher.subscribe("viewer");
        let response = dispatcher
            .handle(
                "viewer",
                request(w::method::SUBSCRIBE, json!({"session_id":session.id})),
            )
            .unwrap();
        assert!(response.get("error").is_none(), "{response}");
        let run = agent
            .run_script(
                &session.id,
                "show",
                "return await tools.ui_open_page({page:'Shown page'});",
            )
            .unwrap();
        assert!(run.text.unwrap().contains("shown to the user"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let notification = loop {
            match notifications.receiver.try_recv() {
                Ok(value) if value["method"] == w::method::UI_ACTION => break value,
                Ok(_) => {}
                Err(mpsc::error::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("{error}"),
            }
        };
        assert_eq!(notification["params"]["action"], "open_page");
        assert_eq!(notification["params"]["page_id"], page);
        let response = dispatcher
            .handle("viewer", request(w::method::UNSUBSCRIBE, Value::Null))
            .unwrap();
        assert!(response.get("error").is_none());
        let run = agent
            .run_script(
                &session.id,
                "unviewed",
                "return await tools.ui_open_page({page:'Shown page'});",
            )
            .unwrap();
        assert!(run.text.unwrap().contains("no app is showing this chat"));
        agent
            .run_script(
                &session.id,
                "edit",
                "await tools.edit_page({page:'Shown page',op:'set_body',markdown:'after'});",
            )
            .unwrap();
        let edit = agent.session_state(&session.id).unwrap().edits[0].clone();
        let result = call(
            &dispatcher,
            w::method::EDIT_UNDO,
            json!({"edit_id":edit.id}),
        );
        assert_eq!(result["undone"], true);
        assert_eq!(vault.page_markdown(page).unwrap(), "before");
    }

    fn request(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
    }

    fn call(dispatcher: &Dispatcher, method: &str, params: Value) -> Value {
        let response = dispatcher
            .handle("device", request(method, params))
            .unwrap();
        assert_eq!(response["id"], 7);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    #[test]
    fn projects_and_permission_mode_dispatch_and_hub_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let agent = Arc::new(
            Agent::open(
                vault.clone(),
                AgentConfig {
                    data_dir: dir.path().join("desktop"),
                    packs: vec![crate::ToolPack::Desktop { root: root.clone() }],
                    ..AgentConfig::default()
                },
            )
            .unwrap(),
        );
        let dispatcher = Dispatcher::new(agent);
        let projects = call(&dispatcher, w::method::PROJECTS_LIST, Value::Null);
        assert_eq!(projects[0]["name"], "Home");
        let changed = call(
            &dispatcher,
            w::method::PERMISSION_MODE_SET,
            json!({"mode":"manual"}),
        );
        assert_eq!(changed, "manual");
        let info = call(&dispatcher, w::method::HOST_INFO, Value::Null);
        assert_eq!(info["permission_mode"], "manual");
        assert_eq!(info["projects"], true);
        let session = call(
            &dispatcher,
            w::method::CREATE_SESSION,
            json!({"title":"project session"}),
        );
        assert_eq!(session["project_id"], projects[0]["id"]);
        assert_eq!(session["project_name"], "Home");
        let second = dir.path().join("second");
        std::fs::create_dir(&second).unwrap();
        let added = call(
            &dispatcher,
            w::method::PROJECT_ADD,
            json!({"name":"Second","path":second.to_string_lossy()}),
        );
        let selected = call(
            &dispatcher,
            w::method::SESSION_PROJECT_SET,
            json!({"session_id":session["id"],"project_id":added["id"]}),
        );
        assert_eq!(selected["project_id"], added["id"]);
        assert_eq!(selected["project_name"], "Second");
        let renamed = call(
            &dispatcher,
            w::method::PROJECT_RENAME,
            json!({"id":added["id"],"name":"Renamed"}),
        );
        assert_eq!(renamed["name"], "Renamed");
        call(
            &dispatcher,
            w::method::PROJECT_REMOVE,
            json!({"id":added["id"]}),
        );
        let sessions = call(&dispatcher, w::method::SESSIONS, Value::Null);
        assert_eq!(sessions[0]["project_id"], projects[0]["id"]);

        let hub = Dispatcher::new(Arc::new(
            Agent::open(
                vault,
                AgentConfig {
                    data_dir: dir.path().join("hub"),
                    host_kind: crate::HostKind::Hub,
                    ..AgentConfig::default()
                },
            )
            .unwrap(),
        ));
        let info = call(&hub, w::method::HOST_INFO, Value::Null);
        assert_eq!(info["projects"], false);
        assert_eq!(
            call(
                &hub,
                w::method::PERMISSION_MODE_SET,
                json!({"mode":"bypass"})
            ),
            "bypass"
        );
        for (method, params) in [
            (w::method::PROJECTS_LIST, Value::Null),
            (
                w::method::PROJECT_ADD,
                json!({"name":"A","path":root.to_string_lossy()}),
            ),
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
            let response = hub.handle("device", request(method, params)).unwrap();
            assert!(
                response["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("not supported on this host"),
                "{response}"
            );
        }
    }

    #[test]
    fn schedule_methods_route_on_desktop_and_refuse_runner_on_hub() {
        use crate::schedules::Scheduler;
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let desktop_dir = dir.path().join("desktop");
        let desktop = Arc::new(
            Agent::open(
                vault.clone(),
                AgentConfig {
                    data_dir: desktop_dir.clone(),
                    host_kind: crate::HostKind::Desktop,
                    ..AgentConfig::default()
                },
            )
            .unwrap(),
        );
        desktop.set_scheduler(
            Scheduler::open(vault.clone(), &desktop_dir, "desktop", "Desktop", false).unwrap(),
        );
        let dispatcher = Dispatcher::new(desktop);
        assert_eq!(
            call(
                &dispatcher,
                w::method::RUNNER_AVAILABLE,
                json!({"available":true})
            ),
            Value::Null
        );
        let preview = call(
            &dispatcher,
            w::method::SCHEDULE_PREVIEW,
            json!({"json":"{}"}),
        );
        assert_eq!(preview["valid"], false);

        let hub_dir = dir.path().join("hub");
        let hub_agent = Arc::new(
            Agent::open(
                vault.clone(),
                AgentConfig {
                    data_dir: hub_dir.clone(),
                    host_kind: crate::HostKind::Hub,
                    ..AgentConfig::default()
                },
            )
            .unwrap(),
        );
        hub_agent.set_scheduler(Scheduler::open(vault, &hub_dir, "hub", "Hub", true).unwrap());
        let hub = Dispatcher::new(hub_agent);
        let response = hub
            .handle(
                "device",
                request(w::method::RUNNER_AVAILABLE, json!({"available":false})),
            )
            .unwrap();
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("desktop-only"),
            "{response}"
        );
    }

    #[test]
    fn request_round_trip_and_subscribe_with_gap() {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let agent = Arc::new(
            Agent::open(
                vault,
                AgentConfig {
                    data_dir: dir.path().join("data"),
                    default_model: "scripted/scripted".into(),
                    test: TestHooks {
                        scripted: Some((vec![], "ok".into())),
                        ..TestHooks::default()
                    },
                    ..AgentConfig::default()
                },
            )
            .unwrap(),
        );
        let dispatcher = Dispatcher::new(agent);
        let session = call(
            &dispatcher,
            w::method::CREATE_SESSION,
            json!({"title": "Round trip"}),
        );
        let sessions = call(&dispatcher, w::method::SESSIONS, Value::Null);
        assert!(sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == session["id"]));
        let mut notifications = dispatcher.notifications("device", 2);
        assert!(notifications.state.lock().unwrap().token.is_none());
        call(&dispatcher, w::method::SUBSCRIBE, Value::Null);
        assert!(notifications.state.lock().unwrap().token.is_some());
        // Feed the exact bounded forwarding path deterministically, so overflow
        // assertions do not depend on the harness's runtime scheduling.
        let sender = notifications
            .state
            .lock()
            .unwrap()
            .events
            .as_ref()
            .unwrap()
            .clone();
        let mut buffer = EventBuffer {
            events: sender,
            last: 0,
            dropped: 0,
        };
        let event = |sequence, durable| w::AgentEvent {
            sequence,
            durable,
            ..w::AgentEvent::blank("test_event")
        };
        assert!(buffer.forward(event(10, true)));
        assert!(buffer.forward(event(11, false)));
        assert!(buffer.forward(event(12, true)));
        assert!(buffer.forward(event(13, true)));
        assert_eq!(
            notifications.receiver.try_recv().unwrap()["params"]["sequence"],
            10
        );
        assert_eq!(
            notifications.receiver.try_recv().unwrap()["params"]["sequence"],
            11
        );
        assert!(buffer.forward(event(14, true)));
        let gap = notifications.receiver.try_recv().unwrap();
        assert_eq!(gap["method"], w::method::EVENT);
        assert_eq!(gap["params"]["kind"], w::GAP);
        assert_eq!(gap["params"]["skipped"], 2);
        assert_eq!(gap["params"]["sequence"], 10);
        assert_eq!(
            notifications.receiver.try_recv().unwrap()["params"]["sequence"],
            14
        );
        call(&dispatcher, w::method::UNSUBSCRIBE, Value::Null);
        assert!(notifications.state.lock().unwrap().token.is_none());
        call(&dispatcher, w::method::SUBSCRIBE, Value::Null);
        let token = notifications.state.lock().unwrap().token.clone().unwrap();
        drop(notifications);
        assert!(token.is_cancelled());
    }
}
