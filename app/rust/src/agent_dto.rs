//! The agent's wire DTOs (`notes_agent_wire`, shared with notes-hub) as the
//! bridge's FRB-facing types in `api::agent`, which keep their own shape so
//! the Dart API stays the same. Field for field.

use notes_agent_wire as w;

use crate::api::agent::*;

impl From<w::PermissionMode> for AgentPermissionMode {
    fn from(mode: w::PermissionMode) -> Self {
        match mode {
            w::PermissionMode::Manual => Self::Manual,
            w::PermissionMode::Auto => Self::Auto,
            w::PermissionMode::Bypass => Self::Bypass,
        }
    }
}

impl From<AgentPermissionMode> for w::PermissionMode {
    fn from(mode: AgentPermissionMode) -> Self {
        match mode {
            AgentPermissionMode::Manual => Self::Manual,
            AgentPermissionMode::Auto => Self::Auto,
            AgentPermissionMode::Bypass => Self::Bypass,
        }
    }
}

impl From<w::Project> for AgentProject {
    fn from(project: w::Project) -> Self {
        Self {
            id: project.id,
            name: project.name,
            path: project.path,
            added_at: project.added_at,
        }
    }
}

impl From<w::HostInfo> for AgentHostInfo {
    fn from(h: w::HostInfo) -> Self {
        Self {
            kind: h.kind,
            packs: h.packs,
            tools: h.tools,
            providers: h.providers,
            can_run_tasks: h.can_run_tasks,
            version: h.version,
            permission_mode: h.permission_mode.into(),
            projects: h.projects,
        }
    }
}

impl From<w::AgentModel> for AgentModel {
    fn from(m: w::AgentModel) -> Self {
        AgentModel {
            id: m.id,
            name: m.name,
        }
    }
}

impl From<w::AgentSession> for AgentSession {
    fn from(s: w::AgentSession) -> Self {
        AgentSession {
            id: s.id,
            title: s.title,
            parent_session_id: s.parent_session_id,
            created_at: s.created_at,
            updated_at: s.updated_at,
            project_id: s.project_id,
            project_name: s.project_name,
        }
    }
}

impl From<w::AgentToolCall> for AgentToolCall {
    fn from(c: w::AgentToolCall) -> Self {
        AgentToolCall {
            id: c.id,
            name: c.name,
            arguments_json: c.arguments_json,
        }
    }
}

impl From<w::AgentMessage> for AgentMessage {
    fn from(m: w::AgentMessage) -> Self {
        AgentMessage {
            id: m.id,
            role: m.role,
            content: m.content,
            text: m.text,
            context: m.context.map(Into::into),
            tool_calls: m.tool_calls.into_iter().map(Into::into).collect(),
            tool_call_id: m.tool_call_id,
            is_error: m.is_error,
            created_at: m.created_at,
        }
    }
}

impl From<w::AgentRunRecord> for AgentRunRecord {
    fn from(r: w::AgentRunRecord) -> Self {
        AgentRunRecord {
            id: r.id,
            session_id: r.session_id,
            model: r.model,
            status: r.status,
            text: r.text,
            error: r.error,
            started_at: r.started_at,
            ended_at: r.ended_at,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_read_tokens: r.cache_read_tokens,
        }
    }
}

impl From<w::AgentApproval> for AgentApproval {
    fn from(a: w::AgentApproval) -> Self {
        AgentApproval {
            id: a.id,
            session_id: a.session_id,
            run_id: a.run_id,
            call_id: a.call_id,
            tool: a.tool,
            arguments_json: a.arguments_json,
            state: a.state,
            reason: a.reason,
            created_at: a.created_at,
            decided_at: a.decided_at,
        }
    }
}

impl From<w::AgentSessionState> for AgentSessionState {
    fn from(s: w::AgentSessionState) -> Self {
        AgentSessionState {
            session: s.session.into(),
            messages: s.messages.into_iter().map(Into::into).collect(),
            runs: s.runs.into_iter().map(Into::into).collect(),
            pending_approvals: s.pending_approvals.into_iter().map(Into::into).collect(),
            activity: s.activity.into_iter().map(Into::into).collect(),
            edits: s.edits.into_iter().map(Into::into).collect(),
            seq: s.seq,
            live_seen: s.live_seen,
            streaming_text: s.streaming_text,
        }
    }
}

impl From<w::AgentSent> for AgentSent {
    fn from(s: w::AgentSent) -> Self {
        AgentSent {
            started: s.started,
            run: s.run.map(Into::into),
            input_id: s.input_id,
        }
    }
}

impl From<w::AgentEvent> for AgentEvent {
    fn from(e: w::AgentEvent) -> Self {
        AgentEvent {
            kind: e.kind,
            sequence: e.sequence,
            durable: e.durable,
            session_id: e.session_id,
            run_id: e.run_id,
            created_at: e.created_at,
            origin_id: e.origin_id,
            origin_name: e.origin_name,
            text: e.text,
            model: e.model,
            tool_call: e.tool_call.map(Into::into),
            call_id: e.call_id,
            tool_name: e.tool_name,
            result: e.result,
            is_error: e.is_error,
            status: e.status,
            error: e.error,
            input_tokens: e.input_tokens,
            output_tokens: e.output_tokens,
            cache_read_tokens: e.cache_read_tokens,
            approval: e.approval.map(Into::into),
            approval_id: e.approval_id,
            approval_state: e.approval_state,
            reason: e.reason,
            skipped: e.skipped,
            ui_action: e.ui_action.map(Into::into),
            json: e.json,
        }
    }
}

impl From<AgentContextPage> for w::ContextPage {
    fn from(p: AgentContextPage) -> Self {
        Self {
            id: p.id,
            title: p.title,
        }
    }
}

impl From<w::ContextPage> for AgentContextPage {
    fn from(p: w::ContextPage) -> Self {
        Self {
            id: p.id,
            title: p.title,
        }
    }
}

impl From<AgentContextSelection> for w::ContextSelection {
    fn from(s: AgentContextSelection) -> Self {
        Self {
            page_id: s.page_id,
            page_title: s.page_title,
            block_ids: s.block_ids,
            text: s.text,
        }
    }
}

impl From<w::ContextSelection> for AgentContextSelection {
    fn from(s: w::ContextSelection) -> Self {
        Self {
            page_id: s.page_id,
            page_title: s.page_title,
            block_ids: s.block_ids,
            text: s.text,
        }
    }
}

impl From<AgentMessageContext> for w::MessageContext {
    fn from(c: AgentMessageContext) -> Self {
        Self {
            pages: c.pages.into_iter().map(Into::into).collect(),
            chats: c.chats.into_iter().map(Into::into).collect(),
            selection: c.selection.map(Into::into),
        }
    }
}

impl From<w::MessageContext> for AgentMessageContext {
    fn from(c: w::MessageContext) -> Self {
        Self {
            pages: c.pages.into_iter().map(Into::into).collect(),
            chats: c.chats.into_iter().map(Into::into).collect(),
            selection: c.selection.map(Into::into),
        }
    }
}

impl From<AgentContextChat> for w::ContextChat {
    fn from(c: AgentContextChat) -> Self {
        Self {
            id: c.id,
            title: c.title,
        }
    }
}

impl From<w::ContextChat> for AgentContextChat {
    fn from(c: w::ContextChat) -> Self {
        Self {
            id: c.id,
            title: c.title,
        }
    }
}

impl From<w::ToolActivity> for AgentToolActivity {
    fn from(a: w::ToolActivity) -> Self {
        Self {
            run_id: a.run_id,
            call_id: a.call_id,
            order: a.order,
            name: a.name,
            summary: a.summary,
            is_error: a.is_error,
            edit_id: a.edit_id,
        }
    }
}

impl From<w::DiffLine> for AgentDiffLine {
    fn from(l: w::DiffLine) -> Self {
        Self {
            tag: l.tag,
            text: l.text,
        }
    }
}

impl From<w::DiffHunk> for AgentDiffHunk {
    fn from(h: w::DiffHunk) -> Self {
        Self {
            old_start: h.old_start,
            new_start: h.new_start,
            lines: h.lines.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<w::AgentEdit> for AgentEdit {
    fn from(e: w::AgentEdit) -> Self {
        Self {
            id: e.id,
            session_id: e.session_id,
            run_id: e.run_id,
            call_id: e.call_id,
            at: e.at,
            page_id: e.page_id,
            page_title: e.page_title,
            kind: e.kind,
            added: e.added,
            removed: e.removed,
            hunks: e.hunks.into_iter().map(Into::into).collect(),
            changed_blocks: e.changed_blocks,
            undone: e.undone,
            undoable: e.undoable,
        }
    }
}

impl From<w::UiAction> for AgentUiAction {
    fn from(a: w::UiAction) -> Self {
        Self {
            session_id: a.session_id,
            run_id: a.run_id,
            action: a.action,
            page_id: a.page_id,
            block_ids: a.block_ids,
        }
    }
}

impl From<w::AgentEvents> for AgentEvents {
    fn from(p: w::AgentEvents) -> Self {
        AgentEvents {
            events: p.events.into_iter().map(Into::into).collect(),
            last_seq: p.last_seq,
            resync: p.resync,
        }
    }
}

impl From<w::ScheduleRun> for AgentScheduleRun {
    fn from(r: w::ScheduleRun) -> Self {
        Self {
            occurrence: r.occurrence,
            at: r.at,
            result: r.result,
            page: r.page,
            error: r.error,
        }
    }
}

impl From<w::ScheduleState> for AgentScheduleState {
    fn from(s: w::ScheduleState) -> Self {
        Self {
            page_id: s.page_id,
            node_id: s.node_id,
            current_hash: s.current_hash,
            accepted: s.accepted,
            hash_matches: s.hash_matches,
            paused: s.paused,
            accepted_at: s.accepted_at,
            last_runs: s.last_runs.into_iter().map(Into::into).collect(),
            next_run: s.next_run,
            refusal_reason: s.refusal_reason,
            runner_state: s.runner_state,
        }
    }
}

impl From<w::ScheduleOccurrence> for AgentScheduleOccurrence {
    fn from(o: w::ScheduleOccurrence) -> Self {
        Self {
            occurrence: o.occurrence,
            title: o.title,
        }
    }
}

impl From<w::SchedulePreview> for AgentSchedulePreview {
    fn from(p: w::SchedulePreview) -> Self {
        Self {
            valid: p.valid,
            current_hash: p.current_hash,
            reason: p.reason,
            occurrences: p.occurrences.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attached_context_and_human_text_survive_bridge_conversion() {
        let context = w::MessageContext {
            chats: vec![w::ContextChat {
                id: "chat".into(),
                title: "Garden chat".into(),
            }],
            pages: vec![w::ContextPage {
                id: "page".into(),
                title: "Garden".into(),
            }],
            selection: Some(w::ContextSelection {
                page_id: "page".into(),
                page_title: "Garden".into(),
                block_ids: vec!["block".into()],
                text: "Grow peas 🌱".into(),
            }),
        };
        let bridge_context: AgentMessageContext = context.clone().into();
        let roundtrip: w::MessageContext = bridge_context.into();
        assert_eq!(roundtrip, context);

        let message: AgentMessage = w::AgentMessage {
            id: "message".into(),
            role: "user".into(),
            content: "model-facing context\nHelp with this".into(),
            text: "Help with this".into(),
            context: Some(context),
            tool_calls: vec![],
            tool_call_id: None,
            is_error: false,
            created_at: "now".into(),
        }
        .into();
        assert_eq!(message.text, "Help with this");
        assert!(message.content.contains("model-facing context"));
        let selection = message.context.unwrap().selection.unwrap();
        assert_eq!(selection.block_ids, ["block"]);
        assert_eq!(selection.text, "Grow peas 🌱");
    }

    #[test]
    fn snapshot_keeps_edit_diff_and_activity_association() {
        let snapshot: w::AgentSessionState = serde_json::from_value(serde_json::json!({
            "session": { "id": "session", "title": "Chat", "created_at": "now", "updated_at": "now" },
            "messages": [], "runs": [], "pending_approvals": [],
            "seq": 4, "live_seen": 0, "streaming_text": "",
            "activity": [{ "run_id": "run", "call_id": "call", "order": 2,
                "name": "page_update", "summary": "Edited Garden", "is_error": false, "edit_id": "edit" }],
            "edits": [{ "id": "edit", "session_id": "session", "run_id": "run", "call_id": "call",
                "at": "now", "page_id": "page", "page_title": "Garden", "kind": "edited",
                "added": 1, "removed": 1, "hunks": [{ "old_start": 1, "new_start": 1,
                    "lines": [{ "tag": "-", "text": "Old" }, { "tag": "+", "text": "New" }] }],
                "changed_blocks": ["block"], "undone": false, "undoable": true }]
        })).unwrap();
        let snapshot: AgentSessionState = snapshot.into();
        assert_eq!(
            snapshot.activity[0].edit_id.as_deref(),
            Some(snapshot.edits[0].id.as_str())
        );
        assert_eq!(snapshot.edits[0].hunks[0].lines[1].text, "New");
        assert_eq!(snapshot.edits[0].changed_blocks, ["block"]);
        assert!(snapshot.edits[0].undoable);
        assert!(!snapshot.edits[0].undone);
    }

    #[test]
    fn ui_action_is_typed_and_ephemeral_in_existing_event_stream() {
        let mut event = w::AgentEvent::blank("ui_action");
        event.session_id = "session".into();
        event.run_id = "run".into();
        event.ui_action = Some(w::UiAction {
            session_id: "session".into(),
            run_id: "run".into(),
            action: "highlight".into(),
            page_id: "page".into(),
            block_ids: vec!["block".into()],
        });
        let event: AgentEvent = event.into();
        assert!(!event.durable);
        let action = event.ui_action.unwrap();
        assert_eq!(action.session_id, "session");
        assert_eq!(action.action, "highlight");
        assert_eq!(action.block_ids, ["block"]);
    }
}
