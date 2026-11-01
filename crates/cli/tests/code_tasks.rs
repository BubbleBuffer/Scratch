//! `run-tasks --once` runs a project task in a git worktree of the project.
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use support::{cli, field, scripted};
use vault_mcp::tasks;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn run_tasks_once_runs_a_project_task_in_a_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("tracked.txt"), "committed").unwrap();
    git(&repo, &["add", "tracked.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "initial",
        ],
    );
    let vault = dir.path().join("vault");
    let vault = vault.to_str().unwrap();
    let hooks = || {
        scripted(
            "await tools.write({path: 'written.txt', content: 'from the task'});",
            "Code task complete.",
        )
    };

    let project = cli(
        &[
            "project",
            "add",
            "--vault",
            vault,
            "--name",
            "repo",
            "--path",
            repo.to_str().unwrap(),
        ],
        Some(&hooks),
    );
    let project = project.trim().to_string();
    assert!(!project.is_empty());
    let id = cli(
        &[
            "task",
            "create",
            "--vault",
            vault,
            "--prompt",
            "Update the project",
            "--project",
            "repo",
        ],
        Some(&hooks),
    );
    let id = id.trim().to_string();
    let queued = cli(&["task", "show", "--vault", vault, &id], None);
    assert_eq!(field(&queued, "status"), tasks::QUEUED, "{queued}");
    assert_eq!(field(&queued, "worktree_path"), "", "{queued}");

    let ran = cli(
        &["run-tasks", "--vault", vault, "--model", "scripted/scripted", "--once"],
        Some(&hooks),
    );
    assert_eq!(ran.lines().collect::<Vec<_>>(), vec![format!("{id} done")], "{ran}");

    let shown = cli(&["task", "show", "--vault", vault, &id], None);
    assert_eq!(field(&shown, "status"), tasks::DONE, "{shown}");
    assert_eq!(field(&shown, "project"), project, "{shown}");
    let branch = field(&shown, "worktree_branch");
    assert!(branch.starts_with("bp/"), "{shown}");
    let worktree = PathBuf::from(field(&shown, "worktree_path"));
    assert!(worktree.join(".git").is_file(), "{shown}");
    assert_eq!(git(&worktree, &["branch", "--show-current"]), branch);
    assert_eq!(
        std::fs::read_to_string(worktree.join("written.txt")).unwrap(),
        "from the task"
    );
    assert!(!repo.join("written.txt").exists());
}
