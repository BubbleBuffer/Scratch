//! Delegated runs have a row-bound progress tool; interactive runs do not.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use vault_agent::bubblepaw_core::{
    Approval, ExecutionMode, Tool, ToolCall, ToolContext, ToolDefinition, ToolResult,
};
use vault_agent::bubblepaw_runner::provider::ScriptedStep;
use vault_agent::{Agent, AgentConfig, PermissionMode, TestHooks, ToolPack};
use vault_core::{PropValue, Vault};
use vault_mcp::tasks;

fn property(v: &Vault, id: &str, key: &str) -> String {
    v.load_page(id.into())
        .unwrap()
        .properties
        .into_iter()
        .find_map(|p| match p.value {
            PropValue::Text(text) if p.key == key => Some(text),
            _ => None,
        })
        .unwrap_or_default()
}

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
    let task = tasks::create(v, "Read the notes", &page, None, "agent").unwrap();
    assert_eq!(
        tasks::claim_next(v, &v.device_id()).unwrap().unwrap().id,
        task.id
    );
    task.id
}

fn open(
    root: &std::path::Path,
    v: Arc<Vault>,
    mode: ExecutionMode,
    script: Vec<ScriptedStep>,
    extra: Vec<Arc<dyn Tool>>,
) -> Agent {
    Agent::open_with_execution(
        v,
        AgentConfig {
            data_dir: root.join("data"),
            default_model: "scripted/scripted".into(),
            bun_path: Some(root.join("no-bun")),
            permission_mode: PermissionMode::Manual,
            packs: vec![ToolPack::Desktop { root: root.into() }],
            test: TestHooks {
                scripted: Some((script, "Finished reading.".into())),
                extra_tools: extra,
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
        mode,
        false,
    )
    .unwrap()
}

#[test]
fn desktop_js_task_reports_progress_and_keeps_it_when_done() {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let task = running(&v);
    let unrelated = running(&v);
    let agent = open(
        dir.path(),
        v.clone(),
        ExecutionMode::Js,
        vec![step(
            "js",
            json!({"code": "return await tools.report_progress({message: '  Read three pages  '});"}),
        )],
        vec![],
    );
    let before = vault_mcp::jobs::now_ms();
    let run = agent.run_task(&v, &task).unwrap();
    assert!(run.ok, "{run:?}");
    assert_eq!(property(&v, &task, "progress"), "Read three pages");
    let at: i64 = property(&v, &task, "progress_at").parse().unwrap();
    assert!((before..=vault_mcp::jobs::now_ms()).contains(&at));
    assert_eq!(tasks::get(&v, &task).unwrap().status, tasks::DONE);
    assert_eq!(property(&v, &unrelated, "progress"), "");

    // Reusing the engine binds the next run to its own row.
    assert!(agent.run_task(&v, &unrelated).unwrap().ok);
    assert_eq!(property(&v, &unrelated, "progress"), "Read three pages");
    assert_eq!(property(&v, &task, "progress_at"), at.to_string());
}

#[test]
fn desktop_native_task_reports_progress() {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let task = running(&v);
    let agent = open(
        dir.path(),
        v.clone(),
        ExecutionMode::Native,
        vec![step(
            "report_progress",
            json!({"message":"Reading references"}),
        )],
        vec![],
    );
    assert!(agent.run_task(&v, &task).unwrap().ok);
    assert_eq!(property(&v, &task, "progress"), "Reading references");
}

struct CancelTask {
    vault: Arc<Vault>,
    task: String,
}

#[async_trait]
impl Tool for CancelTask {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "test_cancel_task".into(),
            description: "Cancel the fixture task before its next progress call.".into(),
            input_schema: json!({"type":"object","properties":{}}),
            read_only: false,
            approval: Approval::None,
        }
    }

    async fn invoke(&self, _: ToolContext, _: Value) -> anyhow::Result<ToolResult> {
        assert!(tasks::cancel(&self.vault, &self.task).unwrap());
        // Stable sentinels prove a rejected call writes neither property.
        self.vault.set_property(
            self.task.clone(),
            "progress_at".into(),
            PropValue::Text("123".into()),
        )?;
        Ok(ToolResult::text("Cancelled."))
    }
}

#[test]
fn report_tool_rejects_after_cancel_without_changing_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let task = running(&v);
    let agent = open(
        dir.path(),
        v.clone(),
        ExecutionMode::Js,
        vec![step(
            "js",
            json!({"code": r#"
            await tools.report_progress({message: "Before cancellation"});
            await tools.test_cancel_task({});
            try {
                await tools.report_progress({message: "Too late"});
                throw new Error("accepted progress after cancellation");
            } catch (error) {
                if (!String(error).includes("running")) throw error;
            }
            return "rejected";
        "#}),
        )],
        vec![Arc::new(CancelTask {
            vault: v.clone(),
            task: task.clone(),
        })],
    );
    let run = agent.run_task(&v, &task).unwrap();
    let state = agent.session_state(&run.session_id).unwrap();
    assert!(
        state
            .messages
            .iter()
            .any(|m| m.content.contains("rejected")),
        "{:?}",
        state.messages
    );
    assert_eq!(tasks::get(&v, &task).unwrap().status, tasks::CANCELLED);
    assert_eq!(property(&v, &task, "progress"), "Before cancellation");
    assert_eq!(property(&v, &task, "progress_at"), "123");
}

#[test]
fn normal_chat_cannot_discover_or_call_progress_even_after_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let task = running(&v);
    let agent = open(dir.path(), v.clone(), ExecutionMode::Js, vec![], vec![]);
    assert!(agent.run_task(&v, &task).unwrap().ok);
    let session = agent.create_session("Normal chat").unwrap();
    let run = agent
        .run_script(
            &session.id,
            "Find tools",
            r#"
        const found = await tools.$search("select:report_progress");
        let rejected = false;
        try { await tools.report_progress({message: "Not a task"}); }
        catch (_) { rejected = true; }
        return {names: found.tools.map(t => t.name), rejected};
    "#,
        )
        .unwrap();
    assert_eq!(run.status, "completed");
    let result: Value = serde_json::from_str(&run.text.unwrap()).unwrap();
    assert_eq!(result, json!({"names":[],"rejected":true}));
    assert_eq!(property(&v, &task, "progress"), "");
}
