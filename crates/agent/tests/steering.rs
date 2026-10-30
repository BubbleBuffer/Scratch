//! A to-do the user adds under a running task reaches the worker: it goes on
//! the worker's todo list, is sent as a steering message, and the attempt's
//! history shows it sent and acknowledged.

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use vault_agent::bubblepaw_sdk::approvals::Approval;
use vault_agent::bubblepaw_sdk::harness::ExecutionMode;
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::bubblepaw_sdk::tools::{Tool, ToolCall, ToolContext, ToolDefinition, ToolResult};
use vault_agent::{Agent, AgentConfig, PermissionMode, TestHooks, ToolPack};
use vault_core::Vault;
use vault_mcp::tasks::{self, attempts};

/// Signals it started, then waits for the run's cancel (or 60 s).
struct HoldTool {
    started: mpsc::Sender<()>,
}

#[async_trait]
impl Tool for HoldTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "test_hold".into(),
            description: "Test tool: signals it started, then waits to be cancelled.".into(),
            input_schema: json!({"type": "object", "properties": {}}),
            read_only: true,
            approval: Approval::None,
        }
    }

    async fn invoke(&self, context: ToolContext, _input: Value) -> anyhow::Result<ToolResult> {
        let _ = self.started.send(());
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(60)) => Ok(ToolResult::text("held")),
            _ = context.cancellation.cancelled() => anyhow::bail!("cancelled"),
        }
    }
}

fn hold_step() -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: vault_core::new_id(),
            name: "js".into(),
            arguments: json!({"code": "return await tools.test_hold({});"}),
        }],
    }
}

fn open(root: &std::path::Path, v: Arc<Vault>, started: mpsc::Sender<()>) -> Agent {
    Agent::open_with_execution(
        v,
        AgentConfig {
            data_dir: root.join("data"),
            default_model: "scripted/scripted".into(),
            bun_path: Some(root.join("no-bun")),
            permission_mode: PermissionMode::Manual,
            packs: vec![ToolPack::Desktop { root: root.into() }],
            test: TestHooks {
                scripted: Some((vec![hold_step()], "Finished.".into())),
                extra_tools: vec![Arc::new(HoldTool { started })],
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
        ExecutionMode::Js,
    )
    .unwrap()
}

fn kinds(v: &Vault, task: &str) -> Vec<String> {
    attempts::list(v, task)
        .unwrap()
        .first()
        .map(|a| a.events.iter().map(|e| e.kind.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn a_step_added_to_a_running_task_reaches_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let page = v.create_page(None, "Work".into()).unwrap();
    let created = tasks::create(&v, "Read the notes", &page, None, "agent").unwrap();
    let task = tasks::claim_next(&v, &v.device_id()).unwrap().unwrap();
    assert_eq!(task.id, created.id);

    let (started, reached) = mpsc::channel();
    let agent = Arc::new(open(dir.path(), v.clone(), started));
    let worker = {
        let (agent, v, id) = (agent.clone(), v.clone(), task.id.clone());
        std::thread::spawn(move || agent.run_task(&v, &id))
    };
    reached
        .recv_timeout(Duration::from_secs(60))
        .expect("the run reached the tool");

    tasks::add_step(&v, &task.id, "Check the dates").unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let k = kinds(&v, &task.id);
        if k.iter().any(|x| x == attempts::kind::STEERING_SENT)
            && k.iter().any(|x| x == attempts::kind::STEERING_ACKNOWLEDGED)
        {
            break;
        }
        assert!(Instant::now() < deadline, "no steering in the history: {k:?}");
        std::thread::sleep(Duration::from_millis(200));
    }
    let record = &attempts::list(&v, &task.id).unwrap()[0];
    assert_eq!(record.steered.len(), 1);
    // The step is not sent twice.
    std::thread::sleep(Duration::from_secs(3));
    let sent = kinds(&v, &task.id)
        .iter()
        .filter(|x| *x == attempts::kind::STEERING_SENT)
        .count();
    assert_eq!(sent, 1);

    tasks::cancel(&v, &task.id).unwrap();
    let run = worker.join().unwrap().unwrap();
    assert!(!run.ok, "{run:?}");
    assert_eq!(tasks::get(&v, &task.id).unwrap().status, tasks::CANCELLED);
}
