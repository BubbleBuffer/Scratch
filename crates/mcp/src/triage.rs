//! Triage runs: a user-started AI task that reviews the pages of one project
//! (or a given set of pages) and proposes merges of duplicates, and the
//! scope the run is bound to. The task is an ordinary row of the AI tasks
//! database, pinned to the runner the caller names; the row's `triage`
//! property marks it and holds its scope. The agent adds the triage tools
//! (`triage_pages`, `triage_propose`) to such runs only. Proposals are stored
//! and merged by the core (`Vault::triage_*`).

use serde::{Deserialize, Serialize};
use vault_core::Vault;

use crate::tasks::{self, Task};
use crate::{err, runners, Result};

/// The task row's text property that marks a triage run and holds its
/// [`Scope`] as JSON.
pub const PROPERTY: &str = "triage";

/// The most pages a run is given to review.
pub const SCOPE_MAX_PAGES: usize = 300;

/// The fixed prompt of a triage run.
pub const PROMPT: &str = "Triage these notes. Call triage_pages to list the pages in scope (id, title, type, a short summary, created time). Find pages that duplicate or heavily overlap each other, judging from titles and summaries; read a page with the vault tools only when the summaries do not decide it. For each such pair call triage_propose with both page ids and a one-sentence reason. Only propose pairs of the same page type within the listed pages; the older page is kept and the newer one is merged into it after the user accepts. Do not edit, merge or delete pages yourself. When done, reply with a short summary of what you proposed.";

/// What a triage run reviews: the subtree under `root`, or `pages`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<String>,
}

/// Queues a triage run on `runner` (a PC's device id, or `hub`) over the
/// project whose top-level page is `root`, or over `pages` when `root` is
/// empty. The run shows in the AI tasks like any other, with its placeholder
/// in `root` (or the first of `pages`). Fewer than two pages in scope is an
/// error.
pub fn start(v: &Vault, root: Option<&str>, pages: &[String], runner: &str) -> Result<Task> {
    runners::check_runner(v, runner)?;
    let root = root.map(str::trim).filter(|r| !r.is_empty());
    let scope = match root {
        Some(root) => Scope {
            root: Some(root.to_owned()),
            pages: vec![],
        },
        None => Scope {
            root: None,
            pages: pages.to_vec(),
        },
    };
    let Some(page) = scope.root.clone().or_else(|| scope.pages.first().cloned()) else {
        return err("triage needs a project or a list of pages");
    };
    if scope.pages.len() > SCOPE_MAX_PAGES {
        return err(format!(
            "triage takes at most {SCOPE_MAX_PAGES} pages at a time"
        ));
    }
    let candidates = v.triage_candidates(scope.root.clone(), scope.pages.clone())?;
    if candidates.len() < 2 {
        return err("there are fewer than two pages to triage");
    }
    let json = serde_json::to_string(&scope).map_err(|e| crate::McpError(e.to_string()))?;
    tasks::create_on_with(v, PROMPT, &page, None, "agent", runner, &[(PROPERTY, &json)])
}

/// The scope of task row `task`, `None` when it is not a triage run.
pub fn scope(v: &Vault, task: &str) -> Result<Option<Scope>> {
    let page = v.load_page(task.to_owned())?;
    let json = tasks::text(&page.properties, PROPERTY);
    if json.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&json)
        .map(Some)
        .map_err(|e| crate::McpError(format!("bad triage scope: {e}")))
}
