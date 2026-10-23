//! Desktop capabilities are optional, confined for files, and approved for commands.
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vault_agent::bubblepaw_core::{EventKind, ToolRegistry};
use vault_agent::{Agent, AgentConfig, Forwarded, HostKind, PermissionMode, ToolPack};
use vault_core::Vault;

// Every test in this file runs under this one lock. Dropping an Agent does not always
// release the store's file lock yet (BubblePaw), so reopening a data directory under
// parallel load fails with "Database is already owned by another process". Remove this
// once the fix lands: wiki/integrations/agent.md, "Store lock outlives Agent drop (BubblePaw)".
static SERIAL: Mutex<()> = Mutex::new(());

fn serial_guard() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    vault: Arc<Vault>,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("computer");
        std::fs::create_dir(&root).unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        Self {
            _dir: dir,
            root,
            vault,
            data,
        }
    }

    fn open(&self, desktop: bool, permission_mode: PermissionMode) -> Agent {
        self.open_in(self.data.clone(), desktop, permission_mode)
    }

    fn open_in(&self, data: PathBuf, desktop: bool, permission_mode: PermissionMode) -> Agent {
        Agent::open(
            self.vault.clone(),
            AgentConfig {
                data_dir: data,
                packs: if desktop {
                    vec![ToolPack::Desktop {
                        root: self.root.clone(),
                    }]
                } else {
                    vec![]
                },
                permission_mode,
                ..AgentConfig::default()
            },
        )
        .unwrap()
    }
}

fn script(agent: &Agent, code: &str) -> Value {
    let session = agent.create_session("Desktop test").unwrap();
    let run = agent
        .run_script(&session.id, "Test desktop tools", code)
        .unwrap();
    assert_eq!(run.status, "completed", "{run:?}");
    serde_json::from_str(&run.text.unwrap()).unwrap()
}

fn echo_command() -> &'static str {
    if cfg!(windows) {
        "cmd /c echo hi"
    } else {
        "echo hi"
    }
}

fn echo_script() -> String {
    format!(
        "const p = await tools.exec({{command: {}, yieldMs: 0}}); await wait({{handle: p.handle}}); return await tools.poll({{handle: p.handle}});",
        json!(echo_command()),
    )
}

fn assert_host_tool_names(names: &[String], desktop: bool) {
    use std::collections::BTreeSet;
    let actual: BTreeSet<_> = names.iter().map(String::as_str).collect();
    assert_eq!(
        actual.len(),
        names.len(),
        "duplicate registered names: {names:?}"
    );
    let mut expected = BTreeSet::from([
        "update_task",
        "notes_help",
        "daily_note",
        "search_notes",
        "read_page",
        "create_page",
        "edit_page",
        "upsert_row",
        "trash_page",
        "move_page",
        "memory_read",
        "memory_write",
        "memory_update",
        "memory_delete",
        "feedback_submit",
        "chat_read",
        "ui_open_page",
        "ui_highlight",
        "history_chapters",
        "history_search",
        "history_read",
        "environments",
        "js",
    ]);
    if desktop {
        expected.extend([
            "read",
            "write",
            "edit",
            "search",
            "list",
            "exec",
            "poll",
            "wait",
            "cancel",
            "project_list",
            "project_current",
            "project_switch",
            "project_open",
        ]);
    } else {
        for name in ["read", "write", "edit", "search", "list", "exec"] {
            assert!(!actual.contains(name), "base host must not register {name}");
        }
    }
    assert_eq!(actual, expected);
    assert!(!names.iter().any(|name| name.starts_with("fs_")));
}

#[test]
fn desktop_searches_files_and_notes_separately_and_records_file_summaries() {
    let _serial = serial_guard();
    let f = Fixture::new();
    std::fs::write(f.root.join("inside.txt"), "sharedneedle file-only-marker").unwrap();
    std::fs::write(
        f.root.parent().unwrap().join("outside.txt"),
        "sharedneedle outside-marker",
    )
    .unwrap();
    let page = f
        .vault
        .create_page(None, "Sharedneedle note".into())
        .unwrap();
    f.vault
        .insert_markdown(
            page,
            None,
            None,
            true,
            "sharedneedle notes-only-marker".into(),
        )
        .unwrap();
    let agent = f.open(true, PermissionMode::Bypass);
    let session = agent.create_session("Distinct searches").unwrap();
    let value = in_session(
        &agent,
        &session.id,
        r#"
        const files = await tools.search({query:'sharedneedle'});
        const notes = await tools.search_notes({query:'sharedneedle'});
        const read = await tools.read({path:'inside.txt'});
        await tools.edit({path:'inside.txt',old_string:'file-only-marker',new_string:'edited-marker'});
        await tools.write({path:'created.txt',content:'created'});
        await tools.list({path:'.'});
        let outsideError = '';
        try { await tools.search({path:'../outside.txt',query:'sharedneedle'}); }
        catch(e) { outsideError = String(e); }
        return {files, notes, outsideError};
    "#,
    );
    assert_eq!(
        value["files"]["matches"].as_array().unwrap().len(),
        1,
        "{value}"
    );
    assert_eq!(value["files"]["matches"][0]["path"], "inside.txt");
    assert!(value["files"]["matches"][0]["text"]
        .as_str()
        .unwrap()
        .contains("file-only-marker"));
    let notes = value["notes"].as_str().unwrap();
    assert!(
        notes.contains("Sharedneedle note") && notes.contains("notes-only-marker"),
        "{value}"
    );
    assert!(!notes.contains("file-only-marker") && !notes.contains("outside-marker"));
    assert!(
        value["outsideError"]
            .as_str()
            .unwrap()
            .contains("outside project \"Home\""),
        "{value}"
    );
    let state = agent.session_state(&session.id).unwrap();
    assert!(
        state.edits.is_empty(),
        "file changes must not create vault edit records"
    );
    let summary = |name: &str| {
        state
            .activity
            .iter()
            .find(|a| a.name == name)
            .unwrap()
            .summary
            .as_str()
    };
    assert_eq!(summary("search_notes"), "Searched notes “sharedneedle”");
    assert_eq!(summary("search"), "Searched files “sharedneedle”");
    assert_eq!(summary("read"), "Read inside.txt");
    assert_eq!(summary("edit"), "Edited inside.txt");
    assert_eq!(summary("write"), "Edited created.txt");
    assert_eq!(summary("list"), "Listed .");
    assert_eq!(
        std::fs::read_to_string(f.root.join("inside.txt")).unwrap(),
        "sharedneedle edited-marker"
    );
}

#[test]
fn desktop_files_stay_inside_the_root() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Bypass);
    let value = script(&agent, "await tools.write({path:'inside.txt',content:'hello'}); return await tools.read({path:'inside.txt'});");
    assert_eq!(value["content"], "hello");
    std::fs::write(f.root.parent().unwrap().join("outside.txt"), "outside").unwrap();
    let outside = script(
        &agent,
        r#"
        let readError = '', writeError = '';
        try { await tools.read({path:'../outside.txt'}); } catch(e) { readError = String(e); }
        try { await tools.write({path:'../new.txt',content:'escape'}); } catch(e) { writeError = String(e); }
        return {readError, writeError};
    "#,
    );
    assert!(
        outside["readError"]
            .as_str()
            .unwrap()
            .contains("outside project \"Home\""),
        "{outside}"
    );
    assert!(
        outside["writeError"]
            .as_str()
            .unwrap()
            .contains("outside project \"Home\""),
        "{outside}"
    );
    assert!(!f.root.parent().unwrap().join("new.txt").exists());
}

#[test]
fn desktop_tools_are_always_shown_with_the_host_approval_policy() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let projects = Arc::new(
        vault_agent::projects::Projects::open(f.data.join("projects.json"), &f.root).unwrap(),
    );
    let mut registry = ToolRegistry::new();
    let _processes =
        vault_agent::tools::register_desktop_tools(&mut registry, projects, &f.data).unwrap();
    assert!(registry.get("agent").is_none());
    for definition in registry.capabilities() {
        assert!(registry.is_shown_always(&definition.name));
        let needs_approval = matches!(
            definition.name.as_str(),
            "write" | "edit" | "exec" | "project_switch" | "project_open"
        );
        assert_eq!(
            definition.approval == vault_agent::bubblepaw_core::Approval::Confirm,
            needs_approval
        );
    }
    assert!(registry
        .get("exec")
        .unwrap()
        .definition()
        .description
        .contains("not a sandbox"));
}

/// The conversation tools every tool set keeps.
const CONVERSATION_TOOLS: [&str; 8] = [
    "chat_read",
    "ui_open_page",
    "ui_highlight",
    "history_chapters",
    "history_search",
    "history_read",
    "environments",
    "js",
];

fn sorted(names: &[String]) -> Vec<String> {
    let mut names = names.to_vec();
    names.sort();
    names
}

#[test]
fn each_runner_registers_only_its_vault_tool_set() {
    let _serial = serial_guard();
    let f = Fixture::new();
    // Read on this computer: the read-only vault tools and the conversation
    // tools, and no desktop or project tools even with the desktop pack.
    f.vault
        .set_agent_tool_set("this_device".into(), Some("read".into()))
        .unwrap();
    let agent = f.open(true, PermissionMode::Bypass);
    let mut expected: Vec<String> = ["search_notes", "read_page", "memory_read", "notes_help"]
        .into_iter()
        .chain(CONVERSATION_TOOLS)
        .map(String::from)
        .collect();
    expected.sort();
    assert_eq!(sorted(&agent.host_info().tools), expected);
    let value = script(
        &agent,
        "return {create: typeof tools.create_page, read: typeof tools.read_page, exec: typeof tools.exec};",
    );
    assert_eq!(
        value,
        json!({"create":"undefined", "read":"function", "exec":"undefined"})
    );
    drop(agent);

    // Write: every vault tool, still no desktop tools.
    f.vault
        .set_agent_tool_set("this_device".into(), Some("write".into()))
        .unwrap();
    let agent = f.open(true, PermissionMode::Bypass);
    assert_host_tool_names(&agent.host_info().tools, false);
    assert!(agent.host_info().packs.is_empty());
    drop(agent);

    // The hub's set is its own: Read there leaves this computer on Write.
    f.vault
        .set_agent_tool_set("hub".into(), Some("read".into()))
        .unwrap();
    let hub = Agent::open(
        f.vault.clone(),
        AgentConfig {
            data_dir: f.data.clone(),
            host_kind: HostKind::Hub,
            ..AgentConfig::default()
        },
    )
    .unwrap();
    assert!(!hub.host_info().tools.iter().any(|t| t == "create_page"));
    drop(hub);

    // Clearing it is Everything again: the desktop tools come back.
    f.vault
        .set_agent_tool_set("this_device".into(), None)
        .unwrap();
    let agent = f.open(true, PermissionMode::Bypass);
    assert_host_tool_names(&agent.host_info().tools, true);
}

#[test]
fn base_host_has_no_desktop_tools() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(false, PermissionMode::Bypass);
    let info = agent.host_info();
    assert!(info.packs.is_empty());
    assert_host_tool_names(&info.tools, false);
    assert!(!info
        .tools
        .iter()
        .any(|name| name == "exec" || name == "read"));
    let value = script(
        &agent,
        "return {read: typeof tools.read, exec: typeof tools.exec};",
    );
    assert_eq!(value, json!({"read":"undefined", "exec":"undefined"}));
}

#[test]
fn host_info_reports_the_configured_capabilities() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Bypass);
    let info = agent.host_info();
    assert_eq!(info.kind, "desktop");
    assert_eq!(info.packs, ["desktop"]);
    assert!(info.can_run_tasks);
    assert_host_tool_names(&info.tools, true);
    assert!(info.providers.iter().any(|provider| provider == "codex"));
    assert!(!info.version.is_empty());
    drop(agent);
    let hub = Agent::open(
        f.vault.clone(),
        AgentConfig {
            data_dir: f.data.clone(),
            host_kind: HostKind::Hub,
            ..AgentConfig::default()
        },
    )
    .unwrap();
    assert_eq!(hub.host_info().kind, "hub");
    assert!(!hub.host_info().can_run_tasks);
    assert_host_tool_names(&hub.host_info().tools, false);
}

#[test]
fn exec_runs_freely_when_approvals_are_off() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Bypass);
    let value = script(&agent, &echo_script());
    assert_eq!(value["exitCode"], 0, "{value}");
    assert_eq!(value["stdout"].as_str().unwrap().trim(), "hi");
}

#[test]
fn exec_waits_for_approval_then_runs() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Manual);
    let session = agent.create_session("Approved exec").unwrap();
    let (tx, rx) = mpsc::channel();
    agent.forward_events(move |event| tx.send(event).is_ok());
    std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            agent
                .run_script(&session.id, "Exec", &echo_script())
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let approval = loop {
            if let Forwarded::Event(event) = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
            {
                if let EventKind::ApprovalRequested { approval } = event.kind {
                    break approval;
                }
            }
        };
        assert_eq!(approval.tool, "exec");
        assert_eq!(
            agent
                .session_state(&session.id)
                .unwrap()
                .pending_approvals
                .len(),
            1
        );
        agent
            .decide_approval(&approval.id, "approve", None)
            .unwrap();
        let run = running.join().unwrap();
        assert_eq!(run.status, "completed", "{run:?}");
        let value: Value = serde_json::from_str(&run.text.unwrap()).unwrap();
        assert_eq!(value["stdout"].as_str().unwrap().trim(), "hi");
    });
}

fn process_exists(pid: u32) -> bool {
    #[cfg(windows)]
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}")])
        .status().unwrap();
    #[cfg(not(windows))]
    let status = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .unwrap();
    status.success()
}

#[test]
fn closing_the_agent_terminates_a_long_running_child() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Bypass);
    #[cfg(windows)]
    let command = "$PID | Set-Content -LiteralPath 'child.pid'; Start-Sleep -Seconds 60";
    #[cfg(not(windows))]
    let command = "echo $$ > child.pid; sleep 60";
    let value = script(
        &agent,
        &format!(
            "return await tools.exec({{command:{},yieldMs:0}});",
            json!(command)
        ),
    );
    assert_eq!(value["status"], "running", "{value}");
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(f.root.join("child.pid")) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                break pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "child did not start in the root directory"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(process_exists(pid));
    drop(agent);
    assert!(
        !process_exists(pid),
        "child {pid} survived closing the agent"
    );
}

fn in_session(agent: &Agent, session: &str, code: &str) -> Value {
    let run = agent.run_script(session, "Project test", code).unwrap();
    assert_eq!(run.status, "completed", "{run:?}");
    serde_json::from_str(&run.text.unwrap()).unwrap()
}

fn with_approval(agent: &Agent, session: &str, code: &str, tool: &str, decision: &str) -> Value {
    let (tx, rx) = mpsc::channel();
    agent.forward_events(move |event| tx.send(event).is_ok());
    std::thread::scope(|scope| {
        let running = scope.spawn(|| in_session(agent, session, code));
        let deadline = Instant::now() + Duration::from_secs(20);
        let approval = loop {
            if let Forwarded::Event(event) = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
            {
                if let EventKind::ApprovalRequested { approval } = event.kind {
                    break approval;
                }
            }
        };
        assert_eq!(approval.tool, tool);
        agent.decide_approval(&approval.id, decision, None).unwrap();
        running.join().unwrap()
    })
}

#[test]
fn projects_switch_files_and_exec_cwd_per_session_and_persist() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let second = f._dir.path().join("second");
    std::fs::create_dir(&second).unwrap();
    let agent = f.open(true, PermissionMode::Auto);
    let home = agent.projects_list().unwrap().remove(0);
    assert_eq!(home.name, "Home");
    // Stored paths drop canonicalize's Windows verbatim prefix.
    assert_eq!(
        home.path,
        std::fs::canonicalize(&f.root)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
    );
    let project = agent
        .project_add("Other", &second.to_string_lossy())
        .unwrap();
    assert!(agent
        .project_add("other", &f._dir.path().to_string_lossy())
        .is_err());
    assert!(agent
        .project_add("Duplicate", &second.to_string_lossy())
        .is_err());
    let one = agent.create_session("one").unwrap();
    let two = agent.create_session("two").unwrap();
    in_session(
        &agent,
        &two.id,
        "return await tools.project_switch({project:'Other'});",
    );
    in_session(
        &agent,
        &one.id,
        "return await tools.write({path:'one.txt',content:'first'});",
    );
    in_session(
        &agent,
        &two.id,
        "return await tools.write({path:'two.txt',content:'second'});",
    );
    assert!(f.root.join("one.txt").exists());
    // BubblePaw's native file tools accept only workspace-relative paths, even
    // for an absolute path inside the project root.
    let absolute = in_session(
        &agent,
        &one.id,
        &format!(
            "try {{ await tools.write({{path:{},content:'absolute'}}); return ''; }} catch(e) {{ return String(e); }}",
            json!(f.root.join("absolute.txt").to_string_lossy())
        ),
    );
    assert!(
        absolute.as_str().unwrap().contains("workspace-relative"),
        "{absolute}"
    );
    assert!(!f.root.join("absolute.txt").exists());
    assert!(second.join("two.txt").exists());
    assert!(!f.root.join("two.txt").exists());
    let command = if cfg!(windows) { "cmd /c cd" } else { "pwd" };
    let cwd = in_session(&agent, &two.id, &format!("const p = await tools.exec({{command:{}}}); await wait({{handle:p.handle}}); return await tools.poll({{handle:p.handle}});", json!(command)));
    let reported = std::fs::canonicalize(cwd["stdout"].as_str().unwrap().trim()).unwrap();
    assert_eq!(reported, std::fs::canonicalize(&second).unwrap());
    let new = agent.create_session("last project").unwrap();
    assert_eq!(
        agent.session_dto(new).unwrap().project_id.as_deref(),
        Some(project.id.as_str())
    );
    let outside = in_session(&agent, &one.id, &format!("let errors=[]; for (const path of {}) {{try{{await tools.write({{path,content:'x'}})}}catch(e){{errors.push(String(e))}}}} return errors;", json!(["../missing/new/file.txt", second.join("missing").join("file.txt").to_string_lossy().as_ref()])));
    for error in outside.as_array().unwrap() {
        assert!(
            error.as_str().unwrap().contains("outside project \"Home\""),
            "{error}"
        );
    }
    assert_eq!(outside.as_array().unwrap().len(), 2);
    agent.permission_mode_set(PermissionMode::Bypass).unwrap();
    drop(agent);
    let reopened = f.open(true, PermissionMode::Manual);
    assert_eq!(reopened.host_info().permission_mode, PermissionMode::Manual);
    assert_eq!(
        std::fs::read_to_string(f.data.join("agent/permission-mode.json")).unwrap(),
        "\"bypass\""
    );
    assert_eq!(reopened.projects_list().unwrap().len(), 2);
    assert_eq!(
        reopened.session_dto(one).unwrap().project_id.as_deref(),
        Some(home.id.as_str())
    );
    assert_eq!(
        reopened.session_dto(two).unwrap().project_id.as_deref(),
        Some(project.id.as_str())
    );
    assert!(f.data.join("agent/projects.json").exists());
}

#[test]
fn manual_auto_bypass_and_next_run_mode_preserve_session_rules() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let other = f._dir.path().join("opened");
    std::fs::create_dir(&other).unwrap();
    let agent = f.open(true, PermissionMode::Manual);
    let session = agent.create_session("permissions").unwrap();
    with_approval(
        &agent,
        &session.id,
        "return await tools.write({path:'manual.txt',content:'ok'});",
        "write",
        "approve_session",
    );
    agent.permission_mode_set(PermissionMode::Auto).unwrap();
    in_session(
        &agent,
        &session.id,
        "return await tools.write({path:'auto.txt',content:'ok'});",
    );
    in_session(&agent, &session.id, &echo_script());
    let opened = with_approval(
        &agent,
        &session.id,
        &format!(
            "return await tools.project_open({{path:{},name:'Home'}});",
            json!(other.to_string_lossy())
        ),
        "project_open",
        "approve",
    );
    assert_eq!(opened["name"], "Home (2)");
    assert_eq!(
        agent
            .session_dto(session.clone())
            .unwrap()
            .project_id
            .as_deref(),
        opened["id"].as_str()
    );
    agent.permission_mode_set(PermissionMode::Manual).unwrap();
    // The durable per-tool session rule takes precedence over Manual.
    in_session(
        &agent,
        &session.id,
        "return await tools.write({path:'session-rule.txt',content:'ok'});",
    );
    let fresh = agent.create_session("asks again").unwrap();
    with_approval(
        &agent,
        &fresh.id,
        "return await tools.write({path:'manual-again.txt',content:'ok'});",
        "write",
        "approve",
    );
    agent.permission_mode_set(PermissionMode::Bypass).unwrap();
    let third = f._dir.path().join("third");
    std::fs::create_dir(&third).unwrap();
    in_session(
        &agent,
        &session.id,
        &format!(
            "return await tools.project_open({{path:{}}});",
            json!(third.to_string_lossy())
        ),
    );
    assert_eq!(agent.projects_list().unwrap().len(), 3);
}

#[test]
fn remove_reassigns_in_use_sessions_to_most_recent_remaining() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Auto);
    let home = agent.projects_list().unwrap()[0].clone();
    let mut projects = Vec::new();
    for name in ["Second", "Third"] {
        let path = f._dir.path().join(name);
        std::fs::create_dir(&path).unwrap();
        projects.push(agent.project_add(name, &path.to_string_lossy()).unwrap());
    }
    let session = agent.create_session("removal").unwrap();
    agent
        .session_project_set(&session.id, &projects[0].id)
        .unwrap();
    agent
        .session_project_set(&session.id, &projects[1].id)
        .unwrap();
    agent.project_remove(&projects[1].id).unwrap();
    assert_eq!(
        agent
            .session_dto(session.clone())
            .unwrap()
            .project_id
            .as_deref(),
        Some(projects[0].id.as_str())
    );
    assert!(PathBuf::from(&projects[1].path).is_dir());
    assert!(agent.project_rename(&projects[0].id, "home").is_err());
    agent.project_remove(&projects[0].id).unwrap();
    assert_eq!(
        agent.session_dto(session).unwrap().project_id.as_deref(),
        Some(home.id.as_str())
    );
    assert!(agent.project_remove(&home.id).is_err());
}

#[test]
fn mode_and_project_switches_preserve_process_handle_and_shutdown() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let agent = f.open(true, PermissionMode::Auto);
    let second = f._dir.path().join("second");
    std::fs::create_dir(&second).unwrap();
    let project = agent
        .project_add("Second", &second.to_string_lossy())
        .unwrap();
    let home = agent.projects_list().unwrap()[0].clone();
    let session = agent.create_session("process preservation").unwrap();
    agent.session_project_set(&session.id, &project.id).unwrap();
    #[cfg(windows)]
    let command = "$PID | Set-Content -LiteralPath 'child.pid'; Start-Sleep -Seconds 60";
    #[cfg(not(windows))]
    let command = "echo $$ > child.pid; sleep 60";
    let started = in_session(
        &agent,
        &session.id,
        &format!(
            "return await tools.exec({{command:{},yieldMs:0}});",
            json!(command)
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(second.join("child.pid")) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                break pid;
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    };
    agent.permission_mode_set(PermissionMode::Manual).unwrap();
    agent.session_project_set(&session.id, &home.id).unwrap();
    assert!(process_exists(pid));
    let polled = in_session(
        &agent,
        &session.id,
        &format!(
            "return await tools.poll({{handle:{}}});",
            json!(started["handle"])
        ),
    );
    assert_eq!(polled["status"], "running");
    drop(agent);
    assert!(!process_exists(pid));
}

#[test]
fn unavailable_last_used_project_does_not_prevent_reopen_or_recovery() {
    let _serial = serial_guard();
    let f = Fixture::new();
    let second = f._dir.path().join("second");
    std::fs::create_dir(&second).unwrap();
    let moved = f._dir.path().join("moved-second");
    let agent = f.open(true, PermissionMode::Auto);
    let home = agent.projects_list().unwrap()[0].clone();
    let project = agent
        .project_add("Second", &second.to_string_lossy())
        .unwrap();
    let session = agent.create_session("missing project recovery").unwrap();
    agent.session_project_set(&session.id, &project.id).unwrap();
    drop(agent);
    std::fs::rename(&second, &moved).unwrap();

    let reopened = f.open(true, PermissionMode::Auto);
    assert_eq!(reopened.projects_list().unwrap().len(), 2);
    assert_eq!(
        reopened
            .session_dto(session.clone())
            .unwrap()
            .project_id
            .as_deref(),
        Some(project.id.as_str())
    );
    let missing = in_session(
        &reopened,
        &session.id,
        "try { await tools.list({}); return {error:null}; } catch(e) { return {error:String(e)}; }",
    );
    assert!(
        missing["error"]
            .as_str()
            .unwrap()
            .contains("workspace root does not exist"),
        "{missing}"
    );
    assert!(
        !second.exists(),
        "opening the agent must not recreate missing project folders"
    );
    in_session(
        &reopened,
        &session.id,
        &format!(
            "return await tools.project_switch({{project:{}}});",
            json!(home.id)
        ),
    );
    in_session(
        &reopened,
        &session.id,
        "return await tools.write({path:'recovered.txt',content:'ok'});",
    );
    assert!(f.root.join("recovered.txt").exists());
    reopened.project_remove(&project.id).unwrap();
    assert_eq!(reopened.projects_list().unwrap().len(), 1);
    assert!(
        moved.is_dir(),
        "unregistering must preserve the moved project folder"
    );
}

#[test]
fn default_bypass_startup_ignores_and_preserves_old_permission_files() {
    let _serial = serial_guard();
    assert_eq!(PermissionMode::default(), PermissionMode::Bypass);
    assert_eq!(
        AgentConfig::default().permission_mode,
        PermissionMode::Bypass
    );
    let f = Fixture::new();
    for (index, stored) in [r#""manual""#, r#""ask""#].into_iter().enumerate() {
        // Each open gets its own data directory, so no open reuses the store of an agent just dropped.
        let data = f._dir.path().join(format!("data-{index}"));
        let dir = data.join("agent");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("permission-mode.json");
        std::fs::write(&file, stored).unwrap();
        let agent = f.open_in(data, true, PermissionMode::default());
        assert_eq!(agent.host_info().permission_mode, PermissionMode::Bypass);
        let session = agent.create_session("Bypass startup").unwrap();
        in_session(
            &agent,
            &session.id,
            &format!("return await tools.write({{path:'bypass-{index}.txt',content:'ok'}});"),
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), stored);
        drop(agent);
    }
}

/// The hub's any-computer `/mcp/agent` lists the core computer tools from
/// `core_computer_tool_definitions` without asking a desktop. They must match
/// what a desktop's relay advertises.
#[test]
fn core_computer_tool_definitions_match_the_relay_list() {
    let _serial = serial_guard();
    let fx = Fixture::new();
    let agent = fx.open(true, PermissionMode::Bypass);
    let relay = agent.relay_tool_definitions(None).unwrap();
    let core = vault_agent::tools::core_computer_tool_definitions().unwrap();
    let names: Vec<_> = core.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, vault_agent::tools::CORE_COMPUTER_TOOLS);
    for tool in core {
        let advertised = relay
            .iter()
            .find(|listed| listed.name == tool.name)
            .unwrap_or_else(|| panic!("relay lists no {}", tool.name));
        assert_eq!(advertised.description, tool.description, "{}", tool.name);
        assert_eq!(advertised.input_schema, tool.input_schema, "{}", tool.name);
    }
}
