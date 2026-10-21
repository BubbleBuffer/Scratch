//! A small HTTP client for the MCP endpoint: `notes-mcp` over stdio
//! forwards tool calls to the program that holds the vault (the app), and
//! tests and the fake agent use it as an MCP client.

use std::time::Duration;

use serde_json::{json, Value};

use crate::{McpError, Result};

pub struct McpClient {
    url: String,
    auth: String,
    agent: ureq::Agent,
}

impl McpClient {
    pub fn new(url: &str, token: &str) -> McpClient {
        let agent = ureq::config::Config::builder()
            .http_status_as_error(false)
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(300)))
            .build()
            .new_agent();
        McpClient {
            url: url.to_string(),
            auth: format!("Bearer {token}"),
            agent,
        }
    }

    /// Posts one message. Returns the HTTP status and the reply (None for
    /// 202 or an empty body).
    pub fn post(&self, msg: &Value) -> Result<(u16, Option<Value>)> {
        let mut resp = self
            .agent
            .post(&self.url)
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .send(msg.to_string().as_bytes())
            .map_err(|e| McpError(format!("cannot reach the MCP endpoint: {e}")))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 << 20)
            .read_to_vec()
            .map_err(|e| McpError(e.to_string()))?;
        if body.is_empty() {
            return Ok((status, None));
        }
        let v = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        Ok((status, Some(v)))
    }

    /// Forwards a JSON-RPC message; errors other than JSON-RPC ones (no
    /// endpoint, bad token) are errors.
    pub fn forward(&self, msg: &Value) -> Result<Option<Value>> {
        match self.post(msg)? {
            (200, v) => Ok(v),
            (202, _) => Ok(None),
            (401, _) => Err(McpError(
                "the MCP endpoint refused the token (was it rotated?)".into(),
            )),
            (s, v) => Err(McpError(format!(
                "MCP endpoint answered {s}: {}",
                v.unwrap_or(Value::Null)
            ))),
        }
    }

    /// `initialize` + `notifications/initialized`.
    pub fn initialize(&self) -> Result<Value> {
        let r = self
            .forward(&json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": { "name": "notes-mcp-client", "version": "0" } } }))?
            .ok_or_else(|| McpError("no reply to initialize".into()))?;
        self.forward(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(r)
    }

    /// Calls a tool; returns its text (an error if the tool failed).
    pub fn call_tool(&self, name: &str, args: Value) -> Result<String> {
        let r = self
            .forward(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": name, "arguments": args } }))?
            .ok_or_else(|| McpError("no reply".into()))?;
        if let Some(e) = r.get("error") {
            return Err(McpError(e.to_string()));
        }
        let res = &r["result"];
        let text = res["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if res["isError"].as_bool() == Some(true) {
            return Err(McpError(text));
        }
        Ok(text)
    }
}
