//! The OAuth-only BubblePaw gateway over a real HTTP listener and native runtime.
//! A local RS256 issuer keeps these tests independent of accounts and providers.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, McpOAuthConfig, OidcConfig, Server};
use openidconnect::core::{CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey};
use openidconnect::{JsonWebKeyId, PrivateSigningKey};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use vault_agent::{bubblepaw_sdk::harness::ExecutionMode, TestHooks};
use vault_core::new_id;

const CLIENT_ID: &str = "notes-mcp";
const ALLOWED_GROUP: &str = "notes-users";
const SESSION: &str = "Mcp-Session-Id";

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct IssuerState {
    url: String,
    key: CoreRsaPrivateSigningKey,
}

struct MockIssuer {
    shared: Arc<IssuerState>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl MockIssuer {
    async fn start() -> Self {
        static PEM: OnceLock<String> = OnceLock::new();
        let pem = PEM.get_or_init(|| {
            use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
            let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
            key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string()
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let shared = Arc::new(IssuerState {
            url: format!("http://{}", listener.local_addr().unwrap()),
            key: CoreRsaPrivateSigningKey::from_pem(
                pem,
                Some(JsonWebKeyId::new("gateway-test".into())),
            )
            .unwrap(),
        });
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            // Rejected users have no additional userinfo grants.
            .route(
                "/userinfo",
                get(|| async { Json(json!({"sub":"denied-sub"})) }),
            )
            .with_state(shared.clone());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self { shared, stop, task }
    }

    fn sign(&self, claims: &Value) -> String {
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header = b64(br#"{"alg":"RS256","typ":"JWT","kid":"gateway-test"}"#);
        let input = format!("{header}.{}", b64(claims.to_string().as_bytes()));
        let signature = self
            .shared
            .key
            .sign(
                &CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
                input.as_bytes(),
            )
            .unwrap();
        format!("{input}.{}", b64(&signature))
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("mock issuer shutdown hung")
            .unwrap();
    }
}

async fn discovery(State(issuer): State<Arc<IssuerState>>) -> Json<Value> {
    Json(json!({
        "issuer": issuer.url,
        "authorization_endpoint": format!("{}/authorize", issuer.url),
        "token_endpoint": format!("{}/token", issuer.url),
        "userinfo_endpoint": format!("{}/userinfo", issuer.url),
        "jwks_uri": format!("{}/jwks", issuer.url),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
    }))
}

async fn jwks(State(issuer): State<Arc<IssuerState>>) -> Json<Value> {
    Json(json!({"keys": [issuer.key.as_verification_key()]}))
}

struct Env {
    server: Server,
    issuer: MockIssuer,
    client: Client,
    base: String,
    url: String,
    hub_url: String,
    static_token: String,
    _data: TempDir,
}

impl Env {
    async fn boot(gateway: bool, agent: bool) -> Self {
        Self::configured(gateway, agent, ExecutionMode::Native, 64 << 20).await
    }

    /// The same hub with `execution_mode = "js"`: only the `js` and `js_help`
    /// entrypoints are offered.
    async fn new_js() -> Self {
        Self::configured(true, true, ExecutionMode::Js, 64 << 20).await
    }

    async fn configured(
        gateway: bool,
        agent: bool,
        execution_mode: ExecutionMode,
        import_limit: usize,
    ) -> Self {
        let data = TempDir(std::env::temp_dir().join(format!("notes_hub_mcp_agent_{}", new_id())));
        let issuer = MockIssuer::start().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let mut config = Config {
            listen: addr.to_string(),
            data_dir: data.0.clone(),
            public_url: format!("{base}/"),
            trust_proxy: false,
            import_source_limit_bytes: import_limit,
            mcp_agent_enabled: gateway,
            mcp_oauth: McpOAuthConfig {
                issuer: issuer.shared.url.clone(),
                client_id: CLIENT_ID.into(),
            },
            oidc: OidcConfig {
                allowed_users: vec!["ann@example.com".into(), "bob-sub".into()],
                allowed_groups: vec![ALLOWED_GROUP.into()],
                ..OidcConfig::default()
            },
            agent: AgentSection {
                enabled: agent,
                execution_mode,
                default_model: "scripted/scripted".into(),
                models: vec!["scripted/scripted".into()],
                ..AgentSection::default()
            },
            ..Config::default()
        };
        config.schedules.runner = false;
        let tokens = AgentTokens::open(config.vault_dir()).unwrap();
        let static_token = tokens
            .add("gateway-static-writer", AgentScope::Write)
            .unwrap();
        let hooks: notes_hub::AgentTestHooks = Arc::new(|| TestHooks {
            scripted: Some((vec![], "unused".into())),
            ..TestHooks::default()
        });
        let server = notes_hub::start_with(listener, config, Some(hooks))
            .await
            .unwrap();
        Self {
            server,
            issuer,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            url: format!("{base}/mcp/agent"),
            hub_url: format!("{base}/mcp/agent?host=hub"),
            base,
            static_token,
            _data: data,
        }
    }

    async fn new() -> Self {
        Self::boot(true, true).await
    }

    fn claims(&self, sub: &str, email: Option<&str>) -> Value {
        let now = chrono::Utc::now().timestamp();
        let mut claims = json!({
            "iss": self.issuer.shared.url,
            "aud": CLIENT_ID,
            "azp": CLIENT_ID,
            "sub": sub,
            "groups": [ALLOWED_GROUP],
            "iat": now,
            "exp": now + 300,
        });
        if let Some(email) = email {
            claims["email"] = json!(email);
        }
        claims
    }

    fn token(&self, sub: &str, email: Option<&str>) -> String {
        self.issuer.sign(&self.claims(sub, email))
    }

    fn ann(&self) -> String {
        self.token("ann-sub", Some("ann@example.com"))
    }

    fn metadata(&self) -> String {
        format!(
            "{}/.well-known/oauth-protected-resource/mcp/agent",
            self.base
        )
    }

    async fn send(&self, token: Option<&str>, session: Option<&str>, body: Value) -> Response {
        self.send_at(&self.hub_url, token, session, body).await
    }

    async fn send_at(
        &self,
        url: &str,
        token: Option<&str>,
        session: Option<&str>,
        body: Value,
    ) -> Response {
        let mut request = self.client.post(url).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(session) = session {
            request = request.header(SESSION, session);
        }
        request.send().await.unwrap()
    }

    async fn rpc(&self, token: &str, session: Option<&str>, method: &str, params: Value) -> Value {
        let response = self.send(Some(token), session, rpc(method, params)).await;
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }

    async fn call(
        &self,
        token: &str,
        session: Option<&str>,
        name: &str,
        arguments: Value,
    ) -> Value {
        self.rpc(
            token,
            session,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        )
        .await
    }

    async fn call_at(
        &self,
        url: &str,
        token: &str,
        session: Option<&str>,
        name: &str,
        arguments: Value,
    ) -> Value {
        let response = self
            .send_at(url, Some(token), session, call(name, arguments))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }

    async fn initialize(&self, token: &str) -> String {
        let response = self
            .send(
                Some(token),
                None,
                rpc(
                    "initialize",
                    json!({
                        "protocolVersion":"2025-03-26",
                        "capabilities": {},
                        "clientInfo": {"name":"gateway-http-test","version":"1"},
                    }),
                ),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let session = response
            .headers()
            .get(SESSION)
            .expect("initialize must return a session id")
            .to_str()
            .unwrap()
            .to_owned();
        assert!(!session.is_empty());
        let reply: Value = response.json().await.unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26", "{reply}");
        assert!(
            reply["result"]["capabilities"]["tools"].is_object(),
            "{reply}"
        );
        session
    }

    async fn js(&self, token: &str, session: &str, code: &str) -> Value {
        self.call(token, Some(session), "js", json!({"code":code}))
            .await
    }

    async fn stop(self) {
        self.server.stop().await;
        self.issuer.stop().await;
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
}

fn call(name: &str, arguments: Value) -> Value {
    rpc("tools/call", json!({"name":name,"arguments":arguments}))
}

fn text(reply: &Value) -> String {
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"]["content"]
        .as_array()
        .expect("MCP content")
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn success_text(reply: &Value) -> String {
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    text(reply)
}

fn returned(reply: &Value) -> Value {
    let output = success_text(reply);
    // Native CodeMode appends state-change diagnostics after the return value.
    let primary = output
        .split_once("\n[---]\n")
        .map_or(output.as_str(), |(value, _)| value);
    serde_json::from_str(primary)
        .unwrap_or_else(|error| panic!("expected a JSON return value: {error}: {output}"))
}

fn tool_error(reply: &Value) -> String {
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    text(reply)
}

#[tokio::test]
async fn gateway_lists_exact_vault_definitions_and_native_host_schemas_without_js() {
    let env = Env::configured(true, true, ExecutionMode::Native, 64 << 20).await;
    let token = env.ann();
    // Discovery is available before initialize and reflects this host's mode.
    let reply = env.rpc(&token, None, "tools/list", json!({})).await;
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: BTreeSet<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names.len(),
        tools.len(),
        "duplicate advertised tools: {reply}"
    );
    let vault_definitions = vault_mcp::tools::definitions();
    for expected in &vault_definitions {
        let actual = tools
            .iter()
            .find(|tool| tool["name"] == expected["name"])
            .unwrap();
        assert_eq!(
            actual, expected,
            "vault definitions must match /mcp exactly"
        );
        assert!(
            actual["inputSchema"]["properties"].get("host").is_none(),
            "{actual}"
        );
    }
    let vault_names: BTreeSet<_> =
        vault_tools::definitions_for(vault_tools::Surface::InAppAgent)
            .map(|tool| tool.name)
            .chain(
                vault_definitions
                    .iter()
                    .map(|tool| tool["name"].as_str().unwrap()),
            )
            .collect();
    let host_tools: Vec<_> = tools
        .iter()
        .filter(|tool| !vault_names.contains(tool["name"].as_str().unwrap()))
        .collect();
    // Locked to the hub: its runtime tools are listed, the computer picker is not.
    assert!(!names.contains("hosts"), "{reply}");
    for required in ["environments", "chat_read"] {
        assert!(
            names.contains(required),
            "missing native host capability {required}: {reply}"
        );
    }
    for name in ["js", "js_help"] {
        assert!(!names.contains(name), "{reply}");
    }
    for tool in host_tools {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{tool}");
        assert!(schema["properties"].get("host").is_none(), "{tool}");
        assert!(
            tool["description"]
                .as_str()
                .is_some_and(|description| !description.is_empty()),
            "{tool}"
        );
    }
    // These tools are real runtime capabilities, but are not gateway capabilities.
    for name in &names {
        assert!(
            !name.starts_with("history_")
                && !name.starts_with("web_")
                && !name.starts_with("ui_"),
            "{reply}"
        );
        assert!(
            !matches!(*name, "web" | "todo" | "question" | "reload"),
            "{reply}"
        );
    }
    assert!(
        !names.contains("import_markdown"),
        "hidden tools remain callable but unlisted"
    );
    // No in-app-only vault adapter may leak as an additional native host tool.
    for name in vault_names {
        assert_eq!(
            tools.iter().filter(|tool| tool["name"] == name).count(),
            usize::from(vault_definitions.iter().any(|tool| tool["name"] == name)),
            "{name}: {reply}"
        );
    }
    for (name, arguments) in [("js", json!({"code":"return 1;"})), ("js_help", json!({}))] {
        let denied = env.call(&token, None, name, arguments).await;
        assert_eq!(denied["error"]["code"], -32602, "{denied}");
    }
    env.stop().await;
}

#[tokio::test]
async fn native_host_calls_require_owned_sessions_and_forward_runtime_validation() {
    let env = Env::configured(true, true, ExecutionMode::Native, 64 << 20).await;
    let token = env.ann();
    let request = call("environments", json!({"action":"inspect"}));
    let response = env.send(Some(&token), None, request.clone()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = env
        .send(Some(&token), Some("missing-session"), request.clone())
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let session = env.initialize(&token).await;
    let listed = env
        .call(
            &token,
            Some(&session),
            "environments",
            json!({"action":"inspect"}),
        )
        .await;
    assert!(returned(&listed)["environments"].is_array(), "{listed}");
    // The strict runtime schema validates the arguments.
    let invalid = env
        .call(
            &token,
            Some(&session),
            "environments",
            json!({"action":"not-an-action"}),
        )
        .await;
    tool_error(&invalid);
    let other = env.token("other-sub", Some("ann@example.com"));
    let response = env.send(Some(&other), Some(&session), request).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    for name in ["history_search", "web", "todo", "question", "reload"] {
        let denied = env.call(&token, Some(&session), name, json!({})).await;
        assert_eq!(denied["error"]["code"], -32602, "{denied}");
    }
    env.stop().await;
}

fn archive(files: &[(&str, &[u8])]) -> String {
    let mut archive = tar::Builder::new(Vec::new());
    for (path, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, *bytes).unwrap();
    }
    base64::engine::general_purpose::STANDARD.encode(archive.into_inner().unwrap())
}

/// Drops the `http://host:port` base from page links (`.../open/<id>`): the
/// link base is process-wide, and hubs started by parallel tests change it.
fn links_by_path(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("http://") {
        out.push_str(&rest[..i]);
        let link = &rest[i..];
        match link.find("/open/") {
            Some(j) if !link[..j].contains(|c: char| c.is_whitespace() || c == '"') => {
                rest = &link[j..];
            }
            _ => {
                out.push_str("http://");
                rest = &link["http://".len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn import_archive(bytes: &[u8]) -> String {
    archive(&[("intro.md", bytes)])
}

fn import_arguments(title: &str, archive: &str, dry_run: bool) -> Value {
    json!({
        "name":title,"repo_path":format!("/repo/{title}"),"dry_run":dry_run,
        "roots":[{"name":"wiki","archive_base64":archive}]
    })
}

fn normalized_import_report(mut report: Value) -> Value {
    report["project_page"] = json!("PROJECT_ID");
    for root in report["roots"].as_array_mut().unwrap() {
        for (index, page) in root["report"]["top_pages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            assert!(page.is_string(), "import must return page IDs");
            *page = json!(format!("TOP_PAGE_{index}"));
        }
    }
    report
}

#[tokio::test]
async fn vault_calls_match_notes_mcp_without_a_session_or_available_execution_host() {
    let env = Env::configured(true, false, ExecutionMode::Native, 8192).await;
    let token = env.ann();
    let notes = format!("{}/mcp", env.base);
    let gateway = format!("{}?host=missing-url-host", env.url);
    let mut creates = Vec::new();
    // The unavailable host must have no effect on direct vault dispatch.
    for url in [&notes, &gateway] {
        let created = env
            .call_at(
                url,
                &token,
                None,
                "create_page",
                json!({"title":"Parity note","markdown":"parityneedle"}),
            )
            .await;
        let output = success_text(&created);
        let id = output
            .split('`')
            .nth(1)
            .expect("created page ID")
            .to_owned();
        let versions = env.server.hub().vault().list_versions(id.clone()).unwrap();
        assert!(!versions.is_empty());
        assert!(
            versions
                .iter()
                .all(|version| version.device == "oauth:ann@example.com"),
            "{versions:?}"
        );
        // The link base is process-wide and the last started hub sets it, so
        // compare links by path only.
        let output = links_by_path(&output);
        creates.push(output.replace(&id, "PAGE_ID"));
        let read = env
            .call_at(&notes, &token, None, "read_page", json!({"page":id}))
            .await;
        let gateway_read = env
            .call_at(&gateway, &token, None, "read_page", json!({"page":id}))
            .await;
        // Another test's hub can change the link base between the two reads.
        assert_eq!(
            links_by_path(&read.to_string()),
            links_by_path(&gateway_read.to_string())
        );
        assert!(success_text(&read).contains("parityneedle"));
    }
    assert_eq!(creates[0], creates[1]);
    for (name, arguments) in [
        ("read_page", json!({"page":"No such parity page"})),
        (
            "create_page",
            json!({"title":"Invalid import","roots":[{"name":"wiki","archive_base64":"invalid!"}]}),
        ),
        (
            "import_markdown",
            json!({"name":"Invalid import","roots":[{"name":"wiki","archive_base64":"invalid!"}]}),
        ),
        (
            "import_markdown",
            import_arguments("Oversized", &import_archive(&vec![b'x'; 9000]), true),
        ),
    ] {
        let direct = env
            .call_at(&notes, &token, None, name, arguments.clone())
            .await;
        let through_gateway = env.call_at(&gateway, &token, None, name, arguments).await;
        assert_eq!(
            through_gateway, direct,
            "{name} error output must match exactly"
        );
        assert!(tool_error(&direct).starts_with("Error: "), "{direct}");
    }
    let archive = import_archive(b"# Imported intro\n\nImported through MCP.\n");
    let mut reports = Vec::new();
    for (index, url) in [&notes, &gateway].into_iter().enumerate() {
        let title = format!("Parity project {index}");
        let imported = env
            .call_at(
                url,
                &token,
                None,
                "import_markdown",
                import_arguments(&title, &archive, false),
            )
            .await;
        let report = returned(&imported);
        let project = report["project_page"].as_str().unwrap().to_owned();
        let vault = env.server.hub().vault();
        let children = vault.list_children(Some(project.clone()), 0, 100).unwrap();
        assert!(!children.is_empty(), "{report}");
        for page in std::iter::once(project.clone()).chain(children.into_iter().map(|page| page.id))
        {
            let versions = vault.list_versions(page).unwrap();
            assert!(!versions.is_empty());
            assert!(
                versions
                    .iter()
                    .all(|version| version.device == "oauth:ann@example.com"),
                "{versions:?}"
            );
        }
        // Repeat via both transports.
        let repeat = env
            .call_at(
                &notes,
                &token,
                None,
                "import_markdown",
                import_arguments(&title, &archive, false),
            )
            .await;
        let repeat_gateway = env
            .call_at(
                &gateway,
                &token,
                None,
                "import_markdown",
                import_arguments(&title, &archive, false),
            )
            .await;
        assert_eq!(repeat, repeat_gateway);
        reports.push(normalized_import_report(report));
    }
    assert_eq!(
        reports[0], reports[1],
        "create_page import reports must match"
    );
    env.stop().await;
}

#[tokio::test]
async fn gateway_preserves_import_only_large_body_allowance() {
    let env = Env::configured(true, false, ExecutionMode::Native, 12 << 20).await;
    let token = env.ann();
    let archive = archive(&[("intro.md", b"intro"), ("large.txt", &vec![b'x'; 7 << 20])]);
    let mut reports = Vec::new();
    for url in [format!("{}/mcp", env.base), env.url.clone()] {
        let accepted = env
            .call_at(
                &url,
                &token,
                None,
                "import_markdown",
                import_arguments("Large parity import", &archive, true),
            )
            .await;
        reports.push(normalized_import_report(returned(&accepted)));
        let ordinary = env
            .send_at(
                &url,
                Some(&token),
                None,
                call(
                    "create_page",
                    json!({"title":"Large ordinary note","markdown":"x".repeat(9 << 20)}),
                ),
            )
            .await;
        assert_eq!(ordinary.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
    assert_eq!(reports[0], reports[1]);
    env.stop().await;
}

#[tokio::test]
async fn locked_mode_is_pinned_to_its_host() {
    let env = Env::new().await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    let listed = env.rpc(&token, None, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().all(|tool| tool["name"] != "hosts"), "{listed}");
    assert!(
        tools
            .iter()
            .all(|tool| tool["inputSchema"]["properties"].get("host").is_none()),
        "{listed}"
    );
    let hosts = env.call(&token, Some(&session), "hosts", json!({})).await;
    assert_eq!(hosts["error"]["code"], -32602, "{hosts}");
    assert_eq!(
        hosts["error"]["message"], "unknown or unavailable host tool: hosts",
        "{hosts}"
    );
    let missing = format!("{}?host=missing-host", env.url);
    let reply = env
        .send_at(&missing, Some(&token), None, rpc("tools/list", json!({})))
        .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let reply: Value = reply.json().await.unwrap();
    let error = reply["error"]["message"]
        .as_str()
        .expect("unknown host error");
    assert!(
        error.contains("missing-host") && error.contains("hub"),
        "{reply}"
    );
    let call = env
        .call_at(
            &missing,
            &token,
            Some(&session),
            "environments",
            json!({"action":"inspect"}),
        )
        .await;
    let error = tool_error(&call);
    assert!(
        error.contains("missing-host") && error.contains("hub"),
        "{call}"
    );
    env.stop().await;
}

#[tokio::test]
async fn native_js_searches_the_shared_vault_and_attributes_writes() {
    let env = Env::new_js().await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    let reply = env
        .js(
            &token,
            &session,
            r#"
        await tools.create_page({title:"Gateway searchable note",markdown:"gatewaysearchneedle"});
        return await tools.search_notes({query:"gatewaysearchneedle"});
    "#,
        )
        .await;
    let output = success_text(&reply);
    assert!(output.contains("Gateway searchable note"), "{output}");
    let vault = env.server.hub().vault();
    let page = vault_mcp::tools::resolve_page(vault, "Gateway searchable note").unwrap();
    let versions = vault.list_versions(page).unwrap();
    assert!(!versions.is_empty());
    assert!(
        versions
            .iter()
            .all(|version| version.device == "oauth:ann@example.com"),
        "{versions:?}"
    );
    env.stop().await;
}

#[tokio::test]
async fn help_lists_tools_and_searches_signatures_and_documentation_without_a_session() {
    let env = Env::new_js().await;
    let token = env.ann();
    let listed = env.call(&token, None, "js_help", json!({})).await;
    let output = success_text(&listed);
    for name in ["search_notes", "read_page", "create_page"] {
        assert!(output.contains(name), "{output}");
    }
    let help = env
        .call(&token, None, "js_help", json!({"query":"search_notes"}))
        .await;
    let output = success_text(&help);
    assert!(output.contains("tools.search_notes({"), "{output}");
    assert!(
        output.contains("query"),
        "signature must describe the arguments: {output}"
    );
    assert!(output.contains("Input schema:"), "{output}");
    let runtime = env
        .call(&token, None, "js_help", json!({"query":"js"}))
        .await;
    let output = success_text(&runtime);
    assert!(
        output.contains("state"),
        "runtime help must document persistent state: {output}"
    );
    assert!(output.contains("maxOutputTokens"), "{output}");
    env.stop().await;
}

#[tokio::test]
async fn connection_uri_host_is_preserved_by_oauth_challenges_and_resource_metadata() {
    let env = Env::new_js().await;
    for suffix in [
        "",
        "?host=hub",
        "?host=unavailable-laptop",
        "?host=hub&connection=cloud%20chat&connection=second",
    ] {
        let connection = format!("{}{suffix}", env.url);
        let metadata = format!("{}{suffix}", env.metadata());
        for (token, error) in [
            (None, ""),
            (Some("not.a.valid.token"), ", error=\"invalid_token\""),
        ] {
            let response = env
                .send_at(&connection, token, None, rpc("tools/list", json!({})))
                .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{connection}");
            assert_eq!(
                response.headers()["www-authenticate"],
                format!("Bearer resource_metadata=\"{metadata}\"{error}")
            );
        }
        let response = env.client.get(&metadata).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{metadata}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["resource"], connection, "{body}");
        assert_eq!(
            body["authorization_servers"],
            json!([env.issuer.shared.url]),
            "{body}"
        );
    }
    let unknown = env
        .call_at(
            &format!("{}?host=unavailable-laptop", env.url),
            &env.ann(),
            None,
            "js_help",
            json!({}),
        )
        .await;
    let output = tool_error(&unknown);
    assert!(output.contains("unavailable-laptop"), "{output}");
    assert!(output.to_lowercase().contains("available"), "{output}");
    assert!(output.contains("hub"), "{output}");
    env.stop().await;
}

#[tokio::test]
async fn oauth_tokens_and_discovery_are_shared_across_notes_and_agent_connection_urls() {
    let env = Env::new().await;
    let connections = [
        format!("{}/mcp", env.base),
        env.url.clone(),
        format!("{}?host=hub", env.url),
    ];
    let mut audiences = vec![json!(CLIENT_ID)];
    audiences.extend(
        connections
            .iter()
            .map(|resource| json!([CLIENT_ID, resource])),
    );
    for audience in audiences {
        let mut claims = env.claims("ann-sub", Some("ann@example.com"));
        claims["aud"] = audience;
        claims["scope"] = json!("openid email profile");
        let token = env.issuer.sign(&claims);
        // A token issued with any one resource can switch to all three URLs.
        for connection in &connections {
            let response = env
                .send_at(connection, Some(&token), None, rpc("tools/list", json!({})))
                .await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "{connection}, audience {}",
                claims["aud"]
            );
            let reply: Value = response.json().await.unwrap();
            assert!(reply["result"]["tools"].is_array(), "{reply}");
        }
    }
    for audience in [
        json!([CLIENT_ID, "https://unrelated.example/mcp"]),
        json!("different-client"),
        json!(env.url),
    ] {
        let mut claims = env.claims("ann-sub", Some("ann@example.com"));
        claims["aud"] = audience;
        let token = env.issuer.sign(&claims);
        for connection in &connections {
            let response = env
                .send_at(connection, Some(&token), None, rpc("tools/list", json!({})))
                .await;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{connection}, audience {}",
                claims["aud"]
            );
        }
    }
    let metadata_urls = [
        format!("{}/.well-known/oauth-protected-resource/mcp", env.base),
        env.metadata(),
        format!("{}?host=hub", env.metadata()),
    ];
    for (connection, metadata) in connections.iter().zip(metadata_urls) {
        let challenge = env
            .send_at(connection, None, None, rpc("tools/list", json!({})))
            .await;
        assert_eq!(challenge.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            challenge.headers()["www-authenticate"],
            format!("Bearer resource_metadata=\"{metadata}\"")
        );
        let response = env.client.get(metadata).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["resource"], connection.as_str(), "{body}");
        assert_eq!(
            body["authorization_servers"],
            json!([env.issuer.shared.url]),
            "{body}"
        );
        assert_eq!(
            body["scopes_supported"],
            json!(["openid", "email", "profile", "offline_access"]),
            "{body}"
        );
    }
    env.stop().await;
}

#[tokio::test]
async fn unreachable_hub_reports_its_name_and_available_hosts() {
    let env = Env::configured(true, false, ExecutionMode::Js, 64 << 20).await;
    let reply = env.call(&env.ann(), None, "js_help", json!({})).await;
    let output = tool_error(&reply);
    assert!(output.contains("hub"), "{output}");
    assert!(output.to_lowercase().contains("available"), "{output}");
    assert!(output.to_lowercase().contains("unavailable"), "{output}");
    let response = env
        .send(Some(&env.ann()), None, rpc("initialize", json!({})))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    env.stop().await;
}

#[tokio::test]
async fn disabled_gateway_and_metadata_are_404_while_notes_mcp_stays_available() {
    let env = Env::boot(false, false).await;
    for method in [
        reqwest::Method::GET,
        reqwest::Method::POST,
        reqwest::Method::DELETE,
    ] {
        let response = env
            .client
            .request(method.clone(), &env.url)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
        assert!(response.headers().get("www-authenticate").is_none());
    }
    let response = env.client.get(env.metadata()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = env
        .client
        .post(format!("{}/mcp", env.base))
        .bearer_auth(&env.static_token)
        .json(&rpc("tools/list", json!({})))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert!(body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "search_notes"));
    env.stop().await;
}

#[tokio::test]
async fn oauth_metadata_challenge_audience_allowlist_and_origins_are_shared_with_notes_mcp() {
    let env = Env::new().await;
    let metadata = env.client.get(env.metadata()).send().await.unwrap();
    assert_eq!(metadata.status(), StatusCode::OK);
    let metadata: Value = metadata.json().await.unwrap();
    assert_eq!(
        metadata,
        json!({
            "resource": env.url,
            "authorization_servers": [env.issuer.shared.url],
            "scopes_supported": ["openid","email","profile","offline_access"],
            "bearer_methods_supported": ["header"],
        })
    );
    let request = rpc("tools/list", json!({}));
    let response = env.send_at(&env.url, None, None, request.clone()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["www-authenticate"],
        format!("Bearer resource_metadata=\"{}\"", env.metadata())
    );
    // Static agent and sync device tokens remain valid only for their original transports.
    let device = env.server.hub().pair("gateway-test-device").unwrap();
    for token in [&env.static_token, &device] {
        let response = env
            .send_at(&env.url, Some(token), None, request.clone())
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()["www-authenticate"],
            format!(
                "Bearer resource_metadata=\"{}\", error=\"invalid_token\"",
                env.metadata()
            )
        );
    }
    let good = env.ann();
    let group_only = env.token("group-only-sub", Some("group-only@example.com"));
    let mut denied = env.claims("denied-sub", Some("denied@example.com"));
    denied["groups"] = json!([]);
    let denied = env.issuer.sign(&denied);
    let mut wrong_audience = env.claims("ann-sub", Some("ann@example.com"));
    wrong_audience["aud"] = json!("some-other-client");
    let wrong_audience = env.issuer.sign(&wrong_audience);
    let mut wrong_issuer = env.claims("ann-sub", Some("ann@example.com"));
    wrong_issuer["iss"] = json!("https://different-issuer.example");
    let wrong_issuer = env.issuer.sign(&wrong_issuer);
    for url in [&env.url, &format!("{}/mcp", env.base)] {
        for (token, expected) in [
            (&good, StatusCode::OK),
            (&group_only, StatusCode::OK),
            (&denied, StatusCode::FORBIDDEN),
            (&wrong_audience, StatusCode::UNAUTHORIZED),
            (&wrong_issuer, StatusCode::UNAUTHORIZED),
        ] {
            let response = env
                .client
                .post(url)
                .bearer_auth(token)
                .json(&request)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{url}");
        }
        for origin in [
            "https://claude.ai",
            "https://claude.com",
            "https://chatgpt.com",
        ] {
            let response = env
                .client
                .post(url)
                .bearer_auth(&good)
                .header("Origin", origin)
                .json(&request)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{url}, {origin}");
        }
        for origin in [
            "https://evil.example",
            "https://claude.ai.evil.example",
            "null",
            "",
        ] {
            let response = env
                .client
                .post(url)
                .bearer_auth(&good)
                .header("Origin", origin)
                .json(&request)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{url}, {origin}");
        }
        let duplicate_origin = env
            .client
            .post(url)
            .bearer_auth(&good)
            .header("Origin", "https://claude.ai")
            .header("Origin", "https://chatgpt.com")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(
            duplicate_origin.status(),
            StatusCode::FORBIDDEN,
            "{url}, duplicate Origin"
        );
    }
    env.stop().await;
}

#[tokio::test]
async fn sessions_preserve_state_are_isolated_and_are_bound_to_subject_not_email() {
    let env = Env::new_js().await;
    let ann = env.ann();
    // Both identities are allowed, deliberately with the same display email.
    let other_subject = env.token("other-sub", Some("ann@example.com"));
    let first = env.initialize(&ann).await;
    let second = env.initialize(&ann).await;
    assert_ne!(first, second);
    let stored = env
        .js(
            &ann,
            &first,
            "state.gatewayMarker = 'first-session'; return state.gatewayMarker;",
        )
        .await;
    assert_eq!(returned(&stored), "first-session");
    let again = env.js(&ann, &first, "return state.gatewayMarker;").await;
    assert_eq!(returned(&again), "first-session");
    let separate = env
        .js(&ann, &second, "return state.gatewayMarker ?? null;")
        .await;
    assert_eq!(returned(&separate), Value::Null);
    let response = env
        .send(
            Some(&other_subject),
            Some(&first),
            call(
                "js",
                json!({"code":"state.gatewayMarker = 'stolen'; return 1;"}),
            ),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // Token refresh and changed email for the same subject do not change ownership.
    let refreshed = env.token("ann-sub", Some("renamed@example.com"));
    let reply = env
        .js(&refreshed, &first, "return state.gatewayMarker;")
        .await;
    assert_eq!(returned(&reply), "first-session");
    let response = env
        .client
        .delete(&env.url)
        .bearer_auth(&other_subject)
        .header(SESSION, &first)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let reply = env.js(&ann, &first, "return state.gatewayMarker;").await;
    assert_eq!(returned(&reply), "first-session");
    env.stop().await;
}

#[tokio::test]
async fn named_environments_and_output_limits_reach_the_native_runtime() {
    let env = Env::new_js().await;
    let token = env.ann();
    let first = env.initialize(&token).await;
    let second = env.initialize(&token).await;
    let made = env
        .js(
            &token,
            &first,
            "await tools.environments({action:'create',environment:'scratch'}); return true;",
        )
        .await;
    assert_eq!(returned(&made), true);
    let args = json!({
        "code":"state.gatewayMarker = 'named-environment'; return state.gatewayMarker;",
        "environment":"scratch", "maxOutputTokens":256,
    });
    let stored = env.call(&token, Some(&first), "js", args).await;
    assert_eq!(returned(&stored), "named-environment");
    let main = env
        .js(&token, &first, "return state.gatewayMarker ?? null;")
        .await;
    assert_eq!(returned(&main), Value::Null);
    let read = json!({"code":"return state.gatewayMarker ?? null;","environment":"scratch"});
    let again = env.call(&token, Some(&first), "js", read.clone()).await;
    assert_eq!(returned(&again), "named-environment");
    let missing = env.call(&token, Some(&second), "js", read).await;
    tool_error(&missing);
    env.stop().await;
}

#[tokio::test]
async fn js_requires_a_live_session_and_delete_closes_only_that_session() {
    let env = Env::new_js().await;
    let token = env.ann();
    let request = call("js", json!({"code":"return 7;"}));
    let missing = env.send(Some(&token), None, request.clone()).await;
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    let unknown = env
        .send(Some(&token), Some("unknown-session"), request.clone())
        .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let first = env.initialize(&token).await;
    let second = env.initialize(&token).await;
    let response = env
        .client
        .delete(&env.url)
        .bearer_auth(&token)
        .header(SESSION, &first)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.bytes().await.unwrap().is_empty());
    let closed = env.send(Some(&token), Some(&first), request).await;
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);
    let reply = env.js(&token, &second, "return 7;").await;
    assert_eq!(returned(&reply), 7);
    env.stop().await;
}

#[tokio::test]
async fn deleting_an_active_session_cancels_suspended_js_without_waiting_for_its_gate() {
    let env = Env::new_js().await;
    let token = env.ann();
    let active_session = env.initialize(&token).await;
    let other_session = env.initialize(&token).await;
    let marker = "Suspended gateway script started";
    let client = env.client.clone();
    let url = env.hub_url.clone();
    let active_token = token.clone();
    let active_id = active_session.clone();
    let active = tokio::spawn(async move {
        client.post(url).bearer_auth(active_token).header(SESSION, active_id)
            .timeout(Duration::from_secs(15))
            .json(&call("js", json!({"code": format!(
                "await tools.create_page({{title:{},markdown:'entered active JavaScript'}}); await new Promise(() => {{}}); return 'unexpected completion';",
                serde_json::to_string(marker).unwrap(),
            )}))).send().await
    });

    // The vault marker proves the native script entered, while a busy result
    // proves its HTTP call still owns the execution gate. No timed JS sleep is
    // needed: this Promise stays pending until the session is cancelled.
    let observed_active = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if vault_mcp::tools::resolve_page(env.server.hub().vault(), marker).is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            let response = env
                .client
                .post(&env.hub_url)
                .bearer_auth(&token)
                .header(SESSION, &active_session)
                .timeout(Duration::from_secs(1))
                .json(&call("js", json!({"code":"return 'probe';"})))
                .send()
                .await
                .expect("busy probe must not wait for the active call");
            assert_eq!(response.status(), StatusCode::OK);
            let reply: Value = response.json().await.unwrap();
            if reply["result"]["isError"] == true {
                let output = tool_error(&reply);
                assert!(output.to_lowercase().contains("busy"), "{output}");
                break;
            } else {
                assert_eq!(returned(&reply), "probe");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    // Always attempt teardown before reporting a readiness failure, so a
    // suspended native runtime is cancelled even when this assertion regresses.
    let deleted = env
        .client
        .delete(&env.url)
        .bearer_auth(&token)
        .header(SESSION, &active_session)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .expect("DELETE must cancel active JS without waiting for its gate");
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(
        observed_active.is_ok(),
        "native JS did not reach its suspended active state"
    );
    let cancelled = tokio::time::timeout(Duration::from_secs(3), active)
        .await
        .expect("active HTTP call must settle after DELETE")
        .expect("active request task panicked")
        .expect("active HTTP call must settle before its request timeout");
    assert_eq!(cancelled.status(), StatusCode::OK);
    let cancelled: Value = cancelled.json().await.unwrap();
    tool_error(&cancelled);

    let closed = env
        .send(
            Some(&token),
            Some(&active_session),
            call("js", json!({"code":"return 1;"})),
        )
        .await;
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);
    let unaffected = env
        .js(
            &token,
            &other_session,
            "return 'other session still works';",
        )
        .await;
    assert_eq!(returned(&unaffected), "other session still works");
    env.stop().await;
}

// ------------------------------------------------- desktop hosts (/host/ws)

/// What a scripted desktop answers to one request: a result, or nothing
/// (the request then waits for the hub's timeout).
type Script = Arc<dyn Fn(&notes_agent_wire::Request) -> Option<Value> + Send + Sync>;

/// A scripted desktop over a real `/host/ws` socket. It records every frame
/// the hub sends.
struct FakeHost {
    seen: Arc<std::sync::Mutex<Vec<Value>>>,
    closed: Arc<tokio::sync::Notify>,
    is_closed: Arc<std::sync::atomic::AtomicBool>,
    stop: Option<oneshot::Sender<()>>,
}

impl FakeHost {
    async fn connect(base: &str, token: &str, device: &str, script: Script) -> FakeHost {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::http::HeaderValue;
        use tokio_tungstenite::tungstenite::Message;
        let url = format!("{}/host/ws", base.replacen("http://", "ws://", 1));
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        req.headers_mut()
            .insert("X-Device", HeaderValue::from_str(device).unwrap());
        let (socket, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let (mut sink, mut stream) = socket.split();
        let seen: Arc<std::sync::Mutex<Vec<Value>>> = Arc::default();
        let closed = Arc::new(tokio::sync::Notify::new());
        let is_closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let (seen2, closed2, is_closed2) = (seen.clone(), closed.clone(), is_closed.clone());
        tokio::spawn(async move {
            loop {
                let frame = tokio::select! {
                    _ = &mut stop_rx => {
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                    frame = stream.next() => frame,
                };
                let text = match frame {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                let value: Value = serde_json::from_str(text.as_str()).unwrap();
                seen2.lock().unwrap().push(value.clone());
                if value.get("id").is_none() {
                    continue;
                }
                let request: notes_agent_wire::Request = serde_json::from_value(value).unwrap();
                if let Some(result) = script(&request) {
                    let reply = notes_agent_wire::Response::ok(request.id.clone(), result);
                    let text = serde_json::to_string(&reply).unwrap();
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
            is_closed2.store(true, std::sync::atomic::Ordering::SeqCst);
            closed2.notify_waiters();
        });
        FakeHost {
            seen,
            closed,
            is_closed,
            stop: Some(stop_tx),
        }
    }

    /// Closes the socket from the desktop's side.
    fn disconnect(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }

    async fn wait_closed(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !self.is_closed.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(tokio::time::Instant::now() < deadline, "socket stayed open");
            let _ = tokio::time::timeout(Duration::from_millis(50), self.closed.notified()).await;
        }
    }

    fn seen(&self, method: &str) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|frame| frame["method"] == method)
            .cloned()
            .collect()
    }
}

/// A desktop offering one `read` tool that answers `answer`.
fn reader(answer: &'static str) -> Script {
    Arc::new(move |request| match request.method.as_str() {
        "tools" => Some(json!({"tools":[{
            "name":"read",
            "description":"Reads a file in the current project.",
            "input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]},
            "read_only":true
        }]})),
        "call" => Some(json!({"content":answer,"is_error":false,"pending":false})),
        "close_session" => Some(Value::Null),
        _ => None,
    })
}

async fn wait_online(env: &Env, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while env.server.hosts_online() != count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {count} hosts online, have {}",
            env.server.hosts_online()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_desktop_host_registers_lists_and_runs_a_routed_call() {
    let env = Env::new().await;
    let (device_token, name) = env.server.hub().login_device("dev-pc", "work-pc").unwrap();
    assert_eq!(name, "work-pc");
    let host = FakeHost::connect(&env.base, &device_token, "dev-pc", reader("file body")).await;
    wait_online(&env, 1).await;
    let token = env.ann();

    let at_host = format!("{}?host=work-pc", env.url);

    // Pinned to the desktop: its tools are listed next to the vault tools.
    let response = env
        .send_at(&at_host, Some(&token), None, rpc("tools/list", json!({})))
        .await;
    let listed: Value = response.json().await.unwrap();
    let names: BTreeSet<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains("read") && names.contains("read_page"));
    assert!(!names.contains("hosts"), "{listed}");
    let read = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "read")
        .unwrap();
    assert!(
        read["inputSchema"]["properties"].get("host").is_none(),
        "{read}"
    );

    let session = env.initialize(&token).await;
    let reply = env
        .call_at(
            &at_host,
            &token,
            Some(&session),
            "read",
            json!({"path":"notes.txt"}),
        )
        .await;
    assert_eq!(success_text(&reply), "file body");
    let reply = env
        .call_at(
            &at_host,
            &token,
            Some(&session),
            "read",
            json!({"path":"b.txt"}),
        )
        .await;
    assert_eq!(success_text(&reply), "file body");

    let calls = host.seen("call");
    assert_eq!(calls.len(), 2, "{calls:?}");
    let params = &calls[0]["params"];
    assert_eq!(params["name"], "read");
    assert_eq!(params["arguments"], json!({"path":"notes.txt"}));
    assert_eq!(params["caller"]["id"], "oauth:ann-sub");
    let relay_session = params["session"].as_str().unwrap();
    assert!(!relay_session.is_empty() && relay_session != session);
    assert_eq!(calls[1]["params"]["session"], relay_session);
    assert_eq!(calls[1]["params"]["arguments"], json!({"path":"b.txt"}));

    // Closing the MCP session closes its counterpart on the desktop.
    let response = env
        .client
        .delete(&env.url)
        .bearer_auth(&token)
        .header(SESSION, &session)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let closes = host.seen("close_session");
    assert_eq!(closes.len(), 1, "{closes:?}");
    assert_eq!(closes[0]["params"]["session"], relay_session);
    env.stop().await;
}

#[tokio::test]
async fn a_closed_host_connection_goes_offline_with_a_clear_error() {
    let env = Env::new().await;
    let (device_token, _) = env.server.hub().login_device("dev-pc", "work-pc").unwrap();
    let mut host = FakeHost::connect(&env.base, &device_token, "dev-pc", reader("x")).await;
    wait_online(&env, 1).await;
    let token = env.ann();
    let session = env.initialize(&token).await;

    host.disconnect();
    host.wait_closed().await;
    wait_online(&env, 0).await;

    let at_host = format!("{}?host=work-pc", env.url);
    let reply = env
        .call_at(
            &at_host,
            &token,
            Some(&session),
            "read",
            json!({"path":"a"}),
        )
        .await;
    assert_eq!(tool_error(&reply), "work-pc is offline");
    // Discovery against it fails the same way.
    let listed: Value = env
        .send_at(&at_host, Some(&token), None, rpc("tools/list", json!({})))
        .await
        .json()
        .await
        .unwrap();
    assert!(
        listed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("work-pc is offline"),
        "{listed}"
    );
    // A name never seen is still unknown.
    let other = format!("{}?host=other-pc", env.url);
    let reply = env
        .call_at(&other, &token, Some(&session), "read", json!({"path":"a"}))
        .await;
    assert!(tool_error(&reply).contains("unknown host"), "{reply}");
    env.stop().await;
}

#[tokio::test]
async fn a_reconnect_replaces_the_old_host_connection() {
    let env = Env::new().await;
    let (device_token, _) = env.server.hub().login_device("dev-pc", "work-pc").unwrap();
    let first = FakeHost::connect(&env.base, &device_token, "dev-pc", reader("first")).await;
    wait_online(&env, 1).await;
    let second = FakeHost::connect(&env.base, &device_token, "dev-pc", reader("second")).await;
    first.wait_closed().await;
    wait_online(&env, 1).await;

    let token = env.ann();
    let session = env.initialize(&token).await;
    let at_host = format!("{}?host=work-pc", env.url);
    let reply = env
        .call_at(
            &at_host,
            &token,
            Some(&session),
            "read",
            json!({"path":"a"}),
        )
        .await;
    assert_eq!(success_text(&reply), "second");
    assert!(first.seen("call").is_empty());
    assert_eq!(second.seen("call").len(), 1);
    env.stop().await;
}

#[tokio::test]
async fn agent_and_device_tokens_cannot_drive_a_host() {
    let env = Env::new().await;
    let (device_token, _) = env.server.hub().login_device("dev-pc", "work-pc").unwrap();
    let host = FakeHost::connect(&env.base, &device_token, "dev-pc", reader("x")).await;
    wait_online(&env, 1).await;
    let at_host = format!("{}?host=work-pc", env.url);
    for bearer in [env.static_token.as_str(), device_token.as_str()] {
        let response = env
            .send_at(
                &at_host,
                Some(bearer),
                None,
                call("read", json!({"path":"a"})),
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert!(host.seen("call").is_empty() && host.seen("tools").is_empty());
    // Without a device token there is no host socket at all.
    let refused = tokio_tungstenite::connect_async(
        format!("{}/host/ws", env.base.replacen("http://", "ws://", 1)).as_str(),
    )
    .await;
    assert!(refused.is_err());
    env.stop().await;
}

#[tokio::test]
async fn a_host_call_times_out_and_is_cancelled_on_the_desktop() {
    let env = Env::new().await;
    env.server.set_host_call_timeout(Duration::from_millis(300));
    let (device_token, _) = env.server.hub().login_device("dev-pc", "work-pc").unwrap();
    let base = reader("never");
    let silent: Script = Arc::new(move |request| {
        if request.method == "call" {
            None
        } else {
            base(request)
        }
    });
    let host = FakeHost::connect(&env.base, &device_token, "dev-pc", silent).await;
    wait_online(&env, 1).await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    let at_host = format!("{}?host=work-pc", env.url);
    let reply = env
        .call_at(
            &at_host,
            &token,
            Some(&session),
            "read",
            json!({"path":"a"}),
        )
        .await;
    assert!(tool_error(&reply).contains("did not finish"), "{reply}");
    let call_id = host.seen("call")[0]["id"].clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while host.seen("cancel").is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no cancel frame");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(host.seen("cancel")[0]["params"]["id"], call_id);
    // The session is free again for the next call.
    let reply = env
        .call_at(
            &at_host,
            &token,
            Some(&session),
            "read",
            json!({"path":"a"}),
        )
        .await;
    assert!(tool_error(&reply).contains("did not finish"), "{reply}");
    env.stop().await;
}

// Any-computer mode: `/mcp/agent` without `?host=`. Every computer tool names
// its computer; nothing is kept per MCP session.

const NO_HOST: &str = "Unknown computer (no host given). Call hosts.";

fn process_handle(label: &str) -> String {
    let n = if label == "laptop" { 1 } else { 2 };
    format!("process:00000000-0000-4000-8000-00000000000{n}")
}

/// A fake desktop advertising the core computer tools as the agent crate
/// defines them, plus `extra` MCP-server tools. `exec` answers a running
/// process with a handle of its own; other calls answer "<label> ran <tool>".
fn computer(label: &'static str, extra: Vec<Value>) -> Script {
    let mut tools: Vec<Value> = vault_agent::tools::core_computer_tool_definitions()
        .unwrap()
        .into_iter()
        .map(|tool| {
            json!({"name":tool.name,"description":tool.description,
                "input_schema":tool.input_schema,"read_only":tool.read_only})
        })
        .collect();
    tools.extend(extra);
    Arc::new(move |request| match request.method.as_str() {
        "tools" => Some(json!({ "tools": tools.clone() })),
        "call" => {
            let name = request.params["name"].as_str().unwrap_or("");
            let content = if name == "exec" {
                json!({"status":"running","handle":process_handle(label)}).to_string()
            } else {
                format!("{label} ran {name}")
            };
            Some(json!({"content":content,"is_error":false,"pending":false}))
        }
        "close_session" => Some(Value::Null),
        _ => None,
    })
}

fn grep_tool() -> Value {
    json!({"name":"mcp__files__grep","description":"Search text in files.",
        "input_schema":{"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]},
        "read_only":true})
}

/// Two computers online: "pc" (core tools only) and "laptop" (plus an
/// MCP-server tool).
async fn two_computers(env: &Env) -> (FakeHost, FakeHost) {
    let (pc_token, pc) = env.server.hub().login_device("dev-pc", "pc").unwrap();
    let (laptop_token, laptop) = env
        .server
        .hub()
        .login_device("dev-laptop", "laptop")
        .unwrap();
    assert_eq!((pc.as_str(), laptop.as_str()), ("pc", "laptop"));
    let pc = FakeHost::connect(&env.base, &pc_token, "dev-pc", computer("pc", vec![])).await;
    let laptop = FakeHost::connect(
        &env.base,
        &laptop_token,
        "dev-laptop",
        computer("laptop", vec![grep_tool()]),
    )
    .await;
    wait_online(env, 2).await;
    (pc, laptop)
}

async fn listed_tools(env: &Env, url: &str, token: &str, session: Option<&str>) -> Vec<Value> {
    let response = env
        .send_at(url, Some(token), session, rpc("tools/list", json!({})))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let listed: Value = response.json().await.unwrap();
    listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .clone()
}

fn names_of(tools: &[Value]) -> BTreeSet<String> {
    tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

/// A tagged result's computer line and the rest.
fn on(text: &str) -> (&str, &str) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    (
        first
            .strip_prefix("[on ")
            .and_then(|line| line.strip_suffix(']'))
            .unwrap_or_else(|| panic!("untagged result: {text}")),
        rest,
    )
}

#[tokio::test]
async fn any_computer_mode_lists_a_fixed_set_and_locked_mode_lists_no_new_tools() {
    let env = Env::new().await;
    let (_pc, _laptop) = two_computers(&env).await;
    let token = env.ann();
    let new_tools = ["hosts", "tool_search", "call_tool"];

    let any = listed_tools(&env, &env.url, &token, None).await;
    let names = names_of(&any);
    assert!(names.contains("read_page") && names.contains("search_notes"));
    for name in vault_agent::tools::CORE_COMPUTER_TOOLS
        .iter()
        .chain(&new_tools)
    {
        assert!(names.contains(*name), "{name} missing: {names:?}");
    }
    for absent in [
        "select_host",
        "mcp__files__grep",
        "js",
        "js_help",
        "environments",
        "chat_read",
        "project_list",
    ] {
        assert!(!names.contains(absent), "{absent} listed: {names:?}");
    }
    for tool in &any {
        let name = tool["name"].as_str().unwrap();
        let schema = &tool["inputSchema"];
        let computer_tool = vault_agent::tools::CORE_COMPUTER_TOOLS.contains(&name)
            || matches!(name, "tool_search" | "call_tool");
        assert_eq!(
            schema["properties"].get("host").is_some(),
            computer_tool,
            "{name}"
        );
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert_eq!(required.contains(&json!("host")), computer_tool, "{name}");
    }

    // Locked mode: the pinned computer's own list, no new tools.
    let laptop_url = format!("{}?host=laptop", env.url);
    let locked = names_of(&listed_tools(&env, &laptop_url, &token, None).await);
    assert!(locked.contains("read") && locked.contains("mcp__files__grep"));
    for name in new_tools {
        assert!(!locked.contains(name), "{name} in locked mode: {locked:?}");
    }

    // Schema drift: apart from host, the hub's core tools match what a
    // desktop advertises.
    let pc_url = format!("{}?host=pc", env.url);
    let pinned = listed_tools(&env, &pc_url, &token, None).await;
    for name in vault_agent::tools::CORE_COMPUTER_TOOLS {
        let find = |tools: &[Value]| {
            tools
                .iter()
                .find(|tool| tool["name"] == *name)
                .cloned()
                .unwrap_or_else(|| panic!("{name} not listed"))
        };
        let mut hub = find(&any);
        let schema = hub["inputSchema"].as_object_mut().unwrap();
        schema["properties"].as_object_mut().unwrap().remove("host");
        let required: Vec<Value> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|field| **field != "host")
            .cloned()
            .collect();
        if required.is_empty() && find(&pinned)["inputSchema"].get("required").is_none() {
            schema.remove("required");
        } else {
            schema.insert("required".into(), Value::Array(required));
        }
        assert_eq!(hub, find(&pinned), "{name}");
    }
    env.stop().await;
}

#[tokio::test]
async fn each_call_runs_on_the_computer_it_names_and_says_so() {
    let env = Env::new().await;
    let (pc, laptop) = two_computers(&env).await;
    let token = env.ann();
    let url = env.url.clone();
    let session = env.initialize(&token).await;

    let reply = env.call_at(&url, &token, None, "hosts", json!({})).await;
    let hosts: Value = serde_json::from_str(&success_text(&reply)).unwrap();
    assert_eq!(
        hosts,
        json!([{"name":"laptop","online":true},{"name":"pc","online":true}])
    );

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "read",
            json!({"path":"a.txt","host":"laptop"}),
        )
        .await;
    assert_eq!(success_text(&reply), "[on laptop]\nlaptop ran read");
    assert_eq!(reply["result"]["_meta"]["notes/host"], "laptop");
    let calls = laptop.seen("call");
    assert_eq!(calls.len(), 1);
    // The host argument is the hub's; the desktop gets the tool's own.
    assert_eq!(calls[0]["params"]["arguments"], json!({"path":"a.txt"}));
    assert!(pc.seen("call").is_empty());

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "read",
            // Host names match ignoring case; the result names the computer.
            json!({"path":"a.txt","host":"PC"}),
        )
        .await;
    assert_eq!(success_text(&reply), "[on pc]\npc ran read");
    assert_eq!(pc.seen("call").len(), 1);

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "exec",
            json!({"command":"echo hi","host":"laptop"}),
        )
        .await;
    let text = success_text(&reply);
    let (computer, body) = on(&text);
    assert_eq!(computer, "laptop");
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap()["handle"],
        process_handle("laptop")
    );
    assert_eq!(laptop.seen("call").len(), 2);
    env.stop().await;
}

#[tokio::test]
async fn missing_unknown_and_offline_computers_are_errors() {
    let env = Env::new().await;
    let (pc, mut laptop) = two_computers(&env).await;
    let token = env.ann();
    let url = env.url.clone();
    let session = env.initialize(&token).await;

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "exec",
            json!({"command":"echo hi"}),
        )
        .await;
    assert_eq!(tool_error(&reply), NO_HOST);
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "exec",
            json!({"command":"echo hi","host":"phone"}),
        )
        .await;
    assert_eq!(tool_error(&reply), "Unknown computer phone. Call hosts.");
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "tool_search",
            json!({"query":"grep"}),
        )
        .await;
    assert_eq!(tool_error(&reply), NO_HOST);
    // The hub runtime is not a computer here.
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "read",
            json!({"path":"a","host":"hub"}),
        )
        .await;
    assert_eq!(tool_error(&reply), "Unknown computer hub. Call hosts.");

    laptop.disconnect();
    laptop.wait_closed().await;
    wait_online(&env, 1).await;
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "exec",
            json!({"command":"echo hi","host":"laptop"}),
        )
        .await;
    assert_eq!(tool_error(&reply), "laptop is offline");
    let reply = env.call_at(&url, &token, None, "hosts", json!({})).await;
    let hosts: Value = serde_json::from_str(&success_text(&reply)).unwrap();
    assert_eq!(
        hosts,
        json!([{"name":"laptop","online":false},{"name":"pc","online":true}])
    );
    assert!(pc.seen("call").is_empty());
    env.stop().await;
}

#[tokio::test]
async fn two_clients_without_shared_state_both_work() {
    let env = Env::new().await;
    let (pc, laptop) = two_computers(&env).await;
    let url = env.url.clone();
    let ann = env.ann();
    let other = env.token("bob-sub", None);
    let first = env.initialize(&ann).await;
    let second = env.initialize(&other).await;

    for _ in 0..2 {
        let reply = env
            .call_at(
                &url,
                &ann,
                Some(&first),
                "read",
                json!({"path":"a","host":"laptop"}),
            )
            .await;
        assert_eq!(success_text(&reply), "[on laptop]\nlaptop ran read");
        let reply = env
            .call_at(
                &url,
                &other,
                Some(&second),
                "read",
                json!({"path":"a","host":"pc"}),
            )
            .await;
        assert_eq!(success_text(&reply), "[on pc]\npc ran read");
    }
    // A reconnected client (a new session) needs nothing from the old one.
    let third = env.initialize(&ann).await;
    let reply = env
        .call_at(
            &url,
            &ann,
            Some(&third),
            "read",
            json!({"path":"a","host":"pc"}),
        )
        .await;
    assert_eq!(success_text(&reply), "[on pc]\npc ran read");
    assert_eq!(laptop.seen("call").len(), 2);
    assert_eq!(pc.seen("call").len(), 3);
    env.stop().await;
}

#[tokio::test]
async fn a_process_is_polled_only_on_the_computer_that_started_it() {
    let env = Env::new().await;
    let (pc, laptop) = two_computers(&env).await;
    let token = env.ann();
    let url = env.url.clone();
    let session = env.initialize(&token).await;
    let handle = process_handle("laptop");

    env.call_at(
        &url,
        &token,
        Some(&session),
        "exec",
        json!({"command":"sleep 1","host":"laptop"}),
    )
    .await;
    for (tool, arguments) in [
        ("poll", json!({"handle":handle,"host":"pc"})),
        ("wait", json!({"handle":handle,"host":"pc"})),
        ("cancel", json!({"handle":handle,"host":"pc"})),
        // The bare uuid names the same process.
        (
            "poll",
            json!({"handle":handle.strip_prefix("process:").unwrap(),"host":"pc"}),
        ),
    ] {
        let reply = env
            .call_at(&url, &token, Some(&session), tool, arguments.clone())
            .await;
        let shown = arguments["handle"].as_str().unwrap().to_owned();
        assert_eq!(
            tool_error(&reply),
            format!(
                "Process {shown} was started on laptop, not pc. Call {tool} with host \"laptop\"."
            )
        );
    }
    assert!(pc.seen("call").is_empty());

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "poll",
            json!({"handle":handle,"host":"laptop"}),
        )
        .await;
    assert_eq!(success_text(&reply), "[on laptop]\nlaptop ran poll");
    assert_eq!(
        laptop.seen("call")[1]["params"]["arguments"],
        json!({"handle":handle})
    );
    env.stop().await;
}

#[tokio::test]
async fn tool_search_and_call_tool_reach_one_computers_mcp_server_tools() {
    let env = Env::new().await;
    let (pc, laptop) = two_computers(&env).await;
    let token = env.ann();
    let url = env.url.clone();
    let session = env.initialize(&token).await;

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "tool_search",
            json!({"query":"grep","host":"laptop"}),
        )
        .await;
    let text = success_text(&reply);
    let (computer, body) = on(&text);
    assert_eq!(computer, "laptop");
    let found: Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        found,
        json!([{"name":"mcp__files__grep","description":"Search text in files.",
            "inputSchema":{"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}}])
    );
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "tool_search",
            json!({"query":"grep","host":"pc"}),
        )
        .await;
    assert_eq!(success_text(&reply), "[on pc]\n[]");
    // Built-in tools are not search results.
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "tool_search",
            json!({"query":"","host":"pc"}),
        )
        .await;
    assert_eq!(success_text(&reply), "[on pc]\n[]");
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "tool_search",
            json!({"query":"SEARCH files","host":"laptop"}),
        )
        .await;
    assert!(success_text(&reply).contains("mcp__files__grep"));

    let arguments = json!({"pattern":"todo"});
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "call_tool",
            json!({"name":"mcp__files__grep","arguments":arguments,"host":"laptop"}),
        )
        .await;
    assert_eq!(
        success_text(&reply),
        "[on laptop]\nlaptop ran mcp__files__grep"
    );
    let calls = laptop.seen("call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["params"]["name"], "mcp__files__grep");
    assert_eq!(calls[0]["params"]["arguments"], arguments);

    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "call_tool",
            json!({"name":"mcp__files__grep","arguments":arguments,"host":"pc"}),
        )
        .await;
    assert_eq!(
        tool_error(&reply),
        "[on pc]\npc has no tool mcp__files__grep; use tool_search"
    );
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "call_tool",
            json!({"name":"mcp__files__grep","arguments":arguments}),
        )
        .await;
    assert_eq!(tool_error(&reply), NO_HOST);
    let reply = env
        .call_at(
            &url,
            &token,
            Some(&session),
            "call_tool",
            json!({"name":"exec","host":"pc"}),
        )
        .await;
    assert!(tool_error(&reply).starts_with("call_tool runs a computer's MCP-server tools"));
    assert!(pc.seen("call").is_empty());
    env.stop().await;
}

#[tokio::test]
async fn agent_and_device_tokens_cannot_drive_any_computer() {
    let env = Env::new().await;
    let (pc, laptop) = two_computers(&env).await;
    let (device_token, _) = env.server.hub().login_device("dev-other", "other").unwrap();
    for bearer in [env.static_token.as_str(), device_token.as_str()] {
        for body in [
            call("hosts", json!({})),
            call("exec", json!({"command":"echo hi","host":"laptop"})),
            call(
                "call_tool",
                json!({"name":"mcp__files__grep","host":"laptop"}),
            ),
        ] {
            let response = env.send_at(&env.url, Some(bearer), None, body).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }
    for host in [&pc, &laptop] {
        assert!(host.seen("call").is_empty() && host.seen("tools").is_empty());
    }
    env.stop().await;
}

async fn rename_device(env: &Env, token: &str, device: &str, name: &str) -> (StatusCode, Value) {
    let reply = env
        .client
        .post(format!("{}/device/name", env.base))
        .bearer_auth(token)
        .header("X-Device", device)
        .json(&json!({ "name": name }))
        .send()
        .await
        .unwrap();
    let status = reply.status();
    (status, reply.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_renamed_computer_routes_by_its_new_name_and_names_stay_unique_among_hosts() {
    let env = Env::new().await;
    let hub = env.server.hub();
    let (desk_token, _) = hub.login_device("dev-desk", "pc").unwrap();
    let _desk = FakeHost::connect(&env.base, &desk_token, "dev-desk", computer("pc", vec![])).await;
    let (laptop_token, _) = hub.login_device("dev-laptop", "laptop").unwrap();
    let _laptop = FakeHost::connect(
        &env.base,
        &laptop_token,
        "dev-laptop",
        computer("laptop", vec![]),
    )
    .await;
    wait_online(&env, 2).await;
    // A sign-in that never ran the relay, and a host that is offline (as
    // after a hub restart: marked in tokens.json, no connection).
    hub.login_device("dev-stale", "Desk").unwrap();
    hub.login_device("dev-office", "Office").unwrap();
    hub.mark_host("dev-office").unwrap();
    let token = env.ann();
    let url = env.url.clone();
    let session = env.initialize(&token).await;

    // The stale sign-in does not hold "Desk".
    let (status, body) = rename_device(&env, &desk_token, "dev-desk", "  Desk ").await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({"name":"Desk"})));
    let reply = env
        .call_at(&url, &token, Some(&session), "read", json!({"path":"a","host":"desk"}))
        .await;
    assert_eq!(success_text(&reply), "[on Desk]\npc ran read");
    let reply = env
        .call_at(&url, &token, Some(&session), "read", json!({"path":"a","host":"pc"}))
        .await;
    assert_eq!(tool_error(&reply), "Unknown computer pc. Call hosts.");
    let locked = format!("{url}?host=Desk");
    let reply = env
        .call_at(&locked, &token, Some(&session), "read", json!({"path":"a"}))
        .await;
    assert_eq!(success_text(&reply), "pc ran read");
    let reply = env.call_at(&url, &token, None, "hosts", json!({})).await;
    let hosts: Value = serde_json::from_str(&success_text(&reply)).unwrap();
    assert_eq!(
        hosts,
        json!([
            {"name":"Desk","online":true},
            {"name":"Office","online":false},
            {"name":"laptop","online":true}
        ])
    );

    // Another host's name is refused, ignoring case, offline hosts included.
    for taken in ["DESK", "office"] {
        let (status, body) = rename_device(&env, &laptop_token, "dev-laptop", taken).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            body["error"],
            format!("Another computer is already called {taken}")
        );
    }
    let reply = env
        .call_at(&url, &token, Some(&session), "read", json!({"path":"a","host":"office"}))
        .await;
    assert_eq!(tool_error(&reply), "Office is offline");
    let (status, _) = rename_device(&env, &laptop_token, "dev-laptop", " ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = rename_device(&env, "wrong", "dev-laptop", "x").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let reply = env
        .call_at(&url, &token, Some(&session), "read", json!({"path":"a","host":"laptop"}))
        .await;
    assert_eq!(success_text(&reply), "[on laptop]\nlaptop ran read");
}
