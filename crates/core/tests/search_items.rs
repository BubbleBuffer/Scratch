//! Item search through the core: the item operators (`is:`, `done:`,
//! `assignee:`, `calendar:`, `undated`) and how page operators, text and
//! `date:` apply to an item; size controls and sort; old queries and
//! properties named like operators; the app's one-hit-per-page view and the
//! live query of an item search.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use serde_json::{json, Value};
use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_search_items_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }

    // Fixed page timestamps make sort:updated independent of scheduling.
    fn timestamps(&self, id: &str, updated: i64) {
        rusqlite::Connection::open(self.0.join(".vault/index.sqlite"))
            .unwrap()
            .execute(
                "UPDATE pages SET updated_ms = ?1 WHERE id = ?2",
                rusqlite::params![updated, id],
            )
            .unwrap();
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn today() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

/// Midnight UTC of an ISO day, the way all-day dates are stored.
fn day_ms(iso: &str) -> i64 {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d")
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

fn page(v: &Vault, title: &str, parent: Option<&str>) -> PageId {
    v.create_page(parent.map(str::to_string), title.into())
        .unwrap()
}

/// A block whose date marks are the given substrings of `text`, each paired
/// with the date it stores.
fn block(
    v: &Vault,
    page: &str,
    id: &str,
    kind: BlockKind,
    text: &str,
    dates: &[(&str, &str)],
    checked: bool,
) {
    let marks = dates
        .iter()
        .map(|(needle, iso)| {
            let at = text.find(needle).expect("needle in block text");
            let start = text[..at].chars().count() as u32;
            Mark {
                start,
                end: start + needle.chars().count() as u32,
                kind: MarkKind::Date((*iso).into()),
            }
        })
        .collect();
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: id.into(),
                kind,
                text: text.into(),
                marks,
                checked,
            },
        }],
    )
    .unwrap();
}

/// A to-do; `text` must contain `date` when one is given.
fn todo(v: &Vault, page: &str, id: &str, text: &str, date: Option<&str>, checked: bool) {
    let dates: Vec<(&str, &str)> = date.map(|d| vec![(d, d)]).unwrap_or_default();
    block(v, page, id, BlockKind::Todo, text, &dates, checked);
}

/// A dated mention: a paragraph with a date mark.
fn mention(v: &Vault, page: &str, id: &str, date: &str) {
    let text = format!("Meet Ana on {date}");
    block(
        v,
        page,
        id,
        BlockKind::Paragraph,
        &text,
        &[(date, date)],
        false,
    );
}

fn paragraph(v: &Vault, page: &str, text: &str) {
    block(v, page, &new_id(), BlockKind::Paragraph, text, &[], false);
}

/// A task note: a page whose `tasks` add-on holds its fields.
fn task(v: &Vault, title: &str, fields: Value) -> PageId {
    let id = page(v, title, None);
    for (key, value) in fields.as_object().unwrap() {
        v.type_data_set(
            id.clone(),
            "tasks".into(),
            vec![key.clone()],
            value.to_string(),
        )
        .unwrap();
    }
    id
}

/// A database with a date property; its rows are child pages.
fn database(v: &Vault, title: &str) -> PageId {
    let db = page(v, title, None);
    v.set_schema(
        db.clone(),
        vec![PropDef {
            key: "date".into(),
            name: "Date".into(),
            kind: PropKind::Date,
            options: vec![],
            config: String::new(),
        }],
    )
    .unwrap();
    db
}

fn row(v: &Vault, db: &str, title: &str, date: Option<&str>) -> PageId {
    let id = v.create_page(Some(db.into()), title.into()).unwrap();
    if let Some(d) = date {
        v.set_property(
            id.clone(),
            "date".into(),
            PropValue::Date {
                start_ms: day_ms(d),
                end_ms: None,
                has_time: false,
            },
        )
        .unwrap();
    }
    id
}

fn tag(v: &Vault, page: &str, tags: &[&str]) {
    v.set_property(
        page.into(),
        "tags".into(),
        PropValue::MultiSelect(tags.iter().map(|t| (*t).to_string()).collect()),
    )
    .unwrap();
}

fn items(v: &Vault, query: &str) -> Vec<ItemHit> {
    let spec = v.parse_search(query.into()).unwrap().spec;
    assert!(spec.selects_items(), "{query:?} selects items");
    v.search_items(&spec, 500, 0).unwrap().items
}

/// Each item's block id, or its title for task notes and rows.
fn keys(hits: &[ItemHit]) -> BTreeSet<String> {
    hits.iter()
        .map(|h| h.block.clone().unwrap_or_else(|| h.title.clone()))
        .collect()
}

fn set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|k| (*k).to_string()).collect()
}

fn assert_items(v: &Vault, query: &str, expected: &[&str]) {
    assert_eq!(keys(&items(v, query)), set(expected), "query {query:?}");
}

fn ids_of(v: &Vault, spec: SearchSpec) -> Vec<String> {
    v.search_spec(spec, 50, 0)
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.id)
        .collect()
}

#[test]
fn item_kinds_done_assignee_calendar_and_undated_select_their_items() {
    let t = TempVault::new();
    let v = t.open();
    let today = today();
    let plan = page(&v, "Plan", None);
    todo(
        &v,
        &plan,
        "todo-open",
        &format!("Water basil {today}"),
        Some(&today),
        false,
    );
    todo(
        &v,
        &plan,
        "todo-done",
        "Sort mail 2000-01-01",
        Some("2000-01-01"),
        true,
    );
    todo(&v, &plan, "todo-undated", "Call Sam", None, false);
    mention(&v, &plan, "mention-today", &today);
    let errands = v
        .create_calendar("Errands".into(), "moss".into(), true, true)
        .unwrap();
    let db = database(&v, "Visits");
    row(&v, &db, "Row A", Some(&today));
    row(&v, &db, "Row Old", Some("2000-01-01"));
    task(
        &v,
        "Note Agent",
        json!({"assignee": "agent", "calendar": errands}),
    );
    task(
        &v,
        "Note Me",
        json!({"status": "done", "assignee": "me", "date": "2000-01-01"}),
    );
    task(&v, "Note Plain", json!({"status": "doing"}));
    v.flush().unwrap();

    // Kinds, alone and combined.
    assert_items(&v, "is:todo", &["todo-open", "todo-done", "todo-undated"]);
    assert_items(&v, "is:task", &["Note Agent", "Note Me", "Note Plain"]);
    assert_items(&v, "is:row", &["Row A", "Row Old"]);
    assert_items(&v, "is:mention", &["mention-today"]);
    assert_items(
        &v,
        "is:todo,task",
        &[
            "todo-open",
            "todo-done",
            "todo-undated",
            "Note Agent",
            "Note Me",
            "Note Plain",
        ],
    );
    // Done state.
    assert_items(&v, "is:todo done:no", &["todo-open", "todo-undated"]);
    assert_items(&v, "is:todo done:yes", &["todo-done"]);
    assert_items(&v, "done:yes", &["todo-done", "Note Me"]);
    assert_items(&v, "is:task -done:yes", &["Note Agent", "Note Plain"]);
    assert_items(&v, "is:todo -done:yes", &["todo-open", "todo-undated"]);
    // Task note assignee and calendar.
    assert_items(&v, "assignee:me", &["Note Me"]);
    assert_items(&v, "assignee:agent", &["Note Agent"]);
    assert_items(&v, "calendar:Errands", &["Note Agent"]);
    // Undated, with item operators.
    assert_items(
        &v,
        "is:todo,task,row,mention undated",
        &["todo-undated", "Note Agent", "Note Plain"],
    );
    assert_items(&v, "is:todo undated", &["todo-undated"]);
    // date: is the item's own date.
    assert_items(&v, "is:todo date:today", &["todo-open"]);
    assert_items(&v, "is:row date:today", &["Row A"]);
    assert_items(&v, "is:mention date:today", &["mention-today"]);
    assert_items(
        &v,
        "is:todo,task,row,mention date:<today",
        &["todo-done", "Note Me", "Row Old"],
    );
    // Negation of a kind.
    assert_items(
        &v,
        "-is:mention",
        &[
            "todo-open",
            "todo-done",
            "todo-undated",
            "Note Agent",
            "Note Me",
            "Note Plain",
            "Row A",
            "Row Old",
        ],
    );
}

#[test]
fn page_operators_text_and_negation_apply_to_the_items_own_page_and_text() {
    let t = TempVault::new();
    let v = t.open();
    let plan = page(&v, "Plan", None);
    let sprint = page(&v, "Sprint", Some(&plan));
    let elsewhere = page(&v, "Elsewhere", None);
    let memory = page(&v, "Memory", None);
    let memo = page(&v, "Lantern memo", Some(&memory));
    todo(&v, &plan, "todo-milk", "Buy oat milk", None, false);
    todo(&v, &plan, "todo-tea", "Brew tea", None, false);
    todo(&v, &sprint, "todo-child", "Child task", None, false);
    todo(
        &v,
        &elsewhere,
        "todo-elsewhere",
        "Elsewhere task",
        None,
        false,
    );
    todo(&v, &memo, "todo-memory", "Memory task", None, false);
    tag(&v, &plan, &["work"]);
    tag(&v, &elsewhere, &["home"]);
    v.flush().unwrap();

    // in: keeps the page's subtree, children included.
    assert_items(
        &v,
        &format!("is:todo in:[[{plan}]]"),
        &["todo-milk", "todo-tea", "todo-child"],
    );
    // in:memory is the top-level Memory page and its subtree.
    assert_items(&v, "is:todo in:memory", &["todo-memory"]);
    // tag: is the item's own page.
    assert_items(&v, "is:todo tag:work", &["todo-milk", "todo-tea"]);
    assert_items(
        &v,
        "is:todo -tag:work",
        &["todo-child", "todo-elsewhere", "todo-memory"],
    );
    // Words match the item's own text, not its page title.
    assert_items(&v, "is:todo milk", &["todo-milk"]);
    assert_items(&v, "is:todo \"oat milk\"", &["todo-milk"]);
    assert_items(&v, "is:todo tea milk", &[]);
    assert_items(&v, "is:todo sprint", &[]);
    assert_items(
        &v,
        "is:todo -milk",
        &["todo-tea", "todo-child", "todo-elsewhere", "todo-memory"],
    );
}

#[test]
fn undated_alone_is_a_word_and_an_unknown_calendar_warns() {
    let t = TempVault::new();
    let v = t.open();
    let plain = v.parse_search("undated".into()).unwrap().spec;
    assert!(!plain.selects_items());
    assert!(plain.terms.iter().any(|term| term.value == "undated"));
    let with_items = v.parse_search("is:todo undated".into()).unwrap().spec;
    assert!(with_items.selects_items());
    assert!(with_items.operators.iter().any(|op| op.key == "undated"));
    let bad = v.parse_search("calendar:Nope".into()).unwrap().spec;
    assert!(
        bad.warnings.iter().any(|w| w.contains("Nope")),
        "{:?}",
        bad.warnings
    );
}

#[test]
fn size_controls_parse_round_trip_and_never_filter() {
    let t = TempVault::new();
    let v = t.open();
    for title in ["Meeting one", "Meeting two", "Meeting three", "Notes"] {
        let id = page(&v, title, None);
        paragraph(&v, &id, &format!("{title} about meeting notes"));
    }
    v.flush().unwrap();
    let plain = v.parse_search("meeting".into()).unwrap().spec;
    let shaped = v
        .parse_search("meeting context:3 full:100 limit:5".into())
        .unwrap()
        .spec;
    assert!(shaped.warnings.is_empty(), "{:?}", shaped.warnings);
    assert_eq!(
        (
            shaped.shape("context"),
            shaped.shape("full"),
            shaped.shape("limit")
        ),
        (Some(3), Some(100), Some(5))
    );
    assert!(!shaped.selects_items());
    let expected = ids_of(&v, plain);
    assert!(!expected.is_empty());
    assert_eq!(
        ids_of(&v, shaped.clone()),
        expected,
        "size controls filter nothing"
    );

    for (query, key) in [
        ("context:99", "context"),
        ("limit:0", "limit"),
        ("limit:500", "limit"),
        ("full:20001", "full"),
    ] {
        let spec = v.parse_search(format!("meeting {query}")).unwrap().spec;
        assert_eq!(spec.shape(key), None, "{query} is ignored");
        assert!(
            spec.warnings.iter().any(|w| w.contains(key)),
            "{query}: {:?}",
            spec.warnings
        );
        assert_eq!(ids_of(&v, spec), expected, "{query} does not filter");
    }

    let printed = shaped.to_query_string();
    let again = v.parse_search(printed.clone()).unwrap().spec;
    assert_eq!(
        (
            again.shape("context"),
            again.shape("full"),
            again.shape("limit")
        ),
        (Some(3), Some(100), Some(5)),
        "printed as {printed:?}"
    );
}

#[test]
fn sort_title_and_updated_reverse_with_direction_and_round_trip() {
    let t = TempVault::new();
    let v = t.open();
    let mut ids = Vec::new();
    for title in ["Bravo", "Alpha", "Charlie"] {
        let id = page(&v, title, None);
        paragraph(&v, &id, "quarterly report");
        ids.push(id);
    }
    v.flush().unwrap();
    let titles = |query: &str| -> Vec<String> {
        let spec = v.parse_search(query.into()).unwrap().spec;
        v.search_spec(spec, 50, 0)
            .unwrap()
            .hits
            .into_iter()
            .map(|h| h.title)
            .collect()
    };
    assert_eq!(titles("report sort:title"), ["Alpha", "Bravo", "Charlie"]);
    assert_eq!(
        titles("report sort:title,asc"),
        ["Alpha", "Bravo", "Charlie"]
    );
    assert_eq!(
        titles("report sort:title,desc"),
        ["Charlie", "Bravo", "Alpha"]
    );

    // Bravo 2000, Alpha 1000, Charlie 3000.
    t.timestamps(&ids[0], 2_000);
    t.timestamps(&ids[1], 1_000);
    t.timestamps(&ids[2], 3_000);
    v.flush().unwrap();
    assert_eq!(
        titles("report sort:updated,desc"),
        ["Charlie", "Bravo", "Alpha"]
    );
    assert_eq!(
        titles("report sort:updated,asc"),
        ["Alpha", "Bravo", "Charlie"]
    );

    let desc = v
        .parse_search("report sort:title,desc".into())
        .unwrap()
        .spec;
    assert_eq!(desc.sort.as_deref(), Some("-title"));
    assert!(desc.to_query_string().contains("sort:title,desc"));
    let asc = v
        .parse_search("report sort:updated,asc".into())
        .unwrap()
        .spec;
    assert_eq!(asc.sort.as_deref(), Some("+updated"));
    assert!(asc.to_query_string().contains("sort:updated,asc"));
    for spec in [desc, asc] {
        let printed = spec.to_query_string();
        let again = v.parse_search(printed.clone()).unwrap().spec;
        assert_eq!(again.sort, spec.sort, "printed as {printed:?}");
    }

    // The same direction rule for items.
    let plan = page(&v, "Plan", None);
    todo(&v, &plan, "todo-a", "Alpha todo", None, false);
    todo(&v, &plan, "todo-b", "Bravo todo", None, false);
    todo(&v, &plan, "todo-c", "Charlie todo", None, false);
    v.flush().unwrap();
    let item_titles =
        |query: &str| -> Vec<String> { items(&v, query).into_iter().map(|h| h.title).collect() };
    assert_eq!(
        item_titles("is:todo sort:title"),
        ["Alpha todo", "Bravo todo", "Charlie todo"]
    );
    assert_eq!(
        item_titles("is:todo sort:title,desc"),
        ["Charlie todo", "Bravo todo", "Alpha todo"]
    );
}

#[test]
fn queries_that_name_properties_like_operators_keep_filtering_them() {
    let t = TempVault::new();
    let v = t.open();
    let parent = page(&v, "Parent", None);
    let a = page(&v, "Doc A", Some(&parent));
    let b = page(&v, "Doc B", None);
    let c = page(&v, "Doc C", None);
    for id in [&a, &b, &c] {
        paragraph(&v, id, "meeting notes");
    }
    v.set_property(a.clone(), "is".into(), PropValue::Text("x".into()))
        .unwrap();
    v.set_property(a.clone(), "done".into(), PropValue::Checkbox(true))
        .unwrap();
    v.set_property(a.clone(), "limit".into(), PropValue::Number(5.0))
        .unwrap();
    v.set_property(
        a.clone(),
        "status".into(),
        PropValue::Select("doing".into()),
    )
    .unwrap();
    v.set_property(b.clone(), "is".into(), PropValue::Text("y".into()))
        .unwrap();
    v.set_property(b.clone(), "done".into(), PropValue::Checkbox(false))
        .unwrap();
    v.set_property(b.clone(), "limit".into(), PropValue::Number(9.0))
        .unwrap();
    tag(&v, &a, &["work"]);
    v.flush().unwrap();

    let page_ids = |query: &str| -> BTreeSet<String> {
        let spec = v.parse_search(query.into()).unwrap().spec;
        assert!(!spec.selects_items(), "{query:?} is a page query");
        ids_of(&v, spec).into_iter().collect()
    };
    let only_a: BTreeSet<String> = [a.clone()].into_iter().collect();
    for (query, quoted) in [
        ("done:yes", "\"done\":yes"),
        ("limit:5", "\"limit\":5"),
        ("is:x", "\"is\":x"),
    ] {
        assert_eq!(page_ids(query), only_a, "{query} filters the property");
        assert_eq!(
            page_ids(quoted),
            page_ids(query),
            "{query} matches {quoted}"
        );
    }

    let in_parent = format!("in:[[{parent}]]");
    let old_queries = [
        "meeting",
        "tag:work",
        in_parent.as_str(),
        "created:week",
        "status:doing",
        "-tag:x",
        "sort:title",
    ];
    for query in old_queries {
        let spec = v.parse_search(query.into()).unwrap().spec;
        assert!(!spec.selects_items(), "{query:?} has no item operator");
        let printed = spec.to_query_string();
        let reparsed = v.parse_search(printed.clone()).unwrap().spec;
        assert!(
            !reparsed.selects_items(),
            "{printed:?} has no item operator"
        );
        assert_eq!(
            ids_of(&v, reparsed),
            ids_of(&v, spec),
            "{query:?} re-parsed from {printed:?}"
        );
    }
}

#[test]
fn app_view_has_one_hit_per_page_with_the_first_open_item_as_snippet() {
    let t = TempVault::new();
    let v = t.open();
    let plan = page(&v, "Plan", None);
    todo(&v, &plan, "open-a", "Alpha open", None, false);
    todo(&v, &plan, "open-b", "Beta open", None, false);
    todo(&v, &plan, "closed", "Gamma closed", None, true);
    let quiet = page(&v, "Quiet", None);
    todo(&v, &quiet, "quiet-done", "Delta closed", None, true);
    v.flush().unwrap();
    let spec = v.parse_search("is:todo done:no".into()).unwrap().spec;
    let results = v.search_spec(spec, 10, 0).unwrap();
    assert_eq!(results.hits.len(), 1, "{:?}", results.hits);
    let hit = &results.hits[0];
    assert_eq!(hit.id, plan);
    assert!(
        ["Alpha open", "Beta open"].contains(&hit.snippet.as_str()),
        "snippet {:?}",
        hit.snippet
    );
    assert!(
        hit.matched_filters
            .iter()
            .any(|f| f.key == "done" && f.value == "no"),
        "{:?}",
        hit.matched_filters
    );
}

#[test]
fn live_item_query_reemits_when_a_todo_is_checked() {
    let t = TempVault::new();
    let v = t.open();
    let plan = page(&v, "Plan", None);
    todo(&v, &plan, "only-open", "Water basil", None, false);
    let other = page(&v, "Other", None);
    todo(&v, &other, "other-open", "Sweep floor", None, false);
    v.flush().unwrap();
    let spec = v.parse_search("is:todo done:no".into()).unwrap().spec;
    let subscription = v.watch_query(spec, 10).unwrap();
    let initial = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(initial.initial);
    let added: BTreeSet<String> = initial.added.iter().map(|h| h.id.clone()).collect();
    assert_eq!(added, [plan.clone(), other.clone()].into_iter().collect());

    v.apply(
        plan.clone(),
        vec![BlockOp::SetChecked {
            id: "only-open".into(),
            checked: true,
        }],
    )
    .unwrap();
    let update = loop {
        let next = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
        if !next.removed.is_empty() {
            break next;
        }
    };
    assert!(!update.initial);
    let key = HitKey {
        source: VAULT_SOURCE.into(),
        id: plan.clone(),
    };
    assert!(update.removed.contains(&key), "{:?}", update.removed);
}
