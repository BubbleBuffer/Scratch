//! AI tasks run by the agent (runner value `agent`).

use std::time::Duration;

use anyhow::Result;
use bubblepaw_core::{ClientInfo, RunOptions};
use vault_core::Vault;
use vault_mcp::tasks::{self, PromptTools};

use crate::{permission_policy, Agent, PermissionMode};

/// The runner value of AI tasks run by the desktop agent.
pub const RUNNER: &str = "agent";

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

impl Agent {
    /// Runs one claimed task in its own session and writes the outcome back
    /// with `tasks::finish`: the final text, or the run's error.
    pub fn run_task(&self, vault: &Vault, task_id: &str) -> Result<TaskRun> {
        let task = tasks::get(vault, task_id).map_err(anyhow::Error::msg)?;
        let prompt =
            tasks::agent_prompt(vault, &task, PromptTools::Js).map_err(anyhow::Error::msg)?;
        let session = self.create_session(&session_title(&task.prompt))?;
        let outcome = match self.run_task_and_wait(&session.id, &prompt) {
            Ok(run) if run.status == "completed" => Ok(run.text.unwrap_or_default()),
            Ok(run) => Err(run
                .error
                .filter(|e| !e.trim().is_empty())
                .unwrap_or_else(|| format!("the agent run ended: {}", run.status))),
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
    fn run_task_and_wait(&self, session: &str, prompt: &str) -> Result<bubblepaw_core::Run> {
        let model = Some(self.default_model()).filter(|m| !m.trim().is_empty());
        let system_prompt = self.run_prompt(session)?;
        self.runtime.block_on(async {
            self.harness.start(
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
            self.harness.wait_for_idle(session).await
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
    pub fn run_job(&self, session: &str, prompt: &str, limit: Duration) -> Result<JobOutcome> {
        let model = Some(self.default_model()).filter(|m| !m.trim().is_empty());
        let system_prompt = self.run_prompt(session)?;
        self.runtime.block_on(async {
            self.harness.start(
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
            match tokio::time::timeout(limit, self.harness.wait_for_idle(session)).await {
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
