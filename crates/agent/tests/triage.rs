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

#[test]
fn a_triage_run_applies_a_link_and_a_tag_and_records_them() {
    let f = fixture();
    let task = start_triage(&f);
    let agent = open(
        f._dir.path(),
        f.v.clone(),
        vec![
            step("triage_link", json!({"from": f.a, "to": f.b})),
            // The same link again changes nothing and records nothing.
            step("triage_link", json!({"from": f.a, "to": f.b})),
            step("triage_tag", json!({"page": f.b, "tag": "#meetings"})),
            // Out of scope: refused.
            step("triage_tag", json!({"page": f.outside, "tag": "meetings"})),
        ],
    );
    let before = f.v.load_page(f.a.clone()).unwrap();
    assert!(agent.run_task(&f.v, &task).unwrap().ok);

    let after = f.v.load_page(f.a.clone()).unwrap();
    let link = after.blocks.last().unwrap();
    assert_eq!(link.kind, BlockKind::LinkToPage);
    assert_eq!(link.target.as_deref(), Some(f.b.as_str()));
    assert_eq!(after.blocks.len(), before.blocks.len() + 1);
    let tags = |id: &str| {
        f.v.load_page(id.into()).unwrap().properties.into_iter().find(|p| p.key == "tags")
    };
    assert_eq!(
        tags(&f.b).map(|p| p.value),
        Some(vault_core::PropValue::MultiSelect(vec!["meetings".into()]))
    );
    assert!(tags(&f.outside).is_none());

    let done = triage::actions(&f.v, &task).unwrap();
    assert_eq!(done.len(), 2, "{done:?}");
    assert_eq!((done[0].kind.as_str(), done[0].page.as_str()), ("link", f.a.as_str()));
    assert_eq!((done[1].kind.as_str(), done[1].target.as_str()), ("tag", "meetings"));

    // The run is listed with both actions until they are undone.
    let runs = triage::applied_runs(&f.v).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].task, task);
    assert_eq!(runs[0].actions, done);

    // The recorded state undoes both.
    assert_eq!(triage::undo_actions(&f.v, &task).unwrap(), 2);
    assert!(triage::applied_runs(&f.v).unwrap().is_empty());
    assert_eq!(f.v.load_page(f.a.clone()).unwrap().blocks, before.blocks);
    assert!(tags(&f.b).is_none());
    assert!(triage::actions(&f.v, &task).unwrap().is_empty());
}

#[test]
fn undoing_a_tag_restores_the_previous_tags_exactly() {
    let f = fixture();
    f.v.set_property(
        f.a.clone(),
        "tags".into(),
        vault_core::PropValue::MultiSelect(vec!["Work".into(), "q4".into()]),
    )
    .unwrap();
    assert!(f.v.triage_tag(f.a.clone(), "work".into()).unwrap().is_none(), "ignoring case");
    let action = f.v.triage_tag(f.a.clone(), "plan".into()).unwrap().unwrap();
    assert_eq!(action.previous, Some(vec!["Work".to_string(), "q4".to_string()]));
    f.v.triage_undo_action(&action).unwrap();
    let value = f.v.load_page(f.a.clone()).unwrap().properties.into_iter().find(|p| p.key == "tags");
    assert_eq!(
        value.map(|p| p.value),
        Some(vault_core::PropValue::MultiSelect(vec!["Work".into(), "q4".into()]))
    );
    assert!(f.v.triage_tag(f.a.clone(), "a,b".into()).is_err());
    assert!(f.v.triage_link(f.a.clone(), f.outside.clone()).is_err(), "other project");
}
