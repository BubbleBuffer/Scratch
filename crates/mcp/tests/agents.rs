//! AI tasks, the agent runner and merge proposals, with the fake agent
//! (`examples/fake_agent.rs`) in place of Claude Code or Codex.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use vault_core::*;
use vault_mcp::http::{HttpOptions, McpHttp};
use vault_mcp::runner::{self, AgentCommand, RunContext};
use vault_mcp::{endpoint, merge, tasks, McpServer};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_agents_test_{}", new_id())))
    }
    fn root(&self) -> String {
        self.0.join("vault").to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The fake agent, built with the tests (`target/<profile>/examples`).
fn fake_agent() -> String {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().join("examples");
    let name = if cfg!(windows) {
        "fake_agent.exe"
    } else {
        "fake_agent"
    };
    let p = dir.join(name);
    assert!(p.is_file(), "build the examples first: {}", p.display());
    p.to_string_lossy().into_owned()
}

struct App {
    v: Arc<Vault>,
    http: McpHttp,
    token: String,
    work: PathBuf,
}

fn app(t: &Temp) -> App {
    let v = Arc::new(Vault::open(t.root()).unwrap());
    let token = endpoint::token(&t.root()).unwrap();
    let http = McpHttp::serve(
        "127.0.0.1:0",
        Arc::new(McpServer::new(v.clone())),
        token.clone(),
        HttpOptions::default(),
    )
    .unwrap();
    App {
        v,
        http,
        token,
        work: t.0.join("work"),
    }
}

impl App {
    fn ctx(&self, timeout: Duration) -> RunContext {
        RunContext {
            mcp_url: self.http.url(),
            mcp_token: self.token.clone(),
            work_dir: self.work.clone(),
            timeout,
        }
    }
}

fn claude_like() -> AgentCommand {
    AgentCommand {
        program: fake_agent(),
        args: vec!["-p".into(), "--mcp-config".into(), "{mcp_config}".into()],
    }
}

fn texts(v: &Vault, page: &str) -> Vec<String> {
    v.load_page(page.into())
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.text.clone())
        .collect()
}

#[test]
fn task_runs_with_a_fake_agent_and_writes_back() {
    let t = Temp::new();
    let a = app(&t);
    let v = &a.v;
    let page = v.create_page(None, "Trip plan".into()).unwrap();
    let ids = v
        .insert_markdown(
            page.clone(),
            None,
            None,
            true,
            "Day one\n\n<br>\n\nDay two".into(),
        )
        .unwrap();
    let blank = v.load_page(page.clone()).unwrap().blocks[1].clone();
    assert!(blank.text.is_empty());

    // Created from the empty paragraph: it becomes the placeholder.
    let task = tasks::create(v, "Summarize the plan", &page, Some(&blank.id), "").unwrap();
    assert_eq!(task.status, tasks::QUEUED);
    assert_eq!(task.block.as_deref(), Some(blank.id.as_str()));
    let p = v.load_page(page.clone()).unwrap();
    assert_eq!(p.blocks[1].kind, BlockKind::Callout);
    assert!(p.blocks[1]
        .text
        .starts_with("AI task queued: Summarize the plan"));
    let coll = tasks::collection(v).unwrap().unwrap();
    assert_eq!(v.load_page(coll.clone()).unwrap().title, "AI tasks");
    assert_eq!(tasks::list(v, 10).unwrap().len(), 1);

    let claimed = tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
    assert_eq!(claimed.id, task.id);
    assert_eq!(claimed.status, tasks::RUNNING);
    assert_eq!(claimed.device, v.device_id());
    assert!(texts(v, &page)[1].starts_with("AI is working on this"));
    assert!(tasks::claim_next(v, "x").unwrap().is_none());

    let prompt = tasks::agent_prompt(v, &claimed, tasks::PromptTools::Mcp).unwrap();
    assert!(
        prompt.contains("Summarize the plan")
            && prompt.contains("Day two")
            && !prompt.contains("AI is working")
    );
    let out = runner::run(&claude_like(), &prompt, &a.ctx(Duration::from_secs(60))).unwrap();
    assert!(out.ok(), "{out:?}");
    // The config file with the token is gone after the run.
    assert_eq!(fs::read_dir(&a.work).unwrap().count(), 0);
    tasks::finish(v, &claimed.id, out.result()).unwrap();

    let after = texts(v, &page);
    assert_eq!(
        after,
        [
            "Day one",
            "Fake answer to: Summarize the plan",
            "Read over MCP: Trip plan",
            "Day two"
        ]
    );
    assert_eq!(v.load_page(page.clone()).unwrap().blocks[0].id, ids[0]);
    let done = tasks::get(v, &claimed.id).unwrap();
    assert_eq!(done.status, tasks::DONE);
    assert!(v.page_markdown(done.id).unwrap().contains("Fake answer to"));
}

#[test]
fn agent_edits_fails_and_times_out() {
    let t = Temp::new();
    let a = app(&t);
    let v = &a.v;
    let me = v.device_id();
    let page = v.create_page(None, "Notes".into()).unwrap();
    v.insert_markdown(page.clone(), None, None, true, "Hello".into())
        .unwrap();

    // Codex-style: the answer goes to {output_file}; empty means "I edited".
    let codex_like = AgentCommand {
        program: fake_agent(),
        args: vec![
            "exec".into(),
            "--answer-file".into(),
            "{output_file}".into(),
        ],
    };
    tasks::create(v, "FAKE:edit please", &page, None, "codex").unwrap();
    let t1 = tasks::claim_next(v, &me).unwrap().unwrap();
    assert_eq!(t1.runner, "codex");
    let out = runner::run(
        &codex_like,
        &tasks::agent_prompt(v, &t1, tasks::PromptTools::Mcp).unwrap(),
        &a.ctx(Duration::from_secs(60)),
    )
    .unwrap();
    assert!(out.ok() && out.stdout.is_empty(), "{out:?}");
    tasks::finish(v, &t1.id, out.result()).unwrap();
    assert_eq!(texts(v, &page), ["Hello", "Edited by the fake agent"]);

    // A failing agent: the task fails and the placeholder says why.
    tasks::create(v, "FAKE:fail now", &page, None, "").unwrap();
    let t2 = tasks::claim_next(v, &me).unwrap().unwrap();
    let out = runner::run(
        &claude_like(),
        &tasks::agent_prompt(v, &t2, tasks::PromptTools::Mcp).unwrap(),
        &a.ctx(Duration::from_secs(60)),
    )
    .unwrap();
    let res = out.result();
    assert!(
        res.as_ref().unwrap_err().contains("code 3")
            && res.as_ref().unwrap_err().contains("asked to fail"),
        "{res:?}"
    );
    tasks::finish(v, &t2.id, res).unwrap();
    let failed = tasks::get(v, &t2.id).unwrap();
    assert_eq!(failed.status, tasks::FAILED);
    let p = v.load_page(page.clone()).unwrap();
    let ph = p.blocks.last().unwrap();
    assert!(
        ph.text.starts_with("AI task failed") && ph.text.contains("code 3"),
        "{}",
        ph.text
    );
    assert_eq!(ph.color.as_deref(), Some("red_background"));
    tasks::retry(v, &t2.id).unwrap();
    let again = tasks::get(v, &t2.id).unwrap();
    assert_eq!(again.status, tasks::QUEUED);
    assert!(again.error.is_empty());

    // A timeout kills the agent.
    let out = runner::run(
        &claude_like(),
        "FAKE:sleep",
        &a.ctx(Duration::from_millis(500)),
    )
    .unwrap();
    assert!(out.timed_out && out.result().is_err());

    // A missing program is an error, not a panic.
    let missing = AgentCommand {
        program: "no-such-agent-xyz".into(),
        args: vec![],
    };
    assert!(runner::run(&missing, "x", &a.ctx(Duration::from_secs(5))).is_err());

    // Tasks left running by this device fail after a restart.
    let t3 = tasks::claim_next(v, &me).unwrap().unwrap();
    assert_eq!(t3.id, t2.id);
    assert_eq!(tasks::recover(v, &me).unwrap(), 1);
    assert_eq!(tasks::get(v, &t3.id).unwrap().status, tasks::FAILED);
}

#[test]
fn default_commands_are_argument_lists() {
    let c = runner::default_command("claude", None).unwrap();
    assert_eq!(c.program, "claude");
    assert!(c.args.contains(&"{mcp_config}".to_string()) && c.args.contains(&"-p".to_string()));
    let x = runner::default_command("codex", Some("C:\\tools\\codex.exe")).unwrap();
    assert_eq!(x.program, "C:\\tools\\codex.exe");
    assert_eq!(x.args.last().unwrap(), "-");
    assert!(runner::default_command("deepseek", None).is_none());
}

#[test]
fn task_from_the_phone_runs_on_the_desktop() {
    let t = Temp::new();
    let desktop = Vault::open(t.root()).unwrap();
    let phone_root = t.0.join("phone").to_string_lossy().into_owned();
    let phone = Vault::open(phone_root).unwrap();
    let send = |from: &Vault, to: &Vault| {
        for pv in from.page_versions().unwrap() {
            to.import_updates(
                pv.page.clone(),
                from.export_updates(pv.page, vec![]).unwrap(),
            )
            .unwrap();
        }
    };
    let page = phone.create_page(None, "Groceries".into()).unwrap();
    let task = tasks::create(&phone, "Add a list", &page, None, "").unwrap();
    send(&phone, &desktop);
    let got = tasks::claim_next(&desktop, &desktop.device_id())
        .unwrap()
        .unwrap();
    assert_eq!(got.id, task.id);
    tasks::finish(&desktop, &got.id, Ok("- milk\n- eggs".into())).unwrap();
    send(&desktop, &phone);
    assert_eq!(tasks::get(&phone, &task.id).unwrap().status, tasks::DONE);
    assert_eq!(texts(&phone, &page), ["milk", "eggs"]);
}

#[test]
fn merge_proposal_from_the_agent() {
    let t = Temp::new();
    let a = app(&t);
    let v = &a.v;
    let page = v.create_page(None, "Shared".into()).unwrap();
    let ids = v
        .insert_markdown(page.clone(), None, None, true, "Meet at 10".into())
        .unwrap();
    let side = |text: &str| Block {
        id: ids[0].clone(),
        kind: BlockKind::Paragraph,
        text: text.into(),
        marks: vec![],
        checked: false,
        children: vec![],
        target: None,
        color: None,
        width: None,
    };
    let c = Conflict {
        page: page.clone(),
        id: ids[0].clone(),
        format: 2,
        block: ids[0].clone(),
        time_ms: 0,
        scope: ConflictScope::Block { block: ids[0].clone() },
        status: ConflictStatus::Pending,
        applied: vec![],
        sides: vec![
            ConflictSide::new("laptop-device".into(), 0, None,
                SideState::Present { content: vec![side("Meet at 10 at the cafe")] }, None),
            ConflictSide::new("phone-device".into(), 0, None,
                SideState::Present { content: vec![side("Meet at 11")] }, None),
        ],
    }.with_legacy_blocks();
    let prompt = merge::prompt(v, &c);
    assert!(prompt.contains("The edited version wins; merge only text that both sides edited."));
    assert!(prompt.contains("Keep children intact, including unchanged descendants."));
    assert!(!prompt.contains("intent of both edits"));
    assert!(
        prompt.contains("Meet at 10 at the cafe")
            && prompt.contains("Meet at 11")
            && prompt.contains("\"Shared\"")
    );
    let out = runner::run(&claude_like(), &prompt, &a.ctx(Duration::from_secs(60))).unwrap();
    let p = merge::parse(v, &out.result().unwrap());
    assert_eq!(p.text, "Merged by the fake agent");
    assert_eq!(p.marks.len(), 1);
    assert_eq!(p.marks[0].kind, MarkKind::Bold);
    // Fences and several paragraphs.
    assert_eq!(merge::parse(v, "```\nA *b*\n```").text, "A b");
    assert_eq!(merge::parse(v, "One\n\nTwo").text, "One\nTwo");
    // Nothing was written.
    assert_eq!(texts(v, &page), ["Meet at 10"]);
}
