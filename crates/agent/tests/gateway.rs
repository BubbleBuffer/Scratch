//! Gateway native calls and JS exercise the shared BubblePaw runtime without providers.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use vault_agent::{
    bubblepaw_sdk::{
        approvals::Approval,
        harness::{ClientInfo, ExecutionMode},
        store::Role,
        tools::{Tool, ToolContext, ToolDefinition, ToolResult},
    },
    Agent, AgentConfig, PermissionMode, TestHooks,
};
use vault_core::Vault;

struct Fixture {
    _dir: tempfile::TempDir,
    vault: Arc<Vault>,
    agent: Agent,
}

impl Fixture {
    fn new() -> Self {
        Self::with_tools(None, Vec::new())
    }

    fn with_tools(
        execution: Option<ExecutionMode>,
        extra_tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let config = AgentConfig {
            data_dir: dir.path().join("data"),
            bun_path: Some(dir.path().join("missing-test-bun")),
            permission_mode: PermissionMode::Manual,
            test: TestHooks {
                confirm_tool: true,
                extra_tools,
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        };
        let agent = match execution {
            Some(mode) => Agent::open_with_execution(vault.clone(), config, mode),
            None => Agent::open(vault.clone(), config),
        }
        .unwrap();
        Self {
            _dir: dir,
            vault,
            agent,
        }
    }

    fn js(&self, session: &str, input: Value) -> ToolResult {
        self.agent
            .gateway_call_as(
                session,
                "js",
                input,
                ClientInfo {
                    id: "mcp-test".into(),
                    name: None,
                },
            )
            .unwrap()
    }
}

struct Probe {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for Probe {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "Gateway test probe".into(),
            input_schema: json!({
                "type":"object", "properties":{"pending":{"type":"boolean"}},
                "additionalProperties":false
            }),
            approval: Approval::None,
            read_only: true,
        }
    }

    async fn invoke(&self, _context: ToolContext, input: Value) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(if input["pending"] == true {
            ToolResult::pending("probe still running")
        } else {
            ToolResult::text("probe complete")
        })
    }
}

fn client() -> ClientInfo {
    ClientInfo {
        id: "mcp-test".into(),
        name: None,
    }
}

fn value(result: ToolResult) -> Value {
    assert!(!result.is_error, "{result:?}");
    assert!(!result.pending, "{result:?}");
    let text = result.content;
    let result = text
        .split_once("\n[---]\n")
        .map_or(text.as_str(), |(result, _)| result);
    serde_json::from_str(result).unwrap()
}

#[test]
fn default_open_keeps_js_and_discovery_does_not_leave_sessions() {
    let f = Fixture::new();
    for _ in 0..3 {
        let definitions = f.agent.gateway_tool_definitions_for_discovery().unwrap();
        assert_eq!(
            definitions
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["js"]
        );
        assert!(f.agent.sessions().unwrap().is_empty());
    }
    let session = f.agent.create_session("ordinary app default").unwrap();
    let run = f
        .agent
        .run_script(&session.id, "default JS", "return 42;")
        .unwrap();
    assert_eq!(run.status, "completed", "{run:?}");
    assert_eq!(run.model, "script");
    assert!(run.text.unwrap().contains("42"));
    assert!(f.agent.gateway_tool_definitions("missing-session").is_err());
}

#[test]
fn native_and_hybrid_surfaces_come_from_the_enabled_runtime() {
    let f = Fixture::with_tools(Some(ExecutionMode::Native), Vec::new());
    let session = f.agent.create_session("native declarations").unwrap();
    let definitions = f.agent.gateway_tool_definitions(&session.id).unwrap();
    let names = definitions
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"chat_read"), "{names:?}");
    assert!(names.contains(&"environments"), "{names:?}");
    assert!(!names.contains(&"js"), "{names:?}");
    assert!(!names.contains(&"hosts"), "{names:?}");
    assert!(!names.contains(&"test_confirm"), "{names:?}");
    for definition in vault_tools::definitions()
        .iter()
        .chain(vault_tools::hidden())
    {
        assert!(
            !names.contains(&definition.name),
            "{}: {names:?}",
            definition.name
        );
    }
    for alias in vault_tools::aliases() {
        assert!(!names.contains(&alias.name), "{}: {names:?}", alias.name);
    }
    let discovery = f.agent.gateway_tool_definitions_for_discovery().unwrap();
    assert_eq!(
        serde_json::to_value(definitions).unwrap(),
        serde_json::to_value(discovery).unwrap()
    );
    assert_eq!(f.agent.sessions().unwrap().len(), 1);
    // Native mode does not offer js, so the gateway refuses a js call.
    assert!(
        f.agent
            .gateway_call_as(&session.id, "js", json!({"code":"return 1;"}), client())
            .unwrap()
            .is_error
    );
    for name in ["hosts", "create_page"] {
        assert!(
            f.agent
                .gateway_call_as(&session.id, name, json!({}), client())
                .unwrap()
                .is_error
        );
    }
    f.agent.gateway_close_session(&session.id).unwrap();
    assert!(f.agent.sessions().unwrap().is_empty());
}

#[test]
fn native_dispatch_is_one_call_and_preserves_errors_and_pending_results() {
    let calls = Arc::new(AtomicUsize::new(0));
    let f = Fixture::with_tools(
        Some(ExecutionMode::Native),
        vec![Arc::new(Probe {
            name: "test_probe",
            calls: calls.clone(),
        })],
    );
    let session = f.agent.create_session("single native call").unwrap();
    let result = f
        .agent
        .gateway_call_as(&session.id, "test_probe", json!({}), client())
        .unwrap();
    assert_eq!(result.content, "probe complete");
    assert!(!result.is_error && !result.pending, "{result:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.runs.len(), 1);
    assert_eq!(state.runs[0].model, "tool");
    assert!(!state
        .messages
        .iter()
        .any(|message| message.role == Role::User));
    assert_eq!(
        state
            .messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .count(),
        1
    );
    for (name, input) in [
        ("test_probe", json!({"unexpected":true})),
        ("missing", json!({})),
    ] {
        let error = f
            .agent
            .gateway_call_as(&session.id, name, input, client())
            .unwrap();
        assert!(error.is_error, "{error:?}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let pending = f
        .agent
        .gateway_call_as(&session.id, "test_probe", json!({"pending":true}), client())
        .unwrap();
    assert!(pending.pending && !pending.is_error, "{pending:?}");
    assert_eq!(pending.content, "probe still running");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn native_and_nested_js_apply_the_same_exclusions_and_noninteractive_policy() {
    let calls = Arc::new(AtomicUsize::new(0));
    let excluded = [
        "history_probe",
        "web",
        "web_search",
        "ui_probe",
        "todo",
        "question",
        "reload",
    ];
    let tools = excluded
        .iter()
        .map(|&name| {
            Arc::new(Probe {
                name,
                calls: calls.clone(),
            }) as Arc<dyn Tool>
        })
        .collect();
    let f = Fixture::with_tools(Some(ExecutionMode::Js), tools);
    let session = f.agent.create_session("gateway policy").unwrap();
    let definitions = f.agent.gateway_tool_definitions(&session.id).unwrap();
    for name in excluded.into_iter().chain(["test_confirm"]) {
        assert!(!definitions.iter().any(|tool| tool.name == name));
        let denied = f
            .agent
            .gateway_call_as(&session.id, name, json!({}), client())
            .unwrap();
        assert!(denied.is_error, "{name}: {denied:?}");
        let code = format!("return '{name}' in tools;");
        assert_eq!(value(f.js(&session.id, json!({"code":code}))), false);
        let code = format!(
            "try {{ await tools.{name}({{}}); return 'ran'; }} catch (error) {{ return 'refused'; }}"
        );
        let refused = f.js(&session.id, json!({"code":code}));
        assert!(refused.is_error || value(refused) == "refused", "{name}");
    }
    let listed = value(f.js(
        &session.id,
        json!({"code":"return (await tools.$list()).map(t => t.name);"}),
    ));
    let listed = listed.as_array().unwrap();
    for name in excluded.into_iter().chain(["test_confirm"]) {
        assert!(!listed.iter().any(|tool| tool.as_str() == Some(name)), "{name}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(f
        .agent
        .session_state(&session.id)
        .unwrap()
        .pending_approvals
        .is_empty());
    f.agent.permission_mode_set(PermissionMode::Bypass).unwrap();
    // In js mode composed tools are not advertised as definitions; the js help
    // lists them, so that is where bypass must show up.
    assert!(f
        .agent
        .gateway_js_help(None)
        .contains("tools.test_confirm("));
    let allowed = f
        .agent
        .gateway_call_as(
            &session.id,
            "test_confirm",
            json!({"text":"native allowed"}),
            client(),
        )
        .unwrap();
    assert_eq!(allowed.content, "confirmed: native allowed");
    assert!(!allowed.is_error);
    assert_eq!(
        value(f.js(
            &session.id,
            json!({"code":"return await tools.test_confirm({text:'JS allowed'});"})
        )),
        "confirmed: JS allowed"
    );
    assert!(f
        .agent
        .session_state(&session.id)
        .unwrap()
        .pending_approvals
        .is_empty());
}

#[test]
fn unrelated_gateway_name_collisions_are_reported() {
    // Includes a normal HTTP tool, optional JS help, hidden MCP-only vault
    // tools and an alias. None is a registered in-app adapter.
    for name in [
        "hosts",
        "js_help",
        "import_markdown",
        "feedback_list",
        "list_pages",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let opened = Agent::open(
            vault,
            AgentConfig {
                data_dir: dir.path().join("data"),
                test: TestHooks {
                    extra_tools: vec![Arc::new(Probe {
                        name,
                        calls: Arc::new(AtomicUsize::new(0)),
                    })],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let error = match opened {
            Ok(_) => panic!("collision was silently hidden: {name}"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("Gateway tool name collision"),
            "{error:#}"
        );
        assert!(error.to_string().contains(name), "{error:#}");
    }
}

#[test]
fn gateway_help_and_discovery_match_the_enabled_host_surface() {
    let f = Fixture::new();
    let listing = f.agent.gateway_js_help(None);
    assert_eq!(
        listing.lines().next(),
        Some("Hosts: hub (online, default).")
    );
    assert!(listing.contains("tools.search_notes("), "{listing}");
    assert!(listing.contains("tools.chat_read("), "{listing}");
    assert!(listing.contains("tools.environments("), "{listing}");
    for absent in [
        "tools.history_",
        "tools.web",
        "tools.ui_",
        "tools.exec(",
        "tools.todo(",
        "tools.question(",
        "tools.reload(",
    ] {
        assert!(!listing.contains(absent), "{absent}: {listing}");
    }
    let help = f.agent.gateway_js_help(Some("search_notes"));
    assert!(help.contains("Input schema:"), "{help}");
    assert!(help.contains("query"), "{help}");
    let definition = f.agent.gateway_js_definition();
    assert_eq!(definition.name, "js");
    // The public schema and validation come from BubblePaw itself.
    assert_eq!(
        definition.input_schema,
        json!({
            "type":"object",
            "properties": {
                "code":{"type":"string"},
                "environment":{"type":"string"},
                "maxOutputTokens":{"type":"integer","minimum":256}
            },
            "required":["code"],
            "additionalProperties":false
        })
    );
    assert!(f
        .agent
        .gateway_js_help(Some("js"))
        .contains(&definition.description));
    assert!(!definition.description.contains("history_"));
    assert!(!definition.description.contains("ui_open_page"));

    let session = f.agent.create_session("gateway discovery").unwrap();
    assert_eq!(
        value(f.js(&session.id, json!({"code":"return await tools.hosts();"}))),
        json!([{"name":"hub","online":true,"default":true}])
    );
    let found = value(f.js(&session.id, json!({"code": "return {tools:await tools.$list(),exec:'exec' in tools,ui:'ui_open_page' in tools};"})));
    assert_eq!(found["exec"], false);
    assert_eq!(found["ui"], false);
    let names = found["tools"].as_array().unwrap();
    assert!(names.iter().any(|tool| tool["name"] == "search_notes"));
    assert!(!names
        .iter()
        .any(|tool| tool["name"] == "exec" || tool["name"] == "ui_open_page"));
    assert!(!names
        .iter()
        .any(|tool| tool["name"].as_str().unwrap().starts_with("history_")));
    let exec = f.js(
        &session.id,
        json!({"code":"return await tools.exec({command:'echo disabled'});"}),
    );
    assert!(exec.is_error, "{exec:?}");
    assert!(f
        .agent
        .session_state(&session.id)
        .unwrap()
        .pending_approvals
        .is_empty());
}

#[test]
fn gateway_vault_mutations_preserve_the_in_app_argument_restrictions() {
    let f = Fixture::new();
    let session = f.agent.create_session("restricted imports").unwrap();
    let run = f.js(&session.id, json!({"code":"return await tools.create_page({title:'Blocked gateway import',roots:[{name:'blocked',archive_base64:''}]});"}));
    assert!(run.is_error, "{run:?}");
    assert!(
        run.content.contains(vault_tools::CREATE_PAGE_ROOTS_ERROR),
        "{run:?}"
    );
    assert!(vault_tools::resolve_page(&f.vault, "Blocked gateway import").is_err());
}

#[test]
fn gateway_confirm_visibility_and_dispatch_follow_live_permissions_without_asking() {
    let f = Fixture::new();
    let session = f.agent.create_session("gateway approvals").unwrap();
    assert!(!f.agent.gateway_js_help(None).contains("test_confirm"));
    let denied = f.js(
        &session.id,
        json!({"code":"return await tools.test_confirm({text:'no wait'});"}),
    );
    assert!(denied.is_error, "{denied:?}");
    assert!(f
        .agent
        .session_state(&session.id)
        .unwrap()
        .pending_approvals
        .is_empty());

    f.agent.permission_mode_set(PermissionMode::Bypass).unwrap();
    assert!(f
        .agent
        .gateway_js_help(None)
        .contains("tools.test_confirm("));
    let allowed = value(f.js(
        &session.id,
        json!({"code":"return await tools.test_confirm({text:'allowed'});"}),
    ));
    assert_eq!(allowed, "confirmed: allowed");
    assert!(f
        .agent
        .session_state(&session.id)
        .unwrap()
        .pending_approvals
        .is_empty());
    f.agent.permission_mode_set(PermissionMode::Manual).unwrap();
    assert!(!f.agent.gateway_js_help(None).contains("test_confirm"));
    let hidden = value(f.js(
        &session.id,
        json!({"code":"const listed = (await tools.$list()).map(t => t.name); let call = 'ran'; try { await tools.test_confirm({text:'hidden'}); } catch (error) { call = 'refused'; } return {in: 'test_confirm' in tools, listed: listed.includes('test_confirm'), call};"}),
    ));
    assert_eq!(
        hidden,
        json!({"in":false, "listed":false, "call":"refused"})
    );
    assert!(f
        .agent
        .session_state(&session.id)
        .unwrap()
        .pending_approvals
        .is_empty());
}

#[test]
fn gateway_full_js_input_preserves_named_state_output_limits_and_session_isolation() {
    let f = Fixture::new();
    let first = f.agent.create_session("first MCP connection").unwrap();
    let other = f.agent.create_session("other MCP connection").unwrap();
    value(f.js(&first.id, json!({"code":"await tools.environments({action:'create',environment:'scratch'}); return true;"})));
    assert_eq!(
        value(f.js(
            &first.id,
            json!({"environment":"scratch","code":"state.marker=42; return state.marker;"})
        )),
        42
    );
    assert_eq!(
        value(f.js(&first.id, json!({"code":"return typeof state.marker;"}))),
        "undefined"
    );
    value(f.js(&other.id, json!({"code":"await tools.environments({action:'create',environment:'scratch'}); return true;"})));
    assert_eq!(
        value(f.js(
            &other.id,
            json!({"environment":"scratch","code":"return typeof state.marker;"})
        )),
        "undefined"
    );

    let limited = f.js(&first.id, json!({"environment":"scratch","maxOutputTokens":256,"code":"return 'large output '.repeat(3000);"}));
    assert!(!limited.is_error, "{limited:?}");
    let text = limited.content;
    assert!(text.contains("fullOutput"), "{text}");
    assert!(text.len() < 20_000, "{} bytes", text.len());
    assert_eq!(
        value(f.js(
            &first.id,
            json!({"environment":"scratch","code":"return state.marker;"})
        )),
        42
    );
}

#[test]
fn gateway_rejects_invalid_input_and_closing_sessions_reclaims_js_capacity() {
    let f = Fixture::new();
    let first = f.agent.create_session("invalid input").unwrap();
    for input in [
        json!({"code":"return 1;","unknown":true}),
        json!({"code":"return 1;","maxOutputTokens":255}),
        json!({"code":"return 1;","environment":null}),
    ] {
        assert!(
            f.agent
                .gateway_call_as(
                    &first.id,
                    "js",
                    input,
                    ClientInfo {
                        id: "mcp".into(),
                        name: None
                    }
                )
                .unwrap()
                .is_error
        );
    }
    f.agent.gateway_close_session(&first.id).unwrap();
    f.agent.gateway_close_session(&first.id).unwrap();
    // There are only sixteen environments globally. More sequential sessions
    // must succeed when explicit MCP teardown really closes and reclaims heaps.
    for _ in 0..17 {
        let session = f.agent.create_session("temporary MCP session").unwrap();
        assert_eq!(
            value(f.js(
                &session.id,
                json!({"code":"state.value=1; return state.value;"})
            )),
            1
        );
        f.agent.gateway_close_session(&session.id).unwrap();
        f.agent.gateway_close_session(&session.id).unwrap();
        assert!(f.agent.session_state(&session.id).is_err());
    }
}

#[test]
fn gateway_oauth_authors_are_scoped_to_the_mutation_worker_and_session() {
    let f = Fixture::new();
    for (title, client, actor) in [
        (
            "Ann gateway page",
            ClientInfo {
                id: "oauth:ann-sub".into(),
                name: Some("ann@example.com".into()),
            },
            "oauth:ann@example.com",
        ),
        (
            "Bob gateway page",
            ClientInfo {
                id: "oauth:bob-sub".into(),
                name: None,
            },
            "oauth:bob-sub",
        ),
    ] {
        let session = f.agent.create_session(title).unwrap();
        let run = f.agent.gateway_call_as(&session.id, "js", json!({"code":format!("await tools.create_page({{title:{},markdown:'OAuth attributed content'}}); return await tools.search_notes({{query:'OAuth attributed content'}});",serde_json::to_string(title).unwrap())}), client).unwrap();
        assert!(!run.is_error, "{run:?}");
        let page = vault_tools::resolve_page(&f.vault, title).unwrap();
        let versions = f.vault.list_versions(page).unwrap();
        assert!(!versions.is_empty());
        assert!(
            versions.iter().all(|version| version.device == actor),
            "{versions:?}"
        );
        f.agent.gateway_close_session(&session.id).unwrap();
    }
    let ordinary = f.agent.create_session("ordinary chat").unwrap();
    let run = f.agent.run_script(&ordinary.id, "create", "return await tools.create_page({title:'Ordinary agent page',markdown:'Normal actor'});").unwrap();
    assert_eq!(run.status, "completed", "{run:?}");
    let page = vault_tools::resolve_page(&f.vault, "Ordinary agent page").unwrap();
    let versions = f.vault.list_versions(page).unwrap();
    assert!(!versions.is_empty());
    assert!(
        versions
            .iter()
            .all(|version| !version.device.starts_with("oauth:")),
        "{versions:?}"
    );
}

#[test]
fn gateway_session_identity_is_stable_when_the_oauth_email_changes() {
    let f = Fixture::new();
    let session = f.agent.create_session("stable OAuth subject").unwrap();
    for (title, email) in [
        ("Before email change", "ann@example.com"),
        ("After email change", "renamed@example.com"),
    ] {
        let run = f.agent.gateway_call_as(
            &session.id,
            "js", json!({"code":format!("state.count=(state.count||0)+1; await tools.create_page({{title:{},markdown:'Same authenticated subject'}}); return state.count;",serde_json::to_string(title).unwrap())}),
            ClientInfo { id:"oauth:ann-sub".into(), name:Some(email.into()) },
        ).unwrap();
        assert_eq!(
            value(run),
            if title == "Before email change" { 1 } else { 2 }
        );
        let page = vault_tools::resolve_page(&f.vault, title).unwrap();
        let versions = f.vault.list_versions(page).unwrap();
        assert!(!versions.is_empty());
        assert!(
            versions
                .iter()
                .all(|version| version.device == format!("oauth:{email}")),
            "{versions:?}"
        );
    }
    // A matching display label cannot authorize another stable subject.
    let error = f
        .agent
        .gateway_call_as(
            &session.id,
            "js",
            json!({"code":"return state.count;"}),
            ClientInfo {
                id: "oauth:bob-sub".into(),
                name: Some("renamed@example.com".into()),
            },
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("gateway session author changed"),
        "{error:#}"
    );
    let run = f
        .agent
        .gateway_call_as(
            &session.id,
            "js",
            json!({"code":"return state.count;"}),
            ClientInfo {
                id: "oauth:ann-sub".into(),
                name: Some("renamed@example.com".into()),
            },
        )
        .unwrap();
    assert_eq!(value(run), 2);
    f.agent.gateway_close_session(&session.id).unwrap();
}
