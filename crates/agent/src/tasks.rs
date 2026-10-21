//! AI tasks run by the agent (runner value `agent`).

use anyhow::Result;
use vault_core::Vault;
use vault_mcp::tasks::{self, PromptTools};

use crate::Agent;

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
        let outcome = match self.run_and_wait(&session.id, &prompt) {
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
