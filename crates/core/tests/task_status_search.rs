use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use serde_json::{json, Value};
use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("task_status_search_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }

    fn index(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.0.join(".vault/index.sqlite")).unwrap()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn status(v: &Vault, page: &str, value: Value) {
    v.type_data_set(
        page.into(),
        "tasks".into(),
        vec!["status".into()],
        value.to_string(),
    )
    .unwrap();
}

fn task(v: &Vault, title: &str, value: Option<Value>) -> String {
    // TaskComposer creates the typed page, then saveTask writes add-on keys.
    let page = v
        .create_page_with_type(None, title.into(), Some("task".into()))
        .unwrap();
    for (key, value) in [("checkbox", json!(true)), ("assignee", json!("me"))] {
        v.type_data_set(
            page.clone(),
            "tasks".into(),
            vec![key.into()],
            value.to_string(),
        )
        .unwrap();
    }
    if let Some(value) = value {
        status(v, &page, value);
    }
    page
}

fn hits(v: &Vault, query: &str) -> HashSet<String> {
    let parsed = v.parse_search(query.into()).unwrap();
    assert!(
        parsed.spec.warnings.is_empty(),
        "{query}: {:?}",
        parsed.spec.warnings
    );
    if query.contains("status:") {
        assert!(
            parsed
                .spec
                .operators
                .iter()
                .any(|op| op.key == "status" && op.property),
            "{query} must be a property operator"
        );
    }
    v.search_spec(parsed.spec, 50, 0)
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.id)
        .collect()
}

fn three_tasks(v: &Vault) -> [String; 3] {
    let pages = [
        task(v, "Alpha", Some(json!("todo"))),
        task(v, "Beta", Some(json!("doing"))),
        task(v, "Gamma", Some(json!("done"))),
    ];
    for page in &pages {
        v.set_property(
            page.clone(),
            "tags".into(),
            PropValue::MultiSelect(vec!["work".into()]),
        )
        .unwrap();
    }
    v.flush().unwrap();
    pages
}

fn assert_three_statuses(v: &Vault, pages: &[String; 3]) {
    assert_eq!(hits(v, "status:todo"), HashSet::from([pages[0].clone()]));
    assert_eq!(
        hits(v, "-status:done"),
        HashSet::from([pages[0].clone(), pages[1].clone()])
    );
    assert_eq!(hits(v, "status:doing"), HashSet::from([pages[1].clone()]));
    assert_eq!(
        hits(v, "#work type:task -status:done"),
        HashSet::from([pages[0].clone(), pages[1].clone()])
    );
}

#[test]
fn task_status_and_work_tag_filters_use_index_only_projection() {
    let temp = TempVault::new();
    let v = temp.open();
    let pages = three_tasks(&v);
    assert_three_statuses(&v, &pages);

    let index = temp.index();
    let projected: i64 = index.query_row("SELECT count(*) FROM props x JOIN pages p ON p.rowid=x.page WHERE p.page_type='task' AND x.key='status'", [], |row| row.get(0)).unwrap();
    assert_eq!(projected, 3);
    for (page, expected) in pages.iter().zip(["todo", "doing", "done"]) {
        let value: String = index.query_row("SELECT x.json FROM props x JOIN pages p ON p.rowid=x.page WHERE p.id=?1 AND x.key='status'", [page], |row| row.get(0)).unwrap();
        assert_eq!(
            serde_json::from_str::<PropValue>(&value).unwrap(),
            PropValue::Status(expected.into())
        );
        assert!(v
            .load_page(page.clone())
            .unwrap()
            .properties
            .iter()
            .all(|prop| prop.key != "status"));
        let data: Value =
            serde_json::from_str(&v.type_data(page.clone(), "tasks".into()).unwrap()).unwrap();
        assert_eq!(data["status"], expected);
    }
    // Export retains the existing type-data field, never an extra status property.
    for entry in std::fs::read_dir(temp.0.join("pages")).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(text.lines().any(|line| line.starts_with("type-data: ")));
        assert!(
            text.lines()
                .filter(|line| !line.starts_with("type-data: "))
                .all(|line| !line.contains("status")),
            "unexpected projected status in Markdown: {text}"
        );
    }
}

#[test]
fn missing_or_invalid_task_status_defaults_to_todo_without_touching_other_pages() {
    let temp = TempVault::new();
    let v = temp.open();
    let mut defaults = HashSet::new();
    defaults.insert(
        v.create_page_with_type(None, "No add-on data".into(), Some("task".into()))
            .unwrap(),
    );
    for (i, value) in [
        None,
        Some(json!("invalid")),
        Some(json!(4)),
        Some(Value::Null),
        Some(json!(false)),
        Some(json!({"value": "done"})),
    ]
    .into_iter()
    .enumerate()
    {
        defaults.insert(task(&v, &format!("Default {i}"), value));
    }
    let plain = v.create_page(None, "Ordinary page".into()).unwrap();
    status(&v, &plain, json!("todo"));
    v.flush().unwrap();
    assert_eq!(hits(&v, "status:todo"), defaults);
    assert!(hits(&v, "type:document status:todo").is_empty());
    // All three values remain recognized even if no task currently has them.
    assert!(hits(&v, "status:doing").is_empty());
    assert!(hits(&v, "status:done").is_empty());
    for id in defaults {
        assert!(v.load_page(id).unwrap().properties.is_empty());
    }
}

#[test]
fn ordinary_database_status_properties_keep_their_values_and_options() {
    let temp = TempVault::new();
    let v = temp.open();
    let collection = v.create_page(None, "Database".into()).unwrap();
    v.set_schema(
        collection.clone(),
        vec![PropDef {
            key: "status".into(),
            name: "Status".into(),
            kind: PropKind::Status,
            options: vec![SelectOption {
                id: "review".into(),
                name: "Review".into(),
                color: String::new(),
            }],
            config: String::new(),
        }],
    )
    .unwrap();
    let row = v
        .create_page(Some(collection), "Review item".into())
        .unwrap();
    v.set_property(
        row.clone(),
        "status".into(),
        PropValue::Status("review".into()),
    )
    .unwrap();
    let todo = task(&v, "Task", Some(json!("todo")));
    v.flush().unwrap();
    assert_eq!(hits(&v, "status:review"), HashSet::from([row]));
    assert_eq!(hits(&v, "status:todo"), HashSet::from([todo]));
    assert!(hits(&v, "status:doing").is_empty());
    assert!(hits(&v, "status:done").is_empty());
}

#[test]
fn app_generated_status_keys_remain_searchable_by_display_name() {
    let temp = TempVault::new();
    let v = temp.open();
    let collection = v.create_page(None, "App database".into()).unwrap();
    let schema = v
        .set_schema(
            collection.clone(),
            vec![PropDef {
                key: String::new(),
                name: "Status".into(),
                kind: PropKind::Status,
                options: ["Todo", "Doing", "Done", "Review"]
                    .into_iter()
                    .map(|name| SelectOption {
                        id: String::new(),
                        name: name.into(),
                        color: String::new(),
                    })
                    .collect(),
                config: String::new(),
            }],
        )
        .unwrap();
    let def = &schema[0];
    assert_ne!(def.key, "status");
    assert!(v
        .complete_search("status:re".into(), 9)
        .unwrap()
        .iter()
        .any(|option| option.label == "Review"));
    let mut rows = Vec::new();
    for name in ["Todo", "Done", "Review"] {
        let option = def
            .options
            .iter()
            .find(|option| option.name == name)
            .unwrap();
        let row = v
            .create_page(Some(collection.clone()), format!("Ordinary {name}"))
            .unwrap();
        v.set_property(
            row.clone(),
            def.key.clone(),
            PropValue::Status(option.id.clone()),
        )
        .unwrap();
        rows.push(row);
    }
    let tasks = three_tasks(&v);
    assert_eq!(
        hits(&v, "status:todo"),
        HashSet::from([rows[0].clone(), tasks[0].clone()])
    );
    assert_eq!(hits(&v, "status:doing"), HashSet::from([tasks[1].clone()]));
    assert_eq!(
        hits(&v, "status:done"),
        HashSet::from([rows[1].clone(), tasks[2].clone()])
    );
    assert_eq!(hits(&v, "status:review"), HashSet::from([rows[2].clone()]));
    assert_eq!(
        hits(&v, "type:document -status:done"),
        HashSet::from([rows[0].clone(), rows[2].clone()])
    );
    assert_eq!(
        hits(&v, "type:task -status:done"),
        HashSet::from([tasks[0].clone(), tasks[1].clone()])
    );
    assert_eq!(
        hits(&v, &format!("{}:todo", def.key)),
        HashSet::from([rows[0].clone()])
    );
    let parsed = v.parse_search("status:todo".into()).unwrap();
    let reparsed = v.parse_search(parsed.spec.to_query_string()).unwrap();
    assert_eq!(
        reparsed.spec, parsed.spec,
        "printing a shared status query must preserve every branch"
    );
    let result = v.search_spec(parsed.spec, 50, 0).unwrap();
    let ordinary = result.hits.iter().find(|hit| hit.id == rows[0]).unwrap();
    assert!(ordinary
        .matched_filters
        .iter()
        .any(|matched| matched.key == def.key && matched.value == "Todo"));
    let task = result.hits.iter().find(|hit| hit.id == tasks[0]).unwrap();
    assert!(task
        .matched_filters
        .iter()
        .any(|matched| matched.key == "status" && matched.value == "todo"));
    let empty = v
        .create_page(Some(collection.clone()), "Empty status".into())
        .unwrap();
    v.flush().unwrap();
    assert_eq!(
        hits(&v, "type:document status:none"),
        HashSet::from([empty])
    );
    assert_eq!(
        hits(&v, "type:document -status:none"),
        rows.iter().cloned().collect()
    );
    // A task can also carry an independent database property named Status.
    let both = v
        .create_page_with_type(Some(collection), "Both fields".into(), Some("task".into()))
        .unwrap();
    status(&v, &both, json!("todo"));
    let done = def
        .options
        .iter()
        .find(|option| option.name == "Done")
        .unwrap();
    v.set_property(
        both.clone(),
        def.key.clone(),
        PropValue::Status(done.id.clone()),
    )
    .unwrap();
    v.flush().unwrap();
    let parsed = v.parse_search("type:task status:done".into()).unwrap();
    let result = v.search_spec(parsed.spec, 50, 0).unwrap();
    let matched = result.hits.iter().find(|hit| hit.id == both).unwrap();
    assert!(matched
        .matched_filters
        .iter()
        .any(|value| value.key == def.key && value.value == "Done"));
    assert!(!matched
        .matched_filters
        .iter()
        .any(|value| value.key == "status"));
}

#[test]
fn numeric_and_text_status_schemas_do_not_hide_task_status_values() {
    for kind in [PropKind::Number, PropKind::Text] {
        let temp = TempVault::new();
        let v = temp.open();
        let collection = v.create_page(None, "Database".into()).unwrap();
        v.set_schema(
            collection.clone(),
            vec![PropDef {
                key: "status".into(),
                name: "Status".into(),
                kind: kind.clone(),
                options: Vec::new(),
                config: String::new(),
            }],
        )
        .unwrap();
        let row = v
            .create_page(Some(collection), "Ordinary row".into())
            .unwrap();
        let (value, query) = if kind == PropKind::Number {
            (PropValue::Number(3.0), "status:3")
        } else {
            (PropValue::Text("Awaiting review".into()), "status:review")
        };
        v.set_property(row.clone(), "status".into(), value).unwrap();
        let todo = task(&v, "Task", Some(json!("todo")));
        v.flush().unwrap();
        assert_eq!(hits(&v, query), HashSet::from([row.clone()]));
        assert_eq!(hits(&v, "status:todo"), HashSet::from([todo.clone()]));
        let mixed = if kind == PropKind::Number {
            "status:todo,3"
        } else {
            "status:todo,review"
        };
        assert_eq!(hits(&v, mixed), HashSet::from([row, todo]));
        assert!(hits(&v, "type:task status:doing").is_empty());
        assert!(hits(&v, "type:task status:done").is_empty());
    }
}

#[test]
fn task_projection_survives_computed_status_properties_without_becoming_document_data() {
    let temp = TempVault::new();
    let v = temp.open();
    let collection = v.create_page(None, "Computed database".into()).unwrap();
    v.set_schema(
        collection.clone(),
        ["status", "score"]
            .into_iter()
            .map(|key| PropDef {
                key: key.into(),
                name: key.into(),
                kind: PropKind::Formula,
                options: Vec::new(),
                config: json!({"formula": if key == "status" { "42" } else { "7" }}).to_string(),
            })
            .chain(std::iter::once(PropDef {
                key: "note".into(),
                name: "Note".into(),
                kind: PropKind::Text,
                options: Vec::new(),
                config: String::new(),
            }))
            .collect(),
    )
    .unwrap();
    let page = v
        .create_page_with_type(Some(collection.clone()), "Task".into(), Some("task".into()))
        .unwrap();
    status(&v, &page, json!("todo"));
    v.set_property(
        page.clone(),
        "note".into(),
        PropValue::Text("Changed".into()),
    )
    .unwrap();
    let ordinary = v
        .create_page(Some(collection.clone()), "Ordinary row".into())
        .unwrap();
    v.flush().unwrap();
    assert_eq!(
        hits(&v, "type:task status:todo"),
        HashSet::from([page.clone()])
    );
    assert_eq!(hits(&v, "status:42"), HashSet::from([ordinary]));
    let loaded = v.load_page(page.clone()).unwrap();
    assert!(loaded.properties.iter().all(|prop| prop.key != "status"));
    assert!(loaded
        .properties
        .iter()
        .any(|prop| prop.key == "score" && prop.value == PropValue::Number(7.0)));
    status(&v, &page, json!("done"));
    v.flush().unwrap();
    assert!(hits(&v, "type:task status:todo").is_empty());
    assert_eq!(
        hits(&v, "type:task status:done"),
        HashSet::from([page.clone()])
    );
    v.set_schema(collection, Vec::new()).unwrap();
    v.flush().unwrap();
    assert_eq!(hits(&v, "type:task status:done"), HashSet::from([page]));
}

#[test]
fn live_task_status_query_observes_initial_add_remove_and_reentry() {
    let temp = TempVault::new();
    let v = temp.open();
    // Register before the first task exists so absent options cannot become text.
    let parsed = v.parse_search("type:task -status:done".into()).unwrap();
    assert!(parsed
        .spec
        .operators
        .iter()
        .any(|op| op.key == "status" && op.property));
    let sub = v.watch_query(parsed.spec, 50).unwrap();
    let initial = sub.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(initial.initial && initial.added.is_empty());
    let page = task(&v, "Watch me", Some(json!("todo")));
    v.flush().unwrap();
    let added = sub.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!added.initial);
    assert_eq!(
        added.added.iter().map(|hit| &hit.id).collect::<Vec<_>>(),
        vec![&page]
    );
    assert_eq!(added.total_bounded, 1);
    status(&v, &page, json!("done"));
    v.flush().unwrap();
    let removed = sub.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(removed.generation > added.generation);
    assert_eq!(
        removed.removed,
        vec![HitKey {
            source: VAULT_SOURCE.into(),
            id: page.clone()
        }]
    );
    assert_eq!(removed.total_bounded, 0);
    status(&v, &page, json!("todo"));
    v.flush().unwrap();
    let back = sub.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(back.generation > removed.generation);
    assert_eq!(
        back.added.iter().map(|hit| &hit.id).collect::<Vec<_>>(),
        vec![&page]
    );
    assert_eq!(back.total_bounded, 1);
    assert_eq!(
        sub.recv_timeout(Duration::from_millis(150)),
        Err(RecvTimeoutError::Timeout)
    );
}

#[test]
fn version_11_reopen_rebuilds_task_status_projection_from_unchanged_documents() {
    let temp = TempVault::new();
    let pages;
    {
        let v = temp.open();
        pages = three_tasks(&v);
    }
    {
        let index = temp.index();
        // Reproduce the old index's missing projection while preserving documents.
        index
            .execute("DELETE FROM props WHERE key='status'", [])
            .unwrap();
        index.pragma_update(None, "user_version", 11).unwrap();
        let count: i64 = index
            .query_row("SELECT count(*) FROM props WHERE key='status'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    let v = temp.open();
    assert_three_statuses(&v, &pages);
    let version: i64 = temp
        .index()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 14);
    for (id, expected) in pages.iter().zip(["todo", "doing", "done"]) {
        assert!(v
            .load_page(id.clone())
            .unwrap()
            .properties
            .iter()
            .all(|prop| prop.key != "status"));
        let data: Value =
            serde_json::from_str(&v.type_data(id.clone(), "tasks".into()).unwrap()).unwrap();
        assert_eq!(data["status"], expected);
    }
}
