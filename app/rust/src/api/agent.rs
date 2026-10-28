//! vault_agent for Dart: the BubblePaw agent over the open vault, either on
//! this computer (`NotesAgent::open`) or on a notes-hub (`NotesAgent::connect`,
//! over `GET /agent/ws`). Sessions, runs, steering, approvals and live events;
//! AI tasks with the `agent` runner (local agent only). Only type conversion
//! happens here (see `crate::agent_impl`, `crate::agent_remote`).
//!
//! `open` works on Windows and Linux; elsewhere it returns "The agent is not
//! available on this device", so the generated Dart is the same everywhere.
//! `connect` works on every target.
//!
//! Local commands name the client `{id: "desktop", name: "Desktop"}`; on a
//! hub, the hub names the device (its id and label).

use flutter_rust_bridge::frb;

use crate::agent_impl::Inner;
use crate::api::types::{FlatBlock, TrustedHubCertificate};
use crate::api::vault::NotesVault;
use crate::frb_generated::StreamSink;

/// Agent-wide permission mode. Changes apply to the next run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPermissionMode {
    Manual,
    Auto,
    Bypass,
}

/// Per-device agent settings (from `ai_settings.json`).
#[derive(Debug, Clone)]
pub struct AgentSettings {
    /// `provider/model`; empty means the built-in default (`codex/...`).
    pub default_model: String,
    /// The `bun` executable for the Claude Code extension.
    pub bun_path: Option<String>,
    /// The Claude Code extension folder (holding `src/main.ts`).
    pub claude_code_dir: Option<String>,
    pub permission_mode: AgentPermissionMode,
    /// Enable the desktop filesystem and process tools on this computer.
    pub desktop_tools: bool,
    /// Initial Home project folder; empty uses the user's home directory.
    /// Only used when creating the project registry for the first time.
    pub desktop_root: String,
    /// App tests only: adds the tool `test_confirm` (`tools.test_confirm({text})`
    /// in `js`), which asks for approval and then returns `confirmed: <text>`.
    pub test_tools: bool,
}

/// Capabilities reported by the agent host.
#[derive(Debug, Clone)]
pub struct AgentHostInfo {
    pub kind: String,
    pub packs: Vec<String>,
    pub tools: Vec<String>,
    pub providers: Vec<String>,
    pub can_run_tasks: bool,
    pub version: String,
    pub permission_mode: AgentPermissionMode,
    pub projects: bool,
    /// The host's own default model (a hub's `[agent] default_model`); None
    /// from hosts too old to report it.
    pub default_model: Option<String>,
}

/// One configured MCP server of a desktop engine ([`NotesAgent::mcp_status`]).
#[derive(Debug, Clone)]
pub struct AgentMcpServer {
    pub name: String,
    /// "running" | "failed" | "off" (disabled in settings)
    pub state: String,
    /// Tools this server currently offers (0 unless running).
    pub tools: u32,
    /// Why it failed (the McpHost warning text without the `MCP server <name>: ` prefix).
    pub error: Option<String>,
}

/// The PC host relay's connection ([`NotesAgent::host_relay_status`]).
#[derive(Debug, Clone)]
pub struct AgentHostRelayStatus {
    /// "off" | "connecting" | "connected" | "retrying"
    pub state: String,
    /// The last connection error while connecting/retrying (e.g. the hub's
    /// "this device has no name on the hub"), else None.
    pub error: Option<String>,
}

/// A named desktop folder used as the agent's working directory.
#[derive(Debug, Clone)]
pub struct AgentProject {
    pub id: String,
    pub name: String,
    pub path: String,
    /// RFC 3339.
    pub added_at: String,
}

/// A model as `provider/model`.
#[derive(Debug, Clone)]
pub struct AgentModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct AgentSession {
    pub id: String,
    pub title: String,
    /// Set for forks and child-agent sessions.
    pub parent_session_id: Option<String>,
    /// RFC 3339.
    pub created_at: String,
    pub updated_at: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    /// The arguments as JSON (for `js`: `{"code": ...}`).
    pub arguments_json: String,
}

/// Detected local runtime paths. Remote hosts do not expose filesystem paths.
#[derive(Debug, Clone)]
pub struct AgentClaudeDetection {
    pub bun_path: Option<String>,
    pub claude_code_dir: Option<String>,
}

/// A page explicitly attached to a user message.
#[derive(Debug, Clone)]
pub struct AgentContextPage {
    pub id: String,
    pub title: String,
}

/// Another chat explicitly attached to a user message.
#[derive(Debug, Clone)]
pub struct AgentContextChat {
    pub id: String,
    pub title: String,
}

/// The selected text and blocks attached to a user message.
#[derive(Debug, Clone)]
pub struct AgentContextSelection {
    pub page_id: String,
    pub page_title: String,
    pub block_ids: Vec<String>,
    pub text: String,
}

#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct AgentMessageContext {
    pub pages: Vec<AgentContextPage>,
    #[frb(default = "const []")]
    pub chats: Vec<AgentContextChat>,
    pub selection: Option<AgentContextSelection>,
}

/// One completed tool call, ordered within its run.
#[derive(Debug, Clone)]
pub struct AgentToolActivity {
    pub run_id: String,
    pub call_id: String,
    pub order: i64,
    pub name: String,
    pub summary: String,
    pub is_error: bool,
    pub edit_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentDiffLine {
    /// `+`, `-` or `=`.
    pub tag: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct AgentDiffHunk {
    pub old_start: i64,
    pub new_start: i64,
    pub lines: Vec<AgentDiffLine>,
}

/// A persisted edit summary; full undo text stays on the agent host.
#[derive(Debug, Clone)]
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
    pub hunks: Vec<AgentDiffHunk>,
    pub changed_blocks: Vec<String>,
    pub undone: bool,
    pub undoable: bool,
}

/// An ephemeral request to the app showing this chat.
#[derive(Debug, Clone)]
pub struct AgentUiAction {
    pub session_id: String,
    pub run_id: String,
    /// `open_page` or `highlight`.
    pub action: String,
    pub page_id: String,
    pub block_ids: Vec<String>,
}

/// A stored message.
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub id: String,
    /// `system`, `user`, `assistant` or `tool`.
    pub role: String,
    pub content: String,
    /// Human-readable text, with attached context removed.
    pub text: String,
    pub context: Option<AgentMessageContext>,
    pub tool_calls: Vec<AgentToolCall>,
    /// For a `tool` message, the call it answers.
    pub tool_call_id: Option<String>,
    pub is_error: bool,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct AgentRunRecord {
    pub id: String,
    pub session_id: String,
    /// `provider/model`, or `script` for a script run.
    pub model: String,
    /// `running`, `completed`, `failed`, `cancelled`, `interrupted`, ...
    pub status: String,
    pub text: Option<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AgentApproval {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub call_id: String,
    pub tool: String,
    pub arguments_json: String,
    /// `pending`, `approved`, `denied`, `expired` or `cancelled`.
    pub state: String,
    pub reason: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// A session snapshot. Follow it with events after `seq`, skipping the
/// first `live_seen` live events that carry `seq` (already in `streaming_text`).
#[derive(Debug, Clone)]
pub struct AgentSessionState {
    pub session: AgentSession,
    pub messages: Vec<AgentMessage>,
    pub runs: Vec<AgentRunRecord>,
    pub pending_approvals: Vec<AgentApproval>,
    pub activity: Vec<AgentToolActivity>,
    pub edits: Vec<AgentEdit>,
    pub seq: i64,
    pub live_seen: i64,
    /// The running run's text since its last stored message.
    pub streaming_text: String,
}

/// What `send` did: started a run, or queued a follow-up (session busy).
#[derive(Debug, Clone)]
pub struct AgentSent {
    pub started: bool,
    pub run: Option<AgentRunRecord>,
    /// The queued follow-up's id when not started.
    pub input_id: Option<String>,
}

/// One harness event, or a gap. Fields not used by a kind are empty.
///
/// Kinds: `run_started` (model), `paragraph` (text, live only),
/// `tool_started` (tool_call), `tool_finished` (call_id, tool_name, result,
/// is_error), `steering_accepted` / `followup_queued` (text), `run_finished`
/// (status, text, error, run_usage), `approval_requested` (approval),
/// `approval_resolved` (approval_id, approval_state, reason), `output`,
/// `session_settled`, and `gap` (skipped, sequence = the last durable
/// sequence delivered; call `events_after(sequence)`).
/// `ui_action` carries a typed, ephemeral request for the app showing this chat.
#[derive(Debug, Clone)]
pub struct AgentEvent {
    pub kind: String,
    pub sequence: i64,
    pub durable: bool,
    pub session_id: String,
    pub run_id: String,
    pub created_at: String,
    /// The client whose command caused the event.
    pub origin_id: Option<String>,
    pub origin_name: Option<String>,
    pub text: Option<String>,
    pub model: Option<String>,
    pub tool_call: Option<AgentToolCall>,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub result: Option<String>,
    pub is_error: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub approval: Option<AgentApproval>,
    pub approval_id: Option<String>,
    pub approval_state: Option<String>,
    pub reason: Option<String>,
    pub skipped: i64,
    pub ui_action: Option<AgentUiAction>,
    /// The whole event as JSON (empty for a gap).
    pub json: String,
}

/// A page of durable events.
#[derive(Debug, Clone)]
pub struct AgentEvents {
    pub events: Vec<AgentEvent>,
    pub last_seq: i64,
    /// Events after `after` were pruned: reload `session_state` instead.
    pub resync: bool,
}

/// What running an AI task with the agent gave.
#[derive(Debug, Clone)]
pub struct AgentTaskRun {
    pub session_id: String,
    pub ok: bool,
    /// The final text when `ok`, else the error written to the task.
    pub text: String,
}

/// One run of a schedule occurrence.
#[derive(Debug, Clone)]
pub struct AgentScheduleRun {
    /// The occurrence's time, RFC 3339.
    pub occurrence: String,
    /// When the run finished, RFC 3339.
    pub at: String,
    pub result: String,
    /// The page the run created, if any.
    pub page: Option<String>,
    pub error: Option<String>,
}

/// A schedule note as this device sees it. `runner_state` is `ok`,
/// `needs_acceptance`, `paused`, `runner_off` or `refused`.
#[derive(Debug, Clone)]
pub struct AgentScheduleState {
    pub page_id: String,
    /// This device's record id.
    pub node_id: String,
    /// Hash of runner, tools and steps now, including refused definitions.
    pub current_hash: Option<String>,
    pub accepted: bool,
    pub hash_matches: bool,
    pub paused: bool,
    /// RFC 3339.
    pub accepted_at: Option<String>,
    pub last_runs: Vec<AgentScheduleRun>,
    /// RFC 3339.
    pub next_run: Option<String>,
    pub refusal_reason: Option<String>,
    pub runner_state: String,
}

/// One occurrence a schedule preview lists.
#[derive(Debug, Clone)]
pub struct AgentScheduleOccurrence {
    pub occurrence: String,
    pub title: String,
}

/// What a schedule definition would do, without saving it.
#[derive(Debug, Clone)]
pub struct AgentSchedulePreview {
    pub valid: bool,
    /// Pass to `schedule_accept` when the definition parses.
    pub current_hash: Option<String>,
    pub reason: Option<String>,
    pub occurrences: Vec<AgentScheduleOccurrence>,
}

/// Read-only compacted history. Ranges use zero-based, half-open canonical message
/// indices (before UI notice filtering). On overlap, compactable is false and
/// segments is empty. Otherwise the final segment is always Messages, even empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactedHistory {
    pub compactable: bool,
    pub boundary: u32,
    pub message_count: u32,
    /// Canonical index-to-ID map, including notices hidden by the UI.
    pub message_ids: Vec<String>,
    pub segments: Vec<HistorySegment>,
}

/// Canonical message indices: start inclusive, end exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCard {
    pub id: String,
    pub kind: LegacyKind,
    pub title: Option<String>,
    pub summary: String,
    pub started_at: String,
    pub ended_at: String,
    pub first_message_id: String,
    pub last_message_id: String,
    pub range: MessageRange,
    pub message_count: u32,
}

#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterCard {
    pub id: String,
    pub status: ChapterStatus,
    pub title: String,
    pub summary: Option<String>,
    /// One-based part number and total parts for this chapter.
    pub part: u32,
    pub parts: u32,
    pub started_at: String,
    pub ended_at: String,
    pub first_message_id: String,
    pub last_message_id: String,
    pub range: MessageRange,
    /// Whole-chapter footer totals, present only on the last part.
    pub message_count: Option<u32>,
    pub episode_count: Option<u32>,
    /// Closed-chapter metadata, populated only on the last part.
    #[frb(default = "const []")]
    pub decisions: Vec<String>,
    #[frb(default = "const []")]
    pub sections: Vec<ChapterSection>,
    pub episodes: Vec<EpisodeCard>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterSection {
    pub name: String,
    /// One-based episode ordinals within the whole chapter, inclusive.
    pub first_episode: u32,
    pub last_episode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeCard {
    pub id: String,
    pub started_at: String,
    pub ended_at: String,
    pub first_message_id: String,
    pub last_message_id: String,
    pub range: MessageRange,
    pub message_count: u32,
    pub title: String,
    pub gist: Option<String>,
    pub actions: Vec<EpisodeAction>,
    pub action_count: u32,
    pub flagged_count: u32,
    pub error_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeAction {
    pub verb: String,
    pub target: String,
    pub target_kind: TargetKind,
    pub change_marker: Option<String>,
    pub point_of_interest: Option<PointOfInterest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointOfInterest {
    pub kind: String,
    pub reason: String,
}

/// Exactly one card payload is present for its kind; Messages has neither.
/// This tagged representation keeps the bridge's plain Dart DTO convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySegment {
    pub kind: HistorySegmentKind,
    pub range: MessageRange,
    pub legacy: Option<LegacyCard>,
    pub chapter: Option<ChapterCard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySegmentKind {
    LegacyCard,
    ChapterCard,
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyKind {
    Checkpoint,
    Chapter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChapterStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Path,
    Page,
    Command,
    Question,
    Other,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod compacted_from_core {
    use super::*;
    use vault_agent::compacted_history as core;

    impl From<core::CompactedHistory> for CompactedHistory {
        fn from(value: core::CompactedHistory) -> Self {
            Self {
                compactable: value.compactable,
                boundary: u32::try_from(value.boundary)
                    .expect("session exceeds bridge message index capacity"),
                message_count: u32::try_from(value.message_count)
                    .expect("session exceeds bridge message index capacity"),
                message_ids: value.message_ids,
                segments: value.segments.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl From<core::MessageRange> for MessageRange {
        fn from(value: core::MessageRange) -> Self {
            Self {
                start: u32::try_from(value.start)
                    .expect("session exceeds bridge message index capacity"),
                end: u32::try_from(value.end)
                    .expect("session exceeds bridge message index capacity"),
            }
        }
    }

    impl From<core::LegacyCard> for LegacyCard {
        fn from(value: core::LegacyCard) -> Self {
            Self {
                id: value.id,
                kind: value.kind.into(),
                title: value.title,
                summary: value.summary,
                started_at: value.started_at,
                ended_at: value.ended_at,
                first_message_id: value.first_message_id,
                last_message_id: value.last_message_id,
                range: value.range.into(),
                message_count: u32::try_from(value.message_count)
                    .expect("session exceeds bridge message index capacity"),
            }
        }
    }

    impl From<core::ChapterCard> for ChapterCard {
        fn from(value: core::ChapterCard) -> Self {
            Self {
                id: value.id,
                status: value.status.into(),
                title: value.title,
                summary: value.summary,
                part: u32::try_from(value.part)
                    .expect("session exceeds bridge message index capacity"),
                parts: u32::try_from(value.parts)
                    .expect("session exceeds bridge message index capacity"),
                started_at: value.started_at,
                ended_at: value.ended_at,
                first_message_id: value.first_message_id,
                last_message_id: value.last_message_id,
                range: value.range.into(),
                message_count: value.message_count.map(|count| {
                    u32::try_from(count).expect("session exceeds bridge message index capacity")
                }),
                episode_count: value.episode_count.map(|count| {
                    u32::try_from(count).expect("session exceeds bridge message index capacity")
                }),
                decisions: value.decisions,
                sections: value.sections.into_iter().map(Into::into).collect(),
                episodes: value.episodes.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl From<core::ChapterSection> for ChapterSection {
        fn from(value: core::ChapterSection) -> Self {
            Self {
                name: value.name,
                first_episode: u32::try_from(value.first_episode)
                    .expect("chapter exceeds bridge episode index capacity"),
                last_episode: u32::try_from(value.last_episode)
                    .expect("chapter exceeds bridge episode index capacity"),
            }
        }
    }

    impl From<core::EpisodeCard> for EpisodeCard {
        fn from(value: core::EpisodeCard) -> Self {
            Self {
                id: value.id,
                started_at: value.started_at,
                ended_at: value.ended_at,
                first_message_id: value.first_message_id,
                last_message_id: value.last_message_id,
                range: value.range.into(),
                message_count: u32::try_from(value.message_count)
                    .expect("session exceeds bridge message index capacity"),
                title: value.title,
                gist: value.gist,
                actions: value.actions.into_iter().map(Into::into).collect(),
                action_count: u32::try_from(value.action_count)
                    .expect("session exceeds bridge message index capacity"),
                flagged_count: u32::try_from(value.flagged_count)
                    .expect("session exceeds bridge message index capacity"),
                error_count: u32::try_from(value.error_count)
                    .expect("session exceeds bridge message index capacity"),
            }
        }
    }

    impl From<core::EpisodeAction> for EpisodeAction {
        fn from(value: core::EpisodeAction) -> Self {
            Self {
                verb: value.verb,
                target: value.target,
                target_kind: value.target_kind.into(),
                change_marker: value.change_marker,
                point_of_interest: value.point_of_interest.map(Into::into),
            }
        }
    }

    impl From<core::PointOfInterest> for PointOfInterest {
        fn from(value: core::PointOfInterest) -> Self {
            Self {
                kind: value.kind,
                reason: value.reason,
            }
        }
    }

    impl From<core::LegacyKind> for LegacyKind {
        fn from(value: core::LegacyKind) -> Self {
            match value {
                core::LegacyKind::Checkpoint => Self::Checkpoint,
                core::LegacyKind::Chapter => Self::Chapter,
            }
        }
    }

    impl From<core::ChapterStatus> for ChapterStatus {
        fn from(value: core::ChapterStatus) -> Self {
            match value {
                core::ChapterStatus::Open => Self::Open,
                core::ChapterStatus::Closed => Self::Closed,
            }
        }
    }

    impl From<core::TargetKind> for TargetKind {
        fn from(value: core::TargetKind) -> Self {
            match value {
                core::TargetKind::Path => Self::Path,
                core::TargetKind::Page => Self::Page,
                core::TargetKind::Command => Self::Command,
                core::TargetKind::Question => Self::Question,
                core::TargetKind::Other => Self::Other,
            }
        }
    }

    impl From<core::HistorySegment> for HistorySegment {
        fn from(value: core::HistorySegment) -> Self {
            match value {
                core::HistorySegment::LegacyCard(card) => Self {
                    kind: HistorySegmentKind::LegacyCard,
                    range: card.range.into(),
                    legacy: Some(card.into()),
                    chapter: None,
                },
                core::HistorySegment::ChapterCard(card) => Self {
                    kind: HistorySegmentKind::ChapterCard,
                    range: card.range.into(),
                    legacy: None,
                    chapter: Some(card.into()),
                },
                core::HistorySegment::Messages(range) => Self {
                    kind: HistorySegmentKind::Messages,
                    range: range.into(),
                    legacy: None,
                    chapter: None,
                },
            }
        }
    }
}

/// Local-only projection. A remote host has no local store to inspect, so the
/// caller must display full history.
fn compacted_at(data_dir: Option<&str>, session_id: &str) -> Result<CompactedHistory, String> {
    let Some(dir) = data_dir else {
        return Ok(CompactedHistory {
            compactable: false,
            boundary: 0,
            message_count: 0,
            message_ids: Vec::new(),
            segments: Vec::new(),
        });
    };
    local_compacted(dir, session_id)
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_compacted(data_dir: &str, session_id: &str) -> Result<CompactedHistory, String> {
    vault_agent::session_compacted_history_at(std::path::Path::new(data_dir), session_id)
        .map(Into::into)
        .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_compacted(_data_dir: &str, _session_id: &str) -> Result<CompactedHistory, String> {
    Err("The agent is not available on this device".into())
}

/// One session's outline, chapters in order. The notes-owned outline when one was
/// saved (even an empty one), else BubblePaw's context chapters without sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatOutline {
    pub chapters: Vec<OutlineChapter>,
}

/// A span of a chat, from `first_message_id` to `last_message_id` (both inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineChapter {
    pub id: String,
    pub title: String,
    pub summary: String,
    /// The sections' segmentation entries, in order: boundary choices, not conclusions.
    pub decisions: Vec<String>,
    /// True while the chapter is still open.
    pub ongoing: bool,
    pub first_message_id: String,
    pub last_message_id: String,
    /// RFC 3339 time of the first anchor message.
    pub started_at: String,
    /// RFC 3339 time of the last anchor message; None while the chapter is open.
    pub ended_at: Option<String>,
    pub sections: Vec<OutlineSection>,
}

/// A part of a chapter. Empty for chapters that came only from BubblePaw's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineSection {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub decisions: Vec<String>,
    pub first_message_id: String,
    pub last_message_id: String,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod outline_from_core {
    use super::{ChatOutline, OutlineChapter, OutlineSection};
    use vault_agent::outline as core;

    impl From<core::ChatOutline> for ChatOutline {
        fn from(outline: core::ChatOutline) -> Self {
            Self {
                chapters: outline.chapters.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl From<core::OutlineChapter> for OutlineChapter {
        fn from(c: core::OutlineChapter) -> Self {
            Self {
                id: c.id,
                title: c.title,
                summary: c.summary,
                decisions: c.decisions,
                ongoing: c.ongoing,
                first_message_id: c.first_message_id,
                last_message_id: c.last_message_id,
                started_at: c.started_at,
                ended_at: c.ended_at,
                sections: c.sections.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl From<core::OutlineSection> for OutlineSection {
        fn from(s: core::OutlineSection) -> Self {
            Self {
                id: s.id,
                title: s.title,
                summary: s.summary,
                decisions: s.decisions,
                first_message_id: s.first_message_id,
                last_message_id: s.last_message_id,
            }
        }
    }

    impl From<OutlineChapter> for core::OutlineChapter {
        fn from(c: OutlineChapter) -> Self {
            Self {
                id: c.id,
                title: c.title,
                summary: c.summary,
                decisions: c.decisions,
                ongoing: c.ongoing,
                first_message_id: c.first_message_id,
                last_message_id: c.last_message_id,
                started_at: c.started_at,
                ended_at: c.ended_at,
                sections: c.sections.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl From<OutlineSection> for core::OutlineSection {
        fn from(s: OutlineSection) -> Self {
            Self {
                id: s.id,
                title: s.title,
                summary: s.summary,
                decisions: s.decisions,
                first_message_id: s.first_message_id,
                last_message_id: s.last_message_id,
            }
        }
    }
}

/// The local outline of a session, from the agent store in `data_dir`.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_outline(data_dir: &str, session_id: &str) -> Result<ChatOutline, String> {
    vault_agent::session_outline_at(std::path::Path::new(data_dir), session_id)
        .map(Into::into)
        .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_outline(_data_dir: &str, _session_id: &str) -> Result<ChatOutline, String> {
    Err("The agent is not available on this device".into())
}

/// The outline for `data_dir` (a local agent), or empty when there is none (a hub).
fn outline_at(data_dir: Option<&str>, session_id: &str) -> Result<ChatOutline, String> {
    match data_dir {
        Some(dir) => local_outline(dir, session_id),
        None => Ok(ChatOutline {
            chapters: Vec::new(),
        }),
    }
}

/// Dev replay status (`vault_agent::replay::ReplayStatus`). Replay needs `NOTES_DEV_REPLAY=1`.
#[derive(Debug, Clone)]
pub struct ReplayStatus {
    /// The recorded user messages, in send order.
    pub user_texts: Vec<String>,
    pub completed_users: u32,
    /// `waiting`, `running`, `paused`, `stopped`, `complete` or `error`.
    pub state: String,
    pub error: Option<String>,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
impl From<vault_agent::replay::ReplayStatus> for ReplayStatus {
    fn from(status: vault_agent::replay::ReplayStatus) -> Self {
        Self {
            user_texts: status.user_texts,
            completed_users: status.completed_users,
            state: status.state,
            error: status.error,
        }
    }
}

/// Loads a recorded replay into a new, empty session. Dev only.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_replay_load(
    data_dir: &str,
    session_id: &str,
    path: &str,
    delay_ms: u64,
) -> Result<ReplayStatus, String> {
    vault_agent::replay::load(
        std::path::Path::new(data_dir),
        session_id,
        std::path::Path::new(path),
        delay_ms,
    )
    .map(Into::into)
    .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_replay_load(
    _data_dir: &str,
    _session_id: &str,
    _path: &str,
    _delay_ms: u64,
) -> Result<ReplayStatus, String> {
    Err("The agent is not available on this device".into())
}

/// The replay status; also publishes the outline that has grown. Dev only.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_replay_status(data_dir: &str, session_id: &str) -> Result<ReplayStatus, String> {
    vault_agent::replay::status(std::path::Path::new(data_dir), session_id)
        .map(Into::into)
        .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_replay_status(_data_dir: &str, _session_id: &str) -> Result<ReplayStatus, String> {
    Err("The agent is not available on this device".into())
}

/// Pauses, resumes or stops the replay. Dev only.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_replay_control(session_id: &str, action: &str) -> Result<ReplayStatus, String> {
    vault_agent::replay::control(session_id, action)
        .map(Into::into)
        .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_replay_control(_session_id: &str, _action: &str) -> Result<ReplayStatus, String> {
    Err("The agent is not available on this device".into())
}

/// The error for an outline edit on a hub: its outline is not stored on this device.
const HUB_OUTLINE_EDIT: &str = "A hub's chat outline cannot be changed from this device";

/// The local outline revision of a session, from the agent store in `data_dir`.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_outline_revision(data_dir: &str, session_id: &str) -> Result<u64, String> {
    vault_agent::session_outline_revision_at(std::path::Path::new(data_dir), session_id)
        .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_outline_revision(_data_dir: &str, _session_id: &str) -> Result<u64, String> {
    Err("The agent is not available on this device".into())
}

/// Adds or replaces a chapter in the local outline; returns the new revision.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_upsert_chapter(
    data_dir: &str,
    session_id: &str,
    chapter: OutlineChapter,
) -> Result<u64, String> {
    vault_agent::upsert_outline_chapter_at(
        std::path::Path::new(data_dir),
        session_id,
        chapter.into(),
    )
    .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_upsert_chapter(
    _data_dir: &str,
    _session_id: &str,
    _chapter: OutlineChapter,
) -> Result<u64, String> {
    Err("The agent is not available on this device".into())
}

/// Adds or replaces a section of a chapter in the local outline; returns the new revision.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_upsert_section(
    data_dir: &str,
    session_id: &str,
    chapter_first_message_id: &str,
    section: OutlineSection,
) -> Result<u64, String> {
    vault_agent::upsert_outline_section_at(
        std::path::Path::new(data_dir),
        session_id,
        chapter_first_message_id,
        section.into(),
    )
    .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_upsert_section(
    _data_dir: &str,
    _session_id: &str,
    _chapter_first_message_id: &str,
    _section: OutlineSection,
) -> Result<u64, String> {
    Err("The agent is not available on this device".into())
}

/// Closes a chapter in the local outline; returns the new revision.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn local_close_chapter(
    data_dir: &str,
    session_id: &str,
    first_message_id: &str,
    last_message_id: &str,
    ended_at: &str,
) -> Result<u64, String> {
    vault_agent::close_outline_chapter_at(
        std::path::Path::new(data_dir),
        session_id,
        first_message_id,
        last_message_id,
        ended_at,
    )
    .map_err(|e| format!("{e:#}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn local_close_chapter(
    _data_dir: &str,
    _session_id: &str,
    _first_message_id: &str,
    _last_message_id: &str,
    _ended_at: &str,
) -> Result<u64, String> {
    Err("The agent is not available on this device".into())
}

/// The agent, local or on a hub. One per vault; the Dart object holds an `Arc`.
#[frb(opaque)]
pub struct NotesAgent {
    pub(crate) inner: Inner,
    /// The local data folder (`<data_dir>/agent/` holds the store); None on a hub.
    data_dir: Option<String>,
}

impl NotesAgent {
    /// Opens the agent over `vault`, keeping its store in `<data_dir>/agent/`.
    pub fn open(
        vault: &NotesVault,
        data_dir: String,
        settings: AgentSettings,
    ) -> Result<NotesAgent, String> {
        Ok(NotesAgent {
            inner: Inner::open(vault, data_dir.clone(), settings)?,
            data_dir: Some(data_dir),
        })
    }

    /// Connects to the agent of the hub at `url` (its base URL; `https://`
    /// gives `wss://`), as device `device` with its device token. Fails if
    /// the first connection fails. Later drops reconnect with backoff and
    /// push a `gap` event; calls while disconnected fail with "hub
    /// unreachable". `close_events` (or dropping it) closes the socket.
    ///
    /// `trusted`: the device's trusted hub certificates, as for
    /// `NotesSync::with_trusted_certificates`.
    pub fn connect(
        url: String,
        device: String,
        token: String,
        trusted: Vec<TrustedHubCertificate>,
    ) -> Result<NotesAgent, String> {
        Ok(NotesAgent {
            inner: Inner::connect(url, device, token, TrustedHubCertificate::to_sync(trusted))?,
            data_dir: None,
        })
    }

    /// "Let the hub use this computer": keeps one socket open to the hub at
    /// `url` (`/host/ws`) as device `device` with its device token, so
    /// `/mcp/agent?host=<this device's name>` runs this agent's tools here
    /// (desktop pack included, no approval prompts). Restarts a running
    /// relay; drops reconnect with backoff. Fails on a hub connection.
    /// `trusted`: the device's trusted hub certificates, as for
    /// `NotesSync::with_trusted_certificates`.
    pub fn host_relay_start(
        &self,
        url: String,
        device: String,
        token: String,
        trusted: Vec<TrustedHubCertificate>,
    ) -> Result<(), String> {
        self.inner
            .host_relay_start(url, device, token, TrustedHubCertificate::to_sync(trusted))
    }

    /// Gives the running host relay the device's trusted hub certificates
    /// after the user trusted or revoked one: its next connection attempt
    /// uses them, and a relay waiting to retry tries again at once. A no-op
    /// when no relay runs.
    pub fn host_relay_set_trust(&self, trusted: Vec<TrustedHubCertificate>) {
        self.inner
            .host_relay_set_trust(TrustedHubCertificate::to_sync(trusted))
    }

    /// Stops the host relay: the socket closes and running calls are
    /// cancelled. Dropping the agent stops it too.
    pub fn host_relay_stop(&self) -> Result<(), String> {
        self.inner.host_relay_stop()
    }

    /// The PC host relay's connection state ("off" when not started).
    pub fn host_relay_status(&self) -> AgentHostRelayStatus {
        self.inner.host_relay_status()
    }

    /// Replaces this engine's MCP servers. `json` is a JSON object
    /// `{"<name>": <BubblePaw McpServerSettings as serde JSON>, ...}`.
    /// Restarts only changed servers; the chat tools, the gateway and the
    /// relay tool list reflect the change at once. Err(text) when the
    /// settings are invalid (state unchanged). A no-op Ok(()) where MCP is
    /// unavailable (hub host, tool set not Everything).
    pub fn set_mcp_servers(&self, json: String) -> Result<(), String> {
        self.inner.set_mcp_servers(json)
    }

    /// One entry per configured server, in name order. Empty where MCP is unavailable.
    pub fn mcp_status(&self) -> Vec<AgentMcpServer> {
        self.inner.mcp_status()
    }

    /// Capabilities of the connected host.
    pub fn host_info(&self) -> Result<AgentHostInfo, String> {
        self.inner.host_info()
    }

    /// The session's outline (see [`ChatOutline`]). Errors for an unknown session.
    /// Empty on a hub: its store is not on this device.
    pub fn session_outline(&self, session_id: String) -> Result<ChatOutline, String> {
        outline_at(self.data_dir.as_deref(), &session_id)
    }

    /// LCM and legacy summary cards interleaved with uncovered canonical messages.
    /// Never reads the notes-owned outline. Remote hosts return compactable=false.
    pub fn compacted_history(&self, session_id: String) -> Result<CompactedHistory, String> {
        compacted_at(self.data_dir.as_deref(), &session_id)
    }

    /// The session's outline revision: 0 until the outline first changes, then one
    /// more for every change; edits that change nothing keep it. Read-only, and 0 on
    /// a hub.
    pub fn session_outline_revision(&self, session_id: String) -> Result<u64, String> {
        match self.data_dir.as_deref() {
            Some(dir) => local_outline_revision(dir, &session_id),
            None => Ok(0),
        }
    }

    /// Adds a chapter to the session's outline, or replaces the chapter with the same
    /// `first_message_id`. Its sections replace the sections with the same
    /// `first_message_id`; other sections are kept. Every anchor must be a message of
    /// this session, in order. The check and the write are one transaction. Returns
    /// the new revision. Fails on a hub.
    pub fn upsert_outline_chapter(
        &self,
        session_id: String,
        chapter: OutlineChapter,
    ) -> Result<u64, String> {
        local_upsert_chapter(self.outline_data_dir()?, &session_id, chapter)
    }

    /// Adds a section to the chapter starting at `chapter_first_message_id`, or replaces
    /// the section with the same `first_message_id`. The section must lie inside the
    /// chapter. Returns the new revision. Fails on a hub.
    pub fn upsert_outline_section(
        &self,
        session_id: String,
        chapter_first_message_id: String,
        section: OutlineSection,
    ) -> Result<u64, String> {
        local_upsert_section(
            self.outline_data_dir()?,
            &session_id,
            &chapter_first_message_id,
            section,
        )
    }

    /// Closes the chapter starting at `first_message_id`: its last anchor becomes
    /// `last_message_id` and it ends at `ended_at` (RFC 3339). Returns the new
    /// revision. Fails on a hub.
    pub fn close_outline_chapter(
        &self,
        session_id: String,
        first_message_id: String,
        last_message_id: String,
        ended_at: String,
    ) -> Result<u64, String> {
        local_close_chapter(
            self.outline_data_dir()?,
            &session_id,
            &first_message_id,
            &last_message_id,
            &ended_at,
        )
    }

    /// Dev only (`NOTES_DEV_REPLAY=1`): loads the read-only replay at `path` into the
    /// new, empty `session_id`. `delay_ms` is the pause between streamed paragraphs.
    /// Fails on a hub.
    pub fn replay_load(
        &self,
        session_id: String,
        path: String,
        delay_ms: u64,
    ) -> Result<ReplayStatus, String> {
        local_replay_load(self.outline_data_dir()?, &session_id, &path, delay_ms)
    }

    /// Dev only: the replay's status, with the outline grown so far. Poll it during
    /// a replay. Fails on a hub.
    pub fn replay_status(&self, session_id: String) -> Result<ReplayStatus, String> {
        local_replay_status(self.outline_data_dir()?, &session_id)
    }

    /// Display policy for local chats; titles remain raw in storage.
    pub fn session_user_named_ids(&self) -> Result<Vec<String>, String> {
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if let Some(data) = self.data_dir.as_deref() {
            return vault_agent::session_user_named_ids_at(std::path::Path::new(data))
                .map_err(|e| format!("{e:#}"));
        }
        Ok(Vec::new())
    }

    /// Dev only: `pause`, `resume` or `stop`. Stop fails the replay's next provider
    /// call; use `cancel` to stop the run. Fails on a hub.
    pub fn replay_control(
        &self,
        session_id: String,
        action: String,
    ) -> Result<ReplayStatus, String> {
        self.outline_data_dir()?;
        local_replay_control(&session_id, &action)
    }

    /// The local data folder that outline edits write to; a hub has none.
    fn outline_data_dir(&self) -> Result<&str, String> {
        self.data_dir
            .as_deref()
            .ok_or_else(|| HUB_OUTLINE_EDIT.into())
    }

    pub fn projects_list(&self) -> Result<Vec<AgentProject>, String> {
        self.inner.projects_list()
    }

    /// User-initiated folder registration; does not request approval.
    pub fn project_add(&self, name: String, path: String) -> Result<AgentProject, String> {
        self.inner.project_add(name, path)
    }

    /// Unregisters a project; its files are untouched. The last project cannot be removed.
    pub fn project_remove(&self, id: String) -> Result<(), String> {
        self.inner.project_remove(id)
    }

    pub fn project_rename(&self, id: String, name: String) -> Result<AgentProject, String> {
        self.inner.project_rename(id, name)
    }

    pub fn session_project_set(
        &self,
        session_id: String,
        project_id: String,
    ) -> Result<AgentSession, String> {
        self.inner.session_project_set(session_id, project_id)
    }

    /// Persists the mode without reopening the agent. An active run keeps its starting policy.
    pub fn permission_mode_set(
        &self,
        mode: AgentPermissionMode,
    ) -> Result<AgentPermissionMode, String> {
        self.inner.permission_mode_set(mode)
    }

    /// Every available model, the default first.
    pub fn models(&self) -> Result<Vec<AgentModel>, String> {
        self.inner.models()
    }

    /// Newest first.
    pub fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        self.inner.sessions()
    }

    pub fn create_session(&self, title: String) -> Result<AgentSession, String> {
        self.inner.create_session(title)
    }

    pub fn rename_session(
        &self,
        session_id: String,
        title: String,
    ) -> Result<AgentSession, String> {
        self.inner.rename_session(session_id, title)
    }

    /// Deletes the session and its history; refused while a run is going.
    pub fn delete_session(&self, session_id: String) -> Result<(), String> {
        self.inner.delete_session(session_id)
    }

    pub fn session_state(&self, session_id: String) -> Result<AgentSessionState, String> {
        self.inner.session_state(session_id)
    }

    /// Starts a run (with `model`, or the default), or queues a follow-up
    /// when the session is busy. Attached context is validated and formatted
    /// by the host; stored messages expose the original human text separately.
    pub fn send(
        &self,
        session_id: String,
        text: String,
        model: Option<String>,
        context: Option<AgentMessageContext>,
    ) -> Result<AgentSent, String> {
        self.inner.send(session_id, text, model, context)
    }

    pub fn claude_detection(&self) -> AgentClaudeDetection {
        self.inner.claude_detection()
    }

    /// Undoes a recorded edit. Refused if newer changes would be overwritten.
    pub fn edit_undo(&self, edit_id: String) -> Result<AgentEdit, String> {
        self.inner.edit_undo(edit_id)
    }

    /// Steers the running run; returns the queued input's id.
    pub fn steer(&self, session_id: String, text: String) -> Result<String, String> {
        self.inner.steer(session_id, text)
    }

    /// Whether anything was cancelled.
    pub fn cancel(&self, session_id: String) -> Result<bool, String> {
        self.inner.cancel(session_id)
    }

    /// `decision` is `approve`, `approve_session` or `deny` (with `reason`).
    pub fn decide_approval(
        &self,
        approval_id: String,
        decision: String,
        reason: Option<String>,
    ) -> Result<AgentApproval, String> {
        self.inner.decide_approval(approval_id, decision, reason)
    }

    /// Durable events after sequence `after` (at most 1000), optionally of one session.
    pub fn events_after(
        &self,
        after: i64,
        session_id: Option<String>,
    ) -> Result<AgentEvents, String> {
        self.inner.events_after(after, session_id)
    }

    /// Live events of every session until `close_events` (or the Dart
    /// stream is closed). After a `gap` event, catch up with `events_after`.
    /// `session_id` marks the chat this app is showing and enables its UI
    /// actions. A global listener does not count as showing a chat. A remote
    /// connection shows one chat at a time; a later scoped subscription switches it.
    pub fn subscribe(
        &self,
        session_id: Option<String>,
        sink: StreamSink<AgentEvent>,
    ) -> Result<(), String> {
        self.inner.subscribe(session_id, sink)
    }

    /// Ends every `subscribe` stream now: each Dart stream finishes without
    /// waiting for another event, so cancelling it returns at once. On a hub
    /// it also closes the socket; later calls fail.
    pub fn close_events(&self) -> Result<(), String> {
        self.inner.close_events()
    }

    /// Runs `code` as a `js` cell without a model and waits for it.
    pub fn run_script(
        &self,
        session_id: String,
        label: String,
        code: String,
    ) -> Result<AgentRunRecord, String> {
        self.inner.run_script(session_id, label, code)
    }

    /// A hub's schedule state for this device, as the hub reports it. On this
    /// computer the schedules belong to `NotesVault`: use its `local_schedule_state`.
    pub fn schedule_state(&self, page_id: String) -> Result<AgentScheduleState, String> {
        self.inner.schedule_state(page_id)
    }

    /// A hub's schedule acceptance for `hash` (from `schedule_preview` or
    /// `schedule_state`). Fails if the definition changed since. On this computer,
    /// use `NotesVault`'s `local_schedule_accept`.
    pub fn schedule_accept(
        &self,
        page_id: String,
        hash: String,
    ) -> Result<AgentScheduleState, String> {
        self.inner.schedule_accept(page_id, hash)
    }

    /// Pauses or resumes a hub's accepted schedule. On this computer, use
    /// `NotesVault`'s `local_schedule_pause`.
    pub fn schedule_pause(
        &self,
        page_id: String,
        paused: bool,
    ) -> Result<AgentScheduleState, String> {
        self.inner.schedule_pause(page_id, paused)
    }

    /// Runs a hub's schedule occurrence now, once. On this computer, use
    /// `NotesVault`'s `local_schedule_run_now`.
    pub fn schedule_run_now(&self, page_id: String) -> Result<AgentScheduleRun, String> {
        self.inner.schedule_run_now(page_id)
    }

    /// Checks a schedule definition (JSON text) on a hub and lists its next
    /// occurrences. Saves nothing. On this computer, `NotesVault.schedule_preview`
    /// does this without an agent.
    pub fn schedule_preview(&self, json: String) -> Result<AgentSchedulePreview, String> {
        self.inner.schedule_preview(json)
    }
}

/// Runs the claimed AI task `task_id` with the agent (runner `agent`) in a
/// session of its own, and writes the result or the error back to the task.
/// Local agent only: a hub agent returns an error.
pub fn ai_run_task_with_agent(
    agent: &NotesAgent,
    vault: &NotesVault,
    task_id: String,
) -> Result<AgentTaskRun, String> {
    agent.inner.run_task(vault, task_id)
}

/// Imports `<data_dir>/mcp.toml` once: Ok(None) when the file is missing;
/// Ok(Some(json)) with the servers in the same JSON shape
/// [`NotesAgent::set_mcp_servers`] takes, after renaming the file to
/// `mcp.toml.imported`; Err(text) when the file can't be read or parsed (the
/// file is left in place). Ok(None) where there is no local agent.
pub fn agent_mcp_import_file(data_dir: String) -> Result<Option<String>, String> {
    crate::agent_impl::mcp_import_file(&data_dir)
}

/// Reads assistant text (markdown) as blocks for display, with the vault's
/// own markdown reader: headings, lists (nested by indent), quotes, code
/// fences, dividers and inline marks. `[[wikilinks]]` resolve to pages by id
/// or title; so does a `[text](target)` link that is not a web or mail
/// link, by its target, else its text. Changes nothing. Available on every
/// platform.
#[frb(sync)]
pub fn agent_markdown_blocks(vault: &NotesVault, text: String) -> Vec<FlatBlock> {
    crate::agent_impl::markdown_blocks(&vault.inner, &text)
}

#[cfg(test)]
mod outline_tests {
    use super::*;

    #[test]
    fn compacted_history_on_a_hub_falls_back_to_full_messages() {
        let result = compacted_at(None, "any-session").unwrap();
        assert!(!result.compactable);
        assert!(result.segments.is_empty());
        assert_eq!(result.boundary, 0);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn compacted_history_bridge_preserves_card_variants_and_empty_live_end() {
        use vault_agent::compacted_history as core;
        let range = core::MessageRange { start: 0, end: 2 };
        let history: CompactedHistory = core::CompactedHistory {
            compactable: true,
            boundary: 2,
            message_count: 2,
            message_ids: vec!["m0".into(), "m1".into()],
            segments: vec![
                core::HistorySegment::LegacyCard(core::LegacyCard {
                    id: "checkpoint".into(),
                    kind: core::LegacyKind::Checkpoint,
                    title: None,
                    summary: "Saved summary".into(),
                    started_at: "first-time".into(),
                    ended_at: "last-time".into(),
                    first_message_id: "m0".into(),
                    last_message_id: "m1".into(),
                    range,
                    message_count: 2,
                }),
                core::HistorySegment::Messages(core::MessageRange { start: 2, end: 2 }),
            ],
        }
        .into();
        assert!(history.compactable);
        assert_eq!(history.boundary, 2);
        assert_eq!(history.message_count, 2);
        let card = &history.segments[0];
        assert_eq!(card.kind, HistorySegmentKind::LegacyCard);
        assert_eq!(card.range, MessageRange { start: 0, end: 2 });
        assert!(card.chapter.is_none());
        let legacy = card.legacy.as_ref().unwrap();
        assert_eq!(legacy.kind, LegacyKind::Checkpoint);
        assert_eq!(legacy.title, None);
        assert_eq!(legacy.summary, "Saved summary");
        assert_eq!(legacy.first_message_id, "m0");
        assert_eq!(legacy.last_message_id, "m1");
        assert_eq!(legacy.started_at, "first-time");
        assert_eq!(legacy.ended_at, "last-time");
        assert_eq!(legacy.message_count, 2);
        let tail = &history.segments[1];
        assert_eq!(tail.kind, HistorySegmentKind::Messages);
        assert_eq!(tail.range, MessageRange { start: 2, end: 2 });
        assert!(tail.chapter.is_none() && tail.legacy.is_none());
    }

    #[test]
    fn a_hub_has_no_local_outline() {
        assert_eq!(
            outline_at(None, "any-session").unwrap(),
            ChatOutline {
                chapters: Vec::new()
            }
        );
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn compacted_history_bridge_keeps_footer_only_on_final_part() {
        use vault_agent::compacted_history as core;
        let first = core::ChapterCard {
            id: "chapter".into(),
            status: core::ChapterStatus::Closed,
            title: "Chapter title".into(),
            summary: Some("Chapter summary".into()),
            part: 1,
            parts: 2,
            started_at: "first-time".into(),
            ended_at: "last-time".into(),
            first_message_id: "m0".into(),
            last_message_id: "m1".into(),
            range: core::MessageRange { start: 0, end: 2 },
            message_count: None,
            episode_count: None,
            decisions: Vec::new(),
            sections: Vec::new(),
            episodes: Vec::new(),
        };
        let last = core::ChapterCard {
            part: 2,
            message_count: Some(3),
            episode_count: Some(2),
            decisions: vec!["Keep canonical history".into()],
            sections: vec![core::ChapterSection {
                name: "Implementation".into(),
                first_episode: 1,
                last_episode: 2,
            }],
            ..first.clone()
        };
        let first: ChapterCard = first.into();
        let last: ChapterCard = last.into();
        assert_eq!(first.id, last.id);
        assert_eq!((first.part, first.parts), (1, 2));
        assert_eq!((last.part, last.parts), (2, 2));
        assert_eq!((first.message_count, first.episode_count), (None, None));
        assert_eq!((last.message_count, last.episode_count), (Some(3), Some(2)));
        assert!(first.decisions.is_empty());
        assert!(first.sections.is_empty());
        assert_eq!(last.decisions, ["Keep canonical history"]);
        assert_eq!(
            last.sections,
            [ChapterSection {
                name: "Implementation".into(),
                first_episode: 1,
                last_episode: 2,
            }]
        );
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn a_local_outline_matches_the_agent_reader() {
        use vault_agent::bubblepaw_sdk::store::Store;
        use vault_agent::outline::{OutlineChapter as Chapter, OutlineSection as Section};
        use vault_agent::outline::{OutlineStore, FILE_NAME};

        let dir = std::env::temp_dir().join(format!("notes_outline_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let agent_dir = dir.join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let data = dir.to_string_lossy().into_owned();

        // A session with no saved outline is an error, and no store is made for it.
        assert!(outline_at(Some(&data), "missing").is_err());

        // Keep the store open, as a running agent does: the helper reuses it.
        let store = Store::open(&agent_dir.join("agent.sqlite")).unwrap();
        let session = store.create_session("local").unwrap();
        let mut outline = OutlineStore::open(&agent_dir.join(FILE_NAME)).unwrap();
        outline
            .save_session(
                &session.id,
                &[Chapter {
                    id: "chapter-1".into(),
                    title: "First".into(),
                    summary: "Summary".into(),
                    decisions: vec!["Context boundary selected from 2 candidates".into()],
                    ongoing: true,
                    first_message_id: "m1".into(),
                    last_message_id: "m2".into(),
                    started_at: "2026-10-08T00:00:00Z".into(),
                    ended_at: None,
                    sections: vec![Section {
                        id: "section-1".into(),
                        title: "Part".into(),
                        summary: "Part summary".into(),
                        decisions: vec!["No context boundary selected from 3 candidates".into()],
                        first_message_id: "m1".into(),
                        last_message_id: "m2".into(),
                    }],
                }],
            )
            .unwrap();
        drop(outline);

        let local = outline_at(Some(&data), &session.id).unwrap();
        assert_eq!(local.chapters.len(), 1);
        let chapter = &local.chapters[0];
        assert_eq!(chapter.id, "chapter-1");
        assert!(chapter.ongoing);
        assert_eq!(chapter.ended_at, None);
        assert_eq!(chapter.sections[0].title, "Part");
        assert_eq!(
            chapter.sections[0].decisions,
            ["No context boundary selected from 3 candidates"]
        );

        // A saved outline does not become a compacted-history card, even when
        // its imported anchors are absent from the canonical store.
        let compacted = compacted_at(Some(&data), &session.id).unwrap();
        assert!(compacted.compactable);
        assert!(compacted.message_ids.is_empty());
        assert_eq!(compacted.segments.len(), 1);
        assert_eq!(compacted.segments[0].kind, HistorySegmentKind::Messages);
        assert_eq!(
            compacted.segments[0].range,
            MessageRange { start: 0, end: 0 }
        );
        assert!(compacted_at(Some(&data), "missing").is_err());

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
