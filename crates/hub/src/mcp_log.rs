//! One structured log line per MCP `tools/call`, shared by every path that
//! serves the call: the vault endpoint (`mcp::dispatch_vault`) and the agent
//! gateway (`mcp_agent`). A line names the tool as called, the sorted top-level
//! argument keys, the size of the result text and the outcome. It never carries
//! argument values, page content, queries or tokens.

use std::time::Instant;

use serde_json::Value;

/// The `error` field of a line: `none` or one fixed word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The call succeeded.
    Succeeded,
    /// The tool ran and reported an error to the assistant.
    Tool,
    /// No tool of that name is served on this endpoint.
    UnknownTool,
    /// The tool exists, but the request's scope may not call it.
    Denied,
    /// The request itself is wrong: no tool name, a bad argument shape, a bad
    /// session reference, or a body too large to serve.
    InvalidArgs,
    /// The hub failed to serve the call.
    Internal,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Outcome::Succeeded => "none",
            Outcome::Tool => "tool",
            Outcome::UnknownTool => "unknown_tool",
            Outcome::Denied => "denied",
            Outcome::InvalidArgs => "invalid_args",
            Outcome::Internal => "internal",
        }
    }
}

/// One answered `tools/call` as its request carried it: the id that pairs it
/// with its reply, the tool name and the argument keys. Values are not kept.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    id: Value,
    tool: Option<String>,
    keys: String,
}

impl Call {
    /// The call a request message makes, when it is a `tools/call` with an id.
    /// Notifications get no reply and run nothing, so they are not logged.
    pub(crate) fn from_request(msg: &Value) -> Option<Call> {
        if msg["method"] != "tools/call" {
            return None;
        }
        let id = msg.get("id")?.clone();
        let params = &msg["params"];
        Some(Call {
            id,
            tool: params["name"].as_str().map(str::to_owned),
            keys: arg_keys(&params["arguments"]),
        })
    }

    /// Every answered `tools/call` in a request body: one message or a batch.
    pub(crate) fn all(request: &Value) -> Vec<Call> {
        match request {
            Value::Array(items) => items.iter().filter_map(Call::from_request).collect(),
            single => Call::from_request(single).into_iter().collect(),
        }
    }

    /// Writes this call's line.
    pub(crate) fn log(&self, outcome: Outcome, result_bytes: usize, started: Instant) {
        emit(
            self.tool.as_deref(),
            &self.keys,
            outcome,
            result_bytes,
            started,
        );
    }
}

/// Logs the calls of a vault request whose reply body is `body`. A call the
/// vault never ran (no name, unknown, or denied in this scope) is classified
/// from the request; the others from their reply, matched by id.
pub(crate) fn log_vault(calls: &[Call], set: vault_tools::ToolSet, body: &str, started: Instant) {
    if calls.is_empty() {
        return;
    }
    let replies: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let items: Vec<&Value> = match &replies {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        single => vec![single],
    };
    for call in calls {
        let reply = items
            .iter()
            .copied()
            .find(|r| r.get("id") == Some(&call.id));
        let outcome = match call.tool.as_deref() {
            None => Outcome::InvalidArgs,
            Some(name) if !vault_mcp::tools::exists(name) => Outcome::UnknownTool,
            Some(name) if !vault_mcp::tools::allowed(name, set) => Outcome::Denied,
            Some(_) => reply.map_or(Outcome::Internal, outcome_of),
        };
        call.log(outcome, reply.map_or(0, result_bytes), started);
    }
}

/// Logs the calls of a vault request that was refused or failed before, or
/// while, it ran: no reply exists for them.
pub(crate) fn log_vault_failure(calls: &[Call], outcome: Outcome, started: Instant) {
    for call in calls {
        call.log(outcome, 0, started);
    }
}

/// The outcome of a reply the server built: a JSON-RPC error, or a tool result
/// that reports `isError`.
pub(crate) fn outcome_of(reply: &Value) -> Outcome {
    if let Some(error) = reply.get("error") {
        let host_unknown = error["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("unknown or unavailable host tool"));
        return if host_unknown {
            Outcome::UnknownTool
        } else if error["code"] == -32602 {
            Outcome::InvalidArgs
        } else {
            Outcome::Internal
        };
    }
    if reply["result"]["isError"] == true {
        Outcome::Tool
    } else {
        Outcome::Succeeded
    }
}

/// The byte size of a reply's text: the tool's result text, or a JSON-RPC
/// error message.
pub(crate) fn result_bytes(reply: &Value) -> usize {
    reply["result"]["content"][0]["text"]
        .as_str()
        .or_else(|| reply["error"]["message"].as_str())
        .map_or(0, str::len)
}

/// The sorted top-level keys of an arguments object, comma-joined. Keys that
/// are not plain identifiers show as `?`, so a key cannot break a log line.
fn arg_keys(args: &Value) -> String {
    let Some(map) = args.as_object() else {
        return "-".to_owned();
    };
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    if keys.is_empty() {
        return "-".to_owned();
    }
    keys.into_iter()
        .map(|k| if plain(k) { k } else { "?" })
        .collect::<Vec<_>>()
        .join(",")
}

/// A tool name as called, or `unknown` for a name that is not a served tool
/// or is not a plain identifier.
fn shown_tool(tool: Option<&str>, outcome: Outcome) -> &str {
    match tool {
        Some(name) if outcome != Outcome::UnknownTool && plain(name) => name,
        _ => "unknown",
    }
}

/// A plain identifier: 1 to 64 letters, digits, `_`, `-`, `.` or `:`.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

fn emit(tool: Option<&str>, keys: &str, outcome: Outcome, result_bytes: usize, started: Instant) {
    let tool = shown_tool(tool, outcome);
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        target: "notes_hub::mcp_call",
        tool = %tool,
        arg_keys = %keys,
        result_bytes = result_bytes,
        error = %outcome.word(),
        duration_ms = duration_ms,
        "mcp call"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_names_only() {
        assert_eq!(
            arg_keys(&json!({"query": "SECRET-Q", "scope": "all", "a": 1})),
            "a,query,scope"
        );
        assert_eq!(arg_keys(&json!({})), "-");
        assert_eq!(arg_keys(&json!("SECRET")), "-");
        assert_eq!(arg_keys(&json!({"bad\nkey": 1})), "?");
    }

    #[test]
    fn unknown_or_odd_names_are_not_shown() {
        assert_eq!(
            shown_tool(Some("search_notes"), Outcome::Succeeded),
            "search_notes"
        );
        assert_eq!(shown_tool(Some("made_up"), Outcome::UnknownTool), "unknown");
        assert_eq!(shown_tool(Some("a b"), Outcome::InvalidArgs), "unknown");
        assert_eq!(shown_tool(None, Outcome::InvalidArgs), "unknown");
    }

    #[test]
    fn outcome_follows_the_reply_shape() {
        let ok =
            json!({"result": {"content": [{"type": "text", "text": "abc"}], "isError": false}});
        let tool = json!({"result": {"content": [{"type": "text", "text": "x"}], "isError": true}});
        let host =
            json!({"error": {"code": -32602, "message": "unknown or unavailable host tool: t"}});
        let args = json!({"error": {"code": -32602, "message": "arguments must be an object"}});
        assert_eq!(outcome_of(&ok), Outcome::Succeeded);
        assert_eq!(result_bytes(&ok), 3);
        assert_eq!(outcome_of(&tool), Outcome::Tool);
        assert_eq!(outcome_of(&host), Outcome::UnknownTool);
        assert_eq!(outcome_of(&args), Outcome::InvalidArgs);
        assert_eq!(result_bytes(&args), "arguments must be an object".len());
    }
}
