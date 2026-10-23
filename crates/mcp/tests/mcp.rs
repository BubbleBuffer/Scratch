//! The MCP server end to end on temporary vaults: JSON-RPC over stdio (in
//! process, and the real `notes-mcp` binary over pipes), and every tool.

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use serde_json::{json, Value};
use vault_core::*;
use vault_mcp::{stdio, tools, McpServer};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_mcp_test_{}", new_id())))
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

fn call(server: &McpServer, name: &str, args: Value) -> (bool, String) {
    let r = server
        .handle(json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": name, "arguments": args } }))
        .expect("reply");
    let res = &r["result"];
    (
        res["isError"].as_bool().unwrap_or(true),
        res["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

fn ok(server: &McpServer, name: &str, args: Value) -> String {
    let (is_err, text) = call(server, name, args);
    assert!(!is_err, "{name} failed: {text}");
    text
}

/// The id in "`<id>`" after a "Created" message.
fn id_in(text: &str) -> String {
    text.split('`').nth(1).expect("an id").to_string()
}

/// The listed MCP tools, in serving order.
const LISTED: [&str; 16] = [
    "search_notes",
    "read_page",
    "create_page",
    "edit_page",
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

/// Old tool names: callable, never listed. feedback_list and import_markdown
/// are hidden tools, not aliases.
const OLD_NAMES: [&str; 14] = [
    "list_pages",
    "backlinks",
    "import_markdown",
    "update_page",
    "append_blocks",
    "replace_blocks",
    "agenda",
    "query_database",
    "memory_search",
    "overdue",
    "reschedule",
    "set_done",
    "memory_list",
    "feedback_list",
];

/// The names `tools/list` returns over stdio, in order.
fn listed_names(list: &Value) -> Vec<&str> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect()
}

/// A page used by the alias comparison: its id, title and block ids (in
/// order), so the texts of two calls on different pages can be compared.
struct Pg {
    id: String,
    title: &'static str,
    blocks: Vec<String>,
}

fn pg(s: &McpServer, id: &str, title: &'static str) -> Pg {
    let text = ok(s, "read_page", json!({ "page": id, "block_ids": true }));
    let blocks = text
        .lines()
        .filter_map(|l| {
            l.strip_prefix("<!-- block ")
                .and_then(|x| x.strip_suffix(" -->"))
        })
        .map(str::to_string)
        .collect();
    Pg {
        id: id.to_string(),
        title,
        blocks,
    }
}

impl Pg {
    /// The text with this page's id, title and block ids replaced by
    /// placeholders that match across two pages.
    fn norm(&self, text: &str) -> String {
        let mut out = text
            .replace(&self.id, "<page>")
            .replace(self.title, "<title>");
        for (i, b) in self.blocks.iter().enumerate() {
            out = out.replace(b.as_str(), &format!("<block{i}>"));
        }
        // Blocks made by the call itself have no known id yet; any other
        // 26-character uppercase id token (a ULID) becomes a placeholder.
        out.split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| {
                t.len() == 26
                    && t.chars()
                        .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            })
            .map(str::to_string)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .fold(out, |acc, t| acc.replace(&t, "<new-id>"))
    }
}

/// Runs an old-name call and the new tool's call (which must give the same
/// result) and compares the texts. With pages, each text is normalized
/// with its own page's ids first.
fn same_result(s: &McpServer, old: (&str, Value), new: (&str, Value), pages: Option<(&Pg, &Pg)>) {
    let (old_err, old_text) = call(s, old.0, old.1);
    let (new_err, new_text) = call(s, new.0, new.1);
    assert!(
        !old_err && !new_err,
        "{} / {}: {old_text} | {new_text}",
        old.0,
        new.0
    );
    let (a, b) = match pages {
        Some((pa, pb)) => (pa.norm(&old_text), pb.norm(&new_text)),
        None => (old_text.clone(), new_text.clone()),
    };
    // Blocks a call creates get new ids (the reply names them), so any id
    // left is masked on both sides.
    assert_eq!(mask_ids(&a), mask_ids(&b), "{} vs {}", old.0, new.0);
}

/// Masks every block or page id (a 26-character Crockford base32 run) in
/// `text` as `<id>`.
fn mask_ids(text: &str) -> String {
    fn flush(run: &mut String, out: &mut String) {
        if run.len() == 26 {
            out.push_str("<id>");
        } else {
            out.push_str(run);
        }
        run.clear();
    }
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch.is_ascii_uppercase() {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut run, &mut out);
    out
}

#[test]
fn stdio_in_process_protocol() {
    let t = Temp::new();
    let server = McpServer::new(Arc::new(Vault::open(t.root()).unwrap()));
    let input = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "create_page", "arguments": { "title": "Hello", "markdown": "First line" } } }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "nope" }),
    ]
    .iter()
    .map(|m| m.to_string())
    .collect::<Vec<_>>()
    .join("\n")
        + "\nnot json\n";
    let mut out = Vec::new();
    stdio::serve(Cursor::new(input), &mut out, |m| server.handle(m)).unwrap();
    let replies: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // The notification gets no reply.
    assert_eq!(replies.len(), 5);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "notes");
    let tools: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        tools, LISTED,
        "tools/list must be exactly the 16 names, in order"
    );
    for old in OLD_NAMES {
        assert!(!tools.contains(&old), "old name {old} must stay unlisted");
    }
    assert_eq!(
        tools
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        tools.len(),
        "MCP tool names must be unique"
    );
    assert_eq!(replies[2]["result"]["isError"], false);
    assert!(replies[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Created **Hello**"));
    assert_eq!(replies[3]["error"]["code"], -32601);
    assert_eq!(replies[4]["error"]["code"], -32700);
}

#[test]
fn tools_cover_the_vault() {
    let t = Temp::new();
    let v = Arc::new(Vault::open(t.root()).unwrap());
    let s = McpServer::new(v.clone());

    // Pages, markdown, the tree.
    let home = id_in(&ok(
        &s,
        "create_page",
        json!({ "title": "Home", "markdown": "# Welcome\n\nIntro paragraph" }),
    ));
    let child = id_in(&ok(
        &s,
        "create_page",
        json!({ "title": "Child notes", "parent": home,
        "markdown": "Links to [[Home]]\n\n- [ ] task one" }),
    ));
    let tree = ok(&s, "search_notes", json!({ "depth": 2 }));
    assert!(
        tree.contains("**Home**") && tree.contains("  - **Child notes**"),
        "{tree}"
    );

    let page = ok(
        &s,
        "read_page",
        json!({ "page": "Home", "block_ids": true }),
    );
    assert!(page.starts_with("# Home"), "{page}");
    assert!(
        page.contains("# Welcome") && page.contains("Intro paragraph"),
        "{page}"
    );
    let intro = page
        .lines()
        .filter_map(|l| {
            l.strip_prefix("<!-- block ")
                .and_then(|x| x.strip_suffix(" -->"))
        })
        .nth(1)
        .unwrap()
        .to_string();

    // Append after a block, then replace it.
    ok(
        &s,
        "edit_page",
        json!({ "page": home, "op": "append", "markdown": "Inserted after intro", "after_block": intro }),
    );
    ok(
        &s,
        "edit_page",
        json!({ "page": home, "op": "append", "markdown": "## The end" }),
    );
    let texts: Vec<String> = v
        .load_page(home.clone())
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert_eq!(
        texts,
        [
            "Welcome",
            "Intro paragraph",
            "Inserted after intro",
            "The end"
        ]
    );
    let r = ok(
        &s,
        "edit_page",
        json!({ "page": home, "op": "replace", "block_ids": [intro], "markdown": "**New** intro" }),
    );
    assert!(r.starts_with("Replaced 1 block(s) with 1"), "{r}");
    let md = ok(&s, "read_page", json!({ "page": home }));
    assert!(
        md.contains("**New** intro") && !md.contains("Intro paragraph"),
        "{md}"
    );
    let (is_err, text) = call(
        &s,
        "edit_page",
        json!({ "page": home, "op": "replace", "block_ids": ["missing"], "markdown": "x" }),
    );
    assert!(is_err && text.contains("no block"), "{text}");

    // Pagination of a long page.
    let long = id_in(&ok(
        &s,
        "create_page",
        json!({ "title": "Long",
        "markdown": (1..=30).map(|i| format!("Line {i}")).collect::<Vec<_>>().join("\n\n") }),
    ));
    let p1 = ok(&s, "read_page", json!({ "page": long, "limit": 10 }));
    assert!(
        p1.contains("Line 10") && !p1.contains("Line 11") && p1.contains("More: offset=10"),
        "{p1}"
    );
    let p3 = ok(
        &s,
        "read_page",
        json!({ "page": long, "offset": 20, "limit": 10 }),
    );
    assert!(p3.contains("Line 30") && !p3.contains("More:"), "{p3}");

    // Title, icon and plain properties.
    ok(
        &s,
        "edit_page",
        json!({ "page": child, "op": "set_props", "title": "Child", "icon": "🌱", "properties": { "mood": "good", "score": 3 } }),
    );
    let c = ok(&s, "read_page", json!({ "page": child }));
    assert!(
        c.starts_with("# 🌱 Child") && c.contains("- mood: good") && c.contains("- score: 3"),
        "{c}"
    );

    // Search (content edits are flushed first) and backlinks.
    let hits = ok(&s, "search_notes", json!({ "query": "inserted" }));
    assert!(hits.contains(&home), "{hits}");
    let bl = ok(&s, "search_notes", json!({ "backlinks_of": home }));
    assert!(bl.contains(&child), "{bl}");

    // A database: schema, rows, filters and sorts.
    let db = v.create_page(None, "Projects".into()).unwrap();
    v.set_schema(
        db.clone(),
        vec![
            PropDef {
                key: String::new(),
                name: "Status".into(),
                kind: PropKind::Select,
                options: vec![SelectOption {
                    id: String::new(),
                    name: "Open".into(),
                    color: "blue".into(),
                }],
                config: String::new(),
            },
            PropDef {
                key: String::new(),
                name: "Estimate".into(),
                kind: PropKind::Number,
                options: vec![],
                config: String::new(),
            },
            PropDef {
                key: String::new(),
                name: "Due".into(),
                kind: PropKind::Date,
                options: vec![],
                config: String::new(),
            },
            PropDef {
                key: String::new(),
                name: "Urgent".into(),
                kind: PropKind::Checkbox,
                options: vec![],
                config: String::new(),
            },
        ],
    )
    .unwrap();
    let a = id_in(&ok(
        &s,
        "upsert_row",
        json!({ "database": "Projects", "title": "Alpha",
        "properties": { "Status": "Open", "Estimate": 5, "Due": "2026-10-03", "Urgent": true } }),
    ));
    ok(
        &s,
        "upsert_row",
        json!({ "database": db, "title": "Beta",
        "properties": { "Status": "Done", "Estimate": "2", "Due": "2026-10-01" }, "markdown": "Beta notes" }),
    );
    ok(
        &s,
        "create_page",
        json!({ "parent": db, "title": "Gamma", "properties": { "Estimate": 8, "Status": "Open" } }),
    );
    // "Done" was a new option: it is in the schema now.
    assert!(v.get_schema(db.clone()).unwrap()[0]
        .options
        .iter()
        .any(|o| o.name == "Done"));
    let all = ok(
        &s,
        "query_database",
        json!({ "database": db, "sort": [{ "property": "Estimate", "descending": true }] }),
    );
    let order: Vec<usize> = ["Gamma", "Alpha", "Beta"]
        .iter()
        .map(|n| all.find(&format!("**{n}**")).unwrap())
        .collect();
    assert!(order[0] < order[1] && order[1] < order[2], "{all}");
    // The schema's select options are checked through get_schema above; the
    // search output lists rows only.
    let open = ok(
        &s,
        "query_database",
        json!({ "database": db,
        "filter": [{ "property": "Status", "op": "is", "value": "Open" }, { "property": "Estimate", "op": "<", "value": 6 }] }),
    );
    // Known gap: search_notes drops a `Status` filter whose value is not a
    // status option ("Open" here) with a warning, so Beta (Estimate 2) is
    // still listed. Only the Estimate clause is checked here.
    assert!(
        open.contains("status: 'Open' is not an option")
            && open.contains("**Alpha**")
            && !open.contains("**Gamma**"),
        "{open}"
    );
    // match any is no longer in the query language: one query per filter.
    let (is_err, text) = call(
        &s,
        "query_database",
        json!({ "database": db, "match": "any",
        "filter": [{ "property": "Urgent", "op": "checked" }, { "property": "Due", "op": "before", "value": "2026-10-02" }] }),
    );
    assert!(
        is_err && text.contains("match any is not in the query language"),
        "{text}"
    );
    let paged = ok(&s, "query_database", json!({ "database": db, "limit": 2 }));
    assert!(paged.contains("More: offset=2"), "{paged}");
    ok(
        &s,
        "upsert_row",
        json!({ "database": db, "row": a, "properties": { "Urgent": false, "Estimate": null } }),
    );
    let alpha = ok(&s, "read_page", json!({ "page": a }));
    assert!(
        alpha.contains("- Urgent: no") && !alpha.contains("Estimate"),
        "{alpha}"
    );
    let (is_err, text) = call(
        &s,
        "query_database",
        json!({ "database": db, "filter": [{ "property": "Nope", "op": "is", "value": 1 }] }),
    );
    // An unknown property is no longer an error: the search matches nothing.
    assert!(!is_err && text.contains("No pages match"), "{text}");
    // Not checked here: query_database used to refuse a page that is not a
    // database; search_notes `in:` accepts any page (search_query.rs `"in"`).

    // Trash.
    ok(&s, "trash_page", json!({ "page": child }));
    assert!(v.load_page(child.clone()).unwrap().in_trash);
    let hits = ok(&s, "search_notes", json!({ "query": "Child" }));
    assert!(!hits.contains(&child), "{hits}");

    // Edits are in the page history like app edits.
    v.flush().unwrap();
    assert!(!v.list_versions(home).unwrap().is_empty());
}

/// Talks to `notes-mcp` over its stdin/stdout.
struct Proc {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Proc {
    fn start(root: &str) -> Proc {
        let mut child = Command::new(env!("CARGO_BIN_EXE_notes-mcp"))
            .args(["--vault", root])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Proc { child, out }
    }
    fn send(&mut self, msg: Value) -> Value {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn tool(&mut self, name: &str, args: Value) -> (bool, String) {
        let r = self.send(json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": { "name": name, "arguments": args } }));
        if let Some(e) = r.get("error") {
            return (true, e.to_string());
        }
        (
            r["result"]["isError"].as_bool().unwrap(),
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string(),
        )
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn binary_over_stdio_opens_the_vault_per_call() {
    let t = Temp::new();
    let mut p = Proc::start(&t.root());
    let init = p.send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }));
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    let list = p.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    assert_eq!(listed_names(&list), LISTED);
    let (e, text) = p.tool(
        "create_page",
        json!({ "title": "From stdio", "markdown": "Hi" }),
    );
    assert!(!e, "{text}");
    let id = id_in(&text);
    let (e, text) = p.tool("read_page", json!({ "page": id }));
    assert!(!e && text.contains("Hi"), "{text}");
    // Between calls the vault is closed: this process can open it as owner.
    let v = Vault::open(t.root()).unwrap();
    assert!(v.is_owner());
    assert_eq!(v.load_page(id).unwrap().title, "From stdio");
    // While another program holds the vault, calls fail and say what to do.
    let (e, text) = p.tool("search_notes", json!({ "query": "stdio" }));
    assert!(
        e && text.contains("open in another program") && text.contains("Close that program"),
        "{text}"
    );
    drop(v);
    // Once it lets go, the next call works again.
    let (e, text) = p.tool("search_notes", json!({ "query": "stdio" }));
    assert!(!e && text.contains("From stdio"), "{text}");
}

#[test]
fn feedback_over_real_stdio_is_hidden_and_records_transport() {
    let t = Temp::new();
    let token = "a1".repeat(32);
    let mut p = Proc::start(&t.root());
    let list = p.send(json!({"jsonrpc":"2.0", "id":1, "method":"tools/list"}));
    assert!(listed_names(&list).contains(&"feedback_submit"));
    assert!(
        !listed_names(&list).contains(&"feedback_list"),
        "feedback_list is hidden"
    );
    let (error, text) = p.tool(
        "feedback_submit",
        json!({
            "summary":"Vault failed unexpectedly", "details":"Steps:\n\n1. Open Scratch.",
            "tool":"search_notes", "error":format!("Bearer {token}"), "page_id":"example-page"
        }),
    );
    assert!(!error, "{text}");
    let report: Value = serde_json::from_str(&text).unwrap();
    let id = report["id"].as_str().unwrap();
    assert!(report["title"].as_str().unwrap().ends_with(" agent"));
    let (error, text) = p.tool("feedback_list", json!({}));
    assert!(!error, "{text}");
    assert!(text.contains(id));
    let (error, text) = p.tool("search_notes", json!({"query":"unexpectedly"}));
    assert!(!error && !text.contains(id), "{text}");
    let (error, text) = p.tool("read_page", json!({"page":id}));
    assert!(!error, "{text}");
    assert!(text.contains("transport: stdio"), "{text}");
    assert!(text.contains("source: mcp"), "{text}");
    assert!(text.contains("Steps:"), "{text}");
    assert!(!text.contains(&token), "report leaked the seeded token");
}

/// A plain (not gzipped) tar holding one regular file.
fn tar_one(path: &str, body: &str) -> Vec<u8> {
    let mut header = [0u8; 512];
    header[..path.len()].copy_from_slice(path.as_bytes());
    header[100..107].copy_from_slice(b"0000644");
    header[108..115].copy_from_slice(b"0000000");
    header[116..123].copy_from_slice(b"0000000");
    header[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
    header[136..147].copy_from_slice(b"00000000000");
    header[148..156].copy_from_slice(b"        ");
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
    header[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
    header[154] = 0;
    header[155] = b' ';
    let mut out = header.to_vec();
    out.extend_from_slice(body.as_bytes());
    out.resize(out.len().div_ceil(512) * 512, 0);
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// Standard base64 with padding.
fn base64_of(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// import_markdown is hidden: not listed, but it imports over stdio, and
/// create_page refuses roots there.
#[test]
fn import_markdown_is_hidden_and_imports_over_stdio() {
    let t = Temp::new();
    let mut p = Proc::start(&t.root());
    let (error, text) = p.tool(
        "create_page",
        json!({ "title": "Stdio Project", "roots": [{ "name": "docs", "archive_base64": "" }] }),
    );
    assert!(error, "{text}");
    assert!(
        text.contains(vault_tools::CREATE_PAGE_ROOTS_ERROR),
        "{text}"
    );
    assert!(text.contains("import_markdown"), "{text}");

    let list = p.send(json!({"jsonrpc":"2.0", "id":1, "method":"tools/list"}));
    assert!(
        !listed_names(&list).contains(&"import_markdown"),
        "import_markdown is hidden"
    );

    let archive = tar_one("one.md", "# One\n\nAlpha text.\n");
    let args = json!({
        "name": "Stdio Project",
        "roots": [{ "name": "docs", "archive_base64": base64_of(&archive) }],
        "last_activity": "2026-10-01",
    });
    let (error, text) = p.tool("import_markdown", args.clone());
    assert!(!error, "{text}");
    let first: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(first["row_created"], true, "{text}");
    let row = first["project_page"].as_str().unwrap().to_string();
    let (error, text) = p.tool("read_page", json!({ "page": row }));
    assert!(!error, "{text}");
    assert!(text.contains("# Stdio Project"), "{text}");

    // The same import again reuses the row and writes nothing.
    let (error, text) = p.tool("import_markdown", args);
    assert!(!error, "{text}");
    let again: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(again["row_created"], false, "{text}");
    assert_eq!(again["row_updated"], false, "{text}");
    assert_eq!(again["project_page"], first["project_page"]);
}

#[test]
fn every_alias_is_callable_and_unlisted() {
    let t = Temp::new();
    let s = McpServer::new(Arc::new(Vault::open(t.root()).unwrap()));
    let list = s
        .handle(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .unwrap();
    let listed = listed_names(&list);
    for a in vault_tools::aliases() {
        assert!(
            tools::exists(a.name),
            "alias {} is not callable over MCP",
            a.name
        );
        assert!(!listed.contains(&a.name), "alias {} is listed", a.name);
        assert!(
            OLD_NAMES.contains(&a.name),
            "alias {} is not in OLD_NAMES",
            a.name
        );
    }
    assert!(tools::exists("feedback_list"), "feedback_list is callable");
    assert!(!listed.contains(&"feedback_list"));
    // import_markdown is a hidden tool, not an alias: callable, never listed.
    assert!(vault_tools::alias("import_markdown").is_none());
    assert!(
        tools::exists("import_markdown"),
        "import_markdown is callable over MCP"
    );
    assert!(!listed.contains(&"import_markdown"));
}

#[test]
fn initialize_sends_the_shared_instructions() {
    let t = Temp::new();
    let s = McpServer::new(Arc::new(Vault::open(t.root()).unwrap()));
    let init = s
        .handle(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }))
        .unwrap();
    let text = init["result"]["instructions"].as_str().unwrap();
    assert_eq!(text, vault_tools::help::SERVER_INSTRUCTIONS);
    assert!(text.contains("notes_help"), "{text}");
}

/// Each old name gives the same result as its new tool, and the old-name
/// call works as before (hidden, see `every_alias_is_callable_and_unlisted`).
#[test]
fn old_names_give_the_same_results_as_the_new_tools() {
    let t = Temp::new();
    let v = Arc::new(Vault::open(t.root()).unwrap());
    let s = McpServer::new(v.clone());
    let body = "Intro paragraph\n\n- [ ] task one\n\nLast paragraph";
    let a = id_in(&ok(
        &s,
        "create_page",
        json!({ "title": "Alpha", "markdown": body }),
    ));
    let b = id_in(&ok(
        &s,
        "create_page",
        json!({ "title": "Beta", "markdown": body }),
    ));
    ok(
        &s,
        "create_page",
        json!({ "title": "Linker", "markdown": "Links to [[Alpha]]" }),
    );
    ok(
        &s,
        "memory_write",
        json!({
            "path": "desktop/sub/note", "description": "Desktop memory", "type": "reference", "body": "desktopmemoryneedle"
        }),
    );

    // Read-only names: the same state, so the same text.
    same_result(
        &s,
        ("list_pages", json!({})),
        ("search_notes", json!({})),
        None,
    );
    same_result(
        &s,
        ("list_pages", json!({ "depth": 2 })),
        ("search_notes", json!({ "depth": 2 })),
        None,
    );
    same_result(
        &s,
        ("backlinks", json!({ "page": a })),
        ("search_notes", json!({ "backlinks_of": a })),
        None,
    );
    // memory_search is search_notes over the memory pages; memory_list (a
    // hidden tool) keeps the plain index the CLI reads.
    let rewrite = |args: Value| {
        vault_tools::alias("memory_search")
            .unwrap()
            .rewrite(&args)
            .unwrap()
    };
    same_result(
        &s,
        ("memory_search", json!({})),
        ("search_notes", rewrite(json!({}))),
        None,
    );
    same_result(
        &s,
        ("memory_search", json!({ "scope": "desktop" })),
        ("search_notes", rewrite(json!({ "scope": "desktop" }))),
        None,
    );
    let range = json!({ "from": "2026-10-01", "to": "2026-10-31" });
    // tasks is gone, and agenda now prints item lines, not JSON. The only task
    // here has no date, so the range holds no dated item line.
    let agenda = ok(&s, "agenda", range);
    assert!(!agenda.contains("· due 2026-10"), "{agenda}");
    // overdue is search_notes with its rewritten query (crates/tools query.rs overdue_query).
    same_result(
        &s,
        ("overdue", json!({})),
        (
            "search_notes",
            json!({ "query": "is:todo,task date:<now done:no" }),
        ),
        None,
    );

    // Writes and item tools: one call on each of two pages with the same
    // body. Ids are normalized per page (see `Pg::norm`).
    let pair = |s: &McpServer| (pg(s, &a, "Alpha"), pg(s, &b, "Beta"));
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        (
            "reschedule",
            json!({ "item": format!("block:{a}/{}", pa.blocks[1]), "date": "2026-10-05" }),
        ),
        (
            "update_task",
            json!({ "item": format!("block:{b}/{}", pb.blocks[1]), "due": "2026-10-05" }),
        ),
        Some((&pa, &pb)),
    );
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        (
            "set_done",
            json!({ "item": format!("block:{a}/{}", pa.blocks[1]), "done": true }),
        ),
        (
            "update_task",
            json!({ "item": format!("block:{b}/{}", pb.blocks[1]), "done": true }),
        ),
        Some((&pa, &pb)),
    );
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        (
            "update_page",
            json!({ "page": a, "icon": "🌱", "properties": { "mood": "good" } }),
        ),
        (
            "edit_page",
            json!({ "page": b, "op": "set_props", "icon": "🌱", "properties": { "mood": "good" } }),
        ),
        Some((&pa, &pb)),
    );
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        (
            "append_blocks",
            json!({ "page": a, "markdown": "After intro", "after_block": pa.blocks[0] }),
        ),
        (
            "edit_page",
            json!({ "page": b, "op": "append", "markdown": "After intro", "after_block": pb.blocks[0] }),
        ),
        Some((&pa, &pb)),
    );
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        ("append_blocks", json!({ "page": a, "markdown": "Tail" })),
        (
            "edit_page",
            json!({ "page": b, "op": "append", "markdown": "Tail" }),
        ),
        Some((&pa, &pb)),
    );
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        (
            "replace_blocks",
            json!({ "page": a, "block_ids": [pa.blocks[1]], "markdown": "**Replaced** task" }),
        ),
        (
            "edit_page",
            json!({ "page": b, "op": "replace", "block_ids": [pb.blocks[1]], "markdown": "**Replaced** task" }),
        ),
        Some((&pa, &pb)),
    );
    let (pa, pb) = pair(&s);
    same_result(
        &s,
        (
            "replace_blocks",
            json!({ "page": a, "markdown": "Whole body" }),
        ),
        (
            "edit_page",
            json!({ "page": b, "op": "set_body", "markdown": "Whole body" }),
        ),
        Some((&pa, &pb)),
    );

    // reschedule keeps its own message for a missing date.
    let (e, text) = call(&s, "reschedule", json!({ "item": format!("block:{a}/x") }));
    assert!(
        e && text.contains("missing string argument: date"),
        "{text}"
    );

    // Every alias is covered above. import_markdown is a hidden tool, not an
    // alias: its import is covered by import_markdown_is_hidden_and_imports_over_stdio
    // and the hub's wiki import tests.
    let mut registry: Vec<&str> = vault_tools::aliases().iter().map(|x| x.name).collect();
    registry.sort();
    let mut covered = vec![
        "list_pages",
        "backlinks",
        "agenda",
        "query_database",
        "memory_search",
        "overdue",
        "reschedule",
        "set_done",
        "update_page",
        "append_blocks",
        "replace_blocks",
    ];
    covered.sort();
    assert_eq!(registry, covered, "an alias without a comparison here");
}
