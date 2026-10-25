//! `run-tasks --once` claims a task made with `task create`, runs it with the
//! agent (scripted provider), and the result and mirrored to-dos land in the
//! vault.
mod support;

use support::{cli, cli_err, field, scripted};
use vault_core::{BlockKind, Vault};
use vault_mcp::tasks;

#[test]
fn run_tasks_once_finishes_a_created_task_and_mirrors_its_todos() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let vault = vault.to_str().unwrap();

    let page = cli(&["page", "create", "--vault", vault, "--title", "Plan"], None);
    let page = page.trim();
    let id = cli(
        &["task", "create", "--vault", vault, "--prompt", "Plan the report", "--page", page],
        None,
    );
    let id = id.trim().to_string();
    let before = cli(&["task", "show", "--vault", vault, &id], None);
    assert_eq!(field(&before, "status"), tasks::QUEUED, "{before}");

    let hooks = || {
        scripted(
            "await tools.todo({todos: [{text: 'Find data', status: 'done'}, {text: 'Write up', status: 'pending'}]});",
            "Report planned.",
        )
    };
    let ran = cli(
        &["run-tasks", "--vault", vault, "--model", "scripted/scripted", "--once"],
        Some(&hooks),
    );
    assert_eq!(ran.lines().collect::<Vec<_>>(), vec![format!("{id} done")], "{ran}");

    let shown = cli(&["task", "show", "--vault", vault, &id], None);
    assert_eq!(field(&shown, "status"), tasks::DONE, "{shown}");
    assert_eq!(field(&shown, "error"), "", "{shown}");
    let result = shown.split_once("result:\n").map(|(_, r)| r.trim());
    assert_eq!(result, Some("Report planned."), "{shown}");

    // Nothing is left: another pass claims nothing and prints nothing.
    let again = cli(
        &["run-tasks", "--vault", vault, "--model", "scripted/scripted", "--once"],
        Some(&hooks),
    );
    assert_eq!(again, "");

    // The todo call is mirrored as nested to-dos under the task's placeholder.
    let v = Vault::open(vault.into()).unwrap();
    let task = tasks::get(&v, &id).unwrap();
    assert_eq!(task.page.as_deref(), Some(page));
    let block = task.block.clone().expect("placeholder block");
    let doc = v.load_page(page.into()).unwrap();
    let placeholder = doc
        .blocks
        .iter()
        .find(|b| b.id == block)
        .expect("placeholder in the page");
    let todos: Vec<(&str, bool)> = placeholder
        .children
        .iter()
        .filter(|c| c.kind == BlockKind::Todo)
        .map(|c| (c.text.as_str(), c.checked))
        .collect();
    assert_eq!(todos, vec![("Find data", true), ("Write up", false)]);
    assert!(
        doc.blocks.iter().any(|b| b.text == "Report planned."),
        "the result is not in the page"
    );
}

#[test]
fn a_vault_open_in_another_instance_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let held = Vault::open(vault.to_string_lossy().into()).unwrap();
    assert!(held.is_owner());
    let error = cli_err(&["run-tasks", "--vault", vault.to_str().unwrap(), "--once"]);
    assert!(error.contains("close Scratch or use another vault"), "{error}");
}

#[test]
fn a_task_without_a_page_gets_one_pinned_to_this_device() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let vault = vault.to_str().unwrap();
    let id = cli(
        &["task", "create", "--vault", vault, "--prompt", "Summarize\nmore"],
        None,
    );
    let v = Vault::open(vault.into()).unwrap();
    let task = tasks::get(&v, id.trim()).unwrap();
    let page = v.load_page(task.page.clone().unwrap()).unwrap();
    assert_eq!(page.title, "Summarize");
    assert_eq!(task.node, v.device_id());
    assert_eq!(task.status, tasks::QUEUED);
}
