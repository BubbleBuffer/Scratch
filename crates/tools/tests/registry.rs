//! The merged tool registry: every hidden alias gives its old tool's result,
//! and the new tools' operations (edit_page ops, tasks views, update_task,
//! search_notes modes, memory paths, create_page, import_markdown) behave as
//! specified.

use std::path::PathBuf;

use base64::Engine as _;
use serde_json::{json, Value};
use vault_core::{BlockKind, BlockOp, Mark, MarkKind, NewBlock, PageId, Vault};
use vault_tools::{alias, aliases, call, call_on, canonical, get, hidden, lookup, Surface};

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp() -> (Temp, Vault) {
    let dir =
        Temp(std::env::temp_dir().join(format!("vault_tools_registry_{}", vault_core::new_id())));
    let vault = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
    (dir, vault)
}

/// A successful call's text.
fn ok(v: &Vault, name: &str, args: Value) -> String {
    call(v, name, &args).unwrap_or_else(|e| panic!("{name} {args}: {e}"))
}

/// A failed call's message.
fn fails(v: &Vault, name: &str, args: Value) -> String {
    match call(v, name, &args) {
        Ok(out) => panic!("{name} {args} should fail, gave: {out}"),
        Err(e) => e.0,
    }
}

fn json_of(out: &str) -> Value {
    serde_json::from_str(out).unwrap_or_else(|e| panic!("not JSON ({e}): {out}"))
}

fn new_page(v: &Vault, title: &str) -> PageId {
    v.create_page(None, title.into()).unwrap()
}

/// A to-do dated `date` (ISO), with a fixed block id.
fn todo(v: &Vault, page: &str, id: &str, date: &str) {
    let text = format!("Buy milk @{date}");
    let marks = vec![Mark {
        start: 9,
        end: text.chars().count() as u32,
        kind: MarkKind::Date(date.into()),
    }];
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Todo,
                text,
                marks,
                checked: false,
            },
        }],
    )
    .unwrap();
}

/// Top-level block ids of a page, in order.
fn block_ids(v: &Vault, page: &str) -> Vec<String> {
    v.load_page(page.into())
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.id.clone())
        .collect()
}

fn read_of(v: &Vault, page: &str) -> String {
    ok(v, "read_page", json!({ "page": page }))
}

/// Each word appears in `text`, in the given order.
fn in_order(text: &str, words: &[&str]) {
    let mut last = 0;
    for w in words {
        let at = text
            .find(*w)
            .unwrap_or_else(|| panic!("{w:?} missing from: {text}"));
        assert!(at >= last, "{w:?} out of order in: {text}");
        last = at;
    }
}

/// Item references of a search_notes item result (the `· ref X` ends of its
/// item lines), in result order.
fn item_refs(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.rsplit_once(" · ref ").map(|(_, r)| r.to_string()))
        .collect()
}

/// The item line of `reference` in an item result.
fn item_line(out: &str, reference: &str) -> String {
    out.lines()
        .find(|l| l.ends_with(&format!(" · ref {reference}")))
        .unwrap_or_else(|| panic!("{reference} not in: {out}"))
        .to_string()
}

fn tar_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(body.len() as u64);
        header.set_path(path).unwrap();
        header.set_cksum();
        builder.append(&header, body.as_bytes()).unwrap();
    }
    builder.into_inner().unwrap()
}

/// Import arguments: the project's title under `title_key` (`name` for
/// import_markdown; `title` only to check create_page refuses roots), one root.
fn import_input(title_key: &str, title: &str) -> Value {
    let archive = tar_of(&[("one.md", "# One\n\nAlpha text.\n")]);
    let mut input = json!({
        "repo_path": "/repos/demo",
        "git_remote": "git@example.com:me/demo.git",
        "last_activity": "2026-10-01",
        "roots": [{
            "name": "docs",
            "archive_base64": base64::engine::general_purpose::STANDARD.encode(archive),
        }],
    });
    input
        .as_object_mut()
        .unwrap()
        .insert(title_key.into(), json!(title));
    input
}

// ------------------------------------------------------------ registry

#[test]
fn alias_set_is_the_old_names_and_each_resolves_to_its_target() {
    let pairs: Vec<(&str, &str)> = aliases().iter().map(|a| (a.name, a.target)).collect();
    assert_eq!(
        pairs,
        vec![
            ("list_pages", "search_notes"),
            ("backlinks", "search_notes"),
            ("update_page", "edit_page"),
            ("append_blocks", "edit_page"),
            ("replace_blocks", "edit_page"),
            ("agenda", "search_notes"),
            ("overdue", "search_notes"),
            ("query_database", "search_notes"),
            ("memory_search", "search_notes"),
            ("reschedule", "update_task"),
            ("set_done", "update_task"),
        ]
    );
    for a in aliases() {
        let def = lookup(a.name).unwrap_or_else(|| panic!("{} has no def", a.name));
        assert_eq!(def.name, a.target, "{}", a.name);
        assert!(std::ptr::eq(def, get(a.target).unwrap()), "{}", a.name);
        assert!(std::ptr::eq(alias(a.name).unwrap(), a), "{}", a.name);
        assert!(get(a.name).is_none(), "{} is listed", a.name);
        assert_eq!(canonical(a.name), a.target, "{}", a.name);
    }
}

#[test]
fn lookup_finds_hidden_feedback_list_and_nothing_retired() {
    let hidden_def = lookup("feedback_list").expect("feedback_list is callable");
    let listed_as_hidden = hidden()
        .iter()
        .find(|t| t.name == "feedback_list")
        .expect("feedback_list is hidden");
    assert!(std::ptr::eq(hidden_def, listed_as_hidden));
    assert!(hidden_def.read_only);
    assert!(get("feedback_list").is_none());
    assert!(lookup("feedback_submit").is_some());
    assert!(lookup("search").is_none());
    assert!(lookup("unknown").is_none());
}

#[test]
fn memory_list_is_hidden_not_an_alias_and_tasks_is_retired() {
    // memory_list is callable over MCP for the CLI index, never listed or aliased.
    assert!(get("memory_list").is_none());
    assert!(alias("memory_list").is_none());
    assert!(hidden().iter().any(|t| t.name == "memory_list"));
    assert!(lookup("memory_list").is_some());
    // tasks was removed with no alias: unknown by name, lookup and call.
    assert!(get("tasks").is_none());
    assert!(alias("tasks").is_none());
    assert!(lookup("tasks").is_none());
    assert_eq!(canonical("tasks"), "tasks");
    let (_t, v) = temp();
    let error = fails(&v, "tasks", json!({}));
    assert_eq!(error, "unknown tool: tasks");
}

#[test]
fn old_search_aliases_run_search_notes_with_rewritten_arguments() {
    let (_t, v) = temp();
    let shop = new_page(&v, "Shopping");
    todo(&v, &shop, "milk", "2000-01-01");
    let db = v.create_page(None, "Books".into()).unwrap();
    v.create_page(Some(db.clone()), "Dune".into()).unwrap();
    ok(
        &v,
        "memory_write",
        json!({"path": "proj/lantern", "description": "Lantern note", "type": "user", "body": "Lantern body."}),
    );

    // Each alias gives exactly search_notes' result for its rewritten arguments.
    let typical = [
        ("agenda", json!({"from": "2000-01-01", "to": "2000-01-02"})),
        ("overdue", json!({})),
        ("query_database", json!({"database": db})),
        (
            "memory_search",
            json!({"query": "lantern", "scope": "proj"}),
        ),
    ];
    for (name, args) in &typical {
        let a = alias(name).unwrap_or_else(|| panic!("{name} is not an alias"));
        assert_eq!(a.target, "search_notes", "{name}");
        assert!(get(name).is_none(), "{name} is listed");
        let rewritten = a.rewrite(args).unwrap();
        assert_eq!(
            ok(&v, name, args.clone()),
            ok(&v, "search_notes", rewritten),
            "{name} {args}"
        );
    }

    // And the text is search_notes' rg-style output, not the old JSON or table.
    let agenda = ok(
        &v,
        "agenda",
        json!({"from": "2000-01-01", "to": "2000-01-02"}),
    );
    assert!(agenda.contains("item(s) match"), "{agenda}");
    assert_eq!(item_refs(&agenda), vec![format!("block:{shop}/milk")]);
    assert!(json_of_opt(&agenda).is_none(), "{agenda}");
    let rows = ok(&v, "query_database", json!({"database": db}));
    assert!(rows.contains("Dune"), "{rows}");
    let memory = ok(
        &v,
        "memory_search",
        json!({"query": "lantern", "scope": "proj"}),
    );
    assert!(memory.contains("Lantern"), "{memory}");
}

/// The value when `text` is a JSON document.
fn json_of_opt(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}

#[test]
fn alias_names_are_callable_on_the_in_app_surface() {
    let (_t, v) = temp();
    let p = new_page(&v, "Surface");
    let out = call_on(
        Surface::InAppAgent,
        &v,
        "append_blocks",
        &json!({"page": p, "markdown": "alpha"}),
    )
    .unwrap();
    assert!(out.starts_with("Added 1 block(s): "), "{out}");
    assert!(call_on(Surface::InAppAgent, &v, "feedback_list", &json!({})).is_err());
}

// ------------------------------------------------------------ aliases

#[test]
fn list_pages_is_search_notes_without_a_query() {
    let (_t, v) = temp();
    let parent = new_page(&v, "Parent");
    let kid = v.create_page(Some(parent.clone()), "Kid".into()).unwrap();
    v.create_page(Some(kid), "Grandkid".into()).unwrap();
    new_page(&v, "Other");

    let old = ok(&v, "list_pages", json!({"parent": parent}));
    assert_eq!(old, ok(&v, "search_notes", json!({"parent": parent})));
    assert!(
        old.contains("**Kid**") && !old.contains("Grandkid"),
        "{old}"
    );
    let deep = ok(&v, "list_pages", json!({"parent": parent, "depth": 2}));
    assert!(deep.contains("Grandkid"), "{deep}");
    let top = ok(&v, "list_pages", json!({}));
    assert_eq!(top, ok(&v, "search_notes", json!({})));
    assert!(
        top.contains("**Other**") && top.contains("**Parent**"),
        "{top}"
    );
    assert_eq!(
        ok(&v, "list_pages", json!({"parent": new_page(&v, "Empty")})),
        "No pages inside."
    );
}

#[test]
fn backlinks_is_search_notes_with_backlinks_of() {
    let (_t, v) = temp();
    let target = new_page(&v, "Target");
    let source = new_page(&v, "Source");
    let lonely = new_page(&v, "Lonely");
    v.insert_markdown(
        source.clone(),
        None,
        None,
        true,
        "Points at [[Target]].".into(),
    )
    .unwrap();
    v.flush().unwrap();

    let old = ok(&v, "backlinks", json!({"page": target}));
    assert_eq!(old, ok(&v, "search_notes", json!({"backlinks_of": target})));
    assert!(old.contains(&source) && !old.contains(&lonely), "{old}");
    assert_eq!(
        ok(&v, "backlinks", json!({"page": lonely})),
        "No pages link here."
    );
    assert_eq!(fails(&v, "backlinks", json!({})), "missing argument: page");
}

#[test]
fn append_blocks_is_edit_page_append_with_after_block() {
    let (_t, v) = temp();
    let a = new_page(&v, "A");
    let b = new_page(&v, "B");
    let old = ok(
        &v,
        "append_blocks",
        json!({"page": a, "markdown": "one\n\ntwo"}),
    );
    let new = ok(
        &v,
        "edit_page",
        json!({"page": b, "op": "append", "markdown": "one\n\ntwo"}),
    );
    assert!(old.starts_with("Added 2 block(s): "), "{old}");
    assert!(new.starts_with("Added 2 block(s): "), "{new}");

    ok(
        &v,
        "append_blocks",
        json!({"page": a, "markdown": "charlie"}),
    );
    let first = block_ids(&v, &a)[0].clone();
    ok(
        &v,
        "append_blocks",
        json!({"page": a, "after_block": first, "markdown": "bravo"}),
    );
    let read = read_of(&v, &a);
    in_order(&read, &["one", "bravo", "two", "charlie"]);

    assert_eq!(
        fails(&v, "append_blocks", json!({"markdown": "x"})),
        "missing argument: page"
    );
    assert_eq!(
        fails(&v, "append_blocks", json!({"page": a})),
        "missing argument: markdown"
    );
    assert_eq!(
        fails(
            &v,
            "append_blocks",
            json!({"page": a, "after_block": "nope", "markdown": "x"})
        ),
        "no block nope in the page"
    );
}

#[test]
fn replace_blocks_with_ids_replaces_them_and_without_ids_replaces_the_body() {
    let (_t, v) = temp();
    let p = new_page(&v, "Replace");
    ok(
        &v,
        "append_blocks",
        json!({"page": p, "markdown": "alpha\n\nbravo"}),
    );
    let ids = block_ids(&v, &p);

    let out = ok(
        &v,
        "replace_blocks",
        json!({"page": p, "block_ids": [ids[0]], "markdown": "delta"}),
    );
    assert!(out.starts_with("Replaced 1 block(s) with 1: "), "{out}");
    let read = read_of(&v, &p);
    assert!(read.contains("delta") && read.contains("bravo"), "{read}");
    assert!(!read.contains("alpha"), "{read}");

    let whole = ok(&v, "replace_blocks", json!({"page": p, "markdown": "echo"}));
    assert!(whole.starts_with("Replaced the page body ("), "{whole}");
    let read = read_of(&v, &p);
    assert!(read.contains("echo"), "{read}");
    assert!(!read.contains("bravo") && !read.contains("delta"), "{read}");

    assert_eq!(
        fails(&v, "replace_blocks", json!({"page": p})),
        "missing argument: markdown"
    );
}

#[test]
fn update_page_is_edit_page_set_props() {
    let (_t, v) = temp();
    let p = new_page(&v, "Old title");
    let out = ok(&v, "update_page", json!({"page": p, "title": "New title"}));
    assert!(out.starts_with("Updated **"), "{out}");
    assert_eq!(v.load_page(p.clone()).unwrap().title, "New title");

    let icon = ok(&v, "update_page", json!({"page": p, "icon": "star"}));
    assert!(icon.starts_with("Updated **"), "{icon}");
    assert!(read_of(&v, &p).starts_with("# star New title"));

    assert_eq!(
        fails(&v, "update_page", json!({"page": p})),
        "nothing to change: give title, icon or properties"
    );
    assert_eq!(
        fails(&v, "update_page", json!({"title": "x"})),
        "missing argument: page"
    );
}

#[test]
fn agenda_and_overdue_aliases_list_the_same_items_as_before() {
    let (_t, v) = temp();
    let p = new_page(&v, "Shopping");
    todo(&v, &p, "old", "2000-01-01");
    let item = format!("block:{p}/old");

    let range = json!({"from": "2000-01-01", "to": "2000-01-02"});
    let agenda = ok(&v, "agenda", range.clone());
    assert_eq!(item_refs(&agenda), vec![item.clone()]);
    // The old JSON kind and date are now the text line's facts.
    assert!(
        item_line(&agenda, &item).contains("· todo · due 2000-01-01 · "),
        "{agenda}"
    );

    // `overdue` is the old overdue list: open items dated before today.
    let overdue = ok(&v, "overdue", json!({}));
    assert_eq!(item_refs(&overdue), vec![item.clone()]);

    // Optional one-sided bounds are agenda's (the old tasks alias required both).
    assert_eq!(
        item_refs(&ok(&v, "agenda", json!({"to": "2000-01-02"}))),
        vec![item.clone()]
    );
    assert_eq!(
        item_refs(&ok(&v, "agenda", json!({"from": "2000-01-01"}))),
        vec![item]
    );
}

#[test]
fn reschedule_and_set_done_aliases_are_update_task() {
    let (_t, v) = temp();
    let p = new_page(&v, "Chores");
    todo(&v, &p, "dishes", "2000-01-01");
    let item = format!("block:{p}/dishes");

    let all_day = ok(
        &v,
        "reschedule",
        json!({"item": item, "date": "2030-03-01"}),
    );
    assert_eq!(
        json_of(&all_day),
        json!({"item": item, "date": "2030-03-01", "time": null})
    );
    let timed = ok(
        &v,
        "reschedule",
        json!({"item": item, "date": "2030-03-02", "time": "07:15"}),
    );
    assert_eq!(
        json_of(&timed),
        json!({"item": item, "date": "2030-03-02", "time": "07:15"})
    );
    assert_eq!(
        item_refs(&ok(
            &v,
            "agenda",
            json!({"from": "2030-03-02", "to": "2030-03-03"})
        )),
        vec![item.clone()]
    );
    assert_eq!(
        fails(&v, "reschedule", json!({"item": item})),
        "missing string argument: date"
    );
    assert_eq!(
        fails(&v, "reschedule", json!({"date": "2030-03-02"})),
        "missing string argument: item"
    );

    assert_eq!(
        ok(&v, "set_done", json!({"item": item, "done": true})),
        json!({"item": item, "done": true}).to_string()
    );
    let done = ok(
        &v,
        "agenda",
        json!({"from": "2030-03-02", "to": "2030-03-03"}),
    );
    assert!(item_line(&done, &item).contains("| [x] "), "{done}");
    assert_eq!(
        fails(&v, "set_done", json!({"item": item})),
        "missing boolean argument: done"
    );
    assert_eq!(
        fails(&v, "set_done", json!({"done": true})),
        "missing string argument: item"
    );
}

#[test]
fn memory_list_is_the_compact_index_the_cli_reads() {
    let (_t, v) = temp();
    ok(
        &v,
        "memory_write",
        json!({"path": "proj/alpha", "description": "First", "type": "user", "body": "Body."}),
    );
    ok(
        &v,
        "memory_write",
        json!({"path": "proj/sub/beta", "description": "Second", "type": "project", "body": "Body."}),
    );
    // Projects with no path, then the memories under a path, as "- path" lines.
    assert_eq!(ok(&v, "memory_list", json!({})), "- proj — 2 memories");
    let under = ok(&v, "memory_list", json!({"path": "proj"}));
    assert!(under.starts_with("- proj/alpha — First"), "{under}");
    assert!(under.contains("- proj/sub/beta — Second"), "{under}");
    assert!(call(&v, "memory_list", &json!({"path": "nope"})).is_err());
    // memory_search with only a scope now runs search_notes (the CLI lists with memory_list).
    let scoped = ok(&v, "memory_search", json!({"scope": "proj"}));
    assert_eq!(
        scoped,
        ok(
            &v,
            "search_notes",
            alias("memory_search")
                .unwrap()
                .rewrite(&json!({"scope": "proj"}))
                .unwrap()
        )
    );
}

#[test]
fn import_markdown_creates_the_project_row_and_create_page_refuses_roots() {
    let (_a, va) = temp();
    let imported = json_of(&ok(&va, "import_markdown", import_input("name", "Demo")));
    assert_eq!(imported["row_created"], true);
    let row = imported["project_page"].as_str().unwrap();
    assert!(read_of(&va, row).contains("# Demo"), "{row}");

    // create_page no longer imports: roots fail with the pointer to import_markdown.
    let (_b, vb) = temp();
    let error = fails(&vb, "create_page", import_input("title", "Demo"));
    assert_eq!(error, vault_tools::CREATE_PAGE_ROOTS_ERROR);
    assert!(error.contains("import_markdown"), "{error}");

    // import_markdown without roots still fails as an import.
    let (_c, vc) = temp();
    let error = fails(&vc, "import_markdown", json!({"name": "Demo"}));
    assert!(error.contains("roots"), "{error}");
}

// ------------------------------------------------------------ edit_page ops

#[test]
fn edit_page_append_at_end_and_after_block() {
    let (_t, v) = temp();
    let p = new_page(&v, "Append");
    let out = ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "append", "markdown": "alpha"}),
    );
    assert!(out.starts_with("Added 1 block(s): "), "{out}");
    ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "append", "markdown": "charlie"}),
    );
    let first = block_ids(&v, &p)[0].clone();
    ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "append", "after_block": first, "markdown": "bravo"}),
    );
    in_order(&read_of(&v, &p), &["alpha", "bravo", "charlie"]);
    assert_eq!(
        fails(
            &v,
            "edit_page",
            json!({"page": p, "op": "append", "after_block": "nope", "markdown": "x"})
        ),
        "no block nope in the page"
    );
}

#[test]
fn edit_page_replace_delete_and_set_body() {
    let (_t, v) = temp();
    let p = new_page(&v, "Blocks");
    ok(
        &v,
        "edit_page",
        json!({"page": p, "ops": [
            {"op": "append", "markdown": "alpha"},
            {"op": "append", "markdown": "bravo"},
            {"op": "append", "markdown": "charlie"}
        ]}),
    );
    let ids = block_ids(&v, &p);
    assert_eq!(ids.len(), 3);

    let replaced = ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "replace", "block_ids": [ids[0]], "markdown": "delta"}),
    );
    assert!(
        replaced.starts_with("Replaced 1 block(s) with 1: "),
        "{replaced}"
    );
    let read = read_of(&v, &p);
    assert!(read.contains("delta") && !read.contains("alpha"), "{read}");
    assert_eq!(
        fails(
            &v,
            "edit_page",
            json!({"page": p, "op": "replace", "markdown": "x"})
        ),
        "replace needs block_ids; use set_body for the whole page"
    );

    let deleted = ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "delete", "block_ids": [ids[1]]}),
    );
    assert!(
        deleted.starts_with("Replaced 1 block(s) with 0"),
        "{deleted}"
    );
    let read = read_of(&v, &p);
    assert!(
        !read.contains("bravo") && read.contains("charlie"),
        "{read}"
    );
    assert_eq!(
        fails(&v, "edit_page", json!({"page": p, "op": "delete"})),
        "delete needs block_ids"
    );

    let whole = ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "set_body", "markdown": "echo"}),
    );
    assert!(whole.starts_with("Replaced the page body ("), "{whole}");
    let read = read_of(&v, &p);
    assert!(read.contains("echo"), "{read}");
    assert!(
        !read.contains("charlie") && !read.contains("delta"),
        "{read}"
    );
    assert_eq!(
        fails(&v, "edit_page", json!({"page": p, "op": "set_body"})),
        "missing argument: markdown"
    );
}

#[test]
fn edit_page_set_props_ops_lists_and_argument_errors() {
    let (_t, v) = temp();
    let p = new_page(&v, "Before");
    let out = ok(
        &v,
        "edit_page",
        json!({"page": p, "op": "set_props", "title": "After", "icon": "star"}),
    );
    assert!(out.starts_with("Updated **"), "{out}");
    assert_eq!(v.load_page(p.clone()).unwrap().title, "After");
    assert!(read_of(&v, &p).starts_with("# star After"));

    let many = ok(
        &v,
        "edit_page",
        json!({"page": p, "ops": [
            {"op": "append", "markdown": "alpha"},
            {"op": "append", "markdown": "bravo"}
        ]}),
    );
    assert!(
        many.starts_with("1. Added 1 block(s): ") && many.contains("\n2. Added 1 block(s): "),
        "{many}"
    );
    in_order(&read_of(&v, &p), &["alpha", "bravo"]);

    // A failing op in a list: op 2 is named, op 1 stays applied.
    let failed = fails(
        &v,
        "edit_page",
        json!({"page": p, "ops": [
            {"op": "append", "markdown": "charlie"},
            {"op": "replace", "markdown": "x"}
        ]}),
    );
    assert!(
        failed.starts_with("op 2 failed: replace needs block_ids"),
        "{failed}"
    );
    assert!(failed.contains("ops 1–1 were applied"), "{failed}");
    assert!(read_of(&v, &p).contains("charlie"));

    assert!(
        fails(&v, "edit_page", json!({"page": p, "ops": [{"op": "frob"}]}))
            .starts_with("unknown op \"frob\""),
    );
    assert_eq!(
        fails(
            &v,
            "edit_page",
            json!({"page": p, "op": "append", "ops": [{"op": "append", "markdown": "x"}], "markdown": "y"})
        ),
        "give op or ops, not both"
    );
    assert_eq!(
        fails(&v, "edit_page", json!({"page": p, "ops": []})),
        "ops must be a non-empty list of op objects"
    );
    assert_eq!(
        fails(&v, "edit_page", json!({"page": p})),
        "missing argument: op (one of append, replace, delete, set_body, set_props)"
    );
    assert_eq!(
        fails(&v, "edit_page", json!({"op": "append", "markdown": "x"})),
        "missing argument: page"
    );
}

// ------------------------------------------------------------ tasks views

#[test]
fn agenda_views_today_overdue_range_and_no_view() {
    let (_t, v) = temp();
    let today = chrono::Local::now().date_naive();
    let tomorrow = today.succ_opt().unwrap();
    let (td, tm) = (
        today.format("%Y-%m-%d").to_string(),
        tomorrow.format("%Y-%m-%d").to_string(),
    );
    let p = new_page(&v, "Plans");
    todo(&v, &p, "now", &td);
    todo(&v, &p, "later", &tm);
    todo(&v, &p, "past", "2000-01-01");
    let (now, later, past) = (
        format!("block:{p}/now"),
        format!("block:{p}/later"),
        format!("block:{p}/past"),
    );

    // Without a view agenda lists every dated item (the old tasks default was today).
    let every = item_refs(&ok(&v, "agenda", json!({})));
    assert!(
        every.contains(&now) && every.contains(&later) && every.contains(&past),
        "{every:?}"
    );

    let today_items = item_refs(&ok(&v, "agenda", json!({"view": "today"})));
    assert!(
        today_items.contains(&now) && !today_items.contains(&later) && !today_items.contains(&past),
        "{today_items:?}"
    );
    assert_eq!(
        item_refs(&ok(&v, "agenda", json!({"view": "today", "from": td}))),
        today_items
    );

    let overdue = item_refs(&ok(&v, "agenda", json!({"view": "overdue"})));
    assert!(
        overdue.contains(&past) && !overdue.contains(&now) && !overdue.contains(&later),
        "{overdue:?}"
    );

    let span = item_refs(&ok(&v, "agenda", json!({"from": td, "to": tm})));
    assert!(span.contains(&now) && !span.contains(&later), "{span:?}");

    assert_eq!(
        fails(&v, "agenda", json!({"view": "week"})),
        "view must be today or overdue"
    );
    assert_eq!(
        fails(&v, "agenda", json!({"from": td, "to": td})),
        "to must be after from (to is exclusive)"
    );
    assert_eq!(
        fails(&v, "agenda", json!({"view": "range"})),
        "view must be today or overdue"
    );
}

// ------------------------------------------------------------ update_task

#[test]
fn update_task_due_done_both_and_neither() {
    let (_t, v) = temp();
    let p = new_page(&v, "Errands");
    todo(&v, &p, "milk", "2000-01-01");
    let item = format!("block:{p}/milk");

    let day = ok(
        &v,
        "update_task",
        json!({"item": item, "due": "2030-01-15"}),
    );
    assert_eq!(
        json_of(&day),
        json!({"item": item, "date": "2030-01-15", "time": null})
    );
    assert_eq!(
        item_refs(&ok(
            &v,
            "agenda",
            json!({"from": "2030-01-15", "to": "2030-01-16"})
        )),
        vec![item.clone()]
    );

    let timed = ok(
        &v,
        "update_task",
        json!({"item": item, "due": "2030-01-16 09:30"}),
    );
    assert_eq!(
        json_of(&timed),
        json!({"item": item, "date": "2030-01-16", "time": "09:30"})
    );
    let on_16 = ok(
        &v,
        "agenda",
        json!({"from": "2030-01-16", "to": "2030-01-17"}),
    );
    assert!(
        item_line(&on_16, &item).contains("due 2030-01-16 09:30"),
        "{on_16}"
    );
    assert!(item_refs(&ok(
        &v,
        "agenda",
        json!({"from": "2030-01-15", "to": "2030-01-16"})
    ))
    .is_empty());

    let done = ok(&v, "update_task", json!({"item": item, "done": true}));
    assert_eq!(json_of(&done), json!({"item": item, "done": true}));
    assert!(item_line(&on_16, &item).contains("| [ ] "), "{on_16}");
    assert!(
        item_line(
            &ok(
                &v,
                "agenda",
                json!({"from": "2030-01-16", "to": "2030-01-17"})
            ),
            &item
        )
        .contains("| [x] "),
        "done state follows update_task"
    );

    let both = ok(
        &v,
        "update_task",
        json!({"item": item, "due": "2030-02-01", "done": true}),
    );
    assert_eq!(
        json_of(&both),
        json!({"item": item, "date": "2030-02-01", "time": null, "done": true})
    );
    let feb = ok(
        &v,
        "agenda",
        json!({"from": "2030-02-01", "to": "2030-02-02"}),
    );
    assert!(item_line(&feb, &item).contains("| [x] "), "{feb}");
    assert!(item_refs(&ok(
        &v,
        "agenda",
        json!({"from": "2030-01-16", "to": "2030-01-17"})
    ))
    .is_empty());

    assert_eq!(
        fails(&v, "update_task", json!({"item": item})),
        "give due, done or both"
    );
    assert_eq!(
        fails(&v, "update_task", json!({"item": item, "due": null})),
        "give due, done or both"
    );
    assert_eq!(
        fails(&v, "update_task", json!({"due": "2030-02-01"})),
        "missing string argument: item"
    );
}

// ------------------------------------------------------------ search_notes, read_page

#[test]
fn search_notes_listing_query_and_backlinks_modes() {
    let (_t, v) = temp();
    let target = new_page(&v, "Target");
    let source = new_page(&v, "Source");
    v.insert_markdown(
        source.clone(),
        None,
        None,
        true,
        "Mentions [[Target]] and zebrafish.".into(),
    )
    .unwrap();
    v.flush().unwrap();

    let hits = ok(&v, "search_notes", json!({"query": "zebrafish"}));
    assert!(
        hits.contains("Read as:") && hits.contains(&source),
        "{hits}"
    );

    let listing = ok(&v, "search_notes", json!({}));
    assert!(
        listing.contains("**Target**") && listing.contains("**Source**"),
        "{listing}"
    );
    assert_eq!(ok(&v, "search_notes", json!({"query": ""})), listing);
    assert_eq!(ok(&v, "search_notes", json!({"query": "  "})), listing);

    let back = ok(&v, "search_notes", json!({"backlinks_of": target}));
    assert_eq!(back, ok(&v, "backlinks", json!({"page": target})));
    assert!(back.contains(&source), "{back}");
    assert_eq!(
        fails(
            &v,
            "search_notes",
            json!({"query": "zebrafish", "backlinks_of": target})
        ),
        "give query or backlinks_of, not both"
    );
}

#[test]
fn search_notes_parent_and_depth_list_the_tree() {
    let (_t, v) = temp();
    let parent = new_page(&v, "Tree");
    let kid = v
        .create_page(Some(parent.clone()), "Branch".into())
        .unwrap();
    v.create_page(Some(kid), "Leaf".into()).unwrap();

    let one = ok(&v, "search_notes", json!({"parent": parent}));
    assert!(one.contains("**Branch**") && !one.contains("Leaf"), "{one}");
    let two = ok(&v, "search_notes", json!({"parent": parent, "depth": 2}));
    assert!(two.contains("**Leaf**"), "{two}");
}

#[test]
fn read_page_include_backlinks_appends_the_linking_pages() {
    let (_t, v) = temp();
    let target = new_page(&v, "Linked");
    let source = new_page(&v, "Linker");
    v.insert_markdown(source.clone(), None, None, true, "See [[Linked]].".into())
        .unwrap();
    v.flush().unwrap();

    let with = ok(
        &v,
        "read_page",
        json!({"page": target, "include_backlinks": true}),
    );
    assert!(
        with.contains("---\nBacklinks:\n") && with.contains(&source),
        "{with}"
    );
    assert!(!read_of(&v, &target).contains("Backlinks:"));
}

// ------------------------------------------------------------ memory paths

#[test]
fn memory_tools_take_path_and_refuse_name() {
    let (_t, v) = temp();
    ok(
        &v,
        "memory_write",
        json!({"path": "proj/alpha", "description": "First", "type": "user", "body": "Body."}),
    );
    let message = "missing argument: path (the memory's path, e.g. project/memory or project/subproject/memory)";
    assert_eq!(fails(&v, "memory_read", json!({})), message);
    assert_eq!(fails(&v, "memory_read", json!({"name": "alpha"})), message);
    assert_eq!(
        fails(&v, "memory_update", json!({"name": "Renamed"})),
        message
    );
    assert_eq!(
        fails(&v, "memory_delete", json!({"name": "alpha"})),
        message
    );
    assert!(ok(&v, "memory_read", json!({"path": "proj/alpha"})).contains("First"));
}

// ------------------------------------------------------------ create_page

#[test]
fn create_page_without_roots_creates_a_page_with_its_body() {
    let (_t, v) = temp();
    let out = ok(
        &v,
        "create_page",
        json!({"title": "Plain", "markdown": "alpha body"}),
    );
    assert!(out.starts_with("Created **Plain** `"), "{out}");
    let id = out.split('`').nth(1).unwrap();
    assert!(read_of(&v, id).contains("alpha body"));
}

#[test]
fn import_markdown_runs_the_import_again_reuses_the_row() {
    let (_t, v) = temp();
    let first = json_of(&ok(&v, "import_markdown", import_input("name", "Demo")));
    assert_eq!(first["row_created"], true);
    let again = json_of(&ok(&v, "import_markdown", import_input("name", "Demo")));
    // An identical re-import writes nothing, so the row is neither created nor updated.
    assert_eq!(again["row_created"], false);
    assert_eq!(again["row_updated"], false);
    assert_eq!(again["project_page"], first["project_page"]);
}

#[test]
fn notes_help_is_dispatched_with_a_contents_page() {
    let (_t, v) = temp();
    let contents = ok(&v, "notes_help", json!({}));
    assert!(contents.contains("tasks"), "{contents}");
}

// ------------------------------------------------------------ page links

#[test]
fn page_results_carry_links_the_user_can_click() {
    let (_t, v) = temp();
    let created = ok(
        &v,
        "create_page",
        json!({"title": "Linked", "markdown": "first"}),
    );
    let id = created.split('`').nth(1).unwrap().to_string();
    // Without a hub that set a base, links go to the desktop hub's bounce route.
    let page_link = format!("http://127.0.0.1:47100/open/{id}");
    assert!(
        created.ends_with(&format!("\nlink: {page_link}")),
        "{created}"
    );

    let read = ok(&v, "read_page", json!({"page": id}));
    assert!(read.contains(&format!("link: {page_link}")), "{read}");

    // An append gives the first new block's link, and the page link comes last.
    let added = ok(
        &v,
        "edit_page",
        json!({"page": id, "op": "append", "markdown": "second"}),
    );
    let block = added
        .lines()
        .next()
        .unwrap()
        .rsplit(": ")
        .next()
        .unwrap()
        .to_string();
    assert!(
        added.contains(&format!("\nblock link: {page_link}?block={block}\n")),
        "{added}"
    );
    assert_eq!(
        added.lines().last(),
        Some(format!("link: {page_link}").as_str()),
        "{added}"
    );

    let replaced = ok(
        &v,
        "edit_page",
        json!({"page": id, "op": "replace", "block_ids": [block], "markdown": "third"}),
    );
    let new_block = replaced
        .lines()
        .next()
        .unwrap()
        .rsplit(": ")
        .next()
        .unwrap()
        .to_string();
    assert!(
        replaced.contains(&format!("block link: {page_link}?block={new_block}")),
        "{replaced}"
    );
    assert!(
        replaced.ends_with(&format!("link: {page_link}")),
        "{replaced}"
    );

    // Search rows keep only the ids.
    let found = ok(&v, "search_notes", json!({"query": "Linked"}));
    assert!(!found.contains("notes://"), "{found}");
}
