//! Calendar categories: the hidden `calendars` page with its lazy defaults,
//! create, update, delete and reorder, per-database calendar switches, and how
//! the categories filter Today and the calendar. Dated to-dos and mentions
//! stay independent of the categories.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use vault_core::*;

const DAY: i64 = 1_791_244_800_000; // 2026-10-06, midnight UTC.
const DAY_MS: i64 = 86_400_000;
// A window that holds every date the tests use, on both the timed and day bounds.
const FROM: i64 = DAY - 30 * DAY_MS;
const TO: i64 = DAY + 30 * DAY_MS;

struct TempVault(PathBuf);

impl TempVault {
    fn new(tag: &str) -> Self {
        Self(std::env::temp_dir().join(format!("vault_calendars_{tag}_{}", new_id())))
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).unwrap()
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

fn cat(
    id: &str,
    name: &str,
    color: &str,
    show_in_calendar: bool,
    show_in_today: bool,
) -> CalendarCategory {
    CalendarCategory {
        id: id.into(),
        name: name.into(),
        color: color.into(),
        show_in_calendar,
        show_in_today,
    }
}

/// The three categories created on first use, in list order.
fn default_calendars() -> Vec<CalendarCategory> {
    vec![
        cat("personal", "Personal", "apricot", true, true),
        cat("work", "Work", "sky", true, true),
        cat("agent", "Agent", "duvet", true, false),
    ]
}

fn default_settings() -> CalendarSettings {
    CalendarSettings {
        calendars: default_calendars(),
        databases: Vec::new(),
    }
}

fn ids(calendars: &[CalendarCategory]) -> Vec<&str> {
    calendars.iter().map(|c| c.id.as_str()).collect()
}

/// Flushes task and block edits, then runs the calendar query over the window.
fn cal_items(v: &Vault) -> Vec<AgendaItem> {
    v.flush().unwrap();
    v.calendar_items(FROM, TO, FROM, TO).unwrap()
}

/// Today's agenda: the day of the tests, with timed and date-only bounds.
fn today_items(v: &Vault) -> Vec<AgendaItem> {
    v.flush().unwrap();
    v.agenda(DAY, DAY + DAY_MS, DAY, DAY + DAY_MS).unwrap()
}

fn overdue_items(v: &Vault) -> Vec<AgendaItem> {
    v.flush().unwrap();
    v.overdue(DAY, DAY, 50).unwrap()
}

fn overdue_total(v: &Vault) -> u32 {
    v.flush().unwrap();
    v.overdue_count(DAY, DAY).unwrap()
}

fn done_items(v: &Vault) -> Vec<AgendaItem> {
    v.flush().unwrap();
    v.done_since(DAY, DAY, DAY, 50).unwrap()
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

/// Expected (page, block) pairs, sorted the way `keys` sorts them.
fn want(pairs: &[(&String, Option<&str>)]) -> Vec<(String, Option<String>)> {
    let mut keys: Vec<_> = pairs
        .iter()
        .map(|(page, block)| (page.to_string(), block.map(str::to_string)))
        .collect();
    keys.sort();
    keys
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

fn add(v: &Vault, page: &str, block: NewBlock) {
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

fn calendar_ids(v: &Vault) -> Vec<String> {
    v.list_calendars()
        .unwrap()
        .calendars
        .into_iter()
        .map(|c| c.id)
        .collect()
}

#[test]
fn defaults_are_created_lazily_on_the_hidden_calendars_page() {
    let t = TempVault::new("lazy");
    let v = t.open();
    assert_eq!(v.find_by_role("calendars".into()).unwrap(), None);

    assert_eq!(v.list_calendars().unwrap(), default_settings());
    assert!(v.find_by_role("calendars".into()).unwrap().is_some());
}

#[test]
fn defaults_are_idempotent_and_survive_reopen() {
    let t = TempVault::new("idempotent");
    let role;
    {
        let v = t.open();
        let first = v.list_calendars().unwrap();
        assert_eq!(first, default_settings());
        assert_eq!(v.list_calendars().unwrap(), first);

        role = v
            .find_by_role("calendars".into())
            .unwrap()
            .expect("calendars page");
        assert_eq!(
            v.find_by_role("calendars".into()).unwrap(),
            Some(role.clone())
        );
        // The lists live in the page's tasks add-on data.
        let data = task_data(&v, &role);
        assert_eq!(data["calendars"].as_array().map(Vec::len), Some(3));
    }

    let v = t.open();
    assert_eq!(
        v.find_by_role("calendars".into()).unwrap(),
        Some(role.clone())
    );
    assert_eq!(v.list_calendars().unwrap(), default_settings());
}

#[test]
fn create_calendar_appends_a_category_with_its_switches() {
    let t = TempVault::new("create");
    let v = t.open();
    let id = v
        .create_calendar("Errands".into(), "moss".into(), true, false)
        .unwrap();
    assert!(!id.is_empty());

    let cals = v.list_calendars().unwrap().calendars;
    assert_eq!(cals.len(), 4);
    assert_eq!(&cals[..3], &default_calendars()[..]);
    assert_eq!(cals[3], cat(&id, "Errands", "moss", true, false));
}

#[test]
fn update_calendar_replaces_only_that_category() {
    let t = TempVault::new("update");
    let v = t.open();
    v.list_calendars().unwrap();

    v.update_calendar(cat("work", "Office", "moss", false, true))
        .unwrap();
    let cals = v.list_calendars().unwrap().calendars;
    assert_eq!(cals[0], default_calendars()[0]);
    assert_eq!(cals[1], cat("work", "Office", "moss", false, true));
    assert_eq!(cals[2], default_calendars()[2]);

    v.update_calendar(cat("agent", "Agent", "duvet", true, true))
        .unwrap();
    assert!(v.list_calendars().unwrap().calendars[2].show_in_today);
}

#[test]
fn delete_calendar_removes_only_that_category() {
    let t = TempVault::new("delete");
    let v = t.open();
    v.list_calendars().unwrap();

    v.delete_calendar("agent".into()).unwrap();
    assert_eq!(calendar_ids(&v), vec!["personal", "work"]);
}

#[test]
fn reorder_calendars_sets_the_list_order_and_refuses_bad_lists() {
    let t = TempVault::new("reorder");
    let v = t.open();
    v.list_calendars().unwrap();

    v.reorder_calendars(vec!["agent".into(), "personal".into(), "work".into()])
        .unwrap();
    let cals = v.list_calendars().unwrap().calendars;
    assert_eq!(ids(&cals), vec!["agent", "personal", "work"]);
    // Fields move with their ids.
    assert_eq!(cals[0], default_calendars()[2]);
    assert_eq!(cals[2], default_calendars()[1]);

    let before = v.list_calendars().unwrap();
    // Missing, duplicate and unknown ids are each refused.
    assert!(v
        .reorder_calendars(vec!["work".into(), "personal".into()])
        .is_err());
    assert!(v
        .reorder_calendars(vec!["work".into(), "work".into(), "agent".into()])
        .is_err());
    assert!(v
        .reorder_calendars(vec![
            "work".into(),
            "personal".into(),
            "agent".into(),
            "ghost".into()
        ])
        .is_err());
    assert_eq!(v.list_calendars().unwrap(), before);
}

#[test]
fn invalid_colours_and_names_are_refused() {
    let t = TempVault::new("invalid");
    let v = t.open();
    let before = v.list_calendars().unwrap();

    for colour in ["pink", "", "Apricot", "#ffffff"] {
        assert!(
            v.create_calendar("Errands".into(), colour.into(), true, true)
                .is_err(),
            "accepted colour {colour:?}"
        );
    }
    for name in ["", "   "] {
        assert!(
            v.create_calendar(name.into(), "moss".into(), true, true)
                .is_err(),
            "accepted name {name:?}"
        );
    }

    let mut work = before.calendars[1].clone();
    work.color = "pink".into();
    assert!(v.update_calendar(work.clone()).is_err());
    work.color = "sky".into();
    work.name = "  ".into();
    assert!(v.update_calendar(work).is_err());
    assert_eq!(v.list_calendars().unwrap(), before);

    // Every colour of the family is accepted.
    for colour in ["apricot", "paw", "moss", "sun", "sky", "duvet"] {
        assert!(
            v.create_calendar(format!("Cal {colour}"), colour.into(), true, true)
                .is_ok(),
            "refused colour {colour:?}"
        );
    }
}

#[test]
fn unknown_calendar_ids_are_refused() {
    let t = TempVault::new("unknown_ids");
    let v = t.open();
    let before = v.list_calendars().unwrap();

    assert!(v
        .update_calendar(cat("ghost", "Ghost", "moss", true, true))
        .is_err());
    assert!(v.delete_calendar("ghost".into()).is_err());
    assert_eq!(v.list_calendars().unwrap(), before);

    // A calendar deleted once cannot be deleted again.
    v.delete_calendar("agent".into()).unwrap();
    assert!(v.delete_calendar("agent".into()).is_err());
}

#[test]
fn edited_calendars_survive_reopen() {
    let t = TempVault::new("persist");
    let before;
    {
        let v = t.open();
        let errands = v
            .create_calendar("Errands".into(), "moss".into(), true, false)
            .unwrap();
        v.update_calendar(cat("work", "Office", "paw", false, true))
            .unwrap();
        v.reorder_calendars(vec![
            "agent".into(),
            "work".into(),
            "personal".into(),
            errands,
        ])
        .unwrap();
        before = v.list_calendars().unwrap();
    }
    let v = t.open();
    assert_eq!(v.list_calendars().unwrap(), before);
}

#[test]
fn today_follows_show_in_today_and_calendar_follows_show_in_calendar() {
    let t = TempVault::new("switches");
    let v = t.open();
    let me = new_task(
        &v,
        "Errand",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "personal"}),
    );
    let office = new_task(
        &v,
        "Report",
        json!({"date": "2026-10-06", "status": "doing", "calendar": "work"}),
    );
    let delegated = new_task(
        &v,
        "Delegated",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "agent"}),
    );

    // Agent is hidden from Today by default, but still shows in the calendar.
    assert_eq!(
        keys(&today_items(&v)),
        want(&[(&me, None), (&office, None)])
    );
    assert_eq!(
        keys(&cal_items(&v)),
        want(&[(&me, None), (&office, None), (&delegated, None)])
    );

    v.update_calendar(cat("agent", "Agent", "duvet", true, true))
        .unwrap();
    v.update_calendar(cat("work", "Work", "sky", false, true))
        .unwrap();
    assert_eq!(
        keys(&today_items(&v)),
        want(&[(&me, None), (&office, None), (&delegated, None)])
    );
    assert_eq!(
        keys(&cal_items(&v)),
        want(&[(&me, None), (&delegated, None)])
    );
}

#[test]
fn overdue_count_and_done_since_follow_show_in_today() {
    let t = TempVault::new("overdue");
    let v = t.open();
    let late_work = new_task(
        &v,
        "Late work",
        json!({"date": "2026-10-05", "status": "todo", "calendar": "work"}),
    );
    let late_agent = new_task(
        &v,
        "Late agent",
        json!({"date": "2026-10-05", "status": "todo", "calendar": "agent"}),
    );
    let late_plain = new_task(
        &v,
        "Late plain",
        json!({"date": "2026-10-05", "status": "todo"}),
    );
    let done_work = new_task(
        &v,
        "Done work",
        json!({"date": "2026-10-05", "status": "done", "calendar": "work"}),
    );
    let done_agent = new_task(
        &v,
        "Done agent",
        json!({"date": "2026-10-05", "status": "done", "calendar": "agent"}),
    );

    // Agent is hidden from Today, so its overdue and done tasks are left out.
    assert_eq!(
        keys(&overdue_items(&v)),
        want(&[(&late_work, None), (&late_plain, None)])
    );
    assert_eq!(overdue_total(&v), 2);
    assert_eq!(keys(&done_items(&v)), want(&[(&done_work, None)]));
    assert!(!has(&done_items(&v), &done_agent, None));
    assert!(!has(&overdue_items(&v), &late_agent, None));

    // Showing Agent in Today brings its tasks back into all four queries.
    v.update_calendar(cat("agent", "Agent", "duvet", true, true))
        .unwrap();
    assert_eq!(
        keys(&overdue_items(&v)),
        want(&[(&late_work, None), (&late_agent, None), (&late_plain, None)])
    );
    assert_eq!(overdue_total(&v), 3);
    assert_eq!(
        keys(&done_items(&v)),
        want(&[(&done_work, None), (&done_agent, None)])
    );
}

#[test]
fn unknown_and_deleted_calendars_fall_back_to_visible() {
    let t = TempVault::new("fallback");
    let v = t.open();
    let ghost = new_task(
        &v,
        "Ghost calendar",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "ghost"}),
    );
    let retired = new_task(
        &v,
        "Retired calendar",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "agent"}),
    );

    // Agent is hidden from Today until it is deleted. A deleted calendar then
    // falls back the same way an unknown one does.
    assert_eq!(keys(&today_items(&v)), want(&[(&ghost, None)]));
    assert_eq!(
        keys(&cal_items(&v)),
        want(&[(&ghost, None), (&retired, None)])
    );

    v.delete_calendar("agent".into()).unwrap();
    assert_eq!(
        keys(&today_items(&v)),
        want(&[(&ghost, None), (&retired, None)])
    );
    assert_eq!(
        keys(&cal_items(&v)),
        want(&[(&ghost, None), (&retired, None)])
    );
}

#[test]
fn dated_todos_and_mentions_ignore_the_calendar_switches() {
    let t = TempVault::new("independent");
    let v = t.open();
    let notes = v.create_page(None, "Notes".into()).unwrap();
    add(
        &v,
        &notes,
        dated_block(
            "todo-today",
            BlockKind::Todo,
            "Call @2026-10-06",
            &[("@2026-10-06", "2026-10-06")],
        ),
    );
    add(
        &v,
        &notes,
        dated_block(
            "todo-late",
            BlockKind::Todo,
            "Pay @2026-10-05",
            &[("@2026-10-05", "2026-10-05")],
        ),
    );
    add(
        &v,
        &notes,
        dated_block(
            "mention",
            BlockKind::Paragraph,
            "Dentist @2026-10-06",
            &[("@2026-10-06", "2026-10-06")],
        ),
    );
    add(
        &v,
        &notes,
        dated_block(
            "mention-late",
            BlockKind::Paragraph,
            "Old @2026-10-05",
            &[("@2026-10-05", "2026-10-05")],
        ),
    );

    // Hide every category from both surfaces.
    for c in default_calendars() {
        v.update_calendar(CalendarCategory {
            show_in_calendar: false,
            show_in_today: false,
            ..c
        })
        .unwrap();
    }

    assert_eq!(
        keys(&today_items(&v)),
        want(&[(&notes, Some("todo-today"))])
    );
    assert_eq!(
        keys(&overdue_items(&v)),
        want(&[(&notes, Some("todo-late"))])
    );
    assert_eq!(overdue_total(&v), 1);
    assert_eq!(
        keys(&cal_items(&v)),
        want(&[
            (&notes, Some("mention")),
            (&notes, Some("mention-late")),
            (&notes, Some("todo-late")),
            (&notes, Some("todo-today")),
        ])
    );
}

#[test]
fn database_calendar_switch_defaults_on_and_ignores_the_agenda_role() {
    let t = TempVault::new("database");
    let v = t.open();
    let db = v.create_page(None, "Chores".into()).unwrap();
    v.set_schema(db.clone(), vec![def("date", PropKind::Date), status_def()])
        .unwrap();
    let row = v.create_page(Some(db.clone()), "Vacuum".into()).unwrap();
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

    // The switch is absent, so the database is visible in the calendar.
    v.set_agenda_settings(
        db.clone(),
        AgendaSettings {
            role: AgendaRole::Off,
            date_prop: None,
        },
    )
    .unwrap();
    assert!(v.list_calendars().unwrap().databases.is_empty());
    assert!(has(&cal_items(&v), &row, None));

    // Hiding the database from the calendar leaves its role alone.
    v.set_database_calendar_visibility(db.clone(), false)
        .unwrap();
    assert_eq!(
        v.list_calendars().unwrap().databases,
        vec![CalendarDatabase {
            id: db.clone(),
            show_in_calendar: false,
        }]
    );
    assert!(!has(&cal_items(&v), &row, None));
    assert_eq!(v.agenda_settings(db.clone()).unwrap().role, AgendaRole::Off);

    // A role change does not reset the switch, and Today is not gated by it.
    v.set_agenda_settings(
        db.clone(),
        AgendaSettings {
            role: AgendaRole::Tasks,
            date_prop: None,
        },
    )
    .unwrap();
    assert_eq!(
        v.list_calendars().unwrap().databases,
        vec![CalendarDatabase {
            id: db.clone(),
            show_in_calendar: false,
        }]
    );
    assert!(!has(&cal_items(&v), &row, None));
    assert!(has(&today_items(&v), &row, None));

    v.set_database_calendar_visibility(db.clone(), true)
        .unwrap();
    assert!(has(&cal_items(&v), &row, None));
}

#[test]
fn calendars_page_is_hidden_from_tree_list_and_search() {
    let t = TempVault::new("hidden");
    let v = t.open();
    let shown = v.create_page(None, "Shown page".into()).unwrap();
    v.list_calendars().unwrap();
    let hidden = v
        .find_by_role("calendars".into())
        .unwrap()
        .expect("calendars page");
    let title = v.load_page(hidden.clone()).unwrap().title;

    let top: Vec<String> = v
        .list_pages(None, 0, 100)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert!(top.contains(&shown), "visible control missing from list");
    assert!(!top.contains(&hidden), "calendars page listed");

    let children: Vec<String> = v
        .list_children(None, 0, 100)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert!(children.contains(&shown));
    assert!(!children.contains(&hidden), "calendars page in the tree");

    let hits: Vec<String> = v
        .search(title.clone(), 50)
        .unwrap()
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert!(!hits.contains(&hidden), "calendars page found by search");

    let page_hits: Vec<String> = v
        .search_pages(title, 50)
        .unwrap()
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert!(
        !page_hits.contains(&hidden),
        "calendars page in page search"
    );
}

#[test]
fn calendars_page_is_left_out_of_the_page_count() {
    let t = TempVault::new("count");
    let v = t.open();
    v.create_page(None, "Shown page".into()).unwrap();
    let before = v.page_count().unwrap();

    v.list_calendars().unwrap();
    assert!(
        v.find_by_role("calendars".into()).unwrap().is_some(),
        "calendars page missing"
    );
    assert_eq!(v.page_count().unwrap(), before, "calendars page counted");

    v.create_page(None, "Another page".into()).unwrap();
    assert_eq!(v.page_count().unwrap(), before + 1);
}

/// The deterministic system page that every device creates the same way.
const CALENDARS_PAGE: &str = "000000000000000000000000CA";

/// A task under `parent`, so its space colour comes from the top-level page.
fn new_in(v: &Vault, parent: &str, title: &str, fields: Value) -> PageId {
    let page = v.create_page(Some(parent.into()), title.into()).unwrap();
    set_task(v, &page, fields);
    page
}

fn space(v: &Vault, title: &str, color: &str) -> PageId {
    let page = v.create_page(None, title.into()).unwrap();
    v.set_page_color(page.clone(), Some(color.into())).unwrap();
    page
}

fn space_color_of(items: &[AgendaItem], page: &str) -> Option<String> {
    items
        .iter()
        .find(|i| i.page == page && i.block.is_none())
        .and_then(|i| i.space_color.clone())
}

/// The persisted `tasks.calendars` ids, read without the on-access dedup.
fn persisted_ids(v: &Vault) -> Vec<String> {
    let data = task_data(v, CALENDARS_PAGE);
    let mut ids: Vec<String> = data["calendars"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

#[test]
fn known_calendar_overrides_the_space_colour() {
    let t = TempVault::new("override");
    let v = t.open();
    let home = space(&v, "Home", "moss");
    let errand = new_in(
        &v,
        &home,
        "Errand",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "personal"}),
    );
    let office = new_in(
        &v,
        &home,
        "Office",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "work"}),
    );
    let delegated = new_in(
        &v,
        &home,
        "Delegated",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "agent"}),
    );
    let plain = new_in(
        &v,
        &home,
        "No calendar",
        json!({"date": "2026-10-06", "status": "todo"}),
    );

    let items = cal_items(&v);
    assert_eq!(space_color_of(&items, &errand).as_deref(), Some("apricot"));
    assert_eq!(space_color_of(&items, &office).as_deref(), Some("sky"));
    assert_eq!(space_color_of(&items, &delegated).as_deref(), Some("duvet"));
    assert_eq!(space_color_of(&items, &plain).as_deref(), Some("moss"));

    // The same override applies on the Today agenda.
    let today = today_items(&v);
    assert_eq!(space_color_of(&today, &office).as_deref(), Some("sky"));
    assert_eq!(space_color_of(&today, &plain).as_deref(), Some("moss"));
}

#[test]
fn calendar_colour_change_takes_effect_on_the_next_query_without_flush() {
    let t = TempVault::new("recolour");
    let v = t.open();
    let home = space(&v, "Home", "moss");
    let office = new_in(
        &v,
        &home,
        "Office",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "work"}),
    );
    assert_eq!(
        space_color_of(&cal_items(&v), &office).as_deref(),
        Some("sky")
    );

    // No flush and no reindex between the edit and the query.
    v.update_calendar(cat("work", "Work", "paw", true, true))
        .unwrap();
    let items = v.calendar_items(FROM, TO, FROM, TO).unwrap();
    assert_eq!(space_color_of(&items, &office).as_deref(), Some("paw"));

    v.update_calendar(cat("work", "Work", "duvet", true, true))
        .unwrap();
    let today = v.agenda(DAY, DAY + DAY_MS, DAY, DAY + DAY_MS).unwrap();
    assert_eq!(space_color_of(&today, &office).as_deref(), Some("duvet"));
}

#[test]
fn deleted_unknown_or_missing_calendar_falls_back_to_the_space_colour() {
    let t = TempVault::new("fallback_colour");
    let v = t.open();
    let home = space(&v, "Home", "moss");
    let ghost = new_in(
        &v,
        &home,
        "Ghost",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "ghost"}),
    );
    let empty = new_in(
        &v,
        &home,
        "Empty id",
        json!({"date": "2026-10-06", "status": "todo", "calendar": ""}),
    );
    let plain = new_in(
        &v,
        &home,
        "Missing",
        json!({"date": "2026-10-06", "status": "todo"}),
    );
    let retired = new_in(
        &v,
        &home,
        "Retired",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "agent"}),
    );
    let office = new_in(
        &v,
        &home,
        "Office",
        json!({"date": "2026-10-06", "status": "todo", "calendar": "work"}),
    );

    let items = cal_items(&v);
    assert_eq!(space_color_of(&items, &ghost).as_deref(), Some("moss"));
    assert_eq!(space_color_of(&items, &empty).as_deref(), Some("moss"));
    assert_eq!(space_color_of(&items, &plain).as_deref(), Some("moss"));
    assert_eq!(space_color_of(&items, &retired).as_deref(), Some("duvet"));

    // Deleting a calendar reverts its tasks on the next query, without flush.
    v.delete_calendar("agent".into()).unwrap();
    v.delete_calendar("work".into()).unwrap();
    let items = v.calendar_items(FROM, TO, FROM, TO).unwrap();
    assert_eq!(space_color_of(&items, &retired).as_deref(), Some("moss"));
    assert_eq!(space_color_of(&items, &office).as_deref(), Some("moss"));
}

/// The same missing-version export/import helper used by `tests/agenda.rs`.
fn push(from: &Vault, to: &Vault) -> SyncImport {
    to.import_purged(from.purged_pages()).unwrap();
    let have: std::collections::HashMap<String, Vec<u8>> = to
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
fn two_devices_lazily_initialize_independently_and_converge() {
    let (ta, tb) = (TempVault::new("device_a"), TempVault::new("device_b"));
    let (a, b) = (ta.open(), tb.open());
    assert_eq!(a.find_by_role("calendars".into()).unwrap(), None);
    assert_eq!(b.find_by_role("calendars".into()).unwrap(), None);

    // Both devices initialize before any sync.
    assert_eq!(a.list_calendars().unwrap(), default_settings());
    assert_eq!(b.list_calendars().unwrap(), default_settings());

    push(&a, &b);
    push(&b, &a);
    // Sync's metadata hook also keeps persisted logical ids unique.
    assert_eq!(persisted_ids(&a), vec!["agent", "personal", "work"]);
    assert_eq!(persisted_ids(&b), vec!["agent", "personal", "work"]);

    // Listing does not introduce new defaults or duplicates.
    for v in [&a, &b] {
        assert_eq!(v.list_calendars().unwrap(), default_settings());
        assert_eq!(
            v.find_by_role("calendars".into()).unwrap(),
            Some(CALENDARS_PAGE.into())
        );
        assert_eq!(persisted_ids(v), vec!["agent", "personal", "work"]);
    }

    // A second merge and list round changes nothing.
    for _ in 0..2 {
        push(&a, &b);
        push(&b, &a);
        for v in [&a, &b] {
            assert_eq!(v.list_calendars().unwrap(), default_settings());
            assert_eq!(
                v.find_by_role("calendars".into()).unwrap(),
                Some(CALENDARS_PAGE.into())
            );
            assert_eq!(persisted_ids(v), vec!["agent", "personal", "work"]);
        }
    }
}

#[test]
fn an_existing_index_with_today_off_rows_omitted_is_repaired_on_open() {
    let t = TempVault::new("legacy_off");
    let row;
    {
        let v = t.open();
        let db = v.create_page(None, "Old database".into()).unwrap();
        v.set_schema(db.clone(), vec![def("date", PropKind::Date)])
            .unwrap();
        row = v
            .create_page(Some(db.clone()), "Existing event".into())
            .unwrap();
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
        v.set_agenda_settings(
            db,
            AgendaSettings {
                role: AgendaRole::Off,
                date_prop: None,
            },
        )
        .unwrap();
        assert!(has(&cal_items(&v), &row, None));
        assert!(today_items(&v).is_empty());
        v.flush().unwrap();
    }
    // Simulate the previous derivation's persisted index; documents and the
    // schema version are unchanged, so reopening must repair the missing row.
    rusqlite::Connection::open(t.0.join(".vault/index.sqlite"))
        .unwrap()
        .execute("DELETE FROM agenda WHERE source = 0 AND block = ''", [])
        .unwrap();
    let reopened = t.open();
    assert!(has(&cal_items(&reopened), &row, None));
    assert!(today_items(&reopened).is_empty());
}

#[test]
fn category_visibility_is_applied_before_caps_and_changes_without_flush() {
    let t = TempVault::new("caps");
    let v = t.open();
    let visible = new_task(
        &v,
        "Visible",
        json!({
            "date": "2026-10-05", "status": "todo", "calendar": "work",
        }),
    );
    let done = new_task(
        &v,
        "Visible done",
        json!({
            "date": "2026-10-05", "status": "done", "calendar": "work",
        }),
    );
    new_task(
        &v,
        "Hidden oldest",
        json!({
            "date": "2026-10-04", "status": "todo", "calendar": "agent",
        }),
    );
    new_task(
        &v,
        "Hidden newest done",
        json!({
            "date": "2026-10-05", "status": "done", "calendar": "agent",
        }),
    );
    v.flush().unwrap();
    assert_eq!(
        keys(&v.overdue(DAY, DAY, 1).unwrap()),
        want(&[(&visible, None)])
    );
    assert_eq!(
        keys(&v.done_since(DAY, DAY, 0, 1).unwrap()),
        want(&[(&done, None)])
    );
    assert_eq!(v.overdue_count(DAY, DAY).unwrap(), 1);

    v.update_calendar(cat("work", "Work", "sky", false, false))
        .unwrap();
    assert!(v.overdue(DAY, DAY, 1).unwrap().is_empty());
    assert!(v.done_since(DAY, DAY, 0, 1).unwrap().is_empty());
    assert_eq!(v.overdue_count(DAY, DAY).unwrap(), 0);
    let items = v.calendar_items(FROM, TO, FROM, TO).unwrap();
    assert!(!has(&items, &visible, None));
    assert!(!has(&items, &done, None));
}

#[test]
fn hidden_role_is_applied_before_pagination_and_search_ranking() {
    let t = TempVault::new("hidden_caps");
    let v = t.open();
    let shown = v.create_page(None, "Calendar ledger".into()).unwrap();
    let hidden = v
        .create_page(None, "Calendar ledger system".into())
        .unwrap();
    // Populate the title cache before changing role, then hide its newest row.
    assert!(!v
        .search_pages("Calendar ledger".into(), 10)
        .unwrap()
        .is_empty());
    v.set_page_role(hidden.clone(), Some("calendars".into()))
        .unwrap();
    v.flush().unwrap();
    assert_eq!(v.list_pages(None, 0, 1).unwrap()[0].id, shown);
    assert_eq!(v.list_children(None, 0, 1).unwrap()[0].id, shown);
    assert_eq!(
        v.search_pages("Calendar ledger".into(), 1).unwrap()[0].id,
        shown
    );
    assert_eq!(v.search("Calendar ledger".into(), 1).unwrap()[0].id, shown);
    for query in ["Calender ledger", "Calendar ledgr"] {
        assert!(v
            .search_pages(query.into(), 10)
            .unwrap()
            .iter()
            .all(|h| h.id != hidden));
    }
}

#[test]
fn hidden_calendar_settings_do_not_shift_the_visible_space_colour_cycle() {
    let t = TempVault::new("space_cycle");
    let v = t.open();
    let mut spaces = Vec::new();
    for title in ["Home", "Office", "Hobbies"] {
        let page = v.create_page(None, title.into()).unwrap();
        v.set_page_color(page.clone(), None).unwrap();
        spaces.push(page);
    }
    v.list_calendars().unwrap();
    assert_eq!(v.ensure_space_colors().unwrap(), 3);
    for (page, color) in spaces.iter().zip(["apricot", "paw", "moss"]) {
        assert_eq!(
            v.load_page(page.clone()).unwrap().color.as_deref(),
            Some(color)
        );
    }
}
