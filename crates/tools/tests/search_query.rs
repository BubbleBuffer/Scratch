//! search_notes output (rg-style hits, item results, the output budget), the
//! aliases that now rewrite to it, read_page / edit_page / trash_page misses,
//! and move_page.

use std::path::PathBuf;

use chrono::{Duration, Local};
use serde_json::{json, Value};
use vault_core::{
    BlockKind, BlockOp, Mark, MarkKind, NewBlock, PropDef, PropKind, PropValue, Vault,
};
use vault_tools::call;

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp() -> (Temp, Vault) {
    let dir = Temp(
        std::env::temp_dir().join(format!("vault_tools_search_query_{}", vault_core::new_id())),
    );
    let vault = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
    (dir, vault)
}

/// A successful call's text.
fn run(v: &Vault, name: &str, args: Value) -> String {
    call(v, name, &args).unwrap_or_else(|e| panic!("{name} {args}: {e}"))
}

/// A failing call's message; panics when the call succeeds.
fn fails(v: &Vault, name: &str, args: Value) -> String {
    match call(v, name, &args) {
        Ok(text) => panic!("{name} {args} should fail, got: {text}"),
        Err(e) => e.to_string(),
    }
}

fn search(v: &Vault, query: &str) -> String {
    run(v, "search_notes", json!({ "query": query }))
}

fn today() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

fn days_from_today(n: i64) -> String {
    (Local::now().date_naive() + Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

/// Paragraphs in order, each block after the one before it.
fn paragraphs<S: AsRef<str>>(v: &Vault, page: &str, blocks: &[(S, String)]) {
    let mut after: Option<String> = None;
    for (id, text) in blocks {
        let id = id.as_ref().to_string();
        v.apply(
            page.into(),
            vec![BlockOp::Insert {
                parent: None,
                after: after.clone(),
                block: NewBlock {
                    id: id.clone(),
                    kind: BlockKind::Paragraph,
                    text: text.clone(),
                    marks: vec![],
                    checked: false,
                },
            }],
        )
        .unwrap();
        after = Some(id);
    }
}

/// A to-do, optionally dated with a date mark (`Buy milk @DATE`).
fn todo(v: &Vault, page: &str, id: &str, text: &str, date: Option<&str>, checked: bool) {
    let (text, marks) = match date {
        Some(d) => {
            let full = format!("{text} @{d}");
            let marks = vec![Mark {
                start: (text.chars().count() + 2) as u32,
                end: full.chars().count() as u32,
                kind: MarkKind::Date(d.into()),
            }];
            (full, marks)
        }
        None => (text.to_string(), vec![]),
    };
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
                checked,
            },
        }],
    )
    .unwrap();
}

/// A date mention in a paragraph (`Meet @DATE`).
fn mention(v: &Vault, page: &str, id: &str, date: &str) {
    let text = format!("Meet @{date}");
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Paragraph,
                checked: false,
                marks: vec![Mark {
                    start: 6,
                    end: text.chars().count() as u32,
                    kind: MarkKind::Date(date.into()),
                }],
                text,
            },
        }],
    )
    .unwrap();
}

/// Filler of about 200 characters, numbered so each block is distinct.
fn filler(label: &str) -> String {
    format!("{label} {}", "padding ".repeat(24))
}

/// The item references of a result (`... · ref REFERENCE`), in order.
fn refs(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains(" · ref "))
        .map(|l| l.rsplit_once(" · ref ").unwrap().1.to_string())
        .collect()
}

/// The block-line ids of a result (`  \`id\` | text`), in order.
fn block_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("  `"))
        .filter_map(|rest| rest.split_once('`').map(|(id, _)| id.to_string()))
        .collect()
}

fn gap_lines(text: &str) -> usize {
    text.lines().filter(|l| l.trim() == "…").count()
}

/// The titles of a result's page headers (`- **Title** ...`), in order.
fn titles(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("- **"))
        .filter_map(|rest| rest.split_once("**").map(|(t, _)| t.to_string()))
        .collect()
}

/// Titles of a page's children, in order.
fn child_titles(v: &Vault, parent: Option<String>) -> Vec<String> {
    v.list_children(parent, 0, 100)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect()
}

// ------------------------------------------------------------ search output

#[test]
fn rg_output_shows_title_path_match_with_two_blocks_of_context_and_gaps() {
    let (_t, v) = temp();
    let parent = v.create_page(None, "Parent".into()).unwrap();
    let page = v
        .create_page(Some(parent.clone()), "Field notes".into())
        .unwrap();
    let blocks: Vec<(String, String)> = (0..9)
        .map(|i| {
            let text = if i == 4 {
                format!("zephyr sighting at the ridge {}", "padding ".repeat(24))
            } else {
                filler(&format!("Filler {i}"))
            };
            (format!("b{i}"), text)
        })
        .collect();
    paragraphs(&v, &page, &blocks);
    let out = search(&v, "zephyr");
    assert!(
        out.contains(&format!("- **Field notes** `{page}` · in Parent")),
        "{out}"
    );
    assert_eq!(block_ids(&out), vec!["b2", "b3", "b4", "b5", "b6"], "{out}");
    assert_eq!(
        gap_lines(&out),
        2,
        "gaps before and after the window: {out}"
    );
    assert!(out.contains("zephyr sighting"), "{out}");
    assert!(!out.contains("Filler 1"), "{out}");
}

#[test]
fn context_zero_shows_only_the_match_and_short_pages_are_shown_whole() {
    let (_t, v) = temp();
    let page = v.create_page(None, "Short note".into()).unwrap();
    paragraphs(
        &v,
        &page,
        &[
            ("s0", "alpha line".to_string()),
            ("s1", "zephyr middle line".to_string()),
            ("s2", "gamma line".to_string()),
        ],
    );
    // Under 1500 characters: the whole page, whatever the context.
    let whole = search(&v, "zephyr context:0");
    assert_eq!(block_ids(&whole), vec!["s0", "s1", "s2"], "{whole}");
    // full:0 turns the whole-page rule off: only the match with no context.
    let only_match = search(&v, "zephyr context:0 full:0");
    assert_eq!(block_ids(&only_match), vec!["s1"], "{only_match}");
    assert!(!only_match.contains("alpha line"), "{only_match}");
    assert!(!only_match.contains("gamma line"), "{only_match}");
}

#[test]
fn filter_only_query_shows_the_start_of_each_page() {
    let (_t, v) = temp();
    let parent = v.create_page(None, "Parent".into()).unwrap();
    let long = v
        .create_page(Some(parent.clone()), "Long page".into())
        .unwrap();
    let blocks: Vec<(String, String)> = (0..9)
        .map(|i| {
            let text = match i {
                0 => format!("Opening line {}", "padding ".repeat(24)),
                8 => format!("Closing line {}", "padding ".repeat(24)),
                _ => filler(&format!("Middle {i}")),
            };
            (format!("l{i}"), text)
        })
        .collect();
    paragraphs(&v, &long, &blocks);
    let short = v
        .create_page(Some(parent.clone()), "Short page".into())
        .unwrap();
    paragraphs(
        &v,
        &short,
        &[
            ("h0", "short first".to_string()),
            ("h1", "short second".to_string()),
        ],
    );
    let out = search(&v, &format!("in:[[{parent}]]"));
    assert!(out.contains(&format!("`{long}`")), "{out}");
    assert!(out.contains("Opening line"), "{out}");
    assert!(out.contains("… 8 more blocks"), "{out}");
    assert!(!out.contains("Closing line"), "{out}");
    assert!(
        out.contains("short first") && out.contains("short second"),
        "{out}"
    );
}

#[test]
fn block_ids_from_output_are_accepted_by_edit_page() {
    let (_t, v) = temp();
    let parent = v.create_page(None, "Parent".into()).unwrap();
    let page = v.create_page(Some(parent), "Field notes".into()).unwrap();
    let blocks: Vec<(String, String)> = (0..9)
        .map(|i| {
            let text = if i == 4 {
                format!("zephyr sighting {}", "padding ".repeat(24))
            } else {
                filler(&format!("Filler {i}"))
            };
            (format!("b{i}"), text)
        })
        .collect();
    paragraphs(&v, &page, &blocks);
    let out = search(&v, "zephyr");
    let id = block_ids(&out)
        .into_iter()
        .find(|id| id == "b4")
        .expect("b4 is in the output");
    run(
        &v,
        "edit_page",
        json!({ "page": page, "op": "replace", "block_ids": [id], "markdown": "replacement sentence" }),
    );
    let read = run(&v, "read_page", json!({ "page": page }));
    assert!(read.contains("replacement sentence"), "{read}");
    assert!(!read.contains("zephyr"), "{read}");
    assert!(read.contains("Filler 3"), "{read}");
}

#[test]
fn item_output_lists_open_todos_and_update_task_round_trips() {
    let (_t, v) = temp();
    let page = v.create_page(None, "Errands".into()).unwrap();
    todo(&v, &page, "t1", "Buy milk", Some("2026-10-20"), false);
    todo(&v, &page, "t2", "Walk dog", None, false);
    todo(&v, &page, "t3", "Old chore", Some("2026-10-01"), true);
    let open = run(&v, "search_notes", json!({ "query": "is:todo done:no" }));
    let line = open
        .lines()
        .find(|l| l.contains("`t1` | [ ]"))
        .unwrap_or_else(|| panic!("t1 line: {open}"));
    assert!(line.contains(" · todo · "), "{line}");
    assert!(line.contains("due 2026-10-20"), "{line}");
    let reference = format!("block:{page}/t1");
    assert!(line.ends_with(&format!(" · ref {reference}")), "{line}");
    assert!(!open.contains("Old chore"), "{open}");
    run(
        &v,
        "update_task",
        json!({ "item": reference, "done": true }),
    );
    let done = run(&v, "search_notes", json!({ "query": "is:todo done:yes" }));
    assert!(done.lines().any(|l| l.contains("`t1` | [x]")), "{done}");
    assert!(!done.contains("`t2`"), "{done}");
}

#[test]
fn budget_trims_forty_long_hits_and_small_hits_have_no_trimming_note() {
    let (_t, v) = temp();
    for i in 0..40 {
        let page = v.create_page(None, format!("Lantern {i:02}")).unwrap();
        paragraphs(
            &v,
            &page,
            &[("p0", format!("lantern {}", "word ".repeat(400)))],
        );
    }
    let out = search(&v, "lantern limit:40");
    let size = out.chars().count();
    assert!(size <= 12_000 + 300, "output is {size} characters: {out}");
    assert!(out.contains("_Trimmed to fit 12000 characters:"), "{out}");
    assert!(out.contains("offset="), "{out}");
    assert!(
        out.contains("matching blocks of at most 160 characters"),
        "the trimming note names what was trimmed: {out}"
    );
    assert_eq!(titles(&out).len(), 40, "all 40 hits are shown: {out}");

    let (_t2, small) = temp();
    for name in ["Lantern A", "Lantern B", "Lantern C"] {
        let page = small.create_page(None, name.into()).unwrap();
        paragraphs(&small, &page, &[("q0", "lantern small".to_string())]);
    }
    let few = search(&small, "lantern");
    assert!(!few.contains("_Trimmed"), "{few}");
    assert_eq!(titles(&few).len(), 3, "{few}");
}

#[test]
fn limit_above_200_warns_and_is_ignored() {
    let (_t, v) = temp();
    let page = v.create_page(None, "Lantern page".into()).unwrap();
    paragraphs(&v, &page, &[("q0", "lantern here".to_string())]);
    let out = search(&v, "lantern limit:500");
    assert!(out.contains("Warnings:"), "{out}");
    assert!(out.contains("must be between"), "{out}");
    assert!(out.contains("Lantern page"), "{out}");
}

// ------------------------------------------------------------ aliases

#[test]
fn agenda_aliases_return_the_same_items_as_their_search_queries() {
    let (_t, v) = temp();
    let plans = v.create_page(None, "Plans".into()).unwrap();
    let other = v.create_page(None, "Other".into()).unwrap();
    let yesterday = days_from_today(-1);
    todo(&v, &plans, "overdue", "Late task", Some(&yesterday), false);
    todo(&v, &plans, "today", "Today task", Some(&today()), false);
    todo(
        &v,
        &plans,
        "range-in",
        "Range in",
        Some("2026-10-03"),
        false,
    );
    todo(
        &v,
        &plans,
        "range-out",
        "Range out",
        Some("2026-10-08"),
        false,
    );
    todo(
        &v,
        &plans,
        "range-before",
        "Range before",
        Some("2026-09-30"),
        false,
    );
    mention(&v, &plans, "m1", &yesterday);
    todo(&v, &other, "other", "Other task", Some(&yesterday), false);

    // view overdue
    let overdue = run(&v, "agenda", json!({ "view": "overdue" }));
    let expected = run(
        &v,
        "search_notes",
        json!({ "query": "is:todo,task date:<now done:no" }),
    );
    assert_eq!(refs(&overdue), refs(&expected), "{overdue}");
    assert!(
        refs(&overdue).contains(&format!("block:{plans}/overdue")),
        "{overdue}"
    );
    assert!(
        !refs(&overdue).contains(&format!("block:{plans}/today")),
        "{overdue}"
    );

    // from / to: to is exclusive
    let range = run(
        &v,
        "agenda",
        json!({ "from": "2026-10-01", "to": "2026-10-08" }),
    );
    let expected = run(
        &v,
        "search_notes",
        json!({ "query": "is:todo,task,row,mention date:2026-10-01..2026-10-07" }),
    );
    assert_eq!(refs(&range), refs(&expected), "{range}");
    assert_eq!(
        refs(&range),
        vec![format!("block:{plans}/range-in")],
        "{range}"
    );

    // page and open
    let in_page = run(&v, "agenda", json!({ "page": plans, "done": "open" }));
    let expected = run(
        &v,
        "search_notes",
        json!({ "query": format!("in:[[{plans}]] is:todo,task,row,mention done:no") }),
    );
    assert_eq!(refs(&in_page), refs(&expected), "{in_page}");
    assert!(
        !refs(&in_page).iter().any(|r| r.contains(&other)),
        "{in_page}"
    );
    assert!(
        refs(&in_page).contains(&format!("block:{plans}/overdue")),
        "{in_page}"
    );
}

#[test]
fn query_database_filters_sorts_and_starts_with_is_refused() {
    let (_t, v) = temp();
    let db = v.create_page(None, "Work".into()).unwrap();
    v.set_schema(
        db.clone(),
        vec![
            PropDef {
                key: "status".into(),
                name: "Status".into(),
                kind: PropKind::Select,
                options: vec![],
                config: String::new(),
            },
            PropDef {
                key: "priority".into(),
                name: "Priority".into(),
                kind: PropKind::Number,
                options: vec![],
                config: String::new(),
            },
        ],
    )
    .unwrap();
    for (title, status, priority) in [
        ("Write", "Doing", 1),
        ("Plan", "Doing", 3),
        ("Ship", "Done", 2),
        ("Review", "Doing", 2),
    ] {
        run(
            &v,
            "upsert_row",
            json!({ "database": db, "title": title, "properties": { "Status": status, "Priority": priority } }),
        );
    }
    let out = run(
        &v,
        "query_database",
        json!({
            "database": db,
            "filter": [{ "property": "Status", "op": "is", "value": "Doing" }],
            "sort": [{ "property": "Priority", "descending": true }]
        }),
    );
    let order = titles(&out);
    assert_eq!(order, vec!["Plan", "Review", "Write"], "{out}");
    assert!(!out.contains("Ship"), "{out}");

    let refused = fails(
        &v,
        "query_database",
        json!({ "database": db, "filter": [{ "property": "Status", "op": "starts_with", "value": "Do" }] }),
    );
    assert!(refused.contains("use contains"), "{refused}");
}

#[test]
fn memory_search_finds_memories_and_scope_limits_them() {
    let (_t, v) = temp();
    for (path, body) in [
        ("alpha/lantern-plan", "The lantern is lit at dusk."),
        ("beta/lantern-notes", "A lantern in the beta project."),
    ] {
        run(
            &v,
            "memory_write",
            json!({
                "path": path,
                "description": format!("Lantern memory for {path}"),
                "type": "project",
                "body": body,
            }),
        );
    }
    let all = run(&v, "memory_search", json!({ "query": "lantern" }));
    assert!(all.contains("lantern-plan"), "{all}");
    assert!(all.contains("lantern-notes"), "{all}");
    let scoped = run(
        &v,
        "memory_search",
        json!({ "query": "lantern", "scope": "alpha" }),
    );
    assert!(scoped.contains("lantern-plan"), "{scoped}");
    assert!(!scoped.contains("lantern-notes"), "{scoped}");
}

// ------------------------------------------------------------ misses

#[test]
fn read_page_miss_lists_title_word_matches_as_search_results_and_opens_nothing() {
    let (_t, v) = temp();
    let project = v.create_page(None, "Project plan".into()).unwrap();
    let plan_b = v.create_page(None, "Plan B".into()).unwrap();
    paragraphs(&v, &project, &[("x0", "Real content".to_string())]);
    let before = v.load_page(project.clone()).unwrap().blocks.len();

    let miss = fails(&v, "read_page", json!({ "page": "Plan" }));
    assert!(
        miss.contains("Search results for pages whose titles contain those words"),
        "{miss}"
    );
    assert!(miss.contains("not opened"), "{miss}");
    assert!(miss.contains(&format!("`{project}`")), "{miss}");
    assert!(miss.contains("Project plan"), "{miss}");
    assert!(miss.contains(&format!("`{plan_b}`")), "{miss}");
    assert!(miss.contains("Plan B"), "{miss}");
    assert!(!miss.contains("Real content"), "{miss}");
    assert_eq!(v.load_page(project.clone()).unwrap().blocks.len(), before);
}

#[test]
fn read_page_typo_lists_title_matches() {
    let (_t, v) = temp();
    let project = v.create_page(None, "Project plan".into()).unwrap();
    let plan_b = v.create_page(None, "Plan B".into()).unwrap();
    paragraphs(&v, &project, &[("x0", "Real content".to_string())]);
    let before = v.load_page(project.clone()).unwrap().blocks.len();

    let typo = fails(&v, "read_page", json!({ "page": "Projct plan" }));
    assert!(
        typo.contains("Search results for pages whose titles contain those words"),
        "{typo}"
    );
    assert!(typo.contains("not opened"), "{typo}");
    assert!(typo.contains(&format!("`{project}`")), "{typo}");
    assert!(typo.contains("Project plan"), "{typo}");
    assert!(typo.contains(&format!("`{plan_b}`")), "{typo}");
    assert!(typo.contains("Plan B"), "{typo}");
    assert_eq!(v.load_page(project.clone()).unwrap().blocks.len(), before);
}

#[test]
fn read_page_plain_miss_fails_with_the_plain_message() {
    let (_t, v) = temp();
    v.create_page(None, "Project plan".into()).unwrap();
    let plain = fails(&v, "read_page", json!({ "page": "Zzyzx" }));
    assert!(plain.contains("no titles contain those words"), "{plain}");
    assert!(!plain.contains("Search results"), "{plain}");
}

#[test]
fn edit_page_and_trash_page_with_an_unknown_title_still_fail() {
    let (_t, v) = temp();
    let project = v.create_page(None, "Project plan".into()).unwrap();
    paragraphs(&v, &project, &[("x0", "Real content".to_string())]);
    let before = v.load_page(project.clone()).unwrap().blocks.len();

    let edit = fails(
        &v,
        "edit_page",
        json!({ "page": "Projct plan", "op": "append", "markdown": "should not land" }),
    );
    assert!(edit.contains("Projct plan"), "{edit}");
    let trash = fails(&v, "trash_page", json!({ "page": "Projct plan" }));
    assert!(trash.contains("Projct plan"), "{trash}");

    assert_eq!(v.load_page(project.clone()).unwrap().blocks.len(), before);
    assert!(v.load_page(project.clone()).is_ok());
    let all = run(&v, "read_page", json!({ "page": project }));
    assert!(!all.contains("should not land"), "{all}");
}

// ------------------------------------------------------------ move_page

#[test]
fn move_page_places_single_multi_after_and_top_level() {
    let (_t, v) = temp();
    let archive = v.create_page(None, "Archive".into()).unwrap();
    let alpha = v.create_page(None, "Alpha".into()).unwrap();
    v.create_page(None, "Beta".into()).unwrap();
    v.create_page(None, "Gamma".into()).unwrap();
    v.create_page(None, "Delta".into()).unwrap();

    let single = run(
        &v,
        "move_page",
        json!({ "pages": ["Alpha"], "parent": archive }),
    );
    assert!(
        single.contains(&format!("- **Alpha** `{alpha}` → Archive")),
        "{single}"
    );
    assert_eq!(child_titles(&v, Some(archive.clone())), vec!["Alpha"]);

    // Several pages keep the given order; without after they go first.
    run(
        &v,
        "move_page",
        json!({ "pages": ["Gamma", "Beta"], "parent": archive }),
    );
    assert_eq!(
        child_titles(&v, Some(archive.clone())),
        vec!["Gamma", "Beta", "Alpha"]
    );

    // after: right after the sibling.
    run(
        &v,
        "move_page",
        json!({ "pages": ["Delta"], "parent": archive, "after": "Beta" }),
    );
    assert_eq!(
        child_titles(&v, Some(archive.clone())),
        vec!["Gamma", "Beta", "Delta", "Alpha"]
    );

    // parent omitted: the top level.
    let top = run(&v, "move_page", json!({ "pages": ["Alpha"] }));
    assert!(
        top.contains(&format!("- **Alpha** `{alpha}` → top level")),
        "{top}"
    );
    assert!(child_titles(&v, None).contains(&"Alpha".to_string()));
    assert!(!child_titles(&v, Some(archive.clone())).contains(&"Alpha".to_string()));
    assert!(v.load_page(alpha.clone()).unwrap().parent.is_none());
}

#[test]
fn move_page_refuses_moves_into_itself_or_its_child_and_moves_nothing() {
    let (_t, v) = temp();
    let archive = v.create_page(None, "Archive".into()).unwrap();
    let gamma = v
        .create_page(Some(archive.clone()), "Gamma".into())
        .unwrap();
    let notes = v.create_page(None, "Notes".into()).unwrap();

    let itself = fails(
        &v,
        "move_page",
        json!({ "pages": ["Archive"], "parent": archive }),
    );
    assert!(
        itself.contains("cannot move a page into itself"),
        "{itself}"
    );

    let child = fails(
        &v,
        "move_page",
        json!({ "pages": ["Archive"], "parent": gamma }),
    );
    assert!(child.contains("cannot move a page into itself"), "{child}");
    assert!(v.load_page(archive.clone()).unwrap().parent.is_none());

    // Refused as the second of two pages: the first does not move either.
    let second = fails(
        &v,
        "move_page",
        json!({ "pages": ["Notes", "Archive"], "parent": archive }),
    );
    assert!(
        second.contains("cannot move a page into itself"),
        "{second}"
    );
    assert!(v.load_page(notes.clone()).unwrap().parent.is_none());
    assert_eq!(child_titles(&v, Some(archive.clone())), vec!["Gamma"]);
}

#[test]
fn move_page_unknown_title_lists_title_matches() {
    let (_t, v) = temp();
    v.create_page(None, "Project plan".into()).unwrap();
    v.create_page(None, "Plan B".into()).unwrap();
    let err = fails(&v, "move_page", json!({ "pages": ["Plan"] }));
    assert!(err.contains("Project plan"), "{err}");
    assert!(err.contains("Plan B"), "{err}");
}

#[test]
fn move_page_into_a_database_and_out_again_keeps_properties() {
    let (_t, v) = temp();
    let db = v.create_page(None, "Tasks".into()).unwrap();
    v.set_schema(
        db.clone(),
        vec![PropDef {
            key: "notes".into(),
            name: "Notes".into(),
            kind: PropKind::Text,
            options: vec![],
            config: String::new(),
        }],
    )
    .unwrap();
    let archive = v.create_page(None, "Archive".into()).unwrap();
    let row = v.create_page(Some(db.clone()), "Row one".into()).unwrap();
    v.set_property(
        row.clone(),
        "notes".into(),
        PropValue::Text("keep me".into()),
    )
    .unwrap();

    run(
        &v,
        "move_page",
        json!({ "pages": ["Row one"], "parent": archive }),
    );
    assert_eq!(
        v.load_page(row.clone()).unwrap().parent,
        Some(archive.clone())
    );
    run(
        &v,
        "move_page",
        json!({ "pages": ["Row one"], "parent": db }),
    );
    let page = v.load_page(row.clone()).unwrap();
    assert_eq!(page.parent, Some(db.clone()));
    assert!(
        page.properties
            .iter()
            .any(|p| p.key == "notes" && matches!(&p.value, PropValue::Text(t) if t == "keep me")),
        "properties after the round trip: {:?}",
        page.properties
    );
}

// ------------------------------------------------------------ description

#[test]
fn search_notes_description_is_under_2500_bytes() {
    let tool = vault_tools::definitions()
        .iter()
        .find(|t| t.name == "search_notes")
        .expect("search_notes is defined");
    let len = tool.description.len();
    println!("search_notes description: {len} bytes");
    assert!(len < 2_500, "search_notes description is {len} bytes");
}
