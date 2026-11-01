//! The agent's wire format, shared by `notes-hub` (`GET /agent/ws`) and the
//! app's bridge (`NotesAgent::connect`):
//!
//! - DTOs equal to the bridge's `Agent*` types (same fields, snake_case),
//! - the method names and their params,
//! - JSON-RPC 2.0 envelopes (one text frame per message).
//!
//! Protocol reference: `wiki/sync/hub.md` (`/agent/ws`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod host;

/// Agent-wide policy. Calls in an already-running run retain its captured policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Manual,
    Auto,
    #[default]
    Bypass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub added_at: String,
}

// ---------------------------------------------------------------- DTOs

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPage {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextChat {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSelection {
    pub page_id: String,
    pub page_title: String,
    pub block_ids: Vec<String>,
    pub text: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContext {
    #[serde(default)]
    pub pages: Vec<ContextPage>,
    #[serde(default)]
    pub chats: Vec<ContextChat>,
    #[serde(default)]
    pub selection: Option<ContextSelection>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub tag: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: i64,
    pub new_start: i64,
    pub lines: Vec<DiffLine>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEdit {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub call_id: String,
    pub at: String,
    pub page_id: String,
    pub page_title: String,
    pub kind: String,
    pub added: i64,
    pub removed: i64,
    pub hunks: Vec<DiffHunk>,
    pub changed_blocks: Vec<String>,
    pub undone: bool,
    pub undoable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolActivity {
    pub run_id: String,
    pub call_id: String,
    pub order: i64,
    pub name: String,
    pub summary: String,
    pub is_error: bool,
    pub edit_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiAction {
    pub session_id: String,
    pub run_id: String,
    pub action: String,
    pub page_id: String,
    pub block_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscribe {
    #[serde(default)]
    pub session_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRef {
    pub edit_id: String,
}

/// Capabilities of the host serving this agent, independent of its transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub projects: bool,
    pub kind: String,
    pub packs: Vec<String>,
    pub tools: Vec<String>,
    pub providers: Vec<String>,
    pub can_run_tasks: bool,
    pub version: String,
    /// The model of runs that name none and of a vault that sets none: the
    /// host's configured default (a hub's `[agent] default_model`). Older
    /// hosts don't send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// A model as `provider/model`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub context: Option<MessageContext>,
    #[serde(default)]
    pub tool_calls: Vec<AgentToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunRecord {
    pub id: String,
    pub session_id: String,
    pub model: String,
    pub status: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApproval {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub call_id: String,
    pub tool: String,
    pub arguments_json: String,
    pub state: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub decided_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionState {
    pub session: AgentSession,
    pub messages: Vec<AgentMessage>,
    pub runs: Vec<AgentRunRecord>,
    pub pending_approvals: Vec<AgentApproval>,
    #[serde(default)]
    pub activity: Vec<ToolActivity>,
    #[serde(default)]
    pub edits: Vec<AgentEdit>,
    pub seq: i64,
    pub live_seen: i64,
    pub streaming_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSent {
    pub started: bool,
    #[serde(default)]
    pub run: Option<AgentRunRecord>,
    #[serde(default)]
    pub input_id: Option<String>,
}

/// One harness event, or a `gap`. Fields a kind does not use are empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub kind: String,
    #[serde(default)]
    pub ui_action: Option<UiAction>,
    pub sequence: i64,
    pub durable: bool,
    pub session_id: String,
    pub run_id: String,
    pub created_at: String,
    #[serde(default)]
    pub origin_id: Option<String>,
    #[serde(default)]
    pub origin_name: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tool_call: Option<AgentToolCall>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_tokens: Option<i64>,
    #[serde(default)]
    pub approval: Option<AgentApproval>,
    #[serde(default)]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub approval_state: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub skipped: i64,
    /// The whole harness event as JSON (empty for a gap).
    #[serde(default)]
    pub json: String,
}

impl AgentEvent {
    /// An event of `kind` with every other field empty.
    pub fn blank(kind: impl Into<String>) -> AgentEvent {
        AgentEvent {
            kind: kind.into(),
            ui_action: None,
            sequence: 0,
            durable: false,
            session_id: String::new(),
            run_id: String::new(),
            created_at: String::new(),
            origin_id: None,
            origin_name: None,
            text: None,
            model: None,
            tool_call: None,
            call_id: None,
            tool_name: None,
            result: None,
            is_error: false,
            status: None,
            error: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            approval: None,
            approval_id: None,
            approval_state: None,
            reason: None,
            skipped: 0,
            json: String::new(),
        }
    }

    /// A `gap`: `skipped` events were dropped; catch up with
    /// `events_after(after)`.
    pub fn gap(skipped: i64, after: i64) -> AgentEvent {
        AgentEvent {
            skipped,
            sequence: after,
            ..AgentEvent::blank(GAP)
        }
    }
}

/// The event kind that tells a subscriber to catch up.
pub const GAP: &str = "gap";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvents {
    pub events: Vec<AgentEvent>,
    pub last_seq: i64,
    pub resync: bool,
}

// ---------------------------------------------------------------- methods

/// Method names, one per `NotesAgent` method.
pub mod method {
    pub const EDIT_UNDO: &str = "edit_undo";
    pub const UI_ACTION: &str = "ui_action";
    pub const PROJECTS_LIST: &str = "projects_list";
    pub const PROJECT_ADD: &str = "project_add";
    pub const PROJECT_REMOVE: &str = "project_remove";
    pub const PROJECT_RENAME: &str = "project_rename";
    pub const SESSION_PROJECT_SET: &str = "session_project_set";
    pub const PERMISSION_MODE_SET: &str = "permission_mode_set";
    pub const HOST_INFO: &str = "host_info";
    pub const MODELS: &str = "models";
    pub const SESSIONS: &str = "sessions";
    pub const CREATE_SESSION: &str = "create_session";
    pub const RENAME_SESSION: &str = "rename_session";
    pub const DELETE_SESSION: &str = "delete_session";
    pub const SESSION_STATE: &str = "session_state";
    pub const SEND: &str = "send";
    pub const STEER: &str = "steer";
    pub const CANCEL: &str = "cancel";
    pub const DECIDE_APPROVAL: &str = "decide_approval";
    pub const EVENTS_AFTER: &str = "events_after";
    pub const RUN_SCRIPT: &str = "run_script";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    pub const SCHEDULE_STATE: &str = "schedule_state";
    pub const SCHEDULE_ACCEPT: &str = "schedule_accept";
    pub const SCHEDULE_PAUSE: &str = "schedule_pause";
    pub const SCHEDULE_RUN_NOW: &str = "schedule_run_now";
    pub const SCHEDULE_PREVIEW: &str = "schedule_preview";
    pub const RUNNER_AVAILABLE: &str = "runner_available";
    /// The server's notification after `subscribe`; params: an `AgentEvent`.
    pub const EVENT: &str = "event";

    pub const ALL: [&str; 28] = [
        EDIT_UNDO,
        PROJECTS_LIST,
        PROJECT_ADD,
        PROJECT_REMOVE,
        PROJECT_RENAME,
        SESSION_PROJECT_SET,
        PERMISSION_MODE_SET,
        HOST_INFO,
        MODELS,
        SESSIONS,
        CREATE_SESSION,
        RENAME_SESSION,
        DELETE_SESSION,
        SESSION_STATE,
        SEND,
        STEER,
        CANCEL,
        DECIDE_APPROVAL,
        EVENTS_AFTER,
        RUN_SCRIPT,
        SUBSCRIBE,
        UNSUBSCRIBE,
        SCHEDULE_STATE,
        SCHEDULE_ACCEPT,
        SCHEDULE_PAUSE,
        SCHEDULE_RUN_NOW,
        SCHEDULE_PREVIEW,
        RUNNER_AVAILABLE,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAdd {
    pub name: String,
    pub path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRename {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProjectSet {
    pub session_id: String,
    pub project_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionModeSet {
    pub mode: PermissionMode,
}

/// Params of `create_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSession {
    pub title: String,
}

/// Params of `rename_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameSession {
    pub session_id: String,
    pub title: String,
}

/// Params of `delete_session`, `session_state` and `cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub session_id: String,
}

/// Params of `send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Send {
    pub session_id: String,
    pub text: String,
    #[serde(default)]
    pub context: Option<MessageContext>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Params of `steer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Steer {
    pub session_id: String,
    pub text: String,
}

/// Params of `decide_approval`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecideApproval {
    pub approval_id: String,
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Params of `events_after`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsAfter {
    pub after: i64,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Params of `run_script`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunScript {
    pub session_id: String,
    pub label: String,
    pub code: String,
}

// ---------------------------------------------------------------- schedules

/// One run of a schedule occurrence, as the runner recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRun {
    /// The occurrence's time (RFC 3339).
    pub occurrence: String,
    /// When the run finished (RFC 3339).
    pub at: String,
    pub result: String,
    /// The page the run created, if any.
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A schedule note as one node sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleState {
    pub page_id: String,
    /// The reporting node's record id.
    pub node_id: String,
    /// Hash of `runner`, `tools` and `steps` now, including refused definitions.
    #[serde(default)]
    pub current_hash: Option<String>,
    /// Whether this node has accepted a version of the schedule.
    pub accepted: bool,
    /// Whether the accepted hash is the current one.
    pub hash_matches: bool,
    pub paused: bool,
    #[serde(default)]
    pub accepted_at: Option<String>,
    #[serde(default)]
    pub last_runs: Vec<ScheduleRun>,
    #[serde(default)]
    pub next_run: Option<String>,
    #[serde(default)]
    pub refusal_reason: Option<String>,
    /// `ok`, `needs_acceptance`, `paused`, `runner_off` or `refused`.
    pub runner_state: String,
}

/// One occurrence a preview lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleOccurrence {
    pub occurrence: String,
    pub title: String,
}

/// What a schedule definition would do, without saving it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePreview {
    pub valid: bool,
    /// The hash to pass to `schedule_accept`, when the definition parses.
    #[serde(default)]
    pub current_hash: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub occurrences: Vec<ScheduleOccurrence>,
}

/// Params of `schedule_state` and `schedule_run_now`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePageRef {
    pub page_id: String,
}

/// Params of `schedule_accept`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleAccept {
    pub page_id: String,
    pub hash: String,
}

/// Params of `schedule_pause`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePause {
    pub page_id: String,
    pub paused: bool,
}

/// Params of `schedule_preview`: the schedule's definition as JSON text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePreviewParams {
    pub json: String,
}

/// Params of `runner_available`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerAvailable {
    pub available: bool,
}

// ---------------------------------------------------------------- JSON-RPC

pub const JSONRPC: &str = "2.0";

/// A request from the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Request {
        Request {
            jsonrpc: JSONRPC.into(),
            id: Value::from(id),
            method: method.into(),
            params,
        }
    }
}

/// A response from the server: `result` or `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Response {
        Response {
            jsonrpc: JSONRPC.into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Response {
        Response {
            jsonrpc: JSONRPC.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A notification (no id): the server's `event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn ui_action(action: &UiAction) -> Notification {
        Notification {
            jsonrpc: JSONRPC.into(),
            method: method::UI_ACTION.into(),
            params: serde_json::to_value(action).unwrap_or(Value::Null),
        }
    }
    pub fn event(event: &AgentEvent) -> Notification {
        Notification {
            jsonrpc: JSONRPC.into(),
            method: method::EVENT.into(),
            params: serde_json::to_value(event).unwrap_or(Value::Null),
        }
    }
}

/// Anything the server sends: a notification (has a `method`) or a
/// response. Notifications are tried first: a missing `id` would read as null.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ServerMessage {
    Notification(Notification),
    Response(Response),
}

/// A JSON-RPC error. `message` is the text the bridge returns as its error
/// string; `data.kind` names typed errors (see [`ErrorKind`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    pub kind: ErrorKind,
    /// For `rate_limit`: when the limit resets (RFC 3339), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The session already has an active run.
    SessionBusy,
    /// An unknown session or approval.
    NotFound,
    /// The provider's sign-in is missing or refused.
    ProviderAuth,
    /// The provider's rate limit; see `resets_at`.
    RateLimit,
}

/// JSON-RPC error codes.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// The agent refused or failed the call.
    pub const AGENT_ERROR: i64 = -32000;
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn kind(&self) -> Option<ErrorKind> {
        self.data.as_ref().map(|d| d.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_info_from_an_older_host_has_no_default_model() {
        let old: HostInfo = serde_json::from_value(json!({
            "kind": "hub", "packs": [], "tools": [], "providers": ["codex"],
            "can_run_tasks": false, "version": "0.1.0"
        }))
        .unwrap();
        assert_eq!(old.default_model, None);
        let new = HostInfo {
            default_model: Some("codex/gpt-6-luna".into()),
            ..old
        };
        let json = serde_json::to_value(&new).unwrap();
        assert_eq!(json["default_model"], "codex/gpt-6-luna");
        assert_eq!(serde_json::from_value::<HostInfo>(json).unwrap(), new);
    }

    #[test]
    fn chat_context_round_trips_and_old_context_defaults_to_no_chats() {
        let old: MessageContext =
            serde_json::from_value(json!({"pages": [], "selection": null})).unwrap();
        assert!(old.chats.is_empty());
        let context = MessageContext {
            chats: vec![ContextChat {
                id: "s".into(),
                title: "Garden".into(),
            }],
            ..Default::default()
        };
        let send = Send {
            session_id: "current".into(),
            text: "hello".into(),
            context: Some(context),
            model: None,
        };
        assert_eq!(
            serde_json::from_value::<Send>(serde_json::to_value(&send).unwrap()).unwrap(),
            send
        );
    }

    #[test]
    fn schedule_state_defaults_absent_optional_fields() {
        let state: ScheduleState = serde_json::from_value(json!({
            "page_id": "p",
            "node_id": "n",
            "accepted": false,
            "hash_matches": false,
            "paused": false,
            "runner_state": "runner_off"
        }))
        .unwrap();
        assert!(state.last_runs.is_empty());
        assert_eq!(state.next_run, None);
        let back = serde_json::to_value(&state).unwrap();
        assert_eq!(
            serde_json::from_value::<ScheduleState>(back).unwrap(),
            state
        );
    }

    #[test]
    fn envelopes_round_trip() {
        let r = Request::new(7, method::SEND, json!({"session_id": "s", "text": "hi"}));
        let text = serde_json::to_string(&r).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","id":7,"method":"send","params":{"session_id":"s","text":"hi"}}"#
        );
        let p: Send = serde_json::from_value(r.params).unwrap();
        assert_eq!(p.model, None);

        let ev = Notification::event(&AgentEvent::gap(3, 41));
        let text = serde_json::to_string(&ev).unwrap();
        match serde_json::from_str::<ServerMessage>(&text).unwrap() {
            ServerMessage::Notification(n) => {
                let e: AgentEvent = serde_json::from_value(n.params).unwrap();
                assert_eq!((e.kind.as_str(), e.skipped, e.sequence), (GAP, 3, 41));
            }
            other => panic!("{other:?}"),
        }

        let err = Response::err(
            json!(1),
            RpcError {
                code: code::AGENT_ERROR,
                message: "limit".into(),
                data: Some(ErrorData {
                    kind: ErrorKind::RateLimit,
                    resets_at: Some("2026-10-08T14:00:00Z".into()),
                }),
            },
        );
        let text = serde_json::to_string(&err).unwrap();
        assert!(text.contains(r#""kind":"rate_limit""#), "{text}");
        match serde_json::from_str::<ServerMessage>(&text).unwrap() {
            ServerMessage::Response(r) => {
                assert_eq!(r.error.unwrap().kind(), Some(ErrorKind::RateLimit))
            }
            other => panic!("{other:?}"),
        }
        let ok = serde_json::to_string(&Response::ok(json!(2), Value::Null)).unwrap();
        assert!(matches!(
            serde_json::from_str::<ServerMessage>(&ok).unwrap(),
            ServerMessage::Response(_)
        ));
    }
}
