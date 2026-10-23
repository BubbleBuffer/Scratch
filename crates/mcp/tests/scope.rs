//! The request scope (read-only or full) and the shared body handler, in
//! process on a temporary vault. Covers what the hub relies on:
//! the tool list and call policy, the JSON-RPC error
//! shapes, and the status codes of [`McpServer::handle_body`].

use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use vault_core::*;
use vault_mcp::tools;
use vault_mcp::McpServer;
use vault_tools::ToolSet;

/// The read-only scope's listed tools: the registry's read-only tools, in
/// name order (see `read_scope_is_the_registry_read_only_tools`).
const READ_SCOPE: [&str; 4] = ["memory_read", "notes_help", "read_page", "search_notes"];

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_mcp_scope_{}", new_id())))
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A server over a fresh vault, and the vault itself for the closure.
fn server(t: &Temp) -> (McpServer, Arc<Vault>) {
    let v = Arc::new(Vault::open(t.root()).unwrap());
    (McpServer::new(v.clone()), v)
}

fn list(reply: &Value) -> Vec<String> {
    let mut names: Vec<String> = reply["result"]["tools"]
        .as_array()
        .expect("a tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

fn call_msg(name: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": name, "arguments": args } })
}

fn list_msg() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })
}

#[test]
fn read_scope_is_the_registry_read_only_tools() {
    // The explicit list must agree with the registry's flags, so a new
    // read-only tool or a flag change cannot slip past the policy unnoticed.
    let mut from_registry: Vec<&str> = vault_tools::definitions()
        .iter()
        .filter(|t| t.read_only)
        .map(|t| t.name)
        .collect();
    from_registry.sort();
    let mut listed: Vec<&str> = READ_SCOPE.to_vec();
    listed.sort();
    assert_eq!(listed, from_registry);
    assert_eq!(listed, READ_SCOPE);
}

#[test]
fn read_scope_allows_old_read_names_and_refuses_write_aliases() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let target = v.create_page(None, "Target".into()).unwrap();
    let calls = Cell::new(0);
    let run = |name: &str, args: Value| {
        s.handle_with(call_msg(name, args), ToolSet::Read, |n, a| {
            calls.set(calls.get() + 1);
            vault_tools::call(&v, n, a).map_err(Into::into)
        })
        .expect("reply")
    };
    // Hidden read-only names run, as their new tools do.
    for (name, args) in [
        ("list_pages", json!({})),
        ("backlinks", json!({ "page": target.to_string() })),
        ("agenda", json!({ "from": "2026-10-01", "to": "2026-10-31" })),
        ("overdue", json!({})),
        ("memory_list", json!({})),
        ("feedback_list", json!({})),
    ] {
        let reply = run(name, args);
        assert_eq!(reply["result"]["isError"], false, "{name}: {reply}");
    }
    assert_eq!(calls.get(), 6);
    // Write aliases and the hidden write tool import_markdown are refused,
    // and their closure never runs.
    for name in [
        "append_blocks",
        "replace_blocks",
        "update_page",
        "reschedule",
        "set_done",
        "import_markdown",
        "move_page",
    ] {
        let reply = run(name, json!({ "page": target.to_string() }));
        assert_eq!(reply["error"]["code"], -32602, "{name}: {reply}");
        assert_eq!(
            reply["error"]["message"],
            format!("tool not available in read-only scope: {name}")
        );
    }
    assert_eq!(calls.get(), 6, "refused calls must not run");
}

#[test]
fn read_scope_lists_only_read_tools() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let reply = s
        .handle_with(list_msg(), ToolSet::Read, |n, a| {
            vault_tools::call(&v, n, a).map_err(Into::into)
        })
        .expect("reply");
    let mut expected: Vec<String> = READ_SCOPE.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(list(&reply), expected);
}

#[test]
fn full_scope_lists_every_tool_as_handle_does() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let all = s.handle(list_msg()).expect("reply");
    let full = s
        .handle_with(list_msg(), ToolSet::Write, |n, a| {
            vault_tools::call(&v, n, a).map_err(Into::into)
        })
        .expect("reply");
    assert_eq!(all, full);
    assert_eq!(list(&all).len(), vault_tools::definitions().len());
    assert_eq!(
        tools::definitions_for(ToolSet::Write).len(),
        vault_tools::definitions().len()
    );
}

#[test]
fn feedback_transport_is_scoped_to_each_dispatch() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let http = s.handle_body_with(
        call_msg("feedback_submit", json!({"summary":"HTTP report"}))
            .to_string()
            .as_bytes(),
        ToolSet::Write,
        |n, a| tools::call(&v, n, a),
    );
    let http: Value = serde_json::from_str(&http.body).unwrap();
    let stdio = s
        .handle(call_msg(
            "feedback_submit",
            json!({"summary":"Stdio report"}),
        ))
        .unwrap();
    for (reply, transport) in [(http, "http"), (stdio, "stdio")] {
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let report: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        let page = v.load_page(report["id"].as_str().unwrap().into()).unwrap();
        assert!(page
            .blocks
            .last()
            .unwrap()
            .text
            .contains(&format!("transport: {transport}")));
    }
}

#[test]
fn read_scope_refuses_write_tools_without_running_them() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let calls = Cell::new(0);
    let reply = s
        .handle_with(
            call_msg("create_page", json!({ "title": "Nope", "markdown": "x" })),
            ToolSet::Read,
            |n, a| {
                calls.set(calls.get() + 1);
                vault_tools::call(&v, n, a).map_err(Into::into)
            },
        )
        .expect("reply");
    assert_eq!(
        calls.get(),
        0,
        "the closure must not run for a refused tool"
    );
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("read-only scope: create_page"));
    assert!(reply.get("result").is_none());
    // And the page was not created.
    let pages = vault_tools::call(&v, "search_notes", &json!({})).unwrap();
    assert!(!pages.contains("Nope"), "{pages}");
}

#[test]
fn read_scope_runs_read_tools_through_the_closure() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let calls = Cell::new(0);
    let reply = s
        .handle_with(call_msg("search_notes", json!({})), ToolSet::Read, |n, a| {
            calls.set(calls.get() + 1);
            vault_tools::call(&v, n, a).map_err(Into::into)
        })
        .expect("reply");
    assert_eq!(calls.get(), 1);
    assert_eq!(reply["result"]["isError"], false);
}

#[test]
fn unknown_tool_is_unknown_in_both_scopes() {
    let t = Temp::new();
    let (s, _v) = server(&t);
    for set in ToolSet::ALL {
        let reply = s
            .handle_with(call_msg("nope", json!({})), set, |_, _| {
                Ok(String::new())
            })
            .expect("reply");
        assert_eq!(reply["error"]["code"], -32602);
        assert_eq!(reply["error"]["message"], "unknown tool: nope");
    }
}

#[test]
fn closure_result_is_wrapped_as_the_tool_result() {
    let t = Temp::new();
    let (s, _v) = server(&t);
    let ok = s
        .handle_with(call_msg("list_pages", json!({})), ToolSet::Write, |_, _| {
            Ok("hello".into())
        })
        .expect("reply");
    assert_eq!(
        ok["result"],
        json!({ "content": [{ "type": "text", "text": "hello" }], "isError": false })
    );
    let err = s
        .handle_with(call_msg("list_pages", json!({})), ToolSet::Write, |_, _| {
            Err(vault_mcp::McpError("boom".into()))
        })
        .expect("reply");
    assert_eq!(
        err["result"],
        json!({ "content": [{ "type": "text", "text": "Error: boom" }], "isError": true })
    );
}

#[test]
fn read_scope_applies_to_each_batch_item() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let batch = json!([
        list_msg(),
        call_msg("create_page", json!({ "title": "Batch", "markdown": "x" })),
        call_msg("search_notes", json!({ "query": "x" })),
    ]);
    let reply = s
        .handle_with(batch, ToolSet::Read, |n, a| {
            vault_tools::call(&v, n, a).map_err(Into::into)
        })
        .expect("reply");
    let items = reply.as_array().expect("a batch reply");
    assert_eq!(items.len(), 3);
    assert_eq!(list(&items[0]).len(), READ_SCOPE.len());
    assert_eq!(items[1]["error"]["code"], -32602);
    assert_eq!(items[2]["result"]["isError"], false);
}

#[test]
fn handle_body_parse_error_is_400_with_null_id() {
    let t = Temp::new();
    let (s, _v) = server(&t);
    let bodies: [&[u8]; 3] = [b"not json", b"{\"jsonrpc\":", &[0xff, 0xfe, 0x00]];
    for body in bodies {
        let r = s.handle_body(body);
        assert_eq!(r.status, 400);
        let parsed: Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(parsed["id"], Value::Null);
        assert_eq!(parsed["error"]["code"], -32700);
        assert!(parsed["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("parse error: "));
    }
}

#[test]
fn handle_body_reply_is_200_json_of_handle() {
    let t = Temp::new();
    let (s, _v) = server(&t);
    let body = list_msg().to_string();
    let r = s.handle_body(body.as_bytes());
    assert_eq!(r.status, 200);
    let expected = s.handle(list_msg()).unwrap();
    assert_eq!(r.body, expected.to_string());
    assert!(r.body.starts_with('{'));
}

#[test]
fn handle_body_notification_is_202_with_empty_body() {
    let t = Temp::new();
    let (s, _v) = server(&t);
    let body = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
    let r = s.handle_body(body.as_bytes());
    assert_eq!(r.status, 202);
    assert_eq!(r.body, "");
}

#[test]
fn handle_body_batch_rules_follow_json_rpc() {
    let t = Temp::new();
    let (s, _v) = server(&t);
    // Only notifications: nothing to answer.
    let notes = json!([{ "jsonrpc": "2.0", "method": "a" }, { "jsonrpc": "2.0", "method": "b" }]);
    assert_eq!(s.handle_body(notes.to_string().as_bytes()).status, 202);
    // An empty batch answers nothing either (as `handle` always has).
    assert_eq!(s.handle_body(b"[]").status, 202);
    // A mixed batch answers with the array of its replies only.
    let mixed = json!([
        { "jsonrpc": "2.0", "method": "ping" },
        { "jsonrpc": "2.0", "id": 3, "method": "ping" },
    ]);
    let r = s.handle_body(mixed.to_string().as_bytes());
    assert_eq!(r.status, 200);
    let parsed: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(parsed, json!([{ "jsonrpc": "2.0", "id": 3, "result": {} }]));
    // A bare number is garbage with no id: no reply.
    assert_eq!(s.handle_body(b"5").status, 202);
}

#[test]
fn handle_body_with_read_scope_hides_and_refuses_writes() {
    let t = Temp::new();
    let (s, v) = server(&t);
    let r = s.handle_body_with(list_msg().to_string().as_bytes(), ToolSet::Read, |n, a| {
        vault_tools::call(&v, n, a).map_err(Into::into)
    });
    assert_eq!(r.status, 200);
    let parsed: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(list(&parsed).len(), READ_SCOPE.len());

    let write = call_msg("trash_page", json!({ "page": "x" })).to_string();
    let r = s.handle_body_with(write.as_bytes(), ToolSet::Read, |_, _| panic!("must not run"));
    assert_eq!(r.status, 200);
    let parsed: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(parsed["error"]["code"], -32602);
}
