//! vault_mcp for Dart: the vault's MCP endpoint the desktop app serves and
//! AI tasks (rows of the synced "AI tasks" collection, answered by the
//! in-app agent, `ai_run_task_with_agent`). Only type conversion happens
//! here.

use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use vault_core::Vault;
use vault_mcp::http::{HttpOptions, McpHttp};
use vault_mcp::{endpoint, runners, tasks, McpServer};

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
        NotesMcp {
            vault: vault.inner.clone(),
            root,
            http: Mutex::new(None),
            addr: Mutex::new(String::new()),
        }
    }

    /// Starts serving on `addr` (restarts if running) and returns the URL.
    pub fn start(&self, addr: String) -> Result<String, String> {
        self.stop();
        let token = endpoint::token(&self.root).map_err(err)?;
        let http = McpHttp::serve(
            &addr,
            Arc::new(McpServer::new(self.vault.clone())),
            token,
            HttpOptions::default(),
        )
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
    /// The latest progress report, empty until one is recorded.
    pub progress: String,
    /// Unix epoch milliseconds as stored in the row, empty until reported.
    pub progress_at: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    /// Where it runs: "" (any desktop), a device id (only that device) or
    /// "hub" (a hub job).
    pub node: String,
    /// An agent project id on the pinned PC; empty for a notes-only task.
    pub project: String,
    /// Saved by the runner before the code task starts.
    pub worktree_branch: String,
    pub worktree_path: String,
    /// A cancel was requested and the runner has not acknowledged it yet.
    pub cancel_requested: bool,
    /// The note changed after the running worker got its brief.
    pub stale: bool,
    /// When the runner of a running task was last heard from (ms), set only
    /// when it has been silent for over ten minutes.
    pub stalled_since_ms: Option<i64>,
    /// The agent session of the run, empty when it has none or it expired.
    pub session: String,
    /// The worker's open question to the user, empty when it has none. A
    /// running task with a question waits for `ai_reply_task`.
    pub question: String,
}

fn task(t: tasks::Task) -> AiTask {
    let stalled_since_ms = tasks::stalled_since(&t, vault_mcp::jobs::now_ms());
    AiTask {
        id: t.id,
        prompt: t.prompt,
        page: t.page,
        block: t.block,
        runner: t.runner,
        status: t.status,
        device: t.device,
        error: t.error,
        progress: t.progress,
        progress_at: t.progress_at,
        created_ms: t.created_ms,
        updated_ms: t.updated_ms,
        node: t.node,
        project: t.project,
        worktree_branch: t.worktree_branch,
        worktree_path: t.worktree_path,
        cancel_requested: t.cancel_requested_ms.is_some(),
        stale: false,
        stalled_since_ms,
        session: t.session,
        question: t.question,
    }
}

/// [task] with `stale` set from the task's attempt and the note.
fn task_in(v: &vault_core::Vault, t: tasks::Task) -> AiTask {
    let stale = tasks::is_stale(v, &t);
    AiTask { stale, ..task(t) }
}

/// The hub or a PC that uses this vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRunnerKind {
    Hub,
    Pc,
}

/// A PC's project choice, without its private local repository path.
#[derive(Debug, Clone)]
pub struct AiRunnerProject {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct AiRunner {
    /// `hub` or the device id in `.vault/device`, never a node-record page id.
    pub id: String,
    pub name: String,
    pub kind: AiRunnerKind,
    pub online: bool,
    /// Unix epoch milliseconds; absent until a timestamp is known.
    pub last_seen_ms: Option<i64>,
    pub is_this_device: bool,
    pub projects: Vec<AiRunnerProject>,
}

#[derive(Debug, Clone)]
pub struct AiRunnerList {
    pub runners: Vec<AiRunner>,
    pub default_runner_id: String,
}

/// Lists the hub first, then the vault's PCs. Pass whether the app has a
/// usable hub link, as AgentStore.usableHosts does; online is independently
/// derived from the synced last-seen timestamp and the five-minute window.
pub fn ai_list_runners(vault: &NotesVault, hub_usable: bool) -> Result<AiRunnerList, String> {
    let local = cfg!(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "macos"
    ));
    let list = runners::list(&vault.inner, hub_usable, local).map_err(err)?;
    Ok(AiRunnerList {
        default_runner_id: list.default_runner_id,
        runners: list
            .runners
            .into_iter()
            .map(|r| AiRunner {
                id: r.id,
                name: r.name,
                kind: match r.kind {
                    runners::RunnerKind::Hub => AiRunnerKind::Hub,
                    runners::RunnerKind::Pc => AiRunnerKind::Pc,
                },
                online: r.online,
                last_seen_ms: r.last_seen_ms,
                is_this_device: r.is_this_device,
                projects: r
                    .projects
                    .into_iter()
                    .map(|p| AiRunnerProject {
                        id: p.id,
                        name: p.name,
                    })
                    .collect(),
            })
            .collect(),
    })
}

/// Queues a real AI task pinned to the selected runner, including offline
/// runners. An unknown id or a block outside the page is rejected.
pub fn ai_assign_task(
    vault: &NotesVault,
    prompt: String,
    page: String,
    block: Option<String>,
    runner_id: String,
    project_id: Option<String>,
) -> Result<AiTask, String> {
    runners::assign(
        &vault.inner,
        &prompt,
        &page,
        block.as_deref(),
        &runner_id,
        project_id.as_deref(),
    )
    .map(task)
    .map_err(err)
}

/// Renames a device from any copy of the vault. Only its trimmed name changes.
pub fn ai_rename_device(vault: &NotesVault, device_id: String, name: String) -> Result<(), String> {
    vault.inner.rename_device(device_id, name).map_err(err)
}

/// Queues a task about `page`; its placeholder goes after block `after`
/// (an empty paragraph there becomes the placeholder) or at the end.
pub fn ai_create_task(
    vault: &NotesVault,
    prompt: String,
    page: String,
    after: Option<String>,
    runner: String,
    project_id: Option<String>,
) -> Result<AiTask, String> {
    let v = &vault.inner;
    tasks::create_for_project(
        v,
        &prompt,
        &page,
        after.as_deref(),
        &runner,
        &v.device_id(),
        project_id.as_deref(),
    )
    .map(task)
    .map_err(err)
}

/// [`ai_create_task`] pinned to this device: only this device claims it
/// ("Ask AI" when the default runner is this computer).
pub fn ai_create_task_here(
    vault: &NotesVault,
    prompt: String,
    page: String,
    after: Option<String>,
    project_id: Option<String>,
) -> Result<AiTask, String> {
    let v = &vault.inner;
    tasks::create_for_project(
        v,
        &prompt,
        &page,
        after.as_deref(),
        "",
        &v.device_id(),
        project_id.as_deref(),
    )
    .map(task)
    .map_err(err)
}

/// Cancels a desktop task that hasn't ended: it is never claimed, and a
/// running one's result is dropped. Returns whether it was still going.
/// Hub jobs are cancelled on the hub.
pub fn ai_cancel_task(vault: &NotesVault, id: String) -> Result<bool, String> {
    tasks::cancel(&vault.inner, &id).map_err(err)
}

/// The user accepts a reviewed result: the task goes from review to done.
/// Returns whether it was in review.
pub fn ai_accept_task(vault: &NotesVault, id: String) -> Result<bool, String> {
    tasks::accept(&vault.inner, &id).map_err(err)
}

/// Answers the worker's open question. The worker gets the reply and the task
/// is plain running again. Returns false when there was no open question.
pub fn ai_reply_task(vault: &NotesVault, id: String, text: String) -> Result<bool, String> {
    tasks::reply(&vault.inner, &id, &text).map_err(err)
}

/// Fails a task whose runner has been silent for over ten minutes. Returns
/// false, and changes nothing, when the task is not stalled.
pub fn ai_mark_task_failed(vault: &NotesVault, id: String) -> Result<bool, String> {
    tasks::mark_failed(&vault.inner, &id).map_err(err)
}

/// Asks the runner to send the running worker the note's current version.
/// Returns whether the task was running and took the request.
pub fn ai_send_update(vault: &NotesVault, id: String) -> Result<bool, String> {
    tasks::request_update(&vault.inner, &id).map_err(err)
}

/// Adds a to-do under the running task; the runner puts it on the worker's
/// list and sends it as a steering message.
pub fn ai_add_step(vault: &NotesVault, id: String, text: String) -> Result<(), String> {
    tasks::add_step(&vault.inner, &id, &text).map_err(err)
}

/// The newest tasks first.
pub fn ai_list_tasks(vault: &NotesVault, limit: u32) -> Result<Vec<AiTask>, String> {
    Ok(tasks::list(&vault.inner, limit)
        .map_err(err)?
        .into_iter()
        .map(|t| task_in(&vault.inner, t))
        .collect())
}

pub fn ai_get_task(vault: &NotesVault, id: String) -> Result<AiTask, String> {
    tasks::get(&vault.inner, &id)
        .map(|t| task_in(&vault.inner, t))
        .map_err(err)
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
pub fn ai_finish_task(
    vault: &NotesVault,
    id: String,
    ok: bool,
    text: String,
) -> Result<(), String> {
    let r = if ok { Ok(text) } else { Err(text) };
    tasks::finish(&vault.inner, &id, r).map_err(err)
}

#[cfg(all(
    test,
    any(target_os = "windows", target_os = "linux", target_os = "macos")
))]
mod runner_tests {
    use super::*;

    #[test]
    fn stored_progress_reaches_task_get_and_list() {
        struct Temp(std::path::PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir =
            Temp(std::env::temp_dir().join(format!("bridge_progress_{}", vault_core::new_id())));
        let vault = NotesVault::open(dir.0.to_string_lossy().into_owned(), None).unwrap();
        let page = vault.inner.create_page(None, "Plan".into()).unwrap();
        let queued = ai_create_task_here(&vault, "Summarize".into(), page, None, None).unwrap();
        assert_eq!(queued.progress, "");
        assert_eq!(queued.progress_at, "");
        let unset = ai_get_task(&vault, queued.id.clone()).unwrap();
        assert_eq!(unset.progress, "");
        assert_eq!(unset.progress_at, "");
        let running = ai_claim_next(&vault).unwrap().unwrap();
        assert_eq!(running.id, queued.id);
        tasks::report_progress(&vault.inner, &running.id, "  Reading the plan  ").unwrap();
        let stored = vault.inner.load_page(running.id.clone()).unwrap();
        let stored_at = stored
            .properties
            .iter()
            .find_map(|p| match (&*p.key, &p.value) {
                ("progress_at", vault_core::PropValue::Text(value)) => Some(value.clone()),
                _ => None,
            })
            .unwrap();
        assert!(stored_at.parse::<i64>().unwrap() > 0);
        let fetched = ai_get_task(&vault, running.id.clone()).unwrap();
        assert_eq!(fetched.progress, "Reading the plan");
        assert_eq!(fetched.progress_at, stored_at);
        let listed = ai_list_tasks(&vault, 10)
            .unwrap()
            .into_iter()
            .find(|t| t.id == running.id)
            .unwrap();
        assert_eq!(listed.progress, fetched.progress);
        assert_eq!(listed.progress_at, fetched.progress_at);
    }

    #[test]
    fn app_open_registers_a_runner_and_picker_calls_reach_real_storage() {
        struct Temp(std::path::PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir =
            Temp(std::env::temp_dir().join(format!("bridge_runners_{}", vault_core::new_id())));
        let root = dir.0.to_string_lossy().into_owned();
        let vault = NotesVault::open(root.clone(), None).unwrap();
        let id = vault.device_id();
        let list = ai_list_runners(&vault, false).unwrap();
        assert_eq!(list.default_runner_id, id);
        assert_eq!(list.runners.len(), 2);
        assert_eq!(list.runners[0].id, "hub");
        assert_eq!(list.runners[1].id, id);
        assert_eq!(list.runners[1].kind, AiRunnerKind::Pc);
        assert!(list.runners[1].is_this_device);
        assert!(list.runners[1].last_seen_ms.is_some());
        assert!(!list.runners[1].name.trim().is_empty());
        ai_rename_device(&vault, id.clone(), "  Studio PC  ".into()).unwrap();
        vault
            .inner
            .publish_device_projects(vec![vault_core::DeviceProject {
                id: "project-1".into(),
                name: "Project one".into(),
            }])
            .unwrap();
        let list = ai_list_runners(&vault, false).unwrap();
        assert!(list.runners[0].projects.is_empty());
        assert_eq!(list.runners[1].projects[0].id, "project-1");
        assert_eq!(list.runners[1].projects[0].name, "Project one");
        let page = vault.inner.create_page(None, "Plan".into()).unwrap();
        let before = vault.inner.load_page(page.clone()).unwrap().blocks;
        assert!(ai_assign_task(
            &vault,
            "Code".into(),
            page.clone(),
            None,
            "hub".into(),
            Some("project-1".into())
        )
        .unwrap_err()
        .contains("hub"));
        assert!(ai_assign_task(
            &vault,
            "Code".into(),
            page.clone(),
            None,
            id.clone(),
            Some("missing".into())
        )
        .unwrap_err()
        .contains("unknown project"));
        assert_eq!(vault.inner.load_page(page.clone()).unwrap().blocks, before);
        let task = ai_assign_task(
            &vault,
            "Summarize".into(),
            page,
            None,
            id.clone(),
            Some("project-1".into()),
        )
        .unwrap();
        assert_eq!(task.node, id);
        assert_eq!(task.project, "project-1");
        assert!(task.worktree_branch.is_empty() && task.worktree_path.is_empty());
        assert_eq!(ai_get_task(&vault, task.id).unwrap().status, tasks::QUEUED);

        // The existing block argument carries the assignment source. Keep
        // placement and source identity together without a second bridge field.
        let page = vault.inner.create_page(None, "Todo plan".into()).unwrap();
        let source = vault_core::new_id();
        vault
            .inner
            .apply(
                page.clone(),
                vec![vault_core::BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: vault_core::NewBlock {
                        id: source.clone(),
                        kind: vault_core::BlockKind::Todo,
                        text: "Prepare the plan".into(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        for runner in [id.clone(), "hub".into()] {
            let assigned = ai_assign_task(
                &vault,
                "Prepare the plan".into(),
                page.clone(),
                Some(source.clone()),
                runner,
                None,
            )
            .unwrap();
            let stored = tasks::get(&vault.inner, &assigned.id).unwrap();
            assert_eq!(stored.source_block.as_deref(), Some(source.as_str()));
            assert_ne!(stored.block.as_deref(), Some(source.as_str()));
        }
        vault.inner.flush().unwrap();
        drop(vault);
        let reopened = NotesVault::open(root, None).unwrap();
        let list = ai_list_runners(&reopened, false).unwrap();
        assert_eq!(list.runners[1].id, id);
        assert_eq!(list.runners[1].name, "Studio PC");
    }
}
