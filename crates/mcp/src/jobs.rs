//! Jobs: runs that outside agents start through the `jobs` tool, stored as
//! rows of the "AI tasks" database ([`crate::tasks`]). The row page is the
//! job note: properties hold the state, the node, the owner, the tool set and
//! the session; the body holds the result.
//!
//! A job row has `node` = [`NODE_HUB`], the only node that runs jobs; a row
//! pinned to a desktop is a task, never a job. Desktops never claim a job
//! ([`crate::tasks::claim_next`] skips hub rows). Only the node's jobs API
//! changes a job's state; this module is its storage.
//!
//! A job may be about a page ("Ask AI" or a task assigned to the agent, sent
//! by a device): it then has a target page and a placeholder there, like a
//! desktop AI task, and its end is written into that page too.
//!
//! States: `queued`, `running`, then `completed` (stored as the task status
//! `done`), `failed`, `cancelled` or `interrupted` (the node restarted while
//! the job ran; it is not queued again).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vault_core::{PageId, PageSummary, PropValue, Sort, Vault};

use crate::tasks::{self, text, CANCELLED, DONE, FAILED, INTERRUPTED, QUEUED, RUNNING};
use crate::{err, Result};

/// The node value of jobs the hub runs.
pub const NODE_HUB: &str = "hub";

/// The API name of the completed state (stored as [`DONE`]).
pub const COMPLETED: &str = "completed";

/// One job, as the jobs API reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    pub id: PageId,
    pub prompt: String,
    /// `queued`, `running`, `completed`, `failed`, `cancelled` or `interrupted`.
    pub status: String,
    pub node: String,
    /// The principal that created it, like `agent:<token name>`.
    #[serde(skip)]
    pub owner: String,
    /// The tool set the run gets: `read`, `write` or `everything`.
    pub tools: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// The page the job is about, whose placeholder gets its result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<PageId>,
    /// The run's final text (completed jobs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// When it was created (ms since the epoch).
    #[serde(rename = "created")]
    pub created_ms: i64,
    /// When it ended (ms since the epoch).
    #[serde(rename = "finished", skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<i64>,
}

impl Job {
    /// Whether the job has ended (it can't change any more).
    pub fn ended(&self) -> bool {
        !matches!(self.status.as_str(), QUEUED | RUNNING)
    }
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Completed(String),
    Failed(String),
    Cancelled,
    Interrupted,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn api_status(stored: &str) -> String {
    match stored {
        DONE => COMPLETED.to_string(),
        s => s.to_string(),
    }
}

fn job_of(v: &Vault, row: &PageSummary, with_result: bool) -> Job {
    let p = &row.properties;
    let status = api_status(&text(p, "status"));
    let result = (with_result && status == COMPLETED)
        .then(|| v.load_page(row.id.clone()).ok())
        .flatten()
        .map(|page| v.blocks_markdown(page.blocks).trim().to_string());
    Job {
        id: row.id.clone(),
        prompt: text(p, "prompt"),
        status,
        node: text(p, "node"),
        owner: text(p, "owner"),
        tools: text(p, "tools"),
        session: text(p, "session"),
        error: text(p, "error"),
        page: p
            .iter()
            .find(|x| x.key == "page")
            .and_then(|x| match &x.value {
                PropValue::Relation(r) => r.first().cloned(),
                _ => None,
            }),
        result,
        created_ms: row.created_ms,
        finished_ms: text(p, "finished").parse().ok(),
    }
}

fn summary(v: &Vault, id: &str) -> Result<Option<PageSummary>> {
    let Ok(p) = v.load_page(id.to_string()) else {
        return Ok(None);
    };
    if p.in_trash {
        return Ok(None);
    }
    let Some(coll) = tasks::collection(v)? else {
        return Ok(None);
    };
    if p.parent.as_ref() != Some(&coll) {
        return Ok(None);
    }
    Ok(Some(PageSummary {
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
    }))
}

/// Creates a queued job for `node` (only [`NODE_HUB`] runs jobs), owned by
/// `owner`, with tool set `tools`.
pub fn create(v: &Vault, prompt: &str, owner: &str, tools: &str, node: &str) -> Result<Job> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return err("the job needs a prompt");
    }
    if node != NODE_HUB {
        return err("a job runs on the hub");
    }
    let coll = tasks::ensure_collection(v)?;
    let row = v.create_page(Some(coll), tasks::first_line(prompt, 60))?;
    // The node before the status, as in [`tasks::create_on`].
    v.set_property(row.clone(), "node".into(), PropValue::Text(node.into()))?;
    v.set_property(row.clone(), "prompt".into(), PropValue::Text(prompt.into()))?;
    v.set_property(row.clone(), "owner".into(), PropValue::Text(owner.into()))?;
    v.set_property(row.clone(), "tools".into(), PropValue::Text(tools.into()))?;
    v.set_property(
        row.clone(),
        "status".into(),
        PropValue::Select(QUEUED.into()),
    )?;
    match get(v, &row)? {
        Some(job) => Ok(job),
        None => err("the job was not saved"),
    }
}

/// [`create`] for a job about `page`: a placeholder goes after block
/// `after` (an empty paragraph there becomes it) or at the page's end, and
/// the job's end is written there.
pub fn create_for_page(
    v: &Vault,
    prompt: &str,
    owner: &str,
    tools: &str,
    node: &str,
    page: &PageId,
    after: Option<&str>,
) -> Result<Job> {
    if node != NODE_HUB {
        return err("a job runs on the hub");
    }
    // The owner and tools are set before the row is queued: the hub may
    // start a queued job at once.
    let extra = [("owner", owner), ("tools", tools)];
    let task = tasks::create_on_with(v, prompt, page, after, "", node, &extra)?;
    let row = task.id;
    match get(v, &row)? {
        Some(job) => Ok(job),
        None => err("the job was not saved"),
    }
}

/// The prompt a job's run gets: a job about a page gets the page's content
/// and how its reply is used (as a desktop AI task does); others `None`.
pub fn page_prompt(v: &Vault, id: &str) -> Result<Option<String>> {
    let t = tasks::get(v, id)?;
    if t.page.is_none() {
        return Ok(None);
    }
    tasks::agent_prompt(v, &t, tasks::PromptTools::Js).map(Some)
}

/// Updates the placeholder of a job about a page (no-op for other jobs).
fn placeholder(v: &Vault, id: &str, status: &str) -> Result<()> {
    let t = tasks::get(v, id)?;
    tasks::set_placeholder(v, &t, status)
}

/// The job with row id `id`, with its result; None when there is no such
/// job (another row, a trashed row, no row, or a desktop task row).
pub fn get(v: &Vault, id: &str) -> Result<Option<Job>> {
    Ok(summary(v, id)?
        .map(|row| job_of(v, &row, true))
        .filter(|j| j.node == NODE_HUB))
}

/// Every job (hub rows), newest first, without results. Desktop task rows
/// are not jobs.
pub fn all(v: &Vault) -> Result<Vec<Job>> {
    let Some(c) = tasks::collection(v)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let sorts = vec![Sort {
            prop: "$created".into(),
            descending: true,
        }];
        let rows = v.query(Some(c.clone()), None, sorts, offset, 500)?.rows;
        let n = rows.len() as u32;
        out.extend(
            rows.iter()
                .map(|r| job_of(v, r, false))
                .filter(|j| j.node == NODE_HUB),
        );
        if n < 500 {
            break;
        }
        offset += 500;
    }
    Ok(out)
}

/// Marks a queued job running in `session`.
pub fn mark_running(v: &Vault, id: &str, session: &str) -> Result<()> {
    v.set_property(
        id.to_string(),
        "session".into(),
        PropValue::Text(session.into()),
    )?;
    v.set_property(
        id.to_string(),
        "device".into(),
        PropValue::Text(NODE_HUB.into()),
    )?;
    v.set_property(
        id.to_string(),
        "status".into(),
        PropValue::Select(RUNNING.into()),
    )?;
    placeholder(v, id, RUNNING)
}

/// Ends a job: the result goes into the row's body, an error into `error`.
pub fn finish(v: &Vault, id: &str, end: End) -> Result<()> {
    let id = id.to_string();
    let status = match &end {
        End::Completed(md) => {
            let md = md.trim();
            let body = if md.is_empty() {
                "(The agent gave no final text.)"
            } else {
                md
            };
            v.replace_markdown(id.clone(), vec![], body.to_string())?;
            v.remove_property(id.clone(), "error".into())?;
            // A job about a page answers into it, like a desktop AI task.
            let t = tasks::get(v, &id)?;
            tasks::write_result(v, &t, md)?;
            DONE
        }
        End::Failed(msg) => {
            let msg = if msg.trim().is_empty() {
                "the run failed"
            } else {
                msg.as_str()
            };
            v.set_property(id.clone(), "error".into(), PropValue::Text(msg.into()))?;
            FAILED
        }
        End::Cancelled => CANCELLED,
        End::Interrupted => {
            v.set_property(
                id.clone(),
                "error".into(),
                PropValue::Text("interrupted: the hub restarted while the job ran".into()),
            )?;
            INTERRUPTED
        }
    };
    v.set_property(
        id.clone(),
        "finished".into(),
        PropValue::Text(now_ms().to_string()),
    )?;
    v.set_property(
        id.clone(),
        "status".into(),
        PropValue::Select(status.into()),
    )?;
    if status != DONE {
        placeholder(v, &id, status)?;
    }
    Ok(())
}

/// Marks `node`'s running jobs interrupted (after a restart). They are not
/// queued again.
pub fn recover(v: &Vault, node: &str) -> Result<u32> {
    let mut n = 0;
    for job in all(v)?
        .into_iter()
        .filter(|j| j.node == node && j.status == RUNNING)
    {
        finish(v, &job.id, End::Interrupted)?;
        n += 1;
    }
    Ok(n)
}

/// Moves `node`'s jobs that ended more than `keep_ms` before `now` to the
/// trash. Returns how many.
pub fn sweep(v: &Vault, node: &str, keep_ms: i64, now: i64) -> Result<u32> {
    let mut n = 0;
    for job in all(v)?.into_iter().filter(|j| j.node == node && j.ended()) {
        let ended = job.finished_ms.unwrap_or(job.created_ms);
        if now.saturating_sub(ended) >= keep_ms {
            v.delete_page(job.id.clone())?;
            n += 1;
        }
    }
    Ok(n)
}
