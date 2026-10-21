//! vault_agent on a temp vault, with script runs and the scripted provider.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use vault_agent::bubblepaw_core::{
    Approval, EventKind, Tool, ToolCall, ToolContext, ToolDefinition, ToolResult,
};
use vault_agent::bubblepaw_runner::provider::ScriptedStep;
use vault_agent::{
    tasks::session_title, Agent, AgentConfig, Forwarded, PermissionMode, Sent, TestHooks,
};
use vault_core::Vault;
use vault_mcp::tasks;

struct Temp {
    _dir: tempfile::TempDir,
    vault: Arc<Vault>,
    data: PathBuf,
}

fn temp() -> Temp {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let vault = Arc::new(Vault::open(root.to_string_lossy().into_owned()).unwrap());
    let data = dir.path().join("data");
    Temp {
        _dir: dir,
        vault,
        data,
    }
}

fn js(code: &str) -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: format!("call-{}", uuid()),
            name: "js".into(),
            arguments: json!({ "code": code }),
        }],
    }
}

fn uuid() -> String {
    vault_core::new_id()
}

fn open(t: &Temp, steps: Vec<ScriptedStep>, extra: Vec<Arc<dyn Tool>>) -> Agent {
    open_with(t, steps, extra, false)
}

fn open_with(
    t: &Temp,
    steps: Vec<ScriptedStep>,
    extra: Vec<Arc<dyn Tool>>,
    confirm_tool: bool,
) -> Agent {
    Agent::open(
        t.vault.clone(),
        AgentConfig {
            data_dir: t.data.clone(),
            default_model: "scripted/scripted".into(),
            // An explicit missing override keeps these scripted tests independent
            // of Claude/Bun installations discovered on the developer machine.
            bun_path: Some(t.data.join("missing-test-bun")),
            permission_mode: PermissionMode::Manual,
            test: TestHooks {
                scripted: Some((steps, "All done.".into())),
                extra_tools: extra,
                confirm_tool,
            },
            ..AgentConfig::default()
        },
    )
    .unwrap()
}

fn subscribe(agent: &Agent) -> mpsc::Receiver<Forwarded> {
    let (tx, rx) = mpsc::channel();
    agent.forward_events(move |event| tx.send(event).is_ok());
    rx
}

/// Events of `session` until one matches `stop`, which is returned last.
fn until(
    rx: &mpsc::Receiver<Forwarded>,
    session: &str,
    stop: impl Fn(&EventKind) -> bool,
) -> Vec<EventKind> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left).expect("event before timeout") {
            Forwarded::Event(event) if event.session_id == session => {
                let done = stop(&event.kind);
                seen.push(event.kind);
                if done {
                    return seen;
                }
            }
            Forwarded::Event(_) => {}
            Forwarded::UiAction(_) => {}
            Forwarded::Gap { .. } => panic!("unexpected gap"),
        }
    }
}

fn top_titles(v: &Vault) -> Vec<String> {
    v.list_children(None, 0, 200)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect()
}

#[test]
fn only_js_is_a_model_tool_and_models_include_the_default() {
    let t = temp();
    let agent = open(&t, vec![], vec![]);
    let models = agent.models();
    assert!(
        models.iter().any(|m| m.id == "scripted/scripted"),
        "{models:?}"
    );
    assert!(!models.iter().any(|m| m.id.starts_with("claude-code/")));
    assert_eq!(
        session_title(&"x\ny ".repeat(40)).chars().count(),
        "AI task: ".len() + 60
    );
}

#[test]
fn script_run_searches_and_creates_a_page_in_the_vault() {
    let t = temp();
    let agent = open(&t, vec![], vec![]);
    let session = agent.create_session("Script").unwrap();
    let run = agent
        .run_script(
            &session.id,
            "[script] create",
            r#"const hits = await tools.search_notes({query: "anything"});
               const made = await tools.create_page({title: "From the agent", markdown: "Hello from js"});
               return {hits: typeof hits, made: typeof made};"#,
        )
        .unwrap();
    assert_eq!(run.status, "completed", "{run:?}");
    let text = run.text.unwrap();
    assert!(text.contains("\"made\""), "{text}");
    assert!(top_titles(&t.vault).contains(&"From the agent".to_string()));
    let page = t
        .vault
        .list_children(None, 0, 200)
        .unwrap()
        .into_iter()
        .find(|p| p.title == "From the agent")
        .unwrap();
    assert!(t
        .vault
        .page_markdown(page.id)
        .unwrap()
        .contains("Hello from js"));
}

#[test]
fn deferred_vault_tools_are_found_with_search_and_callable() {
    let t = temp();
    t.vault.create_page(None, "Existing page".into()).unwrap();
    let agent = open(&t, vec![], vec![]);
    let session = agent.create_session("Deferred").unwrap();
    let run = agent
        .run_script(
            &session.id,
            "[script] deferred",
            r#"const found = await tools.$search("select:search_notes");
               const listing = await tools.search_notes({});
               const old = await tools.$search("select:list_pages");
               return {found: found.tools.map(t => t.name), listing, old: old.tools.map(t => t.name)};"#,
        )
        .unwrap();
    assert_eq!(run.status, "completed", "{run:?}");
    let value: Value = serde_json::from_str(&run.text.unwrap()).unwrap();
    assert_eq!(value["found"], json!(["search_notes"]), "{value}");
    // Old names are not registered in the in-app agent, so search cannot find them.
    assert_eq!(value["old"], json!([]), "{value}");
    assert!(
        value["listing"].as_str().unwrap().contains("Existing page"),
        "{value}"
    );
}

#[test]
fn model_run_streams_events_to_a_subscriber() {
    let t = temp();
    let agent = open(
        &t,
        vec![js(r#"return await tools.search_notes({query: "x"});"#)],
        vec![],
    );
    let events = subscribe(&agent);
    let session = agent.create_session("Chat").unwrap();
    let sent = agent.send(&session.id, "Find x", None).unwrap();
    assert!(matches!(sent, Sent::Started(_)));
    let seen = until(&events, &session.id, |k| {
        matches!(k, EventKind::RunFinished { .. })
    });
    assert!(matches!(seen[0], EventKind::RunStarted { .. }), "{seen:?}");
    assert!(seen
        .iter()
        .any(|k| matches!(k, EventKind::ToolStarted { call } if call.name == "js")));
    assert!(seen
        .iter()
        .any(|k| matches!(k, EventKind::ToolFinished { result, .. } if !result.is_error)));
    assert!(seen
        .iter()
        .any(|k| matches!(k, EventKind::Paragraph { text } if text == "All done.")));
    match seen.last().unwrap() {
        EventKind::RunFinished { status, text, .. } => {
            assert_eq!(status, "completed");
            assert_eq!(text.as_deref(), Some("All done."));
        }
        other => panic!("{other:?}"),
    }
}

/// A test-only tool that needs approval.
struct Dangerous(Arc<AtomicUsize>);

#[async_trait]
impl Tool for Dangerous {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "dangerous".into(),
            description: "Needs a person's approval".into(),
            input_schema: json!({"type": "object"}),
            read_only: false,
            approval: Approval::Confirm,
        }
    }
    async fn invoke(&self, _: ToolContext, _: Value) -> anyhow::Result<ToolResult> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(ToolResult::text("dangerous done"))
    }
}

#[test]
fn confirm_tool_waits_for_approval_and_busy_send_follows_up() {
    let t = temp();
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = open(
        &t,
        vec![js(r#"return await tools.dangerous({});"#)],
        vec![Arc::new(Dangerous(calls.clone()))],
    );
    let events = subscribe(&agent);
    let session = agent.create_session("Approval").unwrap();
    agent.send(&session.id, "Do it", None).unwrap();
    let seen = until(&events, &session.id, |k| {
        matches!(k, EventKind::ApprovalRequested { .. })
    });
    let EventKind::ApprovalRequested { approval } = seen.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(approval.tool, "dangerous");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let state = agent.session_state(&session.id).unwrap();
    assert_eq!(state.pending_approvals.len(), 1);

    // Busy: a send becomes a follow-up.
    let sent = agent.send(&session.id, "And then this", None).unwrap();
    assert!(matches!(sent, Sent::FollowUp(_)), "{sent:?}");

    let decided = agent
        .decide_approval(&approval.id, "approve", None)
        .unwrap();
    assert_eq!(decided.id, approval.id);
    let seen = until(&events, &session.id, |k| {
        matches!(k, EventKind::SessionSettled {})
    });
    assert!(seen
        .iter()
        .any(|k| matches!(k, EventKind::ApprovalResolved { .. })));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(seen
        .iter()
        .any(|k| matches!(k, EventKind::RunFinished { status, .. } if status == "completed")));
}

#[test]
fn store_survives_reopen_and_events_after_catches_up() {
    let t = temp();
    let (id, before, after) = {
        let agent = open(&t, vec![], vec![]);
        let session = agent.create_session("Kept").unwrap();
        let before = agent.events_after(0, None).unwrap().last_seq;
        let run = agent
            .run_script(&session.id, "[script] one", "return 1 + 1;")
            .unwrap();
        assert_eq!(run.status, "completed");
        let doomed = agent.create_session("Doomed").unwrap();
        agent
            .rename_session(&session.id, "Kept and renamed")
            .unwrap();
        agent.delete_session(&doomed.id).unwrap();
        (
            session.id,
            before,
            agent.events_after(0, None).unwrap().last_seq,
        )
    };
    assert!(after > before);
    let agent = open(&t, vec![], vec![]);
    let sessions = agent.sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, "Kept and renamed");
    let state = agent.session_state(&id).unwrap();
    assert!(state.messages.len() >= 3, "{:?}", state.messages);
    assert_eq!(state.runs.len(), 1);
    // BubblePaw reserves a fresh sequence block on reopen: sequences stay
    // monotonic across restarts, and a cursor from before the restart asks the
    // client to resync from session state (which holds the completed run above).
    assert!(state.position.seq >= after);
    let page = agent.events_after(before, Some(&id)).unwrap();
    assert!(page.resync);
    assert!(page.last_seq >= after);
    let page = agent.events_after(state.position.seq, Some(&id)).unwrap();
    assert!(!page.resync);
    assert!(page.events.is_empty());
}

#[test]
fn ai_task_runs_in_its_own_session_and_writes_back() {
    let t = temp();
    let v = &t.vault;
    let page = v.create_page(None, "Trip".into()).unwrap();
    v.insert_markdown(page.clone(), None, None, true, "Day one".into())
        .unwrap();
    let task = tasks::create(v, "Plan day two", &page, None, vault_agent::tasks::RUNNER).unwrap();
    let claimed = tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
    assert_eq!(claimed.id, task.id);

    let agent = open(
        &t,
        vec![js(r#"return await tools.read_page({page: "Trip"});"#)],
        vec![],
    );
    let run = agent.run_task(v, &task.id).unwrap();
    assert!(run.ok, "{run:?}");
    assert_eq!(run.text, "All done.");
    let done = tasks::get(v, &task.id).unwrap();
    assert_eq!(done.status, tasks::DONE);
    assert_eq!(done.runner, "agent");
    assert!(v.page_markdown(page).unwrap().contains("All done."));

    let session = agent.session_state(&run.session_id).unwrap();
    assert_eq!(session.session.title, "AI task: Plan day two");
    let prompt = &session.messages[0].content;
    assert!(
        prompt.contains("tools.$search") && !prompt.contains("MCP"),
        "{prompt}"
    );
}

/// Sends `()` when dropped.
struct DropSignal(mpsc::Sender<()>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn closing_event_streams_drops_each_forwarder_at_once() {
    let t = temp();
    let agent = open(&t, vec![], vec![]);
    let forward = |agent: &Agent| {
        let (tx, rx) = mpsc::channel();
        let signal = DropSignal(tx);
        let token = agent.forward_events(move |_| {
            let _keep = &signal;
            true
        });
        (token, rx)
    };

    // One stream, by its token: no event has to arrive first.
    let (token, rx) = forward(&agent);
    let started = Instant::now();
    token.cancel();
    rx.recv_timeout(Duration::from_secs(2))
        .expect("forwarder dropped");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );

    // Every stream at once.
    let (_a, rx_a) = forward(&agent);
    let (_b, rx_b) = forward(&agent);
    let started = Instant::now();
    agent.close_event_streams();
    rx_a.recv_timeout(Duration::from_secs(2))
        .expect("first dropped");
    rx_b.recv_timeout(Duration::from_secs(2))
        .expect("second dropped");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn test_confirm_tool_exists_only_with_its_flag_and_reports_a_denial() {
    let t = temp();
    let without = open(&t, vec![], vec![]);
    let session = without.create_session("No test tools").unwrap();
    let run = without
        .run_script(
            &session.id,
            "Try",
            "return await tools.test_confirm({text: 'x'});",
        )
        .unwrap();
    let state = without.session_state(&session.id).unwrap();
    assert!(
        state.messages.iter().any(|m| m.is_error) || run.status != "completed",
        "{run:?}"
    );
    drop(without);

    let t = temp();
    let agent = open_with(
        &t,
        vec![
            js(r#"return await tools.test_confirm({text: "first"});"#),
            js(r#"return await tools.test_confirm({text: "second"});"#),
        ],
        vec![],
        true,
    );
    let events = subscribe(&agent);
    let session = agent.create_session("Test tool").unwrap();
    agent.send(&session.id, "Go", None).unwrap();
    let approval_of = |seen: &[EventKind]| match seen.last().unwrap() {
        EventKind::ApprovalRequested { approval } => approval.clone(),
        other => panic!("{other:?}"),
    };
    let first = approval_of(&until(&events, &session.id, |k| {
        matches!(k, EventKind::ApprovalRequested { .. })
    }));
    assert_eq!(first.tool, "test_confirm");
    agent.decide_approval(&first.id, "approve", None).unwrap();
    let seen = until(&events, &session.id, |k| {
        matches!(k, EventKind::ApprovalRequested { .. })
    });
    assert!(seen.iter().any(|k| matches!(k,
        EventKind::ToolFinished { name, result, .. } if name == "test_confirm" && result.content.contains("confirmed: first"))),
        "{seen:?}");
    let second = approval_of(&seen);
    agent
        .decide_approval(&second.id, "deny", Some("Not today".into()))
        .unwrap();
    let seen = until(&events, &session.id, |k| {
        matches!(k, EventKind::RunFinished { .. })
    });
    assert!(seen.iter().any(|k| matches!(k,
        EventKind::ToolFinished { name, result, .. } if name == "js" && result.content.contains("Not today"))),
        "{seen:?}");
}
