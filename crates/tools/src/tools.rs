//! Vault tools: definitions (name, description, input schema) and their
//! implementation over a [`Vault`]. Outputs are compact markdown; lists
//! are paginated with `offset`/`limit` and say how to get the next page.
//! Every write goes through the core, so it is in the page history and
//! syncs like an edit made in the app.

use std::sync::{LazyLock, RwLock};

use serde_json::{json, Value};
use vault_core::{Page, PageId, PageSummary, PropDef, Vault};

use crate::props;
use crate::{err, Result, ToolError};

/// Whether an agent host must ask the user before running a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    None,
    Confirm,
}

/// A host that serves vault tools. Each tool lists the surfaces it appears on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The app's own chat agent.
    InAppAgent,
    /// MCP servers: the hub's `/mcp` endpoint and the stdio server.
    Mcp,
}

/// Served on every surface.
const ALL_SURFACES: &[Surface] = &[Surface::InAppAgent, Surface::Mcp];

/// A vault action and the metadata an agent host needs to serve it.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    schema: fn() -> Value,
    pub read_only: bool,
    pub destructive_hint: bool,
    pub approval: Approval,
    surfaces: &'static [Surface],
}

impl ToolDef {
    /// The JSON schema accepted by this tool.
    pub fn input_schema(&self) -> Value {
        (self.schema)()
    }

    /// The JSON schema as served on `surface`. Every surface that serves a
    /// tool takes the same arguments.
    pub fn input_schema_for(&self, _surface: Surface) -> Value {
        self.input_schema()
    }

    /// Whether this tool is served on `surface`.
    pub fn serves(&self, surface: Surface) -> bool {
        self.surfaces.contains(&surface)
    }
}

fn page_arg() -> Value {
    json!({ "type": "string", "description": "Page id (or exact title)" })
}

/// The search_notes description: the operator grammar comes from the
/// operator table in the core, so it can't drift from the parser.
fn search_description() -> &'static str {
    let text = format!(
        "Finds pages, blocks and tasks. With a query, results read like rg: each page's title, id and path, then its matching blocks with context, each with the block id edit_page takes; a short page is shown whole; item hits (to-dos, task notes, dated rows, date mentions) show done state, date and the ref update_task takes. Without a query: lists the page tree (children of parent, or the top level). With backlinks_of: the pages linking to that page. Words are ANDed, the last a prefix; \"exact phrase\"; -word excludes; prefix an operator with - to negate it; commas mean any of. Operators: {}. DATE: 2026, 2026-10, 2026-10-10, today, now, week, month, 7d (last 7 days), +7d (next 7), ranges a..b, comparisons <today or >=2026-09. Unknown keys are plain text. Results stay under {} characters: previews shrink before hits drop, and the result says what it trimmed. Full reference with examples: notes_help topic query.",
        vault_core::search_ops::operator_grammar(),
        vault_core::search_ops::OUTPUT_BUDGET_CHARS,
    );
    Box::leak(text.into_boxed_str())
}

static TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| vec![
    ToolDef {
        name: "search_notes",
        description: search_description(),
        schema: || json!({ "type": "object", "properties": {
            "query": { "type": "string", "description": "Search query; empty or omitted lists pages" },
            "parent": { "type": "string", "description": "Listing only: page id whose children to list; omit for the top level" },
            "depth": { "type": "integer", "default": 1, "maximum": 3, "description": "Listing only: levels to show" },
            "backlinks_of": { "type": "string", "description": "Page id or exact title: list the pages linking to it instead" },
            "limit": { "type": "integer", "maximum": 200, "description": "Default 20 for search (or limit:N in the query), 50 for listing" },
            "offset": { "type": "integer", "default": 0 } } }),
        read_only: true,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "read_page",
        description: "Reads a page as markdown: title, properties and blocks. Use it before edit_page; set block_ids to see every block's id, including nested blocks. Paginated by top-level blocks. include_backlinks adds the pages linking here.",
        schema: || json!({ "type": "object", "properties": {
            "page": page_arg(),
            "block_ids": { "type": "boolean", "default": false },
            "include_backlinks": { "type": "boolean", "default": false },
            "offset": { "type": "integer", "default": 0 },
            "limit": { "type": "integer", "default": 200 } }, "required": ["page"] }),
        read_only: true,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "create_page",
        description: "Creates a page (inside parent, or at the top level) with an optional markdown body. Inside a database it is a row and takes properties by name.",
        schema: || json!({ "type": "object", "properties": {
            "title": { "type": "string", "description": "Page title" },
            "parent": { "type": "string", "description": "Parent page or database id" },
            "markdown": { "type": "string", "description": "Page body as markdown (notes_help topic markdown)" },
            "properties": { "type": "object", "description": "Name: value" } }, "required": ["title"] }),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "edit_page",
        description: "Changes a page. One op per call, or a list of ops applied in order: append (markdown at the end, or after after_block), replace (block_ids with markdown), delete (block_ids), set_body (the whole body as markdown, keeping unchanged blocks), set_props (title, icon emoji, properties by name; null clears). Block ids come from read_page with block_ids. Each op is saved to the page history.",
        schema: || {
            let op = json!({
                "op": { "type": "string", "enum": ["append", "replace", "delete", "set_body", "set_props"] },
                "markdown": { "type": "string", "description": "append, replace, set_body" },
                "after_block": { "type": "string", "description": "append: insert after this block" },
                "block_ids": { "type": "array", "items": { "type": "string" }, "description": "replace, delete" },
                "title": { "type": "string", "description": "set_props" },
                "icon": { "type": ["string", "null"], "description": "set_props" },
                "properties": { "type": "object", "description": "set_props: name: value" } });
            let mut props = op.as_object().cloned().unwrap_or_default();
            props.insert("page".into(), page_arg());
            props.insert("ops".into(), json!({ "type": "array", "description": "Several ops in order instead of op", "items": { "type": "object", "properties": op, "required": ["op"] } }));
            json!({ "type": "object", "properties": props, "required": ["page"] })
        },
        read_only: false,
        // Strictest of the merged tools (replace_blocks). Approval stays None
        // for every op, so an append needs no more approval than append_blocks.
        destructive_hint: true,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "trash_page",
        description: "Moves a page and its sub-pages to the trash (restorable in the app).",
        schema: || json!({ "type": "object", "properties": { "page": page_arg() }, "required": ["page"] }),
        read_only: false,
        destructive_hint: true,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "move_page",
        description: "Moves pages, with their sub-pages, under parent (omit for the top level), right after the sibling after (omit for first). Several pages keep the given order. Moving into or out of a database is a plain move, as in the app. A page can't move into itself or its own subtree. Returns each page's new path.",
        schema: || json!({ "type": "object", "properties": {
            "pages": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "Page ids or exact titles, moved in this order" },
            "parent": { "type": "string", "description": "New parent page id or exact title; omit for the top level" },
            "after": { "type": "string", "description": "Sibling under the new parent to place the pages after; omit for first" } }, "required": ["pages"] }),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "daily_note",
        description: "Returns the daily note's id and title for an ISO date (default today, host local timezone). Creates it once with empty Tasks and Log toggle sections, as the app does, and reuses it thereafter. Use edit_page with op append to write into it.",
        schema: || json!({ "type": "object", "properties": { "date": { "type": "string", "format": "date" } } }),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "update_task",
        description: "Changes a to-do, task note or database row from search_notes item results by its reference (block:<page>/<block>, note:<page> or row:<database>/<row>). due moves it to YYYY-MM-DD (all day) or YYYY-MM-DD HH:MM (host local time, lets the app remind the user); done sets completion. Give due, done or both. Date mentions are read-only.",
        schema: || json!({ "type": "object", "properties": {
            "item": { "type": "string" },
            "due": { "type": "string", "description": "YYYY-MM-DD or YYYY-MM-DD HH:MM" },
            "done": { "type": "boolean" } }, "required": ["item"] }),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "upsert_row",
        description: "Creates a database row, or updates one when row is given. Properties by name; select options are created when new. Optional markdown becomes (or replaces) the row's content.",
        schema: || json!({ "type": "object", "properties": {
            "database": page_arg(),
            "row": { "type": "string", "description": "Row page id to update" },
            "title": { "type": "string" },
            "properties": { "type": "object" },
            "markdown": { "type": "string" } }, "required": ["database"] }),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "memory_read",
        description: "Reads a memory by project/memory or project/subproject/memory. Returns path, name, description, type, extra fields, last modified and markdown body; no page ids.",
        schema: || json!({"type":"object", "properties": {
            "path":{"type":"string", "description":"project/memory or project/subproject/memory"}
        }, "required":["path"]}),
        read_only: true,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "memory_write",
        description: "Creates or replaces a memory's fields and markdown body by project/memory or project/subproject/memory. Creates missing Memory, project and subproject databases. Name defaults to the final path segment; it does not rename the path. Segments: [A-Za-z0-9][A-Za-z0-9._-]{0,79}.",
        schema: || json!({"type":"object", "properties": {
            "path":{"type":"string"}, "name":{"type":"string"},
            "description":{"type":"string"},
            "type":{"type":"string", "enum":["user","feedback","project","reference"]},
            "body":{"type":"string"}
        }, "required":["path","description","type","body"]}),
        read_only: false,
        destructive_hint: true,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "memory_update",
        description: "Merges supplied fields into an existing memory. Body replaces markdown; append adds a paragraph after it. Fields sets existing schema properties (null clears). Name changes the property. New_path moves or renames the same memory into an existing project/subproject, preserving metadata by field and option name without creating schema fields or options. Refuses an occupied destination or incompatible schema. Errors if the memory is missing.",
        schema: || json!({"type":"object", "properties": {
            "path":{"type":"string"}, "name":{"type":"string"},
            "new_path":{"type":"string", "description":"Full destination memory path; its project/subproject must already exist"},
            "description":{"type":"string"},
            "type":{"type":"string", "enum":["user","feedback","project","reference"]},
            "body":{"type":"string"}, "append":{"type":"string"},
            "fields":{"type":"object"}
        }, "required":["path"]}),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "memory_delete",
        description: "Moves an existing memory to the vault trash, restorable in the app. Address by project/memory or project/subproject/memory; no page ids.",
        schema: || json!({"type":"object", "properties": {
            "path":{"type":"string", "description":"project/memory or project/subproject/memory"}
        }, "required":["path"]}),
        read_only: false,
        destructive_hint: true,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "feedback_submit",
        description: "Files a report on the same hidden Feedback page as app reports. Use it when a vault tool fails unexpectedly, when the user complains about Scratch, sync or the vault, or when the user explicitly asks for a report. Do not file a second report for an issue already filed in this session. The summary is one plain line; details are markdown. Credential patterns (Bearer values, token, key, secret and password parameters, and 64-digit hex tokens) are redacted before storing.",
        schema: || json!({"type":"object", "properties": {
            "summary":{"type":"string", "description":"One line: what is wrong or wanted"},
            "details":{"type":"string", "description":"Optional markdown with more detail"},
            "kind":{"type":"string", "enum":["bug","complaint","idea","other"], "default":"bug", "description":"Report kind"},
            "page_id":{"type":"string", "description":"Optional page the feedback is about"},
            "tool":{"type":"string", "description":"Optional tool name the feedback is about"},
            "error":{"type":"string", "description":"Optional error message seen"}
        }, "required": ["summary"]}),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
    ToolDef {
        name: "notes_help",
        description: "How Scratch notes are written: markdown and blocks, links and embeds, properties and databases, day notes, tasks and dates, templates, hidden system pages, memory pages, and the tool list. Call it without a topic for the table of contents, then with a topic id. Use it before writing anything beyond plain paragraphs.",
        schema: || json!({"type":"object", "properties": {
            "topic":{"type":"string", "description":"Topic id from the table of contents; omit for the contents"}
        }}),
        read_only: true,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: ALL_SURFACES,
    },
]);

/// Tools that are callable over MCP but never listed. `memory_list` is the
/// memory index the CLI reads (it was an alias of the old memory_search
/// listing). `feedback_list` left
/// the tool list; its code stays for the user (`notes feedback` and direct
/// calls). `import_markdown` is the repo import the `notes wiki` CLI sends;
/// models never see it.
static HIDDEN: &[ToolDef] = &[
    ToolDef {
        // The compact memory index the `notes memory` CLI and the plugin's
        // session-start hook read; agents search memories with search_notes.
        name: "memory_list",
        description: "Lists memory projects, or the memories under a path as a compact index sorted by path (path, description, type).",
        schema: || json!({"type":"object", "properties": {
            "path":{"type":"string", "description":"project or project/subproject; omit to list projects"}
        }}),
        read_only: true,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: &[Surface::Mcp],
    },
    ToolDef {
        name: "feedback_list",
        description: "Lists feedback reports, newest first: id, title, created time (UTC), kind, source (mcp or app) and first line. Optional kind filter.",
        schema: || json!({"type":"object", "properties": {
            "limit":{"type":"integer", "minimum":0, "maximum":100, "default":20, "description":"Reports to return"},
            "kind":{"type":"string", "enum":["bug","complaint","idea","other","app"]}
        }}),
        read_only: true,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: &[Surface::Mcp],
    },
    ToolDef {
        name: "import_markdown",
        description: "Imports markdown folders (plain tar archives, base64) into the Projects database as the row titled name: one row per exact name, reused on repeat, status kept once set. Returns the row id, created or updated, and per root the import report. dry_run imports into an empty temporary vault and writes nothing.",
        schema: || json!({ "type": "object", "properties": {
            "name": { "type": "string", "description": "The project row's exact title" },
            "roots": { "type": "array", "minItems": 1, "description": "Markdown folders to import", "items": { "type": "object", "properties": {
                "name": { "type": "string", "description": "Root name; its container's title when there are several roots" },
                "archive_base64": { "type": "string", "description": "Plain (not gzipped) tar of the markdown folder, base64 encoded" },
                "exclude": { "type": "array", "items": { "type": "string" }, "description": "Relative paths or globs under this root" }
            }, "required": ["name", "archive_base64"] } },
            "repo_path": { "type": "string" },
            "git_remote": { "type": ["string", "null"] },
            "last_activity": { "type": ["string", "null"], "description": "RFC 3339 time with offset (e.g. git %cI), or an ISO date; omit or null when the source has no activity" },
            "exclude": { "type": "array", "items": { "type": "string" }, "description": "Relative paths or globs excluded from every root" },
            "dry_run": { "type": "boolean" } }, "required": ["name", "roots"] }),
        read_only: false,
        destructive_hint: false,
        approval: Approval::None,
        surfaces: &[Surface::Mcp],
    },
];

/// An old tool name kept as a hidden alias: accepted when called, never
/// listed, and mapped to its new tool with rewritten arguments.
///
/// Legacy names from plugin 0.5.0 remain callable for compatibility.
/// `agenda`, `query_database` and `memory_search` became aliases of
/// search_notes in 0.6.3; remove them in the following version. Their
/// results follow search: rows of databases whose agenda role is off are
/// included (the role is a calendar setting only).
/// The compatibility contracts are checked in `tests/registry.rs`.
pub struct Alias {
    pub name: &'static str,
    pub target: &'static str,
    rewrite: fn(&Value) -> Result<Value>,
}

impl Alias {
    /// The new tool's arguments for an old-name call, or the error the old
    /// tool gave for arguments the new one reads differently.
    pub fn rewrite(&self, a: &Value) -> Result<Value> {
        (self.rewrite)(a)
    }
}

/// `a` as an object, with `extra` merged in (extra wins).
fn with(a: &Value, extra: Value) -> Value {
    let mut out = a.as_object().cloned().unwrap_or_default();
    if let Value::Object(extra) = extra {
        out.extend(extra);
    }
    Value::Object(out)
}

/// `a` as an object without `keys`.
fn without(a: &Value, keys: &[&str]) -> Value {
    let mut out = a.as_object().cloned().unwrap_or_default();
    for k in keys {
        out.remove(*k);
    }
    Value::Object(out)
}

// REMOVE (aliases): see [`Alias`].
static ALIASES: &[Alias] = &[
    Alias {
        name: "list_pages",
        target: "search_notes",
        rewrite: |a| Ok(without(a, &["query", "backlinks_of"])),
    },
    Alias {
        name: "backlinks",
        target: "search_notes",
        rewrite: |a| Ok(json!({ "backlinks_of": req(a, "page")? })),
    },
    Alias {
        name: "update_page",
        target: "edit_page",
        rewrite: |a| Ok(with(a, json!({ "op": "set_props" }))),
    },
    Alias {
        name: "append_blocks",
        target: "edit_page",
        rewrite: |a| Ok(with(a, json!({ "op": "append" }))),
    },
    Alias {
        name: "replace_blocks",
        target: "edit_page",
        // Without block_ids replace_blocks replaced the whole body.
        rewrite: |a| {
            let some_ids = a
                .get("block_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(Value::is_string));
            Ok(with(
                a,
                json!({ "op": if some_ids { "replace" } else { "set_body" } }),
            ))
        },
    },
    Alias {
        // Replaced by search_notes item queries in plugin 0.6.3; remove in
        // the following version.
        name: "agenda",
        target: "search_notes",
        rewrite: crate::query::agenda_query,
    },
    Alias {
        name: "overdue",
        target: "search_notes",
        rewrite: crate::query::overdue_query,
    },
    Alias {
        // Replaced by search_notes `in:[[Database]]` in plugin 0.6.3.
        name: "query_database",
        target: "search_notes",
        rewrite: crate::query::database_query,
    },
    Alias {
        // Replaced by search_notes `in:memory` in plugin 0.6.3.
        name: "memory_search",
        target: "search_notes",
        rewrite: crate::query::memory_query,
    },
    Alias {
        name: "reschedule",
        target: "update_task",
        // Keeps date and time apart so their old validation messages hold.
        rewrite: |a| {
            let mut out = without(a, &["date", "done"]);
            out["due"] = a.get("date").cloned().unwrap_or(Value::Null);
            // update_task reads a null due as absent; reschedule required it.
            if !out["due"].is_string() {
                if !out["item"].is_string() {
                    return err("missing string argument: item");
                }
                return err("missing string argument: date");
            }
            Ok(out)
        },
    },
    Alias {
        name: "set_done",
        target: "update_task",
        // A missing done still fails as set_done did.
        rewrite: |a| {
            let mut out = without(a, &["due", "time"]);
            if out.get("done").is_none() {
                out["done"] = Value::Null;
            }
            Ok(out)
        },
    },
];

/// The listed vault tool definitions, in their serving order.
pub fn definitions() -> &'static [ToolDef] {
    TOOLS.as_slice()
}

/// The listed tool definitions served on `surface`, in their serving order.
pub fn definitions_for(surface: Surface) -> impl Iterator<Item = &'static ToolDef> {
    TOOLS.iter().filter(move |t| t.serves(surface))
}

/// The hidden aliases of old tool names.
pub fn aliases() -> &'static [Alias] {
    ALIASES
}

/// Tools that are callable but not listed (not aliases).
pub fn hidden() -> &'static [ToolDef] {
    HIDDEN
}

/// Finds the definition of a listed tool by name.
pub fn get(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Finds the definition a call to `name` runs: a listed tool, a hidden
/// tool, or an alias's new tool. Use it to check names before dispatch
/// (scopes, allow-lists, schedules) so old names keep working.
pub fn lookup(name: &str) -> Option<&'static ToolDef> {
    get(name)
        .or_else(|| HIDDEN.iter().find(|t| t.name == name))
        .or_else(|| alias(name).and_then(|a| get(a.target)))
}

/// The alias for an old tool name.
pub fn alias(name: &str) -> Option<&'static Alias> {
    ALIASES.iter().find(|a| a.name == name)
}

/// The tool name a call to `name` runs (the alias's target, or `name`).
pub fn canonical(name: &str) -> &str {
    alias(name).map_or(name, |a| a.target)
}

/// Runs tool `name` as served on `surface`: tools another surface serves
/// are unknown here.
pub fn call_on(surface: Surface, v: &Vault, name: &str, a: &Value) -> Result<String> {
    let (name, a) = match alias(name) {
        Some(x) => (x.target, std::borrow::Cow::Owned(x.rewrite(a)?)),
        None => (name, std::borrow::Cow::Borrowed(a)),
    };
    let Some(def) = lookup(name) else {
        return err(format!("unknown tool: {name}"));
    };
    if !def.serves(surface) {
        return err(format!("unknown tool: {name}"));
    }
    if surface == Surface::InAppAgent && name == "feedback_submit" {
        return crate::feedback::submit(v, &a, IN_APP_TRANSPORT);
    }
    dispatch(v, name, &a)
}

/// The transport recorded on feedback the in-app agent files.
pub const IN_APP_TRANSPORT: &str = "in-app agent";

/// Runs tool `name` (listed, hidden or an alias); the text result, or an
/// error for the assistant.
pub fn call(v: &Vault, name: &str, a: &Value) -> Result<String> {
    match alias(name) {
        Some(x) => dispatch(v, x.target, &x.rewrite(a)?),
        None => dispatch(v, name, a),
    }
}

fn dispatch(v: &Vault, name: &str, a: &Value) -> Result<String> {
    match name {
        "memory_list" | "memory_read" | "memory_write" | "memory_update" | "memory_delete" => {
            crate::memory::call(v, name, a)
        }
        "update_task" => update_task(v, a),
        "daily_note" => crate::agenda::daily_note(v, a),
        "search_notes" => search_or_list(v, a),
        "read_page" => read_page(v, a),
        "create_page" => create_page_only(v, a),
        "import_markdown" => {
            crate::wiki_import::call(v, a, crate::wiki_import::DEFAULT_SOURCE_LIMIT)
        }
        "edit_page" => edit_page(v, a),
        "upsert_row" => upsert_row(v, a),
        "trash_page" => trash_page(v, a),
        "move_page" => crate::organize::move_page(v, a),
        // Hosts that reach the server over HTTP call `feedback::submit` with their transport.
        "feedback_submit" => crate::feedback::submit(v, a, "stdio"),
        "feedback_list" => crate::feedback::list(v, a),
        "notes_help" => crate::help::notes_help(a),
        _ => err(format!("unknown tool: {name}")),
    }
}

// ------------------------------------------------------------ merged tools

fn search_or_list(v: &Vault, a: &Value) -> Result<String> {
    let query = s(a, "query").filter(|q| !q.trim().is_empty());
    match (a.get("backlinks_of").filter(|b| !b.is_null()), query) {
        (Some(_), Some(_)) => err("give query or backlinks_of, not both"),
        (Some(b), None) => backlinks(v, &json!({ "page": b })),
        (None, Some(_)) => crate::query::search(v, a),
        (None, None) => list_pages(v, a),
    }
}

/// When a call to `name` is a repo import (the hidden `import_markdown`),
/// the arguments for [`crate::wiki_import::call`]. Hosts with their own
/// archive limit (the hub) run imports through it.
pub fn import_args(name: &str, a: &Value) -> Option<Result<Value>> {
    (name == "import_markdown").then(|| Ok(a.clone()))
}

/// The error for create_page with roots: repo imports moved to their own tool.
pub const CREATE_PAGE_ROOTS_ERROR: &str = "create_page no longer imports archives (roots); repo imports use import_markdown, sent by the `notes wiki` CLI";

fn create_page_only(v: &Vault, a: &Value) -> Result<String> {
    if a.get("roots").is_some() {
        return err(CREATE_PAGE_ROOTS_ERROR);
    }
    create_page(v, a)
}

const EDIT_OPS: &[&str] = &["append", "replace", "delete", "set_body", "set_props"];

fn edit_page(v: &Vault, a: &Value) -> Result<String> {
    let page = a.get("page").cloned().unwrap_or(Value::Null);
    let ops: Vec<Value> = match (a.get("op"), a.get("ops")) {
        (Some(_), Some(_)) => return err("give op or ops, not both"),
        (Some(_), None) => vec![without(a, &["page", "ops"])],
        (None, Some(Value::Array(list))) if !list.is_empty() => list.clone(),
        (None, Some(_)) => return err("ops must be a non-empty list of op objects"),
        (None, None) => {
            return err(format!(
                "missing argument: op (one of {})",
                EDIT_OPS.join(", ")
            ))
        }
    };
    let several = ops.len() > 1;
    let mut out = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let args = with(op, json!({ "page": page.clone() }));
        match edit_op(v, &args) {
            Ok(r) if several => out.push(format!("{}. {r}", i + 1)),
            Ok(r) => out.push(r),
            Err(e) if several => {
                return err(format!(
                    "op {} failed: {}{}",
                    i + 1,
                    e.0,
                    if i > 0 {
                        format!(" (ops 1–{i} were applied:\n{})", out.join("\n"))
                    } else {
                        String::new()
                    }
                ))
            }
            Err(e) => return Err(e),
        }
    }
    // One page link for the whole call; update_page is an alias of set_props here.
    let id = page_of(v, a, "page")?;
    out.push(format!("link: {}", page_link(&id, None)));
    Ok(out.join("\n"))
}

/// One edit_page op, run by the tool it replaces so its validation and
/// history stay the same.
fn edit_op(v: &Vault, a: &Value) -> Result<String> {
    let ids_given = a
        .get("block_ids")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(Value::is_string));
    match req(a, "op")? {
        "append" => append_blocks(v, a),
        "replace" if !ids_given => err("replace needs block_ids; use set_body for the whole page"),
        "replace" => replace_blocks(v, a),
        "delete" if !ids_given => err("delete needs block_ids"),
        "delete" => replace_blocks(v, &with(a, json!({ "markdown": "" }))),
        "set_body" => replace_blocks(v, &without(a, &["block_ids"])),
        "set_props" => update_page(v, a),
        other => err(format!(
            "unknown op {other:?} (one of {})",
            EDIT_OPS.join(", ")
        )),
    }
}

fn update_task(v: &Vault, a: &Value) -> Result<String> {
    let due = a.get("due").filter(|d| !d.is_null());
    let done = a.get("done");
    if due.is_none() && done.is_none() {
        return err("give due, done or both");
    }
    if done.is_some_and(|d| !d.is_boolean()) {
        return err("missing boolean argument: done");
    }
    let item = a.get("item").cloned().unwrap_or(Value::Null);
    let mut result = serde_json::Map::new();
    if let Some(due) = due {
        let mut args = json!({ "item": item.clone() });
        match (due.as_str(), a.get("time")) {
            // The reschedule alias passes date and time apart.
            (_, Some(time)) => {
                args["date"] = due.clone();
                args["time"] = time.clone();
            }
            (Some(d), None) => match d.trim().split_once([' ', 'T']) {
                Some((date, time)) => {
                    args["date"] = json!(date);
                    args["time"] = json!(time.trim());
                }
                None => args["date"] = json!(d.trim()),
            },
            (None, None) => return err("due must be YYYY-MM-DD or YYYY-MM-DD HH:MM"),
        }
        let out = crate::agenda::reschedule(v, &args)?;
        if done.is_none() {
            return Ok(out);
        }
        if let Ok(Value::Object(m)) = serde_json::from_str(&out) {
            result.extend(m);
        }
    }
    if let Some(done) = done {
        let out = crate::agenda::set_done(v, &json!({ "item": item, "done": done }))?;
        if due.is_none() {
            return Ok(out);
        }
        if let Ok(Value::Object(m)) = serde_json::from_str(&out) {
            result.extend(m);
        }
    }
    Ok(Value::Object(result).to_string())
}

// ------------------------------------------------------------ arguments

fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn req<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    s(a, k).ok_or_else(|| ToolError(format!("missing argument: {k}")))
}

fn num(a: &Value, k: &str, default: u32, max: u32) -> u32 {
    a.get(k)
        .and_then(Value::as_u64)
        .map(|n| n.min(max as u64) as u32)
        .unwrap_or(default)
}

/// A page id from an id or an exact (case-insensitive) title.
pub fn resolve_page(v: &Vault, p: &str) -> Result<PageId> {
    let p = p.trim();
    if p.is_empty() {
        return err("empty page id");
    }
    if v.load_page(p.to_string()).is_ok() {
        return Ok(p.to_string());
    }
    let hits = v.search_pages(p.to_string(), 50).unwrap_or_default();
    let same: Vec<_> = hits
        .iter()
        .filter(|h| h.title.eq_ignore_ascii_case(p))
        .collect();
    match same.len() {
        1 => Ok(same[0].id.clone()),
        0 => {
            let near = crate::query::title_matches(v, p, 5);
            if near.is_empty() {
                err(format!("no page {p:?}; use search_notes to find its id"))
            } else {
                err(format!(
                    "no page {p:?}; titles containing those words:\n{}",
                    near.join("\n")
                ))
            }
        }
        _ => err(format!(
            "several pages are titled {p:?}: {}",
            same.iter()
                .map(|h| h.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn page_of(v: &Vault, a: &Value, k: &str) -> Result<PageId> {
    resolve_page(v, req(a, k)?)
}

/// The database a page is a row of.
fn database_of(v: &Vault, page: &Page) -> Option<PageId> {
    let parent = page.parent.clone()?;
    v.load_page(parent.clone())
        .ok()
        .filter(|p| p.is_database())
        .map(|_| parent)
}

fn more(shown: usize, offset: u32, total: Option<u32>, has_more: bool) -> String {
    let end = offset as usize + shown;
    match total {
        Some(t) if (end as u32) < t => {
            format!("\n_{}–{} of {t}. More: offset={end}_", offset + 1, end)
        }
        None if has_more => format!("\n_More: offset={end}_"),
        _ => String::new(),
    }
}

// ------------------------------------------------------------ tools

/// Base of the page links agents see. A hub sets it when it starts (its
/// `public_url`, or its loopback address); until then the desktop's embedded
/// hub address is used.
static LINK_BASE: RwLock<Option<String>> = RwLock::new(None);

/// The base when no hub has set one: the desktop app's hub.
const DESKTOP_LINK_BASE: &str = "http://127.0.0.1:47100";

/// Sets the base of the links the tools return, for example
/// `https://notes.example.com`, without a trailing slash. `None` restores the
/// desktop default.
pub fn set_link_base(base: Option<&str>) {
    let base = base.map(|b| b.trim_end_matches('/').to_string());
    *LINK_BASE.write().unwrap_or_else(|e| e.into_inner()) = base;
}

/// The link that opens a page, or one block of it, in the browser: an http
/// address that bounces to `notes://page/...` (the Claude app only opens
/// http links from chat).
fn page_link(page: &str, block: Option<&str>) -> String {
    let base = LINK_BASE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| DESKTOP_LINK_BASE.to_string());
    match block {
        Some(b) => format!("{base}/open/{page}?block={b}"),
        None => format!("{base}/open/{page}"),
    }
}

fn title_or(t: &str) -> &str {
    if t.trim().is_empty() {
        "Untitled"
    } else {
        t
    }
}

fn read_page(v: &Vault, a: &Value) -> Result<String> {
    let name = req(a, "page")?;
    let id = match resolve_page(v, name) {
        Ok(id) => id,
        // Never opens a near match: the hits are listed as search results.
        Err(e) if e.0.starts_with("no page ") => return err(crate::query::read_page_miss(v, name.trim())),
        Err(e) => return Err(e),
    };
    let page = v.load_page(id.clone())?;
    let offset = num(a, "offset", 0, u32::MAX) as usize;
    let limit = num(a, "limit", 200, 1000).max(1) as usize;
    let with_ids = a.get("block_ids").and_then(Value::as_bool).unwrap_or(false);
    let mut out = format!(
        "# {}{}\n",
        page.icon
            .as_deref()
            .map(|i| format!("{i} "))
            .unwrap_or_default(),
        title_or(&page.title)
    );
    let mut meta = vec![
        format!("id `{}`", page.id),
        format!("link: {}", page_link(&page.id, None)),
    ];
    let db = database_of(v, &page);
    match (&db, &page.parent) {
        (Some(d), _) => meta.push(format!("row of database `{d}`")),
        (None, Some(p)) => meta.push(format!("parent `{p}`")),
        _ => meta.push("top level".into()),
    }
    if page.is_database() {
        meta.push(format!("database (its rows: search_notes in:[[{}]])", page.id));
    }
    if page.in_trash {
        meta.push("in the trash".into());
    }
    meta.push(format!("updated {}", props::iso(page.updated_ms, true)));
    out.push_str(&meta.join(" · "));
    out.push('\n');
    let schema = db
        .as_ref()
        .and_then(|d| v.get_schema(d.clone()).ok())
        .unwrap_or_default();
    if !page.properties.is_empty() {
        out.push('\n');
        for p in &page.properties {
            let def = schema.iter().find(|d| d.key == p.key);
            let name = def.map(|d| d.name.as_str()).unwrap_or(&p.key);
            out.push_str(&format!("- {name}: {}\n", props::display(v, &p.value, def)));
        }
    }
    if page.is_database() {
        if let Ok(s) = v.get_schema(id.clone()) {
            out.push_str("\nProperties:\n");
            out.push_str(&props::schema_lines(&s));
        }
    }
    let total = page.blocks.len();
    let slice: Vec<_> = page
        .blocks
        .iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect();
    out.push_str("\n---\n");
    if slice.is_empty() {
        out.push_str(if total == 0 {
            "(empty page)"
        } else {
            "(no blocks at this offset)"
        });
    } else if with_ids {
        blocks_with_ids(v, &slice, "", &mut out);
    } else {
        out.push_str(&v.blocks_markdown(slice.clone()));
    }
    let shown = slice.len();
    if offset + shown < total {
        out.push_str(&format!(
            "\n\n_Blocks {}–{} of {total}. More: offset={}_",
            offset + 1,
            offset + shown,
            offset + shown
        ));
    }
    if a.get("include_backlinks")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        out.push_str("\n\n---\nBacklinks:\n");
        out.push_str(&backlinks(v, &json!({ "page": id }))?);
    }
    Ok(out.trim_end().to_string())
}

/// Render each block once, keeping its id beside its own content rather than
/// hiding descendants inside the parent's markdown. The core renderer handles
/// each block's marks, metadata and fences; indentation follows its tree rules.
fn blocks_with_ids(v: &Vault, blocks: &[vault_core::Block], indent: &str, out: &mut String) {
    let mut number = 0;
    for b in blocks {
        number = if b.kind == vault_core::BlockKind::Numbered {
            number + 1
        } else {
            0
        };
        let mut own = b.clone();
        own.children.clear();
        let mut markdown = v.blocks_markdown(vec![own]);
        if number > 1 {
            if let Some(text) = markdown.strip_prefix("1. ") {
                // Standalone numbered blocks render as `1.`. Keep sibling
                // numbering and align multiline text after wider markers.
                let padding = " ".repeat(number.to_string().len() - 1);
                markdown = format!("{number}. {}", text.replace('\n', &format!("\n{padding}")));
            }
        }
        out.push_str(&format!("{indent}<!-- block {} -->\n", b.id));
        for line in markdown.split('\n') {
            out.push_str(indent);
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        let width = if number > 0 {
            format!("{number}. ").len()
        } else {
            2
        };
        blocks_with_ids(
            v,
            &b.children,
            &format!("{indent}{}", " ".repeat(width)),
            out,
        );
    }
}

fn summary_line(p: &PageSummary) -> String {
    let mut s = format!(
        "{}**{}** `{}`",
        p.icon
            .as_deref()
            .map(|i| format!("{i} "))
            .unwrap_or_default(),
        title_or(&p.title),
        p.id
    );
    if p.is_database() {
        s.push_str(" (database)");
    }
    if p.child_count > 0 {
        s.push_str(&format!(" · {} inside", p.child_count));
    }
    s
}

fn list_pages(v: &Vault, a: &Value) -> Result<String> {
    let parent = match s(a, "parent") {
        Some(p) if !p.trim().is_empty() => Some(resolve_page(v, p)?),
        _ => None,
    };
    let depth = num(a, "depth", 1, 3).max(1);
    let offset = num(a, "offset", 0, u32::MAX);
    let limit = num(a, "limit", 50, 200).max(1);
    let kids = v.list_children(parent.clone(), offset, limit + 1)?;
    let has_more = kids.len() > limit as usize;
    let mut out = String::new();
    let mut lines = 0usize;
    fn walk(
        v: &Vault,
        p: &PageSummary,
        level: u32,
        depth: u32,
        out: &mut String,
        lines: &mut usize,
    ) {
        if level >= depth || p.child_count == 0 || p.is_database() {
            return;
        }
        let kids = v
            .list_children(Some(p.id.clone()), 0, 26)
            .unwrap_or_default();
        for (i, k) in kids.iter().enumerate() {
            if *lines >= 400 {
                return;
            }
            if i == 25 {
                out.push_str(&format!(
                    "{}- … (list_pages parent={} for more)\n",
                    "  ".repeat(level as usize),
                    p.id
                ));
                break;
            }
            out.push_str(&format!(
                "{}- {}\n",
                "  ".repeat(level as usize),
                summary_line(k)
            ));
            *lines += 1;
            walk(v, k, level + 1, depth, out, lines);
        }
    }
    for k in kids.iter().take(limit as usize) {
        out.push_str(&format!("- {}\n", summary_line(k)));
        lines += 1;
        walk(v, k, 1, depth, &mut out, &mut lines);
    }
    if out.is_empty() {
        return Ok(if parent.is_some() {
            "No pages inside.".into()
        } else {
            "The vault is empty.".into()
        });
    }
    out.push_str(&more(
        kids.len().min(limit as usize),
        offset,
        None,
        has_more,
    ));
    Ok(out.trim_end().to_string())
}

fn create_page(v: &Vault, a: &Value) -> Result<String> {
    let title = req(a, "title")?;
    let parent = match s(a, "parent") {
        Some(p) if !p.trim().is_empty() => Some(resolve_page(v, p)?),
        _ => None,
    };
    let db = parent
        .as_ref()
        .and_then(|p| v.load_page(p.clone()).ok())
        .filter(|p| p.is_database())
        .map(|p| p.id);
    let id = v.create_page(parent, title.to_string())?;
    if let Some(p) = a.get("properties").filter(|p| !p.is_null()) {
        props::set_props(v, &id, db.as_ref(), p)?;
    }
    if let Some(md) = s(a, "markdown").filter(|m| !m.trim().is_empty()) {
        v.insert_markdown(id.clone(), None, None, true, md.to_string())?;
    }
    Ok(format!(
        "Created **{}** `{id}`\nlink: {}",
        title_or(title),
        page_link(&id, None)
    ))
}

fn append_blocks(v: &Vault, a: &Value) -> Result<String> {
    let id = page_of(v, a, "page")?;
    let md = req(a, "markdown")?;
    let after = s(a, "after_block")
        .filter(|b| !b.is_empty())
        .map(String::from);
    // A block anywhere in the page: insert next to it, under its parent.
    let parent = match &after {
        Some(b) => {
            let page = v.load_page(id.clone())?;
            Some(
                parent_in(&page.blocks, None, b)
                    .ok_or_else(|| ToolError(format!("no block {b} in the page")))?,
            )
        }
        None => None,
    };
    let ids = v.insert_markdown(
        id.clone(),
        parent.flatten(),
        after.clone(),
        after.is_none(),
        md.to_string(),
    )?;
    let link = ids
        .first()
        .map(|b| format!("\nblock link: {}", page_link(&id, Some(b.as_str()))))
        .unwrap_or_default();
    Ok(format!(
        "Added {} block(s): {}{link}",
        ids.len(),
        ids.join(", ")
    ))
}

fn parent_in(
    blocks: &[vault_core::Block],
    parent: Option<&str>,
    id: &str,
) -> Option<Option<String>> {
    for b in blocks {
        if b.id == id {
            return Some(parent.map(String::from));
        }
        if let Some(x) = parent_in(&b.children, Some(&b.id), id) {
            return Some(x);
        }
    }
    None
}

fn replace_blocks(v: &Vault, a: &Value) -> Result<String> {
    let id = page_of(v, a, "page")?;
    let md = req(a, "markdown")?;
    let ids: Vec<String> = a
        .get("block_ids")
        .and_then(Value::as_array)
        .map(|x| {
            x.iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let whole = ids.is_empty();
    let n = ids.len();
    let new = v.replace_markdown(id.clone(), ids, md.to_string())?;
    let link = new
        .first()
        .map(|b| format!("\nblock link: {}", page_link(&id, Some(b.as_str()))))
        .unwrap_or_default();
    Ok(if whole {
        format!(
            "Replaced the page body ({} top-level block(s) now).{link}",
            new.len()
        )
    } else {
        format!(
            "Replaced {n} block(s) with {}: {}{link}",
            new.len(),
            new.join(", ")
        )
    })
}

fn update_page(v: &Vault, a: &Value) -> Result<String> {
    let id = page_of(v, a, "page")?;
    let page = v.load_page(id.clone())?;
    let mut done = Vec::new();
    if let Some(t) = s(a, "title") {
        v.rename_page(id.clone(), t.to_string())?;
        done.push("title".to_string());
    }
    if let Some(i) = a.get("icon") {
        v.set_icon(
            id.clone(),
            i.as_str().filter(|x| !x.is_empty()).map(String::from),
        )?;
        done.push("icon".into());
    }
    if let Some(p) = a.get("properties").filter(|p| !p.is_null()) {
        let db = database_of(v, &page);
        done.extend(props::set_props(v, &id, db.as_ref(), p)?);
    }
    if done.is_empty() {
        return err("nothing to change: give title, icon or properties");
    }
    Ok(format!(
        "Updated **{}**: {}",
        title_or(&page.title),
        done.len()
    ))
}

fn database(v: &Vault, a: &Value) -> Result<(PageId, Vec<PropDef>)> {
    let id = page_of(v, a, "database")?;
    let p = v.load_page(id.clone())?;
    if !p.is_database() {
        return err(format!("{} is not a database", title_or(&p.title)));
    }
    let schema = v.get_schema(id.clone())?;
    Ok((id, schema))
}

fn upsert_row(v: &Vault, a: &Value) -> Result<String> {
    let (db, _) = database(v, a)?;
    let (row, created) = match s(a, "row").filter(|r| !r.is_empty()) {
        Some(r) => {
            let r = resolve_page(v, r)?;
            let p = v.load_page(r.clone())?;
            if p.parent.as_deref() != Some(db.as_str()) {
                return err(format!("{r} is not a row of this database"));
            }
            if let Some(t) = s(a, "title") {
                v.rename_page(r.clone(), t.to_string())?;
            }
            (r, false)
        }
        None => (
            v.create_page(
                Some(db.clone()),
                s(a, "title").unwrap_or_default().to_string(),
            )?,
            true,
        ),
    };
    if let Some(p) = a.get("properties").filter(|p| !p.is_null()) {
        props::set_props(v, &row, Some(&db), p)?;
    }
    if let Some(md) = s(a, "markdown") {
        if created {
            v.insert_markdown(row.clone(), None, None, true, md.to_string())?;
        } else {
            v.replace_markdown(row.clone(), vec![], md.to_string())?;
        }
    }
    let title = v.load_page(row.clone())?.title;
    Ok(format!(
        "{} row **{}** `{row}`",
        if created { "Created" } else { "Updated" },
        title_or(&title)
    ))
}

fn backlinks(v: &Vault, a: &Value) -> Result<String> {
    let id = page_of(v, a, "page")?;
    // Links come from the background writer; make recent edits visible.
    let _ = v.flush();
    let list = v.backlinks(id)?;
    if list.is_empty() {
        return Ok("No pages link here.".into());
    }
    Ok(list
        .iter()
        .map(|p| format!("- {}", summary_line(p)))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn trash_page(v: &Vault, a: &Value) -> Result<String> {
    let id = page_of(v, a, "page")?;
    let title = v.load_page(id.clone())?.title;
    v.delete_page(id.clone())?;
    Ok(format!(
        "Moved **{}** `{id}` to the trash.",
        title_or(&title)
    ))
}

#[cfg(test)]
mod search_notes_tests {
    use super::*;
    use vault_core::{new_id, PropValue};

    #[test]
    fn operators_reading_warnings_and_matched_values_reach_the_agent() {
        let root = std::env::temp_dir().join(format!("notes-tools-filters-{}", new_id()));
        {
            let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
            let wanted = vault.create_page(None, "Work meeting".into()).unwrap();
            let other = vault.create_page(None, "Home meeting".into()).unwrap();
            for (id, tag) in [(&wanted, "work"), (&other, "home")] {
                vault
                    .set_property(
                        id.clone(),
                        "tags".into(),
                        PropValue::MultiSelect(vec![tag.into()]),
                    )
                    .unwrap();
            }
            let output = crate::query::search(
                &vault,
                &serde_json::json!({"query":"meeting tag:work created:banana"}),
            )
            .unwrap();
            assert!(output.contains("Read as: `meeting tag:work`"), "{output}");
            assert!(output.contains("Warnings:"), "{output}");
            assert!(output.contains("created:"), "{output}");
            assert!(output.contains(&wanted), "{output}");
            assert!(!output.contains(&other), "{output}");
            assert!(output.contains("[tag: work]"), "{output}");
            let empty = crate::query::search(
                &vault,
                &serde_json::json!({"query":"unfindableword created:banana"}),
            )
            .unwrap();
            assert!(empty.contains("Warnings:"), "{empty}");
            assert!(empty.contains("No pages match"), "{empty}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
