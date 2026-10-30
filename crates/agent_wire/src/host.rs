//! The host relay: a desktop that lets the hub use it keeps one socket open
//! to `GET /host/ws`, and the hub sends it host tool calls from
//! `/mcp/agent?host=<name>`. Roles are reversed from `/agent/ws`: the hub
//! sends the requests ([`crate::Request`]) and the desktop answers them
//! ([`crate::Response`]), one text frame per message.
//!
//! Protocol reference: `wiki/sync/hub.md` (`/host/ws`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The route the desktop connects to.
pub const PATH: &str = "/host/ws";

/// Method names. Every request is answered; [`CANCEL`] is a notification.
pub mod method {
    /// [`super::ToolsParams`] → [`super::ToolsResult`].
    pub const TOOLS: &str = "tools";
    /// [`super::CallParams`] → [`super::CallResult`].
    pub const CALL: &str = "call";
    /// [`super::CloseParams`] → null. Ends one relay session.
    pub const CLOSE: &str = "close_session";
    /// [`super::CancelParams`], no answer: stop the named request.
    pub const CANCEL: &str = "cancel";
}

/// Lists the tools offered to a relay session; none lists them for discovery.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// One offered tool, as the MCP gateway lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolsResult {
    pub tools: Vec<ToolSpec>,
}

/// Who is calling: the OAuth login on `/mcp/agent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caller {
    /// `oauth:<subject>`.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Runs one tool in a relay session. `session` is an opaque id chosen by the
/// hub, one per MCP session; the desktop opens its own session for it on
/// first use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallParams {
    pub session: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    pub caller: Caller,
}

/// A tool's outcome. A failing tool is `is_error`, not an RPC error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallResult {
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseParams {
    pub session: String,
}

/// Names the request (by its JSON-RPC id) whose call should stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelParams {
    pub id: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn call_params_round_trip() {
        let params = CallParams {
            session: "r1".into(),
            name: "read".into(),
            arguments: json!({"path":"a.txt"}),
            caller: Caller {
                id: "oauth:sub".into(),
                name: Some("me@example.com".into()),
            },
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["caller"]["id"], "oauth:sub");
        assert_eq!(serde_json::from_value::<CallParams>(value).unwrap(), params);
        let result: CallResult = serde_json::from_value(json!({"content":"x"})).unwrap();
        assert!(!result.is_error && !result.pending);
        assert_eq!(
            serde_json::to_value(ToolsParams::default()).unwrap(),
            json!({})
        );
    }
}
