use std::path::PathBuf;

use chrono::{Local, NaiveDate, TimeZone, Utc};
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
    let dir =
        Temp(std::env::temp_dir().join(format!("vault_tools_agenda_{}", vault_core::new_id())));
    let vault = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
    (dir, vault)
}
/// A successful call's text (the agenda alias answers search_notes' text).
fn run(v: &Vault, name: &str, args: Value) -> String {
    call(v, name, &args).unwrap_or_else(|e| panic!("{name} {args}: {e}"))
}
/// A successful call's JSON (daily_note still answers JSON).
fn run_json(v: &Vault, name: &str, args: Value) -> Value {
    serde_json::from_str(&run(v, name, args)).unwrap()
}
fn utc_day(date: &str) -> i64 {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}
fn todo(v: &Vault, page: &str, id: &str, date: Option<&str>) {
    let text = date
        .map(|d| format!("Buy milk @{d}"))
        .unwrap_or("Buy milk".into());
    let marks = date
        .map(|d| {
            vec![Mark {
                start: 9,
                end: text.chars().count() as u32,
                kind: MarkKind::Date(d.into()),
            }]
        })
        .unwrap_or_default();
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
fn database(v: &Vault) -> String {
    let db = v.create_page(None, "Tasks".into()).unwrap();
    v.set_schema(
        db.clone(),
        vec![
            PropDef {
                key: "date".into(),
                name: "Date".into(),
                kind: PropKind::Date,
                options: vec![],
                config: String::new(),
            },
            PropDef {
                key: "done".into(),
                name: "Done".into(),
                kind: PropKind::Checkbox,
                options: vec![],
                config: String::new(),
            },
        ],
    )
    .unwrap();
    db
}

fn task_fields(v: &Vault, page: &str, fields: Value) {
    for (key, value) in fields.as_object().unwrap() {
        v.type_data_set(
            page.into(),
            "tasks".into(),
            vec![key.clone()],
            value.to_string(),
        )
        .unwrap();
    }
}

/// The item lines of a search_notes item result: `  \`id\` | [x] title · kind
/// · facts · ref REFERENCE`.
fn item_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.contains(" · ref ")).collect()
}

/// The item references of a result, in result order.
fn refs(text: &str) -> Vec<String> {
    item_lines(text)
        .into_iter()
        .map(|l| l.rsplit_once(" · ref ").unwrap().1.to_string())
        .collect()
}

/// The item line of `reference`.
fn line(text: &str, reference: &str) -> String {
    item_lines(text)
        .into_iter()
        .find(|l| l.ends_with(&format!(" · ref {reference}")))
        .unwrap_or_else(|| panic!("{reference} not in: {text}"))
        .to_string()
}

/// The count from "N item(s) match; ...", or 0 when nothing matches.
fn total(text: &str) -> usize {
    match text.lines().find_map(|l| l.split_once(" item(s) match;")) {
        Some((n, _)) => n.trim().parse().unwrap(),
        None => {
            assert!(text.contains("No items match"), "{text}");
            0
        }
    }
}

/// The offset of a `_More: offset=N_` line, if the result has one.
fn more(text: &str) -> Option<usize> {
    let rest = text.split("_More: offset=").nth(1)?;
    rest.split('_').next()?.parse().ok()
}

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
                    start: 5,
                    end: text.chars().count() as u32,
                    kind: MarkKind::Date(date.into()),
                }],
                text,
            },
        }],
    )
    .unwrap();
}

#[test]
fn agenda_defaults_include_all_sources_and_undated_but_calendar_settings_are_not_tasks() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Sources".into()).unwrap();
    todo(&v, &page, "dated", Some("2000-01-01"));
    todo(&v, &page, "undated", None);
    mention(&v, &page, "mention", "2000-01-01");
    let note = v.create_page(None, "Task note".into()).unwrap();
    task_fields(&v, &note, json!({"status":"doing","assignee":"me"}));
    let db = database(&v);
    let row = v.create_page(Some(db.clone()), "Dated row".into()).unwrap();
    v.set_property(
        row.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: utc_day("2000-01-01"),
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();
    v.create_page(Some(db.clone()), "Undated row excluded".into())
        .unwrap();
    let all = run(&v, "agenda", json!({}));
    assert_eq!(total(&all), 5, "{all}");
    assert!(
        v.find_by_role("calendars".into()).unwrap().is_none(),
        "agenda must not initialize settings"
    );
    for (kind, expected) in [
        (
            "todo",
            vec![
                format!("block:{page}/dated"),
                format!("block:{page}/undated"),
            ],
        ),
        ("task_note", vec![format!("note:{note}")]),
        ("row", vec![format!("row:{db}/{row}")]),
        ("mention", vec![format!("mention:{page}/mention")]),
    ] {
        assert_eq!(
            refs(&run(&v, "agenda", json!({"kinds":[kind]}))),
            expected,
            "{kind}"
        );
    }
    let undated = run(&v, "agenda", json!({"undated":true}));
    assert_eq!(total(&undated), 2, "{undated}");
    assert!(
        item_lines(&undated)
            .iter()
            .all(|l| l.contains(" · no date · ")),
        "{undated}"
    );
    assert_eq!(run(&v, "agenda", json!({"undated":false})), all);
    assert!(refs(&run(
        &v,
        "agenda",
        json!({"undated":true,"from":"2000-01-01"})
    ))
    .is_empty());
    assert!(refs(&run(&v, "agenda", json!({"undated":true,"view":"today"}))).is_empty());
    // An empty kinds list is an error now (it used to list nothing).
    assert!(call(&v, "agenda", &json!({"kinds":[]})).is_err());
    v.list_calendars().unwrap();
    assert_eq!(
        total(&run(&v, "agenda", json!({}))),
        5,
        "calendar settings must not become an undated task"
    );
}

#[test]
fn agenda_date_bounds_are_independent_half_open_and_completed_ranges_use_scheduled_dates() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Dates".into()).unwrap();
    for (id, date) in [
        ("before", "2000-01-01"),
        ("inside", "2000-01-02"),
        ("after", "2000-01-03"),
    ] {
        todo(&v, &page, id, Some(date));
    }
    todo(&v, &page, "undated", None);
    run(
        &v,
        "update_task",
        json!({"item":format!("block:{page}/inside"),"done":true}),
    );
    assert_eq!(
        refs(&run(&v, "agenda", json!({"from":"2000-01-02"}))),
        vec![
            format!("block:{page}/inside"),
            format!("block:{page}/after")
        ]
    );
    assert_eq!(
        refs(&run(&v, "agenda", json!({"to":"2000-01-02"}))),
        vec![format!("block:{page}/before")]
    );
    assert_eq!(
        refs(&run(
            &v,
            "agenda",
            json!({"from":"2000-01-02","to":"2000-01-03"})
        )),
        vec![format!("block:{page}/inside")]
    );
    assert_eq!(
        refs(&run(
            &v,
            "agenda",
            json!({"done":"done","from":"2000-01-02","to":"2000-01-03"})
        )),
        vec![format!("block:{page}/inside")]
    );
    assert_eq!(total(&run(&v, "agenda", json!({"done":"open"}))), 3);
    assert_eq!(total(&run(&v, "agenda", json!({"done":"any"}))), 4);
    assert!(refs(&run(
        &v,
        "agenda",
        json!({"done":"done","from":"2026-01-01"})
    ))
    .is_empty());
    assert!(
        call(
            &v,
            "agenda",
            &json!({"from":"2000-01-01","to":"2001-01-01"})
        )
        .is_ok(),
        "new agenda has no legacy 62-day cap"
    );
}

#[test]
fn agenda_range_retains_overlapping_multi_day_rows_and_timed_notes() {
    let (_temp, v) = temp();
    let db = database(&v);
    let row = v.create_page(Some(db.clone()), "Multi-day".into()).unwrap();
    v.set_property(
        row.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: utc_day("2030-01-01"),
            end_ms: Some(utc_day("2030-01-03")),
            has_time: false,
        },
    )
    .unwrap();
    let note = v.create_page(None, "Across midnight".into()).unwrap();
    let local = Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2030, 1, 2)
                .unwrap()
                .and_hms_opt(23, 30, 0)
                .unwrap(),
        )
        .single()
        .unwrap();
    task_fields(
        &v,
        &note,
        json!({"date": local.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string(), "duration_min":60}),
    );
    let overlap = run(&v, "agenda", json!({"from":"2030-01-03","to":"2030-01-04"}));
    let found = refs(&overlap);
    assert!(found.contains(&format!("row:{db}/{row}")));
    assert!(found.contains(&format!("note:{note}")));
    assert!(refs(&run(
        &v,
        "agenda",
        json!({"from":"2030-01-04","to":"2030-01-05"})
    ))
    .is_empty());
}

#[test]
fn agenda_calendar_assignment_tags_and_exact_page_subtree_filters_intersect() {
    let (_temp, v) = temp();
    let hidden = v
        .create_calendar("Private work".into(), "sky".into(), false, false)
        .unwrap();
    let root = v.create_page(None, "Filter root".into()).unwrap();
    let child = v.create_page(Some(root.clone()), "Child".into()).unwrap();
    let grandchild = v
        .create_page(Some(child.clone()), "Grandchild".into())
        .unwrap();
    let outside = v.create_page(None, "Outside".into()).unwrap();
    for (page, assignee) in [
        (&root, "me"),
        (&child, "agent"),
        (&grandchild, "agent"),
        (&outside, "agent"),
    ] {
        task_fields(
            &v,
            page,
            json!({"status":"doing","calendar":hidden,"assignee":assignee}),
        );
    }
    v.set_property(
        child.clone(),
        "tags".into(),
        PropValue::MultiSelect(vec!["urgent".into()]),
    )
    .unwrap();
    v.set_property(
        outside.clone(),
        "tags".into(),
        PropValue::MultiSelect(vec!["urgent".into()]),
    )
    .unwrap();
    let category = run(&v, "agenda", json!({"calendar":["Private work"]}));
    assert_eq!(total(&category), 4, "hidden UI categories remain queryable");
    assert!(
        item_lines(&category)
            .iter()
            .all(|l| l.contains(" · status doing · ") && l.contains(" · calendar Private work · ")),
        "{category}"
    );
    // calendar: takes names and, through search_notes, ids too.
    assert_eq!(
        refs(&run(&v, "agenda", json!({"calendar":[hidden]}))),
        refs(&category),
        "calendar accepts category ids as well as names"
    );
    assert_eq!(
        refs(&run(&v, "agenda", json!({"assignee":"me"}))),
        vec![format!("note:{root}")]
    );
    assert_eq!(total(&run(&v, "agenda", json!({"assignee":"agent"}))), 3);
    // Without subtree, `page` is that page only, not its children.
    assert_eq!(
        refs(&run(&v, "agenda", json!({"page":"Filter root"}))),
        vec![format!("note:{root}")]
    );
    assert_eq!(
        total(&run(&v, "agenda", json!({"page":root,"subtree":true}))),
        3
    );
    assert_eq!(
        refs(&run(
            &v,
            "agenda",
            json!({"calendar":["Private work"],"page":root,"subtree":true,"assignee":"agent","tag":"urgent"})
        )),
        vec![format!("note:{child}")]
    );
    assert_eq!(total(&run(&v, "agenda", json!({"tag":"urgent"}))), 2);
    // An empty calendar list is an error now (it used to list nothing).
    assert!(call(&v, "agenda", &json!({"calendar":[]})).is_err());
}

#[test]
fn agenda_tags_match_database_option_names_and_literal_text_properties() {
    for (kind, value) in [
        (
            PropKind::MultiSelect,
            PropValue::MultiSelect(vec!["urgent-id".into()]),
        ),
        (PropKind::Text, PropValue::Text("urgent".into())),
    ] {
        // One vault per kind: the parser reads a property key by the first
        // definition it finds, so a select Tags elsewhere would win.
        let (_temp, v) = temp();
        let db = database(&v);
        let mut schema = v.get_schema(db.clone()).unwrap();
        schema.push(PropDef {
            key: "tags".into(),
            name: "Tags".into(),
            kind,
            options: if kind == PropKind::MultiSelect {
                vec![vault_core::SelectOption {
                    id: "urgent-id".into(),
                    name: "urgent".into(),
                    color: String::new(),
                }]
            } else {
                vec![]
            },
            config: String::new(),
        });
        v.set_schema(db.clone(), schema).unwrap();
        let row = v
            .create_page(Some(db.clone()), "Tagged row".into())
            .unwrap();
        v.set_property(
            row.clone(),
            "date".into(),
            PropValue::Date {
                start_ms: utc_day("2000-01-01"),
                end_ms: None,
                has_time: false,
            },
        )
        .unwrap();
        v.set_property(row.clone(), "tags".into(), value).unwrap();
        // agenda tag matches the tags property's select options, by name
        // or id. A text property is not a tag, so it never matches here.
        let expected = if kind == PropKind::MultiSelect {
            vec![format!("row:{db}/{row}")]
        } else {
            Vec::new()
        };
        for tag in ["urgent", "urgent-id"] {
            assert_eq!(
                refs(&run(
                    &v,
                    "agenda",
                    json!({"page":db,"subtree":true,"tag":tag})
                )),
                expected,
                "agenda tag {tag}"
            );
        }
        // search_notes finds the text property by its name.
        if kind == PropKind::Text {
            let found = run(
                &v,
                "search_notes",
                json!({"query": format!("in:[[{db}]] \"tags\":urgent")}),
            );
            assert!(
                found.contains(&format!("`{row}`")),
                "search_notes \"tags\":urgent finds the text property: {found}"
            );
        }
    }
}

#[test]
fn agenda_pagination_is_stable_complete_and_applies_after_filters() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Many tasks".into()).unwrap();
    let blocks = (0..57)
        .map(|i| BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: format!("task-{i:03}"),
                kind: BlockKind::Todo,
                text: format!("Task {i}"),
                marks: vec![],
                checked: i % 2 == 0,
            },
        })
        .collect();
    v.apply(page.clone(), blocks).unwrap();
    // The alias's default page is search_notes' 20; ask for 50 as the old default was.
    let first = run(&v, "agenda", json!({"limit":50}));
    assert_eq!(total(&first), 57);
    assert_eq!(refs(&first).len(), 50);
    assert_eq!(more(&first), Some(50), "{first}");
    let second = run(&v, "agenda", json!({"offset":50,"limit":50}));
    assert_eq!(refs(&second).len(), 7);
    assert_eq!(more(&second), None, "{second}");
    let mut combined = refs(&first);
    combined.extend(refs(&second));
    assert_eq!(combined, refs(&run(&v, "agenda", json!({"limit":200}))));
    assert_eq!(
        first,
        run(&v, "agenda", json!({"limit":50})),
        "repeated pages have deterministic order"
    );
    let filtered = run(&v, "agenda", json!({"done":"open","limit":3,"offset":2}));
    assert_eq!(total(&filtered), 28);
    assert_eq!(
        refs(&filtered),
        vec![
            format!("block:{page}/task-005"),
            format!("block:{page}/task-007"),
            format!("block:{page}/task-009")
        ]
    );
    assert!(refs(&run(&v, "agenda", json!({"offset":999}))).is_empty());
}

#[test]
fn bare_typed_task_is_undated_and_its_note_reference_accepts_due_and_done() {
    let (_temp, v) = temp();
    let task = v
        .create_page_with_type(None, "Bare task".into(), Some("task".into()))
        .unwrap();
    let plain = v.create_page(None, "Ordinary empty page".into()).unwrap();
    let reference = format!("note:{task}");
    let agenda = run(&v, "agenda", json!({"undated":true}));
    assert_eq!(refs(&agenda), vec![reference.clone()]);
    let bare = line(&agenda, &reference);
    assert!(bare.contains(" · task · no date · "), "{bare}");
    assert!(bare.contains(" · status todo · "), "{bare}");
    assert!(bare.contains("| [ ] "), "{bare}");
    assert!(vault_tools::resolve_agenda_item(&v, &format!("note:{plain}")).is_err());
    assert!(call(
        &v,
        "update_task",
        &json!({"item":format!("note:{plain}"),"done":true})
    )
    .is_err());
    run(
        &v,
        "update_task",
        json!({"item":reference,"due":"2000-01-02","done":true}),
    );
    let dated = run(
        &v,
        "agenda",
        json!({"from":"2000-01-02","to":"2000-01-03","done":"done"}),
    );
    assert_eq!(refs(&dated), vec![reference.clone()]);
    let done = line(&dated, &reference);
    assert!(done.contains("due 2000-01-02 · "), "{done}");
    assert!(done.contains(" · status done · "), "{done}");
    assert!(done.contains("| [x] "), "{done}");
}

#[test]
fn task_note_references_mutate_the_note_and_database_refs_mutate_the_row() {
    let (_temp, v) = temp();
    let db = database(&v);
    let page = v
        .create_page(Some(db.clone()), "Dual source".into())
        .unwrap();
    v.set_property(
        page.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: utc_day("2000-01-01"),
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();
    task_fields(&v, &page, json!({"status":"doing","assignee":"agent"}));
    let note_ref = format!("note:{page}");
    let row_ref = format!("row:{db}/{page}");
    let both = run(&v, "agenda", json!({}));
    assert_eq!(total(&both), 2);
    assert!(refs(&both).contains(&note_ref));
    assert!(refs(&both).contains(&row_ref));
    run(
        &v,
        "update_task",
        json!({"item":note_ref,"due":"2000-01-02","done":true}),
    );
    let note_data: Value =
        serde_json::from_str(&v.type_data(page.clone(), "tasks".into()).unwrap()).unwrap();
    assert_eq!(note_data["date"], "2000-01-02");
    assert_eq!(note_data["status"], "done");
    assert!(v.load_page(page.clone()).unwrap().properties.iter().any(|p| p.key == "date" && matches!(&p.value, PropValue::Date {start_ms, ..} if *start_ms == utc_day("2000-01-01"))));
    run(
        &v,
        "update_task",
        json!({"item":row_ref,"due":"2000-01-03","done":true}),
    );
    let after = run(&v, "agenda", json!({"page":page}));
    assert!(
        line(&after, &note_ref).contains("due 2000-01-02 · "),
        "{after}"
    );
    assert!(
        line(&after, &row_ref).contains("due 2000-01-03 · "),
        "{after}"
    );
    assert!(
        item_lines(&after).iter().all(|l| l.contains("| [x] ")),
        "{after}"
    );
    // A dated task note formerly failed the entire tasks range because it
    // had neither a block nor a database. Both entry points must serialize it.
    let listed = run(&v, "agenda", json!({"from":"2000-01-01","to":"2000-01-04"}));
    let mut listed_refs = refs(&listed);
    listed_refs.sort();
    let mut expected = vec![note_ref, row_ref];
    expected.sort();
    assert_eq!(listed_refs, expected);
}

#[test]
fn mentions_resolve_for_reading_and_update_task_refuses_to_mutate_them() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Mention".into()).unwrap();
    mention(&v, &page, "date", "2000-01-01");
    let reference = format!("mention:{page}/date");
    let found = run(&v, "agenda", json!({"kinds":["mention"]}));
    assert_eq!(refs(&found), vec![reference.clone()]);
    let target = vault_tools::resolve_agenda_item(&v, &reference).unwrap();
    assert_eq!(target.page, page);
    assert_eq!(target.block.as_deref(), Some("date"));
    assert!(!target.writable);
    let before = v.load_page(page.clone()).unwrap();
    for args in [
        json!({"item":reference,"due":"2000-01-03"}),
        json!({"item":reference,"done":true}),
    ] {
        assert!(call(&v, "update_task", &args)
            .unwrap_err()
            .0
            .contains("read-only"));
    }
    assert_eq!(v.load_page(page).unwrap(), before);
}

#[test]
fn agenda_is_paged_and_the_tasks_alias_is_gone() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Legacy".into()).unwrap();
    for i in 0..205 {
        todo(&v, &page, &format!("past-{i:03}"), Some("2000-01-01"));
    }
    let today = Local::now().date_naive().to_string();
    todo(&v, &page, "today", Some(&today));
    assert_eq!(
        refs(&run(&v, "agenda", json!({"view":"today"}))),
        vec![format!("block:{page}/today")]
    );
    // The old tasks range was uncapped; agenda pages at search_notes' 20 and
    // stops at 200.
    let range = run(&v, "agenda", json!({"from":"2000-01-01","to":"2000-01-02"}));
    assert_eq!(total(&range), 205);
    assert_eq!(refs(&range).len(), 20, "{range}");
    assert_eq!(more(&range), Some(20), "{range}");
    assert!(call(&v, "agenda", &json!({"limit":201})).is_err());
    assert_eq!(total(&run(&v, "overdue", json!({}))), 205);
    assert!(vault_tools::get("tasks").is_none());
    assert!(vault_tools::alias("tasks").is_none());
    assert!(call(&v, "tasks", &json!({})).is_err());
    assert_eq!(vault_tools::canonical("agenda"), "search_notes");
    assert_eq!(vault_tools::canonical("overdue"), "search_notes");
}

#[test]
fn agenda_rejects_bad_filter_values_and_combined_updates_validate_before_writing() {
    let (_temp, v) = temp();
    for args in [
        json!({"kinds":["event"]}),
        json!({"kinds":"todo"}),
        json!({"calendar":"Work"}),
        json!({"calendar":[1]}),
        json!({"done":"yes"}),
        json!({"assignee":"someone"}),
        json!({"from":"2000-1-01"}),
        json!({"to":"2000-02-30"}),
        json!({"from":3}),
        json!({"undated":"true"}),
        json!({"subtree":true}),
        json!({"view":"range"}),
        json!({"limit":0}),
        json!({"limit":201}),
        json!({"offset":-1}),
        json!({"offset":0.5}),
    ] {
        assert!(call(&v, "agenda", &args).is_err(), "{args}");
    }
    let page = v.create_page(None, "No partial update".into()).unwrap();
    todo(&v, &page, "todo", None);
    let before = v.load_page(page.clone()).unwrap();
    assert!(call(
        &v,
        "update_task",
        &json!({"item":format!("block:{page}/todo"),"due":"2000-01-01","done":"yes"})
    )
    .is_err());
    assert_eq!(v.load_page(page).unwrap(), before);
}

#[test]
fn agenda_today_and_overdue_keep_legacy_source_eligibility_without_category_visibility() {
    let (_temp, v) = temp();
    let today = Local::now().date_naive();
    let yesterday = today.pred_opt().unwrap();
    let tomorrow = today.succ_opt().unwrap();
    let page = v.create_page(None, "Shortcut sources".into()).unwrap();
    todo(&v, &page, "today", Some(&today.to_string()));
    todo(&v, &page, "past", Some(&yesterday.to_string()));
    mention(&v, &page, "mention", &today.to_string());
    let hidden = v
        .create_calendar("Hidden".into(), "sky".into(), false, false)
        .unwrap();
    let note = v.create_page(None, "Hidden note".into()).unwrap();
    task_fields(
        &v,
        &note,
        json!({"date":today.to_string(),"status":"todo","calendar":hidden}),
    );

    let off = database(&v);
    v.set_agenda_settings(
        off.clone(),
        vault_core::AgendaSettings {
            role: vault_core::AgendaRole::Off,
            date_prop: None,
        },
    )
    .unwrap();
    let off_today = v
        .create_page(Some(off.clone()), "Off today".into())
        .unwrap();
    let off_past = v.create_page(Some(off.clone()), "Off past".into()).unwrap();
    for (row, date) in [(&off_today, today), (&off_past, yesterday)] {
        v.set_property(
            row.clone(),
            "date".into(),
            PropValue::Date {
                start_ms: utc_day(&date.to_string()),
                end_ms: None,
                has_time: false,
            },
        )
        .unwrap();
    }
    let events = v.create_page(None, "Events".into()).unwrap();
    v.set_schema(
        events.clone(),
        vec![PropDef {
            key: "date".into(),
            name: "Date".into(),
            kind: PropKind::Date,
            options: vec![],
            config: String::new(),
        }],
    )
    .unwrap();
    let event = v
        .create_page(Some(events.clone()), "Ongoing event".into())
        .unwrap();
    v.set_property(
        event.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: utc_day(&yesterday.to_string()),
            end_ms: Some(utc_day(&tomorrow.to_string())),
            has_time: false,
        },
    )
    .unwrap();
    let past_event = v
        .create_page(Some(events.clone()), "Past event".into())
        .unwrap();
    v.set_property(
        past_event.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: utc_day(&yesterday.to_string()),
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();

    let today_refs = refs(&run(&v, "agenda", json!({"view":"today"})));
    assert!(today_refs.contains(&format!("block:{page}/today")));
    assert!(
        today_refs.contains(&format!("note:{note}")),
        "new shortcut ignores category visibility"
    );
    assert!(
        today_refs.contains(&format!("row:{events}/{event}")),
        "overlapping events remain in Today"
    );
    assert!(!today_refs.contains(&format!("mention:{page}/mention")));
    // Rows of databases whose agenda role is off still appear in Today.
    assert!(today_refs.contains(&format!("row:{off}/{off_today}")));
    // Overdue lists todos and tasks only, so past rows and events are left out.
    assert_eq!(
        refs(&run(&v, "agenda", json!({"view":"overdue"}))),
        vec![format!("block:{page}/past")]
    );
    let range_refs = refs(&run(
        &v,
        "agenda",
        json!({"from":yesterday.to_string(),"to":tomorrow.to_string()}),
    ));
    assert!(range_refs.contains(&format!("row:{off}/{off_past}")));
    assert!(range_refs.contains(&format!("row:{off}/{off_today}")));
    assert!(range_refs.contains(&format!("row:{events}/{past_event}")));
    assert!(range_refs.contains(&format!("mention:{page}/mention")));
    assert!(
        refs(&run(
            &v,
            "agenda",
            json!({"view":"today","from":tomorrow.to_string()})
        ))
        .is_empty(),
        "an inverted intersection stays empty even for spanning events"
    );
}

#[test]
fn agenda_and_overdue_return_stable_todo_and_row_refs_that_mutate() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Shopping".into()).unwrap();
    todo(&v, &page, "buy", Some("2000-01-01"));
    let db = database(&v);
    let row = v.create_page(Some(db.clone()), "Row task".into()).unwrap();
    v.set_property(
        row.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: utc_day("2000-01-01"),
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();
    let block_ref = format!("block:{page}/buy");
    let row_ref = format!("row:{db}/{row}");
    let agenda = run(
        &v,
        "agenda",
        json!({"from":"2000-01-01", "to":"2000-01-02"}),
    );
    assert_eq!(total(&agenda), 2, "{agenda}");
    let block = line(&agenda, &block_ref);
    assert!(block.contains("Buy milk"), "{block}");
    assert!(block.contains(" · todo · due 2000-01-01 · "), "{block}");
    assert!(block.contains("| [ ] "), "{block}");
    assert!(agenda.contains("- **Shopping** "), "{agenda}");
    assert!(line(&agenda, &row_ref).contains(" · row · "), "{agenda}");
    // The overdue tool lists todos and tasks; rows come through agenda kinds.
    let overdue = run(&v, "overdue", json!({}));
    assert_eq!(refs(&overdue), vec![block_ref.clone()], "{overdue}");
    let overdue_rows = run(&v, "agenda", json!({"view":"overdue","kinds":["row"]}));
    assert_eq!(refs(&overdue_rows), vec![row_ref.clone()], "{overdue_rows}");

    run(&v, "set_done", json!({"item":block_ref,"done":true}));
    run(&v, "set_done", json!({"item":row_ref,"done":true}));
    assert_eq!(total(&run(&v, "overdue", json!({}))), 0);
    let done = run(
        &v,
        "agenda",
        json!({"from":"2000-01-01", "to":"2000-01-02"}),
    );
    assert!(
        item_lines(&done).iter().all(|l| l.contains("| [x] ")),
        "{done}"
    );
    run(&v, "set_done", json!({"item":row_ref,"done":false}));
    assert_eq!(total(&run(&v, "overdue", json!({}))), 0);
    let rows = run(&v, "agenda", json!({"view":"overdue","kinds":["row"]}));
    assert_eq!(refs(&rows), vec![row_ref.clone()], "{rows}");

    run(
        &v,
        "reschedule",
        json!({"item":block_ref,"date":"2000-01-03"}),
    );
    run(
        &v,
        "reschedule",
        json!({"item":row_ref,"date":"2000-01-03"}),
    );
    assert_eq!(
        total(&run(
            &v,
            "agenda",
            json!({"from":"2000-01-01", "to":"2000-01-02"})
        )),
        0
    );
    let moved = run(
        &v,
        "agenda",
        json!({"from":"2000-01-03", "to":"2000-01-04"}),
    );
    assert_eq!(total(&moved), 2, "{moved}");
    assert!(
        item_lines(&moved)
            .iter()
            .all(|l| l.contains(" due 2000-01-03 · ref ")),
        "{moved}"
    );
}

#[test]
fn local_time_reschedule_is_an_instant_while_all_day_stays_utc_calendar() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Timed".into()).unwrap();
    todo(&v, &page, "task", None);
    let db = database(&v);
    let row = v.create_page(Some(db.clone()), "Timed row".into()).unwrap();
    let refs = [format!("block:{page}/task"), format!("row:{db}/{row}")];
    let date = NaiveDate::from_ymd_opt(2030, 1, 15).unwrap();
    let local_stamp = Local
        .from_local_datetime(&date.and_hms_opt(0, 15, 0).unwrap())
        .single()
        .unwrap()
        .timestamp_millis();
    for item in &refs {
        run(
            &v,
            "reschedule",
            json!({"item":item,"date":"2030-01-15","time":"00:15"}),
        );
    }
    v.flush().unwrap();
    let core = v.agenda(local_stamp - 1, local_stamp + 1, 0, 1).unwrap();
    assert_eq!(core.len(), 2);
    assert!(core.iter().all(|i| i.start_ms == local_stamp && i.has_time));
    let agenda = run(&v, "agenda", json!({"from":"2030-01-15","to":"2030-01-16"}));
    assert_eq!(total(&agenda), 2, "{agenda}");
    assert!(
        item_lines(&agenda)
            .iter()
            .all(|l| l.contains("due 2030-01-15 00:15 · ")),
        "{agenda}"
    );
    assert_eq!(
        total(&run(
            &v,
            "agenda",
            json!({"from":"2030-01-14","to":"2030-01-15"})
        )),
        0
    );
    for item in &refs {
        run(&v, "reschedule", json!({"item":item,"date":"2030-01-15"}));
    }
    v.flush().unwrap();
    let core = v
        .agenda(0, 1, utc_day("2030-01-15"), utc_day("2030-01-16"))
        .unwrap();
    assert_eq!(core.len(), 2);
    assert!(core
        .iter()
        .all(|i| i.start_ms == utc_day("2030-01-15") && !i.has_time));
    assert_eq!(
        Utc.timestamp_millis_opt(local_stamp)
            .single()
            .unwrap()
            .timestamp_millis(),
        local_stamp
    );
}

#[test]
fn daily_note_seeds_tasks_and_log_and_preserves_added_content() {
    let (_temp, v) = temp();
    assert!(
        vault_tools::resolve_daily_note(&v, &json!({"date":"2026-10-09"}))
            .unwrap()
            .is_none()
    );
    let first = run_json(&v, "daily_note", json!({"date":"2026-10-09"}));
    assert_eq!(first["title"], "Friday 9 October 2026");
    assert_eq!(first["created"], true);
    let id = first["id"].as_str().unwrap();
    let note = v.load_page(id.into()).unwrap();
    assert_eq!(
        note.blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Tasks", "Log"]
    );
    call(
        &v,
        "append_blocks",
        &json!({"page":id,"markdown":"A saved thought."}),
    )
    .unwrap();
    let second = run_json(&v, "daily_note", json!({"date":"2026-10-09"}));
    assert_eq!(first["id"], second["id"]);
    assert_eq!(second["created"], false);
    assert_eq!(
        vault_tools::resolve_daily_note(&v, &json!({"date":"2026-10-09"}))
            .unwrap()
            .as_deref(),
        Some(id)
    );
    assert!(v
        .load_page(id.into())
        .unwrap()
        .blocks
        .iter()
        .any(|b| b.text == "A saved thought."));
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(v.list_children(Some(journal), 0, 100).unwrap().len(), 1);
}

#[test]
fn rejects_bad_dates_times_ranges_and_mismatched_row_membership_without_mutation() {
    let (_temp, v) = temp();
    let page = v.create_page(None, "Validation".into()).unwrap();
    todo(&v, &page, "todo", None);
    let item = format!("block:{page}/todo");
    let original = v.load_page(page.clone()).unwrap().blocks;
    for args in [
        json!({"item":item,"date":"2026-02-30"}),
        json!({"item":item,"date":"2026-10-09","time":"24:01"}),
        json!({"item":item,"date":"2026-10-09","time":"9:30"}),
        json!({"item":item,"date":"2026-10-09","time":42}),
    ] {
        assert!(call(&v, "reschedule", &args).is_err());
    }
    assert_eq!(v.load_page(page.clone()).unwrap().blocks, original);
    assert!(call(&v, "set_done", &json!({"item":item,"done":"yes"})).is_err());
    // tasks is retired; agenda has no 62-day cap, so any span is accepted.
    assert!(call(&v, "tasks", &json!({"from":"2026-01-01","to":"2026-03-04"})).is_err());
    assert!(call(
        &v,
        "agenda",
        &json!({"from":"2026-01-01","to":"2026-03-05"})
    )
    .is_ok());
    assert!(call(
        &v,
        "agenda",
        &json!({"from":"2026-01-01","to":"2026-01-01"})
    )
    .is_err());
    assert!(call(
        &v,
        "agenda",
        &json!({"from":"2026-02-30","to":"2026-03-02"})
    )
    .is_err());
    let db = database(&v);
    assert!(vault_tools::resolve_agenda_item(&v, &format!("row:{db}/{page}")).is_err());
    assert!(vault_tools::resolve_agenda_item(&v, "block:/todo").is_err());
    v.delete_page(page).unwrap();
    assert!(call(&v, "reschedule", &json!({"item":item,"date":"2026-10-09"})).is_err());
}

#[test]
fn overdue_keeps_timed_tasks_due_earlier_today_and_all_day_tasks_until_tomorrow() {
    let (_temp, v) = temp();
    let today = Local::now().date_naive();
    let tomorrow = today.succ_opt().unwrap();
    let local_midnight = |date: NaiveDate| {
        Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis()
    };
    let db = database(&v);
    let timed = v
        .create_page(Some(db.clone()), "Timed today".into())
        .unwrap();
    v.set_property(
        timed.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: local_midnight(today),
            end_ms: None,
            has_time: true,
        },
    )
    .unwrap();
    let future = v
        .create_page(Some(db.clone()), "Timed tomorrow".into())
        .unwrap();
    v.set_property(
        future,
        "date".into(),
        PropValue::Date {
            start_ms: local_midnight(tomorrow),
            end_ms: None,
            has_time: true,
        },
    )
    .unwrap();
    let page = v.create_page(None, "All day".into()).unwrap();
    let today_text = today.format("%Y-%m-%d").to_string();
    todo(&v, &page, "today", Some(today_text.as_str()));
    // The all-day todo due today is not overdue until tomorrow.
    let overdue = run(&v, "overdue", json!({}));
    assert_eq!(total(&overdue), 0, "{overdue}");
    // The overdue tool lists todos and tasks only; agenda's row kind shows
    // the timed row due earlier today.
    let rows = run(&v, "agenda", json!({"view":"overdue","kinds":["row"]}));
    assert_eq!(
        refs(&rows),
        vec![format!("row:{db}/{timed}")],
        "only the timed task due earlier today is overdue: {rows}"
    );
    assert_eq!(total(&rows), 1, "{rows}");
    assert!(line(&rows, &format!("row:{db}/{timed}")).contains("00:00"));
}
