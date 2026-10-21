//! The unified calendar source: task notes (pages whose `tasks` add-on data
//! has a valid `date`) and dated mentions in ordinary blocks. Covers how each
//! is derived, the task writes, Markdown import and rescan, reindexing, reopen
//! and the forced schema rebuild.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use vault_core::*;

const DAY: i64 = 1_791_244_800_000; // 2026-10-06, midnight UTC.
const DAY_MS: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;
// A window that holds every date the tests use, on both the timed and day bounds.
const FROM: i64 = DAY - 30 * DAY_MS;
const TO: i64 = DAY + 30 * DAY_MS;

struct TempVault(PathBuf);

impl TempVault {
    fn new(tag: &str) -> Self {
        Self(std::env::temp_dir().join(format!("vault_calendar_{tag}_{}", new_id())))
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).unwrap()
    }
    fn index(&self) -> PathBuf {
        self.0.join(".vault").join("index.sqlite")
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

fn status_def() -> PropDef {
    PropDef {
        options: ["todo", "doing", "done"]
            .into_iter()
            .map(|id| SelectOption {
                id: id.into(),
                name: id.into(),
                color: String::new(),
            })
            .collect(),
        ..def("status", PropKind::Status)
    }
}

fn calendar(v: &Vault) -> Vec<AgendaItem> {
    // Task-note and block index updates use the background writer contract.
    v.flush().unwrap();
    v.calendar_items(FROM, TO, FROM, TO).unwrap()
}

fn agenda(v: &Vault) -> Vec<AgendaItem> {
    v.flush().unwrap();
    v.agenda(FROM, TO, FROM, TO).unwrap()
}

fn find<'a>(items: &'a [AgendaItem], page: &str, block: Option<&str>) -> &'a AgendaItem {
    items
        .iter()
        .find(|i| i.page == page && i.block.as_deref() == block)
        .unwrap_or_else(|| panic!("no calendar item for {page} {block:?}"))
}

fn item(v: &Vault, page: &str, block: Option<&str>) -> AgendaItem {
    find(&calendar(v), page, block).clone()
}

fn has(items: &[AgendaItem], page: &str, block: Option<&str>) -> bool {
    items
        .iter()
        .any(|i| i.page == page && i.block.as_deref() == block)
}

/// Pages and blocks of the items, sorted, for set comparisons.
fn keys(items: &[AgendaItem]) -> Vec<(String, Option<String>)> {
    let mut keys: Vec<_> = items
        .iter()
        .map(|i| (i.page.clone(), i.block.clone()))
        .collect();
    keys.sort();
    keys
}

/// Items without the fields that a reindex or reopen may restamp.
fn stable(items: &[AgendaItem]) -> Vec<Value> {
    let mut rows: Vec<Value> = items
        .iter()
        .map(|i| {
            let mut v = serde_json::to_value(i).unwrap();
            v.as_object_mut().unwrap().remove("updated_ms");
            v
        })
        .collect();
    rows.sort_by_key(|r| r.to_string());
    rows
}

/// Writes each field of `fields` into the page's `tasks` add-on data.
fn set_task(v: &Vault, page: &str, fields: Value) {
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

fn new_task(v: &Vault, title: &str, fields: Value) -> PageId {
    let page = v.create_page(None, title.into()).unwrap();
    set_task(v, &page, fields);
    page
}

fn task_data(v: &Vault, page: &str) -> Value {
    serde_json::from_str(&v.type_data(page.into(), "tasks".into()).unwrap()).unwrap()
}

fn all_data(v: &Vault, page: &str) -> Value {
    serde_json::from_str(&v.type_data_all(page.into()).unwrap()).unwrap()
}

/// A block whose date marks are the given substrings of `text`, each paired
/// with the date it stores. List the marks in text order.
fn dated_block(id: &str, kind: BlockKind, text: &str, marks: &[(&str, &str)]) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind,
        text: text.into(),
        marks: marks
            .iter()
            .map(|(needle, iso)| {
                let at = text.find(*needle).expect("needle in block text");
                let start = text[..at].chars().count() as u32;
                Mark {
                    start,
                    end: start + needle.chars().count() as u32,
                    kind: MarkKind::Date((*iso).into()),
                }
            })
            .collect(),
        checked: false,
    }
}

fn add(v: &Vault, page: &str, parent: Option<&str>, block: NewBlock) {
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: parent.map(str::to_string),
            after: None,
            block,
        }],
    )
    .unwrap();
}

/// Rewrites the file's `type-data:` line with `value`.
fn rewrite_type_data(file: &PathBuf, value: &Value) {
    let text = fs::read_to_string(file).unwrap();
    let edited: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with("type-data: ") {
                format!("type-data: {value}")
            } else {
                l.to_string()
            }
        })
        .collect();
    fs::write(file, edited.join("\n") + "\n").unwrap();
}

fn md_file(t: &TempVault, id: &str) -> PathBuf {
    fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
        .expect("markdown file")
}

fn user_version(t: &TempVault) -> i64 {
    rusqlite::Connection::open(t.index())
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

/// One of each calendar source: a timed task note, a date-only task note, a
/// to-do and a paragraph mention. Returns the page ids in that order.
fn mixed(v: &Vault) -> (PageId, PageId, PageId) {
    let timed = new_task(
        v,
        "Standup",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 90, "status": "doing", "calendar": "Work", "assignee": "me"}),
    );
    let day = new_task(v, "Water", json!({"date": "2026-10-07", "status": "done"}));
    let notes = v.create_page(None, "Notes".into()).unwrap();
    add(
        v,
        &notes,
        None,
        dated_block(
            "todo",
            BlockKind::Todo,
            "Call @2026-10-08",
            &[("@2026-10-08", "2026-10-08")],
        ),
    );
    add(
        v,
        &notes,
        None,
        dated_block(
            "mention",
            BlockKind::Paragraph,
            "Dentist @2026-10-09T08:15Z",
            &[("@2026-10-09T08:15Z", "2026-10-09T08:15Z")],
        ),
    );
    (timed, day, notes)
}

#[test]
fn all_day_task_note_spans_its_day_and_ignores_duration() {
    let t = TempVault::new("all_day");
    let v = t.open();
    let p = new_task(
        &v,
        "Water plants",
        json!({"date": "2026-10-06", "duration_min": 120}),
    );
    let a = item(&v, &p, None);
    assert_eq!(a.title, "Water plants");
    assert_eq!(a.kind, AgendaKind::Task);
    assert_eq!(a.block, None);
    assert_eq!((a.start_ms, a.end_ms, a.has_time), (DAY, Some(DAY), false));
    assert!(!a.done);

    // No status and no duration: still a task note, on its own day.
    let q = new_task(&v, "Plain", json!({"date": "2026-10-07"}));
    let b = item(&v, &q, None);
    assert_eq!(
        (b.start_ms, b.end_ms, b.has_time),
        (DAY + DAY_MS, Some(DAY + DAY_MS), false)
    );
}

#[test]
fn timed_task_note_uses_duration_and_fractional_seconds() {
    let t = TempVault::new("timed");
    let v = t.open();
    let p = new_task(
        &v,
        "Standup",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 90}),
    );
    let a = item(&v, &p, None);
    assert_eq!(
        (a.start_ms, a.end_ms, a.has_time),
        (DAY + 10 * HOUR, Some(DAY + 11 * HOUR + 30 * MINUTE), true)
    );

    let q = new_task(
        &v,
        "Precise",
        json!({"date": "2026-10-06T10:00:30.500Z", "duration_min": 1}),
    );
    let b = item(&v, &q, None);
    assert_eq!(
        (b.start_ms, b.end_ms, b.has_time),
        (
            DAY + 10 * HOUR + 30_500,
            Some(DAY + 10 * HOUR + 30_500 + MINUTE),
            true
        )
    );

    // Seconds without milliseconds; a zero duration ends where it starts.
    let r = new_task(
        &v,
        "Zero",
        json!({"date": "2026-10-06T10:00:15Z", "duration_min": 0}),
    );
    let c = item(&v, &r, None);
    assert_eq!(
        (c.start_ms, c.end_ms),
        (DAY + 10 * HOUR + 15_000, Some(DAY + 10 * HOUR + 15_000))
    );
}

#[test]
fn invalid_durations_never_overflow_or_end_before_start() {
    let t = TempVault::new("durations");
    let v = t.open();
    // A timed task without a usable duration is a point.
    let missing = new_task(&v, "Missing", json!({"date": "2026-10-06T10:00Z"}));
    let a = item(&v, &missing, None);
    assert!(a.end_ms.unwrap_or(a.start_ms) >= a.start_ms);

    let durations = [
        json!(null),
        json!(-30),
        json!(1.5),
        json!("45"),
        json!(1_000_000_000_000_000_i64),
        json!(i64::MAX),
    ];
    for d in durations {
        let mut fields = json!({"date": "2026-10-06T10:00Z"});
        fields["duration_min"] = d.clone();
        let p = new_task(&v, "Odd", fields);
        let a = item(&v, &p, None);
        assert_eq!(a.start_ms, DAY + 10 * HOUR, "{d}");
        assert!(
            a.end_ms.unwrap_or(a.start_ms) >= a.start_ms,
            "{d}: end before start ({:?})",
            a.end_ms
        );
    }
}

#[test]
fn status_defaults_to_todo_and_checkbox_does_not_decide_completion() {
    let t = TempVault::new("status");
    let v = t.open();
    let default = new_task(&v, "Default", json!({"date": "2026-10-06"}));
    let doing = new_task(
        &v,
        "Doing",
        json!({"date": "2026-10-06", "status": "doing"}),
    );
    let done = new_task(&v, "Done", json!({"date": "2026-10-06", "status": "done"}));
    // A status the add-on does not define reads as todo.
    let other = new_task(
        &v,
        "Other",
        json!({"date": "2026-10-06", "status": "later"}),
    );
    let checked = new_task(
        &v,
        "Checked",
        json!({"date": "2026-10-06", "status": "todo"}),
    );
    v.set_property(checked.clone(), "done".into(), PropValue::Checkbox(true))
        .unwrap();

    let items = calendar(&v);
    let state = |page: &str| {
        let a = find(&items, page, None);
        (a.status.clone(), a.status_group.clone(), a.done)
    };
    assert_eq!(
        state(&default),
        (Some("todo".into()), Some("todo".into()), false)
    );
    assert_eq!(
        state(&doing),
        (Some("doing".into()), Some("progress".into()), false)
    );
    assert_eq!(
        state(&done),
        (Some("done".into()), Some("complete".into()), true)
    );
    assert_eq!(
        state(&other),
        (Some("todo".into()), Some("todo".into()), false)
    );
    assert_eq!(
        state(&checked),
        (Some("todo".into()), Some("todo".into()), false)
    );
}

#[test]
fn missing_and_invalid_dates_produce_no_item() {
    let t = TempVault::new("dates");
    let v = t.open();
    let valid = new_task(&v, "Valid", json!({"date": "2026-10-06"}));
    let cases = [
        json!({"status": "doing", "duration_min": 30}),
        json!({"date": 20261006}),
        json!({"date": ""}),
        json!({"date": "soon"}),
        json!({"date": "2026-02-30"}),
        json!({"date": "2026-1-06"}),
        // Timed dates are UTC and need the Z suffix.
        json!({"date": "2026-10-06T10:00"}),
        json!({"date": "2026-10-06T24:00Z"}),
        json!({"date": "2026-10-06T10:00:00.5Z"}),
        json!({"date": "2026-10-06 10:00Z"}),
    ];
    let invalid: Vec<PageId> = cases
        .iter()
        .enumerate()
        .map(|(i, fields)| new_task(&v, &format!("Bad {i}"), fields.clone()))
        .collect();

    let items = calendar(&v);
    assert_eq!(keys(&items), vec![(valid.clone(), None)]);
    for page in &invalid {
        assert!(!items.iter().any(|i| &i.page == page), "{page} listed");
    }
    assert!(!agenda(&v).iter().any(|i| invalid.contains(&i.page)));
}

#[test]
fn date_updates_and_deletion_add_and_remove_the_task() {
    let t = TempVault::new("updates");
    let v = t.open();
    let p = new_task(
        &v,
        "Moving",
        json!({"date": "2026-10-06", "status": "doing"}),
    );

    set_task(
        &v,
        &p,
        json!({"date": "2026-10-08T09:00Z", "duration_min": 30}),
    );
    let a = item(&v, &p, None);
    assert_eq!(
        (a.start_ms, a.end_ms, a.has_time),
        (
            DAY + 2 * DAY_MS + 9 * HOUR,
            Some(DAY + 2 * DAY_MS + 9 * HOUR + 30 * MINUTE),
            true
        )
    );

    // An invalid date removes the task from the calendar but keeps its data.
    set_task(&v, &p, json!({"date": "2026-13-01"}));
    assert!(!has(&calendar(&v), &p, None));
    assert_eq!(task_data(&v, &p)["status"], "doing");

    set_task(&v, &p, json!({"date": "2026-10-07"}));
    let b = item(&v, &p, None);
    assert_eq!(
        (b.start_ms, b.end_ms, b.has_time, b.status_group.as_deref()),
        (DAY + DAY_MS, Some(DAY + DAY_MS), false, Some("progress"))
    );

    v.type_data_delete(p.clone(), "tasks".into(), vec!["date".into()])
        .unwrap();
    assert!(!has(&calendar(&v), &p, None));
    assert_eq!(
        task_data(&v, &p),
        json!({"status": "doing", "duration_min": 30})
    );

    set_task(&v, &p, json!({"date": "2026-10-09"}));
    assert_eq!(item(&v, &p, None).start_ms, DAY + 3 * DAY_MS);
}

#[test]
fn unrelated_add_on_writes_leave_the_task_unchanged() {
    let t = TempVault::new("unrelated");
    let v = t.open();
    let p = new_task(
        &v,
        "Shared",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 60, "status": "doing", "calendar": "Work", "assignee": "agent"}),
    );
    let before = stable(&calendar(&v));

    v.type_data_set(
        p.clone(),
        "mod:other".into(),
        vec!["date".into()],
        "\"2030-01-01\"".into(),
    )
    .unwrap();
    v.type_data_set(p.clone(), "mod:other".into(), vec!["n".into()], "2".into())
        .unwrap();
    v.type_data_set(
        p.clone(),
        "tasks".into(),
        vec!["notes".into()],
        "\"kept\"".into(),
    )
    .unwrap();

    assert_eq!(stable(&calendar(&v)), before);
    // A date under another add-on is not a task note's date.
    assert_eq!(
        all_data(&v, &p)["mod:other"],
        json!({"date": "2030-01-01", "n": 2})
    );
    assert_eq!(task_data(&v, &p)["notes"], "kept");
}

#[test]
fn set_agenda_done_writes_status_and_keeps_other_fields() {
    let t = TempVault::new("done");
    let v = t.open();
    let p = new_task(
        &v,
        "Finish",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 45, "status": "doing", "calendar": "Work", "assignee": "me", "extra": {"n": 1}}),
    );
    let before = all_data(&v, &p);
    let mut expected = before.clone();

    v.set_agenda_done(p.clone(), None, true).unwrap();
    expected["tasks"]["status"] = json!("done");
    assert_eq!(all_data(&v, &p), expected);
    let a = item(&v, &p, None);
    assert!(a.done);
    assert_eq!(a.status_group.as_deref(), Some("complete"));
    assert_eq!(
        (
            a.start_ms,
            a.end_ms,
            a.calendar.as_deref(),
            a.assignee.as_deref()
        ),
        (
            DAY + 10 * HOUR,
            Some(DAY + 10 * HOUR + 45 * MINUTE),
            Some("Work"),
            Some("me")
        )
    );

    v.set_agenda_done(p.clone(), None, false).unwrap();
    expected["tasks"]["status"] = json!("todo");
    assert_eq!(all_data(&v, &p), expected);
    let b = item(&v, &p, None);
    assert!(!b.done);
    assert_eq!(b.status_group.as_deref(), Some("todo"));
}

#[test]
fn reschedule_keeps_fields_and_duration() {
    let t = TempVault::new("reschedule");
    let v = t.open();
    let p = new_task(
        &v,
        "Move me",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 60, "status": "doing", "calendar": "Work", "assignee": "agent"}),
    );
    v.type_data_set(p.clone(), "mod:other".into(), vec!["x".into()], "2".into())
        .unwrap();
    let mut expected = all_data(&v, &p);

    v.reschedule_agenda(p.clone(), None, DAY + DAY_MS + 9 * HOUR + 30 * MINUTE, true)
        .unwrap();
    expected["tasks"]["date"] = json!("2026-10-07T09:30:00.000Z");
    assert_eq!(all_data(&v, &p), expected);
    let a = item(&v, &p, None);
    assert_eq!(
        (a.start_ms, a.end_ms, a.has_time),
        (
            DAY + DAY_MS + 9 * HOUR + 30 * MINUTE,
            Some(DAY + DAY_MS + 10 * HOUR + 30 * MINUTE),
            true
        )
    );
    assert_eq!(
        (
            a.status_group.as_deref(),
            a.calendar.as_deref(),
            a.assignee.as_deref()
        ),
        (Some("progress"), Some("Work"), Some("agent"))
    );

    // A date-only move keeps the duration in the data but ends the same day.
    v.reschedule_agenda(p.clone(), None, DAY + 2 * DAY_MS, false)
        .unwrap();
    expected["tasks"]["date"] = json!("2026-10-08");
    assert_eq!(all_data(&v, &p), expected);
    let b = item(&v, &p, None);
    assert_eq!(
        (b.start_ms, b.end_ms, b.has_time),
        (DAY + 2 * DAY_MS, Some(DAY + 2 * DAY_MS), false)
    );

    // A date-only move must start at midnight, and a rejected move changes nothing.
    let snapshot = all_data(&v, &p);
    assert!(v
        .reschedule_agenda(p.clone(), None, DAY + 2 * DAY_MS + HOUR, false)
        .is_err());
    assert_eq!(all_data(&v, &p), snapshot);
}

#[test]
fn markdown_import_reads_task_note_and_rescan_updates_it() {
    let t = TempVault::new("md");
    let src = TempVault::new("md_src");
    fs::create_dir_all(&src.0).unwrap();
    let data = json!({"tasks": {"date": "2026-10-06T14:00Z", "duration_min": 30, "status": "doing", "calendar": "Health", "assignee": "me"}});
    fs::write(
        src.0.join("Dentist.md"),
        format!("---\ntype-data: {data}\n---\n\nBring forms\n"),
    )
    .unwrap();

    let v = t.open();
    let report = v.import_markdown(src.root(), None).unwrap();
    let page = report.top_pages[0].clone();
    let a = item(&v, &page, None);
    assert_eq!(a.title, "Dentist");
    assert_eq!(a.kind, AgendaKind::Task);
    assert_eq!(
        (a.start_ms, a.end_ms, a.has_time),
        (DAY + 14 * HOUR, Some(DAY + 14 * HOUR + 30 * MINUTE), true)
    );
    assert_eq!(
        (
            a.status.as_deref(),
            a.calendar.as_deref(),
            a.assignee.as_deref()
        ),
        (Some("doing"), Some("Health"), Some("me"))
    );

    // An outside edit: date-only, done, no duration and no assignee.
    v.flush().unwrap();
    rewrite_type_data(
        &md_file(&t, &page),
        &json!({"tasks": {"date": "2026-10-07", "status": "done", "calendar": "Health"}}),
    );
    assert!(v.rescan().unwrap() >= 1);
    let b = item(&v, &page, None);
    assert_eq!(
        (b.start_ms, b.end_ms, b.has_time, b.done),
        (DAY + DAY_MS, Some(DAY + DAY_MS), false, true)
    );
    assert_eq!(b.assignee, None);
    assert_eq!(b.title, "Dentist");
}

#[test]
fn first_valid_date_mark_of_a_non_todo_block_is_a_mention() {
    let t = TempVault::new("mention");
    let v = t.open();
    let p = v.create_page(None, "Journal".into()).unwrap();
    // The first mark is not a real date, so the second one is the mention.
    add(
        &v,
        &p,
        None,
        dated_block(
            "skip",
            BlockKind::Paragraph,
            "Launch @2026-02-30 party @2026-10-07 now",
            &[("@2026-02-30", "2026-02-30"), ("@2026-10-07", "2026-10-07")],
        ),
    );
    // Surrounding spaces go; the mark's own text is removed from the title.
    add(
        &v,
        &p,
        None,
        dated_block(
            "trim",
            BlockKind::Paragraph,
            "  Call mum @2026-10-07  ",
            &[("@2026-10-07", "2026-10-07")],
        ),
    );
    add(
        &v,
        &p,
        None,
        dated_block(
            "timed",
            BlockKind::Paragraph,
            "Lunch @2026-10-06T12:30Z",
            &[("@2026-10-06T12:30Z", "2026-10-06T12:30Z")],
        ),
    );
    add(
        &v,
        &p,
        None,
        dated_block(
            "bad",
            BlockKind::Paragraph,
            "Only @2026-02-30",
            &[("@2026-02-30", "2026-02-30")],
        ),
    );
    add(
        &v,
        &p,
        None,
        dated_block(
            "todo",
            BlockKind::Todo,
            "Pay @2026-10-06",
            &[("@2026-10-06", "2026-10-06")],
        ),
    );
    add(
        &v,
        &p,
        None,
        dated_block(
            "todo2",
            BlockKind::Todo,
            "Ship @2026-13-01 then @2026-10-09",
            &[("@2026-13-01", "2026-13-01"), ("@2026-10-09", "2026-10-09")],
        ),
    );

    let items = calendar(&v);
    let skip = find(&items, &p, Some("skip"));
    assert_eq!(skip.kind, AgendaKind::Mention);
    assert_eq!(skip.title, "Launch @2026-02-30 party  now");
    assert_eq!(
        (skip.start_ms, skip.has_time, skip.done),
        (DAY + DAY_MS, false, false)
    );

    let trim = find(&items, &p, Some("trim"));
    assert_eq!(
        (trim.kind, trim.title.as_str(), trim.start_ms),
        (AgendaKind::Mention, "Call mum", DAY + DAY_MS)
    );

    let timed = find(&items, &p, Some("timed"));
    assert_eq!(timed.kind, AgendaKind::Mention);
    assert_eq!(
        (timed.start_ms, timed.has_time),
        (DAY + 12 * HOUR + 30 * MINUTE, true)
    );

    assert!(!has(&items, &p, Some("bad")));

    // To-dos stay tasks, and the first valid mark of a to-do is its date.
    let pay = find(&items, &p, Some("todo"));
    assert_eq!(
        (pay.kind, pay.title.as_str(), pay.start_ms, pay.done),
        (AgendaKind::Task, "Pay", DAY, false)
    );
    let ship = find(&items, &p, Some("todo2"));
    assert_eq!(
        (ship.kind, ship.title.as_str(), ship.start_ms),
        (AgendaKind::Task, "Ship @2026-13-01 then", DAY + 3 * DAY_MS)
    );

    // A mention is not an agenda task.
    assert!(v
        .set_agenda_done(p.clone(), Some("skip".into()), true)
        .is_err());
    v.set_agenda_done(p.clone(), Some("todo".into()), true)
        .unwrap();
    assert!(item(&v, &p, Some("todo")).done);
}

#[test]
fn nested_mentions_and_todos_keep_their_kinds() {
    let t = TempVault::new("nested");
    let v = t.open();
    let p = v.create_page(None, "Plan".into()).unwrap();
    add(
        &v,
        &p,
        None,
        dated_block(
            "parent",
            BlockKind::Bulleted,
            "Trip @2026-10-06",
            &[("@2026-10-06", "2026-10-06")],
        ),
    );
    add(
        &v,
        &p,
        Some("parent"),
        dated_block(
            "child",
            BlockKind::Heading2,
            "Review @2026-10-08",
            &[("@2026-10-08", "2026-10-08")],
        ),
    );
    add(
        &v,
        &p,
        Some("parent"),
        dated_block(
            "child-todo",
            BlockKind::Todo,
            "Ship @2026-10-09",
            &[("@2026-10-09", "2026-10-09")],
        ),
    );
    add(
        &v,
        &p,
        Some("child"),
        dated_block(
            "grand",
            BlockKind::Paragraph,
            "Deep @2026-10-07",
            &[("@2026-10-07", "2026-10-07")],
        ),
    );

    let items = calendar(&v);
    let parent = find(&items, &p, Some("parent"));
    assert_eq!(
        (parent.kind, parent.title.as_str()),
        (AgendaKind::Mention, "Trip")
    );

    let child = find(&items, &p, Some("child"));
    assert_eq!(
        (child.kind, child.title.as_str(), child.start_ms),
        (AgendaKind::Mention, "Review", DAY + 2 * DAY_MS)
    );

    let todo = find(&items, &p, Some("child-todo"));
    assert_eq!((todo.kind, todo.title.as_str()), (AgendaKind::Task, "Ship"));

    let grand = find(&items, &p, Some("grand"));
    assert_eq!(
        (grand.kind, grand.title.as_str(), grand.start_ms),
        (AgendaKind::Mention, "Deep", DAY + DAY_MS)
    );
    assert!(v
        .set_agenda_done(p.clone(), Some("child".into()), true)
        .is_err());
}

#[test]
fn mentions_reach_calendar_items_but_not_legacy_queries() {
    let t = TempVault::new("legacy");
    let v = t.open();
    let past = "2026-10-05";
    let mark = format!("@{past}");
    let notes = v.create_page(None, "Notes".into()).unwrap();
    add(
        &v,
        &notes,
        None,
        dated_block(
            "mention",
            BlockKind::Paragraph,
            &format!("Renew passport {mark}"),
            &[(mark.as_str(), past)],
        ),
    );
    add(
        &v,
        &notes,
        None,
        dated_block(
            "todo",
            BlockKind::Todo,
            &format!("Open task {mark}"),
            &[(mark.as_str(), past)],
        ),
    );
    add(
        &v,
        &notes,
        None,
        NewBlock {
            checked: true,
            ..dated_block(
                "closed",
                BlockKind::Todo,
                &format!("Closed {mark}"),
                &[(mark.as_str(), past)],
            )
        },
    );
    let overdue_note = new_task(&v, "Overdue note", json!({"date": past}));
    let finished_note = new_task(&v, "Finished note", json!({"date": past, "status": "done"}));
    v.flush().unwrap();

    let cal = calendar(&v);
    assert_eq!(
        find(&cal, &notes, Some("mention")).kind,
        AgendaKind::Mention
    );
    assert!(has(&cal, &notes, Some("closed")));

    let legacy = agenda(&v);
    assert!(!has(&legacy, &notes, Some("mention")));
    assert!(has(&legacy, &notes, Some("todo")));
    assert!(has(&legacy, &overdue_note, None));

    // Overdue and done lists exclude mentions: only to-dos and task notes count.
    let overdue = v.overdue(DAY, DAY, 100).unwrap();
    let mut want = vec![
        (notes.clone(), Some("todo".to_string())),
        (overdue_note.clone(), None),
    ];
    want.sort();
    assert_eq!(keys(&overdue), want);
    assert_eq!(v.overdue_count(DAY, DAY).unwrap(), 2);

    let done = v.done_since(DAY, DAY, 0, 100).unwrap();
    let mut want = vec![
        (notes.clone(), Some("closed".to_string())),
        (finished_note.clone(), None),
    ];
    want.sort();
    assert_eq!(keys(&done), want);
}

#[test]
fn calendar_items_include_database_rows_tasks_and_mentions() {
    let t = TempVault::new("sources");
    let v = t.open();
    let tasks_db = v.create_page(None, "Tasks".into()).unwrap();
    v.set_schema(
        tasks_db.clone(),
        vec![def("date", PropKind::Date), status_def()],
    )
    .unwrap();
    let row = v.create_page(Some(tasks_db.clone()), "Row".into()).unwrap();
    v.set_property(
        row.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: DAY,
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();

    let events_db = v.create_page(None, "Events".into()).unwrap();
    v.set_schema(events_db.clone(), vec![def("date", PropKind::Date)])
        .unwrap();
    let event = v.create_page(Some(events_db), "Launch".into()).unwrap();
    v.set_property(
        event.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: DAY + DAY_MS,
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();

    let (timed, _, notes) = mixed(&v);
    let legacy = agenda(&v);
    let cal = calendar(&v);

    // Legacy agenda: the database row, the event, both task notes and the to-do.
    assert_eq!(legacy.len(), 5);
    assert!(!has(&legacy, &notes, Some("mention")));
    assert_eq!(find(&cal, &row, None).kind, AgendaKind::Task);
    assert_eq!(find(&cal, &event, None).kind, AgendaKind::Event);
    assert_eq!(find(&cal, &timed, None).kind, AgendaKind::Task);
    assert_eq!(find(&cal, &notes, Some("todo")).kind, AgendaKind::Task);
    assert_eq!(
        find(&cal, &notes, Some("mention")).kind,
        AgendaKind::Mention
    );
    // Every legacy item is in the calendar, and the calendar adds only mentions.
    for i in &legacy {
        assert!(cal.contains(i), "calendar lacks {i:?}");
    }
    assert_eq!(cal.len(), legacy.len() + 1);
}

#[test]
fn task_note_overdue_uses_end_and_status() {
    let t = TempVault::new("overdue");
    let v = t.open();
    let long = new_task(
        &v,
        "Long",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 60}),
    );
    let doing = new_task(
        &v,
        "Doing",
        json!({"date": "2026-10-06", "status": "doing"}),
    );
    let done = new_task(
        &v,
        "Done",
        json!({"date": "2026-10-06T10:00Z", "duration_min": 60, "status": "done"}),
    );
    v.flush().unwrap();

    // A timed task is overdue once its end has passed, not its start.
    assert!(v
        .overdue(DAY + 10 * HOUR + 30 * MINUTE, DAY, 10)
        .unwrap()
        .is_empty());
    let late = v
        .overdue(DAY + 11 * HOUR + 30 * MINUTE, DAY + DAY_MS, 10)
        .unwrap();
    let mut want = vec![long.clone(), doing.clone()];
    want.sort();
    let mut got: Vec<String> = late.iter().map(|i| i.page.clone()).collect();
    got.sort();
    assert_eq!(got, want);
    assert_eq!(
        v.overdue_count(DAY + 11 * HOUR + 30 * MINUTE, DAY + DAY_MS)
            .unwrap(),
        2
    );

    let finished = v
        .done_since(DAY + 11 * HOUR + 30 * MINUTE, DAY + DAY_MS, 0, 10)
        .unwrap();
    assert_eq!(keys(&finished), vec![(done.clone(), None)]);
}

#[test]
fn reindex_and_reopen_keep_task_notes_and_mentions() {
    let t = TempVault::new("reindex");
    let before;
    {
        let v = t.open();
        mixed(&v);
        before = stable(&calendar(&v));
        assert_eq!(before.len(), 4);

        v.reindex().unwrap();
        assert_eq!(
            stable(&calendar(&v)),
            before,
            "reindex changed the calendar"
        );
    }
    let v = t.open();
    assert_eq!(stable(&calendar(&v)), before, "reopen changed the calendar");
}

#[test]
fn forced_schema_rebuild_from_user_version_9_keeps_the_calendar() {
    let t = TempVault::new("schema");
    let before;
    let version;
    {
        let v = t.open();
        mixed(&v);
        before = stable(&calendar(&v));
        version = user_version(&t);
    }
    assert_ne!(
        version, 9,
        "the forced version must differ from the current one"
    );

    // An index written by the previous schema is rebuilt from the vault's documents.
    rusqlite::Connection::open(t.index())
        .unwrap()
        .pragma_update(None, "user_version", 9)
        .unwrap();
    let v = t.open();
    assert_eq!(stable(&calendar(&v)), before);
    assert_eq!(user_version(&t), version, "the index was not rebuilt");
}

#[test]
fn assignee_is_kept_only_when_it_is_me_or_agent() {
    let t = TempVault::new("assignee");
    let v = t.open();
    let me = new_task(&v, "Mine", json!({"date": "2026-10-06", "assignee": "me"}));
    let agent = new_task(
        &v,
        "Delegated",
        json!({"date": "2026-10-06", "assignee": "agent"}),
    );
    let other = new_task(
        &v,
        "Other",
        json!({"date": "2026-10-06", "assignee": "Ana"}),
    );
    let number = new_task(&v, "Number", json!({"date": "2026-10-06", "assignee": 3}));
    let missing = new_task(&v, "Missing", json!({"date": "2026-10-06"}));

    let items = calendar(&v);
    assert_eq!(find(&items, &me, None).assignee.as_deref(), Some("me"));
    assert_eq!(
        find(&items, &agent, None).assignee.as_deref(),
        Some("agent")
    );
    assert_eq!(find(&items, &other, None).assignee, None);
    assert_eq!(find(&items, &number, None).assignee, None);
    assert_eq!(find(&items, &missing, None).assignee, None);
}
