//! Triage runs get the triage tools; other task runs do not.

use std::sync::Arc;

use serde_json::{json, Value};
use vault_agent::bubblepaw_sdk::harness::ExecutionMode;
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::bubblepaw_sdk::tools::ToolCall;
use vault_agent::{Agent, AgentConfig, PermissionMode, TestHooks, ToolPack};
use vault_core::{BlockKind, BlockOp, NewBlock, Vault};
use vault_mcp::{tasks, triage};

fn step(name: &str, arguments: Value) -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: vault_core::new_id(),
            name: name.into(),
            arguments,
        }],
    }
}

fn page(v: &Vault, parent: Option<&str>, title: &str, text: &str) -> String {
    std::thread::sleep(std::time::Duration::from_millis(5));
    let id = v.create_page(parent.map(String::from), title.into()).unwrap();
    v.apply(
        id.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: vault_core::new_id(),
                kind: BlockKind::Paragraph,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    id
}

fn open(root: &std::path::Path, v: Arc<Vault>, script: Vec<ScriptedStep>) -> Agent {
    Agent::open_with_execution(
        v,
        AgentConfig {
            data_dir: root.join("data"),
            default_model: "scripted/scripted".into(),
            bun_path: Some(root.join("no-bun")),
            permission_mode: PermissionMode::Manual,
            packs: vec![ToolPack::Desktop { root: root.into() }],
            test: TestHooks {
                scripted: Some((script, "Proposed.".into())),
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
        ExecutionMode::Native,
    )
    .unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    v: Arc<Vault>,
    root: String,
    a: String,
    b: String,
    outside: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Meeting notes", "Agenda and decisions");
    let b = page(&v, Some(&root), "Meeting notes (copy)", "Agenda and decisions");
    let other = page(&v, None, "Other project", "");
    let outside = page(&v, Some(&other), "Meeting notes again", "Agenda");
    // The device record the runner check needs.
    v.publish_device_activity().unwrap();
    Fixture { _dir: dir, v, root, a, b, outside }
}

fn start_triage(f: &Fixture) -> String {
    let task = triage::start(&f.v, Some(&f.root), &[], &f.v.device_id()).unwrap();
    assert_eq!(
        tasks::claim_next(&f.v, &f.v.device_id()).unwrap().unwrap().id,
        task.id
    );
    task.id
}

#[test]
fn a_triage_run_proposes_a_merge_through_its_tool() {
    let f = fixture();
    let task = start_triage(&f);
    let scope = triage::scope(&f.v, &task).unwrap().unwrap();
    assert_eq!(scope.root.as_deref(), Some(f.root.as_str()));
    let agent = open(
        f._dir.path(),
        f.v.clone(),
        vec![
            step("triage_pages", json!({})),
            step(
                "triage_propose",
                json!({"a": f.b, "b": f.a, "reason": "Same agenda"}),
            ),
        ],
    );
    let run = agent.run_task(&f.v, &task).unwrap();
    assert!(run.ok, "{run:?}");
    let pending = f.v.triage_proposals(true).unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].keep, f.a, "the older page is kept");
    assert_eq!(pending[0].dropped, f.b);
    assert_eq!(pending[0].task, task);
    assert_eq!(pending[0].reason, "Same agenda");
    // Proposing changes nothing else.
    assert!(!f.v.load_page(f.b.clone()).unwrap().in_trash);
}

#[test]
fn a_triage_run_cannot_propose_a_page_outside_its_scope() {
    let f = fixture();
    let task = start_triage(&f);
    let agent = open(
        f._dir.path(),
        f.v.clone(),
        vec![step(
            "triage_propose",
            json!({"a": f.a, "b": f.outside, "reason": "Looks alike"}),
        )],
    );
    assert!(agent.run_task(&f.v, &task).unwrap().ok);
    assert!(f.v.triage_proposals(false).unwrap().is_empty());
}

#[test]
fn an_ordinary_task_has_no_triage_tools() {
    let f = fixture();
    let task = tasks::create(&f.v, "Read the notes", &f.root, None, "agent").unwrap();
    assert_eq!(
        tasks::claim_next(&f.v, &f.v.device_id()).unwrap().unwrap().id,
        task.id
    );
    let agent = open(
        f._dir.path(),
        f.v.clone(),
        vec![step(
            "triage_propose",
            json!({"a": f.a, "b": f.b, "reason": "Same agenda"}),
        )],
    );
    agent.run_task(&f.v, &task.id).unwrap();
    assert!(f.v.triage_proposals(false).unwrap().is_empty());
}

#[test]
fn starting_a_run_checks_the_runner_and_the_scope() {
    let f = fixture();
    assert!(triage::start(&f.v, Some(&f.root), &[], "no-such-device").is_err());
    let lonely = page(&f.v, None, "Lonely", "x");
    assert!(
        triage::start(&f.v, Some(&lonely), &[], &f.v.device_id()).is_err(),
        "one page is nothing to triage"
    );
    let task = triage::start(&f.v, None, &[f.a.clone(), f.b.clone()], "hub").unwrap();
    assert_eq!(task.node, "hub");
    let scope = triage::scope(&f.v, &task.id).unwrap().unwrap();
    assert_eq!(scope.pages, vec![f.a.clone(), f.b.clone()]);
}
