//! The agent's values as `notes_agent_wire` DTOs, shared by the desktop
//! bridge and notes-hub's `/agent/ws`, plus the typed-error classification
//! the hub sends as `data.kind`.

use bubblepaw_sdk::approvals::ApprovalRecord;
use bubblepaw_sdk::events::{EventKind, HarnessEvent};
use bubblepaw_sdk::store::{Message, Session};
use bubblepaw_sdk::core::{ModelInfo, Run};
use bubblepaw_sdk::providers::{ProviderError, ProviderErrorKind, Usage};
use bubblepaw_sdk::harness::SessionBusy;
use bubblepaw_sdk::tools::ToolCall;
use notes_agent_wire as w;
use serde_json::Value;

use crate::{EventsPage, Forwarded, Sent, SessionState};

pub use notes_agent_wire;

/// The error text every host returns: the whole anyhow chain.
pub fn message(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

/// The typed kind of an agent error, if it has one.
pub fn error_data(error: &anyhow::Error) -> Option<w::ErrorData> {
    if error
        .chain()
        .any(|c| c.downcast_ref::<SessionBusy>().is_some())
    {
        return Some(w::ErrorData {
            kind: w::ErrorKind::SessionBusy,
            resets_at: None,
        });
    }
    if let Some(p) = ProviderError::of(error) {
        return match &p.kind {
            ProviderErrorKind::Auth => Some(w::ErrorData {
                kind: w::ErrorKind::ProviderAuth,
                resets_at: None,
            }),
            ProviderErrorKind::RateLimit { resets_at } => Some(w::ErrorData {
                kind: w::ErrorKind::RateLimit,
                resets_at: resets_at.clone(),
            }),
            _ => None,
        };
    }
    // BubblePaw's store reports these as plain messages (a missing session
    // row in a snapshot read is SQLite's "no rows").
    let text = message(error);
    if [
        "Unknown session",
        "Unknown approval",
        "Query returned no rows",
    ]
    .iter()
    .any(|m| text.contains(m))
    {
        return Some(w::ErrorData {
            kind: w::ErrorKind::NotFound,
            resets_at: None,
        });
    }
    None
}

/// An agent error as a JSON-RPC error.
pub fn rpc_error(error: &anyhow::Error) -> w::RpcError {
    w::RpcError {
        code: w::code::AGENT_ERROR,
        message: message(error),
        data: error_data(error),
    }
}

fn i(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn name_of<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        Ok(v) => v["type"].as_str().unwrap_or_default().to_string(),
        Err(_) => String::new(),
    }
}

pub fn model(m: ModelInfo) -> w::AgentModel {
    w::AgentModel {
        id: m.id,
        name: m.name,
    }
}

pub fn session(s: Session) -> w::AgentSession {
    w::AgentSession {
        project_id: None,
        project_name: None,
        id: s.id,
        title: s.title,
        parent_session_id: s.parent_session_id,
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

pub fn call(c: ToolCall) -> w::AgentToolCall {
    w::AgentToolCall {
        id: c.id,
        name: c.name,
        arguments_json: c.arguments.to_string(),
    }
}

pub fn message_dto(m: Message) -> w::AgentMessage {
    let (text, context) = if name_of(&m.role) == "user" {
        crate::context::decode(&m.content)
    } else {
        (m.content.clone(), None)
    };
    w::AgentMessage {
        id: m.id,
        role: name_of(&m.role),
        content: m.content,
        text,
        context,
        tool_calls: m.tool_calls.into_iter().map(call).collect(),
        tool_call_id: m.tool_call_id,
        is_error: m.is_error,
        created_at: m.created_at,
    }
}

fn usage(u: Option<&Usage>) -> (Option<i64>, Option<i64>, Option<i64>) {
    match u {
        Some(u) => (
            Some(i(u.input_tokens)),
            Some(i(u.output_tokens)),
            // BubblePaw now reports an unreported cache count as 0; keep the
            // wire's previous "not reported" value for it.
            (u.cache_read_tokens > 0).then(|| i(u.cache_read_tokens)),
        ),
        None => (None, None, None),
    }
}

pub fn run(r: Run) -> w::AgentRunRecord {
    let (input_tokens, output_tokens, cache_read_tokens) = usage(r.usage.as_ref());
    w::AgentRunRecord {
        id: r.id,
        session_id: r.session_id,
        model: r.model,
        status: r.status,
        text: r.text,
        error: r.error,
        started_at: r.started_at,
        ended_at: r.ended_at,
        input_tokens,
        output_tokens,
        cache_read_tokens,
    }
}

pub fn approval(a: ApprovalRecord) -> w::AgentApproval {
    w::AgentApproval {
        id: a.id,
        session_id: a.session_id,
        run_id: a.run_id,
        call_id: a.call_id,
        tool: a.tool,
        arguments_json: a.arguments.to_string(),
        state: a.state.as_str().to_string(),
        reason: a.reason,
        created_at: a.created_at,
        decided_at: a.decided_at,
    }
}

pub fn session_state(s: SessionState) -> w::AgentSessionState {
    let mut session = session(s.session);
    session.project_id = s.project.as_ref().map(|project| project.id.clone());
    session.project_name = s.project.map(|project| project.name);
    w::AgentSessionState {
        session,
        messages: s.messages.into_iter().map(message_dto).collect(),
        runs: s.runs.into_iter().map(run).collect(),
        pending_approvals: s.pending_approvals.into_iter().map(approval).collect(),
        activity: s.activity,
        edits: s.edits,
        seq: i(s.position.seq),
        live_seen: i(s.position.live_seen),
        streaming_text: s.position.streaming_text,
    }
}

pub fn sent(s: Sent) -> w::AgentSent {
    match s {
        Sent::Started(r) => w::AgentSent {
            started: true,
            run: Some(run(r)),
            input_id: None,
        },
        Sent::FollowUp(q) => w::AgentSent {
            started: false,
            run: None,
            input_id: Some(q.id),
        },
    }
}

pub fn event(ev: HarnessEvent) -> w::AgentEvent {
    let mut out = w::AgentEvent::blank(name_of(&ev.kind));
    out.json = serde_json::to_string(&ev).unwrap_or_default();
    out.sequence = i(ev.sequence);
    out.durable = ev.durable;
    out.origin_id = ev.origin.as_ref().map(|o| o.id.clone());
    out.origin_name = ev.origin.and_then(|o| o.name);
    out.session_id = ev.session_id;
    out.run_id = ev.run_id;
    out.created_at = ev.created_at;
    match ev.kind {
        EventKind::RunStarted { model } => out.model = Some(model),
        EventKind::Paragraph { text }
        | EventKind::SteeringAccepted { text }
        | EventKind::FollowupQueued { text } => out.text = Some(text),
        EventKind::ToolStarted { call: c } => out.tool_call = Some(call(c)),
        EventKind::ToolFinished {
            call_id,
            name,
            result,
        } => {
            out.call_id = Some(call_id);
            out.tool_name = Some(name);
            out.is_error = result.is_error;
            out.result = Some(result.content);
        }
        EventKind::RunFinished {
            status,
            text,
            error,
            usage: u,
        } => {
            (out.input_tokens, out.output_tokens, out.cache_read_tokens) = usage(u.as_ref());
            out.status = Some(status);
            out.text = text;
            out.error = error;
        }
        EventKind::ApprovalRequested { approval: a } => out.approval = Some(approval(a)),
        EventKind::ApprovalResolved {
            approval_id,
            state,
            reason,
        } => {
            out.approval_id = Some(approval_id);
            out.approval_state = Some(state.as_str().to_string());
            out.reason = reason;
        }
        EventKind::Output { .. }
        | EventKind::SessionSettled {}
        | EventKind::HostStateChanged { .. }
        | EventKind::ModelRequestStarted {}
        | EventKind::TextDelta { .. } => {}
    }
    out
}

/// A forwarded event, or a `gap` (sequence = the last durable sequence
/// delivered).
pub fn forwarded(f: Forwarded) -> w::AgentEvent {
    match f {
        Forwarded::UiAction(action) => ui_action(action),
        Forwarded::Event(ev) => event(*ev),
        Forwarded::Gap { skipped, after } => w::AgentEvent::gap(i(skipped), i(after)),
    }
}
pub fn ui_action(action: w::UiAction) -> w::AgentEvent {
    let mut event = w::AgentEvent::blank(w::method::UI_ACTION);
    event.session_id = action.session_id.clone();
    event.run_id = action.run_id.clone();
    event.ui_action = Some(action);
    event
}

pub fn events_page(p: EventsPage) -> w::AgentEvents {
    w::AgentEvents {
        events: p.events.into_iter().map(event).collect(),
        last_seq: i(p.last_seq),
        resync: p.resync,
    }
}
