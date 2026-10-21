//! Dev-only replay of a recorded BubblePaw session through the normal agent run path.
//!
//! Gated by the process environment: `NOTES_DEV_REPLAY=1` registers the `replay`
//! provider (model `replay/replay`) when the agent opens, and enables [`load`].
//!
//! The provider never calls a model and never executes recorded code. Each
//! recorded assistant record becomes one `ModelResponse`: its text is streamed
//! paragraph by paragraph, then stored by the harness as the normal assistant
//! message. Each recorded tool call becomes a no-op `js` call with a unique
//! synthetic ID, so the native tool result is a normal durable message. Every
//! native message carries its source `seq` in `provider_state`, so outline anchors
//! map by source seq, not by title or text.
//!
//! The replay's read-only source is `replay.sqlite`. Chapters grow with their
//! native message range; sections publish only when their last source message
//! has a native anchor (see [`status`]). Session state lives in a process-wide registry
//! keyed by session ID, shared by the provider and the free functions.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Context, Result};
use async_trait::async_trait;
use bubblepaw_core::{
    CancellationToken, Message, ModelInfo, ModelRequest, ModelResponse, Provider, Role,
    TextCallback, ToolCall,
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::watch;
use vault_tools::harness::{self, Part};
use vault_tools::outline::{OutlineChapter, OutlineSection, OutlineStore};

/// The provider ID; models are `replay/<model>`.
pub const PROVIDER_ID: &str = "replay";
/// The only model. Sessions with a loaded replay always run on it.
pub const MODEL_ID: &str = "replay/replay";
/// Pause between streamed paragraphs when the caller has no preference.
pub const DEFAULT_DELAY_MS: u64 = 300;
/// A recorded turn can hold ~190 assistant records; the default cap is 32 rounds.
pub(crate) const MAX_ROUNDS: usize = 4096;

const ENV: &str = "NOTES_DEV_REPLAY";
const NOOP_CODE: &str = "text('Recorded work omitted from the replay.')";
const SUMMARY_TEXT: &str = "# Replayed earlier conversation\nEarlier recorded turns are not replayed as model context; this summary stands in for them.";

/// Whether this process has dev replay on (`NOTES_DEV_REPLAY=1`).
pub fn enabled() -> bool {
    std::env::var(ENV).is_ok_and(|value| value == "1")
}

/// What a dev driver needs: the recorded user texts in send order, how many
/// have completed, and the replay's state. `state` is one of `waiting`
/// (no turn in progress), `running`, `paused`, `stopped`, `complete` or `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayStatus {
    pub user_texts: Vec<String>,
    pub completed_users: u32,
    pub state: String,
    pub error: Option<String>,
}

/// Loads `replay` (a read-only `replay.sqlite`) into the new, empty `session`.
/// `delay_ms` is the pause between streamed paragraphs. Errors when dev replay is
/// off, the session is unknown, not empty, or already has a replay.
pub fn load(data_dir: &Path, session: &str, replay: &Path, delay_ms: u64) -> Result<ReplayStatus> {
    ensure!(
        enabled(),
        "dev replay is disabled; start with {ENV}=1 to enable it"
    );
    let (store, _) = super::agent_store_at(data_dir)?;
    store.session(session)?;
    ensure!(
        store.messages(session)?.is_empty() && store.runs(session)?.is_empty(),
        "dev replay needs a new empty session"
    );
    ensure!(
        lookup(session).is_none(),
        "this session already has a dev replay"
    );
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let source = read_source(replay, &nonce)?;
    let state = Arc::new(Replay::new(source, Duration::from_millis(delay_ms)));
    let status = state.snapshot();
    REPLAYS
        .lock()
        .map_err(|_| anyhow!("replay registry poisoned"))?
        .insert(session.into(), state);
    Ok(status)
}

/// The replay's status. Also publishes every chapter and section whose anchors
/// now exist as native messages, so the outline grows while the replay runs.
pub fn status(data_dir: &Path, session: &str) -> Result<ReplayStatus> {
    let state =
        lookup(session).ok_or_else(|| anyhow!("no dev replay is loaded for this session"))?;
    let (store, outline_path) = super::agent_store_at(data_dir)?;
    state.sync_outline(&store, &outline_path, session)?;
    Ok(state.snapshot())
}

/// Applies `pause`, `resume` or `stop`. Stop ends the replay for this session:
/// the provider's next call fails, and an in-flight paragraph stream stops after
/// its current paragraph. It does not cancel the run itself; use `cancel` for that.
pub fn control(session: &str, action: &str) -> Result<ReplayStatus> {
    let state =
        lookup(session).ok_or_else(|| anyhow!("no dev replay is loaded for this session"))?;
    let next = match action {
        "pause" => Control::Paused,
        "resume" => Control::Running,
        "stop" => Control::Stopped,
        other => bail!("unknown replay action {other:?}; use pause, resume or stop"),
    };
    let current = *state.control.borrow();
    if current != Control::Stopped {
        state.control.send_replace(next);
    }
    Ok(state.snapshot())
}

/// Whether a replay is loaded for `session`.
pub(crate) fn is_loaded(session: &str) -> bool {
    lookup(session).is_some()
}

/// Drops the replay state of a deleted session.
pub(crate) fn forget(session: &str) {
    if let Ok(mut registry) = REPLAYS.lock() {
        registry.remove(session);
    }
}

/// Registered as the `replay` provider when [`enabled`].
pub(crate) struct ReplayProvider;

#[async_trait]
impl Provider for ReplayProvider {
    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "replay".into(),
            name: "Dev replay (recorded session)".into(),
        }]
    }

    async fn generate(
        &self,
        request: ModelRequest,
        on_text: TextCallback,
        cancellation: CancellationToken,
    ) -> Result<ModelResponse> {
        // Context compaction asks for a chapter with no tools. Answer locally.
        if request.tools.is_empty() {
            return Ok(ModelResponse {
                text: SUMMARY_TEXT.into(),
                ..ModelResponse::default()
            });
        }
        let state = lookup(&request.session_id)
            .ok_or_else(|| anyhow!("dev replay is not loaded for this session"))?;
        state.respond(&request, on_text, cancellation).await
    }
}

type Registry = Mutex<HashMap<String, Arc<Replay>>>;
static REPLAYS: LazyLock<Registry> = LazyLock::new(Registry::default);

fn lookup(session: &str) -> Option<Arc<Replay>> {
    REPLAYS.lock().ok()?.get(session).cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Running,
    Paused,
    Stopped,
}

/// One loaded replay: the source plan, run controls and progress.
struct Replay {
    source: Source,
    delay: Duration,
    control: watch::Sender<Control>,
    progress: Mutex<Progress>,
}

#[derive(Default)]
struct Progress {
    completed: u32,
    /// The turn in progress, an index into `source.turns`.
    turn: Option<usize>,
    /// The next step of the turn in progress.
    cursor: usize,
    /// Call IDs the last response issued; the next tool result must be one.
    pending: Vec<String>,
    error: Option<String>,
}

impl Replay {
    fn new(source: Source, delay: Duration) -> Self {
        let (control, _) = watch::channel(Control::Running);
        Self {
            source,
            delay,
            control,
            progress: Mutex::default(),
        }
    }

    fn snapshot(&self) -> ReplayStatus {
        let progress = self.progress.lock().unwrap();
        let control = *self.control.borrow();
        let state = if progress.error.is_some() {
            "error"
        } else if control == Control::Stopped {
            "stopped"
        } else if progress.completed as usize == self.source.turns.len() {
            "complete"
        } else if control == Control::Paused {
            "paused"
        } else if progress.turn.is_some() {
            "running"
        } else {
            "waiting"
        };
        ReplayStatus {
            user_texts: self
                .source
                .turns
                .iter()
                .map(|t| t.user_text.clone())
                .collect(),
            completed_users: progress.completed,
            state: state.into(),
            error: progress.error.clone(),
        }
    }

    /// Validates the request against the plan, then streams and returns the next
    /// step. Anything unexpected fails the run and never advances the plan.
    async fn respond(
        &self,
        request: &ModelRequest,
        on_text: TextCallback,
        cancellation: CancellationToken,
    ) -> Result<ModelResponse> {
        let (step, last) = self.begin(request)?;
        // Cancellation drops this future, so the interruption is recorded on drop.
        let mut interrupt = Interrupt {
            replay: self,
            done: false,
        };
        self.stream(&step.text, &on_text, &cancellation).await?;
        if last {
            let mut progress = self.progress.lock().unwrap();
            progress.completed += 1;
            progress.turn = None;
            progress.cursor = 0;
        }
        interrupt.done = true;
        let calls = step
            .calls
            .iter()
            .map(|call| ToolCall {
                id: call.id.clone(),
                name: "js".into(),
                arguments: json!({ "code": NOOP_CODE }),
            })
            .collect();
        let tag = Tag {
            replay: TagBody {
                seq: step.seq,
                calls: step
                    .calls
                    .iter()
                    .map(|c| TagCall {
                        id: c.id.clone(),
                        tool_seq: c.tool_seq,
                    })
                    .collect(),
            },
        };
        Ok(ModelResponse {
            text: step.text,
            tool_calls: calls,
            usage: None,
            last_request_usage: None,
            provider_state: Some(serde_json::to_value(tag)?),
        })
    }

    /// Checks the request is the expected next user message or tool result and
    /// takes the next step. Returns the step and whether it ends the turn.
    fn begin(&self, request: &ModelRequest) -> Result<(Step, bool)> {
        let mut progress = self.progress.lock().unwrap();
        if *self.control.borrow() == Control::Stopped {
            bail!("dev replay was stopped");
        }
        // The view can end with per-step runtime notes (system, or generated user
        // notices since BubblePaw records them with an origin); the transcript
        // tail is the last message that is neither.
        let last = request.messages.iter().rev().find(|message| {
            message.role != Role::System
                && (message.role != Role::User || bubblepaw_core::is_typed_user(message))
        });
        match last {
            Some(message) if message.role == Role::User => {
                if progress.turn.is_some() {
                    return Err(reject(
                        &mut progress,
                        "the previous recorded turn did not finish; a dev replay cannot resume mid-turn".into(),
                    ));
                }
                let index = progress.completed as usize;
                let Some(turn) = self.source.turns.get(index) else {
                    return Err(reject(
                        &mut progress,
                        "every recorded user message has already been sent".into(),
                    ));
                };
                if message.content != turn.user_text {
                    let text = format!(
                        "user message {} does not match the recorded text",
                        index + 1
                    );
                    return Err(reject(&mut progress, text));
                }
                progress.turn = Some(index);
                progress.cursor = 0;
                progress.error = None;
            }
            Some(message) if message.role == Role::Tool => {
                let id = message.tool_call_id.as_deref().unwrap_or_default();
                if progress.turn.is_none() || !progress.pending.iter().any(|p| p == id) {
                    return Err(reject(
                        &mut progress,
                        "a tool result does not answer the replay's last call".into(),
                    ));
                }
            }
            _ => {
                return Err(reject(
                    &mut progress,
                    "dev replay expects a user message or a tool result".into(),
                ))
            }
        }
        let index = progress.turn.expect("a turn is active");
        let turn = &self.source.turns[index];
        let step = turn.steps[progress.cursor].clone();
        progress.cursor += 1;
        progress.pending = step.calls.iter().map(|c| c.id.clone()).collect();
        let last = progress.cursor == turn.steps.len();
        Ok((step, last))
    }

    /// Streams `text` piece by piece, waiting for each piece's delay. The pieces
    /// concatenate to `text` exactly.
    async fn stream(
        &self,
        text: &str,
        on_text: &TextCallback,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        for piece in text.split_inclusive("\n\n") {
            self.wait_running(cancellation).await?;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => bail!("Cancelled during replay"),
                _ = tokio::time::sleep(self.delay) => {}
            }
            on_text(piece);
        }
        Ok(())
    }

    async fn wait_running(&self, cancellation: &CancellationToken) -> Result<()> {
        let mut control = self.control.subscribe();
        loop {
            let state = *control.borrow_and_update();
            match state {
                Control::Running => return Ok(()),
                Control::Stopped => bail!("dev replay was stopped"),
                Control::Paused => {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => bail!("Cancelled during replay"),
                        changed = control.changed() => {
                            changed.map_err(|_| anyhow!("replay control closed"))?;
                        }
                    }
                }
            }
        }
    }

    /// Publishes every chapter and section whose anchors are native messages now.
    /// Sections go in source order; a chapter grows from its first native message
    /// and closes at its source boundary once all its sections are ready. Writes
    /// only what changed, so polling does not bump the outline revision.
    fn sync_outline(
        &self,
        store: &bubblepaw_core::Store,
        path: &Path,
        session: &str,
    ) -> Result<()> {
        let messages = store.messages(session)?;
        let native = self.native_ids(&messages);
        let created: HashMap<&str, &str> = messages
            .iter()
            .map(|m| (m.id.as_str(), m.created_at.as_str()))
            .collect();
        let order: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
        let existing = OutlineStore::read_session(path, session)?.unwrap_or_default();
        let mut outline = OutlineStore::open(path)?;
        for chapter in &self.source.chapters {
            let first_seq = chapter
                .episodes
                .first()
                .expect("chapter has sections")
                .first_seq;
            let last_seq = chapter
                .episodes
                .last()
                .expect("chapter has sections")
                .last_seq;
            let Some(first_id) = native.get(&first_seq).cloned() else {
                break;
            };
            // Include the current, unfinished section in the chapter's span.
            // Otherwise a bottom reader would look outside the old chapter
            // just before it closes, and its messages could fold under them.
            let last_id = native
                .iter()
                .filter(|(seq, _)| **seq >= first_seq && **seq <= last_seq)
                .max_by_key(|(seq, _)| **seq)
                .expect("the first anchor exists")
                .1
                .clone();
            let mut ready = Vec::new();
            for episode in &chapter.episodes {
                match (
                    native.get(&episode.first_seq),
                    native.get(&episode.last_seq),
                ) {
                    (Some(first), Some(last)) => ready.push((episode, first.clone(), last.clone())),
                    _ => break,
                }
            }
            let all_ready = ready.len() == chapter.episodes.len();
            let closed = chapter.closed && all_ready;
            let current = existing.iter().find(|c| c.first_message_id == first_id);
            let current_open = current.is_none_or(|c| c.ended_at.is_none());
            let id = current.map_or_else(vault_core::new_id, |c| c.id.clone());
            if current_open {
                let (title, summary) = match (&chapter.description, &chapter.summary) {
                    (Some(title), Some(summary)) if closed => (title.clone(), summary.clone()),
                    _ => (
                        chapter.episodes[0].title.clone(),
                        ready
                            .iter()
                            .map(|(e, _, _)| e.summary.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                    ),
                };
                outline.upsert_chapter(
                    session,
                    &order,
                    OutlineChapter {
                        id: id.clone(),
                        title,
                        summary,
                        decisions: ready
                            .iter()
                            .flat_map(|(e, _, _)| e.decisions.clone())
                            .collect(),
                        ongoing: true,
                        started_at: created
                            .get(first_id.as_str())
                            .copied()
                            .unwrap_or_default()
                            .into(),
                        ended_at: None,
                        first_message_id: first_id.clone(),
                        last_message_id: last_id.clone(),
                        sections: Vec::new(),
                    },
                )?;
            }
            for (episode, first, last) in &ready {
                let section =
                    current.and_then(|c| c.sections.iter().find(|s| &s.first_message_id == first));
                let unchanged = section.is_some_and(|s| {
                    s.title == episode.title
                        && s.summary == episode.summary
                        && s.decisions == episode.decisions
                        && &s.last_message_id == last
                });
                if !unchanged {
                    outline.upsert_section(
                        session,
                        &order,
                        &first_id,
                        OutlineSection {
                            id: section.map_or_else(vault_core::new_id, |s| s.id.clone()),
                            title: episode.title.clone(),
                            summary: episode.summary.clone(),
                            decisions: episode.decisions.clone(),
                            first_message_id: first.clone(),
                            last_message_id: last.clone(),
                        },
                    )?;
                }
            }
            if closed && current_open {
                let ended_at = created.get(last_id.as_str()).copied().unwrap_or_default();
                outline.close_chapter(session, &order, &first_id, &last_id, ended_at)?;
            }
            if !all_ready {
                break;
            }
        }
        super::outline_titles::refresh(store, path, session)?;
        Ok(())
    }

    /// Source seq → native message ID, for every native message that carries one.
    fn native_ids(&self, messages: &[Message]) -> HashMap<i64, String> {
        let mut native = HashMap::new();
        let mut tool_seq_of_call: HashMap<String, i64> = HashMap::new();
        // A rejected send leaves a user message that is not the recorded text, so
        // users are matched to the next expected recorded text, not by position.
        let mut next_user = 0;
        for message in messages {
            match message.role {
                Role::User => {
                    if let Some(turn) = self.source.turns.get(next_user) {
                        if message.content == turn.user_text {
                            native.insert(turn.user_seq, message.id.clone());
                            next_user += 1;
                        }
                    }
                }
                Role::Assistant => {
                    let Some(tag) = message
                        .provider_state
                        .clone()
                        .and_then(|v| serde_json::from_value::<Tag>(v).ok())
                    else {
                        continue;
                    };
                    if let Some(seq) = tag.replay.seq {
                        native.insert(seq, message.id.clone());
                    }
                    for call in tag.replay.calls {
                        if let Some(seq) = call.tool_seq {
                            tool_seq_of_call.insert(call.id, seq);
                        }
                    }
                }
                Role::Tool => {
                    let call = message.tool_call_id.as_deref().unwrap_or_default();
                    if let Some(seq) = tool_seq_of_call.get(call) {
                        native.insert(*seq, message.id.clone());
                    }
                }
                Role::System => {}
            }
        }
        // A dropped seq takes the native message of the kept one that stands in for it.
        for (from, to) in &self.source.aliases {
            if let Some(id) = native.get(to).cloned() {
                native.entry(*from).or_insert(id);
            }
        }
        native
    }
}

/// Marks a turn interrupted when its response future is dropped before it ends.
/// A cancelled or failed stream leaves the turn half-recorded: the next send is
/// refused rather than replaying the turn from its start. A stop is not an error.
struct Interrupt<'a> {
    replay: &'a Replay,
    done: bool,
}

impl Drop for Interrupt<'_> {
    fn drop(&mut self) {
        if self.done || *self.replay.control.borrow() == Control::Stopped {
            return;
        }
        if let Ok(mut progress) = self.replay.progress.lock() {
            progress.error = Some("the recorded turn was interrupted".into());
        }
    }
}

/// Rejects a request: records the error, ends the turn, and returns the error.
fn reject(progress: &mut Progress, message: String) -> anyhow::Error {
    progress.error = Some(message.clone());
    progress.turn = None;
    progress.cursor = 0;
    progress.pending.clear();
    anyhow!(message)
}

#[derive(Serialize, Deserialize)]
struct Tag {
    replay: TagBody,
}

#[derive(Serialize, Deserialize)]
struct TagBody {
    seq: Option<i64>,
    calls: Vec<TagCall>,
}

#[derive(Serialize, Deserialize)]
struct TagCall {
    id: String,
    tool_seq: Option<i64>,
}

/// The recorded session as a plan: turns of steps, and the recorded outline.
struct Source {
    turns: Vec<Turn>,
    chapters: Vec<Chapter>,
    /// Source seq of each hidden harness message (and its dropped replies) → the
    /// source seq of the kept message that stands in for it.
    aliases: HashMap<i64, i64>,
}

struct Turn {
    user_seq: i64,
    user_text: String,
    steps: Vec<Step>,
}

/// One provider response: a recorded assistant record (`seq`), or a synthetic
/// continuation or final response (`seq` None).
#[derive(Clone)]
struct Step {
    seq: Option<i64>,
    text: String,
    calls: Vec<Call>,
}

#[derive(Clone)]
struct Call {
    id: String,
    /// The recorded tool result this call stands in for, if any.
    tool_seq: Option<i64>,
}

struct Chapter {
    closed: bool,
    description: Option<String>,
    summary: Option<String>,
    episodes: Vec<Episode>,
}

struct Episode {
    first_seq: i64,
    last_seq: i64,
    title: String,
    summary: String,
    decisions: Vec<String>,
}

/// Reads the replay read-only: the messages (system context skipped), the turn
/// plan, and the subject chapters with their episodes and decisions.
fn read_source(path: &Path, nonce: &str) -> Result<Source> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", path.display()))?;
    let mut records: Vec<(i64, Message)> = Vec::new();
    let mut stmt = conn.prepare("SELECT seq, data FROM messages ORDER BY seq")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (seq, data) = row?;
        let message: Message = serde_json::from_str(&data).context("parse a replay message")?;
        if message.role != Role::System {
            records.push((seq, message));
        }
    }
    drop(stmt);

    ensure!(!records.is_empty(), "replay has no conversation messages");
    ensure!(
        records
            .iter()
            .map(|(_, m)| m.session_id.as_str())
            .collect::<HashSet<_>>()
            .len()
            == 1,
        "replay must contain one source session"
    );

    let calls_recorded: HashSet<String> = records
        .iter()
        .filter(|(_, m)| m.role == Role::Assistant)
        .flat_map(|(_, m)| m.tool_calls.iter().map(|c| c.id.clone()))
        .collect();
    let mut tool_of_call: HashMap<String, i64> = HashMap::new();
    for (seq, message) in &records {
        if message.role != Role::Tool {
            continue;
        }
        let id = message
            .tool_call_id
            .clone()
            .with_context(|| format!("replay tool message {seq} has no call ID"))?;
        ensure!(
            calls_recorded.contains(&id),
            "replay tool message {seq} answers no recorded call"
        );
        ensure!(
            tool_of_call.insert(id, *seq).is_none(),
            "replay tool message {seq} repeats a recorded call"
        );
    }

    // Harness-written user text is not a turn. Its replies stay in the turn before,
    // and seqs of dropped messages alias the kept message that stands in for them.
    let parts: Vec<Part> = records
        .iter()
        .map(|(_, m)| match m.role {
            Role::User => Part::User(&m.content),
            Role::Assistant => {
                Part::Assistant(m.tool_calls.iter().map(|c| c.id.as_str()).collect())
            }
            _ => Part::Tool(m.tool_call_id.as_deref().unwrap_or_default()),
        })
        .collect();
    let visible = harness::visible(&parts);
    let aliases: HashMap<i64, i64> = (0..records.len())
        .filter(|&i| !visible.keep[i])
        .filter_map(|i| visible.stand_in[i].map(|t| (records[i].0, records[t].0)))
        .collect();
    let kept: Vec<(i64, Message)> = records
        .iter()
        .zip(&visible.keep)
        .filter(|(_, keep)| **keep)
        .map(|(record, _)| record.clone())
        .collect();

    let mut turns: Vec<Turn> = Vec::new();
    for (seq, message) in &kept {
        match message.role {
            Role::User => turns.push(Turn {
                user_seq: *seq,
                user_text: message.content.clone(),
                steps: Vec::new(),
            }),
            Role::Assistant => {
                let turn = turns.last_mut().with_context(|| {
                    format!("replay assistant message {seq} comes before any user message")
                })?;
                let calls = message
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(i, call)| Call {
                        id: format!("replay-{nonce}-{seq}-{i}"),
                        tool_seq: tool_of_call.get(&call.id).copied(),
                    })
                    .collect();
                turn.steps.push(Step {
                    seq: Some(*seq),
                    text: message.content.clone(),
                    calls,
                });
            }
            _ => {}
        }
    }
    for turn in &mut turns {
        let mut steps = Vec::with_capacity(turn.steps.len() + 1);
        let count = turn.steps.len();
        for (i, mut step) in std::mem::take(&mut turn.steps).into_iter().enumerate() {
            let last_assistant = i + 1 == count;
            if step.calls.is_empty() && !last_assistant {
                // Keeps the native message boundary: a no-op js call continues the turn.
                let seq = step.seq.unwrap_or_default();
                step.calls.push(Call {
                    id: format!("replay-{nonce}-{seq}-c"),
                    tool_seq: None,
                });
            }
            let ends_with_calls = !step.calls.is_empty();
            steps.push(step);
            if last_assistant && ends_with_calls {
                steps.push(Step {
                    seq: None,
                    text: String::new(),
                    calls: Vec::new(),
                });
            }
        }
        if steps.is_empty() || steps.last().is_some_and(|s| !s.calls.is_empty()) {
            steps.push(Step {
                seq: None,
                text: String::new(),
                calls: Vec::new(),
            });
        }
        turn.steps = steps;
    }

    let chapters = read_chapters(&conn, &records)?;
    ensure!(!turns.is_empty(), "replay has no user messages");
    Ok(Source {
        turns,
        chapters,
        aliases,
    })
}

/// The subject chapters in order, each with its episodes in order and their
/// decisions. Mirrors the importer's wording for decisions.
fn read_chapters(conn: &Connection, records: &[(i64, Message)]) -> Result<Vec<Chapter>> {
    let seqs: HashSet<i64> = records.iter().map(|(seq, _)| *seq).collect();
    let mut decisions: HashMap<i64, Vec<String>> = HashMap::new();
    // `split` rows are segmentation bookkeeping, not decisions a person made.
    let mut stmt = conn.prepare(
        "SELECT episode_ordinal, data FROM context_decisions WHERE lower(kind) <> 'split' ORDER BY episode_ordinal",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (ordinal, data) = row?;
        let value: Value = serde_json::from_str(&data).context("parse a replay decision")?;
        let count = value["option_message_ids"].as_array().map_or(0, Vec::len);
        let text = if value["choice"].is_string() {
            format!("Context boundary selected from {count} candidates")
        } else {
            format!("No context boundary selected from {count} candidates")
        };
        decisions.entry(ordinal).or_default().push(text);
    }
    drop(stmt);

    let mut episodes: HashMap<String, Vec<(i64, Episode)>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT ordinal, first_message_seq, last_message_seq, chapter_id, data FROM context_episodes ORDER BY ordinal",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (ordinal, first_seq, last_seq, chapter_id, data) = row?;
        let value: Value = serde_json::from_str(&data).context("parse a replay episode")?;
        ensure!(
            seqs.contains(&first_seq) && seqs.contains(&last_seq) && first_seq <= last_seq,
            "replay episode {ordinal} has anchors outside the messages"
        );
        let title = non_empty(value["title"].as_str())
            .with_context(|| format!("replay episode {ordinal} has no title"))?;
        episodes.entry(chapter_id).or_default().push((
            ordinal,
            Episode {
                first_seq,
                last_seq,
                title,
                summary: value["summary"].as_str().unwrap_or("").to_string(),
                decisions: decisions.remove(&ordinal).unwrap_or_default(),
            },
        ));
    }
    drop(stmt);

    let mut stmt =
        conn.prepare("SELECT id, status, data FROM context_subject_chapters ORDER BY ordinal")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut chapters = Vec::new();
    for row in rows {
        let (id, status, data) = row?;
        let value: Value = serde_json::from_str(&data).context("parse a replay chapter")?;
        let members = episodes
            .remove(&id)
            .with_context(|| format!("replay chapter {id} has no episodes"))?;
        chapters.push(Chapter {
            closed: status == "closed",
            description: non_empty(value["description"].as_str()),
            summary: non_empty(value["summary"].as_str()),
            episodes: members.into_iter().map(|(_, episode)| episode).collect(),
        });
    }
    ensure!(
        episodes.is_empty(),
        "replay episodes belong to no subject chapter"
    );
    Ok(chapters)
}

fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty()).map(str::to_string)
}
