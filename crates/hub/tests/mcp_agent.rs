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
use vault_agent::{bubblepaw_core::ExecutionMode, TestHooks};
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
    static_token: String,
    _data: TempDir,
}

impl Env {
    async fn boot(gateway: bool, agent: bool) -> Self {
        Self::with_default_host(gateway, agent, "hub").await
    }

    async fn with_default_host(gateway: bool, agent: bool, default_host: &str) -> Self {
        Self::configured(gateway, agent, default_host, true, 64 << 20).await
    }

    async fn configured(
        gateway: bool,
        agent: bool,
        default_host: &str,
        enable_js: bool,
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
            mcp_agent_default_host: default_host.into(),
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
                execution_mode: ExecutionMode::Native,
                enable_js,
                default_model: "scripted/scripted".into(),
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
        self.send_at(&self.url, token, session, body).await
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
async fn gateway_lists_exact_vault_definitions_and_native_host_schemas_with_optional_js() {
    for enable_js in [false, true] {
        let env = Env::configured(true, true, "hub", enable_js, 64 << 20).await;
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
        for required in ["hosts", "environments", "chat_read"] {
            assert!(
                names.contains(required),
                "missing native host capability {required}: {reply}"
            );
        }
        for name in ["js", "js_help"] {
            assert_eq!(names.contains(name), enable_js, "{reply}");
        }
        for tool in host_tools {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{tool}");
            assert_eq!(schema["properties"]["host"]["type"], "string", "{tool}");
            assert!(
                !schema["required"]
                    .as_array()
                    .is_some_and(|fields| fields.iter().any(|field| field == "host")),
                "host must remain optional: {tool}"
            );
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
        if enable_js {
            for (name, expected) in [
                ("js", vec!["code", "environment", "host", "maxOutputTokens"]),
                ("js_help", vec!["host", "query"]),
            ] {
                let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
                let mut properties: Vec<_> = tool["inputSchema"]["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect();
                properties.sort_unstable();
                assert_eq!(properties, expected, "{tool}");
            }
        } else {
            for (name, arguments) in [("js", json!({"code":"return 1;"})), ("js_help", json!({}))] {
                let denied = env.call(&token, None, name, arguments).await;
                assert_eq!(denied["error"]["code"], -32602, "{denied}");
            }
        }
        env.stop().await;
    }
}

#[tokio::test]
async fn native_host_calls_require_owned_sessions_and_forward_runtime_validation() {
    let env = Env::configured(true, true, "hub", false, 64 << 20).await;
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
            json!({"action":"inspect","host":"hub"}),
        )
        .await;
    assert!(returned(&listed)["environments"].is_array(), "{listed}");
    // The selector must be stripped before the strict runtime schema validates input.
    let invalid = env
        .call(
            &token,
            Some(&session),
            "environments",
            json!({"action":"not-an-action","host":"hub"}),
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
    let env = Env::configured(true, false, "missing-global-host", false, 8192).await;
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
        let output: String = output
            .lines()
            .map(|line| match line.split_once("/open/") {
                Some((_, path)) if line.starts_with("link: ") => format!("link: /open/{path}"),
                _ => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("
");
        creates.push(output.replace(&id, "PAGE_ID"));
        let read = env
            .call_at(&notes, &token, None, "read_page", json!({"page":id}))
            .await;
        let gateway_read = env
            .call_at(&gateway, &token, None, "read_page", json!({"page":id}))
            .await;
        assert_eq!(read, gateway_read);
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
    let env = Env::configured(true, false, "hub", false, 12 << 20).await;
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
async fn top_level_hosts_reports_actual_availability_and_selection_without_a_session() {
    for (enabled, default_host) in [(true, ""), (true, "global-host"), (false, "hub")] {
        let env = Env::configured(true, enabled, default_host, false, 64 << 20).await;
        let token = env.ann();
        for (suffix, arguments, selected) in [
            (
                "",
                json!({}),
                if default_host.is_empty() {
                    "hub"
                } else {
                    default_host
                },
            ),
            ("?host=hub", json!({}), "hub"),
            ("?host=missing-url", json!({}), "missing-url"),
            ("?host=missing-url", json!({"host":"hub"}), "hub"),
            (
                "?host=hub",
                json!({"host":"missing-argument"}),
                "missing-argument",
            ),
        ] {
            let reply = env
                .call_at(
                    &format!("{}{suffix}", env.url),
                    &token,
                    None,
                    "hosts",
                    arguments,
                )
                .await;
            if selected == "hub" {
                assert_eq!(
                    returned(&reply),
                    json!([{"name":"hub","online":enabled,"default":true}]),
                    "{suffix}: {reply}"
                );
            } else {
                let error = tool_error(&reply);
                assert!(error.contains(selected), "{reply}");
                assert!(error.to_lowercase().contains("available"), "{reply}");
                assert!(error.contains("hub"), "{reply}");
            }
        }
        env.stop().await;
    }
}

#[tokio::test]
async fn native_host_selection_and_discovery_respect_argument_url_and_global_defaults() {
    let env = Env::configured(true, true, "global-host", false, 64 << 20).await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    for (suffix, arguments, expected) in [
        ("", json!({"action":"inspect"}), "global-host"),
        ("?host=url-host", json!({"action":"inspect"}), "url-host"),
        (
            "?host=url-host",
            json!({"action":"inspect","host":"argument-host"}),
            "argument-host",
        ),
    ] {
        let reply = env
            .call_at(
                &format!("{}{suffix}", env.url),
                &token,
                Some(&session),
                "environments",
                arguments,
            )
            .await;
        let error = tool_error(&reply);
        assert!(
            error.contains(expected)
                && error.contains("hub")
                && error.to_lowercase().contains("available"),
            "{reply}"
        );
    }
    for (suffix, arguments) in [
        ("?host=hub", json!({"action":"inspect"})),
        (
            "?host=missing-url",
            json!({"action":"inspect","host":"hub"}),
        ),
    ] {
        let reply = env
            .call_at(
                &format!("{}{suffix}", env.url),
                &token,
                Some(&session),
                "environments",
                arguments,
            )
            .await;
        assert!(returned(&reply)["environments"].is_array(), "{reply}");
    }
    for (suffix, expected) in [("", "global-host"), ("?host=missing-url", "missing-url")] {
        let response = env
            .send_at(
                &format!("{}{suffix}", env.url),
                Some(&token),
                None,
                rpc("tools/list", json!({})),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let reply: Value = response.json().await.unwrap();
        let error = reply["error"]["message"]
            .as_str()
            .expect("discovery must reject unknown selected hosts");
        assert!(error.contains(expected) && error.contains("hub"), "{reply}");
    }
    let response = env
        .send_at(
            &format!("{}?host=hub", env.url),
            Some(&token),
            None,
            rpc("tools/list", json!({})),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    assert!(reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "environments"));
    env.stop().await;
}

#[tokio::test]
async fn native_js_searches_the_shared_vault_and_attributes_writes() {
    let env = Env::new().await;
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
    let env = Env::new().await;
    let token = env.ann();
    let listed = env.call(&token, None, "js_help", json!({})).await;
    let output = success_text(&listed);
    for name in ["search_notes", "read_page", "create_page"] {
        assert!(output.contains(name), "{output}");
    }
    let help = env
        .call(
            &token,
            None,
            "js_help",
            json!({"query":"search_notes","host":"hub"}),
        )
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
async fn unknown_hosts_report_available_hosts_for_both_tools() {
    let env = Env::new().await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    for (name, arguments) in [
        (
            "js",
            json!({"code":"return 1;","host":"missing-workstation"}),
        ),
        (
            "js_help",
            json!({"query":"search_notes","host":"missing-workstation"}),
        ),
    ] {
        let reply = env.call(&token, Some(&session), name, arguments).await;
        let output = tool_error(&reply);
        assert!(output.contains("missing-workstation"), "{output}");
        assert!(output.to_lowercase().contains("available"), "{output}");
        assert!(output.contains("hub"), "{output}");
    }
    env.stop().await;
}

#[tokio::test]
async fn configured_default_host_applies_to_both_tools_and_explicit_host_overrides_it() {
    let env = Env::with_default_host(true, true, "preferred-workstation").await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    for (name, arguments) in [
        ("js", json!({"code":"return 23;"})),
        ("js_help", json!({"query":"search_notes"})),
    ] {
        let reply = env
            .call(&token, Some(&session), name, arguments.clone())
            .await;
        let output = tool_error(&reply);
        assert!(output.contains("preferred-workstation"), "{output}");
        assert!(output.contains("hub"), "{output}");
        let mut explicit = arguments;
        explicit["host"] = json!("hub");
        let reply = env.call(&token, Some(&session), name, explicit).await;
        if name == "js" {
            assert_eq!(returned(&reply), 23);
        } else {
            assert!(success_text(&reply).contains("search_notes"));
        }
    }
    env.stop().await;
}

#[tokio::test]
async fn connection_uri_host_is_preserved_by_oauth_challenges_and_resource_metadata() {
    let env = Env::new().await;
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
async fn host_selection_uses_argument_then_uri_then_global() {
    let env = Env::with_default_host(true, true, "global-default").await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    for (name, arguments) in [
        ("js", json!({"code":"return 41;"})),
        ("js_help", json!({"query":"search_notes"})),
    ] {
        let selected = env
            .call(&token, Some(&session), name, arguments.clone())
            .await;
        let output = tool_error(&selected);
        assert!(output.contains("global-default"), "{output}");
        assert!(output.to_lowercase().contains("available"), "{output}");
        assert!(output.contains("hub"), "{output}");

        let uri = format!("{}?host=uri-default", env.url);
        let selected = env
            .call_at(&uri, &token, Some(&session), name, arguments.clone())
            .await;
        assert!(tool_error(&selected).contains("uri-default"));
        let mut explicit = arguments.clone();
        explicit["host"] = json!("argument-default");
        let selected = env
            .call_at(&uri, &token, Some(&session), name, explicit)
            .await;
        assert!(tool_error(&selected).contains("argument-default"));

        let uri_hub = format!("{}?host=hub", env.url);
        let selected = env
            .call_at(&uri_hub, &token, Some(&session), name, arguments.clone())
            .await;
        if name == "js" {
            assert_eq!(returned(&selected), 41);
        } else {
            assert!(success_text(&selected).contains("search_notes"));
        }
        let mut explicit_hub = arguments;
        explicit_hub["host"] = json!("hub");
        let selected = env
            .call_at(&uri, &token, Some(&session), name, explicit_hub)
            .await;
        if name == "js" {
            assert_eq!(returned(&selected), 41);
        } else {
            assert!(success_text(&selected).contains("search_notes"));
        }
    }
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
async fn native_hosts_and_help_expose_hub_availability_and_default_fallback() {
    // An empty global host falls back to hub.
    let env = Env::with_default_host(true, true, "").await;
    let token = env.ann();
    let session = env.initialize(&token).await;
    let hosts = env
        .js(&token, &session, "return await tools.hosts();")
        .await;
    assert_eq!(
        returned(&hosts),
        json!([{"name":"hub","online":true,"default":true}])
    );
    for arguments in [
        json!({}),
        json!({"query":"search_notes"}),
        json!({"query":"js"}),
    ] {
        let reply = env.call(&token, None, "js_help", arguments).await;
        let output = success_text(&reply);
        let prefix = output.split("tools.").next().unwrap();
        assert!(
            output
                .lines()
                .next()
                .unwrap()
                .to_lowercase()
                .contains("hosts"),
            "{output}"
        );
        assert!(prefix.contains("hub"), "{output}");
        assert!(prefix.to_lowercase().contains("online"), "{output}");
        assert!(prefix.to_lowercase().contains("default"), "{output}");
    }
    env.stop().await;
}

#[tokio::test]
async fn unreachable_hub_reports_its_name_and_available_hosts() {
    let env = Env::boot(true, false).await;
    let reply = env
        .call(&env.ann(), None, "js_help", json!({"host":"hub"}))
        .await;
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
    let response = env.send(None, None, request.clone()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["www-authenticate"],
        format!("Bearer resource_metadata=\"{}\"", env.metadata())
    );
    // Static agent and sync device tokens remain valid only for their original transports.
    let device = env.server.hub().pair("gateway-test-device").unwrap();
    for token in [&env.static_token, &device] {
        let response = env.send(Some(token), None, request.clone()).await;
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
    let env = Env::new().await;
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
    let env = Env::new().await;
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
        "environment":"scratch", "maxOutputTokens":256, "host":"hub",
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
    let env = Env::new().await;
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
    let env = Env::new().await;
    let token = env.ann();
    let active_session = env.initialize(&token).await;
    let other_session = env.initialize(&token).await;
    let marker = "Suspended gateway script started";
    let client = env.client.clone();
    let url = env.url.clone();
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
                .post(&env.url)
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
