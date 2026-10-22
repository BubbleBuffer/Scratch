//! MCP servers on the desktop agent, with a fake stdio server
//! (`fixtures/fake_mcp.py`, needs `python3`).

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde_json::json;
use vault_agent::bubblepaw_core::{ClientInfo, EventKind, ToolCall};
use vault_agent::bubblepaw_runner::provider::ScriptedStep;
use vault_agent::mcp::{self, McpServerSettings, McpSettings, ServerState};
use vault_agent::{Agent, AgentConfig, Forwarded, HostKind, PermissionMode, TestHooks};
use vault_core::Vault;

const ECHO: &str = "mcp__fake__echo";
const CODE: &str = r#"return await tools.mcp__fake__echo({text: "in app"});"#;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_mcp.py")
}

fn fake() -> McpServerSettings {
    McpServerSettings {
        command: "python3".into(),
        args: vec![fixture().to_string_lossy().into_owned()],
        ..McpServerSettings::default()
    }
}

fn broken() -> McpServerSettings {
    McpServerSettings {
        command: "notes-no-such-mcp-server".into(),
        startup_timeout_secs: 5,
        ..McpServerSettings::default()
    }
}

fn servers(list: Vec<(&str, McpServerSettings)>) -> McpSettings {
    McpSettings {
        servers: list.into_iter().map(|(n, s)| (n.to_string(), s)).collect(),
        ..McpSettings::default()
    }
}

/// `fake` (the fixture) and `broken` (a command that does not exist).
fn both() -> McpSettings {
    servers(vec![("fake", fake()), ("broken", broken())])
}

fn open(
    dir: &tempfile::TempDir,
    host_kind: HostKind,
    steps: Vec<ScriptedStep>,
    mcp: McpSettings,
) -> Agent {
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
            mcp,
            test: TestHooks {
                scripted: Some((steps, "All done.".into())),
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
    )
    .unwrap()
}

fn relay_names(agent: &Agent) -> Vec<String> {
    agent
        .relay_tool_definitions(None)
        .unwrap()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

fn relay_echo(agent: &Agent, text: &str) -> vault_agent::bubblepaw_core::ToolResult {
    let session = agent.create_session("relay").unwrap();
    agent
        .relay_call(
            &session.id,
            ECHO,
            json!({ "text": text }),
            ClientInfo {
                id: "relay-test".into(),
                name: None,
            },
            Default::default(),
        )
        .unwrap()
}

#[test]
fn import_file_reads_and_renames_once_and_leaves_a_bad_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(mcp::FILE);
    assert!(mcp::import_file(dir.path()).unwrap().is_none());

    std::fs::write(
        &path,
        "[mcp.servers.fake]\ncommand = \"python3\"\nargs = [\"x.py\"]\nenabled = false\n",
    )
    .unwrap();
    let settings = mcp::import_file(dir.path()).unwrap().unwrap();
    let server = &settings.servers["fake"];
    assert_eq!(server.command, "python3");
    assert_eq!(server.args, vec!["x.py".to_string()]);
    assert!(!server.enabled);
    assert!(!path.exists());
    assert!(dir.path().join(mcp::IMPORTED).exists());
    assert!(mcp::import_file(dir.path()).unwrap().is_none());

    std::fs::write(&path, "[mcp.servers.x\n").unwrap();
    assert!(mcp::import_file(dir.path()).is_err());
    assert!(path.exists());
}

#[test]
fn the_relay_offers_and_runs_a_server_tool_and_reports_the_broken_server() {
    let dir = tempfile::tempdir().unwrap();
    let agent = open(&dir, HostKind::Desktop, vec![], both());
    let names = relay_names(&agent);
    assert!(names.iter().any(|n| n == ECHO), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("mcp__broken__")), "{names:?}");

    let result = relay_echo(&agent, "from the hub");
    assert!(!result.is_error, "{result:?}");
    assert!(!result.pending, "{result:?}");
    assert!(result.content.contains("echo: from the hub"), "{result:?}");

    let status = agent.mcp_status();
    assert_eq!(status.len(), 2, "{status:?}");
    assert_eq!(status[0].name, "broken");
    assert_eq!(status[0].state, ServerState::Failed, "{status:?}");
    assert_eq!(status[0].tools, 0);
    let error = status[0].error.as_deref().unwrap_or_default();
    assert!(
        !error.is_empty() && !error.starts_with("MCP server"),
        "{error}"
    );
    assert_eq!(status[1].name, "fake");
    assert_eq!(status[1].state, ServerState::Running, "{status:?}");
    assert_eq!(status[1].tools, 1);
    assert_eq!(status[1].error, None);
}

#[test]
fn set_mcp_adds_disables_and_removes_a_server_without_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let agent = open(&dir, HostKind::Desktop, vec![], McpSettings::default());
    assert!(!relay_names(&agent).iter().any(|n| n == ECHO));
    assert!(agent.mcp_status().is_empty());

    agent.set_mcp(servers(vec![("fake", fake())])).unwrap();
    let names = relay_names(&agent);
    assert!(names.iter().any(|n| n == ECHO), "{names:?}");
    let result = relay_echo(&agent, "after set");
    assert!(result.content.contains("echo: after set"), "{result:?}");
    let status = agent.mcp_status();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].state, ServerState::Running);
    assert_eq!(status[0].tools, 1);

    // Invalid settings change nothing.
    let invalid = McpServerSettings {
        url: Some("http://example.com/mcp".into()),
        ..McpServerSettings::default()
    };
    assert!(agent.set_mcp(servers(vec![("web", invalid)])).is_err());
    assert!(relay_names(&agent).iter().any(|n| n == ECHO));
    assert_eq!(agent.mcp_status()[0].name, "fake");

    let off = McpServerSettings {
        enabled: false,
        ..fake()
    };
    agent.set_mcp(servers(vec![("fake", off)])).unwrap();
    assert!(!relay_names(&agent).iter().any(|n| n == ECHO));
    let status = agent.mcp_status();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].state, ServerState::Off);
    assert_eq!(status[0].tools, 0);
    assert_eq!(status[0].error, None);

    agent.set_mcp(servers(vec![("fake", fake())])).unwrap();
    assert!(relay_names(&agent).iter().any(|n| n == ECHO));
    agent.set_mcp(McpSettings::default()).unwrap();
    assert!(!relay_names(&agent).iter().any(|n| n.starts_with("mcp__")));
    assert!(agent.mcp_status().is_empty());
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
    // Applied after open, like the app does.
    let agent = open(&dir, HostKind::Desktop, vec![step], McpSettings::default());
    agent.set_mcp(both()).unwrap();
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
    let agent = open(&dir, HostKind::Hub, vec![], both());
    assert!(!relay_names(&agent).iter().any(|n| n.starts_with("mcp__")));
    agent.set_mcp(both()).unwrap();
    assert!(!relay_names(&agent).iter().any(|n| n.starts_with("mcp__")));
    assert!(agent.mcp_status().is_empty());
}
