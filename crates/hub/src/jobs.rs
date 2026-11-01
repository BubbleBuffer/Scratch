//! The jobs API: outside agents start agent runs on the hub through the
//! `jobs` registry tool (`/mcp` with an agent token or OAuth, `/mcp/agent`,
//! and `notes jobs`). Reference: `wiki/sync/hub.md` (Jobs).
//!
//! - A job is a row of the "AI tasks" database ([`vault_mcp::jobs`]) with
//!   node `hub`. Its owner is the principal that created it
//!   (`agent:<token name>`, `oauth:<user>` or `device:<device id>`); only
//!   the owner sees, waits for or cancels it, except that any signed-in
//!   device acts on any device's job (a task assigned on the desktop can be
//!   unassigned on the phone). Devices see every job as a row.
//! - Devices create jobs with their device token ("Ask AI" and tasks
//!   assigned to the agent when the default runner is the hub). Such a job
//!   may be about a page (`page`, `after`): it gets a placeholder there and
//!   its end is written into the page.
//! - The run's tool set is the caller's set narrowed by the hub runner's set
//!   (`tool_sets.hub`) and by the optional `tools` argument. It runs on a job
//!   engine that registers exactly that set's tools, with no approval prompts.
//!   Read callers can't create jobs.
//! - Limits ([`JobLimits`]): one running job per owner, 4 queued per hub,
//!   `wait` at most 60 s, runs stopped and failed after 1 h, ended jobs keep their outcome and lose their
//!   transcript after 30 days. A restart marks running jobs interrupted; they are not queued
//!   again.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Notify;
use vault_agent::tasks::{job_client, limit_error, JobOutcome, RUN_LIMIT};
use vault_agent::HostKind;
use vault_core::Vault;
use vault_mcp::jobs::{self, End, Job, NODE_HUB};
use vault_mcp::tasks::QUEUED;
use vault_tools::ToolSet;

use crate::agent::Shared;
use crate::AppState;

/// How long a cancel waits for the runner's acknowledgement before it
/// answers with the job still running (cancel requested).
const CANCEL_ACK_WAIT: Duration = Duration::from_secs(3);

/// The jobs API's limits. Tests shorten them.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLimits {
    /// Queued jobs the hub holds; `create` is refused beyond it.
    pub max_queued: usize,
    /// The longest `wait`.
    pub max_wait: Duration,
    /// A run still going after this is cancelled and the job fails.
    pub max_run: Duration,
    /// An ended job's session (transcript and logs) is deleted after this long;
    /// the job row stays.
    pub keep: Duration,
    /// How often the worker looks for work and old jobs without a wake-up.
    pub tick: Duration,
}

impl Default for JobLimits {
    fn default() -> Self {
        JobLimits {
            max_queued: 4,
            max_wait: Duration::from_secs(60),
            max_run: RUN_LIMIT,
            keep: Duration::from_secs(30 * 24 * 60 * 60),
            tick: Duration::from_secs(30),
        }
    }
}

/// A job the worker has taken.
struct Running {
    owner: String,
    engine: Option<Shared>,
    session: Option<String>,
    /// Cancelled through the API (the row already says so).
    cancelled: bool,
}

/// The hub's jobs: the API's state and the worker's wake-up.
pub(crate) struct Jobs {
    vault: Arc<Vault>,
    limits: JobLimits,
    agent_enabled: bool,
    running: Mutex<HashMap<String, Running>>,
    /// Held while counting the queue and creating a row.
    create: Mutex<()>,
    wake: Notify,
    stopping: AtomicBool,
}

fn s<'a>(a: &'a Value, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}

fn rank(set: ToolSet) -> usize {
    ToolSet::ALL.iter().position(|s| *s == set).unwrap_or(0)
}

/// The narrower of two sets.
fn narrower(a: ToolSet, b: ToolSet) -> ToolSet {
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
}

/// The principal prefix of signed-in devices.
pub(crate) const DEVICE_PREFIX: &str = "device:";

/// Whether `actor` may see, wait for and cancel a job owned by `owner`.
fn may_act(actor: &str, owner: &str) -> bool {
    owner == actor || (actor.starts_with(DEVICE_PREFIX) && owner.starts_with(DEVICE_PREFIX))
}

fn job_json(job: &Job) -> Result<String, String> {
    serde_json::to_string_pretty(job).map_err(|e| e.to_string())
}

impl Jobs {
    pub(crate) fn new(vault: Arc<Vault>, limits: JobLimits, agent_enabled: bool) -> Self {
        Jobs {
            vault,
            limits,
            agent_enabled,
            running: Mutex::new(HashMap::new()),
            create: Mutex::new(()),
            wake: Notify::new(),
            stopping: AtomicBool::new(false),
        }
    }

    /// Stops the worker; running jobs are left as they are (the next start
    /// marks them interrupted).
    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// The `jobs` tool for `actor` with tool set `set`. Blocking: `wait`
    /// sleeps on the calling thread.
    pub(crate) fn call(&self, actor: &str, set: ToolSet, a: &Value) -> Result<String, String> {
        match s(a, "action").unwrap_or_default() {
            "create" => self.create(actor, set, a),
            "get" => job_json(&self.owned(actor, a)?),
            "wait" => self.wait(actor, a),
            "cancel" => self.cancel(actor, a),
            "list" => self.list(actor, a),
            "" => Err("jobs needs an action: create, get, wait, cancel or list".into()),
            other => Err(format!(
                "unknown jobs action: {other} (use create, get, wait, cancel or list)"
            )),
        }
    }

    fn create(&self, actor: &str, set: ToolSet, a: &Value) -> Result<String, String> {
        if set == ToolSet::Read {
            return Err(
                "read-only callers can't create jobs: a job needs the Write or Everything tool set"
                    .into(),
            );
        }
        if !self.agent_enabled {
            return Err("the agent is off on this hub ([agent] enabled = false)".into());
        }
        let prompt = s(a, "prompt").unwrap_or_default().trim();
        if prompt.is_empty() {
            return Err("create needs a prompt".into());
        }
        let wanted = match s(a, "tools").filter(|t| !t.trim().is_empty()) {
            Some(t) => {
                let wanted: ToolSet = t.trim().parse()?;
                if rank(wanted) > rank(set) {
                    return Err(format!(
                        "tools {wanted} is wider than your tool set ({set}); a job can't widen it"
                    ));
                }
                wanted
            }
            None => set,
        };
        let page = s(a, "page").map(str::trim).filter(|p| !p.is_empty());
        let after = s(a, "after").map(str::trim).filter(|b| !b.is_empty());
        if after.is_some() && page.is_none() {
            return Err("after needs a page".into());
        }
        if let Some(page) = page {
            match self.vault.load_page(page.to_string()) {
                Ok(p) if !p.in_trash => {}
                _ => return Err(format!("no such page: {page}")),
            }
        }
        let runner = vault_agent::runner_tool_set(&self.vault, HostKind::Hub);
        let tools = narrower(wanted, runner);
        let job = {
            let _create = self.create.lock().unwrap();
            let queued = jobs::all(&self.vault)
                .map_err(|e| e.0)?
                .iter()
                .filter(|j| j.node == NODE_HUB && j.status == QUEUED)
                .count();
            if queued >= self.limits.max_queued {
                return Err(format!(
                    "the hub's job queue is full ({queued} queued, at most {}); try again later",
                    self.limits.max_queued
                ));
            }
            match page {
                Some(page) => jobs::create_for_page(
                    &self.vault,
                    prompt,
                    actor,
                    tools.as_str(),
                    NODE_HUB,
                    &page.to_string(),
                    after,
                ),
                None => jobs::create(&self.vault, prompt, actor, tools.as_str(), NODE_HUB),
            }
            .map_err(|e| e.0)?
        };
        self.wake.notify_one();
        job_json(&job)
    }

    /// The caller's job named by `job`. Another principal's job is "no such
    /// job", like a missing one.
    fn owned(&self, actor: &str, a: &Value) -> Result<Job, String> {
        let id = s(a, "job").unwrap_or_default().trim();
        if id.is_empty() {
            return Err("give the job id (job)".into());
        }
        match jobs::get(&self.vault, id).map_err(|e| e.0)? {
            Some(j) if j.node == NODE_HUB && may_act(actor, &j.owner) => Ok(j),
            _ => Err(format!("no such job: {id}")),
        }
    }

    fn wait(&self, actor: &str, a: &Value) -> Result<String, String> {
        let asked = a
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(30));
        let timeout = asked.min(self.limits.max_wait);
        let start = Instant::now();
        loop {
            let job = self.owned(actor, a)?;
            let left = timeout.saturating_sub(start.elapsed());
            if job.ended() || left.is_zero() || self.stopping.load(Ordering::SeqCst) {
                return job_json(&job);
            }
            std::thread::sleep(left.min(Duration::from_millis(100)));
        }
    }

    fn cancel(&self, actor: &str, a: &Value) -> Result<String, String> {
        let job = self.owned(actor, a)?;
        if job.ended() {
            return Err(format!("the job already ended: {}", job.status));
        }
        let target = {
            let mut running = self.running.lock().unwrap();
            let target = running.get_mut(&job.id).map(|r| {
                r.cancelled = true;
                (r.engine.clone(), r.session.clone())
            });
            let queued = jobs::get(&self.vault, &job.id)
                .map_err(|e| e.0)?
                .is_some_and(|j| j.status == QUEUED);
            if queued {
                // No run to stop: nothing to wait for.
                jobs::finish(&self.vault, &job.id, End::Cancelled).map_err(|e| e.0)?;
            } else {
                // A running job stays running until its run stops; `end`
                // acknowledges. Never report stopped work that still runs.
                jobs::request_cancel(&self.vault, &job.id).map_err(|e| e.0)?;
            }
            target
        };
        if let Some((Some(engine), Some(session))) = target {
            engine.cancel_as(&session, job_client());
        }
        self.wake.notify_one();
        // A run usually stops within moments: give the acknowledgement a
        // short time before answering with the job's state.
        let start = Instant::now();
        loop {
            let now = self.owned(actor, a)?;
            if now.ended() || start.elapsed() >= CANCEL_ACK_WAIT {
                return job_json(&now);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn list(&self, actor: &str, a: &Value) -> Result<String, String> {
        let limit = a
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;
        let mine: Vec<Job> = jobs::all(&self.vault)
            .map_err(|e| e.0)?
            .into_iter()
            .filter(|j| j.node == NODE_HUB && may_act(actor, &j.owner))
            .take(limit)
            .collect();
        serde_json::to_string_pretty(&json!({ "jobs": mine })).map_err(|e| e.to_string())
    }

    /// Takes the queued jobs that may start now (oldest first, one per
    /// owner with no running job) and marks them running.
    fn take_ready(&self) -> Result<Vec<Job>, String> {
        let mut queued: Vec<Job> = jobs::all(&self.vault)
            .map_err(|e| e.0)?
            .into_iter()
            .filter(|j| j.node == NODE_HUB && j.status == QUEUED)
            .collect();
        queued.reverse();
        let mut running = self.running.lock().unwrap();
        let mut busy: HashSet<String> = running.values().map(|r| r.owner.clone()).collect();
        let mut taken = Vec::new();
        for job in queued {
            if busy.contains(&job.owner) {
                continue;
            }
            // The row may have changed since the listing (a cancel).
            match jobs::get(&self.vault, &job.id).map_err(|e| e.0)? {
                Some(j) if j.status == QUEUED => {}
                _ => continue,
            }
            jobs::mark_running(&self.vault, &job.id, "").map_err(|e| e.0)?;
            busy.insert(job.owner.clone());
            running.insert(
                job.id.clone(),
                Running {
                    owner: job.owner.clone(),
                    engine: None,
                    session: None,
                    cancelled: false,
                },
            );
            taken.push(job);
        }
        Ok(taken)
    }

    /// Ends `id` unless the hub is stopping, and forgets it. A job whose
    /// cancel was asked for ends as cancelled: this is the runner's
    /// acknowledgement.
    fn end(&self, id: &str, end: End) {
        let mut running = self.running.lock().unwrap();
        let cancelled = running.remove(id).is_some_and(|r| r.cancelled);
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let end = if cancelled { End::Cancelled } else { end };
        if let Err(e) = jobs::finish(&self.vault, id, end) {
            tracing::warn!("job {id}: could not save its end: {}", e.0);
        }
    }

    /// Runs one taken job on `engine` to its end (blocking).
    fn run_one(&self, job: &Job, engine: Shared) {
        let session = match engine.create_session(&format!(
            "Job: {}",
            job.prompt
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(60)
                .collect::<String>()
        )) {
            Ok(s) => s.id,
            Err(e) => return self.end(&job.id, End::Failed(format!("{e:#}"))),
        };
        {
            let mut running = self.running.lock().unwrap();
            let Some(r) = running.get_mut(&job.id) else {
                return;
            };
            if r.cancelled {
                running.remove(&job.id);
                drop(running);
                // Cancelled before its run started: acknowledge now.
                if let Err(e) = jobs::finish(&self.vault, &job.id, End::Cancelled) {
                    tracing::warn!("job {}: could not save its cancel: {}", job.id, e.0);
                }
                return;
            }
            r.engine = Some(engine.clone());
            r.session = Some(session.clone());
            let _ = self.vault.set_property(
                job.id.clone(),
                "session".into(),
                vault_core::PropValue::Text(session.clone()),
            );
        }
        // A job about a page (from a device) gets the page and answers into
        // it; other jobs come from outside agents.
        let prompt = match jobs::page_prompt(&self.vault, &job.id) {
            Ok(Some(prompt)) => prompt,
            _ => format!(
                "A job from an outside agent. Do the task below with your tools. Your final reply is the job's result, returned to that agent; make it complete on its own.\n\n{}",
                job.prompt
            ),
        };
        let end = match engine.run_job(&job.id, &session, &prompt, self.limits.max_run) {
            Ok(JobOutcome::Completed(text)) => End::Completed(text),
            Ok(JobOutcome::Failed(error)) => End::Failed(error),
            Ok(JobOutcome::Cancelled) => End::Cancelled,
            Ok(JobOutcome::TimedOut) => End::Failed(limit_error(self.limits.max_run)),
            Err(e) => End::Failed(format!("{e:#}")),
        };
        self.end(&job.id, end);
    }
}

/// The worker: marks interrupted jobs at start, then starts ready jobs and
/// trashes old ones whenever woken (a create, a cancel, a job's end) or
/// every tick.
pub(crate) async fn run_worker(state: Arc<AppState>) {
    let st = state.clone();
    let recovered = tokio::task::spawn_blocking(move || {
        jobs::recover(st.hub.vault(), NODE_HUB).map_err(|e| e.0)
    })
    .await;
    match recovered {
        Ok(Ok(n)) if n > 0 => tracing::info!("{n} running jobs marked interrupted"),
        Ok(Err(e)) => tracing::warn!("jobs: recovery failed: {e}"),
        _ => {}
    }
    loop {
        if state.jobs.stopping.load(Ordering::SeqCst) {
            return;
        }
        let st = state.clone();
        let keep = st.jobs.limits.keep.as_millis() as i64;
        let expired = {
            let st = st.clone();
            tokio::task::spawn_blocking(move || {
                jobs::expired_sessions(st.hub.vault(), NODE_HUB, keep, jobs::now_ms())
            })
            .await
        };
        // Transcripts and logs expire; the job row, its outcome and its
        // attempt history stay.
        if let Ok(Ok(list)) = expired {
            for job in list {
                let set: ToolSet = job.tools.parse().unwrap_or(ToolSet::Read);
                let Ok(engine) = state.agent.job_engine(set).await else {
                    continue;
                };
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    // A session that is already gone counts as expired.
                    let _ = engine.delete_session(&job.session);
                    if let Err(e) = jobs::session_expired(st.hub.vault(), &job.id) {
                        tracing::warn!("jobs: cleanup failed: {}", e.0);
                    }
                })
                .await;
            }
        }
        let taken = tokio::task::spawn_blocking(move || st.jobs.take_ready()).await;
        match taken {
            Ok(Ok(taken)) => {
                for job in taken {
                    tokio::spawn(start(state.clone(), job));
                }
            }
            Ok(Err(e)) => tracing::warn!("jobs: {e}"),
            Err(_) => {}
        }
        tokio::select! {
            _ = state.jobs.wake.notified() => {}
            _ = tokio::time::sleep(state.jobs.limits.tick) => {}
        }
    }
}

async fn start(state: Arc<AppState>, job: Job) {
    let set: ToolSet = job.tools.parse().unwrap_or(ToolSet::Read);
    // The runner's set may have narrowed since the job was created.
    let st = state.clone();
    let runner = tokio::task::spawn_blocking(move || {
        vault_agent::runner_tool_set(st.hub.vault(), HostKind::Hub)
    })
    .await
    .unwrap_or(ToolSet::Read);
    let set = narrower(set, runner);
    match state.agent.job_engine(set).await {
        Ok(engine) => {
            let st = state.clone();
            let id = job.id.clone();
            let ran = tokio::task::spawn_blocking(move || st.jobs.run_one(&job, engine)).await;
            if ran.is_err() {
                state
                    .jobs
                    .end(&id, End::Failed("the job's run panicked".into()));
            }
        }
        Err(e) => state.jobs.end(&job.id, End::Failed(e)),
    }
    state.jobs.wake.notify_one();
}
