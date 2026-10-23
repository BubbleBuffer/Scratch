//! vault_mcp for Dart: AI tasks (rows of the synced "AI tasks" collection,
//! answered by the in-app agent, `ai_run_task_with_agent`). Only type
//! conversion happens here.

use vault_mcp::tasks;

use crate::api::vault::NotesVault;
use crate::convert::*;

/// An AI task (a row of the "AI tasks" collection).
#[derive(Debug, Clone)]
pub struct AiTask {
    pub id: String,
    pub prompt: String,
    /// The page the task is about (None if it was deleted).
    pub page: Option<String>,
    /// The placeholder block in that page.
    pub block: Option<String>,
    /// "" or "agent"; rows from before 2026-10-22 may name a removed runner
    /// ("claude", "codex", "deepseek"). The agent answers every row.
    pub runner: String,
    /// "queued", "running", "done" or "failed".
    pub status: String,
    /// The device that ran it.
    pub device: String,
    pub error: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    /// Where it runs: "" (any desktop), a device id (only that device) or
    /// "hub" (a hub job).
    pub node: String,
}

fn task(t: tasks::Task) -> AiTask {
    AiTask {
        id: t.id,
        prompt: t.prompt,
        page: t.page,
        block: t.block,
        runner: t.runner,
        status: t.status,
        device: t.device,
        error: t.error,
        created_ms: t.created_ms,
        updated_ms: t.updated_ms,
        node: t.node,
    }
}

/// Queues a task about `page`; its placeholder goes after block `after`
/// (an empty paragraph there becomes the placeholder) or at the end.
pub fn ai_create_task(
    vault: &NotesVault,
    prompt: String,
    page: String,
    after: Option<String>,
    runner: String,
) -> Result<AiTask, String> {
    tasks::create(&vault.inner, &prompt, &page, after.as_deref(), &runner).map(task).map_err(err)
}

/// [`ai_create_task`] pinned to this device: only this device claims it
/// ("Ask AI" when the default runner is this computer).
pub fn ai_create_task_here(
    vault: &NotesVault,
    prompt: String,
    page: String,
    after: Option<String>,
) -> Result<AiTask, String> {
    let v = &vault.inner;
    tasks::create_on(v, &prompt, &page, after.as_deref(), "", &v.device_id()).map(task).map_err(err)
}

/// Cancels a desktop task that hasn't ended: it is never claimed, and a
/// running one's result is dropped. Returns whether it was still going.
/// Hub jobs are cancelled on the hub.
pub fn ai_cancel_task(vault: &NotesVault, id: String) -> Result<bool, String> {
    tasks::cancel(&vault.inner, &id).map_err(err)
}

/// The newest tasks first.
pub fn ai_list_tasks(vault: &NotesVault, limit: u32) -> Result<Vec<AiTask>, String> {
    Ok(tasks::list(&vault.inner, limit).map_err(err)?.into_iter().map(task).collect())
}

pub fn ai_get_task(vault: &NotesVault, id: String) -> Result<AiTask, String> {
    tasks::get(&vault.inner, &id).map(task).map_err(err)
}

/// The "AI tasks" collection page, if it exists.
pub fn ai_tasks_collection(vault: &NotesVault) -> Result<Option<String>, String> {
    tasks::collection(&vault.inner).map_err(err)
}

/// Marks the oldest queued task running on this device and returns it.
pub fn ai_claim_next(vault: &NotesVault) -> Result<Option<AiTask>, String> {
    let v = &vault.inner;
    Ok(tasks::claim_next(v, &v.device_id()).map_err(err)?.map(task))
}

/// Fails the tasks this device left running (after a restart).
pub fn ai_recover_tasks(vault: &NotesVault) -> Result<u32, String> {
    let v = &vault.inner;
    tasks::recover(v, &v.device_id()).map_err(err)
}

pub fn ai_retry_task(vault: &NotesVault, id: String) -> Result<(), String> {
    tasks::retry(&vault.inner, &id).map_err(err)
}

/// Writes the result back: on success `text` (markdown) replaces the
/// placeholder; on failure `text` is the error.
pub fn ai_finish_task(vault: &NotesVault, id: String, ok: bool, text: String) -> Result<(), String> {
    let r = if ok { Ok(text) } else { Err(text) };
    tasks::finish(&vault.inner, &id, r).map_err(err)
}
