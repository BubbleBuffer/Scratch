//! A delegated run can ask the user a question with `ask_user`: the task
//! stays running with the question set, the user's reply is the tool's result,
//! and a run that ends while it waits is "waiting for input", not failed.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use vault_agent::bubblepaw_sdk::harness::ExecutionMode;
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::bubblepaw_sdk::tools::ToolCall;
use vault_agent::{Agent, AgentConfig, PermissionMode, TestHooks, ToolPack};
use vault_core::Vault;
use vault_mcp::tasks::{self, attempts};

fn step(name: &str, arguments: Value) -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: vault_core::new_id(),
            name: name.into(),
            arguments,
        }],
    }
}

fn running(v: &Vault) -> String {
    let page = v.create_page(None, "Work".into()).unwrap();
    let task = tasks::create(v, "Decide it", &page, None, "agent").unwrap();
    tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
    task.id
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
                scripted: Some((script, "All done.".into())),
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
        ExecutionMode::Native,
    )
    .unwrap()
}

#[test]
fn ask_user_waits_for_the_reply_and_the_run_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let task = running(&v);
    let agent = open(
        dir.path(),
        v.clone(),
        vec![step("ask_user", json!({"question": "Blue or green?"}))],
    );
    let (vault, id) = (v.clone(), task.clone());
    let answerer = std::thread::spawn(move || {
        for _ in 0..200 {
            if tasks::get(&vault, &id).unwrap().question == "Blue or green?" {
                assert_eq!(tasks::get(&vault, &id).unwrap().status, tasks::RUNNING);
                assert!(tasks::reply(&vault, &id, " Green ").unwrap());
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the question never appeared");
    });
    let run = agent.run_task(&v, &task).unwrap();
    answerer.join().unwrap();
    assert!(run.ok, "{run:?}");
    let t = tasks::get(&v, &task).unwrap();
    assert_eq!(t.status, tasks::DONE);
    assert!(t.question.is_empty());
    let kinds: Vec<String> = attempts::list(&v, &task).unwrap()[0]
        .events
        .iter()
        .map(|e| e.kind.clone())
        .collect();
    assert!(kinds.contains(&attempts::kind::QUESTION.to_string()));
    assert!(kinds.contains(&attempts::kind::REPLY.to_string()));
}
