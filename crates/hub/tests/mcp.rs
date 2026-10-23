//! The hub's agent MCP transport, using a real listener and shared vault.

use std::fs;
use std::path::PathBuf;

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, Server};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};
use vault_core::new_id;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    server: Server,
    store: AgentTokens,
    client: Client,
    url: String,
    read: String,
    write: String,
    everything: String,
    _data: TempDir,
}

impl Env {
    async fn new() -> Self {
        Self::with_import_limit(64 << 20).await
    }

    async fn with_import_limit(limit: usize) -> Self {
        let data = TempDir(std::env::temp_dir().join(format!("notes_hub_mcp_{}", new_id())));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            listen: listener.local_addr().unwrap().to_string(),
            data_dir: data.0.clone(),
            trust_proxy: false,
            import_source_limit_bytes: limit,
            agent: AgentSection {
                enabled: false,
                ..AgentSection::default()
            },
            ..Config::default()
        };
        let store = AgentTokens::open(config.vault_dir()).unwrap();
        let read = store.add("reader", AgentScope::Read).unwrap();
        let write = store.add("writer", AgentScope::Write).unwrap();
        let everything = store.add("everything", AgentScope::Everything).unwrap();
        let server = notes_hub::start_on(listener, config).await.unwrap();
        let url = format!("http://{}/mcp", server.addr());
        Self {
            server,
            store,
            client: Client::new(),
            url,
            read,
            write,
            everything,
            _data: data,
        }
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(token)
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        no_cors(&response);
        response.json().await.unwrap()
    }

    async fn call(&self, token: &str, name: &str, arguments: Value) -> Value {
        self.rpc(
            token,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        )
        .await
    }
}

fn no_cors(response: &Response) {
    assert!(response
        .headers()
        .keys()
        .all(|h| !h.as_str().starts_with("access-control-")));
}

fn tool_text(reply: &Value) -> &str {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    reply["result"]["content"][0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn initialize_and_scoped_tool_lists() {
    let env = Env::new().await;
    let initialized = env
        .rpc(
            &env.read,
            "initialize",
            json!({"protocolVersion":"2025-03-26"}),
        )
        .await;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
    assert!(initialized["result"]["capabilities"]["tools"].is_object());
    let list = env.rpc(&env.read, "tools/list", json!({})).await;
    let mut names: Vec<_> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    // Only the registry's read-only listed tools; agenda, query_database and
    // memory_search are hidden aliases of search_notes and are not listed.
    assert_eq!(
        names,
        ["memory_read", "notes_help", "read_page", "search_notes"]
    );
    let list = env.rpc(&env.write, "tools/list", json!({})).await;
    let listed: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        listed, LISTED,
        "tools/list over HTTP is exactly the 17 names, in order"
    );
    assert!(
        !listed.contains(&"feedback_list"),
        "feedback_list is hidden"
    );
    env.server.stop().await;
}

/// The names a token's `tools/list` shows, in serving order.
async fn listed_names(env: &Env, token: &str) -> Vec<String> {
    let list = env.rpc(token, "tools/list", json!({})).await;
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_token_lists_and_calls_only_its_tool_set() {
    let env = Env::new().await;
    // Everything sees every tool /mcp serves; on /mcp that is every vault
    // tool, the same as Write (old write tokens are unchanged).
    assert_eq!(listed_names(&env, &env.everything).await, LISTED);
    assert_eq!(listed_names(&env, &env.write).await, LISTED);
    let read = listed_names(&env, &env.read).await;
    assert_eq!(read, ["search_notes", "read_page", "memory_read", "notes_help"]);

    // Everything calls listed write tools, hidden tools and old names.
    let created = env
        .call(
            &env.everything,
            "create_page",
            json!({"title":"Set target","markdown":"setneedle"}),
        )
        .await;
    let page = id_in(tool_text(&created));
    tool_text(&env.call(&env.everything, "feedback_list", json!({"limit":1})).await);
    tool_text(
        &env.call(
            &env.everything,
            "append_blocks",
            json!({"page": page, "markdown": "everythingappended"}),
        )
        .await,
    );

    // A read token is refused a write tool with a clear JSON-RPC error.
    let refused = env
        .call(&env.read, "create_page", json!({"title":"Refused"}))
        .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    assert_eq!(
        refused["error"]["message"],
        "tool not available in read-only scope: create_page"
    );
    let read_back = env
        .call(&env.read, "read_page", json!({"page": page}))
        .await;
    assert!(tool_text(&read_back).contains("everythingappended"));

    // The token list reports the set under `scope`.
    let listed = env.store.list().unwrap();
    let everything = listed.iter().find(|t| t.name == "everything").unwrap();
    assert_eq!(everything.scope, AgentScope::Everything);
    env.server.stop().await;
}

/// The 17 listed MCP tools, in serving order.
const LISTED: [&str; 17] = [
    "search_notes",
    "read_page",
    "create_page",
    "edit_page",
    "attach_file",
    "trash_page",
    "move_page",
    "daily_note",
    "update_task",
    "upsert_row",
    "memory_read",
    "memory_write",
    "memory_update",
    "memory_delete",
    "feedback_submit",
    "notes_help",
    "jobs",
];

#[tokio::test]
async fn old_names_are_hidden_aliases_over_http_in_each_scope() {
    let env = Env::new().await;
    let created = env
        .call(
            &env.write,
            "create_page",
            json!({"title":"Alias target","markdown":"aliasneedle"}),
        )
        .await;
    let page = id_in(tool_text(&created));
    env.call(
        &env.write,
        "memory_write",
        json!({"path":"testproj/a", "description":"alias target", "type":"project", "body":"aliasmemory"}),
    )
    .await;
    // Read scope: old read-only names run (the same as the new tools).
    for (name, args) in [
        ("list_pages", json!({})),
        ("backlinks", json!({"page": page})),
        ("memory_list", json!({})),
        ("feedback_list", json!({"limit":1})),
    ] {
        let reply = env.call(&env.read, name, args).await;
        tool_text(&reply);
    }
    let listed = env.call(&env.read, "list_pages", json!({})).await;
    assert!(tool_text(&listed).contains("Alias target"));
    // Write scope: old write names run as their new tools do.
    let appended = env
        .call(
            &env.write,
            "append_blocks",
            json!({"page": page, "markdown": "aliasappended"}),
        )
        .await;
    tool_text(&appended);
    let read = env
        .call(&env.read, "read_page", json!({"page": page}))
        .await;
    assert!(tool_text(&read).contains("aliasappended"));
    // Read scope refuses the write aliases.
    let refused = env
        .call(
            &env.read,
            "append_blocks",
            json!({"page": page, "markdown": "deniedalias"}),
        )
        .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("read-only scope: append_blocks"));
    let read = env
        .call(&env.read, "read_page", json!({"page": page}))
        .await;
    assert!(!tool_text(&read).contains("deniedalias"));
    env.server.stop().await;
}

/// The id in "`<id>`" after a "Created" message.
fn id_in(text: &str) -> String {
    text.split('`').nth(1).expect("an id").to_string()
}

#[tokio::test]
async fn feedback_over_hub_http_preserves_scope_history_and_redacts_tokens() {
    let env = Env::new().await;
    let refused = env
        .call(&env.read, "feedback_submit", json!({"summary":"Denied"}))
        .await;
    assert_eq!(refused["error"]["code"], -32602);
    let reply = env
        .call(
            &env.write,
            "feedback_submit",
            json!({
                "summary":"Scratch lost a change", "kind":"complaint",
                "details":format!("Request used Bearer {}", env.write),
                "tool":"edit_page", "error":format!("https://example.test/?token={}", env.write)
            }),
        )
        .await;
    let report: Value = serde_json::from_str(tool_text(&reply)).unwrap();
    let id = report["id"].as_str().unwrap();
    let vault = env.server.hub().vault();
    let page = vault.load_page(id.into()).unwrap();
    assert_eq!(page.parent.as_deref(), Some("000000000000000000000000FB"));
    let text = page
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("transport: http"), "{text}");
    assert!(text.contains("source: mcp"), "{text}");
    assert!(!text.contains(&env.write));
    assert!(!text.contains(&env.read));
    assert!(vault
        .list_versions(id.into())
        .unwrap()
        .iter()
        .all(|v| v.device == "agent:writer"));
    let listed = env
        .call(
            &env.read,
            "feedback_list",
            json!({"kind":"complaint", "limit":1}),
        )
        .await;
    let text = tool_text(&listed);
    assert!(text.contains(id), "{text}");
    assert!(!text.contains(&env.write));
    env.server.stop().await;
}

/// Whether a search_notes reply lists the page titled `title`.
fn lists_page(text: &str, title: &str) -> bool {
    text.contains(&format!("**{title}** `"))
}

#[tokio::test]
async fn memory_paths_scopes_and_token_history_over_http() {
    let env = Env::new().await;
    for path in ["testproj/a", "testproj/sub/b"] {
        let reply = env
            .call(
                &env.write,
                "memory_write",
                json!({
                    "path":path, "description":format!("Description of {path}"),
                    "type":"project", "body":"memorytransportneedle"
                }),
            )
            .await;
        assert!(tool_text(&reply).contains(path));
    }
    let list = env
        .call(&env.read, "memory_search", json!({"scope":"testproj"}))
        .await;
    let text = tool_text(&list);
    // search_notes lists pages by title and location, not by memory path.
    assert!(lists_page(text, "a"), "{text}");
    assert!(lists_page(text, "b"), "{text}");
    for (scope, includes_a) in [("all", true), ("testproj", true), ("testproj/sub", false)] {
        let search = env
            .call(
                &env.read,
                "memory_search",
                json!({
                    "query":"memorytransportneedle", "scope":scope
                }),
            )
            .await;
        let text = tool_text(&search);
        assert_eq!(lists_page(text, "a"), includes_a, "{text}");
        assert!(lists_page(text, "b"), "{text}");
    }
    let read = env
        .call(&env.read, "memory_read", json!({"path":"testproj/a"}))
        .await;
    assert!(tool_text(&read).contains("memorytransportneedle"));
    for (name, args) in [
        (
            "memory_write",
            json!({"path":"testproj/denied", "description":"denied", "type":"user", "body":"denied"}),
        ),
        (
            "memory_update",
            json!({"path":"testproj/a", "body":"denied"}),
        ),
        ("memory_delete", json!({"path":"testproj/a"})),
    ] {
        let refused = env.call(&env.read, name, args).await;
        assert!(refused["error"]["code"].is_number(), "{refused}");
        assert!(refused.get("result").is_none());
    }
    let vault = env.server.hub().vault();
    let page = vault_mcp::tools::resolve_page(vault, "a").unwrap();
    // memory_read addresses the page by path and must not show its id. The
    // memory_search alias now prints search_notes' rows, which carry page ids,
    // so the listing is checked by title above instead.
    assert!(!tool_text(&read).contains(&page), "{read}");
    let versions = vault.list_versions(page.clone()).unwrap();
    assert!(!versions.is_empty());
    assert!(
        versions.iter().all(|v| v.device == "agent:writer"),
        "{versions:?}"
    );
    let unchanged = env
        .call(&env.read, "memory_read", json!({"path":"testproj/a"}))
        .await;
    assert!(tool_text(&unchanged).contains("memorytransportneedle"));
    assert!(!tool_text(&unchanged).contains("denied"));
    vault.flush().unwrap();
    let root = env._data.0.join("vault");
    env.server.stop().await;
    let reopened = vault_core::Vault::open(root.to_string_lossy().into_owned()).unwrap();
    let versions = reopened.list_versions(page).unwrap();
    assert!(!versions.is_empty());
    assert!(
        versions.iter().all(|v| v.device == "agent:writer"),
        "{versions:?}"
    );
}

#[tokio::test]
async fn reads_and_write_attribution_with_separate_actors() {
    let env = Env::new().await;
    let reply = env
        .call(
            &env.write,
            "create_page",
            json!({"title":"Agent test note","markdown":"agentneedle"}),
        )
        .await;
    let text = tool_text(&reply);
    let page = vault_mcp::tools::resolve_page(env.server.hub().vault(), "Agent test note").unwrap();
    assert!(text.contains(&page));
    let versions = env
        .server
        .hub()
        .vault()
        .list_versions(page.clone())
        .unwrap();
    assert!(!versions.is_empty());
    assert!(
        versions.iter().all(|v| v.device == "agent:writer"),
        "{versions:?}"
    );
    let read = env.call(&env.read, "read_page", json!({"page":page})).await;
    assert!(tool_text(&read).contains("agentneedle"));
    let search = env
        .call(&env.read, "search_notes", json!({"query":"agentneedle"}))
        .await;
    assert!(tool_text(&search).contains(&page));

    let refused = env
        .call(
            &env.read,
            "edit_page",
            json!({"page":page,"op":"append","markdown":"deniedmarker"}),
        )
        .await;
    assert!(refused["error"]["code"].is_number(), "{refused}");
    assert!(refused.get("result").is_none());
    let read = env.call(&env.read, "read_page", json!({"page":page})).await;
    assert!(!tool_text(&read).contains("deniedmarker"));

    // A store change is visible without reopening the server.
    let second = env.store.add("second-writer", AgentScope::Write).unwrap();
    let changed = env
        .call(
            &second,
            "edit_page",
            json!({"page":page,"op":"set_props","title":"Renamed by second"}),
        )
        .await;
    tool_text(&changed);
    let changed = env
        .call(
            &env.write,
            "edit_page",
            json!({"page":page,"op":"append","markdown":"first writer again"}),
        )
        .await;
    tool_text(&changed);
    let vault = env.server.hub().vault();
    vault
        .rename_page(page.clone(), "Ordinary hub write".into())
        .unwrap();
    let versions = vault.list_versions(page.clone()).unwrap();
    assert!(
        versions.iter().any(|v| v.device == "agent:second-writer"),
        "{versions:?}"
    );
    assert!(versions.iter().any(|v| v.device == "agent:writer"));
    assert_eq!(versions[0].device, vault.device_id());
    vault.flush().unwrap();
    let root = env._data.0.join("vault");
    env.server.stop().await;
    let reopened = vault_core::Vault::open(root.to_string_lossy().into_owned()).unwrap();
    let versions = reopened.list_versions(page).unwrap();
    assert!(versions.iter().any(|v| v.device == "agent:writer"));
    assert!(versions.iter().any(|v| v.device == "agent:second-writer"));
}

#[tokio::test]
async fn authentication_origin_method_and_live_revocation() {
    let env = Env::new().await;
    let device = env.server.hub().pair("test-device").unwrap();
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
    for token in [None, Some("bad-token"), Some(device.as_str())] {
        let mut req = env.client.post(&env.url).json(&request);
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let response = req.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        no_cors(&response);
        let body = response.text().await.unwrap();
        assert!(!body.contains("jsonrpc"));
        assert!(serde_json::from_str::<Value>(&body).is_err());
    }
    for origin in ["http://localhost", "https://evil.example", "null", ""] {
        let response = env
            .client
            .post(&env.url)
            .bearer_auth(&env.write)
            .header("Origin", origin)
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        no_cors(&response);
    }
    let response = env.client.get(&env.url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    no_cors(&response);
    env.rpc(&env.write, "tools/list", json!({})).await;
    env.store.revoke("writer").unwrap();
    let response = env
        .client
        .post(&env.url)
        .bearer_auth(&env.write)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    no_cors(&response);
    env.rpc(&env.read, "tools/list", json!({})).await;
    env.server.stop().await;
}

#[tokio::test]
async fn json_rpc_parse_errors_notifications_and_batches() {
    let env = Env::new().await;
    let response = env
        .client
        .post(&env.url)
        .bearer_auth(&env.read)
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply["error"]["code"], -32700);
    let response = env
        .client
        .post(&env.url)
        .bearer_auth(&env.read)
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(response.bytes().await.unwrap().is_empty());
    let response = env.client.post(&env.url).bearer_auth(&env.read).json(&json!([
        {"jsonrpc":"2.0","id":1,"method":"ping"},
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_page","arguments":{"title":"Forbidden"}}}
    ])).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply.as_array().unwrap().len(), 2);
    assert_eq!(reply[0]["result"], json!({}));
    assert!(reply[1]["error"].is_object());
    env.server.stop().await;
}

#[tokio::test]
async fn token_cli_works_while_the_hub_owns_the_vault() {
    use std::process::Command;

    let env = Env::new().await;
    let config = env._data.0.join("hub.toml");
    fs::write(
        &config,
        format!(
            "data_dir = {}\n",
            serde_json::to_string(&env._data.0.to_string_lossy()).unwrap()
        ),
    )
    .unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_notes-hub"))
            .arg("--config")
            .arg(&config)
            .arg("--agent-token")
            .args(args)
            .output()
            .unwrap()
    };
    let added = cli(&["add", "cli-writer", "--scope", "write"]);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let token = String::from_utf8(added.stdout).unwrap().trim().to_owned();
    assert_eq!(token.len(), 64);
    env.rpc(&token, "tools/list", json!({})).await;
    let listed = cli(&["list"]);
    assert!(listed.status.success());
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(listed.contains("cli-writer\twrite\t"));
    assert!(!listed.contains(&token));
    assert!(!listed.contains(&vault_core::sha256_hex(token.as_bytes())));
    let duplicate = cli(&["add", "cli-writer", "--scope", "read"]);
    assert!(!duplicate.status.success());
    assert!(duplicate.stdout.is_empty());
    let revoked = cli(&["revoke", "cli-writer"]);
    assert!(revoked.status.success());
    let response = env
        .client
        .post(&env.url)
        .bearer_auth(&token)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    env.server.stop().await;
}

#[test]
fn actor_context_is_scoped_by_vault_thread_and_unwind() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Barrier;
    use vault_core::Vault;

    let root = TempDir(std::env::temp_dir().join(format!("notes_actor_{}", new_id())));
    let other_root = TempDir(std::env::temp_dir().join(format!("notes_actor_{}", new_id())));
    let vault = Vault::open(root.0.to_string_lossy().into_owned()).unwrap();
    let other = Vault::open(other_root.0.to_string_lossy().into_owned()).unwrap();
    let label = |v: &Vault, page| v.list_versions(page).unwrap()[0].device.clone();
    vault.with_actor("agent:outer", || {
        let page = other.create_page(None, "Other vault".into()).unwrap();
        assert_eq!(label(&other, page), other.device_id());
        let page = vault.with_actor("agent:inner", || {
            vault.create_page(None, "Nested".into()).unwrap()
        });
        assert_eq!(label(&vault, page), "agent:inner");
        let page = vault.create_page(None, "Outer".into()).unwrap();
        assert_eq!(label(&vault, page), "agent:outer");
    });
    let panic = catch_unwind(AssertUnwindSafe(|| {
        vault.with_actor("agent:panicking", || panic!("test unwind"));
    }));
    assert!(panic.is_err());
    let page = vault.create_page(None, "After unwind".into()).unwrap();
    assert_eq!(label(&vault, page), vault.device_id());

    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            vault.with_actor("agent:concurrent-a", || {
                barrier.wait();
                vault.create_page(None, "Concurrent A".into()).unwrap()
            })
        });
        let second = scope.spawn(|| {
            vault.with_actor("agent:concurrent-b", || {
                barrier.wait();
                vault.create_page(None, "Concurrent B".into()).unwrap()
            })
        });
        assert_eq!(label(&vault, first.join().unwrap()), "agent:concurrent-a");
        assert_eq!(label(&vault, second.join().unwrap()), "agent:concurrent-b");
    });
}
