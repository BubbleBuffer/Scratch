//! Organizer queries share one derived index for edits, rebuilds and sync.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use rusqlite::types::Value;
use vault_core::*;

const DAY: i64 = 1_791_244_800_000; // 2026-10-06, midnight UTC.
const DAY_MS: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_agenda_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }

    fn sql(&self, sql: &str) -> Vec<Vec<Value>> {
        let conn = rusqlite::Connection::open(self.0.join(".vault/index.sqlite")).unwrap();
        let mut stmt = conn.prepare(sql).unwrap();
        let cols = stmt.column_count();
        stmt.query_map([], |row| (0..cols).map(|i| row.get(i)).collect())
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn def(key: &str, kind: PropKind) -> PropDef {
    PropDef {
        key: key.into(),
        name: key.into(),
        kind,
        options: Vec::new(),
        config: String::new(),
    }
}

fn status(key: &str) -> PropDef {
    PropDef {
        options: ["todo", "doing", "done"]
            .into_iter()
            .map(|id| SelectOption {
                id: id.into(),
                name: id.into(),
                color: String::new(),
            })
            .collect(),
        ..def(key, PropKind::Status)
    }
}

fn collection(v: &Vault, title: &str, schema: Vec<PropDef>) -> PageId {
    let page = v.create_page(None, title.into()).unwrap();
    v.set_schema(page.clone(), schema).unwrap();
    page
}

fn tasks(v: &Vault) -> PageId {
    collection(
        v,
        "Tasks",
        vec![def("date", PropKind::Date), status("status")],
    )
}

fn date(v: &Vault, page: &str, key: &str, start_ms: i64, end_ms: Option<i64>, timed: bool) {
    v.set_property(
        page.into(),
        key.into(),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time: timed,
        },
    )
    .unwrap();
}

fn row(v: &Vault, parent: &str, title: &str, when: i64) -> PageId {
    let page = v.create_page(Some(parent.into()), title.into()).unwrap();
    date(v, &page, "date", when, None, false);
    page
}

fn settings(v: &Vault, page: &str, role: AgendaRole, date_prop: Option<&str>) {
    v.set_agenda_settings(
        page.into(),
        AgendaSettings {
            role,
            date_prop: date_prop.map(Into::into),
        },
    )
    .unwrap();
}

fn all(v: &Vault) -> Vec<AgendaItem> {
    // Block-derived index updates use the existing background writer contract.
    v.flush().unwrap();
    v.agenda(
        DAY - 30 * DAY_MS,
        DAY + 30 * DAY_MS,
        DAY - 30 * DAY_MS,
        DAY + 30 * DAY_MS,
    )
    .unwrap()
}

fn item(v: &Vault, page: &str, block: Option<&str>) -> AgendaItem {
    all(v)
        .into_iter()
        .find(|item| item.page == page && item.block.as_deref() == block)
        .expect("agenda item")
}

fn todo(id: &str, checked: bool, dated: bool) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind: BlockKind::Todo,
        text: if dated {
            "Water plants @today"
        } else {
            "Water plants"
        }
        .into(),
        marks: if dated {
            vec![Mark {
                start: 13,
                end: 19,
                kind: MarkKind::Date("2026-10-06".into()),
            }]
        } else {
            Vec::new()
        },
        checked,
    }
}

fn insert(v: &Vault, page: &str, block: NewBlock) {
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block,
        }],
    )
    .unwrap();
}

fn property(v: &Vault, page: &str, key: &str) -> PropValue {
    v.load_page(page.into())
        .unwrap()
        .properties
        .into_iter()
        .find(|p| p.key == key)
        .expect("property")
        .value
}

#[test]
fn automatic_roles_and_explicit_overrides_recompute_existing_rows() {
    let t = TempVault::new();
    let v = t.open();
    let task_collection = tasks(&v);
    let event_collection = collection(&v, "Events", vec![def("date", PropKind::Date)]);
    let off_collection = collection(&v, "Undated", vec![def("text", PropKind::Text)]);
    let task = row(&v, &task_collection, "Task", DAY);
    let event = row(&v, &event_collection, "Event", DAY);
    v.create_page(Some(off_collection.clone()), "No date".into())
        .unwrap();
    assert_eq!(all(&v).len(), 2);
    assert_eq!(item(&v, &task, None).kind, AgendaKind::Task);
    assert_eq!(item(&v, &event, None).kind, AgendaKind::Event);
    assert_eq!(
        v.agenda_settings(task_collection.clone()).unwrap().role,
        AgendaRole::Auto
    );

    settings(&v, &task_collection, AgendaRole::Off, None);
    assert_eq!(all(&v).len(), 1);
    settings(&v, &task_collection, AgendaRole::Events, None);
    assert_eq!(item(&v, &task, None).kind, AgendaKind::Event);
    settings(&v, &event_collection, AgendaRole::Tasks, None);
    assert_eq!(item(&v, &event, None).kind, AgendaKind::Task);
    settings(&v, &task_collection, AgendaRole::Auto, None);
    assert_eq!(item(&v, &task, None).kind, AgendaKind::Task);
    // Created and edited time alone do not count as the Date property.
    v.set_schema(
        off_collection,
        vec![def("created", PropKind::CreatedTime), status("state")],
    )
    .unwrap();
    assert_eq!(all(&v).len(), 2);
}

#[test]
fn schema_order_date_override_and_schema_changes_recompute() {
    let t = TempVault::new();
    let v = t.open();
    let schema = vec![
        def("date", PropKind::Date),
        def("later", PropKind::Date),
        def("done", PropKind::Checkbox),
    ];
    let c = collection(&v, "Two dates", schema.clone());
    let p = row(&v, &c, "Row", DAY);
    date(&v, &p, "later", DAY + 3 * DAY_MS, None, false);
    assert_eq!(item(&v, &p, None).start_ms, DAY);
    settings(&v, &c, AgendaRole::Auto, Some("later"));
    assert_eq!(item(&v, &p, None).start_ms, DAY + 3 * DAY_MS);
    settings(&v, &c, AgendaRole::Auto, Some("removed"));
    assert_eq!(item(&v, &p, None).start_ms, DAY);
    settings(&v, &c, AgendaRole::Auto, Some("later"));
    v.set_schema(c.clone(), vec![schema[0].clone(), schema[2].clone()])
        .unwrap();
    assert_eq!(item(&v, &p, None).start_ms, DAY);
    v.set_schema(c.clone(), vec![schema[0].clone()]).unwrap();
    assert_eq!(item(&v, &p, None).kind, AgendaKind::Event);
    assert!(v.in_motion(10).unwrap().iter().all(|p| p.page != c));
    v.set_schema(c, vec![def("text", PropKind::Text)]).unwrap();
    assert!(all(&v).is_empty());
}

#[test]
fn duplicated_collection_keeps_agenda_role_and_selected_date() {
    let t = TempVault::new();
    let v = t.open();
    let c = collection(
        &v,
        "Events override",
        vec![
            def("date", PropKind::Date),
            def("chosen", PropKind::Date),
            status("status"),
        ],
    );
    let p = row(&v, &c, "Entry", DAY);
    date(&v, &p, "chosen", DAY + DAY_MS, None, false);
    settings(&v, &c, AgendaRole::Events, Some("chosen"));
    let copied = v.duplicate_page(c.clone()).unwrap();
    assert_eq!(
        v.agenda_settings(copied.clone()).unwrap(),
        v.agenda_settings(c).unwrap()
    );
    let entries: Vec<_> = all(&v)
        .into_iter()
        .filter(|a| a.collection.as_deref() == Some(copied.as_str()))
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, AgendaKind::Event);
    assert_eq!(entries[0].start_ms, DAY + DAY_MS);
}

#[test]
fn status_groups_and_checkbox_done_properties_round_trip() {
    let t = TempVault::new();
    let v = t.open();
    let c = collection(
        &v,
        "Tasks",
        vec![
            def("date", PropKind::Date),
            status("status"),
            def("done", PropKind::Checkbox),
        ],
    );
    let p = row(&v, &c, "Task", DAY);
    for (state, done) in [("todo", false), ("doing", false), ("done", true)] {
        v.set_property(p.clone(), "status".into(), PropValue::Status(state.into()))
            .unwrap();
        v.set_property(p.clone(), "done".into(), PropValue::Checkbox(!done))
            .unwrap();
        assert_eq!(
            item(&v, &p, None).done,
            done,
            "Status takes priority over Checkbox"
        );
    }
    v.set_agenda_done(p.clone(), None, false).unwrap();
    assert_eq!(property(&v, &p, "status"), PropValue::Status("todo".into()));
    v.set_agenda_done(p.clone(), None, true).unwrap();
    assert_eq!(property(&v, &p, "status"), PropValue::Status("done".into()));

    let mut configured = status("status");
    configured.config =
        r#"{"groups":{"todo":"complete","doing":"todo","done":"inProgress"}}"#.into();
    v.set_schema(c.clone(), vec![def("date", PropKind::Date), configured])
        .unwrap();
    assert!(!item(&v, &p, None).done);
    v.set_agenda_done(p.clone(), None, true).unwrap();
    assert_eq!(property(&v, &p, "status"), PropValue::Status("todo".into()));
    assert!(item(&v, &p, None).done);
    v.set_agenda_done(p.clone(), None, false).unwrap();
    assert_eq!(
        property(&v, &p, "status"),
        PropValue::Status("doing".into())
    );

    v.set_schema(
        c.clone(),
        vec![def("date", PropKind::Date), def("done", PropKind::Checkbox)],
    )
    .unwrap();
    v.set_agenda_done(p.clone(), None, true).unwrap();
    assert_eq!(property(&v, &p, "done"), PropValue::Checkbox(true));
    assert!(item(&v, &p, None).done);
    v.set_agenda_done(p.clone(), None, false).unwrap();
    assert!(!item(&v, &p, None).done);
    settings(&v, &c, AgendaRole::Events, None);
    assert!(!item(&v, &p, None).done);
    assert!(v.set_agenda_done(p, None, true).is_err());
}

#[test]
fn dated_todos_strip_mentions_move_on_edit_and_round_trip() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Checklist".into()).unwrap();
    insert(&v, &p, todo("dated", false, true));
    insert(&v, &p, todo("plain", false, false));
    insert(
        &v,
        &p,
        NewBlock {
            kind: BlockKind::Paragraph,
            ..todo("paragraph", false, true)
        },
    );
    assert_eq!(all(&v).len(), 1);
    let a = item(&v, &p, Some("dated"));
    assert_eq!(
        (a.title.as_str(), a.start_ms, a.end_ms, a.has_time, a.done),
        ("Water plants", DAY, None, false, false)
    );
    assert_eq!(a.kind, AgendaKind::Task);
    v.set_agenda_done(p.clone(), Some("dated".into()), true)
        .unwrap();
    assert!(item(&v, &p, Some("dated")).done);
    assert!(
        v.load_page(p.clone())
            .unwrap()
            .blocks
            .iter()
            .find(|b| b.id == "dated")
            .unwrap()
            .checked
    );
    v.set_agenda_done(p.clone(), Some("dated".into()), false)
        .unwrap();
    assert!(!item(&v, &p, Some("dated")).done);
    assert!(v
        .set_agenda_done(p.clone(), Some("plain".into()), true)
        .is_err());
    assert!(v
        .set_agenda_done(p.clone(), Some("paragraph".into()), true)
        .is_err());

    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "dated".into(),
            text: "Water plants @tomorrow".into(),
            marks: vec![Mark {
                start: 13,
                end: 22,
                kind: MarkKind::Date("2026-10-07".into()),
            }],
        }],
    )
    .unwrap();
    assert_eq!(item(&v, &p, Some("dated")).start_ms, DAY + DAY_MS);
    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "dated".into(),
            text: "Water plants @bad".into(),
            marks: vec![Mark {
                start: 13,
                end: 17,
                kind: MarkKind::Date("2026-02-30".into()),
            }],
        }],
    )
    .unwrap();
    assert!(all(&v).is_empty());
}

#[test]
fn nested_todos_and_first_valid_date_mark() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Nested".into()).unwrap();
    insert(
        &v,
        &p,
        NewBlock {
            kind: BlockKind::Heading2,
            ..todo("heading", false, false)
        },
    );
    v.apply(
        p.clone(),
        vec![BlockOp::Insert {
            parent: Some("heading".into()),
            after: None,
            block: NewBlock {
                id: "child".into(),
                kind: BlockKind::Todo,
                text: "@bad Buy milk @today @later".into(),
                checked: true,
                marks: vec![
                    Mark {
                        start: 0,
                        end: 4,
                        kind: MarkKind::Date("invalid".into()),
                    },
                    Mark {
                        start: 14,
                        end: 20,
                        kind: MarkKind::Date("2026-10-06".into()),
                    },
                    Mark {
                        start: 21,
                        end: 27,
                        kind: MarkKind::Date("2026-10-07".into()),
                    },
                ],
            },
        }],
    )
    .unwrap();
    let a = item(&v, &p, Some("child"));
    assert_eq!(a.start_ms, DAY);
    assert!(a.done);
    assert!(!a.title.contains("@today"));
}

#[test]
fn ranges_use_separate_timed_and_day_bounds_and_include_done() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let day = row(&v, &c, "Whole day", DAY);
    let range = row(&v, &c, "Range", DAY - 2 * DAY_MS);
    date(&v, &range, "date", DAY - 2 * DAY_MS, Some(DAY), false);
    let timed = row(&v, &c, "Timed", DAY);
    date(&v, &timed, "date", DAY - HOUR, Some(DAY + HOUR), true);
    let boundary = row(&v, &c, "At to", DAY + DAY_MS);
    let expired = row(&v, &c, "Before from", DAY - DAY_MS);
    v.set_agenda_done(day.clone(), None, true).unwrap();
    let agenda = v.agenda(DAY, DAY + DAY_MS, DAY, DAY + DAY_MS).unwrap();
    assert_eq!(
        agenda.iter().map(|a| a.page.as_str()).collect::<Vec<_>>(),
        vec![range.as_str(), day.as_str(), timed.as_str()]
    );
    assert!(agenda.iter().find(|a| a.page == day).unwrap().done);
    assert!(agenda
        .iter()
        .all(|a| a.page != boundary && a.page != expired));
    // Bounds are independently supplied by the app: changing only the timed
    // bounds must leave both whole-day entries in place.
    let shifted = v
        .agenda(DAY + 2 * HOUR, DAY + DAY_MS, DAY, DAY + DAY_MS)
        .unwrap();
    assert_eq!(shifted.len(), 2);
    assert!(shifted.iter().all(|a| !a.has_time));
    let only_timed = v
        .agenda(DAY, DAY + DAY_MS, DAY + DAY_MS, DAY + 2 * DAY_MS)
        .unwrap();
    assert!(only_timed.iter().any(|a| a.page == timed));
    assert!(only_timed.iter().all(|a| a.page != day && a.page != range));
}

#[test]
fn overdue_is_undone_tasks_by_matching_end_bound_oldest_first() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let oldest = row(&v, &c, "Oldest", DAY - 3 * DAY_MS);
    let range = row(&v, &c, "Ended yesterday", DAY - 2 * DAY_MS);
    date(
        &v,
        &range,
        "date",
        DAY - 2 * DAY_MS,
        Some(DAY - DAY_MS),
        false,
    );
    let ongoing = row(&v, &c, "Ends today", DAY - DAY_MS);
    date(&v, &ongoing, "date", DAY - DAY_MS, Some(DAY), false);
    let complete = row(&v, &c, "Complete", DAY - 4 * DAY_MS);
    v.set_agenda_done(complete, None, true).unwrap();
    let timed = row(&v, &c, "Timed", DAY);
    date(&v, &timed, "date", DAY - HOUR, None, true);
    let events = collection(&v, "Events", vec![def("date", PropKind::Date)]);
    row(&v, &events, "Old event", DAY - 7 * DAY_MS);
    let overdue = v.overdue(DAY, DAY, 50).unwrap();
    assert_eq!(
        overdue.iter().map(|a| a.page.as_str()).collect::<Vec<_>>(),
        vec![oldest.as_str(), range.as_str(), timed.as_str()]
    );
    assert!(overdue
        .iter()
        .all(|a| a.kind == AgendaKind::Task && !a.done && a.page != ongoing));
    assert_eq!(v.overdue(DAY - 2 * HOUR, DAY, 50).unwrap().len(), 2);
    assert_eq!(v.overdue(DAY, DAY - 3 * DAY_MS, 50).unwrap().len(), 1);
    assert_eq!(v.overdue_count(DAY, DAY).unwrap(), 3);
    assert_eq!(v.overdue_count(DAY - 2 * HOUR, DAY).unwrap(), 2);
    assert_eq!(v.overdue_count(DAY, DAY - 3 * DAY_MS).unwrap(), 1);
    assert!(v.overdue(DAY, DAY, 0).unwrap().is_empty());
    assert_eq!(v.overdue(DAY, DAY, 1).unwrap()[0].page, oldest);
}

#[test]
fn overdue_cap_and_count_share_live_membership_for_rows_and_blocks() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    for n in 0..55 {
        row(&v, &c, &format!("Task {n:02}"), DAY - DAY_MS);
    }
    let done = row(&v, &c, "Done", DAY - DAY_MS);
    v.set_agenda_done(done, None, true).unwrap();
    let trashed = row(&v, &c, "Trashed", DAY - DAY_MS);
    v.delete_page(trashed).unwrap();
    let template = v.create_template(c.clone(), "Template".into()).unwrap();
    date(&v, &template, "date", DAY - DAY_MS, None, false);
    let page = v.create_page(None, "Checklist".into()).unwrap();
    insert(&v, &page, todo("dated", false, true));
    insert(&v, &page, todo("checked", true, true));
    v.flush().unwrap();
    let before = DAY + DAY_MS;
    let all = v.overdue(before, before, u32::MAX).unwrap();
    let capped = v.overdue(before, before, 50).unwrap();
    assert_eq!(all.len(), 56);
    assert_eq!(capped.len(), 50);
    assert_eq!(capped, all[..50]);
    assert_eq!(v.overdue_count(before, before).unwrap(), 56);
    assert!(all.iter().any(|a| a.block.as_deref() == Some("dated")));
    v.delete_page(c).unwrap();
    assert_eq!(v.overdue_count(before, before).unwrap(), 1);
    v.delete_page(page).unwrap();
    assert_eq!(v.overdue_count(before, before).unwrap(), 0);
    assert!(v.overdue(before, before, 50).unwrap().is_empty());
}

#[test]
fn done_since_includes_yesterdays_tasks_completed_today_and_removes_undo() {
    // The clock is process-wide, so isolate this case from parallel tests.
    const CHILD: &str = "NOTES_DONE_SINCE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "done_since_includes_yesterdays_tasks_completed_today_and_removes_undo",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    Vault::set_test_clock(Some(DAY - DAY_MS + 11 * HOUR));
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let due_yesterday = row(&v, &c, "Finish today", DAY - DAY_MS);
    let done_yesterday = row(&v, &c, "Finished yesterday", DAY - DAY_MS);
    v.set_agenda_done(done_yesterday.clone(), None, true)
        .unwrap();
    let page = v.create_page(None, "Checklist".into()).unwrap();
    let mut block = todo("yesterday", false, true);
    block.marks[0].kind = MarkKind::Date("2026-10-05".into());
    insert(&v, &page, block);
    v.flush().unwrap();
    drop(v);

    Vault::set_test_clock(Some(DAY + 11 * HOUR));
    let v = t.open();
    assert!(v.done_since(DAY, DAY, DAY, 50).unwrap().is_empty());
    v.set_agenda_done(due_yesterday.clone(), None, true)
        .unwrap();
    v.set_agenda_done(page.clone(), Some("yesterday".into()), true)
        .unwrap();
    let timed = row(&v, &c, "Timed yesterday", DAY - DAY_MS);
    date(&v, &timed, "date", DAY - HOUR, None, true);
    v.set_agenda_done(timed.clone(), None, true).unwrap();
    let today = row(&v, &c, "Due today", DAY);
    v.set_agenda_done(today, None, true).unwrap();
    let ongoing = row(&v, &c, "Ends today", DAY - DAY_MS);
    date(&v, &ongoing, "date", DAY - DAY_MS, Some(DAY), false);
    v.set_agenda_done(ongoing, None, true).unwrap();
    let events = collection(&v, "Events", vec![def("date", PropKind::Date)]);
    row(&v, &events, "Yesterday's event", DAY - DAY_MS);
    let trashed = row(&v, &c, "Trashed", DAY - DAY_MS);
    v.set_agenda_done(trashed.clone(), None, true).unwrap();
    v.delete_page(trashed).unwrap();
    let template = v.create_template(c, "Template".into()).unwrap();
    date(&v, &template, "date", DAY - DAY_MS, None, false);
    v.set_property(template, "status".into(), PropValue::Status("done".into()))
        .unwrap();
    v.flush().unwrap();

    let done = v.done_since(DAY, DAY, DAY, 50).unwrap();
    assert_eq!(done.len(), 3);
    assert!(done
        .iter()
        .all(|i| i.done && i.kind == AgendaKind::Task && i.updated_ms >= DAY));
    assert!(done.iter().any(|i| i.page == due_yesterday));
    assert!(done
        .iter()
        .any(|i| i.page == page && i.block.as_deref() == Some("yesterday")));
    assert!(done.iter().all(|i| i.page != done_yesterday));
    assert_eq!(v.done_since(DAY, DAY, DAY, 1).unwrap(), done[..1]);
    assert!(v.done_since(DAY, DAY, DAY, 0).unwrap().is_empty());
    assert_eq!(v.done_since(DAY - 2 * HOUR, DAY, DAY, 50).unwrap().len(), 2);
    assert_eq!(v.done_since(DAY, DAY - DAY_MS, DAY, 50).unwrap().len(), 1);
    assert!(v.done_since(DAY, DAY, DAY + DAY_MS, 50).unwrap().is_empty());
    assert_eq!(v.overdue_count(DAY, DAY).unwrap(), 0);

    v.set_agenda_done(due_yesterday, None, false).unwrap();
    v.set_agenda_done(page, Some("yesterday".into()), false)
        .unwrap();
    v.flush().unwrap();
    let remaining = v.done_since(DAY, DAY, DAY, 50).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].page, timed);
    assert_eq!(v.overdue_count(DAY, DAY).unwrap(), 2);
    // There is no completion timestamp: editing an already completed page
    // today intentionally brings it into this edit-time approximation.
    v.rename_page(done_yesterday.clone(), "Edited after completion".into())
        .unwrap();
    v.flush().unwrap();
    let remaining = v.done_since(DAY, DAY, DAY, 50).unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|i| i.page == done_yesterday));
    drop(v);
    let v = t.open();
    assert_eq!(v.done_since(DAY, DAY, DAY, 50).unwrap(), remaining);
    drop(v);
    Vault::set_test_clock(None);
}

#[test]
fn moving_rows_trash_and_templates_update_membership_and_spaces() {
    let t = TempVault::new();
    let v = t.open();
    let space = v.create_page(None, "Space".into()).unwrap();
    v.set_page_color(space.clone(), Some("sky".into())).unwrap();
    let folder = v.create_page(Some(space.clone()), "Folder".into()).unwrap();
    let c = v.create_page(Some(folder), "Tasks".into()).unwrap();
    v.set_schema(
        c.clone(),
        vec![def("date", PropKind::Date), status("status")],
    )
    .unwrap();
    let p = row(&v, &c, "Task", DAY);
    v.set_icon(p.clone(), Some("leaf".into())).unwrap();
    insert(&v, &p, todo("block", false, true));
    for a in all(&v) {
        assert_eq!(a.collection.as_deref(), Some(c.as_str()));
        assert_eq!(a.space.as_deref(), Some(space.as_str()));
        assert_eq!(a.space_color.as_deref(), Some("sky"));
        assert_eq!(a.icon.as_deref(), Some("leaf"));
    }
    let template = v.create_template(c.clone(), "Template".into()).unwrap();
    date(&v, &template, "date", DAY, None, false);
    insert(&v, &template, todo("template-block", false, true));
    assert_eq!(all(&v).len(), 2);
    v.move_page(p.clone(), Some(space.clone()), None).unwrap();
    let agenda = all(&v);
    assert_eq!(agenda.len(), 1);
    assert_eq!(agenda[0].block.as_deref(), Some("block"));
    assert!(agenda[0].collection.is_none());
    v.move_page(p.clone(), Some(c.clone()), None).unwrap();
    assert_eq!(all(&v).len(), 2);
    v.delete_page(space.clone()).unwrap();
    assert!(all(&v).is_empty());
    assert!(v
        .overdue(DAY + DAY_MS, DAY + DAY_MS, 50)
        .unwrap()
        .is_empty());
    assert!(v.in_motion(10).unwrap().is_empty());
    // Rows are retained for restoration, including dated template blocks.
    assert_eq!(
        t.sql("SELECT count(*) FROM agenda")[0][0],
        Value::Integer(4)
    );
    v.restore_page(space).unwrap();
    assert_eq!(all(&v).len(), 2);
    v.set_template(template.clone(), false).unwrap();
    assert_eq!(all(&v).len(), 4);
    v.set_template(template, true).unwrap();
    assert_eq!(all(&v).len(), 2);
}

#[test]
fn collection_recompute_emits_one_collection_change() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let rows: Vec<_> = (0..3)
        .map(|i| row(&v, &c, &format!("Task {i}"), DAY))
        .collect();
    let seq = v.poll_changes(0).seq;
    settings(&v, &c, AgendaRole::Off, None);
    let feed = v.poll_changes(seq);
    assert!(feed.changes.contains(&Change::PageChanged(c.clone())));
    assert!(rows
        .iter()
        .all(|row| !feed.changes.contains(&Change::PageChanged(row.clone()))));
    assert!(all(&v).is_empty());
    settings(&v, &c, AgendaRole::Auto, None);
    let seq = v.poll_changes(0).seq;
    let mut s = status("status");
    s.config = r#"{"groups":{"todo":"complete"}}"#.into();
    v.set_schema(c.clone(), vec![def("date", PropKind::Date), s])
        .unwrap();
    let feed = v.poll_changes(seq);
    assert!(feed.changes.contains(&Change::PageChanged(c)));
    assert!(rows
        .iter()
        .all(|row| !feed.changes.contains(&Change::PageChanged(row.clone()))));
}

#[test]
fn in_motion_counts_undated_rows_and_nested_checklists_and_orders_by_latest_edit() {
    let t = TempVault::new();
    let v = t.open();
    let checklist = v.create_page(None, "Checklist".into()).unwrap();
    insert(&v, &checklist, todo("open", false, false));
    v.apply(
        checklist.clone(),
        vec![BlockOp::Insert {
            parent: Some("open".into()),
            after: None,
            block: todo("done", true, false),
        }],
    )
    .unwrap();
    let finished = v.create_page(None, "Finished".into()).unwrap();
    insert(&v, &finished, todo("finished", true, false));
    let c = tasks(&v);
    let open = v
        .create_page(Some(c.clone()), "Undated open task".into())
        .unwrap();
    let done = v
        .create_page(Some(c.clone()), "Undated done task".into())
        .unwrap();
    v.set_property(
        done.clone(),
        "status".into(),
        PropValue::Status("done".into()),
    )
    .unwrap();
    let trashed = row(&v, &c, "Trashed", DAY);
    v.delete_page(trashed).unwrap();
    let template = v.create_template(c.clone(), "Template".into()).unwrap();
    insert(&v, &template, todo("template", false, false));
    // Make the most recent edit explicit instead of depending on test timing.
    std::thread::sleep(std::time::Duration::from_millis(2));
    v.rename_page(open.clone(), "Latest task".into()).unwrap();
    v.flush().unwrap();
    let progress = v.in_motion(10).unwrap();
    assert_eq!(progress.len(), 2);
    assert_eq!(progress[0].page, c);
    assert_eq!(progress[0].kind, ProgressKind::Tasks);
    assert_eq!((progress[0].done, progress[0].total), (1, 2));
    assert_eq!(
        progress[0].updated_ms,
        v.load_page(open).unwrap().updated_ms
    );
    assert_eq!(progress[1].page, checklist);
    assert_eq!(progress[1].kind, ProgressKind::Checklist);
    assert_eq!((progress[1].done, progress[1].total), (1, 2));
    assert_eq!(progress[1].space.as_deref(), Some(checklist.as_str()));
    assert_eq!(
        progress[1].space_color,
        v.load_page(checklist.clone()).unwrap().color
    );
    assert_eq!(v.in_motion(1).unwrap().len(), 1);
    assert!(v.in_motion(0).unwrap().is_empty());
    v.set_property(done, "status".into(), PropValue::Status("todo".into()))
        .unwrap();
    assert_eq!(v.in_motion(1).unwrap()[0].done, 0);
}

#[test]
fn daily_note_is_idempotent_starts_with_sections_and_finds_renamed_journal() {
    let t = TempVault::new();
    let v = t.open();
    assert!(v.daily_note(DAY, "Today".into(), false).unwrap().is_none());
    assert_eq!(v.page_count().unwrap(), 0);
    let note = v
        .daily_note(DAY, "Tuesday 6 October".into(), true)
        .unwrap()
        .unwrap();
    assert_eq!(
        v.daily_note(DAY, "A different title".into(), true).unwrap(),
        Some(note.clone())
    );
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    let j = v.load_page(journal.clone()).unwrap();
    assert_eq!(j.title, "Journal");
    assert_eq!(j.role.as_deref(), Some("journal"));
    assert!(j.parent.is_none() && j.is_database());
    assert_eq!(
        v.agenda_settings(journal.clone()).unwrap().role,
        AgendaRole::Off
    );
    let templates = v.list_templates(journal.clone()).unwrap();
    assert!(templates.is_empty());
    assert_eq!(v.default_template(journal.clone()).unwrap(), None);
    let page = v.load_page(note.clone()).unwrap();
    assert_eq!(page.title, "Tuesday 6 October");
    assert_eq!(page.parent.as_deref(), Some(journal.as_str()));
    assert_eq!(page.blocks.len(), 2);
    assert_eq!(page.blocks[0].kind, BlockKind::Toggle);
    assert_eq!(page.blocks[0].text, "Tasks");
    assert_eq!(page.blocks[1].kind, BlockKind::Toggle);
    assert_eq!(page.blocks[1].text, "Log");
    assert!(page.blocks.iter().all(|b| b.children.is_empty()));
    assert!(page.properties.iter().any(|p| p.value
        == PropValue::Date {
            start_ms: DAY,
            end_ms: None,
            has_time: false
        }));
    assert!(all(&v).is_empty());
    assert_eq!(
        v.daily_note(DAY, "Lookup".into(), false).unwrap(),
        Some(note)
    );
    assert!(v
        .daily_note(DAY + DAY_MS, "Tomorrow".into(), false)
        .unwrap()
        .is_none());
    v.rename_page(journal.clone(), "My diary".into()).unwrap();
    let timed = v
        .create_page_from(journal.clone(), "Timed date".into(), None)
        .unwrap();
    let date_key = page
        .properties
        .iter()
        .find(|p| matches!(p.value, PropValue::Date { .. }))
        .unwrap()
        .key
        .clone();
    date(&v, &timed, &date_key, DAY + DAY_MS, None, true);
    assert!(v
        .daily_note(DAY + DAY_MS, "Lookup".into(), false)
        .unwrap()
        .is_none());
    let next = v
        .daily_note(DAY + DAY_MS, "Wednesday".into(), true)
        .unwrap()
        .unwrap();
    assert_ne!(next, timed, "timed rows are not daily notes");
    assert_eq!(v.load_page(next).unwrap().parent, Some(journal.clone()));
    assert_eq!(
        v.find_by_role("journal".into()).unwrap(),
        Some(journal.clone())
    );
    v.delete_page(journal).unwrap();
    assert!(v.find_by_role("journal".into()).unwrap().is_none());
    assert!(v.daily_note(DAY, "Today".into(), false).unwrap().is_none());
}

#[test]
fn concurrent_daily_note_calls_create_one_journal_and_one_row() {
    let t = TempVault::new();
    let v = t.open();
    let barrier = std::sync::Barrier::new(6);
    let notes = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    v.daily_note(DAY, "Today".into(), true).unwrap().unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(notes.iter().all(|p| p == &notes[0]));
    assert_eq!(v.list_children(None, 0, 100).unwrap().len(), 1);
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(
        v.list_children(Some(journal.clone()), 0, 100)
            .unwrap()
            .len(),
        1
    );
    assert!(v.list_templates(journal).unwrap().is_empty());
}

#[test]
fn new_spaces_start_uncoloured_and_backfill_keeps_sidebar_order_and_meta() {
    let t = TempVault::new();
    let v = t.open();
    let cycle = ["apricot", "paw", "moss", "sun", "sky", "duvet"];
    let mut pages = Vec::new();
    for i in 0..8 {
        let p = v.create_page(None, format!("Space {i}")).unwrap();
        // New top-level pages are explicitly uncoloured, not cycled.
        assert_eq!(
            v.load_page(p.clone()).unwrap().color.as_deref(),
            Some("none")
        );
        pages.push(p);
    }
    let child = v
        .create_page(Some(pages[0].clone()), "Child".into())
        .unwrap();
    assert!(v.load_page(child).unwrap().color.is_none());
    assert!(v
        .set_page_color(pages[0].clone(), Some("purple".into()))
        .is_err());
    assert!(v
        .set_page_role(pages[0].clone(), Some("unknown".into()))
        .is_err());
    assert_eq!(
        v.load_page(pages[0].clone()).unwrap().color.as_deref(),
        Some("none")
    );
    v.set_page_color(pages[0].clone(), Some("duvet".into()))
        .unwrap();
    v.set_page_role(pages[0].clone(), Some("journal".into()))
        .unwrap();
    assert_eq!(
        v.find_by_role("journal".into()).unwrap(),
        Some(pages[0].clone())
    );
    for i in [1, 2, 4] {
        v.set_page_color(pages[i].clone(), None).unwrap();
    }
    v.delete_page(pages[2].clone()).unwrap();
    let sidebar = v.list_children(None, 0, 100).unwrap();
    assert_eq!(v.ensure_space_colors().unwrap(), 2);
    for id in [&pages[1], &pages[4]] {
        let pos = sidebar.iter().position(|p| &p.id == id).unwrap();
        assert_eq!(
            v.load_page(id.clone()).unwrap().color.as_deref(),
            Some(cycle[pos % 6])
        );
    }
    assert!(v.load_page(pages[2].clone()).unwrap().color.is_none());
    assert_eq!(
        v.load_page(pages[0].clone()).unwrap().color.as_deref(),
        Some("duvet")
    );
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    let ninth = v.create_page(None, "Ninth".into()).unwrap();
    assert_eq!(
        v.load_page(ninth.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    // Backfill leaves the new uncoloured page alone: "none" is not empty.
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    assert_eq!(v.load_page(ninth).unwrap().color.as_deref(), Some("none"));
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    let p = v.load_page(pages[0].clone()).unwrap();
    assert_eq!(
        (p.color.as_deref(), p.role.as_deref()),
        (Some("duvet"), Some("journal"))
    );
    v.set_page_role(pages[0].clone(), None).unwrap();
    assert!(v.find_by_role("journal".into()).unwrap().is_none());
}

#[test]
fn explicit_no_space_color_survives_backfill_reindex_reopen_and_sync() {
    let (t, remote) = (TempVault::new(), TempVault::new());
    let v = t.open();
    let none = v.create_page(None, "No colour".into()).unwrap();
    let unassigned = v.create_page(None, "Unassigned".into()).unwrap();
    let empty = v.create_page(None, "Legacy empty colour".into()).unwrap();
    v.set_page_color(none.clone(), Some("none".into())).unwrap();
    v.set_page_color(unassigned.clone(), None).unwrap();
    v.flush().unwrap();
    // Simulate an empty colour in a legacy index, distinct from explicit None.
    t.sql(&format!("UPDATE pages SET color = '' WHERE id = '{empty}'"));
    assert_eq!(v.ensure_space_colors().unwrap(), 2);
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    assert_eq!(
        v.load_page(none.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    assert_eq!(
        v.load_page(unassigned).unwrap().color.as_deref(),
        Some("paw")
    );
    assert_eq!(v.load_page(empty).unwrap().color.as_deref(), Some("moss"));
    v.reindex().unwrap();
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    assert_eq!(
        v.load_page(none.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    let b = remote.open();
    push(&v, &b);
    assert_eq!(b.ensure_space_colors().unwrap(), 0);
    assert_eq!(b.load_page(none).unwrap().color.as_deref(), Some("none"));
}

#[test]
fn imported_and_rescanned_spaces_start_uncoloured_and_survive_reindex_and_sync() {
    let (t, source, remote) = (TempVault::new(), TempVault::new(), TempVault::new());
    let v = t.open();
    let existing = v.create_page(None, "Existing space".into()).unwrap();
    // Clearing a colour is an explicit edit. Only ensure_space_colors should
    // later backfill it; loading documents through sync/reindex must preserve it.
    v.set_page_color(existing.clone(), None).unwrap();
    fs::create_dir_all(&source.0).unwrap();
    fs::write(source.0.join("Imported.md"), "An imported note.\n").unwrap();
    let report = v
        .import_markdown(source.0.to_string_lossy().into_owned(), None)
        .unwrap();
    assert_eq!(report.pages, 1);
    assert_eq!(report.top_pages.len(), 1);
    let imported = report.top_pages[0].clone();
    assert_eq!(
        v.load_page(imported.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    let child_report = v
        .import_markdown(
            source.0.to_string_lossy().into_owned(),
            Some(existing.clone()),
        )
        .unwrap();
    assert!(v
        .load_page(child_report.top_pages[0].clone())
        .unwrap()
        .color
        .is_none());
    v.flush().unwrap();
    fs::write(t.0.join("pages/External.md"), "A new external note.\n").unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let external = v
        .list_children(None, 0, 100)
        .unwrap()
        .into_iter()
        .find(|p| p.title == "External")
        .unwrap()
        .id;
    assert_eq!(
        v.load_page(external.clone()).unwrap().color.as_deref(),
        Some("none")
    );

    v.reindex().unwrap();
    assert!(v.load_page(existing.clone()).unwrap().color.is_none());
    assert_eq!(
        v.load_page(imported.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    assert_eq!(
        v.load_page(external.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    let b = remote.open();
    push(&v, &b);
    assert!(b.load_page(existing).unwrap().color.is_none());
    assert_eq!(
        b.load_page(imported).unwrap().color.as_deref(),
        Some("none")
    );
    assert_eq!(
        b.load_page(external).unwrap().color.as_deref(),
        Some("none")
    );
    assert_eq!(b.ensure_space_colors().unwrap(), 1);
}

#[test]
fn duplicated_coloured_space_starts_uncoloured_and_colours_persist_on_reopen() {
    let t = TempVault::new();
    let v = t.open();
    let source = v.create_page(None, "Coloured space".into()).unwrap();
    v.set_page_color(source.clone(), Some("sky".into()))
        .unwrap();
    let copy = v.duplicate_page(source.clone()).unwrap();
    assert_eq!(
        v.load_page(copy.clone()).unwrap().title,
        "Coloured space (copy)"
    );
    assert_eq!(
        v.load_page(copy.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    assert_eq!(
        v.load_page(source.clone()).unwrap().color.as_deref(),
        Some("sky")
    );
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    assert_eq!(
        v.load_page(copy.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    assert_eq!(v.load_page(source).unwrap().color.as_deref(), Some("sky"));
    assert_eq!(v.ensure_space_colors().unwrap(), 0);
}

#[test]
fn agenda_and_progress_work_in_uncoloured_and_cleared_spaces() {
    let t = TempVault::new();
    let v = t.open();
    let space = v.create_page(None, "Plain space".into()).unwrap();
    assert_eq!(
        v.load_page(space.clone()).unwrap().color.as_deref(),
        Some("none")
    );
    let c = v.create_page(Some(space.clone()), "Tasks".into()).unwrap();
    v.set_schema(
        c.clone(),
        vec![def("date", PropKind::Date), status("status")],
    )
    .unwrap();
    let p = row(&v, &c, "Task", DAY);
    insert(&v, &p, todo("block", false, true));
    insert(&v, &space, todo("open", false, false));
    insert(&v, &space, todo("done", true, false));
    for a in all(&v) {
        assert_eq!(a.space.as_deref(), Some(space.as_str()));
        assert_eq!(a.space_color.as_deref(), Some("none"));
    }
    let progress = v.in_motion(10).unwrap();
    let checklist = progress.iter().find(|p| p.page == space).unwrap();
    assert_eq!((checklist.done, checklist.total), (1, 2));
    assert_eq!(checklist.space_color.as_deref(), Some("none"));
    // A space with no colour at all (an explicit clear) still lists its rows.
    v.set_page_color(space.clone(), None).unwrap();
    assert!(all(&v).iter().all(|a| a.space_color.is_none()));
    let progress = v.in_motion(10).unwrap();
    let checklist = progress.iter().find(|p| p.page == space).unwrap();
    assert_eq!((checklist.done, checklist.total), (1, 2));
    assert!(checklist.space_color.is_none());
    assert!(all(&v).iter().any(|a| a.block.as_deref() == Some("block")));
}

fn agenda_table(t: &TempVault) -> Vec<Vec<Value>> {
    // Page rowids can change during reindex; compare their stable page ids.
    t.sql("SELECT p.id, a.block, a.kind, a.d0, a.d1, a.has_time, a.done, a.title FROM agenda a JOIN pages p ON p.rowid=a.page ORDER BY p.id, a.block")
}

#[test]
fn task_note_date_survives_database_metadata_refresh_and_agenda_off() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let p = row(&v, &c, "Two date sources", DAY);
    for (key, value) in [("date", "\"2026-10-07\""), ("calendar", "\"personal\"")] {
        v.type_data_set(p.clone(), "tasks".into(), vec![key.into()], value.into())
            .unwrap();
    }
    let items: Vec<_> = all(&v).into_iter().filter(|i| i.page == p).collect();
    assert_eq!(items.len(), 2);
    let note = items.iter().find(|i| i.calendar.is_some()).unwrap();
    assert_eq!(note.start_ms, DAY + DAY_MS);
    assert!(note.collection.is_none());
    let database = items.iter().find(|i| i.collection.is_some()).unwrap();
    assert_eq!(database.start_ms, DAY);
    assert!(database.calendar.is_none());

    // Completion targets the task note and leaves the database properties alone.
    v.set_agenda_done(p.clone(), None, true).unwrap();
    let items: Vec<_> = all(&v).into_iter().filter(|i| i.page == p).collect();
    assert!(items.iter().find(|i| i.calendar.is_some()).unwrap().done);
    assert!(!items.iter().find(|i| i.collection.is_some()).unwrap().done);

    // A synchronous metadata refresh must not delete the independent note row.
    date(&v, &p, "date", DAY + 2 * DAY_MS, None, false);
    settings(&v, &c, AgendaRole::Off, None);
    let items: Vec<_> = all(&v).into_iter().filter(|i| i.page == p).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].start_ms, DAY + DAY_MS);
    assert!(items[0].done);
    let before = items[0].clone();
    v.reindex().unwrap();
    assert_eq!(item(&v, &p, None), before);
}

#[test]
fn mention_titles_are_capped_by_characters_without_changing_todo_titles() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Long titles".into()).unwrap();
    for (id, kind) in [("mention", BlockKind::Paragraph), ("todo", BlockKind::Todo)] {
        insert(
            &v,
            &p,
            NewBlock {
                id: id.into(),
                kind,
                text: format!("{} @today", "猫".repeat(250)),
                marks: vec![Mark {
                    start: 251,
                    end: 257,
                    kind: MarkKind::Date("2026-10-06".into()),
                }],
                checked: false,
            },
        );
    }
    v.flush().unwrap();
    let items = v
        .calendar_items(DAY, DAY + DAY_MS, DAY, DAY + DAY_MS)
        .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items
            .iter()
            .find(|i| i.kind == AgendaKind::Mention)
            .unwrap()
            .title,
        "猫".repeat(200)
    );
    assert_eq!(
        items
            .iter()
            .find(|i| i.kind == AgendaKind::Task)
            .unwrap()
            .title,
        "猫".repeat(250)
    );
}

fn progress_columns(t: &TempVault) -> Vec<Vec<Value>> {
    t.sql("SELECT id, todos_total, todos_done, task_done FROM pages ORDER BY id")
}

#[test]
fn full_reindex_matches_incremental_tables_after_content_schema_moves_and_trash() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let p = row(&v, &c, "Task", DAY);
    insert(&v, &p, todo("dated", false, true));
    insert(&v, &p, todo("undated", true, false));
    v.set_agenda_done(p.clone(), None, true).unwrap();
    let other = row(&v, &c, "Other", DAY - DAY_MS);
    v.delete_page(other).unwrap();
    let template = v.create_template(c.clone(), "Template".into()).unwrap();
    insert(&v, &template, todo("template", false, true));
    let moved = row(&v, &c, "Moved", DAY + DAY_MS);
    v.move_page(moved, None, None).unwrap();
    settings(&v, &c, AgendaRole::Off, None);
    settings(&v, &c, AgendaRole::Auto, None);
    let agenda = all(&v);
    let rows = agenda_table(&t);
    let progress = progress_columns(&t);
    v.reindex().unwrap();
    assert_eq!(agenda_table(&t), rows);
    assert_eq!(progress_columns(&t), progress);
    assert_eq!(all(&v), agenda);
    v.flush().unwrap();
    drop(v);
    let reopened = t.open();
    assert_eq!(agenda_table(&t), rows);
    assert_eq!(progress_columns(&t), progress);
    assert_eq!(all(&reopened), agenda);
}

/// The same missing-version export/import helper used by `tests/sync.rs`.
fn push(from: &Vault, to: &Vault) -> SyncImport {
    to.import_purged(from.purged_pages()).unwrap();
    let have: HashMap<String, Vec<u8>> = to
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    let mut updates = Vec::new();
    for v in from.page_versions().unwrap() {
        let theirs = have.get(&v.page).cloned().unwrap_or_default();
        if !version_includes(&theirs, &v.version).unwrap() {
            updates.push(PageUpdate {
                bytes: from.export_updates(v.page.clone(), theirs).unwrap(),
                page: v.page,
            });
        }
    }
    to.import_updates_batch(updates).unwrap()
}

#[test]
fn sync_builds_identical_agenda_and_progress_from_documents() {
    let (ta, tb) = (TempVault::new(), TempVault::new());
    let (a, b) = (ta.open(), tb.open());
    let c = tasks(&a);
    a.set_page_color(c.clone(), Some("moss".into())).unwrap();
    let p = row(&a, &c, "Task", DAY);
    insert(&a, &p, todo("block", false, true));
    a.set_agenda_done(p.clone(), None, true).unwrap();
    let undated = a.create_page(Some(c.clone()), "Undated".into()).unwrap();
    insert(&a, &undated, todo("undated", false, false));
    a.daily_note(DAY, "Today".into(), true).unwrap();
    let journal = a.find_by_role("journal".into()).unwrap().unwrap();
    a.rename_page(journal.clone(), "Private diary".into())
        .unwrap();
    push(&a, &b);
    assert_eq!(all(&b), all(&a));
    assert_eq!(agenda_table(&tb), agenda_table(&ta));
    assert_eq!(progress_columns(&tb), progress_columns(&ta));
    assert_eq!(b.in_motion(10).unwrap(), a.in_motion(10).unwrap());

    assert_eq!(
        b.find_by_role("journal".into()).unwrap(),
        Some(journal.clone())
    );
    let remote_journal = b.load_page(journal.clone()).unwrap();
    let source_journal = a.load_page(journal).unwrap();
    assert_eq!(remote_journal.title, "Private diary");
    assert_eq!(remote_journal.color, source_journal.color);
    assert_eq!(remote_journal.role, source_journal.role);

    // Collection-only changes must recompute the receiver's existing rows.
    settings(&a, &c, AgendaRole::Off, None);
    push(&a, &b);
    assert_eq!(all(&b), all(&a));
    assert!(all(&b).iter().all(|item| item.block.is_some()));
    settings(&a, &c, AgendaRole::Auto, None);
    let mut configured = status("status");
    configured.config = r#"{"groups":{"done":"todo"}}"#.into();
    a.set_schema(c.clone(), vec![def("date", PropKind::Date), configured])
        .unwrap();
    push(&a, &b);
    assert!(!item(&b, &p, None).done);
    assert_eq!(all(&b), all(&a));
    assert_eq!(progress_columns(&tb), progress_columns(&ta));

    settings(&a, &c, AgendaRole::Events, None);
    a.rename_page(p.clone(), "Renamed event".into()).unwrap();
    a.set_agenda_done(p.clone(), Some("block".into()), true)
        .unwrap();
    push(&a, &b);
    assert_eq!(all(&b), all(&a));
    assert_eq!(agenda_table(&tb), agenda_table(&ta));
    assert_eq!(progress_columns(&tb), progress_columns(&ta));
    a.move_page(p, None, None).unwrap();
    push(&a, &b);
    assert_eq!(all(&b), all(&a));
    assert_eq!(b.in_motion(10).unwrap(), a.in_motion(10).unwrap());
}

#[test]
fn agenda_status_metadata_uses_labels_colors_and_configured_groups() {
    let t = TempVault::new();
    let v = t.open();
    let mut state = status("status");
    state.options = [
        ("todo", "Ready", "red"),
        ("doing", "Working", "blue"),
        ("done", "Finished", "green"),
    ]
    .into_iter()
    .map(|(id, name, color)| SelectOption {
        id: id.into(),
        name: name.into(),
        color: color.into(),
    })
    .collect();
    // The configured groups deliberately disagree with the option order.
    state.config = r#"{"groups":{"todo":"complete","doing":"todo","done":"inProgress"}}"#.into();
    let c = collection(
        &v,
        "Status tasks",
        vec![def("date", PropKind::Date), state.clone()],
    );
    for (option, label, color, group, done) in [
        ("todo", "Ready", "red", "complete", true),
        ("doing", "Working", "blue", "todo", false),
        ("done", "Finished", "green", "progress", false),
    ] {
        let p = row(&v, &c, label, DAY);
        v.set_property(p.clone(), "status".into(), PropValue::Status(option.into()))
            .unwrap();
        let a = item(&v, &p, None);
        assert_eq!(a.status.as_deref(), Some(label));
        assert_eq!(a.status_group.as_deref(), Some(group));
        assert_eq!(a.status_color.as_deref(), Some(color));
        assert_eq!(a.done, done);
    }
    let absent = row(&v, &c, "No status selected", DAY);
    let assert_null = |a: AgendaItem| {
        assert!(a.status.is_none());
        assert!(a.status_group.is_none());
        assert!(a.status_color.is_none());
    };
    assert_null(item(&v, &absent, None));
    // A dated block never inherits its parent database row's status.
    v.set_property(
        absent.clone(),
        "status".into(),
        PropValue::Status("todo".into()),
    )
    .unwrap();
    insert(&v, &absent, todo("dated-in-status-row", false, true));
    assert_null(item(&v, &absent, Some("dated-in-status-row")));
    let check_collection = collection(
        &v,
        "Checkbox tasks",
        vec![
            def("date", PropKind::Date),
            def("checked", PropKind::Checkbox),
        ],
    );
    let check = row(&v, &check_collection, "Checked without a status", DAY);
    v.set_property(check.clone(), "checked".into(), PropValue::Checkbox(true))
        .unwrap();
    let checked = item(&v, &check, None);
    assert!(checked.done);
    assert_null(checked);
    // Option renames, colours and group changes update without an index migration.
    state.options[0].name = "Delivered".into();
    state.options[0].color.clear();
    state.config = r#"{"groups":{"todo":"inProgress"}}"#.into();
    v.set_schema(c, vec![def("date", PropKind::Date), state])
        .unwrap();
    let a = item(&v, &absent, None);
    assert_eq!(a.status.as_deref(), Some("Delivered"));
    assert_eq!(a.status_group.as_deref(), Some("progress"));
    assert_eq!(a.status_color.as_deref(), Some("default"));
    assert!(!a.done);
    v.reindex().unwrap();
    assert_eq!(item(&v, &absent, None), a);
}

#[test]
fn foreground_properties_and_pending_writer_keep_agenda_in_sync() {
    let t = TempVault::new();
    let v = t.open();
    let c = tasks(&v);
    let pages: Vec<_> = (0..12)
        .map(|i| {
            let p = row(&v, &c, &format!("Task {i}"), DAY);
            insert(&v, &p, todo("dated", false, true));
            p
        })
        .collect();
    v.flush().unwrap();
    std::thread::scope(|threads| {
        let (start, pending) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let v = &v;
        threads.spawn(move || {
            for _ in 0..20 {
                if pending.recv().is_err() {
                    return;
                }
                // Flush uses the separate writer connection while the caller
                // below changes properties on the foreground connection.
                if finished.send(v.flush()).is_err() {
                    return;
                }
            }
        });
        for round in 0..20 {
            for p in &pages {
                v.apply(
                    p.clone(),
                    vec![BlockOp::SetChecked {
                        id: "dated".into(),
                        checked: round % 2 == 0,
                    }],
                )
                .unwrap();
            }
            start.send(()).unwrap();
            for (i, p) in pages.iter().enumerate() {
                let state = ["todo", "doing", "done"][(round + i) % 3];
                v.set_property(p.clone(), "status".into(), PropValue::Status(state.into()))
                    .unwrap();
                date(
                    v,
                    p,
                    "date",
                    DAY + (round % 2) as i64 * DAY_MS,
                    None,
                    false,
                );
            }
            done.recv().unwrap().unwrap();
        }
    });
    let agenda = all(&v);
    for p in &pages {
        let page = v.load_page(p.clone()).unwrap();
        let PropValue::Status(state) = property(&v, p, "status") else {
            panic!("status property");
        };
        let a = agenda
            .iter()
            .find(|a| a.page == *p && a.block.is_none())
            .unwrap();
        assert_eq!(a.status.as_deref(), Some(state.as_str()));
        let group = match state.as_str() {
            "done" => "complete",
            "doing" => "progress",
            _ => "todo",
        };
        assert_eq!(a.status_group.as_deref(), Some(group));
        assert_eq!(a.done, state == "done");
        let PropValue::Date { start_ms, .. } = property(&v, p, "date") else {
            panic!("date property");
        };
        assert_eq!(a.start_ms, start_ms);
        let block = page.blocks.iter().find(|b| b.id == "dated").unwrap();
        let b = agenda
            .iter()
            .find(|a| a.page == *p && a.block.as_deref() == Some("dated"))
            .unwrap();
        assert_eq!(b.done, block.checked);
        assert!(b.status.is_none());
    }
    let rows = agenda_table(&t);
    let progress = progress_columns(&t);
    v.reindex().unwrap();
    assert_eq!(all(&v), agenda);
    assert_eq!(agenda_table(&t), rows);
    assert_eq!(progress_columns(&t), progress);
}

#[test]
fn reschedule_first_nested_todo_date_preserves_unicode_marks_and_later_dates() {
    let t = TempVault::new();
    let v = t.open();
    let page = v.create_page(None, "Schedule".into()).unwrap();
    insert(
        &v,
        &page,
        NewBlock {
            id: "parent".into(),
            kind: BlockKind::Toggle,
            text: "Tasks".into(),
            marks: vec![],
            checked: false,
        },
    );
    v.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: Some("parent".into()),
            after: None,
            block: NewBlock {
                id: "nested".into(),
                kind: BlockKind::Todo,
                text: "🐈 café @first then @later".into(),
                checked: false,
                // Deliberately not ordered: first means first in the text.
                marks: vec![
                    Mark {
                        start: 19,
                        end: 25,
                        kind: MarkKind::Date("2026-10-08".into()),
                    },
                    Mark {
                        start: 0,
                        end: 6,
                        kind: MarkKind::Bold,
                    },
                    Mark {
                        start: 7,
                        end: 13,
                        kind: MarkKind::Date("2026-10-06".into()),
                    },
                ],
            },
        }],
    )
    .unwrap();
    v.reschedule_agenda(page.clone(), Some("nested".into()), DAY + HOUR, true)
        .unwrap();
    let data = v.load_page(page.clone()).unwrap();
    let b = &data.blocks[0].children[0];
    assert_eq!(b.text, "🐈 café @2026-10-06T01:00:00.000Z then @later");
    assert!(b
        .marks
        .iter()
        .any(|m| m.start == 0 && m.end == 6 && m.kind == MarkKind::Bold));
    let later = b
        .marks
        .iter()
        .find(|m| m.kind == MarkKind::Date("2026-10-08".into()))
        .unwrap();
    assert_eq!(
        b.text
            .chars()
            .skip(later.start as usize)
            .take((later.end - later.start) as usize)
            .collect::<String>(),
        "@later"
    );
    let a = item(&v, &page, Some("nested"));
    assert!(a.has_time);
    assert_eq!(a.start_ms, DAY + HOUR);
    v.set_agenda_done(page.clone(), Some("nested".into()), true)
        .unwrap();
    assert!(item(&v, &page, Some("nested")).done);
    v.reindex().unwrap();
    assert_eq!(item(&v, &page, Some("nested")).start_ms, DAY + HOUR);
    v.reschedule_agenda(page.clone(), Some("nested".into()), DAY + DAY_MS, false)
        .unwrap();
    let a = item(&v, &page, Some("nested"));
    assert!(!a.has_time);
    assert_eq!(a.start_ms, DAY + DAY_MS);
}

#[test]
fn reschedule_appends_date_and_uses_rows_configured_date_property() {
    let t = TempVault::new();
    let v = t.open();
    let page = v.create_page(None, "Undated".into()).unwrap();
    insert(
        &v,
        &page,
        NewBlock {
            id: "todo".into(),
            kind: BlockKind::Todo,
            text: "🐾 Plan".into(),
            marks: vec![],
            checked: false,
        },
    );
    v.reschedule_agenda(page.clone(), Some("todo".into()), DAY, false)
        .unwrap();
    let b = v.load_page(page.clone()).unwrap().blocks.remove(0);
    assert_eq!(b.text, "🐾 Plan @2026-10-06");
    assert_eq!(b.marks[0].start, 7);
    assert_eq!(item(&v, &page, Some("todo")).start_ms, DAY);

    let db = collection(
        &v,
        "Rows",
        vec![
            def("first", PropKind::Date),
            def("actual", PropKind::Date),
            def("done", PropKind::Checkbox),
        ],
    );
    settings(&v, &db, AgendaRole::Auto, Some("actual"));
    let row = v.create_page(Some(db), "Row".into()).unwrap();
    date(&v, &row, "first", DAY - DAY_MS, None, false);
    v.reschedule_agenda(row.clone(), None, DAY + 2 * HOUR, true)
        .unwrap();
    assert_eq!(
        property(&v, &row, "first"),
        PropValue::Date {
            start_ms: DAY - DAY_MS,
            end_ms: None,
            has_time: false
        }
    );
    assert_eq!(
        property(&v, &row, "actual"),
        PropValue::Date {
            start_ms: DAY + 2 * HOUR,
            end_ms: None,
            has_time: true
        }
    );
    assert_eq!(item(&v, &row, None).start_ms, DAY + 2 * HOUR);
    v.set_agenda_done(row.clone(), None, true).unwrap();
    assert!(item(&v, &row, None).done);
    assert!(v.reschedule_agenda(page, None, DAY, false).is_err());
    assert!(v.reschedule_agenda(row, None, DAY + 1, false).is_err());
}

fn task_fields(v: &Vault, page: &str, fields: serde_json::Value) {
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

#[test]
fn stale_dated_todo_completion_rejects_removed_date_but_tool_source_can_complete() {
    let t = TempVault::new();
    let v = t.open();
    let page = v.create_page(None, "Checklist".into()).unwrap();
    insert(&v, &page, todo("stale", false, true));
    let stale = item(&v, &page, Some("stale"));
    v.apply(
        page.clone(),
        vec![BlockOp::SetText {
            id: "stale".into(),
            text: "Water plants".into(),
            marks: vec![],
        }],
    )
    .unwrap();

    assert!(v
        .set_agenda_done(stale.page, stale.block, true)
        .is_err());
    assert!(!v.load_page(page.clone()).unwrap().blocks[0].checked);
    assert!(all(&v).is_empty());

    v.set_agenda_source_done(page.clone(), Some("stale".into()), "todo".into(), true)
        .unwrap();
    assert!(v.load_page(page.clone()).unwrap().blocks[0].checked);
    v.flush().unwrap();
    let entry = v.query_agenda().unwrap().remove(0);
    assert_eq!(entry.date_ms, None);
    assert!(entry.item.done);
}

#[test]
fn legacy_task_note_mutations_require_dates_and_keep_database_row_precedence() {
    let t = TempVault::new();
    let v = t.open();
    let note = v.create_page(None, "Task note".into()).unwrap();
    task_fields(&v, &note, serde_json::json!({"date":"2026-10-06", "status":"doing"}));
    assert_eq!(item(&v, &note, None).start_ms, DAY);
    task_fields(&v, &note, serde_json::json!({"date":null}));
    assert!(v.set_agenda_done(note.clone(), None, true).is_err());
    assert!(v.reschedule_agenda(note.clone(), None, DAY, false).is_err());
    let data: serde_json::Value =
        serde_json::from_str(&v.type_data(note.clone(), "tasks".into()).unwrap()).unwrap();
    assert_eq!(data["status"], "doing");
    assert!(data["date"].is_null());

    v.set_agenda_source_done(note.clone(), None, "task_note".into(), true)
        .unwrap();
    v.reschedule_agenda_source(note.clone(), None, "task_note".into(), DAY, false)
        .unwrap();
    let note_item = item(&v, &note, None);
    assert!(note_item.done);
    assert_eq!(note_item.start_ms, DAY);

    let db = tasks(&v);
    let page = row(&v, &db, "Row with undated note", DAY);
    task_fields(&v, &page, serde_json::json!({"date":"", "status":"doing"}));
    v.set_agenda_done(page.clone(), None, true).unwrap();
    v.reschedule_agenda(page.clone(), None, DAY + DAY_MS, false)
        .unwrap();
    assert_eq!(property(&v, &page, "status"), PropValue::Status("done".into()));
    assert_eq!(
        property(&v, &page, "date"),
        PropValue::Date {
            start_ms: DAY + DAY_MS,
            end_ms: None,
            has_time: false,
        }
    );
    let data: serde_json::Value =
        serde_json::from_str(&v.type_data(page, "tasks".into()).unwrap()).unwrap();
    assert_eq!(data["date"], "");
    assert_eq!(data["status"], "doing");
}

#[test]
fn undated_todos_and_task_notes_are_queryable_without_leaking_into_app_views() {
    let t = TempVault::new();
    let v = t.open();
    let db = tasks(&v);
    let dated = row(&v, &db, "Dated baseline", DAY - DAY_MS);
    insert(&v, &dated, todo("dated-open", false, true));
    insert(&v, &dated, todo("dated-done", true, true));
    v.flush().unwrap();
    let agenda_before = all(&v);
    let calendar_before = v
        .calendar_items(0, DAY + 2 * DAY_MS, 0, DAY + 2 * DAY_MS)
        .unwrap();
    let overdue_before = v.overdue(DAY + DAY_MS, DAY + DAY_MS, 100).unwrap();
    let done_before = v.done_since(DAY + DAY_MS, DAY + DAY_MS, 0, 100).unwrap();
    let overdue_count_before = v.overdue_count(DAY + DAY_MS, DAY + DAY_MS).unwrap();

    let category = v
        .create_calendar("Hidden tasks".into(), "sky".into(), false, false)
        .unwrap();
    let first = v.create_page(None, "First page".into()).unwrap();
    let second = v.create_page(None, "Second page".into()).unwrap();
    insert(&v, &first, todo("undated-open", false, false));
    insert(&v, &second, todo("undated-done", true, false));
    for (i, date_value) in [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!("")),
    ]
    .into_iter()
    .enumerate()
    {
        let page = v.create_page(None, format!("Undated note {i}")).unwrap();
        task_fields(
            &v,
            &page,
            serde_json::json!({"status":"doing", "calendar":category, "assignee":"agent"}),
        );
        if let Some(value) = date_value {
            task_fields(&v, &page, serde_json::json!({"date":value}));
        }
    }
    let undated_database_row = v
        .create_page(Some(db), "Undated database row".into())
        .unwrap();
    v.flush().unwrap();

    assert_eq!(all(&v), agenda_before);
    assert_eq!(
        v.calendar_items(0, DAY + 2 * DAY_MS, 0, DAY + 2 * DAY_MS)
            .unwrap(),
        calendar_before
    );
    assert_eq!(
        v.overdue(DAY + DAY_MS, DAY + DAY_MS, 100).unwrap(),
        overdue_before
    );
    assert_eq!(
        v.done_since(DAY + DAY_MS, DAY + DAY_MS, 0, 100).unwrap(),
        done_before
    );
    assert_eq!(
        v.overdue_count(DAY + DAY_MS, DAY + DAY_MS).unwrap(),
        overdue_count_before
    );
    let entries = v.query_agenda().unwrap();
    let undated: Vec<_> = entries
        .iter()
        .filter(|entry| entry.date_ms.is_none())
        .collect();
    assert_eq!(undated.len(), 5);
    assert_eq!(
        undated
            .iter()
            .filter(|entry| entry.source == "todo")
            .count(),
        2
    );
    assert_eq!(
        undated
            .iter()
            .filter(|entry| entry.source == "task_note")
            .count(),
        3
    );
    assert!(entries
        .iter()
        .all(|entry| entry.item.page != undated_database_row));
    for entry in undated.iter().filter(|entry| entry.source == "task_note") {
        assert_eq!(entry.calendar_name.as_deref(), Some("Hidden tasks"));
        assert_eq!(entry.item.status.as_deref(), Some("doing"));
        assert_eq!(entry.item.status_group.as_deref(), Some("progress"));
        assert_eq!(entry.item.assignee.as_deref(), Some("agent"));
    }
    assert_eq!(
        t.sql("SELECT count(*) FROM agenda WHERE d0 IS NULL")[0][0],
        Value::Integer(5)
    );
}

#[test]
fn undated_cache_survives_metadata_refresh_reindex_and_schema_rebuild() {
    let t = TempVault::new();
    let page;
    let rows;
    {
        let v = t.open();
        v.list_calendars().unwrap();
        let db = tasks(&v);
        page = v
            .create_page(Some(db.clone()), "Undated sources".into())
            .unwrap();
        insert(&v, &page, todo("undated", false, false));
        task_fields(
            &v,
            &page,
            serde_json::json!({"status":"doing", "assignee":"me", "date":null, "calendar":"personal"}),
        );
        v.flush().unwrap();
        // Date-property and collection settings write metadata without
        // replacing the separately derived task-note/block cache rows.
        date(&v, &page, "date", DAY, None, false);
        settings(&v, &db, AgendaRole::Off, None);
        v.set_icon(page.clone(), Some("leaf".into())).unwrap();
        v.flush().unwrap();
        let entries = v.query_agenda().unwrap();
        let note = entries
            .iter()
            .find(|entry| entry.source == "task_note")
            .unwrap();
        assert_eq!(note.date_ms, None);
        assert_eq!(note.item.assignee.as_deref(), Some("me"));
        assert_eq!(note.calendar_name.as_deref(), Some("Personal"));
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.item.page == page)
                .count(),
            3
        );
        rows = agenda_table(&t);
        v.reindex().unwrap();
        assert_eq!(agenda_table(&t), rows);
        v.flush().unwrap();
    }
    // Simulate an old derived cache. Opening rebuilds from existing documents;
    // no sync or persistent document migration is involved.
    rusqlite::Connection::open(t.0.join(".vault/index.sqlite"))
        .unwrap()
        .pragma_update(None, "user_version", 12)
        .unwrap();
    let v = t.open();
    assert_eq!(agenda_table(&t), rows);
    let entries = v.query_agenda().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.item.page == page && entry.date_ms.is_none())
            .count(),
        2
    );
    assert!(all(&v).is_empty());
}

#[test]
fn explicit_task_note_and_database_row_mutations_keep_their_sources_separate() {
    let t = TempVault::new();
    let v = t.open();
    let db = tasks(&v);
    let page = row(&v, &db, "Two sources", DAY);
    insert(&v, &page, todo("undated", false, false));
    task_fields(&v, &page, serde_json::json!({"status":"doing", "date":""}));
    v.flush().unwrap();

    v.set_agenda_source_done(page.clone(), Some("undated".into()), "todo".into(), true)
        .unwrap();
    v.set_agenda_source_done(page.clone(), None, "row".into(), true)
        .unwrap();
    assert_eq!(
        property(&v, &page, "status"),
        PropValue::Status("done".into())
    );
    let data: serde_json::Value =
        serde_json::from_str(&v.type_data(page.clone(), "tasks".into()).unwrap()).unwrap();
    assert_eq!(data["status"], "doing");
    v.set_agenda_source_done(page.clone(), None, "task_note".into(), true)
        .unwrap();
    v.reschedule_agenda_source(page.clone(), None, "task_note".into(), DAY + DAY_MS, false)
        .unwrap();
    assert_eq!(
        property(&v, &page, "date"),
        PropValue::Date {
            start_ms: DAY,
            end_ms: None,
            has_time: false
        }
    );
    v.reschedule_agenda_source(page.clone(), None, "row".into(), DAY + 2 * DAY_MS, false)
        .unwrap();
    v.flush().unwrap();
    let entries = v.query_agenda().unwrap();
    let note = entries
        .iter()
        .find(|entry| entry.source == "task_note")
        .unwrap();
    let row = entries.iter().find(|entry| entry.source == "row").unwrap();
    assert_eq!(note.date_ms, Some(DAY + DAY_MS));
    assert_eq!(row.date_ms, Some(DAY + 2 * DAY_MS));
    assert!(note.item.done && row.item.done);
    assert!(v
        .set_agenda_source_done(page.clone(), Some("undated".into()), "mention".into(), true)
        .is_err());
    assert!(v
        .reschedule_agenda_source(page, None, "todo".into(), DAY, false)
        .is_err());
}

#[test]
fn tool_agenda_is_read_only_and_keeps_calendar_source_and_today_eligibility() {
    let t = TempVault::new();
    let v = t.open();
    assert!(v.query_agenda().unwrap().is_empty());
    assert_eq!(v.find_by_role("calendars".into()).unwrap(), None);
    assert_eq!(v.page_count().unwrap(), 0);

    let hidden = v.create_calendar("Hidden".into(), "sky".into(), false, false).unwrap();
    let note = v.create_page(None, "Hidden dated task".into()).unwrap();
    task_fields(&v, &note, serde_json::json!({"date":"2026-10-06", "calendar":hidden}));
    let db = tasks(&v);
    let row_page = row(&v, &db, "Off row", DAY);
    settings(&v, &db, AgendaRole::Off, None);
    let daily = v.daily_note(DAY, "Today".into(), true).unwrap().unwrap();
    insert(&v, &daily, todo("journal-todo", false, true));
    let template = v.create_template(db, "Template".into()).unwrap();
    insert(&v, &template, todo("template-todo", false, false));
    let trashed = v.create_page(None, "Trash".into()).unwrap();
    insert(&v, &trashed, todo("trash-todo", false, false));
    v.delete_page(trashed.clone()).unwrap();
    v.flush().unwrap();

    let entries = v.query_agenda().unwrap();
    assert!(entries.iter().all(|entry| entry.item.page != template && entry.item.page != trashed));
    assert!(!entries.iter().any(|entry| entry.item.page == daily && entry.source == "row"));
    assert!(entries.iter().any(|entry| entry.item.page == daily && entry.source == "todo"));
    let hidden_note = entries.iter().find(|entry| entry.item.page == note).unwrap();
    assert_eq!(hidden_note.calendar_name.as_deref(), Some("Hidden"));
    assert!(hidden_note.today_eligible);
    assert!(!entries.iter().find(|entry| entry.item.page == row_page).unwrap().today_eligible);
    // System settings are not task notes, including in the derived cache.
    let system = v.find_by_role("calendars".into()).unwrap().unwrap();
    assert!(!entries.iter().any(|entry| entry.item.page == system));
    assert_eq!(t.sql(&format!("SELECT count(*) FROM agenda a JOIN pages p ON p.rowid=a.page WHERE p.id='{system}'"))[0][0], Value::Integer(0));
}

#[test]
fn bare_task_pages_are_undated_notes_without_persisting_default_metadata() {
    let t = TempVault::new();
    let bare;
    let cleared;
    {
        let v = t.open();
        bare = v.create_page_with_type(None, "Bare task".into(), Some("task".into())).unwrap();
        cleared = v.create_page_with_type(None, "Cleared task".into(), Some("task".into())).unwrap();
        task_fields(&v, &cleared, serde_json::json!({"status":"doing"}));
        v.type_data_delete(cleared.clone(), "tasks".into(), vec!["status".into()]).unwrap();
        let ordinary = v.create_page(None, "Empty document".into()).unwrap();
        v.flush().unwrap();
        let entries = v.query_agenda().unwrap();
        for page in [&bare, &cleared] {
            let entry = entries.iter().find(|entry| &entry.item.page == page)
                .unwrap_or_else(|| panic!("missing task note for {page}"));
            assert_eq!(entry.source, "task_note");
            assert_eq!(entry.date_ms, None);
            assert_eq!(entry.item.status.as_deref(), Some("todo"));
            assert_eq!(entry.item.assignee.as_deref(), Some("me"));
            assert!(!entry.item.done);
            assert_eq!(v.type_data(page.clone(), "tasks".into()).unwrap(), "{}");
        }
        assert!(!entries.iter().any(|entry| entry.item.page == ordinary));
        assert!(all(&v).is_empty());
        assert!(v.calendar_items(0, DAY + DAY_MS, 0, DAY + DAY_MS).unwrap().is_empty());
        assert!(v.overdue(DAY + DAY_MS, DAY + DAY_MS, 100).unwrap().is_empty());
        assert!(v.done_since(DAY + DAY_MS, DAY + DAY_MS, 0, 100).unwrap().is_empty());
        v.reindex().unwrap();
        assert_eq!(v.query_agenda().unwrap().len(), 2);
        v.flush().unwrap();
    }
    let v = t.open();
    assert_eq!(v.query_agenda().unwrap().len(), 2);
    v.set_agenda_source_done(bare.clone(), None, "task_note".into(), true).unwrap();
    v.reschedule_agenda_source(bare.clone(), None, "task_note".into(), DAY, false).unwrap();
    v.set_page_type(cleared.clone(), "document".into()).unwrap();
    v.flush().unwrap();
    let entries = v.query_agenda().unwrap();
    let bare_entry = entries.iter().find(|entry| entry.item.page == bare).unwrap();
    assert!(bare_entry.item.done);
    assert_eq!(bare_entry.date_ms, Some(DAY));
    assert!(!entries.iter().any(|entry| entry.item.page == cleared));
}

#[test]
fn undated_day_note_todos_are_dated_to_their_note_and_plain_todos_stay_out() {
    let t = TempVault::new();
    let v = t.open();
    let yesterday = v
        .daily_note(DAY - DAY_MS, "Monday 5 October".into(), true)
        .unwrap()
        .unwrap();
    insert(&v, &yesterday, todo("undated-day", false, false));
    insert(
        &v,
        &yesterday,
        NewBlock {
            id: "explicit-day".into(),
            kind: BlockKind::Todo,
            text: "Pay rent @due".into(),
            marks: vec![Mark {
                start: 9,
                end: 13,
                kind: MarkKind::Date("2026-09-28".into()),
            }],
            checked: false,
        },
    );
    let plain = v.create_page(None, "Plain page".into()).unwrap();
    insert(&v, &plain, todo("plain-undated", false, false));
    v.flush().unwrap();

    let overdue = v.overdue(DAY, DAY, 50).unwrap();
    let undated = overdue
        .iter()
        .find(|i| i.block.as_deref() == Some("undated-day"))
        .expect("undone undated day-note to-do is overdue today");
    assert_eq!(undated.page, yesterday);
    assert_eq!(undated.start_ms, DAY - DAY_MS);
    assert!(!undated.has_time);
    let explicit = overdue
        .iter()
        .find(|i| i.block.as_deref() == Some("explicit-day"))
        .expect("explicitly dated day-note to-do is overdue today");
    assert_eq!(explicit.start_ms, DAY - 8 * DAY_MS);
    assert!(overdue
        .iter()
        .all(|i| i.block.as_deref() != Some("plain-undated")));
    assert!(all(&v)
        .iter()
        .all(|i| i.block.as_deref() != Some("plain-undated")));
    assert_eq!(
        t.sql("SELECT d0 FROM agenda WHERE block = 'plain-undated'"),
        vec![vec![Value::Null]]
    );

    v.set_agenda_done(yesterday.clone(), Some("undated-day".into()), true)
        .unwrap();
    v.flush().unwrap();
    assert!(v
        .overdue(DAY, DAY, 50)
        .unwrap()
        .iter()
        .all(|i| i.block.as_deref() != Some("undated-day")));
}
