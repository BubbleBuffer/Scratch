//! AI tasks run by the agent (runner value `agent`).

use std::pin::pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use async_trait::async_trait;
use bubblepaw_core::{
    Approval, ClientInfo, Harness, Run, RunOptions, Tool, ToolContext, ToolDefinition,
    ToolRegistry, ToolResult,
};
use serde_json::{json, Value};
use vault_core::{PropValue, Vault};
use vault_mcp::jobs::NODE_HUB;
use vault_mcp::tasks::{self, PromptTools, Task, CANCELLED};

use crate::{permission_policy, Agent, PermissionMode};

/// The runner value of AI tasks run by the desktop agent.
pub const RUNNER: &str = "agent";

/// How long a run may go before it is cancelled and its task or job fails:
/// one hour. Desktop tasks and the hub's jobs (`JobLimits::max_run`) share it.
pub const RUN_LIMIT: Duration = Duration::from_secs(60 * 60);

/// How often a run's wait looks at its task row for a cancel.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Only delegated runs register this capability. The row and session come
/// from the runner, never from model arguments.
struct ReportProgress {
    vault: Arc<Vault>,
    task: String,
    session: String,
}

#[async_trait]
impl Tool for ReportProgress {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "report_progress".into(),
            description: "Report current progress on this delegated task. Keeps only the latest message (trimmed, at most 500 characters). Use while working; the final reply remains the result.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"message": {"type": "string"}},
                "required": ["message"],
                "additionalProperties": false
            }),
            read_only: false,
            approval: Approval::None,
        }
    }

    async fn invoke(&self, context: ToolContext, input: Value) -> Result<ToolResult> {
        if context.session_id != self.session {
            return Ok(ToolResult::error(
                "progress belongs to another task session",
            ));
        }
        let Some(message) = input.get("message").and_then(Value::as_str) else {
            return Ok(ToolResult::error("report_progress needs a message string"));
        };
        let (vault, task, message) = (self.vault.clone(), self.task.clone(), message.to_string());
        let result =
            tokio::task::spawn_blocking(move || tasks::report_progress(&vault, &task, &message))
                .await?;
        Ok(match result {
            Ok(()) => ToolResult::text("Progress reported."),
            Err(error) => ToolResult::error(error.0),
        })
    }
}

impl Agent {
    /// A separate registry over the same store/runtime preserves chat and MCP
    /// surfaces and cancellation, without ever advertising progress to chats.
    fn task_harness(&self, task: &str, session: &str) -> Result<Harness> {
        let progress: Arc<dyn Tool> = Arc::new(ReportProgress {
            vault: self.vault.clone(),
            task: task.into(),
            session: session.into(),
        });
        let register = move |mut registry: ToolRegistry| -> Result<ToolRegistry> {
            registry.register_host(progress.clone())?;
            registry.show_always(["report_progress"]);
            Ok(registry)
        };
        let mut config = self.task_harness_config.clone();
        let tools = register(self.base_tools.clone())?;
        if let Some(resolve) = config.registry_resolver.take() {
            config.registry_resolver = Some(Arc::new(move || {
                register(resolve()).expect("delegated progress tool name is reserved")
            }));
        }
        Ok(Harness::new(
            self.store.clone(),
            self.providers.clone(),
            tools,
            config,
        ))
    }
}

/// What running a task gave.
#[derive(Debug, Clone)]
pub struct TaskRun {
    pub session_id: String,
    pub ok: bool,
    /// The final text when `ok`, else the error written to the task.
    pub text: String,
}

/// The session title of a task: `AI task: ` and the prompt's first 60 characters.
pub fn session_title(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("AI task: {}", flat.chars().take(60).collect::<String>())
}

/// The error a run that passed its limit fails with (the hub's jobs use it too).
pub fn limit_error(limit: Duration) -> String {
    let secs = limit.as_secs();
    let text = if secs >= 3600 && secs % 3600 == 0 {
        format!("{} h", secs / 3600)
    } else if secs >= 60 && secs % 60 == 0 {
        format!("{} min", secs / 60)
    } else {
        format!("{:.1} s", limit.as_secs_f64())
    };
    format!("the run passed the {text} limit and was stopped")
}

/// The task's text when it was cancelled while its run was starting.
const CANCELLED_TEXT: &str = "the task was cancelled";

/// How a task's wait ended.
enum Ended {
    Run(Run),
    Cancelled,
    Limit,
}

/// Whether this device may run `task`: unpinned, or pinned here. Hub jobs
/// never run here.
fn runs_here(vault: &Vault, task: &Task) -> bool {
    task.node.is_empty() || (task.node == vault.device_id() && task.node != NODE_HUB)
}

/// Whether the task's row says it was cancelled (here, or by sync).
fn is_cancelled(vault: &Vault, task_id: &str) -> bool {
    tasks::get(vault, task_id).is_ok_and(|t| t.status == CANCELLED)
}

impl Agent {
    /// Runs one claimed task in its own session and writes the outcome back
    /// with `tasks::finish`: the final text, or the run's error. A run still
    /// going after [`RUN_LIMIT`] is cancelled and the task fails.
    pub fn run_task(&self, vault: &Vault, task_id: &str) -> Result<TaskRun> {
        self.run_task_with_limit(vault, task_id, RUN_LIMIT)
    }

    /// [`Agent::run_task`] with `limit` instead of [`RUN_LIMIT`], so tests
    /// can reach the limit without waiting an hour. The bridge never calls it.
    #[doc(hidden)]
    pub fn run_task_with_limit(
        &self,
        vault: &Vault,
        task_id: &str,
        limit: Duration,
    ) -> Result<TaskRun> {
        let task = tasks::get(vault, task_id).map_err(anyhow::Error::msg)?;
        if !runs_here(vault, &task) {
            bail!("the task is pinned to another device or runs on the hub");
        }
        if is_cancelled(vault, task_id) {
            bail!("the task was cancelled before it started");
        }
        let prompt =
            tasks::agent_prompt(vault, &task, PromptTools::Js).map_err(anyhow::Error::msg)?;
        let session = self.create_session(&session_title(&task.prompt))?;
        vault.set_property(
            task_id.to_string(),
            "session".into(),
            PropValue::Text(session.id.clone()),
        )?;
        let outcome = match self.run_task_and_wait(vault, task_id, &session.id, &prompt, limit) {
            Ok(Ended::Run(run)) if run.status == "completed" => Ok(run.text.unwrap_or_default()),
            Ok(Ended::Run(run)) => Err(run
                .error
                .filter(|e| !e.trim().is_empty())
                .unwrap_or_else(|| format!("the agent run ended: {}", run.status))),
            Ok(Ended::Cancelled) => Err(CANCELLED_TEXT.into()),
            Ok(Ended::Limit) => Err(limit_error(limit)),
            Err(error) => Err(format!("{error:#}")),
        };
        tasks::finish(vault, task_id, outcome.clone()).map_err(anyhow::Error::msg)?;
        let (ok, text) = match outcome {
            Ok(text) => (true, text),
            Err(error) => (false, error),
        };
        Ok(TaskRun {
            session_id: session.id,
            ok,
            text,
        })
    }
}

impl Agent {
    /// Runs a task's prompt and waits: this engine's tools (the runner's
    /// synced tool set), the synced default model, and no approval prompts,
    /// since nobody watches a task run (decision 0009, integration spec
    /// phase 7).
    ///
    /// While it waits it watches the task row: a cancel (here, or arriving
    /// by sync) cancels the engine through [`Agent::cancel_as`], and a run
    /// still going after `limit` is cancelled too. Either way it waits for
    /// the run to stop before it returns.
    fn run_task_and_wait(
        &self,
        vault: &Vault,
        task_id: &str,
        session: &str,
        prompt: &str,
        limit: Duration,
    ) -> Result<Ended> {
        let model = Some(self.default_model()).filter(|m| !m.trim().is_empty());
        let system_prompt = self.run_prompt(session)?;
        let harness = self.task_harness(task_id, session)?;
        self.runtime.block_on(async {
            // Cancelled after the session was saved, before the engine starts.
            if is_cancelled(vault, task_id) {
                return Ok(Ended::Cancelled);
            }
            harness.start(
                session,
                prompt,
                RunOptions {
                    approval: Some(permission_policy(PermissionMode::Bypass)),
                    system_prompt: Some(system_prompt),
                    model,
                    client: Some(crate::desktop_client()),
                    ..RunOptions::default()
                },
            )?;
            let mut wait = pin!(harness.wait_for_idle(session));
            let started = Instant::now();
            loop {
                let left = limit.saturating_sub(started.elapsed());
                match tokio::time::timeout(left.min(WATCH_EVERY), wait.as_mut()).await {
                    Ok(run) => return Ok(Ended::Run(run?)),
                    Err(_) => {
                        let cancelled = is_cancelled(vault, task_id);
                        if cancelled || started.elapsed() >= limit {
                            self.cancel_as(session, crate::desktop_client());
                            let _ = wait.as_mut().await;
                            return Ok(if cancelled {
                                Ended::Cancelled
                            } else {
                                Ended::Limit
                            });
                        }
                    }
                }
            }
        })
    }
}

/// How a job's run ended ([`Agent::run_job`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// The final text.
    Completed(String),
    /// The run's error.
    Failed(String),
    /// Cancelled (by the jobs API, or at shutdown).
    Cancelled,
    /// Cancelled at the time limit.
    TimedOut,
}

/// The client a job's events come from.
pub fn job_client() -> ClientInfo {
    ClientInfo {
        id: "jobs".into(),
        name: Some("Jobs API".into()),
    }
}

impl Agent {
    /// Runs `prompt` in `session` as a job and waits for it: this engine's
    /// tools only, no approval prompts (every call it has is allowed), the
    /// runner's default model. A run still going after `limit` is cancelled.
    pub fn run_job(
        &self,
        task_id: &str,
        session: &str,
        prompt: &str,
        limit: Duration,
    ) -> Result<JobOutcome> {
        let model = Some(self.default_model()).filter(|m| !m.trim().is_empty());
        let system_prompt = self.run_prompt(session)?;
        let harness = self.task_harness(task_id, session)?;
        self.runtime.block_on(async {
            harness.start(
                session,
                prompt,
                RunOptions {
                    approval: Some(permission_policy(PermissionMode::Bypass)),
                    system_prompt: Some(system_prompt),
                    model,
                    client: Some(job_client()),
                    ..RunOptions::default()
                },
            )?;
            match tokio::time::timeout(limit, harness.wait_for_idle(session)).await {
                Ok(run) => {
                    let run = run?;
                    Ok(match run.status.as_str() {
                        "completed" => JobOutcome::Completed(run.text.unwrap_or_default()),
                        "cancelled" => JobOutcome::Cancelled,
                        status => JobOutcome::Failed(
                            run.error
                                .filter(|e| !e.trim().is_empty())
                                .unwrap_or_else(|| format!("the agent run ended: {status}")),
                        ),
                    })
                }
                Err(_) => {
                    self.harness.cancel_from(session, Some(job_client()));
                    let _ = self.harness.wait_for_idle(session).await;
                    Ok(JobOutcome::TimedOut)
                }
            }
        })
    }
}
