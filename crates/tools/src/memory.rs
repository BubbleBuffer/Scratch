//! Agent memory: six path-based tools over vault databases.
//!
//! A top-level `Memory` database holds one database per project; a project
//! holds memory pages (rows) and optional subproject databases with the same
//! schema. Addresses are `project/memory` or `project/subproject/memory`.
//! Only `memory_write` creates databases; no tool turns a memory into a
//! database or the reverse. Results are compact markdown with paths, not ids.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde_json::{json, Map, Value};
use vault_core::{
    Page, PageId, PageSummary, PropDef, PropKind, PropValue, Property, SearchHit, SelectOption,
    Vault, VAULT_SOURCE,
};

use crate::props;
use crate::{err, Result, ToolError};

const MEMORY: &str = "Memory";
const TYPES: [&str; 4] = ["user", "feedback", "project", "reference"];
/// The fields every memory has; they are shown even when empty.
const CORE: [&str; 3] = ["name", "description", "type"];
// Resolution and creation span several core calls. Serialize memory requests
// so simultaneous first writes cannot create duplicate containers, and reads
// cannot observe a container before its schema is installed.
static ACCESS: Mutex<()> = Mutex::new(());

/// Runs memory tool `name` (one of the `memory_*` names in `tools.rs`).
pub(crate) fn call(v: &Vault, name: &str, a: &Value) -> Result<String> {
    let _access = ACCESS.lock().unwrap_or_else(|e| e.into_inner());
    match name {
        "memory_list" => list(v, a),
        "memory_search" => search(v, a),
        "memory_read" => read(v, a),
        "memory_write" => write(v, a),
        "memory_update" => update(v, a),
        "memory_delete" => delete(v, a),
        _ => err(format!("unknown tool: {name}")),
    }
}

// ------------------------------------------------------------ arguments and paths

fn missing(k: &str) -> ToolError {
    match k {
        "path" => ToolError(
            "missing argument: path (the memory's path, e.g. project/memory or project/subproject/memory)"
                .into(),
        ),
        _ => ToolError(format!("missing argument: {k}")),
    }
}

/// A text argument as supplied, untrimmed and possibly empty; `None` when
/// absent. Anything else is an error, never a default.
fn text<'a>(a: &'a Value, k: &str) -> Result<Option<&'a str>> {
    match a.get(k) {
        None => Ok(None),
        Some(v) => match v.as_str() {
            Some(t) => Ok(Some(t)),
            None => err(format!("{k} must be text")),
        },
    }
}

/// The `path` argument, exactly as supplied.
fn path_arg(a: &Value) -> Result<&str> {
    text(a, "path")?.ok_or_else(|| missing("path"))
}

/// A memory type: one of the four exact values.
fn check_type(t: &str) -> Result<&str> {
    if TYPES.contains(&t) {
        Ok(t)
    } else {
        err(format!("type must be one of {}", TYPES.join(", ")))
    }
}

fn is_core(name: &str) -> bool {
    CORE.iter().any(|c| name.eq_ignore_ascii_case(c))
}

fn valid_segment(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 80
        && b[0].is_ascii_alphanumeric()
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// The `/`-separated segments of `path`, each checked. Nothing is trimmed:
/// a path that does not match exactly is an error.
fn segments(path: &str) -> Result<Vec<String>> {
    let parts: Vec<String> = path.split('/').map(str::to_string).collect();
    if parts.iter().all(|p| valid_segment(p)) {
        Ok(parts)
    } else {
        err(format!(
            "invalid path {path:?}: each part needs 1 to 80 letters, digits, '.', '_' or '-', starting with a letter or digit"
        ))
    }
}

/// A memory address: `project/memory` or `project/subproject/memory`.
fn memory_path(path: &str) -> Result<Vec<String>> {
    let segs = segments(path)?;
    if (2..=3).contains(&segs.len()) {
        Ok(segs)
    } else {
        err(format!(
            "{path:?} is not a memory path: use project/memory or project/subproject/memory"
        ))
    }
}

/// A scope: `project` or `project/subproject`.
fn scope_of(path: &str) -> Result<Vec<String>> {
    let segs = segments(path)?;
    if segs.len() <= 2 {
        Ok(segs)
    } else {
        err(format!(
            "{path:?} is too deep: use project or project/subproject"
        ))
    }
}

/// The steps from the top level down: `Memory`, then the path segments.
fn steps_of(segs: &[String]) -> Vec<&str> {
    std::iter::once(MEMORY)
        .chain(segs.iter().map(String::as_str))
        .collect()
}

/// The path of step `i`, for messages (`Memory` for the top level).
fn label_of(steps: &[&str], i: usize) -> String {
    if i == 0 {
        MEMORY.to_string()
    } else {
        steps[1..=i].join("/")
    }
}

/// Why step `depth` cannot hold memories, for messages.
fn not_container(depth: usize, label: &str) -> String {
    match depth {
        0 => format!("{MEMORY:?} is a page, not a database"),
        1 => format!("{label:?} is a memory, not a project"),
        _ => format!("{label:?} is a memory, not a subproject"),
    }
}

// ------------------------------------------------------------ pages

/// Children of `parent` (top level for `None`), all pages, not in the trash.
fn kids(v: &Vault, parent: Option<&str>) -> Result<Vec<PageSummary>> {
    const BATCH: u32 = 200;
    let mut out = Vec::new();
    loop {
        let batch = v.list_children(parent.map(str::to_string), out.len() as u32, BATCH)?;
        let done = batch.len() < BATCH as usize;
        out.extend(batch);
        if done {
            return Ok(out);
        }
    }
}

/// The error for pages that share a title under `label`, named by kind.
fn ambiguous(label: &str, title: &str, same: &[&PageSummary]) -> ToolError {
    let conflicts: Vec<String> = same
        .iter()
        .map(|p| {
            let kind = if p.is_database() {
                "database"
            } else {
                "memory"
            };
            format!("{:?} ({kind}, created {} ms)", p.title, p.created_ms)
        })
        .collect();
    ToolError(format!(
        "ambiguous path {label}: {} pages are titled {title:?}: {}; rename one in the app",
        same.len(),
        conflicts.join("; ")
    ))
}

/// The one child titled exactly `title`, if any. Two such pages are an
/// error that names both by kind, never by id.
fn sibling(
    v: &Vault,
    parent: Option<&str>,
    title: &str,
    label: &str,
) -> Result<Option<PageSummary>> {
    let mut same: Vec<PageSummary> = kids(v, parent)?
        .into_iter()
        .filter(|p| p.title == title)
        .collect();
    if same.len() > 1 {
        let refs: Vec<&PageSummary> = same.iter().collect();
        return Err(ambiguous(label, title, &refs));
    }
    Ok(same.pop())
}

/// Every child of `parent` (top level for `None`), for listing. Two children
/// with one title would give indistinguishable paths, so that is an error.
fn children(v: &Vault, parent: Option<&str>, label: &str) -> Result<Vec<PageSummary>> {
    let all = kids(v, parent)?;
    let dup = {
        let mut by_title: BTreeMap<&str, Vec<&PageSummary>> = BTreeMap::new();
        for p in &all {
            by_title.entry(p.title.as_str()).or_default().push(p);
        }
        by_title
            .into_iter()
            .find(|(_, same)| same.len() > 1)
            .map(|(title, same)| ambiguous(label, title, &same))
    };
    match dup {
        Some(error) => Err(error),
        None => Ok(all),
    }
}

/// The page at `steps` (from `Memory` down). Every step must be a database,
/// except the last one of a lookup (`create` false). With `create`, missing
/// databases on the way are made, so every step is a container and each
/// existing one is checked. Once a step is missing, all later ones are too,
/// so an error never comes after something was created.
fn resolve(v: &Vault, steps: &[&str], create: bool) -> Result<Option<PageId>> {
    let mut parent: Option<PageId> = None;
    for (i, &title) in steps.iter().enumerate() {
        let last = i + 1 == steps.len();
        let label = label_of(steps, i);
        let id = match sibling(v, parent.as_deref(), title, &label)? {
            Some(p) => {
                if (create || !last) && !p.is_database() {
                    return err(not_container(i, &label));
                }
                p.id
            }
            None if create => {
                let id = v.create_page(parent.clone(), title.to_string())?;
                let schema = if i == 0 { Vec::new() } else { memory_schema() };
                v.set_schema(id.clone(), schema)?;
                id
            }
            None => return Ok(None),
        };
        parent = Some(id);
    }
    Ok(parent)
}

/// The top-level `Memory` database, if it exists.
fn memory_root(v: &Vault) -> Result<Option<PageSummary>> {
    match sibling(v, None, MEMORY, MEMORY)? {
        Some(p) if !p.is_database() => err(not_container(0, MEMORY)),
        root => Ok(root),
    }
}

fn project_of(v: &Vault, root: &PageSummary, name: &str) -> Result<PageSummary> {
    match sibling(v, Some(root.id.as_str()), name, name)? {
        Some(p) if p.is_database() => Ok(p),
        Some(_) => err(not_container(1, name)),
        None => err(format!("no memory project {name:?}")),
    }
}

fn sub_of(v: &Vault, project: &PageSummary, sub: &str) -> Result<PageSummary> {
    let label = format!("{}/{sub}", project.title);
    match sibling(v, Some(project.id.as_str()), sub, &label)? {
        Some(p) if p.is_database() => Ok(p),
        Some(_) => err(not_container(2, &label)),
        None => err(format!("no memory subproject {label:?}")),
    }
}

/// The memory at `path`. Errors when it is missing or is a database.
fn memory_at(v: &Vault, path: &str) -> Result<Page> {
    let segs = memory_path(path)?;
    let steps = steps_of(&segs);
    let Some(id) = resolve(v, &steps, false)? else {
        return err(format!("no memory {path:?}"));
    };
    let page = v.load_page(id)?;
    if page.is_database() {
        return err(format!("{path:?} is a project or subproject, not a memory"));
    }
    Ok(page)
}

fn text_field(name: &str) -> PropDef {
    PropDef {
        key: String::new(),
        name: name.to_string(),
        kind: PropKind::Text,
        options: Vec::new(),
        config: String::new(),
    }
}

/// The schema of projects and subprojects: name, description, type, source.
fn memory_schema() -> Vec<PropDef> {
    vec![
        text_field("name"),
        text_field("description"),
        PropDef {
            key: String::new(),
            name: "type".into(),
            kind: PropKind::Select,
            options: TYPES
                .iter()
                .map(|t| SelectOption {
                    id: String::new(),
                    name: t.to_string(),
                    color: "default".into(),
                })
                .collect(),
            config: String::new(),
        },
        text_field("source"),
    ]
}

/// A schema field of a memory's database, by name (case-insensitive).
fn field_def<'a>(schema: &'a [PropDef], name: &str) -> Result<&'a PropDef> {
    if name.eq_ignore_ascii_case("title") {
        return err("the title is the final path segment; name changes the name field");
    }
    schema
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let names: Vec<&str> = schema.iter().map(|d| d.name.as_str()).collect();
            ToolError(format!("no field {name:?}; fields: {}", names.join(", ")))
        })
}

// ------------------------------------------------------------ listing and search

struct Entry {
    path: String,
    id: PageId,
    description: String,
    kind: String,
}

/// A property's display value by schema name, empty when unset.
fn field(v: &Vault, schema: &[PropDef], values: &[Property], name: &str) -> String {
    let Some(def) = schema.iter().find(|d| d.name.eq_ignore_ascii_case(name)) else {
        return String::new();
    };
    values
        .iter()
        .find(|p| p.key == def.key)
        .map(|p| props::display(v, &p.value, Some(def)))
        .unwrap_or_default()
}

/// The memory rows of `db`, under `prefix` (the path of `db`).
fn leaves(v: &Vault, db: &PageSummary, prefix: &str, out: &mut Vec<Entry>) -> Result<()> {
    let schema = v.get_schema(db.id.clone())?;
    for row in children(v, Some(db.id.as_str()), prefix)?
        .into_iter()
        .filter(|r| !r.is_database())
    {
        out.push(Entry {
            path: format!("{prefix}/{}", row.title),
            description: field(v, &schema, &row.properties, "description"),
            kind: field(v, &schema, &row.properties, "type"),
            id: row.id,
        });
    }
    Ok(())
}

fn databases(pages: Vec<PageSummary>) -> Vec<PageSummary> {
    pages.into_iter().filter(PageSummary::is_database).collect()
}

/// Every memory in `scope` (empty: all; `project`: with its subprojects;
/// `project/subproject`: only that one), in no particular order. Any
/// container it enumerates must have unique child titles.
fn collect(v: &Vault, scope: &[String]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let Some(root) = memory_root(v)? else {
        return match scope.first() {
            Some(name) => err(format!("no memory project {name:?}")),
            None => Ok(out),
        };
    };
    let projects: Vec<PageSummary> = match scope.first() {
        Some(name) => vec![project_of(v, &root, name)?],
        None => databases(children(v, Some(root.id.as_str()), MEMORY)?),
    };
    for project in &projects {
        match scope.get(1) {
            Some(name) => {
                let sub = sub_of(v, project, name)?;
                let prefix = format!("{}/{}", project.title, sub.title);
                leaves(v, &sub, &prefix, &mut out)?;
            }
            None => {
                leaves(v, project, &project.title, &mut out)?;
                let subs = databases(children(v, Some(project.id.as_str()), &project.title)?);
                for sub in &subs {
                    leaves(
                        v,
                        sub,
                        &format!("{}/{}", project.title, sub.title),
                        &mut out,
                    )?;
                }
            }
        }
    }
    Ok(out)
}

/// Text on one line: runs of whitespace (line breaks included) become one space.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `- <path> — <description> (<type>)`, always on one line.
fn line(e: &Entry) -> String {
    let description = one_line(&e.description);
    let description = if description.is_empty() {
        "(no description)".to_string()
    } else {
        description
    };
    let kind = one_line(&e.kind);
    let kind = if kind.is_empty() {
        "no type".to_string()
    } else {
        kind
    };
    format!("- {} — {description} ({kind})", e.path)
}

fn lines(entries: &mut [Entry]) -> String {
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries.iter().map(line).collect::<Vec<_>>().join("\n")
}

fn list(v: &Vault, a: &Value) -> Result<String> {
    match text(a, "path")? {
        None => list_projects(v),
        Some(path) => {
            let scope = scope_of(path)?;
            let mut entries = collect(v, &scope)?;
            if entries.is_empty() {
                return Ok(format!("No memories in {path:?}."));
            }
            Ok(lines(&mut entries))
        }
    }
}

fn list_projects(v: &Vault) -> Result<String> {
    let Some(root) = memory_root(v)? else {
        return Ok("No memory projects yet.".into());
    };
    let mut projects = databases(children(v, Some(root.id.as_str()), MEMORY)?);
    projects.sort_by(|a, b| a.title.cmp(&b.title));
    if projects.is_empty() {
        return Ok("No memory projects yet.".into());
    }
    let mut out = Vec::new();
    for p in &projects {
        let n = collect(v, std::slice::from_ref(&p.title))?.len();
        let noun = if n == 1 { "memory" } else { "memories" };
        out.push(format!("- {} — {n} {noun}", p.title));
    }
    Ok(out.join("\n"))
}

/// A search snippet on one line, shortened.
fn snippet(s: &str) -> String {
    const MAX: usize = 160;
    let flat = one_line(s);
    if flat.chars().count() <= MAX {
        return flat;
    }
    let mut short: String = flat.chars().take(MAX).collect();
    short.push('…');
    short
}

/// The page of a memory scope (`project` or `project/subproject`), for the
/// `memory_search` alias of search_notes.
pub(crate) fn scope_container(v: &Vault, scope: &str) -> Result<PageId> {
    let segs = scope_of(scope)?;
    resolve(v, &steps_of(&segs), false)?
        .ok_or_else(|| ToolError(format!("no memory scope {scope:?}")))
}

/// Every memory in the scope whose text matches the query, in rank order.
/// There is no result cap: the scope is applied to every hit before anything
/// is kept, so unrelated hits cannot push a memory out.
fn search(v: &Vault, a: &Value) -> Result<String> {
    // An empty or absent query lists, as memory_list did: projects for no
    // scope (or all), else the memories in that scope.
    let query = text(a, "query")?.unwrap_or_default();
    if query.trim().is_empty() {
        return match text(a, "scope")? {
            None | Some("all") => list(v, &json!({})),
            Some(p) => list(v, &json!({ "path": p })),
        };
    }
    let scope = match text(a, "scope")? {
        None | Some("all") => Vec::new(),
        Some(p) => scope_of(p)?,
    };
    let entries = collect(v, &scope)?;
    if entries.is_empty() {
        return Ok(format!("No memories match {query:?}."));
    }
    // Content edits reach the index through the background writer.
    v.flush()?;
    let container = resolve(v, &steps_of(&scope), false)?
        .ok_or_else(|| ToolError("memory search scope disappeared".into()))?;
    // Compose parsed queries so even incomplete user syntax cannot swallow
    // the internal subtree filter. Canonical operators retain their page ids.
    let mut spec = v.parse_search(query.to_string())?.spec;
    spec.operators.extend(
        v.parse_search(format!("in:[[{container}]]"))?
            .spec
            .operators,
    );
    let hits = v.search_full(spec.to_query_string(), u32::MAX)?;
    let by_id: HashMap<&str, &Entry> = entries.iter().map(|e| (e.id.as_str(), e)).collect();
    let found: Vec<(&Entry, &SearchHit)> = hits
        .iter()
        // External rows share the id space with pages, so only vault hits can be memories.
        .filter(|h| h.source == VAULT_SOURCE)
        .filter_map(|h| by_id.get(h.id.as_str()).map(|e| (*e, h)))
        .collect();
    if found.is_empty() {
        return Ok(format!("No memories match {query:?}."));
    }
    let mut out = String::new();
    for (e, h) in &found {
        out.push_str(&line(e));
        out.push('\n');
        let s = snippet(&h.snippet);
        if !s.is_empty() {
            out.push_str(&format!("  {s}\n"));
        }
    }
    Ok(out.trim_end().to_string())
}

// ------------------------------------------------------------ read, write, update, delete

fn read(v: &Vault, a: &Value) -> Result<String> {
    let path = path_arg(a)?;
    let page = memory_at(v, path)?;
    let schema = match page.parent.as_deref() {
        Some(db) => v.get_schema(db.to_string())?,
        None => Vec::new(),
    };
    let mut out = format!("# {path}\n");
    for def in &schema {
        let value = page
            .properties
            .iter()
            .find(|p| p.key == def.key)
            .map(|p| props::display(v, &p.value, Some(def)))
            .unwrap_or_default();
        if is_core(&def.name) || !value.is_empty() {
            out.push_str(&format!("- {}: {value}\n", def.name));
        }
    }
    out.push_str(&format!(
        "- modified: {}\n",
        props::iso(page.updated_ms, true)
    ));
    out.push_str("\n---\n");
    if page.blocks.is_empty() {
        out.push_str("(empty)");
    } else {
        out.push_str(&v.blocks_markdown(page.blocks));
    }
    Ok(out.trim_end().to_string())
}

/// Creates or replaces the memory at `path`. A replaced row gets exactly the
/// fields the write sets: `source` and every extra field are cleared (null).
/// Other rows and the schema are untouched.
fn write(v: &Vault, a: &Value) -> Result<String> {
    let path = path_arg(a)?;
    let segs = memory_path(path)?;
    let description = text(a, "description")?.ok_or_else(|| missing("description"))?;
    let kind = check_type(text(a, "type")?.ok_or_else(|| missing("type"))?)?;
    let body = text(a, "body")?.ok_or_else(|| missing("body"))?;
    let leaf = segs.last().map(String::as_str).unwrap_or_default();
    let name = text(a, "name")?.unwrap_or(leaf);

    let steps = steps_of(&segs);
    let containers = &steps[..steps.len() - 1];
    let db = resolve(v, containers, true)?
        .ok_or_else(|| ToolError("could not create the memory's database".into()))?;
    let mut set = Map::new();
    set.insert("name".into(), json!(name));
    set.insert("description".into(), json!(description));
    set.insert("type".into(), json!(kind));
    match sibling(v, Some(db.as_str()), leaf, path)? {
        Some(p) if p.is_database() => {
            err(format!("{path:?} is a project or subproject, not a memory"))
        }
        Some(p) => {
            let schema = v.get_schema(db.clone())?;
            let mut changes: Map<String, Value> = schema
                .iter()
                .filter(|d| !is_core(&d.name))
                .map(|d| (d.name.clone(), Value::Null))
                .collect();
            changes.extend(set);
            props::set_props(v, &p.id, Some(&db), &Value::Object(changes))?;
            v.replace_markdown(p.id, Vec::new(), body.to_string())?;
            Ok(format!("Updated memory **{path}**."))
        }
        None => {
            let id = v.create_page(Some(db.clone()), leaf.to_string())?;
            props::set_props(v, &id, Some(&db), &Value::Object(set))?;
            if !body.trim().is_empty() {
                v.insert_markdown(id, None, None, true, body.to_string())?;
            }
            Ok(format!("Created memory **{path}**."))
        }
    }
}

/// Merges the supplied fields into an existing memory; unspecified fields keep
/// their values. `body` replaces the text, `append` adds text at the end.
fn update(v: &Vault, a: &Value) -> Result<String> {
    let path = path_arg(a)?;
    let page = memory_at(v, path)?;
    let db = page
        .parent
        .clone()
        .ok_or_else(|| ToolError(format!("{path:?} is not in a project")))?;
    let schema = v.get_schema(db.clone())?;

    let mut changes = Map::new();
    if let Some(d) = text(a, "description")? {
        changes.insert("description".into(), json!(d));
    }
    if let Some(t) = text(a, "type")? {
        changes.insert("type".into(), json!(check_type(t)?));
    }
    if let Some(n) = text(a, "name")? {
        changes.insert("name".into(), json!(n));
    }
    if let Some(fields) = a.get("fields") {
        let fields = fields
            .as_object()
            .ok_or_else(|| ToolError("fields must be an object of name: value".into()))?;
        for (name, value) in fields {
            let def = field_def(&schema, name)?;
            if def.name.eq_ignore_ascii_case("type") {
                check_type(value.as_str().unwrap_or_default())?;
            }
            changes.insert(def.name.clone(), value.clone());
        }
    }
    let body = text(a, "body")?;
    let append = text(a, "append")?.filter(|x| !x.trim().is_empty());
    let new_path = text(a, "new_path")?;
    if changes.is_empty() && body.is_none() && append.is_none() && new_path.is_none() {
        return err(
            "nothing to change: give description, type, name, fields, body, append or new_path",
        );
    }

    let mut done: Vec<String> = changes.keys().cloned().collect();
    if let Some(new_path) = new_path {
        // Resolve collisions and every property/option mapping before any
        // mutation. The core move keeps its ordinary tree-only semantics.
        let plan = plan_move(v, &page, &schema, new_path, &changes)?;
        if page.parent.as_ref() != Some(&plan.parent) {
            v.move_page(page.id.clone(), Some(plan.parent), None)?;
        }
        if page.title != plan.title {
            v.rename_page(page.id.clone(), plan.title)?;
        }
        for (key, value) in &plan.properties {
            v.set_property(page.id.clone(), key.clone(), value.clone())?;
        }
        for property in &page.properties {
            if !plan.properties.iter().any(|(key, _)| *key == property.key) {
                v.remove_property(page.id.clone(), property.key.clone())?;
            }
        }
        done.push("new_path".into());
    } else if !changes.is_empty() {
        props::set_props(v, &page.id, Some(&db), &Value::Object(changes))?;
    }
    if let Some(b) = body {
        v.replace_markdown(page.id.clone(), Vec::new(), b.to_string())?;
        done.push("body".into());
    }
    if let Some(x) = append {
        v.insert_markdown(page.id.clone(), None, None, true, x.to_string())?;
        done.push("append".into());
    }
    Ok(format!(
        "Updated memory **{}**: {}",
        new_path.unwrap_or(path),
        done.join(", ")
    ))
}

struct MemoryMove {
    parent: PageId,
    title: String,
    properties: Vec<(String, PropValue)>,
}

/// Plans a path move using only existing destination schema definitions. A
/// populated source field that cannot be represented there refuses the move,
/// rather than silently dropping data or creating schema fields/options.
fn plan_move(
    v: &Vault,
    page: &Page,
    source_schema: &[PropDef],
    new_path: &str,
    changes: &Map<String, Value>,
) -> Result<MemoryMove> {
    let parts = memory_path(new_path)?;
    let title = parts.last().cloned().unwrap_or_default();
    let scope = &parts[..parts.len() - 1];
    let parent = resolve(v, &steps_of(scope), false)?
        .ok_or_else(|| ToolError("destination memory project/subproject does not exist".into()))?;
    if !v.load_page(parent.clone())?.is_database() {
        return err("destination memory project/subproject is not a database");
    }
    if sibling(v, Some(&parent), &title, new_path)?.is_some_and(|p| p.id != page.id) {
        return err("destination memory path is already occupied");
    }
    let destination = v.get_schema(parent.clone())?;
    let mut properties = BTreeMap::new();
    for property in &page.properties {
        let source = source_schema
            .iter()
            .find(|d| d.key == property.key)
            .ok_or_else(|| ToolError("memory has a property outside its source schema".into()))?;
        // Explicit updates replace old values, so only their replacement
        // needs to be representable in the destination.
        if changes
            .keys()
            .any(|name| name.eq_ignore_ascii_case(&source.name))
        {
            continue;
        }
        let target = field_def(&destination, &source.name)?;
        if source.kind != target.kind {
            return err(format!(
                "destination field {:?} has an incompatible type",
                source.name
            ));
        }
        let value = remap_value(&property.value, source, target)?;
        properties.insert(target.key.clone(), value);
    }
    for (name, value) in changes {
        let mut target = field_def(&destination, name)?.clone();
        let source = field_def(source_schema, name)?;
        if source.kind != target.kind {
            return err(format!(
                "destination field {name:?} has an incompatible type"
            ));
        }
        let mut added = false;
        let converted = props::to_value(v, &mut target, value, &mut added)?;
        if added {
            return err(format!(
                "destination field {name:?} does not have the requested option"
            ));
        }
        properties.insert(target.key, converted);
    }
    Ok(MemoryMove {
        parent,
        title,
        properties: properties.into_iter().collect(),
    })
}

fn remap_value(value: &PropValue, source: &PropDef, target: &PropDef) -> Result<PropValue> {
    let option = |id: &str| -> Result<String> {
        let name = source
            .options
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.name.as_str())
            .ok_or_else(|| {
                ToolError(format!(
                    "source field {:?} has an unknown option",
                    source.name
                ))
            })?;
        target
            .options
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
            .map(|o| o.id.clone())
            .ok_or_else(|| {
                ToolError(format!(
                    "destination field {:?} has no matching option",
                    target.name
                ))
            })
    };
    Ok(match value {
        PropValue::Select(id) => PropValue::Select(option(id)?),
        PropValue::Status(id) => PropValue::Status(option(id)?),
        PropValue::MultiSelect(ids) => {
            PropValue::MultiSelect(ids.iter().map(|id| option(id)).collect::<Result<_>>()?)
        }
        other => other.clone(),
    })
}

fn delete(v: &Vault, a: &Value) -> Result<String> {
    let path = path_arg(a)?;
    let page = memory_at(v, path)?;
    v.delete_page(page.id)?;
    Ok(format!("Moved memory **{path}** to the trash."))
}

#[cfg(test)]
mod tests {
    use vault_core::ExternalSearchRow;

    use super::*;

    struct Temp(std::path::PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open() -> (Temp, Vault) {
        let temp =
            Temp(std::env::temp_dir().join(format!("vault_memory_test_{}", vault_core::new_id())));
        let vault = Vault::open(temp.0.to_string_lossy().into_owned()).unwrap();
        (temp, vault)
    }

    fn save(v: &Vault, path: &str, description: &str, kind: &str, body: &str) -> Result<String> {
        call(
            v,
            "memory_write",
            &json!({"path": path, "description": description, "type": kind, "body": body}),
        )
    }

    fn read_memory(v: &Vault, path: &str) -> String {
        call(v, "memory_read", &json!({ "path": path })).unwrap()
    }

    /// The child of `parent` titled exactly `title`.
    fn child(v: &Vault, parent: Option<&str>, title: &str) -> PageSummary {
        v.list_children(parent.map(str::to_string), 0, 500)
            .unwrap()
            .into_iter()
            .find(|p| p.title == title)
            .unwrap()
    }

    #[test]
    fn moved_memory_retains_import_source_for_rescanning() {
        let (_temp, v) = open();
        save(
            &v,
            "origin/note",
            "Original description",
            "reference",
            "Imported body.",
        )
        .unwrap();
        call(
            &v,
            "memory_update",
            &json!({
                "path":"origin/note", "name":"Visible name",
                "fields":{"source":"origin/note"}
            }),
        )
        .unwrap();
        save(
            &v,
            "destination/anchor",
            "Anchor",
            "project",
            "Anchor body.",
        )
        .unwrap();
        let root = child(&v, None, MEMORY);
        let origin = child(&v, Some(&root.id), "origin");
        let destination = child(&v, Some(&root.id), "destination");
        let note = child(&v, Some(&origin.id), "note");
        let origin_schema = v.get_schema(origin.id).unwrap();
        let destination_schema = v.get_schema(destination.id.clone()).unwrap();
        assert_ne!(origin_schema[0].key, destination_schema[0].key);
        assert_ne!(
            origin_schema[2].options[0].id,
            destination_schema[2].options[0].id
        );

        call(
            &v,
            "memory_update",
            &json!({
                "path":"origin/note", "new_path":"destination/note"
            }),
        )
        .unwrap();

        let moved = child(&v, Some(&destination.id), "note");
        assert_eq!(moved.id, note.id);
        assert_eq!(
            v.get_schema(destination.id).unwrap(),
            destination_schema,
            "a move must not create or alter schema fields or options"
        );
        let read = read_memory(&v, "destination/note");
        for expected in [
            "- name: Visible name",
            "- description: Original description",
            "- type: reference",
            "- source: origin/note",
            "Imported body.",
        ] {
            assert!(
                read.contains(expected),
                "missing {expected:?} after move: {read}"
            );
        }
        let listing = call(&v, "memory_list", &json!({"path":"destination"})).unwrap();
        assert!(
            listing.contains("destination/note — Original description (reference)"),
            "{listing}"
        );
        assert!(!call(&v, "memory_list", &json!({"path":"origin"}))
            .unwrap()
            .contains("origin/note"));
        call(
            &v,
            "memory_update",
            &json!({"path":"destination/note", "body":"Reimported body."}),
        )
        .unwrap();
        let reread = read_memory(&v, "destination/note");
        assert!(reread.contains("- source: origin/note"), "{reread}");
        assert!(reread.contains("Reimported body."), "{reread}");
    }

    #[test]
    fn path_moves_merge_updates_and_support_subprojects_and_renaming() {
        let (_temp, v) = open();
        save(&v, "origin/note", "Before", "reference", "Original body.").unwrap();
        call(
            &v,
            "memory_update",
            &json!({"path":"origin/note", "fields":{"source":"origin/note"}}),
        )
        .unwrap();
        save(&v, "destination/sub/anchor", "Anchor", "project", "").unwrap();
        let original = memory_at(&v, "origin/note").unwrap();
        call(&v, "memory_update", &json!({
            "path":"origin/note", "new_path":"destination/sub/renamed",
            "description":"After", "name":"Changed name", "type":"feedback", "append":"Extra paragraph."
        })).unwrap();
        let moved = memory_at(&v, "destination/sub/renamed").unwrap();
        assert_eq!(original.id, moved.id);
        let read = read_memory(&v, "destination/sub/renamed");
        for expected in [
            "- name: Changed name",
            "- description: After",
            "- type: feedback",
            "- source: origin/note",
            "Original body.",
            "Extra paragraph.",
        ] {
            assert!(read.contains(expected), "missing {expected:?}: {read}");
        }
        call(
            &v,
            "memory_update",
            &json!({
                "path":"destination/sub/renamed", "new_path":"destination/sub/final"
            }),
        )
        .unwrap();
        assert_eq!(
            memory_at(&v, "destination/sub/final").unwrap().id,
            original.id
        );
        assert!(memory_at(&v, "destination/sub/renamed").is_err());
    }

    #[test]
    fn path_move_rejections_leave_original_and_schemas_unchanged() {
        let (_temp, v) = open();
        save(
            &v,
            "origin/note",
            "Original",
            "reference",
            "Keep this body.",
        )
        .unwrap();
        save(
            &v,
            "destination/taken",
            "Taken",
            "user",
            "Keep destination.",
        )
        .unwrap();
        let original = memory_at(&v, "origin/note").unwrap();
        let destination = memory_at(&v, "destination/taken").unwrap();
        let parent = destination.parent.unwrap();
        let schema = v.get_schema(parent.clone()).unwrap();
        for new_path in [
            "destination/taken",
            "missing/note",
            "destination/missing/note",
            "destination/bad path",
        ] {
            assert!(
                call(
                    &v,
                    "memory_update",
                    &json!({"path":"origin/note", "new_path":new_path,
                "description":"Must not be applied", "body":"Must not replace"})
                )
                .is_err(),
                "{new_path}"
            );
            assert_eq!(memory_at(&v, "origin/note").unwrap(), original);
            assert_eq!(v.get_schema(parent.clone()).unwrap(), schema);
        }
        // Missing select options also fail before the move or any update.
        let mut incompatible = schema.clone();
        incompatible
            .iter_mut()
            .find(|d| d.name == "type")
            .unwrap()
            .options
            .retain(|o| o.name != "reference");
        v.set_schema(parent.clone(), incompatible.clone()).unwrap();
        assert!(call(
            &v,
            "memory_update",
            &json!({"path":"origin/note", "new_path":"destination/free"})
        )
        .is_err());
        assert_eq!(memory_at(&v, "origin/note").unwrap(), original);
        assert_eq!(v.get_schema(parent).unwrap(), incompatible);
        assert!(memory_at(&v, "destination/free").is_err());
    }

    #[test]
    fn creates_databases_and_round_trips_a_memory() {
        let (_temp, v) = open();
        assert_eq!(
            call(&v, "memory_list", &json!({})).unwrap(),
            "No memory projects yet."
        );
        let out = save(
            &v,
            "notes-app/codex-exec-stdin",
            "Close stdin",
            "feedback",
            "Close stdin before waiting.",
        )
        .unwrap();
        assert_eq!(out, "Created memory **notes-app/codex-exec-stdin**.");
        let memory = child(&v, None, MEMORY);
        assert!(memory.is_database());
        let project = child(&v, Some(memory.id.as_str()), "notes-app");
        assert!(project.is_database());
        assert!(!child(&v, Some(project.id.as_str()), "codex-exec-stdin").is_database());

        let read = read_memory(&v, "notes-app/codex-exec-stdin");
        assert!(read.starts_with("# notes-app/codex-exec-stdin\n"), "{read}");
        assert!(read.contains("- name: codex-exec-stdin"), "{read}");
        assert!(read.contains("- description: Close stdin"), "{read}");
        assert!(read.contains("- type: feedback"), "{read}");
        assert!(read.contains("before waiting"), "{read}");

        save(
            &v,
            "notes-app/codex-exec-stdin",
            "Close stdin",
            "project",
            "Replacement text.",
        )
        .unwrap();
        let read = read_memory(&v, "notes-app/codex-exec-stdin");
        assert!(read.contains("- type: project"), "{read}");
        assert!(read.contains("Replacement text."), "{read}");
        assert!(!read.contains("before waiting"), "{read}");
        assert_eq!(
            v.list_children(Some(project.id.clone()), 0, 50)
                .unwrap()
                .len(),
            1,
            "replacing keeps one memory"
        );
    }

    #[test]
    fn simultaneous_first_writes_share_the_same_containers() {
        let (_temp, v) = open();
        let ready = std::sync::Barrier::new(4);
        std::thread::scope(|threads| {
            for i in 0..4 {
                let vault = &v;
                let ready = &ready;
                threads.spawn(move || {
                    ready.wait();
                    save(
                        vault,
                        &format!("proj/sub/m{i}"),
                        "Concurrent",
                        "project",
                        "Body.",
                    )
                    .unwrap();
                });
            }
        });
        assert_eq!(v.list_children(None, 0, 10).unwrap().len(), 1);
        let root = child(&v, None, MEMORY);
        assert_eq!(v.list_children(Some(root.id), 0, 10).unwrap().len(), 1);
        let listing = call(&v, "memory_list", &json!({"path":"proj"})).unwrap();
        assert_eq!(listing.lines().count(), 4, "{listing}");
        for i in 0..4 {
            assert!(read_memory(&v, &format!("proj/sub/m{i}")).contains("Body."));
        }
    }

    #[test]
    fn nested_write_creates_a_subproject_database() {
        let (_temp, v) = open();
        save(
            &v,
            "notes-app/sync/flush-lock",
            "Lock",
            "project",
            "Flush lock.",
        )
        .unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "notes-app");
        let sub = child(&v, Some(project.id.as_str()), "sync");
        assert!(sub.is_database());
        assert!(!child(&v, Some(sub.id.as_str()), "flush-lock").is_database());
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "notes-app/sync"})).unwrap(),
            "- notes-app/sync/flush-lock — Lock (project)"
        );
        let error = call(&v, "memory_read", &json!({"path": "notes-app/sync"})).unwrap_err();
        assert!(error.0.contains("not a memory"), "{error}");
    }

    #[test]
    fn title_stays_the_path_segment_when_name_differs() {
        let (_temp, v) = open();
        call(
            &v,
            "memory_write",
            &json!({"path": "proj/leaf", "name": "Other Name", "description": "d", "type": "user", "body": ""}),
        )
        .unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        assert!(child(&v, Some(project.id.as_str()), "leaf").title == "leaf");
        assert!(!v
            .list_children(Some(project.id.clone()), 0, 50)
            .unwrap()
            .iter()
            .any(|p| p.title == "Other Name"));
        assert!(read_memory(&v, "proj/leaf").contains("- name: Other Name"));
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "proj"})).unwrap(),
            "- proj/leaf — d (user)"
        );

        call(
            &v,
            "memory_update",
            &json!({"path": "proj/leaf", "name": "Renamed"}),
        )
        .unwrap();
        assert!(read_memory(&v, "proj/leaf").contains("- name: Renamed"));
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "proj"})).unwrap(),
            "- proj/leaf — d (user)"
        );
    }

    #[test]
    fn update_merges_fields_and_appends_paragraphs() {
        let (_temp, v) = open();
        save(&v, "proj/m", "First", "feedback", "Body one.").unwrap();
        call(
            &v,
            "memory_update",
            &json!({"path": "proj/m", "description": "Second"}),
        )
        .unwrap();
        let read = read_memory(&v, "proj/m");
        assert!(read.contains("- description: Second"), "{read}");
        assert!(read.contains("- type: feedback"), "{read}");
        assert!(read.contains("Body one."), "{read}");

        call(
            &v,
            "memory_update",
            &json!({"path": "proj/m", "append": "Body two."}),
        )
        .unwrap();
        let read = read_memory(&v, "proj/m");
        let one = read.find("Body one.").unwrap();
        let two = read.find("Body two.").unwrap();
        assert!(one < two, "{read}");

        call(
            &v,
            "memory_update",
            &json!({"path": "proj/m", "body": "Replaced."}),
        )
        .unwrap();
        let read = read_memory(&v, "proj/m");
        assert!(read.contains("Replaced."), "{read}");
        assert!(!read.contains("Body one."), "{read}");

        let error = call(
            &v,
            "memory_update",
            &json!({"path": "proj/missing", "description": "x"}),
        )
        .unwrap_err();
        assert!(error.0.contains("no memory"), "{error}");
        assert!(call(&v, "memory_update", &json!({"path": "proj/m"})).is_err());
        assert!(call(
            &v,
            "memory_update",
            &json!({"path": "proj/m", "type": "bogus"})
        )
        .is_err());
        assert!(read_memory(&v, "proj/m").contains("- type: feedback"));
    }

    #[test]
    fn write_replaces_source_and_extras_while_update_merges() {
        let (_temp, v) = open();
        save(&v, "proj/a", "A", "user", "").unwrap();
        save(&v, "proj/b", "B", "user", "").unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        let mut schema = v.get_schema(project.id.clone()).unwrap();
        schema.push(text_field("Priority"));
        v.set_schema(project.id.clone(), schema).unwrap();
        call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "fields": {"source": "import:1", "Priority": "high"}}),
        )
        .unwrap();
        call(
            &v,
            "memory_update",
            &json!({"path": "proj/b", "fields": {"Priority": "low"}}),
        )
        .unwrap();

        // Update merges: fields it does not name keep their values.
        call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "description": "A2"}),
        )
        .unwrap();
        let read = read_memory(&v, "proj/a");
        assert!(read.contains("- description: A2"), "{read}");
        assert!(read.contains("- source: import:1"), "{read}");
        assert!(read.contains("- Priority: high"), "{read}");

        // Write replaces: source and extra fields it does not set are cleared.
        save(&v, "proj/a", "A3", "project", "Fresh.").unwrap();
        let read = read_memory(&v, "proj/a");
        assert!(read.contains("- description: A3"), "{read}");
        assert!(read.contains("- type: project"), "{read}");
        assert!(read.contains("Fresh."), "{read}");
        assert!(!read.contains("import:1"), "write clears source: {read}");
        assert!(!read.contains("Priority"), "write clears extras: {read}");
        assert!(
            read_memory(&v, "proj/b").contains("- Priority: low"),
            "other rows keep their values"
        );
        let after: Vec<String> = v
            .get_schema(project.id.clone())
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(after, ["name", "description", "type", "source", "Priority"]);

        assert!(call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "fields": {"type": null}})
        )
        .is_err());
    }

    #[test]
    fn lists_sorted_with_subprojects_included() {
        let (_temp, v) = open();
        save(&v, "proj/b", "Bee", "user", "").unwrap();
        save(&v, "proj/a", "Ay", "reference", "").unwrap();
        save(&v, "proj/sub/c", "Cee", "project", "").unwrap();
        save(&v, "other/z", "Zed", "user", "").unwrap();
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "proj"})).unwrap(),
            "- proj/a — Ay (reference)\n- proj/b — Bee (user)\n- proj/sub/c — Cee (project)"
        );
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "proj/sub"})).unwrap(),
            "- proj/sub/c — Cee (project)"
        );
        assert_eq!(
            call(&v, "memory_list", &json!({})).unwrap(),
            "- other — 1 memory\n- proj — 3 memories"
        );
        assert!(call(&v, "memory_list", &json!({"path": "proj/nope"})).is_err());
        assert!(call(&v, "memory_list", &json!({"path": "nope"})).is_err());
    }

    #[test]
    fn search_is_scoped_to_memories_in_the_scope() {
        let (_temp, v) = open();
        save(&v, "proj/a", "Lantern rule", "user", "A lantern glows.").unwrap();
        save(&v, "proj/sub/c", "Sub lantern", "user", "Lantern in sub.").unwrap();
        save(&v, "other/z", "Other", "user", "Lantern elsewhere.").unwrap();
        let note = v.create_page(None, "Lantern note".into()).unwrap();
        v.insert_markdown(
            note.clone(),
            None,
            None,
            true,
            "Lantern outside the memory tree.".into(),
        )
        .unwrap();
        v.flush().unwrap();

        let all = call(&v, "memory_search", &json!({"query": "lantern"})).unwrap();
        assert!(all.contains("proj/a"), "{all}");
        assert!(all.contains("proj/sub/c"), "{all}");
        assert!(all.contains("other/z"), "{all}");
        assert!(!all.contains("Lantern note"), "{all}");
        assert!(!all.contains(note.as_str()), "{all}");

        let project = call(
            &v,
            "memory_search",
            &json!({"query": "lantern", "scope": "proj"}),
        )
        .unwrap();
        assert!(project.contains("proj/a"), "{project}");
        assert!(project.contains("proj/sub/c"), "{project}");
        assert!(!project.contains("other/z"), "{project}");

        let sub = call(
            &v,
            "memory_search",
            &json!({"query": "lantern", "scope": "proj/sub"}),
        )
        .unwrap();
        assert!(sub.contains("proj/sub/c"), "{sub}");
        assert!(!sub.contains("proj/a"), "{sub}");
        assert!(!sub.contains("other/z"), "{sub}");

        let none = call(&v, "memory_search", &json!({"query": "zzzunmatched"})).unwrap();
        assert_eq!(none, "No memories match \"zzzunmatched\".");
    }

    #[test]
    fn unrelated_hits_cannot_hide_a_scoped_memory() {
        let (_temp, v) = open();
        save(
            &v,
            "proj/quiet",
            "Quiet rule",
            "user",
            "Lantern in the body only.",
        )
        .unwrap();
        // More title hits than any fixed cap would keep, all outside the memory tree.
        for i in 0..1050 {
            v.create_page(None, format!("Lantern pile {i}")).unwrap();
        }
        v.flush().unwrap();
        let out = call(
            &v,
            "memory_search",
            &json!({"query": "lantern", "scope": "proj"}),
        )
        .unwrap();
        assert!(out.starts_with("- proj/quiet — Quiet rule (user)"), "{out}");
    }

    #[test]
    fn external_rows_never_stand_in_for_memories() {
        let (_temp, v) = open();
        save(&v, "proj/quiet", "Quiet rule", "user", "Nothing to see.").unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        let quiet = child(&v, Some(project.id.as_str()), "quiet");
        // The external row's id is the memory's page id; its text matches the query.
        v.upsert_search_row(
            "agent".into(),
            ExternalSearchRow {
                id: quiet.id.clone(),
                title: "Chat".into(),
                body: "lantern in a chat".into(),
                page_type: "chat".into(),
                target: None,
                updated_ms: 0,
            },
        )
        .unwrap();
        v.flush().unwrap();
        assert_eq!(
            call(&v, "memory_search", &json!({"query": "lantern"})).unwrap(),
            "No memories match \"lantern\"."
        );
        assert!(call(&v, "memory_search", &json!({"query": "nothing"}))
            .unwrap()
            .starts_with("- proj/quiet"));
    }

    #[test]
    fn duplicate_titles_are_rejected_without_page_ids() {
        let (_temp, v) = open();
        save(&v, "proj/dup", "One", "user", "").unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        v.create_page(Some(project.id.clone()), "dup".into())
            .unwrap();
        let ids: Vec<String> = v
            .list_children(Some(project.id.clone()), 0, 50)
            .unwrap()
            .into_iter()
            .filter(|p| p.title == "dup")
            .map(|p| p.id)
            .collect();
        assert_eq!(ids.len(), 2);

        let read = call(&v, "memory_read", &json!({"path": "proj/dup"}))
            .unwrap_err()
            .0;
        assert!(read.contains("ambiguous"), "{read}");
        assert!(read.contains("\"dup\""), "{read}");
        for id in &ids {
            assert!(!read.contains(id.as_str()), "{read}");
        }
        let write = save(&v, "proj/dup", "Two", "user", "").unwrap_err().0;
        assert!(write.contains("ambiguous"), "{write}");

        // Listing and search expose no duplicate paths either.
        let list = call(&v, "memory_list", &json!({"path": "proj"}))
            .unwrap_err()
            .0;
        assert!(
            list.contains("ambiguous") && list.contains("\"dup\""),
            "{list}"
        );
        for id in &ids {
            assert!(!list.contains(id.as_str()), "{list}");
        }
        assert!(call(&v, "memory_list", &json!({})).is_err());
        assert!(call(&v, "memory_search", &json!({"query": "dup"})).is_err());
        assert!(call(
            &v,
            "memory_search",
            &json!({"query": "dup", "scope": "proj"})
        )
        .is_err());

        assert_eq!(
            v.list_children(Some(project.id.clone()), 0, 50)
                .unwrap()
                .len(),
            2,
            "a rejected write adds nothing"
        );
    }

    #[test]
    fn memory_and_subproject_with_one_title_are_ambiguous() {
        let (_temp, v) = open();
        save(&v, "proj/x", "X", "user", "").unwrap();
        save(&v, "proj/dup/inner", "I", "user", "").unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        // The app can make a plain page with the subproject's title; write cannot.
        v.create_page(Some(project.id.clone()), "dup".into())
            .unwrap();
        let list = call(&v, "memory_list", &json!({"path": "proj"}))
            .unwrap_err()
            .0;
        assert!(list.contains("\"dup\" (database,"), "{list}");
        assert!(list.contains("\"dup\" (memory,"), "{list}");
        assert!(call(&v, "memory_read", &json!({"path": "proj/x"})).is_ok());
        // Resolving through the ambiguous title fails too.
        let read = call(&v, "memory_read", &json!({"path": "proj/dup/inner"}))
            .unwrap_err()
            .0;
        assert!(read.contains("ambiguous"), "{read}");
    }

    #[test]
    fn invalid_paths_are_rejected_before_anything_is_created() {
        let (_temp, v) = open();
        let long_first = format!("{}/x", "a".repeat(81));
        for path in [
            "",
            "solo",
            "a/b/c/d",
            "bad path/x",
            "proj/.hidden",
            "-x/y",
            "proj//x",
            "proj/x/",
            " proj/x",
            "proj/x ",
            long_first.as_str(),
        ] {
            let error = save(&v, path, "d", "user", "").unwrap_err();
            assert!(error.0.contains("path"), "{path:?}: {error}");
        }
        let error = call(&v, "memory_write", &json!({})).unwrap_err();
        assert!(error.0.contains("missing argument: path"), "{error}");
        assert!(
            v.list_children(None, 0, 10).unwrap().is_empty(),
            "invalid paths create nothing"
        );

        let longest = format!("p/{}", "a".repeat(80));
        save(&v, &longest, "d", "user", "").unwrap();
        assert!(read_memory(&v, &longest).contains("- description: d"));
    }

    #[test]
    fn malformed_paths_and_scopes_are_errors_not_defaults() {
        let (_temp, v) = open();
        save(&v, "proj/a", "A", "user", "").unwrap();
        for args in [
            json!({"path": ""}),
            json!({"path": " proj"}),
            json!({"path": 5}),
            json!({"path": null}),
        ] {
            assert!(call(&v, "memory_list", &args).is_err(), "{args}");
        }
        for args in [
            json!({"query": "a", "scope": ""}),
            json!({"query": "a", "scope": " proj"}),
            json!({"query": "a", "scope": 5}),
            json!({"query": "a", "scope": null}),
            json!({"query": "a", "scope": "proj/a/b"}),
        ] {
            assert!(call(&v, "memory_search", &args).is_err(), "{args}");
        }
        assert!(call(&v, "memory_read", &json!({"path": " proj/a"})).is_err());
        assert!(call(
            &v,
            "memory_write",
            &json!({"path": "proj/a ", "description": "d", "type": "user", "body": ""})
        )
        .is_err());
        assert_eq!(
            v.list_children(None, 0, 10).unwrap().len(),
            1,
            "only the Memory database exists"
        );
    }

    #[test]
    fn metadata_text_is_kept_and_listed_on_one_line() {
        let (_temp, v) = open();
        save(&v, "proj/empty", "", "user", "").unwrap();
        save(
            &v,
            "proj/lines",
            "first line\nsecond  line",
            "reference",
            "",
        )
        .unwrap();
        save(&v, "proj/padded", "  padded   text  ", "user", "").unwrap();
        let listing = call(&v, "memory_list", &json!({"path": "proj"})).unwrap();
        assert!(
            listing.contains("- proj/empty — (no description) (user)"),
            "{listing}"
        );
        assert!(
            listing.contains("- proj/lines — first line second line (reference)"),
            "{listing}"
        );
        assert!(
            listing.contains("- proj/padded — padded text (user)"),
            "{listing}"
        );
        assert_eq!(listing.lines().count(), 3, "{listing}");
        assert!(read_memory(&v, "proj/empty").contains("- description:"));

        for kind in [" user", "User", ""] {
            let error = save(&v, "proj/typed", "d", kind, "").unwrap_err();
            assert!(error.0.contains("type must be one of"), "{kind:?}: {error}");
        }
    }

    #[test]
    fn memories_and_databases_are_never_converted() {
        let (_temp, v) = open();
        save(&v, "proj/x", "X", "user", "Body.").unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        let x = child(&v, Some(project.id.as_str()), "x");
        // The last container is a memory: nothing may be created under it.
        let error = save(&v, "proj/x/y", "Y", "user", "").unwrap_err();
        assert!(error.0.contains("not a subproject"), "{error}");
        assert!(
            v.list_children(Some(x.id.clone()), 0, 50)
                .unwrap()
                .is_empty(),
            "a rejected write adds nothing"
        );

        save(&v, "proj/sub/m", "M", "user", "").unwrap();
        let error = save(&v, "proj/sub", "S", "user", "").unwrap_err();
        assert!(error.0.contains("not a memory"), "{error}");
        let error = call(&v, "memory_delete", &json!({"path": "proj/sub"})).unwrap_err();
        assert!(error.0.contains("not a memory"), "{error}");
        assert!(read_memory(&v, "proj/x").contains("Body."));

        // A plain page under Memory is not a project, and nothing is added under it.
        let loose = v
            .create_page(Some(memory.id.clone()), "loose".into())
            .unwrap();
        let error = save(&v, "loose/x", "X", "user", "").unwrap_err();
        assert!(error.0.contains("not a project"), "{error}");
        assert!(v.list_children(Some(loose), 0, 50).unwrap().is_empty());

        let (_temp2, v2) = open();
        v2.create_page(None, MEMORY.into()).unwrap();
        let error = save(&v2, "proj/x", "X", "user", "").unwrap_err();
        assert!(error.0.contains("not a database"), "{error}");
    }

    #[test]
    fn delete_moves_the_memory_to_trash() {
        let (_temp, v) = open();
        save(&v, "proj/gone", "G", "user", "bye").unwrap();
        assert_eq!(
            call(&v, "memory_delete", &json!({"path": "proj/gone"})).unwrap(),
            "Moved memory **proj/gone** to the trash."
        );
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "proj"})).unwrap(),
            "No memories in \"proj\"."
        );
        assert!(call(&v, "memory_read", &json!({"path": "proj/gone"})).is_err());
        assert!(v
            .list_trash()
            .unwrap()
            .iter()
            .any(|t| t.page.title == "gone"));
        assert!(call(&v, "memory_delete", &json!({"path": "proj/gone"})).is_err());
        save(&v, "proj/gone", "G2", "user", "").unwrap();
        assert!(read_memory(&v, "proj/gone").contains("- description: G2"));
    }

    #[test]
    fn extra_schema_fields_are_set_through_fields() {
        let (_temp, v) = open();
        save(&v, "proj/a", "A", "user", "").unwrap();
        let memory = child(&v, None, MEMORY);
        let project = child(&v, Some(memory.id.as_str()), "proj");
        let mut schema = v.get_schema(project.id.clone()).unwrap();
        schema.push(text_field("Priority"));
        v.set_schema(project.id.clone(), schema).unwrap();

        call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "fields": {"Priority": "high", "name": "Alpha"}}),
        )
        .unwrap();
        let read = read_memory(&v, "proj/a");
        assert!(read.contains("- Priority: high"), "{read}");
        assert!(read.contains("- name: Alpha"), "{read}");
        assert!(read.contains("- description: A"), "{read}");
        assert_eq!(
            call(&v, "memory_list", &json!({"path": "proj"})).unwrap(),
            "- proj/a — A (user)"
        );

        let error = call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "fields": {"title": "Nope"}}),
        )
        .unwrap_err();
        assert!(error.0.contains("title"), "{error}");
        let error = call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "fields": {"Missing": "x"}}),
        )
        .unwrap_err();
        assert!(error.0.contains("no field"), "{error}");
        assert!(call(
            &v,
            "memory_update",
            &json!({"path": "proj/a", "fields": {"type": "bogus"}})
        )
        .is_err());

        save(&v, "proj/b", "B", "user", "").unwrap();
        assert!(!read_memory(&v, "proj/b").contains("Priority"));
    }
}
