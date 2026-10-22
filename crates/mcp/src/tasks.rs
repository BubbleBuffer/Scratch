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
//! - A task may be pinned to one device (`node` = its device id: "Ask AI"
//!   on a computer whose default runner is this computer); only that device
//!   claims it. Rows whose node is `hub` are jobs ([`crate::jobs`]); a job
//!   about a page uses the same placeholder.
//! - A task can be cancelled ([`cancel`]): a queued one is never claimed,
//!   and a running one's result is dropped when it ends.

use serde::{Deserialize, Serialize};
use vault_core::{
    Block, BlockKind, BlockOp, Condition, Filter, FilterOp, NewBlock, PageId, PageSummary, PropDef,
    PropKind, PropValue, SelectOption, Sort, Vault,
};

use crate::{err, Result};

/// The page property that marks the tasks database.
pub const MARKER: &str = "ai.tasks";
pub const TITLE: &str = "AI tasks";
/// The callout icon of a task's placeholder.
pub const ICON: &str = "✨";

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
/// Jobs only (see [`crate::jobs`]): cancelled by their owner.
pub const CANCELLED: &str = "cancelled";
/// Jobs only: the hub restarted while the job ran.
pub const INTERRUPTED: &str = "interrupted";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// The row's page id.
    pub id: PageId,
    pub prompt: String,
    /// The page the task is about (None if it was deleted).
    pub page: Option<PageId>,
    /// The placeholder block in that page.
    pub block: Option<String>,
    /// `agent` (the BubblePaw agent) or empty. Rows from before 2026-10-22
    /// may name a removed runner (`claude`, `codex`, `deepseek`); the agent
    /// answers them like any other.
    pub runner: String,
    /// Where it runs: empty (any desktop), a device id (only that device),
    /// or `hub` (a job, see [`crate::jobs`]).
    #[serde(default)]
    pub node: String,
    pub status: String,
    /// The device that claimed it.
    pub device: String,
    pub error: String,
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
        // Jobs (crate::jobs): the node that runs it (`hub`), the principal
        // that created it, its tool set, its session and when it ended.
        def("node", "Node", PropKind::Text, vec![]),
        def("owner", "Owner", PropKind::Text, vec![]),
        def("tools", "Tools", PropKind::Text, vec![]),
        def("session", "Session", PropKind::Text, vec![]),
        def("finished", "Finished", PropKind::Text, vec![]),
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
    Task {
        id: row.id.clone(),
        prompt: text(p, "prompt"),
        page,
        block,
        runner: text(p, "runner"),
        node: text(p, "node"),
        status: text(p, "status"),
        device: text(p, "device"),
        error: text(p, "error"),
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
        INTERRUPTED => "AI task interrupted",
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
/// the page. `runner` may be empty (the desktop's default).
pub fn create(
    v: &Vault,
    prompt: &str,
    page: &PageId,
    after: Option<&str>,
    runner: &str,
) -> Result<Task> {
    create_on(v, prompt, page, after, runner, "")
}

/// [`create`] for `node`: empty (any desktop claims it), a device id (only
/// that device claims it) or `hub` (a job; the jobs API sets its owner and
/// tool set).
pub fn create_on(
    v: &Vault,
    prompt: &str,
    page: &PageId,
    after: Option<&str>,
    runner: &str,
    node: &str,
) -> Result<Task> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return err("the task needs a prompt");
    }
    let target = v.load_page(page.clone())?;
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
    v.set_property(row.clone(), "prompt".into(), PropValue::Text(prompt.into()))?;
    v.set_property(
        row.clone(),
        "page".into(),
        PropValue::Relation(vec![page.clone()]),
    )?;
    v.set_property(row.clone(), "block".into(), PropValue::Text(block))?;
    v.set_property(
        row.clone(),
        "status".into(),
        PropValue::Select(QUEUED.into()),
    )?;
    if !runner.is_empty() {
        v.set_property(row.clone(), "runner".into(), PropValue::Text(runner.into()))?;
    }
    if !node.is_empty() {
        v.set_property(row.clone(), "node".into(), PropValue::Text(node.into()))?;
    }
    get(v, &row)
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

/// Claims the oldest queued task for `device` (marks it running): one with
/// no node, or one pinned to `device`.
pub fn claim_next(v: &Vault, device: &str) -> Result<Option<Task>> {
    let Some(c) = collection(v)? else {
        return Ok(None);
    };
    let sorts = vec![Sort {
        prop: "$created".into(),
        descending: false,
    }];
    // Jobs (node `hub`) run only on the hub, pinned tasks only on their
    // device.
    let Some(row) = v
        .query(Some(c), with_status(QUEUED), sorts, 0, 200)?
        .rows
        .into_iter()
        .find(|r| {
            let node = text(&r.properties, "node");
            node.is_empty() || (node == device && node != crate::jobs::NODE_HUB)
        })
    else {
        return Ok(None);
    };
    let t = task_of(&row);
    v.set_property(
        t.id.clone(),
        "device".into(),
        PropValue::Text(device.into()),
    )?;
    set_status(v, &t, RUNNING)?;
    Ok(Some(get(v, &t.id)?))
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
pub fn retry(v: &Vault, id: &str) -> Result<()> {
    let t = get(v, id)?;
    v.remove_property(t.id.clone(), "error".into())?;
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

/// Cancels a desktop task that hasn't ended (queued or running): it is
/// never claimed, and a running one's result is dropped when it ends. Jobs
/// (node `hub`) are cancelled through the hub's jobs API instead. Returns
/// whether the task was still going.
pub fn cancel(v: &Vault, id: &str) -> Result<bool> {
    let t = get(v, id)?;
    if t.node == crate::jobs::NODE_HUB {
        return err("this task runs on the hub: cancel it there");
    }
    if t.status != QUEUED && t.status != RUNNING {
        return Ok(false);
    }
    v.set_property(
        t.id.clone(),
        "status".into(),
        PropValue::Select(CANCELLED.into()),
    )?;
    set_placeholder(v, &t, CANCELLED)?;
    Ok(true)
}

/// Writes a finished task's result `md` into its page: in place of the
/// placeholder, or at the end when the placeholder is gone. An empty result
/// removes the placeholder (the agent edited the vault itself).
pub(crate) fn write_result(v: &Vault, t: &Task, md: &str) -> Result<()> {
    let md = md.trim().to_string();
    let Some(page) = &t.page else {
        return Ok(());
    };
    let Ok(p) = v.load_page(page.clone()) else {
        return Ok(());
    };
    let placeholder = t.block.as_deref().filter(|b| find(&p.blocks, b).is_some());
    match (placeholder, md.is_empty()) {
        (Some(b), _) => {
            v.replace_markdown(page.clone(), vec![b.to_string()], md)?;
        }
        (None, false) => {
            v.insert_markdown(page.clone(), None, None, true, md)?;
        }
        (None, true) => {}
    }
    Ok(())
}

/// Ends a task. `Ok(markdown)`: the result goes into the task's row and
/// replaces the placeholder in the target page (an empty result removes
/// the placeholder: the agent edited the vault itself). `Err(message)`:
/// the task fails and the placeholder says why. A cancelled task stays
/// cancelled and its result is dropped.
pub fn finish(v: &Vault, id: &str, result: std::result::Result<String, String>) -> Result<()> {
    let t = get(v, id)?;
    if t.status == CANCELLED {
        return Ok(());
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
            set_status(v, &t, DONE)
        }
        Err(msg) => {
            let msg = if msg.trim().is_empty() {
                "the agent failed".to_string()
            } else {
                msg
            };
            v.set_property(t.id.clone(), "error".into(), PropValue::Text(msg.clone()))?;
            v.set_property(
                t.id.clone(),
                "status".into(),
                PropValue::Select(FAILED.into()),
            )?;
            let t = Task { error: msg, ..t };
            set_placeholder(v, &t, FAILED)
        }
    }
}

/// How the agent running a task reaches the vault; sets the prompt's wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTools {
    /// A BubblePaw agent whose `js` tool exposes vault actions as `tools.*`.
    Js,
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
    if let Some(page) = &t.page {
        if let Ok(p) = v.load_page(page.clone()) {
            let mut md = v.blocks_markdown(
                p.blocks
                    .iter()
                    .filter(|b| Some(&b.id) != t.block.as_ref())
                    .cloned()
                    .collect(),
            );
            if md.chars().count() > 20_000 {
                md = format!(
                    "{}\n\n[… truncated; use read_page for the rest]",
                    md.chars().take(20_000).collect::<String>()
                );
            }
            out.push_str(&format!(
                "The task was asked on the page \"{}\" (page id {}). Its current content:\n<page>\n{}\n</page>\n\n",
                p.title, p.id, md
            ));
        }
    }
    out.push_str(
        "Your final reply is inserted into that page where the user asked. Reply with only the \
         content to insert, as markdown (no preamble). If you made the changes yourself with the \
         tools and there is nothing to insert, reply with nothing.",
    );
    Ok(out)
}
