//! AI tasks, stored in the vault so they sync: a task created on the phone
//! runs on the desktop.
//!
//! - Tasks are rows of a top-level database "AI tasks", found by the page
//!   property [`MARKER`] and created on first use. Its schema has fixed
//!   keys (see [`schema`]); the status is a select with the option ids
//!   `queued`, `running`, `done` and `failed`.
//! - Creating a task also puts a placeholder callout into the target page
//!   (an empty paragraph the task was asked from becomes it). The runner
//!   updates its text with the status, and replaces it with the result (or
//!   an error) when the task ends. So the status shows, and syncs, in the
//!   page itself.
//! - The runner (desktop) claims the oldest queued task, runs it, and
//!   writes the result into the row's body and the target page.
//! - A task is pinned to one device (`node` = its device id; every new task
//!   is, see [`create_on`]); only that device claims it. Rows with no node
//!   are legacy rows from before pinning, and any desktop claims them. Rows
//!   whose node is `hub` are jobs ([`crate::jobs`]); a job about a page uses
//!   the same placeholder.
//! - A task can be cancelled ([`cancel`]): a queued one is never claimed,
//!   and a running one's result is dropped when it ends.
//! - A running task can report its progress ([`report_progress`]). Only the
//!   row's latest message and its time change; the status and placeholder
//!   stay as they are, and the last report stays when the task ends.
//! - A run's todo list is mirrored into the page as nested todos (see
//!   [`subtasks`]): under the task's source todo, or under its placeholder
//!   when it has none.

use serde::{Deserialize, Serialize};
use vault_core::{
    Block, BlockKind, BlockOp, Condition, Filter, FilterOp, NewBlock, PageId, PageSummary, PropDef,
    PropKind, PropValue, SelectOption, Sort, Vault,
};

use crate::{err, Result};

pub mod attempts;
pub mod board;
mod subtasks;

#[cfg(test)]
mod source_tests;

pub use subtasks::{
    add_step, begin_subtasks, mirror_subtasks, pending_steps, record_baseline, Step, SubtaskItem,
};

/// The page property that marks the tasks database.
pub const MARKER: &str = "ai.tasks";
pub const TITLE: &str = "AI tasks";
/// The callout icon of a task's placeholder.
pub const ICON: &str = "✨";

/// Not a stored status: the placeholder text of a running task whose cancel
/// the runner has not acknowledged yet.
pub(crate) const CANCELLING: &str = "cancelling";

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
/// A finished run of a task with a review policy (see [`set_review`]): the
/// result waits for the user's acceptance, which makes it `done`.
pub const REVIEW: &str = "review";
pub const FAILED: &str = "failed";
/// Jobs only (see [`crate::jobs`]): cancelled by their owner.
pub const CANCELLED: &str = "cancelled";
/// Jobs only: the hub restarted while the job ran.
pub const INTERRUPTED: &str = "interrupted";

/// The most characters of a progress message kept (see [`report_progress`]).
pub const PROGRESS_MAX: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// The row's page id.
    pub id: PageId,
    pub prompt: String,
    /// The page the task is about (None if it was deleted).
    pub page: Option<PageId>,
    /// The placeholder block in that page.
    pub block: Option<String>,
    /// The todo block the run's subtasks nest under instead of the
    /// placeholder: set when the task was asked after a todo (see
    /// [`create_on_with`]).
    #[serde(default)]
    pub source_block: Option<String>,
    /// `agent` (the BubblePaw agent) or empty. Rows from before 2026-10-22
    /// may name a removed runner (`claude`, `codex`, `deepseek`); the agent
    /// answers them like any other.
    pub runner: String,
    /// Where it runs: a device id (only that device), `hub` (a job, see
    /// [`crate::jobs`]), or empty for a legacy row from before pinning (any
    /// desktop claims it).
    #[serde(default)]
    pub node: String,
    pub status: String,
    /// The device that claimed it.
    pub device: String,
    pub error: String,
    /// The latest progress report, empty until one is recorded.
    #[serde(default)]
    pub progress: String,
    /// Unix epoch milliseconds as stored in the row, empty until reported.
    #[serde(default)]
    pub progress_at: String,
    /// The project a code task runs in: a project id of its PC runner, empty
    /// for a task with no project.
    #[serde(default)]
    pub project: String,
    /// The git worktree a code task runs in, empty until its runner makes it.
    #[serde(default)]
    pub worktree_branch: String,
    #[serde(default)]
    pub worktree_path: String,
    /// The current attempt's id, empty until one starts (see [`attempts`]).
    #[serde(default)]
    pub attempt: String,
    /// When a cancel command was recorded (ms), while the runner has not
    /// acknowledged it. A task that is `running` with this set shows
    /// "cancellation requested".
    #[serde(default)]
    pub cancel_requested_ms: Option<i64>,
    /// The runner's last sign of life while it runs (ms), written about
    /// every minute. None for a row whose runner does not write one.
    #[serde(default)]
    pub heartbeat_ms: Option<i64>,
    /// The worker's open question to the user (see [`ask`]), empty when it
    /// has none. A running task with a question is waiting for the user's
    /// reply (`needs_input`).
    #[serde(default)]
    pub question: String,
    /// The agent session of the run, empty until it starts or after it expired.
    #[serde(default)]
    pub session: String,
    pub created_ms: i64,
    pub updated_ms: i64,
}

fn def(key: &str, name: &str, kind: PropKind, options: Vec<SelectOption>) -> PropDef {
    PropDef {
        key: key.into(),
        name: name.into(),
        kind,
        options,
        config: String::new(),
    }
}

fn opt(id: &str, name: &str, color: &str) -> SelectOption {
    SelectOption {
        id: id.into(),
        name: name.into(),
        color: color.into(),
    }
}

/// The tasks database's schema.
pub fn schema() -> Vec<PropDef> {
    vec![
        def(
            "status",
            "Status",
            PropKind::Select,
            vec![
                opt(QUEUED, "Queued", "gray"),
                opt(RUNNING, "Running", "blue"),
                opt(DONE, "Done", "green"),
                opt(REVIEW, "Review", "purple"),
                opt(FAILED, "Failed", "red"),
                opt(CANCELLED, "Cancelled", "default"),
                opt(INTERRUPTED, "Interrupted", "yellow"),
            ],
        ),
        def("prompt", "Prompt", PropKind::Text, vec![]),
        def("page", "Page", PropKind::Relation, vec![]),
        def("runner", "Runner", PropKind::Text, vec![]),
        def("device", "Ran on", PropKind::Text, vec![]),
        def("error", "Error", PropKind::Text, vec![]),
        def("block", "Block", PropKind::Text, vec![]),
        def("created", "Created", PropKind::CreatedTime, vec![]),
        // Every task names its runner node and records its session. Jobs
        // additionally record their owner, tool set and finish time.
        def("node", "Node", PropKind::Text, vec![]),
        def("owner", "Owner", PropKind::Text, vec![]),
        def("tools", "Tools", PropKind::Text, vec![]),
        def("session", "Session", PropKind::Text, vec![]),
        def("finished", "Finished", PropKind::Text, vec![]),
        // A running task's latest progress message, and when it was reported
        // (ms since the epoch, as `finished`).
        def("progress", "Progress", PropKind::Text, vec![]),
        def("progress_at", "Progress at", PropKind::Text, vec![]),
        // A code task's project (a PC runner's project id), and the git
        // worktree its runner made for it.
        def("project", "Project", PropKind::Text, vec![]),
        def("worktree_branch", "Worktree branch", PropKind::Text, vec![]),
        def("worktree_path", "Worktree path", PropKind::Text, vec![]),
        // A triage run (see [`crate::triage`]): its scope as JSON.
        def("triage", "Triage scope", PropKind::Text, vec![]),
        def("triage_actions", "Triage actions", PropKind::Text, vec![]),
        // The todo the subtasks nest under (empty: the placeholder), and the
        // run's subtask tracking (see [`subtasks`]).
        def("source_block", "Source block", PropKind::Text, vec![]),
        def("subtask_blocks", "Subtask blocks", PropKind::Text, vec![]),
        // The attempt history (see [`attempts`]) and a cancel command the
        // runner has not acknowledged yet.
        def(attempts::CURRENT, "Attempt", PropKind::Text, vec![]),
        def(attempts::HISTORY, "Attempts", PropKind::Text, vec![]),
        def(
            attempts::CANCEL_REQUESTED,
            "Cancel requested",
            PropKind::Text,
            vec![],
        ),
        def(
            attempts::UPDATE_REQUESTED,
            "Update requested",
            PropKind::Text,
            vec![],
        ),
        // The runner's last sign of life, and the review policy.
        def(HEARTBEAT, "Heartbeat", PropKind::Text, vec![]),
        def(REVIEW_POLICY, "Review", PropKind::Text, vec![]),
        // The worker's question to the user and the user's reply.
        def(QUESTION, "Question", PropKind::Text, vec![]),
        def(REPLY, "Reply", PropKind::Text, vec![]),
    ]
}

/// Adds the properties and status options of [`schema`] that an older
/// tasks database lacks (it was written once, when it was created).
/// Existing properties, options and values stay as they are.
pub fn migrate_schema(v: &Vault, collection: &PageId) -> Result<()> {
    let mut current = v.get_schema(collection.clone())?;
    let mut changed = false;
    for want in schema() {
        match current.iter_mut().find(|d| d.key == want.key) {
            Some(have) => {
                for o in want.options {
                    if !have.options.iter().any(|x| x.id == o.id) {
                        have.options.push(o);
                        changed = true;
                    }
                }
            }
            None => {
                current.push(want);
                changed = true;
            }
        }
    }
    if changed {
        v.set_schema(collection.clone(), current)?;
    }
    Ok(())
}

/// The tasks database, if it exists (the oldest one if two devices made
/// one each before syncing).
pub fn collection(v: &Vault) -> Result<Option<PageId>> {
    let mut found: Option<PageSummary> = None;
    let mut offset = 0;
    loop {
        let top = v.list_children(None, offset, 200)?;
        for p in &top {
            let marked = p.is_database()
                && p.properties
                    .iter()
                    .any(|x| x.key == MARKER && x.value == PropValue::Checkbox(true));
            if marked && found.as_ref().is_none_or(|f| p.created_ms < f.created_ms) {
                found = Some(p.clone());
            }
        }
        if top.len() < 200 {
            break;
        }
        offset += 200;
    }
    Ok(found.map(|p| p.id))
}

/// The tasks database, created if missing.
pub fn ensure_collection(v: &Vault) -> Result<PageId> {
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
        return Ok(c);
    }
    let id = v.create_page(None, TITLE.into())?;
    v.make_collection(id.clone())?;
    v.set_schema(id.clone(), schema())?;
    v.set_icon(id.clone(), Some(ICON.into()))?;
    v.set_property(id.clone(), MARKER.into(), PropValue::Checkbox(true))?;
    Ok(id)
}

pub(crate) fn text(p: &[vault_core::Property], key: &str) -> String {
    p.iter()
        .find(|x| x.key == key)
        .and_then(|x| match &x.value {
            PropValue::Text(s) | PropValue::Select(s) | PropValue::Status(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn task_of(row: &PageSummary) -> Task {
    let p = &row.properties;
    let page = p
        .iter()
        .find(|x| x.key == "page")
        .and_then(|x| match &x.value {
            PropValue::Relation(r) => r.first().cloned(),
            _ => None,
        });
    let block = Some(text(p, "block")).filter(|b| !b.is_empty());
    let source_block = Some(text(p, "source_block")).filter(|b| !b.is_empty());
    Task {
        id: row.id.clone(),
        prompt: text(p, "prompt"),
        page,
        block,
        source_block,
        runner: text(p, "runner"),
        node: text(p, "node"),
        status: text(p, "status"),
        device: text(p, "device"),
        error: text(p, "error"),
        progress: text(p, "progress"),
        progress_at: text(p, "progress_at"),
        project: text(p, "project"),
        worktree_branch: text(p, "worktree_branch"),
        worktree_path: text(p, "worktree_path"),
        attempt: text(p, attempts::CURRENT),
        cancel_requested_ms: text(p, attempts::CANCEL_REQUESTED).parse().ok(),
        heartbeat_ms: text(p, HEARTBEAT).parse().ok(),
        question: text(p, QUESTION),
        session: text(p, "session"),
        created_ms: row.created_ms,
        updated_ms: row.updated_ms,
    }
}

/// One task by its row id.
pub fn get(v: &Vault, id: &str) -> Result<Task> {
    let p = v.load_page(id.to_string())?;
    let summary = PageSummary {
        id: p.id.clone(),
        title: p.title.clone(),
        parent: p.parent.clone(),
        updated_ms: p.updated_ms,
        child_count: 0,
        created_ms: p.created_ms,
        icon: p.icon.clone(),
        page_type: p.page_type.clone(),
        favorite: p.favorite,
        properties: p.properties.clone(),
    };
    Ok(task_of(&summary))
}

pub(crate) fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line.to_string()
    }
}

fn placeholder_text(status: &str, prompt: &str) -> String {
    let what = match status {
        QUEUED => "AI task queued",
        RUNNING => "AI is working on this",
        FAILED => "AI task failed",
        CANCELLED => "AI task cancelled",
        CANCELLING => "AI task cancellation requested",
        INTERRUPTED => "AI task interrupted",
        DONE => "AI task done",
        REVIEW => "AI task ready for review",
        _ => "AI task",
    };
    format!("{what}: {}", first_line(prompt, 100))
}

fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|b| {
        if b.id == id {
            Some(b)
        } else {
            find(&b.children, id)
        }
    })
}

/// Creates a task about `page`. The placeholder goes after block `after`
/// (an empty paragraph there becomes the placeholder), or at the end of
/// the page. A todo named by `after` also becomes the source for the run's
/// nested subtasks. `runner` may be empty (the desktop's default).
pub fn create(
    v: &Vault,
    prompt: &str,
    page: &PageId,
    after: Option<&str>,
    runner: &str,
) -> Result<Task> {
    create_on(v, prompt, page, after, runner, "")
}

/// [`create`] for `node`: a device id (only that device claims it) or `hub`
/// (a job; the jobs API sets its owner and tool set). Empty means this
/// device, so a new row is never unpinned.
pub fn create_on(
    v: &Vault,
    prompt: &str,
    page: &PageId,
    after: Option<&str>,
    runner: &str,
    node: &str,
) -> Result<Task> {
    create_on_with(v, prompt, page, after, runner, node, &[])
}

/// [`create_on`] for a code task in `project`, a project id that the node's PC
/// runner lists (checked by [`crate::runners::check_project`] before anything
/// is written). An empty or blank project is no project, as in [`create_on`].
/// The project is set before the row is queued.
pub fn create_for_project(
    v: &Vault,
    prompt: &str,
    page: &PageId,
    after: Option<&str>,
    runner: &str,
    node: &str,
    project: Option<&str>,
) -> Result<Task> {
    let node = if node.is_empty() {
        v.device_id()
    } else {
        node.to_string()
    };
    let project = project.map(str::trim).filter(|p| !p.is_empty());
    crate::runners::check_project(v, &node, project.unwrap_or(""))?;
    let extra: Vec<(&str, &str)> = project.map(|p| ("project", p)).into_iter().collect();
    create_on_with(v, prompt, page, after, runner, &node, &extra)
}

/// [`create_on`] that also sets `extra` properties before the row is queued,
/// so a claim never sees the row without them (a job's owner and tools).
/// `source_block` in `extra` selects a todo independently of placement.
pub(crate) fn create_on_with(
    v: &Vault,
    prompt: &str,
    page: &PageId,
    after: Option<&str>,
    runner: &str,
    node: &str,
    extra: &[(&str, &str)],
) -> Result<Task> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return err("the task needs a prompt");
    }
    let target = v.load_page(page.clone())?;
    // Checked before anything is written.
    let source = source_todo(&target.blocks, after, extra)?;
    let coll = ensure_collection(v)?;
    // The placeholder.
    let text = placeholder_text(QUEUED, prompt);
    let reuse = after.and_then(|a| find(&target.blocks, a)).filter(|b| {
        b.kind == BlockKind::Paragraph && b.text.trim().is_empty() && b.children.is_empty()
    });
    let block = match reuse {
        Some(b) => {
            v.apply(
                page.clone(),
                vec![
                    BlockOp::SetKind {
                        id: b.id.clone(),
                        kind: BlockKind::Callout,
                    },
                    BlockOp::SetText {
                        id: b.id.clone(),
                        text,
                        marks: vec![],
                    },
                    BlockOp::SetTarget {
                        id: b.id.clone(),
                        target: Some(ICON.into()),
                    },
                    BlockOp::SetColor {
                        id: b.id.clone(),
                        color: Some("gray_background".into()),
                    },
                ],
            )?;
            b.id.clone()
        }
        None => {
            let (parent, prev) = match after.and_then(|a| parent_of(&target.blocks, None, a)) {
                Some(parent) => (parent, after.map(String::from)),
                None => (None, target.blocks.last().map(|b| b.id.clone())),
            };
            let id = vault_core::new_id();
            v.apply(
                page.clone(),
                vec![
                    BlockOp::Insert {
                        parent,
                        after: prev,
                        block: NewBlock {
                            id: id.clone(),
                            kind: BlockKind::Callout,
                            text,
                            marks: vec![],
                            checked: false,
                        },
                    },
                    BlockOp::SetTarget {
                        id: id.clone(),
                        target: Some(ICON.into()),
                    },
                    BlockOp::SetColor {
                        id: id.clone(),
                        color: Some("gray_background".into()),
                    },
                ],
            )?;
            id
        }
    };
    let row = v.create_page(Some(coll), first_line(prompt, 60))?;
    // The node before the status: a queued row always has its node, so no
    // claim can see a new pinned row as a legacy one.
    let node = if node.is_empty() {
        v.device_id()
    } else {
        node.to_string()
    };
    v.set_property(row.clone(), "node".into(), PropValue::Text(node))?;
    for (key, value) in extra.iter().filter(|(k, _)| *k != "source_block") {
        v.set_property(
            row.clone(),
            key.to_string(),
            PropValue::Text(value.to_string()),
        )?;
    }
    if let Some(source) = source {
        v.set_property(row.clone(), "source_block".into(), PropValue::Text(source))?;
    }
    v.set_property(row.clone(), "prompt".into(), PropValue::Text(prompt.into()))?;
    v.set_property(
        row.clone(),
        "page".into(),
        PropValue::Relation(vec![page.clone()]),
    )?;
    v.set_property(row.clone(), "block".into(), PropValue::Text(block))?;
    if !runner.is_empty() {
        v.set_property(row.clone(), "runner".into(), PropValue::Text(runner.into()))?;
    }
    v.set_property(
        row.clone(),
        "status".into(),
        PropValue::Select(QUEUED.into()),
    )?;
    get(v, &row)
}

/// The source todo of a task created after `after`: the `source_block` in
/// `extra` if given, else `after` when it is a todo. Anything else is no
/// source. A given source must be a todo of the page.
fn source_todo(
    blocks: &[Block],
    after: Option<&str>,
    extra: &[(&str, &str)],
) -> Result<Option<String>> {
    let is_todo = |id: &str| find(blocks, id).is_some_and(|b| b.kind == BlockKind::Todo);
    let given = extra
        .iter()
        .find(|(k, _)| *k == "source_block")
        .map(|(_, id)| id.trim())
        .filter(|id| !id.is_empty());
    let Some(id) = given.or_else(|| after.filter(|a| is_todo(*a))) else {
        return Ok(None);
    };
    if !is_todo(id) {
        return err(format!("source block {id:?} is not a todo of the page"));
    }
    Ok(Some(id.to_string()))
}

fn parent_of(blocks: &[Block], parent: Option<&str>, id: &str) -> Option<Option<String>> {
    for b in blocks {
        if b.id == id {
            return Some(parent.map(String::from));
        }
        if let Some(x) = parent_of(&b.children, Some(&b.id), id) {
            return Some(x);
        }
    }
    None
}

pub(crate) fn with_status(status: &str) -> Option<Filter> {
    Some(Filter::Cond(Condition {
        prop: "status".into(),
        op: FilterOp::Is,
        value: PropValue::Select(status.into()),
    }))
}

/// The newest tasks first.
pub fn list(v: &Vault, limit: u32) -> Result<Vec<Task>> {
    let Some(c) = collection(v)? else {
        return Ok(Vec::new());
    };
    let sorts = vec![Sort {
        prop: "$created".into(),
        descending: true,
    }];
    Ok(v.query(Some(c), None, sorts, 0, limit)?
        .rows
        .iter()
        .map(task_of)
        .collect())
}

fn set_status(v: &Vault, t: &Task, status: &str) -> Result<()> {
    v.set_property(
        t.id.clone(),
        "status".into(),
        PropValue::Select(status.into()),
    )?;
    if status == RUNNING || status == QUEUED {
        set_placeholder(v, t, status)?;
    }
    Ok(())
}

/// Sets the placeholder's text for `status` (if the block still exists).
pub(crate) fn set_placeholder(v: &Vault, t: &Task, status: &str) -> Result<()> {
    let (Some(page), Some(block)) = (&t.page, &t.block) else {
        return Ok(());
    };
    let Ok(p) = v.load_page(page.clone()) else {
        return Ok(());
    };
    if find(&p.blocks, block).is_none() {
        return Ok(());
    }
    let text = match status {
        FAILED | INTERRUPTED if !t.error.trim().is_empty() => format!(
            "{} ({})",
            placeholder_text(status, &t.prompt),
            first_line(&t.error, 200)
        ),
        _ => placeholder_text(status, &t.prompt),
    };
    let mut ops = vec![BlockOp::SetText {
        id: block.clone(),
        text,
        marks: vec![],
    }];
    if status == FAILED || status == INTERRUPTED {
        ops.push(BlockOp::SetColor {
            id: block.clone(),
            color: Some("red_background".into()),
        });
    }
    v.apply(page.clone(), ops)?;
    Ok(())
}

/// Whether `device` may claim a queued row whose node is `node`: one pinned
/// to it, or a legacy row with no node. Jobs (node `hub`) never go to a
/// desktop, and another device's rows never come here.
fn claimable(node: &str, device: &str) -> bool {
    node != crate::jobs::NODE_HUB && (node.is_empty() || node == device)
}

/// Claims the oldest queued task `device` may run (marks it running): see
/// [`claimable`].
pub fn claim_next(v: &Vault, device: &str) -> Result<Option<Task>> {
    let Some(c) = collection(v)? else {
        return Ok(None);
    };
    let oldest_first = || {
        vec![Sort {
            prop: "$created".into(),
            descending: false,
        }]
    };
    let first = v.query(Some(c.clone()), with_status(QUEUED), oldest_first(), 0, 200)?;
    // Other devices' rows must not hide this device's task: a longer queue
    // is read whole, in one query. Paging could skip a row when a claim on
    // another thread shifts the queue between two pages.
    let rows = if first.total as usize > first.rows.len() {
        v.query(Some(c), with_status(QUEUED), oldest_first(), 0, first.total)?
            .rows
    } else {
        first.rows
    };
    let Some(row) = rows
        .into_iter()
        .find(|r| claimable(&text(&r.properties, "node"), device))
    else {
        return Ok(None);
    };
    let t = task_of(&row);
    v.set_property(
        t.id.clone(),
        "device".into(),
        PropValue::Text(device.into()),
    )?;
    // A new attempt before the status flips: whoever sees `running` finds
    // its attempt.
    let actor = text(&row.properties, "owner");
    attempts::begin(
        v,
        &Task {
            device: device.into(),
            ..t.clone()
        },
        &actor,
    )?;
    let _ = heartbeat(v, &t.id);
    set_status(v, &t, RUNNING)?;
    Ok(Some(get(v, &t.id)?))
}

/// The row property holding the runner's last sign of life (ms).
pub(crate) const HEARTBEAT: &str = "heartbeat";
pub(crate) const QUESTION: &str = "question";
pub(crate) const REPLY: &str = "reply";

/// The most characters of a question or reply kept.
pub const QUESTION_MAX: usize = 2000;

/// A run that ended while it waited for the user's reply starts its error
/// with this: the attempt's outcome is "waiting for input", not a failure.
pub const WAITING_FOR_INPUT: &str = "waiting for input";

/// The worker asks the user a question and waits (the `ask_user` tool). The
/// task stays `running` with the question set; the card shows it with a reply
/// field. Refused when the task is not running or a cancel is pending.
pub fn ask(v: &Vault, id: &str, question: &str) -> Result<()> {
    let t = get(v, id)?;
    if t.status != RUNNING || t.cancel_requested_ms.is_some() {
        return err("the task is not running");
    }
    let q: String = question.trim().chars().take(QUESTION_MAX).collect();
    if q.is_empty() {
        return err("ask_user needs a question");
    }
    v.remove_property(t.id.clone(), REPLY.into())?;
    v.set_property(t.id.clone(), QUESTION.into(), PropValue::Text(q.clone()))?;
    let _ = attempts::event(v, id, None, attempts::kind::QUESTION, &q);
    Ok(())
}

/// The user answers the worker's question. The reply is stored for the
/// waiting tool and the question is cleared, which puts the task back to
/// plain `running`. A task whose run ended while it waited (`interrupted`,
/// "waiting for input") starts a new attempt instead, with the question and
/// the reply added to its prompt. False when there is no open question.
pub fn reply(v: &Vault, id: &str, text_in: &str) -> Result<bool> {
    let t = get(v, id)?;
    let r: String = text_in.trim().chars().take(QUESTION_MAX).collect();
    if t.question.is_empty() || r.is_empty() {
        return Ok(false);
    }
    if t.status == INTERRUPTED && t.error.starts_with(WAITING_FOR_INPUT) {
        // The run ended while it waited: a reply starts a new attempt on the
        // same runner, with the question and the reply added to the brief.
        let prompt = format!(
            "{}

Earlier in this task you asked the user: {}
The user replied: {r}",
            t.prompt, t.question
        );
        v.set_property(t.id.clone(), "prompt".into(), PropValue::Text(prompt))?;
        let _ = attempts::event(v, id, None, attempts::kind::REPLY, &r);
        retry(v, id)?;
        return Ok(true);
    }
    if t.status != RUNNING {
        return Ok(false);
    }
    v.set_property(t.id.clone(), REPLY.into(), PropValue::Text(r.clone()))?;
    v.remove_property(t.id.clone(), QUESTION.into())?;
    let _ = attempts::event(v, id, None, attempts::kind::REPLY, &r);
    Ok(true)
}

/// What the waiting tool looks at: `Some(reply)` once the user answered (the
/// reply is taken), `None` while the question is open, an error when the
/// task is no longer running.
pub fn take_reply(v: &Vault, id: &str) -> Result<Option<String>> {
    let t = get(v, id)?;
    if t.status != RUNNING || t.cancel_requested_ms.is_some() {
        return err("the task is no longer running");
    }
    let row = v.load_page(t.id.clone())?;
    let r = text(&row.properties, REPLY);
    if t.question.is_empty() && !r.is_empty() {
        v.remove_property(t.id.clone(), REPLY.into())?;
        return Ok(Some(r));
    }
    Ok(None)
}
/// The row property that makes a successful run end in `review`.
pub(crate) const REVIEW_POLICY: &str = "review";

/// A run with no sign of life for this long is shown as stalled.
pub const STALL_MS: i64 = 10 * 60 * 1000;

/// Makes the task's successful runs end in `review` instead of `done`.
pub fn set_review(v: &Vault, id: &str) -> Result<()> {
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    v.set_property(id.to_string(), REVIEW_POLICY.into(), PropValue::Text("1".into()))?;
    Ok(())
}

/// The user accepts a reviewed result: `review` becomes `done`. Returns
/// whether the task was in review.
pub fn accept(v: &Vault, id: &str) -> Result<bool> {
    let t = get(v, id)?;
    if t.status != REVIEW {
        return Ok(false);
    }
    set_status(v, &t, DONE)?;
    if let Some(card) = &t.page {
        let _ = board::move_card(v, card, board::Target::Last);
    }
    Ok(true)
}

/// Records the runner's sign of life on a running task. Called about every
/// minute by the runner while it works.
pub fn heartbeat(v: &Vault, id: &str) -> Result<()> {
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    v.set_property(
        id.to_string(),
        HEARTBEAT.into(),
        PropValue::Text(crate::jobs::now_ms().to_string()),
    )?;
    Ok(())
}

/// When the runner was last heard from, if the task is `running`, writes a
/// heartbeat and the last one is more than [`STALL_MS`] before `now`.
pub fn stalled_since(t: &Task, now: i64) -> Option<i64> {
    let beat = t.heartbeat_ms?;
    (t.status == RUNNING && now.saturating_sub(beat) > STALL_MS).then_some(beat)
}

/// Fails a task whose runner stalled, from another device. Refused (false)
/// when the task is not stalled; nothing ends a stalled task by itself.
pub fn mark_failed(v: &Vault, id: &str) -> Result<bool> {
    let t = get(v, id)?;
    if stalled_since(&t, crate::jobs::now_ms()).is_none() {
        return Ok(false);
    }
    finish(v, id, Err("the runner stopped responding".into()))?;
    Ok(true)
}

/// This device's ended tasks whose session (transcript and logs) is older
/// than `keep_ms`: the caller deletes each session and then calls
/// [`session_expired`]. The row, outcome and attempt history stay for good.
pub fn expired_sessions(
    v: &Vault,
    device: &str,
    keep_ms: i64,
    now: i64,
) -> Result<Vec<(Task, String)>> {
    let Some(c) = collection(v)? else {
        return Ok(vec![]);
    };
    let rows = v.query(Some(c), None, vec![], 0, 5000)?.rows;
    Ok(rows
        .iter()
        .map(|r| (task_of(r), text(&r.properties, "session")))
        .filter(|(t, session)| {
            !session.is_empty()
                && t.device == device
                && t.node != "hub"
                && ![QUEUED, RUNNING].contains(&t.status.as_str())
                && now.saturating_sub(t.updated_ms) >= keep_ms
        })
        .collect())
}

/// Clears a task's `session` once its session is deleted.
pub fn session_expired(v: &Vault, id: &str) -> Result<()> {
    v.remove_property(id.to_string(), "session".into())?;
    Ok(())
}

/// Marks tasks this device was running as failed (after a restart).
pub fn recover(v: &Vault, device: &str) -> Result<u32> {
    let Some(c) = collection(v)? else {
        return Ok(0);
    };
    let rows = v
        .query(Some(c), with_status(RUNNING), vec![], 0, 1000)?
        .rows;
    let mut n = 0;
    for r in rows.iter().map(task_of).filter(|t| t.device == device) {
        finish(
            v,
            &r.id,
            Err("interrupted: the app closed while it ran".into()),
        )?;
        n += 1;
    }
    Ok(n)
}

/// Puts a failed (or finished) task back in the queue.
///
/// Only an ended task is retried. A queued task is already waiting. A running
/// one, including one whose cancel the runner has not acknowledged, is an
/// error: there is one active attempt per task, and the new attempt starts
/// only after the old one ended.
pub fn retry(v: &Vault, id: &str) -> Result<()> {
    let t = get(v, id)?;
    match t.status.as_str() {
        QUEUED => return Ok(()),
        RUNNING if t.cancel_requested_ms.is_some() => {
            return err("the cancel is not acknowledged yet: retry once the task stops")
        }
        RUNNING => return err("the task is still running: cancel it before retrying"),
        _ => {}
    }
    v.remove_property(t.id.clone(), "error".into())?;
    v.remove_property(t.id.clone(), attempts::CANCEL_REQUESTED.into())?;
    v.remove_property(t.id.clone(), HEARTBEAT.into())?;
    v.remove_property(t.id.clone(), QUESTION.into())?;
    v.remove_property(t.id.clone(), REPLY.into())?;
    // The last attempt's progress line belongs to that attempt.
    v.remove_property(t.id.clone(), "progress".into())?;
    v.remove_property(t.id.clone(), "progress_at".into())?;
    if !t.project.is_empty() {
        // Review metadata belongs to an attempt, not to the reusable row.
        // Retain the checkout itself; the next run creates a fresh binding.
        v.remove_property(t.id.clone(), "worktree_branch".into())?;
        v.remove_property(t.id.clone(), "worktree_path".into())?;
    }
    // A placeholder replaced by a result is gone: add a new one.
    let has_block = match (&t.page, &t.block) {
        (Some(p), Some(b)) => v
            .load_page(p.clone())
            .ok()
            .is_some_and(|pg| find(&pg.blocks, b).is_some()),
        _ => false,
    };
    if !has_block {
        if let Some(page) = &t.page {
            let fresh = create_placeholder_at_end(v, page, &t.prompt)?;
            v.set_property(t.id.clone(), "block".into(), PropValue::Text(fresh))?;
        }
    } else if let (Some(p), Some(b)) = (&t.page, &t.block) {
        v.apply(
            p.clone(),
            vec![BlockOp::SetColor {
                id: b.clone(),
                color: Some("gray_background".into()),
            }],
        )?;
    }
    set_status(v, &get(v, id)?, QUEUED)
}

fn create_placeholder_at_end(v: &Vault, page: &PageId, prompt: &str) -> Result<String> {
    let p = v.load_page(page.clone())?;
    let id = vault_core::new_id();
    v.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: p.blocks.last().map(|b| b.id.clone()),
                block: NewBlock {
                    id: id.clone(),
                    kind: BlockKind::Callout,
                    text: placeholder_text(QUEUED, prompt),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: id.clone(),
                target: Some(ICON.into()),
            },
            BlockOp::SetColor {
                id: id.clone(),
                color: Some("gray_background".into()),
            },
        ],
    )?;
    Ok(id)
}

/// Cancels a desktop task that hasn't ended. A queued task is cancelled at
/// once: no runner has it. A running task gets a cancel request and stays
/// `running` until its runner stops the run and acknowledges (the runner
/// then ends it through [`finish_attempt`]); until then it shows
/// "cancellation requested", never stopped. Jobs (node `hub`) are cancelled
/// through the hub's jobs API instead. Returns whether the task was still
/// going.
pub fn cancel(v: &Vault, id: &str) -> Result<bool> {
    let t = get(v, id)?;
    if t.node == crate::jobs::NODE_HUB {
        return err("this task runs on the hub: cancel it there");
    }
    match t.status.as_str() {
        QUEUED => {
            v.set_property(
                t.id.clone(),
                "status".into(),
                PropValue::Select(CANCELLED.into()),
            )?;
            set_placeholder(v, &t, CANCELLED)?;
            Ok(true)
        }
        RUNNING => {
            request_cancel(v, &t)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Records a cancel command on a running task (see [`cancel`]) and shows it
/// in the placeholder. Idempotent.
pub(crate) fn request_cancel(v: &Vault, t: &Task) -> Result<()> {
    attempts::request_cancel(v, t)?;
    set_placeholder(v, t, CANCELLING)
}

/// Records a running task's latest progress: `message` is trimmed and kept
/// to [`PROGRESS_MAX`] characters, and the row gets it with the time it was
/// reported. Only those two properties change. An empty message, or a task
/// that is not running (queued, ended or cancelled), is an error and writes
/// nothing.
pub fn report_progress(v: &Vault, id: &str, message: &str) -> Result<()> {
    let message: String = message.trim().chars().take(PROGRESS_MAX).collect();
    if message.is_empty() {
        return err("the progress needs a message");
    }
    let t = get(v, id)?;
    if t.status != RUNNING {
        return err("only a running task takes progress");
    }
    if t.cancel_requested_ms.is_some() {
        return err("only a running task takes progress; this one is being cancelled");
    }
    // A database from before these properties gets them now: set_property
    // refuses a key its collection's schema lacks.
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    v.set_property(
        t.id.clone(),
        "progress".into(),
        PropValue::Text(message.clone()),
    )?;
    v.set_property(
        t.id.clone(),
        "progress_at".into(),
        PropValue::Text(crate::jobs::now_ms().to_string()),
    )?;
    // The history is best effort: the row already has the report.
    let _ = attempts::event(v, &t.id, None, attempts::kind::CHECKPOINT, &message);
    Ok(())
}

/// Writes a finished task's result `md` into its page: in place of the
/// placeholder, or at the end when the placeholder is gone. An empty result
/// removes the placeholder (the agent edited the vault itself). A placeholder
/// with children (the run's subtasks, see [`subtasks`]) stays: it is labelled
/// done and the result goes after it, so the subtasks stay nested under it.
pub(crate) fn write_result(v: &Vault, t: &Task, md: &str) -> Result<()> {
    let md = md.trim().to_string();
    let Some(page) = &t.page else {
        return Ok(());
    };
    let Ok(p) = v.load_page(page.clone()) else {
        return Ok(());
    };
    let placeholder = t.block.as_deref().and_then(|b| find(&p.blocks, b));
    match placeholder {
        Some(b) if !b.children.is_empty() => {
            v.apply(
                page.clone(),
                vec![BlockOp::SetText {
                    id: b.id.clone(),
                    text: placeholder_text(DONE, &t.prompt),
                    marks: vec![],
                }],
            )?;
            if !md.is_empty() {
                let parent = parent_of(&p.blocks, None, &b.id).flatten();
                v.insert_markdown(page.clone(), parent, Some(b.id.clone()), false, md)?;
            }
        }
        Some(b) => {
            v.replace_markdown(page.clone(), vec![b.id.clone()], md)?;
        }
        None if !md.is_empty() => {
            v.insert_markdown(page.clone(), None, None, true, md)?;
        }
        None => {}
    }
    Ok(())
}

/// Ends a task's current attempt: [`finish_attempt`] without naming the
/// attempt, for callers that cannot have a stale one (a restart's recovery).
pub fn finish(v: &Vault, id: &str, result: std::result::Result<String, String>) -> Result<()> {
    finish_attempt(v, id, None, result)
}

/// A summary of an outcome for the attempt record.
fn outcome_text(result: &std::result::Result<String, String>) -> String {
    match result {
        Ok(md) => md.clone(),
        Err(msg) => msg.clone(),
    }
}

/// Ends a task for the runner of attempt `attempt` (the current one when
/// None). `Ok(markdown)`: the result goes into the task's row and replaces
/// the placeholder in the target page (an empty result removes the
/// placeholder: the agent edited the vault itself). `Err(message)`: the task
/// fails and the placeholder says why.
///
/// Two cases never touch the task, and the outcome is kept in the attempt's
/// history instead:
/// - a late result: `attempt` is no longer the row's current attempt (it was
///   retried), or the task already ended (cancelled);
/// - a cancel the runner now acknowledges: the task becomes `cancelled`
///   whatever the run returned, so a result never replaces a cancellation.
pub fn finish_attempt(
    v: &Vault,
    id: &str,
    attempt: Option<&str>,
    result: std::result::Result<String, String>,
) -> Result<()> {
    let t = get(v, id)?;
    let summary = outcome_text(&result);
    let waiting = matches!(&result, Err(m) if m.starts_with(WAITING_FOR_INPUT));
    let outcome = if result.is_ok() {
        DONE
    } else if waiting {
        WAITING_FOR_INPUT
    } else {
        FAILED
    };
    let current = (!t.attempt.is_empty()).then(|| t.attempt.clone());
    let stale = match (attempt, &current) {
        (Some(a), Some(c)) => a != c,
        _ => false,
    };
    if stale
        || t.status == CANCELLED
        || (t.status != RUNNING && t.status != QUEUED && attempt.is_some())
    {
        // Not this attempt's turn: keep what it produced and change nothing.
        let target = attempt.map(str::to_string).or(current);
        if target.is_some() {
            let _ = attempts::close(v, id, target.as_deref(), outcome, Some(&summary), true);
        }
        return Ok(());
    }
    if t.cancel_requested_ms.is_some() && t.status == RUNNING {
        // The runner stopped (or the run raced the cancel): acknowledge.
        v.set_property(
            t.id.clone(),
            "status".into(),
            PropValue::Select(CANCELLED.into()),
        )?;
        set_placeholder(v, &t, CANCELLED)?;
        let _ = attempts::close(
            v,
            id,
            attempt,
            CANCELLED,
            result.as_ref().ok().map(String::as_str),
            false,
        );
        if result.is_ok() {
            let _ = attempts::event(
                v,
                id,
                attempt,
                attempts::kind::LATE_RESULT,
                "a result came after the cancel and was kept in history",
            );
        }
        let _ = attempts::event(v, id, attempt, attempts::kind::CANCEL_ACKNOWLEDGED, "");
        return Ok(());
    }
    if !waiting {
        v.remove_property(t.id.clone(), QUESTION.into())?;
        v.remove_property(t.id.clone(), REPLY.into())?;
    }
    let closed = attempts::close(v, id, attempt, outcome, Some(&summary), false);
    if let Err(e) = closed {
        // The history never blocks the task's own end.
        eprintln!("task {id}: could not record the attempt: {}", e.0);
    }
    match result {
        Ok(md) => {
            let md = md.trim().to_string();
            let body = if md.is_empty() {
                "(The agent changed the vault directly.)".to_string()
            } else {
                md.clone()
            };
            v.replace_markdown(t.id.clone(), vec![], body)?;
            write_result(v, &t, &md)?;
            v.remove_property(t.id.clone(), "error".into())?;
            let row = v.load_page(t.id.clone())?;
            if text(&row.properties, REVIEW_POLICY).is_empty() {
                set_status(v, &t, DONE)
            } else {
                set_status(v, &t, REVIEW)?;
                if let Some(card) = &t.page {
                    let _ = board::move_card(v, card, board::Target::Review);
                }
                Ok(())
            }
        }
        Err(msg) => {
            let msg = if msg.trim().is_empty() {
                "the agent failed".to_string()
            } else {
                msg
            };
            v.set_property(t.id.clone(), "error".into(), PropValue::Text(msg.clone()))?;
            let end = if waiting { INTERRUPTED } else { FAILED };
            v.set_property(t.id.clone(), "status".into(), PropValue::Select(end.into()))?;
            let t = Task { error: msg, ..t };
            set_placeholder(v, &t, end)
        }
    }
}

/// A stable fingerprint of the note a run was briefed from: the task's page
/// without the run's own blocks (the placeholder and what nests in it, the
/// mirrored subtasks, and the to-dos the user added as steering). Editing
/// anything else in the page changes it. Empty when the task has no page.
/// The same text gives the same value on every device.
pub fn source_revision(v: &Vault, t: &Task) -> String {
    let Some(page) = &t.page else {
        return String::new();
    };
    let Ok(p) = v.load_page(page.clone()) else {
        return String::new();
    };
    let mut skip: std::collections::HashSet<String> = std::collections::HashSet::new();
    skip.extend(t.block.iter().cloned());
    if let Ok(row) = v.load_page(t.id.clone()) {
        skip.extend(subtasks::owned_blocks(&text(
            &row.properties,
            "subtask_blocks",
        )));
        if let Ok(all) = attempts::list(v, &t.id) {
            for a in all {
                skip.extend(a.steered);
            }
        }
    }
    fn without(blocks: &[Block], skip: &std::collections::HashSet<String>) -> Vec<Block> {
        blocks
            .iter()
            .filter(|b| !skip.contains(&b.id))
            .map(|b| {
                let mut b = b.clone();
                b.children = without(&b.children, skip);
                b
            })
            .collect()
    }
    let md = v.blocks_markdown(without(&p.blocks, &skip));
    // FNV-1a, 64 bit: short and the same everywhere.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in md.bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// How the agent running a task reaches the vault; sets the prompt's wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTools {
    /// A BubblePaw agent whose `js` tool exposes vault actions as `tools.*`.
    Js,
}

/// The task's page as the worker is shown it: its title and id, and its
/// content without the task's placeholder. None when the task has no page.
fn page_context(v: &Vault, t: &Task) -> Option<String> {
    let page = t.page.as_ref()?;
    let p = v.load_page(page.clone()).ok()?;
    let mut md = v.blocks_markdown(
        p.blocks
            .iter()
            .filter(|b| Some(&b.id) != t.block.as_ref())
            .cloned()
            .collect(),
    );
    if md.chars().count() > 20_000 {
        md = format!(
            "{}\n\n[\u{2026} truncated; use read_page for the rest]",
            md.chars().take(20_000).collect::<String>()
        );
    }
    Some(format!(
        "The task was asked on the page \"{}\" (page id {}). Its current content:\n<page>\n{}\n</page>\n\n",
        p.title, p.id, md
    ))
}

/// Whether the note changed since the running attempt's brief was made: the
/// worker runs on an earlier revision of it. False for a task that is not
/// running, or whose attempt recorded no revision.
pub fn is_stale(v: &Vault, t: &Task) -> bool {
    if t.status != RUNNING || t.attempt.is_empty() {
        return false;
    }
    let Ok(all) = attempts::list(v, &t.id) else {
        return false;
    };
    let Some(a) = all.iter().find(|a| a.id == t.attempt) else {
        return false;
    };
    !a.source_rev.is_empty() && a.source_rev != source_revision(v, t)
}

/// Asks the runner to send the worker the note's current version. Like a
/// cancel, it is a command: the runner sends it and the history shows
/// `update_sent` when it has. Only a running task that is not being cancelled
/// takes it. Returns whether the request was recorded.
pub fn request_update(v: &Vault, id: &str) -> Result<bool> {
    let t = get(v, id)?;
    if t.status != RUNNING || t.cancel_requested_ms.is_some() {
        return Ok(false);
    }
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    let row = v.load_page(id.to_string())?;
    if text(&row.properties, attempts::UPDATE_REQUESTED).is_empty() {
        v.set_property(
            t.id.clone(),
            attempts::UPDATE_REQUESTED.into(),
            PropValue::Text(crate::jobs::now_ms().to_string()),
        )?;
        let _ = attempts::event(v, id, None, attempts::kind::UPDATE_REQUESTED, "");
    }
    Ok(true)
}

/// The message to steer the worker with, and the revision it carries, when an
/// update was requested for the running task. None otherwise.
pub fn update_message(v: &Vault, id: &str) -> Result<Option<(String, String)>> {
    let t = get(v, id)?;
    let row = v.load_page(id.to_string())?;
    if t.status != RUNNING
        || t.cancel_requested_ms.is_some()
        || text(&row.properties, attempts::UPDATE_REQUESTED).is_empty()
    {
        return Ok(None);
    }
    let Some(context) = page_context(v, &t) else {
        return Ok(None);
    };
    let message = format!(
        "The user changed the note since you started. Continue the task with this version, and keep \
         what you already did where it still applies.\n\n{context}"
    );
    Ok(Some((message, source_revision(v, &t))))
}

/// Records that the update reached the worker: the attempt now works from
/// `rev`, and the request is cleared.
pub fn update_sent(v: &Vault, id: &str, attempt: Option<&str>, rev: &str) -> Result<()> {
    attempts::update(v, id, attempt, |a| a.source_rev = rev.to_string())?;
    let _ = attempts::event(v, id, attempt, attempts::kind::UPDATE_SENT, "");
    v.remove_property(id.to_string(), attempts::UPDATE_REQUESTED.into())?;
    Ok(())
}

/// The prompt given to an agent: the task, the target page's content and
/// how the result is used. A BubblePaw agent (`PromptTools::Js`) reaches
/// the vault through its `js` tool.
pub fn agent_prompt(v: &Vault, t: &Task, tools: PromptTools) -> Result<String> {
    let mut out = String::new();
    match tools {
        PromptTools::Js => out.push_str(
            "You are helping the user with their personal notes app. You can read and edit the \
             notes vault with your js tool, where vault actions are async tools.* functions. \
             tools.search_notes, tools.read_page and tools.create_page are shown in its description; \
             find the others with tools.$search.\n\n",
        ),
    }
    out.push_str(&format!("Task from the user:\n{}\n\n", t.prompt));
    if let Some(context) = page_context(v, t) {
        out.push_str(&context);
    }
    out.push_str(
        "Your final reply is inserted into that page where the user asked. Reply with only the \
         content to insert, as markdown (no preamble). If you made the changes yourself with the \
         tools and there is nothing to insert, reply with nothing.",
    );
    Ok(out)
}
