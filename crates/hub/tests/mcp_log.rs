//! One structured log line per MCP tools/call, captured from the hub's own
//! tracing output. This binary installs a global subscriber, so it holds a
//! single test and no other tests.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, Server};
use reqwest::Client;
use serde_json::{json, Value};
use tracing_subscriber::fmt::MakeWriter;
use vault_core::new_id;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

/// Everything logged so far. The first call installs the global subscriber.
fn captured() -> String {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    let capture = CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("one subscriber");
        capture
    });
    String::from_utf8(capture.0.lock().unwrap().clone()).unwrap()
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    server: Server,
    client: Client,
    url: String,
    read: String,
    write: String,
    _data: TempDir,
}

impl Env {
    async fn new() -> Self {
        let data = TempDir(std::env::temp_dir().join(format!("notes_hub_log_{}", new_id())));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            listen: listener.local_addr().unwrap().to_string(),
            data_dir: data.0.clone(),
            trust_proxy: false,
            agent: AgentSection {
                enabled: false,
                ..AgentSection::default()
            },
            ..Config::default()
        };
        let store = AgentTokens::open(config.vault_dir()).unwrap();
        let read = store.add("reader", AgentScope::Read).unwrap();
        let write = store.add("writer", AgentScope::Write).unwrap();
        let server = notes_hub::start_on(listener, config).await.unwrap();
        let url = format!("http://{}/mcp", server.addr());
        Self {
            server,
            client: Client::new(),
            url,
            read,
            write,
            _data: data,
        }
    }

    async fn post(&self, token: &str, body: &Value) -> Value {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }

    async fn call(&self, token: &str, name: &str, arguments: Value) -> Value {
        self.post(
            token,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
        )
        .await
    }
}

/// The log line for `tool` with `error`, if one was written.
fn line<'a>(logs: &'a str, tool: &str, error: &str) -> Option<&'a str> {
    logs.lines().find(|l| {
        l.contains("notes_hub::mcp_call")
            && l.contains(&format!(" tool={tool} "))
            && l.contains(&format!(" error={error} "))
    })
}

#[tokio::test]
async fn each_tools_call_logs_names_and_outcome_but_no_values() {
    // Install the capture before the first call, so every line is recorded.
    captured();
    let env = Env::new().await;
    let created = env
        .call(
            &env.write,
            "create_page",
            json!({"title":"SECRET-TITLE-456","markdown":"SECRET-BODY-789"}),
        )
        .await;
    assert_eq!(created["result"]["isError"], false, "{created}");
    let found = env
        .call(
            &env.read,
            "search_notes",
            json!({"query":"SECRET-QUERY-VALUE-123"}),
        )
        .await;
    assert_eq!(found["result"]["isError"], false, "{found}");
    // An unknown tool name is refused by the protocol layer before any tool runs.
    let unknown = env
        .call(
            &env.read,
            "no_such_tool",
            json!({"secret":"SECRET-UNKNOWN-ARG-000"}),
        )
        .await;
    assert_eq!(unknown["error"]["code"], -32602, "{unknown}");
    // A write tool from the read scope is denied.
    let denied = env
        .call(
            &env.read,
            "create_page",
            json!({"title":"SECRET-DENIED-111"}),
        )
        .await;
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
    // The tool runs and reports an error to the assistant (create_page no
    // longer takes archive roots).
    let failed = env
        .call(
            &env.write,
            "create_page",
            json!({"title":"SECRET-ROOTS-666","roots":[{"name":"SECRET-ROOT-777"}]}),
        )
        .await;
    assert_eq!(failed["result"]["isError"], true, "{failed}");
    // A call without a tool name.
    let nameless = env
        .post(
            &env.read,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{"secret":"SECRET-NAMELESS-333"}}}),
        )
        .await;
    assert_eq!(nameless["error"]["code"], -32602, "{nameless}");
    // A batch makes one line per call.
    let batch = env
        .post(
            &env.read,
            &json!([
                {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_page","arguments":{"page":"SECRET-BATCH-444"}}},
                {"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_notes","arguments":{"query":"SECRET-TOPIC-555"}}}
            ]),
        )
        .await;
    assert_eq!(batch.as_array().unwrap().len(), 2, "{batch}");

    let logs = captured();
    let created_line = line(&logs, "create_page", "none")
        .unwrap_or_else(|| panic!("no create_page line in:\n{logs}"));
    assert!(
        created_line.contains("arg_keys=markdown,title"),
        "{created_line}"
    );
    assert!(created_line.contains("result_bytes="), "{created_line}");
    assert!(created_line.contains("duration_ms="), "{created_line}");
    let found_line = line(&logs, "search_notes", "none").expect("search_notes line");
    assert!(found_line.contains("arg_keys=query"), "{found_line}");
    let unknown_line = line(&logs, "unknown", "unknown_tool").expect("unknown tool line");
    assert!(unknown_line.contains("arg_keys=secret"), "{unknown_line}");
    let denied_line = line(&logs, "create_page", "denied").expect("denied line");
    assert!(denied_line.contains("arg_keys=title"), "{denied_line}");
    let failed_line = line(&logs, "create_page", "tool").expect("tool error line");
    assert!(
        failed_line.contains("arg_keys=roots,title"),
        "{failed_line}"
    );
    assert!(line(&logs, "unknown", "invalid_args").is_some(), "{logs}");
    // Eight calls (two of them in one batch) make eight lines, no more.
    assert_eq!(
        logs.lines()
            .filter(|l| l.contains("notes_hub::mcp_call"))
            .count(),
        8,
        "one line per tools/call: {logs}"
    );

    // No argument value, query, page content or token reaches the log.
    for secret in [
        "SECRET-TITLE-456",
        "SECRET-BODY-789",
        "SECRET-QUERY-VALUE-123",
        "SECRET-UNKNOWN-ARG-000",
        "SECRET-DENIED-111",
        "SECRET-ROOTS-666",
        "SECRET-ROOT-777",
        "SECRET-NAMELESS-333",
        "SECRET-BATCH-444",
        "SECRET-TOPIC-555",
        env.write.as_str(),
        env.read.as_str(),
    ] {
        assert!(!logs.contains(secret), "{secret} leaked into logs: {logs}");
    }
    env.server.stop().await;
}
