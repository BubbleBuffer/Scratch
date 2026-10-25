//! Delegated todo lists mirror into the page as nested todos. These tests run
//! real task runs through the scripted provider: todo calls interleaved with
//! fixture edits and checks, in JS and native runs. Fixture polls are bounded;
//! a failed poll is recorded, and the test asserts on what was recorded.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use vault_agent::bubblepaw_sdk::approvals::{Approval};
use vault_agent::bubblepaw_sdk::events::{EventKind};
use vault_agent::bubblepaw_sdk::harness::{ExecutionMode};
use vault_agent::bubblepaw_sdk::store::{Role};
use vault_agent::bubblepaw_sdk::tools::{Tool, ToolCall, ToolContext, ToolDefinition, ToolResult};
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::{Agent, AgentConfig, PermissionMode, TestHooks, ToolPack};
use vault_core::{new_id, Block, BlockKind, BlockOp, PropValue, Vault};
use vault_mcp::tasks::{self, Task};
use vault_tools::ToolSet;

/// What the fixtures saw: `{"tool": name, "result": value}` in call order.
type Probe = Arc<Mutex<Vec<Value>>>;

/// One step of a run: a tool name and its arguments.
struct Call {
    name: &'static str,
    args: Value,
}

fn call(name: &'static str, args: Value) -> Call {
    Call { name, args }
}

/// The todo tool takes the whole list as `{todos: [...]}`; the state it saves
/// is the bare array.
fn todo(items: Vec<Value>) -> Call {
    call("todo", json!({"todos": items}))
}

fn item(text: &str, status: &str) -> Value {
    json!({"text": text, "status": status})
}

fn checked(text: &str, checked: bool) -> Value {
    json!({"text": text, "checked": checked})
}

fn vault(dir: &tempfile::TempDir) -> Arc<Vault> {
    Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap())
}

/// A page with `markdown` as its blocks; returns the page id.
fn page_with(v: &Vault, title: &str, markdown: &str) -> String {
    let page = v.create_page(None, title.into()).unwrap();
    v.insert_markdown(page.clone(), None, None, true, markdown.into())
        .unwrap();
    page
}

/// Id of the first block with `text` in the page.
fn block_id(v: &Vault, page: &str, text: &str) -> String {
    let doc = v.load_page(page.into()).unwrap();
    find_text(&doc.blocks, text)
        .unwrap_or_else(|| panic!("no block {text:?}"))
        .id
        .clone()
}

fn find_text<'a>(blocks: &'a [Block], text: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|b| {
        if b.text == text {
            Some(b)
        } else {
            find_text(&b.children, text)
        }
    })
}

fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|b| {
        if b.id == id {
            Some(b)
        } else {
            find(&b.children, id)
        }
    })
}

/// A task asked after `after` (a block of `page`), claimed by this device.
fn claim(v: &Vault, page: &str, after: Option<&str>) -> Task {
    let task = tasks::create(v, "Plan the report", &page.to_string(), after, "agent").unwrap();
    assert_eq!(
        tasks::claim_next(v, &v.device_id()).unwrap().unwrap().id,
        task.id
    );
    task
}

fn one(name: &str, arguments: Value) -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: new_id(),
            name: name.into(),
            arguments,
        }],
    }
}

fn js(code: &str) -> ScriptedStep {
    one("js", json!({"code": code}))
}

/// The provider steps for `calls`. A native run makes one tool call per step;
/// a JS run makes the same calls in one cell.
fn script(mode: ExecutionMode, calls: &[Call]) -> Vec<ScriptedStep> {
    if mode == ExecutionMode::Js {
        let code: String = calls
            .iter()
            .map(|c| format!("await tools.{}({});\n", c.name, c.args))
            .collect();
        return vec![js(&code)];
    }
    calls.iter().map(|c| one(c.name, c.args.clone())).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Fixture {
    Mirror,
    Edit,
    Record,
}

impl Fixture {
    fn name(self) -> &'static str {
        match self {
            Fixture::Mirror => "test_mirror",
            Fixture::Edit => "test_edit",
            Fixture::Record => "test_record",
        }
    }
}

/// A fixture tool bound to one task. `test_mirror` polls the mirrored list,
/// `test_edit` changes the page like a user would, and `test_record` keeps
/// its input so a test can read what the run saw.
struct Hook {
    fixture: Fixture,
    vault: Arc<Vault>,
    task: String,
    probe: Probe,
}

fn fixtures(v: &Arc<Vault>, task: &str, probe: &Probe) -> Vec<Arc<dyn Tool>> {
    [Fixture::Mirror, Fixture::Edit, Fixture::Record]
        .into_iter()
        .map(|fixture| {
            Arc::new(Hook {
                fixture,
                vault: v.clone(),
                task: task.into(),
                probe: probe.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

#[async_trait]
impl Tool for Hook {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.fixture.name().into(),
            description: format!("Test fixture {}.", self.fixture.name()),
            input_schema: json!({"type": "object"}),
            read_only: self.fixture != Fixture::Edit,
            approval: Approval::None,
        }
    }

    async fn invoke(&self, _: ToolContext, input: Value) -> anyhow::Result<ToolResult> {
        let result = match self.fixture {
            Fixture::Record => input,
            Fixture::Edit => {
                let (vault, task) = (self.vault.clone(), self.task.clone());
                tokio::task::spawn_blocking(move || edit(&vault, &task, &input)).await??;
                json!("edited")
            }
            Fixture::Mirror => mirror(self.vault.clone(), self.task.clone(), input).await,
        };
        self.probe
            .lock()
            .unwrap()
            .push(json!({"tool": self.fixture.name(), "result": result}));
        Ok(ToolResult::text(result.to_string()))
    }
}

/// What the page shows for the task: the todos under the source (or the
/// placeholder), the other children there, and the source and placeholder.
fn observe(v: &Vault, task: &str) -> Value {
    let Ok(t) = tasks::get(v, task) else {
        return json!({"missing": "task"});
    };
    let Some(page) = t.page.clone() else {
        return json!({"missing": "page"});
    };
    let Ok(doc) = v.load_page(page) else {
        return json!({"missing": "page"});
    };
    let source = t
        .source_block
        .as_deref()
        .and_then(|id| find(&doc.blocks, id));
    let placeholder = t.block.as_deref().and_then(|id| find(&doc.blocks, id));
    let children = source
        .or(placeholder)
        .map(|b| b.children.as_slice())
        .unwrap_or_default();
    json!({
        "todos": children
            .iter()
            .filter(|c| c.kind == BlockKind::Todo)
            .map(|c| checked(&c.text, c.checked))
            .collect::<Vec<_>>(),
        "other": children
            .iter()
            .filter(|c| c.kind != BlockKind::Todo)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>(),
        "source": source.map(|b| checked(&b.text, b.checked)),
        "placeholder": placeholder.map(|b| b.text.clone()),
    })
}

/// The observed todos, polled (bounded) until they equal `expect` when one
/// is given. The result says whether the poll matched.
async fn mirror(vault: Arc<Vault>, task: String, input: Value) -> Value {
    let expect = input.get("expect").filter(|e| !e.is_null()).cloned();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (v, t) = (vault.clone(), task.clone());
        let seen = tokio::task::spawn_blocking(move || observe(&v, &t))
            .await
            .unwrap();
        let matched = match &expect {
            Some(want) => seen["todos"] == *want,
            None => true,
        };
        if matched || Instant::now() >= deadline {
            return json!({"matched": matched, "seen": seen});
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Changes the page the way a user would.
fn edit(vault: &Vault, task: &str, input: &Value) -> anyhow::Result<()> {
    let t = tasks::get(vault, task).map_err(anyhow::Error::msg)?;
    let page = t
        .page
        .clone()
        .ok_or_else(|| anyhow::anyhow!("the task has no page"))?;
    let text = input["text"].as_str().unwrap_or_default();
    match input["op"].as_str().unwrap_or_default() {
        "delete_child" => {
            let doc = vault.load_page(page.clone())?;
            let container = t
                .source_block
                .as_deref()
                .or(t.block.as_deref())
                .unwrap_or_default();
            let child = find(&doc.blocks, container)
                .into_iter()
                .flat_map(|b| b.children.iter())
                .find(|c| c.kind == BlockKind::Todo && c.text == text)
                .ok_or_else(|| anyhow::anyhow!("no mirrored item {text:?}"))?;
            vault.apply(
                page,
                vec![BlockOp::Delete {
                    id: child.id.clone(),
                }],
            )?;
        }
        "add_user_child" => {
            let parent = if input["where"] == "source" {
                t.source_block.clone()
            } else {
                t.block.clone()
            };
            vault.insert_markdown(page, parent, None, false, text.into())?;
        }
        "delete_source" => {
            let source = t
                .source_block
                .clone()
                .ok_or_else(|| anyhow::anyhow!("the task has no source block"))?;
            vault.apply(page, vec![BlockOp::Delete { id: source }])?;
        }
        "set_property" => {
            let key = input["key"].as_str().unwrap_or_default();
            let value = input["value"].as_str().unwrap_or_default();
            vault.set_property(task.into(), key.into(), PropValue::Text(value.into()))?;
        }
        other => anyhow::bail!("unknown test edit {other:?}"),
    }
    Ok(())
}

fn open(
    root: &std::path::Path,
    v: Arc<Vault>,
    mode: ExecutionMode,
    steps: Vec<ScriptedStep>,
    extra: Vec<Arc<dyn Tool>>,
    tool_set: Option<ToolSet>,
) -> Agent {
    Agent::open_with_execution(
        v,
        AgentConfig {
            data_dir: root.join("data"),
            default_model: "scripted/scripted".into(),
            bun_path: Some(root.join("no-bun")),
            permission_mode: PermissionMode::Manual,
            packs: vec![ToolPack::Desktop { root: root.into() }],
            tool_set,
            test: TestHooks {
                scripted: Some((steps, "Finished reading.".into())),
                extra_tools: extra,
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
        mode,
    )
    .unwrap()
}

/// Every tool result the run got back was a success.
fn assert_clean(agent: &Agent, session: &str) {
    let state = agent.session_state(session).unwrap();
    let errors: Vec<&str> = state
        .messages
        .iter()
        .filter(|m| m.role == Role::Tool && m.is_error)
        .map(|m| m.content.as_str())
        .collect();
    assert!(errors.is_empty(), "tool errors: {errors:?}");
}

/// The lists the todo tool saved in the run, in order.
fn saved_lists(agent: &Agent, session: &str) -> Vec<Value> {
    agent
        .events_after(0, Some(session))
        .unwrap()
        .events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::HostStateChanged { key, value } if key == "todo" => Some(value),
            _ => None,
        })
        .collect()
}

fn observed(probe: &Probe, tool: &str) -> Vec<Value> {
    probe
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e["tool"] == tool)
        .map(|e| e["result"].clone())
        .collect()
}

#[test]
fn a_source_todo_mirrors_each_todo_call_in_js_and_native_runs() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir);
        let page = page_with(&v, "Plan", "- [ ] Write the report\n\nBackground");
        let source = block_id(&v, &page, "Write the report");
        let task = claim(&v, &page, Some(&source));
        assert_eq!(task.source_block.as_deref(), Some(source.as_str()));
        let calls = [
            todo(vec![
                item("Outline", "pending"),
                item("Draft", "in_progress"),
            ]),
            call(
                "test_mirror",
                json!({"expect": [checked("Outline", false), checked("Draft", false)]}),
            ),
            // Reorder, change statuses and add an item in one replacement.
            todo(vec![
                item("Draft", "done"),
                item("Outline", "done"),
                item("Cite sources", "pending"),
            ]),
            call(
                "test_mirror",
                json!({"expect": [
                    checked("Draft", true),
                    checked("Outline", true),
                    checked("Cite sources", false),
                ]}),
            ),
            // Text edit: the leftover item keeps its place and takes the new text.
            todo(vec![
                item("Draft", "done"),
                item("Outline", "done"),
                item("Cite the sources", "pending"),
            ]),
            call(
                "test_mirror",
                json!({"expect": [
                    checked("Draft", true),
                    checked("Outline", true),
                    checked("Cite the sources", false),
                ]}),
            ),
            // Removals: the two finished items go, the rest stays.
            todo(vec![item("Cite the sources", "done")]),
            call(
                "test_mirror",
                json!({"expect": [checked("Cite the sources", true)]}),
            ),
            // Not polled: the drain before the run ends must apply it.
            todo(vec![]),
        ];
        let probe = Probe::default();
        let agent = open(
            dir.path(),
            v.clone(),
            mode,
            script(mode, &calls),
            fixtures(&v, &task.id, &probe),
            None,
        );
        let run = agent.run_task(&v, &task.id).unwrap();
        assert!(run.ok, "{mode:?}: {run:?}");
        assert_clean(&agent, &run.session_id);
        assert_eq!(saved_lists(&agent, &run.session_id).len(), 5);

        let seen = observed(&probe, "test_mirror");
        assert_eq!(seen.len(), 4, "{mode:?}: {seen:?}");
        let placeholder = seen[0]["seen"]["placeholder"].clone();
        for s in &seen {
            assert_eq!(s["matched"], json!(true), "{mode:?}: {s}");
            // The source todo and the placeholder stay as they were until
            // the run finishes.
            assert_eq!(
                s["seen"]["source"],
                checked("Write the report", false),
                "{s}"
            );
            assert_eq!(s["seen"]["placeholder"], placeholder, "{s}");
        }

        assert_eq!(tasks::get(&v, &task.id).unwrap().status, tasks::DONE);
        let doc = v.load_page(page.clone()).unwrap();
        let source_now = find(&doc.blocks, &source).unwrap();
        assert_eq!(
            (
                source_now.text.as_str(),
                source_now.checked,
                source_now.children.len()
            ),
            ("Write the report", false, 0)
        );
        assert!(v
            .blocks_markdown(doc.blocks.clone())
            .contains("Finished reading."));
    }
}

#[test]
fn a_nested_source_todo_gets_the_subtasks_and_its_siblings_stay_put() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir);
        let page = page_with(
            &v,
            "Outline",
            "- Outline\n  - [ ] Nested step\n  - Sibling note",
        );
        let source = block_id(&v, &page, "Nested step");
        let before = v.load_page(page.clone()).unwrap();
        let sibling = find_text(&before.blocks, "Sibling note").unwrap().clone();
        let task = claim(&v, &page, Some(&source));
        assert_eq!(task.source_block.as_deref(), Some(source.as_str()));
        let calls = [
            todo(vec![item("Find data", "pending")]),
            call(
                "test_mirror",
                json!({"expect": [checked("Find data", false)]}),
            ),
            todo(vec![item("Find data", "done"), item("Write up", "pending")]),
            call(
                "test_mirror",
                json!({"expect": [checked("Find data", true), checked("Write up", false)]}),
            ),
        ];
        let probe = Probe::default();
        let agent = open(
            dir.path(),
            v.clone(),
            mode,
            script(mode, &calls),
            fixtures(&v, &task.id, &probe),
            None,
        );
        let run = agent.run_task(&v, &task.id).unwrap();
        assert!(run.ok, "{mode:?}: {run:?}");
        assert_clean(&agent, &run.session_id);
        for s in observed(&probe, "test_mirror") {
            assert_eq!(s["matched"], json!(true), "{mode:?}: {s}");
        }

        let doc = v.load_page(page.clone()).unwrap();
        let nested = find(&doc.blocks, &source).unwrap();
        let subtasks: Vec<(&str, bool)> = nested
            .children
            .iter()
            .map(|c| (c.text.as_str(), c.checked))
            .collect();
        assert_eq!(subtasks, vec![("Find data", true), ("Write up", false)]);
        let outline = find_text(&doc.blocks, "Outline").unwrap();
        let texts: Vec<&str> = outline.children.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Nested step", "Finished reading.", "Sibling note"]
        );
        assert_eq!(find(&doc.blocks, &sibling.id).unwrap(), &sibling);
    }
}

#[test]
fn a_task_without_a_source_todo_nests_under_its_placeholder_and_its_result_follows_it() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir);
        let page = page_with(&v, "Notes", "Plain paragraph");
        let after = block_id(&v, &page, "Plain paragraph");
        let task = claim(&v, &page, Some(&after));
        assert_eq!(task.source_block, None);
        let calls = [
            call(
                "test_edit",
                json!({"op": "add_user_child", "where": "placeholder", "text": "My own note"}),
            ),
            todo(vec![item("Read", "pending"), item("Summarize", "pending")]),
            call(
                "test_mirror",
                json!({"expect": [checked("Read", false), checked("Summarize", false)]}),
            ),
            todo(vec![item("Read", "done"), item("Summarize", "in_progress")]),
            call(
                "test_mirror",
                json!({"expect": [checked("Read", true), checked("Summarize", false)]}),
            ),
        ];
        let probe = Probe::default();
        let agent = open(
            dir.path(),
            v.clone(),
            mode,
            script(mode, &calls),
            fixtures(&v, &task.id, &probe),
            None,
        );
        let run = agent.run_task(&v, &task.id).unwrap();
        assert!(run.ok, "{mode:?}: {run:?}");
        assert_clean(&agent, &run.session_id);

        let seen = observed(&probe, "test_mirror");
        assert_eq!(seen.len(), 2, "{mode:?}: {seen:?}");
        for s in &seen {
            assert_eq!(s["matched"], json!(true), "{mode:?}: {s}");
            assert_eq!(s["seen"]["source"], Value::Null, "{s}");
            // The user's own child is not a mirror and stays untouched.
            assert_eq!(s["seen"]["other"], json!(["My own note"]), "{s}");
        }

        // The placeholder keeps its children, is labelled done, and the
        // final reply goes right after it.
        let doc = v.load_page(page.clone()).unwrap();
        let placeholder = task.block.clone().unwrap();
        let at = doc.blocks.iter().position(|b| b.id == placeholder).unwrap();
        assert!(
            doc.blocks[at].text.starts_with("AI task done"),
            "{}",
            doc.blocks[at].text
        );
        assert_eq!(doc.blocks[at + 1].text, "Finished reading.");
        let kept = observe(&v, &task.id);
        assert_eq!(
            kept["todos"],
            json!([checked("Read", true), checked("Summarize", false)])
        );
        assert_eq!(kept["other"], json!(["My own note"]));
    }
}

#[test]
fn reconcile_keeps_user_children_and_never_recreates_a_mirror_the_user_deleted() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir);
        let page = page_with(&v, "Sources", "- [ ] Gather sources");
        let source = block_id(&v, &page, "Gather sources");
        let task = claim(&v, &page, Some(&source));
        let calls = [
            todo(vec![item("First", "pending"), item("Second", "pending")]),
            call(
                "test_mirror",
                json!({"expect": [checked("First", false), checked("Second", false)]}),
            ),
            call(
                "test_edit",
                json!({"op": "add_user_child", "where": "source", "text": "My own note"}),
            ),
            call("test_edit", json!({"op": "delete_child", "text": "First"})),
            todo(vec![
                item("First", "pending"),
                item("Second", "done"),
                item("Third", "pending"),
            ]),
            // "First" stays deleted: the list still names it.
            call(
                "test_mirror",
                json!({"expect": [checked("Second", true), checked("Third", false)]}),
            ),
            todo(vec![item("First", "done"), item("Second", "done")]),
            call("test_mirror", json!({"expect": [checked("Second", true)]})),
        ];
        let probe = Probe::default();
        let agent = open(
            dir.path(),
            v.clone(),
            mode,
            script(mode, &calls),
            fixtures(&v, &task.id, &probe),
            None,
        );
        let run = agent.run_task(&v, &task.id).unwrap();
        assert!(run.ok, "{mode:?}: {run:?}");
        assert_clean(&agent, &run.session_id);

        let seen = observed(&probe, "test_mirror");
        assert_eq!(seen.len(), 3, "{mode:?}: {seen:?}");
        for s in &seen {
            assert_eq!(s["matched"], json!(true), "{mode:?}: {s}");
        }
        assert_eq!(
            seen[2]["seen"]["other"],
            json!(["My own note"]),
            "{mode:?}: {seen:?}"
        );
        let doc = v.load_page(page.clone()).unwrap();
        let source_now = find(&doc.blocks, &source).unwrap();
        let texts: Vec<&str> = source_now
            .children
            .iter()
            .map(|c| c.text.as_str())
            .collect();
        assert!(texts.contains(&"My own note"), "{texts:?}");
        assert!(!texts.contains(&"First"), "{texts:?}");
    }
}

/// Run one, then retry: the second run's mirror must match `expect`.
fn retry_case(mode: ExecutionMode, first: Vec<Value>, second: Vec<Value>, expect: Value) {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let page = page_with(&v, "Sources", "- [ ] Gather sources");
    let source = block_id(&v, &page, "Gather sources");
    let task = claim(&v, &page, Some(&source));
    let probe = Probe::default();
    let mut steps = script(mode, &[todo(first)]);
    steps.push(ScriptedStep::Text {
        text: "First run done.".into(),
    });
    steps.extend(script(
        mode,
        &[todo(second), call("test_mirror", json!({"expect": expect}))],
    ));
    let agent = open(
        dir.path(),
        v.clone(),
        mode,
        steps,
        fixtures(&v, &task.id, &probe),
        None,
    );
    let first_run = agent.run_task(&v, &task.id).unwrap();
    assert!(first_run.ok, "{mode:?}: {first_run:?}");
    tasks::retry(&v, &task.id).unwrap();
    assert_eq!(
        tasks::claim_next(&v, &v.device_id()).unwrap().unwrap().id,
        task.id
    );
    let second_run = agent.run_task(&v, &task.id).unwrap();
    assert!(second_run.ok, "{mode:?}: {second_run:?}");
    assert_clean(&agent, &second_run.session_id);
    let seen = observed(&probe, "test_mirror");
    assert_eq!(seen.len(), 1, "{mode:?}: {seen:?}");
    assert_eq!(seen[0]["matched"], json!(true), "{mode:?}: {}", seen[0]);
}

#[test]
fn a_retry_starts_fresh_and_keeps_checked_mirrors_from_the_run_before() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        // Nothing checked: the old mirrors go, and only the new list is left.
        retry_case(
            mode,
            vec![item("First", "pending"), item("Second", "pending")],
            vec![item("Third", "pending")],
            json!([checked("Third", false)]),
        );
        // Something checked: the old mirrors stay and the new ones follow.
        retry_case(
            mode,
            vec![item("First", "done"), item("Second", "pending")],
            vec![item("Third", "pending")],
            json!([
                checked("First", true),
                checked("Second", false),
                checked("Third", false),
            ]),
        );
    }
}

#[test]
fn a_deleted_source_block_does_not_fail_the_run_or_come_back() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir);
        let page = page_with(&v, "Gone soon", "- [ ] Gather sources");
        let source = block_id(&v, &page, "Gather sources");
        let task = claim(&v, &page, Some(&source));
        let calls = [
            todo(vec![item("First", "pending")]),
            // The mirror exists before the source goes.
            call("test_mirror", json!({"expect": [checked("First", false)]})),
            call("test_edit", json!({"op": "delete_source"})),
            // The mirror cannot write into the missing block; the run goes on.
            todo(vec![item("First", "done"), item("Second", "pending")]),
        ];
        let probe = Probe::default();
        let agent = open(
            dir.path(),
            v.clone(),
            mode,
            script(mode, &calls),
            fixtures(&v, &task.id, &probe),
            None,
        );
        let run = agent.run_task(&v, &task.id).unwrap();
        assert!(run.ok, "{mode:?}: {run:?}");
        assert_clean(&agent, &run.session_id);
        // Both replacements were saved; the mirror could not write them.
        assert_eq!(
            saved_lists(&agent, &run.session_id),
            vec![
                json!([item("First", "pending")]),
                json!([item("First", "done"), item("Second", "pending")]),
            ],
            "{mode:?}"
        );
        assert_eq!(tasks::get(&v, &task.id).unwrap().status, tasks::DONE);
        // The deletion holds: nothing brought the source back.
        let doc = v.load_page(page.clone()).unwrap();
        assert!(
            find_text(&doc.blocks, "Gather sources").is_none(),
            "{mode:?}"
        );
    }
}

#[test]
fn a_malformed_tracking_property_does_not_fail_the_run() {
    for mode in [ExecutionMode::Js, ExecutionMode::Native] {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir);
        let page = page_with(&v, "Broken tracking", "- [ ] Gather sources");
        let source = block_id(&v, &page, "Gather sources");
        let task = claim(&v, &page, Some(&source));
        let calls = [
            call(
                "test_edit",
                json!({"op": "set_property", "key": "subtask_blocks", "value": "{not json"}),
            ),
            todo(vec![item("First", "pending")]),
            todo(vec![item("First", "done")]),
        ];
        let probe = Probe::default();
        let agent = open(
            dir.path(),
            v.clone(),
            mode,
            script(mode, &calls),
            fixtures(&v, &task.id, &probe),
            None,
        );
        let run = agent.run_task(&v, &task.id).unwrap();
        assert!(run.ok, "{mode:?}: {run:?}");
        assert_clean(&agent, &run.session_id);
        assert_eq!(
            saved_lists(&agent, &run.session_id),
            vec![
                json!([item("First", "pending")]),
                json!([item("First", "done")]),
            ],
            "{mode:?}"
        );
        // The malformed state blocks every mirror write: no children appear.
        let doc = v.load_page(page.clone()).unwrap();
        let source_now = find(&doc.blocks, &source).unwrap();
        assert!(source_now.children.is_empty(), "{mode:?}: {source_now:?}");
        assert_eq!(tasks::get(&v, &task.id).unwrap().status, tasks::DONE);
    }
}

#[test]
fn only_delegated_runs_see_or_call_todo_and_the_gateway_omits_it() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let page = page_with(&v, "Plan", "Paragraph");
    let task = claim(&v, &page, None);
    let probe = Probe::default();
    let task_code = r#"
        const found = await tools.$search("select:todo");
        await tools.test_record({names: found.tools.map(t => t.name)});
        await tools.todo({todos: [{text: "Found it", status: "pending"}]});
    "#;
    let chat_code = r#"
        const found = await tools.$search("select:todo");
        let rejected = false;
        try { await tools.todo({todos: []}); } catch (_) { rejected = true; }
        return {names: found.tools.map(t => t.name), rejected};
    "#;
    let agent = open(
        dir.path(),
        v.clone(),
        ExecutionMode::Js,
        vec![
            js(task_code),
            ScriptedStep::Text {
                text: "Found.".into(),
            },
        ],
        fixtures(&v, &task.id, &probe),
        None,
    );

    let run = agent.run_task(&v, &task.id).unwrap();
    assert!(run.ok, "{run:?}");
    assert_clean(&agent, &run.session_id);
    let seen = observed(&probe, "test_record");
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0]["names"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "todo"),
        "{seen:?}"
    );
    assert_eq!(
        saved_lists(&agent, &run.session_id),
        vec![json!([item("Found it", "pending")])]
    );

    // A normal chat finds no todo tool and cannot call it.
    let session = agent.create_session("Normal chat").unwrap();
    let chat = agent
        .run_script(&session.id, "Find tools", chat_code)
        .unwrap();
    assert_eq!(chat.status, "completed");
    let result: Value = serde_json::from_str(&chat.text.unwrap()).unwrap();
    assert_eq!(result, json!({"names": [], "rejected": true}));
    assert!(saved_lists(&agent, &session.id).is_empty());

    // The gateway's JS help never lists it.
    let help = agent.gateway_js_help(None);
    assert!(!help.contains("tools.todo("), "{help}");
}

#[test]
fn the_read_tool_set_keeps_todo_for_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let page = page_with(&v, "Read only", "- [ ] Gather sources");
    let source = block_id(&v, &page, "Gather sources");
    let task = claim(&v, &page, Some(&source));
    let calls = [
        todo(vec![item("First", "pending")]),
        call("test_mirror", json!({"expect": [checked("First", false)]})),
    ];
    let probe = Probe::default();
    let agent = open(
        dir.path(),
        v.clone(),
        ExecutionMode::Native,
        script(ExecutionMode::Native, &calls),
        fixtures(&v, &task.id, &probe),
        Some(ToolSet::Read),
    );
    let run = agent.run_task(&v, &task.id).unwrap();
    assert!(run.ok, "{run:?}");
    assert_clean(&agent, &run.session_id);
    let seen = observed(&probe, "test_mirror");
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0]["matched"], json!(true), "{}", seen[0]);
    assert_eq!(saved_lists(&agent, &run.session_id).len(), 1);
}
