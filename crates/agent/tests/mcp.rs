//! MCP servers from `mcp.toml` on the desktop agent, with a fake stdio
//! server (`fixtures/fake_mcp.py`, needs `python3`).

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde_json::json;
use vault_agent::bubblepaw_core::{ClientInfo, EventKind, ToolCall};
use vault_agent::bubblepaw_runner::provider::ScriptedStep;
use vault_agent::mcp::{self, McpSettings};
use vault_agent::{Agent, AgentConfig, Forwarded, HostKind, PermissionMode, TestHooks};
use vault_core::Vault;

const ECHO: &str = "mcp__fake__echo";
const CODE: &str = r#"return await tools.mcp__fake__echo({text: "in app"});"#;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_mcp.py")
}

/// `fake` (the fixture) and `broken` (a command that does not exist).
fn settings(dir: &std::path::Path) -> McpSettings {
    let text = format!(
        "[mcp.servers.fake]\ncommand = \"python3\"\nargs = [{:?}]\n\n\
         [mcp.servers.broken]\ncommand = \"notes-no-such-mcp-server\"\nstartup_timeout_secs = 5\n",
        fixture().to_string_lossy()
    );
    let path = dir.join(mcp::FILE);
    std::fs::write(&path, text).unwrap();
    let (settings, error) = mcp::load_settings(&path);
    assert_eq!(error, None);
    assert_eq!(settings.servers.len(), 2);
    settings
}

fn open(dir: &tempfile::TempDir, host_kind: HostKind, steps: Vec<ScriptedStep>) -> Agent {
    let root = dir.path().join("vault").to_string_lossy().into_owned();
    let vault = Arc::new(Vault::open(root).unwrap());
    Agent::open(
        vault,
        AgentConfig {
            data_dir: dir.path().join("data"),
            default_model: "scripted/scripted".into(),
            bun_path: Some(dir.path().join("missing-test-bun")),
            permission_mode: PermissionMode::Manual,
            host_kind,
            mcp: settings(dir.path()),
            test: TestHooks {
                scripted: Some((steps, "All done.".into())),
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
    )
    .unwrap()
}

#[test]
fn a_missing_file_means_no_servers_and_a_bad_one_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(mcp::FILE);
    let (settings, error) = mcp::load_settings(&path);
    assert!(settings.servers.is_empty());
    assert_eq!(error, None);

    std::fs::write(&path, "").unwrap();
    let (settings, error) = mcp::load_settings(&path);
    assert!(settings.servers.is_empty());
    assert_eq!(error, None);

    std::fs::write(&path, "[mcp.servers.x\n").unwrap();
    let (settings, error) = mcp::load_settings(&path);
    assert!(settings.servers.is_empty());
    assert!(error.unwrap().contains("no MCP servers"));
}

#[test]
fn the_relay_offers_and_runs_a_server_tool_and_skips_the_broken_server() {
    let dir = tempfile::tempdir().unwrap();
    let agent = open(&dir, HostKind::Desktop, vec![]);
    let names: Vec<String> = agent
        .relay_tool_definitions(None)
        .unwrap()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(names.iter().any(|n| n == ECHO), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("mcp__broken__")), "{names:?}");

    let session = agent.create_session("relay").unwrap();
    let result = agent
        .relay_call(
            &session.id,
            ECHO,
            json!({ "text": "from the hub" }),
            ClientInfo {
                id: "relay-test".into(),
                name: None,
            },
            Default::default(),
        )
        .unwrap();
    assert!(!result.is_error, "{result:?}");
    assert!(!result.pending, "{result:?}");
    assert!(result.content.contains("echo: from the hub"), "{result:?}");
}

#[test]
fn an_in_app_run_calls_a_server_tool_after_its_confirm_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let step = ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: "call-mcp".into(),
            name: "js".into(),
            arguments: json!({ "code": CODE }),
        }],
    };
    let agent = open(&dir, HostKind::Desktop, vec![step]);
    let (tx, rx) = mpsc::channel();
    agent.forward_events(move |event| tx.send(event).is_ok());
    let session = agent.create_session("Chat").unwrap();
    agent.send(&session.id, "Echo", None).unwrap();

    // MCP tools default to `approval = "confirm"`; Manual mode asks.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Forwarded::Event(event) = rx.recv_timeout(left).expect("event before timeout")
        else {
            continue;
        };
        if event.session_id != session.id {
            continue;
        }
        match &event.kind {
            EventKind::ApprovalRequested { approval } => {
                assert_eq!(approval.tool, ECHO);
                agent.decide_approval(&approval.id, "approve", None).unwrap();
            }
            EventKind::RunFinished { .. } => {
                seen.push(event.kind);
                break;
            }
            _ => {}
        }
        seen.push(event.kind);
    }
    assert!(
        seen.iter().any(|k| matches!(k, EventKind::ApprovalRequested { .. })),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|k| matches!(k,
            EventKind::ToolFinished { result, .. } if result.content.contains("echo: in app"))),
        "{seen:?}"
    );
}

#[test]
fn the_hub_never_starts_servers() {
    let dir = tempfile::tempdir().unwrap();
    let agent = open(&dir, HostKind::Hub, vec![]);
    let names: Vec<String> = agent
        .relay_tool_definitions(None)
        .unwrap()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(!names.iter().any(|n| n.starts_with("mcp__")), "{names:?}");
}
