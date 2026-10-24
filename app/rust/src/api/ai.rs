//! vault_mcp for Dart: the vault's MCP endpoint the desktop app serves and
//! AI tasks (rows of the synced "AI tasks" collection, answered by the
//! in-app agent, `ai_run_task_with_agent`). Only type conversion happens
//! here.

use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use vault_core::Vault;
use vault_mcp::http::{HttpOptions, McpHttp};
use vault_mcp::{endpoint, tasks, McpServer};

use crate::api::vault::NotesVault;
use crate::convert::*;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The default listen address of the app's MCP endpoint.
#[frb(sync)]
pub fn mcp_default_addr() -> String {
    endpoint::DEFAULT_ADDR.to_string()
}

/// The vault's MCP endpoint over HTTP (bearer token, 127.0.0.1 by
/// default). While it runs, `<vault>/.vault/mcp/endpoint.json` names it so
/// `notes-mcp` (stdio, started by Claude desktop or Claude Code) forwards
/// to the app instead of opening the vault itself.
#[frb(opaque)]
pub struct NotesMcp {
    vault: Arc<Vault>,
    root: String,
    http: Mutex<Option<McpHttp>>,
    addr: Mutex<String>,
}

impl NotesMcp {
    #[frb(sync)]
    pub fn new(vault: &NotesVault) -> NotesMcp {
        let root = vault.inner.root();
        NotesMcp { vault: vault.inner.clone(), root, http: Mutex::new(None), addr: Mutex::new(String::new()) }
    }

    /// Starts serving on `addr` (restarts if running) and returns the URL.
    pub fn start(&self, addr: String) -> Result<String, String> {
        self.stop();
        let token = endpoint::token(&self.root).map_err(err)?;
        let http = McpHttp::serve(&addr, Arc::new(McpServer::new(self.vault.clone())), token, HttpOptions::default())
            .map_err(err)?;
        let url = http.url();
        endpoint::write_endpoint(&self.root, &url).map_err(err)?;
        *lock(&self.addr) = addr;
        *lock(&self.http) = Some(http);
        Ok(url)
    }

    /// Stops serving and removes the endpoint file.
    pub fn stop(&self) {
        if let Some(h) = lock(&self.http).take() {
            endpoint::clear_endpoint(&self.root, &h.url());
            h.stop();
        }
    }

    /// The URL while running.
    #[frb(sync)]
    pub fn url(&self) -> Option<String> {
        lock(&self.http).as_ref().map(|h| h.url())
    }

    /// The bearer token (created on first use, kept in `.vault/mcp/token`).
    pub fn token(&self) -> Result<String, String> {
        endpoint::token(&self.root).map_err(err)
    }

    /// Replaces the token (and restarts the endpoint if it runs); clients
    /// configured with the old one stop working.
    pub fn rotate_token(&self) -> Result<String, String> {
        let t = endpoint::rotate_token(&self.root).map_err(err)?;
        let running = lock(&self.http).is_some();
        if running {
            let addr = lock(&self.addr).clone();
            self.start(addr)?;
        }
        Ok(t)
    }
}

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
    let v = &vault.inner;
    tasks::create_on(v, &prompt, &page, after.as_deref(), &runner, &v.device_id()).map(task).map_err(err)
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
