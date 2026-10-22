//! JSON-RPC 2.0 handling for MCP: `initialize`, `ping`, `tools/list` and
//! `tools/call`. Transport-independent: [`McpServer::handle`] takes one
//! message and returns the reply (None for notifications and responses),
//! and [`McpServer::handle_body`] takes the bytes of one HTTP POST body and
//! returns the status and body to send back.
//!
//! A request carries a tool set ([`McpServer::handle_with`]): `tools/list`
//! shows only the set's tools, and `tools/call` of any other tool is refused
//! with a JSON-RPC error. The local server serves Everything.

use std::sync::Arc;

use serde_json::{json, Value};
use vault_core::Vault;

use crate::tools;
use vault_tools::ToolSet;
use crate::Result;

/// Protocol versions this server speaks, newest first. `initialize`
/// answers with the client's version when it is one of these.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = vault_tools::help::SERVER_INSTRUCTIONS;

enum Source {
    /// A vault open for the server's lifetime (the app's endpoint, or
    /// `notes-mcp --http`).
    Open(Arc<Vault>),
    /// Opened for each tool call and closed after it, so the vault lock is
    /// held only during the call (`notes-mcp` over stdio while the app is
    /// not running). The call fails if another process holds the lock.
    PerCall(String),
}

pub struct McpServer {
    source: Source,
    /// The tools [`McpServer::handle`] and [`McpServer::handle_body`] list
    /// and call.
    set: ToolSet,
}

/// The reply to one HTTP POST body: the status and the JSON text (empty
/// for `202 Accepted`, when there is nothing to answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyReply {
    pub status: u16,
    pub body: String,
}

impl McpServer {
    /// Serves an open vault.
    pub fn new(vault: Arc<Vault>) -> McpServer {
        McpServer {
            source: Source::Open(vault),
            set: ToolSet::Everything,
        }
    }

    /// Opens the vault at `root` for each tool call (see `Source::PerCall`).
    pub fn per_call(root: String) -> McpServer {
        McpServer {
            source: Source::PerCall(root),
            set: ToolSet::Everything,
        }
    }

    /// Handles one JSON-RPC message (or a batch array). Returns the reply,
    /// or None when there is nothing to answer (notifications, responses).
    pub fn handle(&self, msg: Value) -> Option<Value> {
        self.handle_with(msg, self.set, |name, args| self.run(name, args))
    }

    /// Like [`McpServer::handle`], but the tools run through `call` (the
    /// hub uses it to attribute edits to a user) and only the tools in `set`
    /// are listed or callable. `call` runs once per `tools/call` that is
    /// allowed.
    pub fn handle_with<F>(&self, msg: Value, set: ToolSet, call: F) -> Option<Value>
    where
        F: Fn(&str, &Value) -> Result<String>,
    {
        tools::with_transport("stdio", || dispatch(msg, set, &call))
    }

    /// Handles the bytes of one HTTP POST body: a parse error is `400` with a
    /// JSON-RPC error, a reply is `200` with JSON, and nothing to answer is
    /// `202` with an empty body.
    pub fn handle_body(&self, body: &[u8]) -> BodyReply {
        self.handle_body_with(body, self.set, |name, args| self.run(name, args))
    }

    /// [`McpServer::handle_body`] with `set` and `call` as in
    /// [`McpServer::handle_with`].
    pub fn handle_body_with<F>(&self, body: &[u8], set: ToolSet, call: F) -> BodyReply
    where
        F: Fn(&str, &Value) -> Result<String>,
    {
        let msg: Value = match serde_json::from_slice(body) {
            Ok(m) => m,
            Err(e) => {
                let r = error(Value::Null, -32700, &format!("parse error: {e}"));
                return BodyReply {
                    status: 400,
                    body: r.to_string(),
                };
            }
        };
        match tools::with_transport("http", || dispatch(msg, set, &call)) {
            Some(r) => BodyReply {
                status: 200,
                body: r.to_string(),
            },
            None => BodyReply {
                status: 202,
                body: String::new(),
            },
        }
    }

    /// Runs tool `name` on this server's vault.
    fn run(&self, name: &str, args: &Value) -> Result<String> {
        match &self.source {
            Source::Open(v) => tools::call(v, name, args),
            Source::PerCall(root) => match Vault::open(root.clone()) {
                Err(e) => Err(e.into()),
                Ok(v) if !v.is_owner() => Err(crate::McpError(
                    "the vault is open in another program (the notes app?) that does not \
                     serve MCP; turn on Settings > AI > MCP endpoint, or close it"
                        .into(),
                )),
                Ok(v) => {
                    let r = tools::call(&v, name, args);
                    // Dropping the vault flushes; report a failed flush.
                    match (r, v.flush()) {
                        (Ok(t), Err(e)) => {
                            Err(crate::McpError(format!("{t}\n(write failed: {e})")))
                        }
                        (r, _) => r,
                    }
                }
            },
        }
    }
}

/// The JSON-RPC dispatch shared by every entry point. A batch answers with
/// the array of its replies, or nothing when none of its messages needs one.
fn dispatch<F>(msg: Value, set: ToolSet, call: &F) -> Option<Value>
where
    F: Fn(&str, &Value) -> Result<String>,
{
    if let Value::Array(items) = msg {
        let out: Vec<Value> = items
            .into_iter()
            .filter_map(|m| dispatch(m, set, call))
            .collect();
        return (!out.is_empty()).then_some(Value::Array(out));
    }
    let id = msg.get("id").cloned();
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        // A response or garbage: answer garbage that carries an id.
        return id.map(|id| error(id, -32600, "invalid request"));
    };
    let id = id?; // Notifications get no reply.
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => ok(id, initialize(&params)),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools::definitions_for(set) })),
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            None => error(id, -32602, "missing tool name"),
            Some(name) => {
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if !tools::exists(name) {
                    error(id, -32602, &format!("unknown tool: {name}"))
                } else if !tools::allowed(name, set) {
                    error(id, -32602, &set.refusal(name))
                } else {
                    ok(id, tool_result(call(name, &args)))
                }
            }
        },
        "resources/list" => ok(id, json!({ "resources": [] })),
        "prompts/list" => ok(id, json!({ "prompts": [] })),
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

/// The MCP `CallToolResult` for a tool's text, or its error for the assistant.
fn tool_result(result: Result<String>) -> Value {
    match result {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        Err(e) => {
            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true })
        }
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "notes", "title": "Notes vault", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
