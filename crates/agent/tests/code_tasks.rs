//! Code tasks use real git worktrees, with a deterministic scripted provider.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use vault_agent::bubblepaw_sdk::approvals::{Approval};
use vault_agent::bubblepaw_sdk::core::{ApprovalState};
use vault_agent::bubblepaw_sdk::events::{EventKind};
use vault_agent::bubblepaw_sdk::harness::{ExecutionMode};
use vault_agent::bubblepaw_sdk::tools::{Tool, ToolCall, ToolContext, ToolDefinition, ToolResult};
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::{Agent, AgentConfig, PermissionMode, TestHooks, ToolPack};
use vault_core::Vault;
use vault_mcp::{runners, tasks};

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

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    vault: Arc<Vault>,
}
impl Fixture {
    fn new(committed: Option<bool>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        if let Some(commit) = committed {
            git(&repo, &["init", "-q"]);
            if commit {
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
            }
        }
        let vault = Arc::new(Vault::open(root.join("vault").to_string_lossy().into()).unwrap());
        Self {
            _dir: dir,
            root,
            repo,
            vault,
        }
    }

    fn open(
        &self,
        desktop: bool,
        mode: ExecutionMode,
        steps: Vec<ScriptedStep>,
        extra_tools: Vec<Arc<dyn Tool>>,
    ) -> Agent {
        Agent::open_with_execution(
            self.vault.clone(),
            AgentConfig {
                data_dir: self.root.join("data"),
                default_model: "scripted/scripted".into(),
                bun_path: Some(self.root.join("no-bun")),
                permission_mode: PermissionMode::Manual,
                packs: if desktop {
                    vec![ToolPack::Desktop {
                        root: self.repo.clone(),
                    }]
                } else {
                    vec![]
                },
                test: TestHooks {
                    scripted: Some((steps, "Code task complete.".into())),
                    extra_tools,
                    ..TestHooks::default()
                },
                ..AgentConfig::default()
            },
            mode,
        )
        .unwrap()
    }

    fn task(&self, project: &str) -> String {
        let page = self.vault.create_page(None, "Code work".into()).unwrap();
        let task = runners::assign(
            &self.vault,
            "Update the project",
            &page,
            None,
            &self.vault.device_id(),
            Some(project),
        )
        .unwrap();
        assert_eq!(
            tasks::claim_next(&self.vault, &self.vault.device_id())
                .unwrap()
                .unwrap()
                .id,
            task.id
        );
        task.id
    }
}

fn call(name: &str, arguments: Value) -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: vault_core::new_id(),
            name: name.into(),
            arguments,
        }],
    }
}

// First provider action checks that review metadata was saved before tools run.
struct CheckRow {
    vault: Arc<Vault>,
    observed: Arc<AtomicBool>,
}
#[async_trait]
impl Tool for CheckRow {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_worktree_row".into(),
            description: "Check task metadata".into(),
            input_schema: json!({"type":"object","properties":{}}),
            read_only: true,
            approval: Approval::None,
        }
    }
    async fn invoke(&self, context: ToolContext, _: Value) -> Result<ToolResult> {
        let task = tasks::list(&self.vault, 10)
            .map_err(anyhow::Error::msg)?
            .into_iter()
            .find(|t| t.status == tasks::RUNNING)
            .unwrap();
        ensure!(task.worktree_branch == format!("bp/{}", context.session_id));
        ensure!(Path::new(&task.worktree_path).join(".git").is_file());
        ensure!(!Path::new(&task.worktree_path).join("written.txt").exists());
        self.observed.store(true, Ordering::SeqCst);
        Ok(ToolResult::text("metadata saved before tools"))
    }
}

#[test]
fn code_task_roots_native_and_js_tools_in_a_retained_worktree_and_retry_is_fresh() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let f = Fixture::new(Some(true));
        std::fs::write(f.repo.join("tracked.txt"), "uncommitted change").unwrap();
        std::fs::write(f.repo.join("untracked.txt"), "untracked").unwrap();
        let observed = Arc::new(AtomicBool::new(false));
        let code = "await tools.check_worktree_row({}); await tools.write({path:'written.txt',content:'first'}); await tools.edit({path:'written.txt',old_string:'first',new_string:'edited'}); const p = await tools.exec({command:'git rev-parse --show-toplevel'}); await wait({handle:p.handle}); return await tools.poll({handle:p.handle});";
        let steps = match mode {
            ExecutionMode::Js => vec![call("js", json!({"code":code}))],
            _ => vec![
                call("check_worktree_row", json!({})),
                call("write", json!({"path":"written.txt","content":"first"})),
                call(
                    "edit",
                    json!({"path":"written.txt","old_string":"first","new_string":"edited"}),
                ),
                call("exec", json!({"command":"git rev-parse --show-toplevel"})),
            ],
        };
        let agent = f.open(
            true,
            mode,
            steps,
            vec![Arc::new(CheckRow {
                vault: f.vault.clone(),
                observed: observed.clone(),
            })],
        );
        let project = agent.projects_list().unwrap().remove(0);
        let id = f.task(&project.id);
        let run = agent.run_task(&f.vault, &id).unwrap();
        assert!(run.ok, "{run:?}");
        assert!(
            observed.load(Ordering::SeqCst),
            "metadata was not present at the first provider action"
        );
        let done = tasks::get(&f.vault, &id).unwrap();
        assert_eq!(done.status, tasks::REVIEW);
        assert_eq!(done.project, project.id);
        assert_eq!(done.worktree_branch, format!("bp/{}", run.session_id));
        let worktree = PathBuf::from(&done.worktree_path);
        assert_eq!(
            worktree,
            f.root
                .join("data/agent/agent.data/worktrees")
                .join(&run.session_id)
        );
        assert_eq!(
            git(&worktree, &["branch", "--show-current"]),
            done.worktree_branch
        );
        assert_eq!(
            git(&worktree, &["rev-parse", "HEAD"]),
            git(&f.repo, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("tracked.txt")).unwrap(),
            "committed"
        );
        assert!(!worktree.join("untracked.txt").exists());
        assert_eq!(
            std::fs::read_to_string(worktree.join("written.txt")).unwrap(),
            "edited"
        );
        assert!(!f.repo.join("written.txt").exists());
        assert_eq!(
            std::fs::read_to_string(f.repo.join("tracked.txt")).unwrap(),
            "uncommitted change"
        );
        assert!(agent
            .session_state(&run.session_id)
            .unwrap()
            .pending_approvals
            .is_empty());
        assert_eq!(agent.host_info().permission_mode, PermissionMode::Manual);
        if mode == ExecutionMode::Js {
            let state = agent.session_state(&run.session_id).unwrap();
            assert!(
                state.messages.iter().any(|m| m
                    .content
                    .contains(&worktree.to_string_lossy().replace('\\', "/"))),
                "exec did not report worktree cwd"
            );
        }
        tasks::retry(&f.vault, &id).unwrap();
        let queued_retry = tasks::get(&f.vault, &id).unwrap();
        assert!(queued_retry.worktree_branch.is_empty() && queued_retry.worktree_path.is_empty());
        tasks::claim_next(&f.vault, &f.vault.device_id())
            .unwrap()
            .unwrap();
        let retry = agent.run_task(&f.vault, &id).unwrap();
        assert!(retry.ok, "{retry:?}");
        assert_ne!(retry.session_id, run.session_id);
        let retried = tasks::get(&f.vault, &id).unwrap();
        assert_ne!(retried.worktree_path, done.worktree_path);
        assert_ne!(retried.worktree_branch, done.worktree_branch);
        assert!(
            worktree.join("written.txt").exists(),
            "retry removed earlier work"
        );
    }
}

#[test]
fn code_task_non_git_and_empty_repositories_fail_without_running_the_provider() {
    for (repo, message) in [
        (None, "not a git repository"),
        (Some(false), "has no commits"),
    ] {
        let f = Fixture::new(repo);
        let agent = f.open(
            true,
            ExecutionMode::Native,
            vec![call(
                "write",
                json!({"path":"must-not-exist.txt","content":"bad"}),
            )],
            vec![],
        );
        let id = f.task(&agent.projects_list().unwrap()[0].id);
        let run = agent.run_task(&f.vault, &id).unwrap();
        assert!(!run.ok);
        assert!(run.text.contains(message), "{}", run.text);
        let task = tasks::get(&f.vault, &id).unwrap();
        assert_eq!(task.status, tasks::FAILED);
        assert!(task.error.contains(message));
        assert!(task.worktree_path.is_empty() && task.worktree_branch.is_empty());
        assert!(agent
            .session_state(&run.session_id)
            .unwrap()
            .messages
            .is_empty());
        assert!(!f.repo.join("must-not-exist.txt").exists());
    }
}

#[test]
fn a_stale_published_project_cannot_fall_back_to_the_current_folder() {
    let f = Fixture::new(Some(true));
    let agent = f.open(true, ExecutionMode::Native, vec![], vec![]);
    f.vault
        .publish_device_projects(vec![vault_core::DeviceProject {
            id: "removed-project".into(),
            name: "Removed".into(),
        }])
        .unwrap();
    let id = f.task("removed-project");
    let run = agent.run_task(&f.vault, &id).unwrap();
    assert!(!run.ok);
    assert!(run.text.contains("unknown project"), "{}", run.text);
    let failed = tasks::get(&f.vault, &id).unwrap();
    assert_eq!(failed.status, tasks::FAILED);
    assert!(failed.worktree_branch.is_empty() && failed.worktree_path.is_empty());
}

#[test]
fn code_task_requires_desktop_tools_and_plain_tasks_keep_working_without_them() {
    let f = Fixture::new(Some(true));
    let project_id = "saved-project";
    f.vault
        .publish_device_projects(vec![vault_core::DeviceProject {
            id: project_id.into(),
            name: "Saved project".into(),
        }])
        .unwrap();
    let agent = f.open(false, ExecutionMode::Native, vec![], vec![]);
    let id = f.task(project_id);
    let run = agent.run_task(&f.vault, &id).unwrap();
    assert!(!run.ok);
    assert!(
        run.text.contains("desktop tools are disabled on this PC"),
        "{}",
        run.text
    );
    assert_eq!(tasks::get(&f.vault, &id).unwrap().status, tasks::FAILED);
    let page = f.vault.create_page(None, "Plain task".into()).unwrap();
    let plain = tasks::create(&f.vault, "Summarize notes", &page, None, "agent").unwrap();
    tasks::claim_next(&f.vault, &f.vault.device_id())
        .unwrap()
        .unwrap();
    assert!(agent.run_task(&f.vault, &plain.id).unwrap().ok);
    let done = tasks::get(&f.vault, &plain.id).unwrap();
    assert!(
        done.project.is_empty() && done.worktree_path.is_empty() && done.worktree_branch.is_empty()
    );
}

// Plain tasks have no worktree: Confirm calls are denied while the other tools
// still run. The script catches each denial so every result comes back in one run.
const PLAIN_TASK_JS: &str = r#"const attempt = async (run) => { try { return await run(); } catch (e) { return String(e); } };
await tools.report_progress({message: 'reading notes'});
return [
  await tools.read({path: 'tracked.txt'}),
  await tools.read_page({page: 'Reference'}),
  await attempt(() => tools.write({path: 'notes-task.txt', content: 'denied'})),
  await attempt(() => tools.exec({command: 'echo shell > exec-task.txt'})),
];"#;

#[test]
fn task_without_a_project_denies_writes_and_shell_but_still_reads_and_reports() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let f = Fixture::new(Some(true));
        let reference = f.vault.create_page(None, "Reference".into()).unwrap();
        f.vault
            .insert_markdown(reference, None, None, true, "Reference body".into())
            .unwrap();
        let steps = match mode {
            ExecutionMode::Js => vec![call("js", json!({"code": PLAIN_TASK_JS}))],
            _ => vec![
                call("report_progress", json!({"message": "reading notes"})),
                call("read", json!({"path": "tracked.txt"})),
                call("read_page", json!({"page": "Reference"})),
                call(
                    "write",
                    json!({"path": "notes-task.txt", "content": "denied"}),
                ),
                call("exec", json!({"command": "echo shell > exec-task.txt"})),
            ],
        };
        let agent = f.open(true, mode, steps, vec![]);
        let page = f.vault.create_page(None, "Notes work".into()).unwrap();
        let plain = tasks::create(&f.vault, "Write a file", &page, None, "agent").unwrap();
        tasks::claim_next(&f.vault, &f.vault.device_id())
            .unwrap()
            .unwrap();
        // Bounded: a regression that asks for approval fails here instead of hanging.
        let run = agent
            .run_task_with_limit(&f.vault, &plain.id, std::time::Duration::from_secs(30))
            .unwrap();
        assert!(run.ok, "{run:?}");
        let done = tasks::get(&f.vault, &plain.id).unwrap();
        assert_eq!(done.status, tasks::DONE);
        assert_eq!(done.progress, "reading notes");
        assert!(
            done.project.is_empty()
                && done.worktree_path.is_empty()
                && done.worktree_branch.is_empty()
        );
        assert!(!f.repo.join("notes-task.txt").exists());
        assert!(!f.repo.join("exec-task.txt").exists());
        assert!(!f.root.join("data/agent/agent.data/worktrees").exists());
        let state = agent.session_state(&run.session_id).unwrap();
        assert!(state.pending_approvals.is_empty());
        let events = agent.events_after(0, Some(&run.session_id)).unwrap();
        for denied_tool in ["write", "exec"] {
            assert!(
                events.events.iter().any(|event| matches!(
                    &event.kind,
                    EventKind::ApprovalRequested { approval }
                        if approval.tool == denied_tool
                            && approval.state == ApprovalState::Denied
                            && approval.reason.as_deref() == Some("policy")
                )),
                "{mode:?}: {denied_tool} did not return a denial: {:?}",
                events.events
            );
        }
        let transcript: String = state.messages.iter().map(|m| m.content.as_str()).collect();
        assert!(transcript.contains("committed"), "{transcript}");
        assert!(transcript.contains("Reference body"), "{transcript}");
        assert!(
            transcript.matches("Denied by the user").count() >= 2,
            "{transcript}"
        );
    }
}

#[test]
fn code_task_that_times_out_keeps_its_worktree_and_review_metadata() {
    let f = Fixture::new(Some(true));
    let agent = f.open(
        true,
        ExecutionMode::Native,
        vec![call("test_sleep", json!({"ms":60_000}))],
        vec![Arc::new(vault_agent::testing::SleepTool)],
    );
    let id = f.task(&agent.projects_list().unwrap()[0].id);
    let run = agent
        .run_task_with_limit(&f.vault, &id, std::time::Duration::from_millis(20))
        .unwrap();
    assert!(!run.ok);
    assert!(run.text.contains("limit"), "{}", run.text);
    let failed = tasks::get(&f.vault, &id).unwrap();
    assert_eq!(failed.status, tasks::FAILED);
    assert_eq!(failed.worktree_branch, format!("bp/{}", run.session_id));
    assert!(Path::new(&failed.worktree_path)
        .join("tracked.txt")
        .is_file());
    assert_eq!(
        git(&f.repo, &["rev-parse", &failed.worktree_branch]),
        git(&f.repo, &["rev-parse", "HEAD"])
    );
}

#[test]
fn project_changes_publish_only_ids_and_names_on_this_device() {
    let f = Fixture::new(None);
    let agent = f.open(true, ExecutionMode::Native, vec![], vec![]);
    let other = f.root.join("other");
    std::fs::create_dir(&other).unwrap();
    let project = agent.project_add("Other", other.to_str().unwrap()).unwrap();
    let record = || {
        f.vault
            .device_records()
            .unwrap()
            .into_iter()
            .find(|r| r.id == f.vault.device_id())
            .unwrap()
    };
    assert!(record()
        .projects
        .iter()
        .any(|p| p.id == project.id && p.name == "Other"));
    agent.project_rename(&project.id, "Renamed").unwrap();
    assert!(record()
        .projects
        .iter()
        .any(|p| p.id == project.id && p.name == "Renamed"));
    let raw = f
        .vault
        .type_data(
            vault_core::node_record_id(&f.vault.device_id()),
            "node".into(),
        )
        .unwrap();
    assert!(!raw.contains(other.to_str().unwrap()));
    assert!(!raw.contains("path"));
    agent.project_remove(&project.id).unwrap();
    assert!(!record().projects.iter().any(|p| p.id == project.id));
}
