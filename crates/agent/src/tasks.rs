//! AI tasks run by the agent (runner value `agent`).

mod subtasks;

use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use async_trait::async_trait;
use bubblepaw_core::{
    Approval, ApprovalMode, ClientInfo, Harness, Run, RunOptions, Tool, ToolContext,
    ToolDefinition, ToolRegistry, ToolResult,
};
use serde_json::{json, Value};
use vault_core::{PropValue, Vault};
use vault_mcp::jobs::NODE_HUB;
use vault_mcp::tasks::{self, PromptTools, Task, CANCELLED};

use crate::{permission_policy, Agent, PermissionMode};
use subtasks::TodoMirror;

/// The runner value of AI tasks run by the desktop agent.
pub const RUNNER: &str = "agent";

/// How long a run may go before it is cancelled and its task or job fails:
/// one hour. Desktop tasks and the hub's jobs (`JobLimits::max_run`) share it.
pub const RUN_LIMIT: Duration = Duration::from_secs(60 * 60);

/// How often a run's wait looks at its task row for a cancel.
const WATCH_EVERY: Duration = Duration::from_millis(500);

const DELEGATED_PROMPT: &str = " Use report_progress to keep the user informed while working. Plan multi-step work with todo and keep the list current; it appears as subtasks in the note, mirrored by Scratch, so do not write those subtasks yourself.";

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
    /// Prepare before admitting the provider: failures are written to the task
    /// by the same finish path as provider errors. A retry has a new session.
    fn prepare_code_task(&self, vault: &Vault, task_id: &str, session: &str) -> Result<()> {
        let task = tasks::get(vault, task_id).map_err(anyhow::Error::msg)?;
        if task.project.is_empty() {
            return Ok(());
        }
        ensure!(
            self.host_info.kind == "desktop",
            "code tasks require a PC runner"
        );
        ensure!(
            task.node == vault.device_id(),
            "code tasks require a pinned PC runner"
        );
        ensure!(
            self.desktop.is_some(),
            "desktop tools are disabled on this PC; enable desktop tools to run a code task"
        );
        let projects = self.project_store()?;
        let project = projects
            .list()
            .into_iter()
            .find(|p| p.id == task.project)
            .with_context(|| format!("unknown project {:?} on this PC", task.project))?;
        projects.switch(session, &project.id)?;
        let info = bubblepaw_runner::worktrees::create(
            &self.store,
            std::path::Path::new(&project.path),
            session,
        )
        .with_context(|| format!("cannot start code task for project {:?}", project.name))?;
        tasks::ensure_collection(vault).map_err(anyhow::Error::msg)?;
        vault.set_property(
            task_id.into(),
            "worktree_branch".into(),
            PropValue::Text(info.branch),
        )?;
        vault.set_property(
            task_id.into(),
            "worktree_path".into(),
            PropValue::Text(info.path.to_string_lossy().into_owned()),
        )?;
        Ok(())
    }

    /// A separate registry over the same store/runtime preserves chat and MCP
    /// surfaces and cancellation, without ever advertising progress to chats.
    fn task_harness(&self, task: &str, session: &str) -> Result<Harness> {
        let progress: Arc<dyn Tool> = Arc::new(ReportProgress {
            vault: self.vault.clone(),
            task: task.into(),
            session: session.into(),
        });
        let store = self.store.clone();
        let register = move |mut registry: ToolRegistry| -> Result<ToolRegistry> {
            registry.register_host(progress.clone())?;
            bubblepaw_runner::todo::register(&mut registry, store.clone())?;
            registry.show_always(["report_progress", "todo"]);
            Ok(registry)
        };
        let mut config = self.task_harness_config.clone();
        let tools = register(self.base_tools.clone())?;
        if let Some(resolve) = config.registry_resolver.take() {
            config.registry_resolver = Some(Arc::new(move || {
                register(resolve()).expect("delegated tool names are reserved")
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
        self.prepare_code_task(vault, task_id, session)?;
        let selected = self.default_model();
        self.check_model(&selected)?;
        let model = Some(selected).filter(|m| !m.trim().is_empty());
        let mut system_prompt = self.run_prompt(session)?;
        system_prompt.push_str(DELEGATED_PROMPT);
        let mut approval = permission_policy(PermissionMode::Bypass);
        if bubblepaw_runner::worktrees::get(&self.store, session)?.is_some() {
            system_prompt.push_str(" This is a code task in an isolated git worktree. File and shell tools are rooted there and Confirm tools run without approval. The worktree and branch are kept for review. The source repository's uncommitted changes were not copied. Stay in this worktree.");
        } else {
            approval.confirm = ApprovalMode::Deny;
            system_prompt.push_str(" This task has no code-task worktree. File writes and shell commands are not available; Confirm-level tools are denied. Read-only tools, vault tools and report_progress remain available within this runner's tool set. Assign the task with a project to enable file writes and shell commands in its worktree.");
        }
        let harness = self.task_harness(task_id, session)?;
        let mut mirror = TodoMirror::new(self.vault.clone(), task_id, session, &harness);
        self.runtime.block_on(async {
            // Cancelled after the session was saved, before the engine starts.
            if is_cancelled(vault, task_id) {
                return Ok(Ended::Cancelled);
            }
            let mut events = harness.subscribe();
            let run = harness.start(
                session,
                prompt,
                RunOptions {
                    approval: Some(approval),
                    system_prompt: Some(system_prompt),
                    model,
                    client: Some(crate::desktop_client()),
                    ..RunOptions::default()
                },
            )?;
            mirror.run_id = run.id;
            let mut wait = pin!(tokio::time::timeout(limit, harness.wait_for_idle(session)));
            let mut watch = tokio::time::interval(WATCH_EVERY);
            watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut events_open = true;
            loop {
                tokio::select! {
                    outcome = wait.as_mut() => {
                        mirror.drain(&harness).await;
                        match outcome {
                            Ok(run) => return Ok(Ended::Run(run?)),
                            Err(_) => {
                                self.cancel_as(session, crate::desktop_client());
                                let _ = harness.wait_for_idle(session).await;
                                mirror.drain(&harness).await;
                                return Ok(Ended::Limit);
                            }
                        }
                    }
                    _ = watch.tick() => {
                        if is_cancelled(vault, task_id) {
                            self.cancel_as(session, crate::desktop_client());
                            let _ = harness.wait_for_idle(session).await;
                            mirror.drain(&harness).await;
                            return Ok(Ended::Cancelled);
                        }
                    }
                    event = events.recv(), if events_open => {
                        events_open = mirror.on_event(event, &harness).await;
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
        let selected = self.default_model();
        self.check_model(&selected)?;
        let model = Some(selected).filter(|m| !m.trim().is_empty());
        let mut system_prompt = self.run_prompt(session)?;
        system_prompt.push_str(DELEGATED_PROMPT);
        let harness = self.task_harness(task_id, session)?;
        let mut mirror = TodoMirror::new(self.vault.clone(), task_id, session, &harness);
        self.runtime.block_on(async {
            let mut events = harness.subscribe();
            let run = harness.start(
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
            mirror.run_id = run.id;
            let mut wait = pin!(tokio::time::timeout(limit, harness.wait_for_idle(session)));
            let mut events_open = true;
            let outcome = loop {
                tokio::select! {
                    outcome = wait.as_mut() => break outcome,
                    event = events.recv(), if events_open => {
                        events_open = mirror.on_event(event, &harness).await;
                    }
                }
            };
            mirror.drain(&harness).await;
            match outcome {
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
                    mirror.drain(&harness).await;
                    Ok(JobOutcome::TimedOut)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_and_hub_task_tool_lists_include_todo_but_chat_and_gateway_do_not() {
        for host_kind in [crate::HostKind::Desktop, crate::HostKind::Hub] {
            let dir = tempfile::tempdir().unwrap();
            let vault =
                Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
            let agent = Agent::open_with_execution(
                vault.clone(),
                crate::AgentConfig {
                    host_kind,
                    data_dir: dir.path().join("data"),
                    default_model: "scripted/scripted".into(),
                    bun_path: Some(dir.path().join("no-bun")),
                    test: crate::TestHooks {
                        scripted: Some((vec![], "Done.".into())),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                bubblepaw_core::ExecutionMode::Native,
                false,
            )
            .unwrap();
            let page = vault.create_page(None, "Plan".into()).unwrap();
            let task = tasks::create(&vault, "Read", &page, None, RUNNER).unwrap();
            let session = agent.create_session("Delegated").unwrap();
            let delegated = agent.task_harness(&task.id, &session.id).unwrap();
            let definitions = delegated
                .effective_tool_definitions(&session.id, &RunOptions::default())
                .unwrap();
            let todo = definitions.iter().find(|tool| tool.name == "todo").unwrap();
            assert_eq!(todo.input_schema["type"], "array");
            assert!(definitions
                .iter()
                .any(|tool| tool.name == "report_progress"));
            let chat = agent
                .harness
                .effective_tool_definitions(&session.id, &RunOptions::default())
                .unwrap();
            assert!(!chat.iter().any(|tool| tool.name == "todo"));
            let gateway = agent.gateway_tool_definitions(&session.id).unwrap();
            assert!(!gateway.iter().any(|tool| tool.name == "todo"));
        }
    }

    #[test]
    fn delegated_prompt_explains_the_mirrored_task_list() {
        assert!(super::DELEGATED_PROMPT.contains("report_progress"));
        assert!(super::DELEGATED_PROMPT.contains("Plan multi-step work with todo"));
        assert!(super::DELEGATED_PROMPT.contains("keep the list current"));
        assert!(super::DELEGATED_PROMPT.contains("appears as subtasks in the note"));
    }
}
