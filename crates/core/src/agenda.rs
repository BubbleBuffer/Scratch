//! Agenda and progress derived from ordinary page documents.
use crate::error::{invalid, not_found, Result};
use crate::ids::{journal_day_id, new_id, now_ms};
use crate::index::{Index, NOT_TEMPLATE};
use crate::types::*;
use crate::vault::{lock, Vault};
use chrono::{Datelike, Local, TimeZone, Utc};
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet};

// Shared by the capped list and count so eligibility and date bounds match.
const OVERDUE_PREDICATE: &str = "a.d0 < max(?1,?2) AND a.kind = 0 AND a.done = 0 AND ((a.has_time = 1 AND coalesce(a.d1,a.d0) < ?1) OR (a.has_time = 0 AND coalesce(a.d1,a.d0) < ?2))";
const DONE_SINCE_PREDICATE: &str = "a.d0 < max(?1,?2) AND a.kind = 0 AND a.done = 1 AND p.updated_ms >= ?3 AND ((a.has_time = 1 AND coalesce(a.d1,a.d0) < ?1) OR (a.has_time = 0 AND coalesce(a.d1,a.d0) < ?2))";

pub(crate) const SPACE_COLORS: [&str; 6] = ["apricot", "paw", "moss", "sun", "sky", "duvet"];
const MENTION_TITLE_CHARS: usize = 200;
// Separate from the hidden Calendars system document. Legacy Journals retain
// their existing key; only a newly initialized Journal uses this identity.
const JOURNAL_PAGE: &str = "000000000000000000000000JR";

/// The agenda rows tools and item searches see: every source except the
/// Journal's own day pages.
pub(crate) const ALL_AGENDA_SOURCES: &str =
    "(a.source != 0 OR a.block != '' OR c.role IS NOT 'journal')";
/// Stable item order: dated first by date, then page, block and source.
pub(crate) const AGENDA_ORDER: &str = "a.d0 IS NULL,a.d0,p.id,a.block,a.source";

/// A tool-facing agenda source, including tasks without a date. `date_ms`
/// is authoritative: `item.start_ms` is zero for undated entries solely to
/// preserve the existing dated-only UI model.
#[derive(Debug, Clone)]
pub struct AgendaEntry {
    pub item: AgendaItem,
    pub date_ms: Option<i64>,
    /// `todo`, `task_note`, `row`, or `mention`.
    pub source: String,
    pub calendar_name: Option<String>,
    /// Eligibility for Today before category visibility is applied.
    pub today_eligible: bool,
}

/// The same settings snapshot drives visibility and colour. Apply visibility
/// in SQL before LIMIT, including counts. Blocks never inherit category flags.
fn calendar_visibility(parameter: usize, calendar: bool) -> String {
    let switch = if calendar {
        "show_in_calendar"
    } else {
        "show_in_today"
    };
    let notes = format!("(a.source != 1 OR NOT EXISTS (SELECT 1 FROM json_each(?{parameter}, '$.calendars') cat WHERE json_extract(cat.value, '$.id') = a.calendar AND json_extract(cat.value, '$.{switch}') = 0))");
    if calendar {
        format!("{notes} AND (a.block != '' OR c.role IS NOT 'journal') AND (a.source != 0 OR a.block != '' OR NOT EXISTS (SELECT 1 FROM json_each(?{parameter}, '$.databases') d WHERE json_extract(d.value, '$.id') = c.id AND json_extract(d.value, '$.show_in_calendar') = 0))")
    } else {
        format!("{notes} AND (a.source != 0 OR a.block != '' OR c.agenda_role IS NOT 'off')")
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BlockAgenda {
    pub total: u32,
    pub done: u32,
    pub rows: Vec<DatedBlock>,
    task: Option<TaskNote>,
    empty_task_data: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct DatedBlock {
    block: String,
    date: Option<i64>,
    has_time: bool,
    done: bool,
    title: String,
    kind: AgendaKind,
}

#[derive(Debug, Clone)]
struct TaskNote {
    date: Option<i64>,
    end: Option<i64>,
    has_time: bool,
    status: String,
    calendar: Option<String>,
    assignee: Option<String>,
}

fn empty_task_data(value: &serde_json::Value) -> bool {
    value.is_null() || value.as_object().is_some_and(|fields| fields.is_empty())
}

fn default_task_note() -> TaskNote {
    TaskNote {
        date: None,
        end: None,
        has_time: false,
        status: "todo".into(),
        calendar: None,
        assignee: Some("me".into()),
    }
}

fn task_note(value: &serde_json::Value) -> Option<TaskNote> {
    // An absent or explicitly cleared date is an undated task. An empty
    // add-on object is ordinary page data, rather than a task note.
    let fields = value.as_object()?;
    if ![
        "date",
        "status",
        "calendar",
        "assignee",
        "duration_min",
        "checkbox",
    ]
    .iter()
    .any(|key| fields.contains_key(*key))
    {
        return None;
    }
    let (date, has_time) = match value.get("date") {
        None | Some(serde_json::Value::Null) => (None, false),
        Some(serde_json::Value::String(iso)) if iso.is_empty() => (None, false),
        Some(serde_json::Value::String(iso)) => {
            let (date, has_time) = mention_date(iso)?;
            if has_time && !iso.ends_with('Z') {
                return None;
            }
            (Some(date), has_time)
        }
        _ => return None,
    };
    // Malformed, negative or overflowing durations leave a point item.
    let end = date.map(|date| {
        if has_time {
            value
                .get("duration_min")
                .and_then(serde_json::Value::as_i64)
                .filter(|n| *n >= 0)
                .and_then(|n| n.checked_mul(60_000))
                .and_then(|n| date.checked_add(n))
                .unwrap_or(date)
        } else {
            date
        }
    });
    Some(TaskNote {
        date,
        end,
        has_time,
        status: value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .filter(|s| matches!(*s, "todo" | "doing" | "done"))
            .unwrap_or("todo")
            .into(),
        calendar: value
            .get("calendar")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        assignee: value
            .get("assignee")
            .and_then(serde_json::Value::as_str)
            .filter(|s| matches!(*s, "me" | "agent"))
            .map(str::to_owned),
    })
}

/// Every page reindex (writer, import, sync and rebuild) derives the same facts.
pub(crate) fn page_agenda(blocks: &[Block], type_data: &str) -> BlockAgenda {
    let mut out = block_agenda(blocks);
    // The document projection stores absent add-on data as an empty string,
    // whereas the public type_data API returns an empty JSON object.
    if type_data.is_empty() {
        out.empty_task_data = true;
    } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(type_data) {
        out.task = task_note(&value["tasks"]);
        out.empty_task_data = empty_task_data(&value["tasks"]);
    }
    out
}

/// Block facts are collected once when content is indexed. Metadata writes
/// retain these rows and recompute the database row from indexed properties.
pub(crate) fn block_agenda(blocks: &[Block]) -> BlockAgenda {
    fn visit(blocks: &[Block], out: &mut BlockAgenda) {
        for b in blocks {
            if b.kind == BlockKind::Todo {
                out.total += 1;
                out.done += u32::from(b.checked);
            }
            let date = selected_date_mark(b);
            if let Some((m, date, has_time)) = date {
                let title: String = b
                    .text
                    .chars()
                    .enumerate()
                    .filter(|(i, _)| *i < m.start as usize || *i >= m.end as usize)
                    .map(|(_, c)| c)
                    .collect();
                out.rows.push(DatedBlock {
                    block: b.id.clone(),
                    date: Some(date),
                    has_time,
                    done: b.kind == BlockKind::Todo && b.checked,
                    title: if b.kind == BlockKind::Todo {
                        title.trim().to_string()
                    } else {
                        title.trim().chars().take(MENTION_TITLE_CHARS).collect()
                    },
                    kind: if b.kind == BlockKind::Todo {
                        AgendaKind::Task
                    } else {
                        AgendaKind::Mention
                    },
                });
            } else if b.kind == BlockKind::Todo {
                out.rows.push(DatedBlock {
                    block: b.id.clone(),
                    date: None,
                    has_time: false,
                    done: b.checked,
                    title: b.text.trim().to_owned(),
                    kind: AgendaKind::Task,
                });
            }
            visit(&b.children, out);
        }
    }
    let mut out = BlockAgenda::default();
    visit(blocks, &mut out);
    out
}

fn selected_date_mark(block: &Block) -> Option<(&Mark, i64, bool)> {
    block.marks.iter().find_map(|m| match &m.kind {
        MarkKind::Date(s) => mention_date(s).map(|(date, timed)| (m, date, timed)),
        _ => None,
    })
}

fn find_dated_block<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|b| {
        if b.id == id {
            Some(b)
        } else {
            find_dated_block(&b.children, id)
        }
    })
}

fn date_mark_ref(block: &Block) -> Option<(DateMarkRef, i64, bool)> {
    if block.kind != BlockKind::Todo {
        return None;
    }
    let (mark, date, timed) = selected_date_mark(block)?;
    if mark.start >= mark.end || mark.end as usize > block.text.chars().count() {
        return None;
    }
    let MarkKind::Date(iso) = &mark.kind else {
        return None;
    };
    Some((
        DateMarkRef {
            start: mark.start,
            end: mark.end,
            iso: iso.clone(),
        },
        date,
        timed,
    ))
}

// Date mentions share the markdown UTC storage convention. Keep the stricter
// calendar validation used for agenda dates, including timed mentions.
fn mention_date(s: &str) -> Option<(i64, bool)> {
    iso_day(s.get(..10)?)?;
    if s.len() == 10 {
        return iso_day(s).map(|ms| (ms, false));
    }
    let suffix = s.get(10..)?.strip_prefix('T')?;
    let suffix = suffix.strip_suffix('Z').unwrap_or(suffix);
    let mut parts = suffix.split(':');
    let hour = parts.next()?;
    let minute = parts.next()?;
    if hour.len() != 2
        || minute.len() != 2
        || !hour
            .bytes()
            .chain(minute.bytes())
            .all(|c| c.is_ascii_digit())
        || hour.parse::<u32>().ok()? > 23
        || minute.parse::<u32>().ok()? > 59
    {
        return None;
    }
    if let Some(second) = parts.next() {
        let (second, millis) = second
            .split_once('.')
            .map_or((second, None), |(s, m)| (s, Some(m)));
        if second.len() != 2
            || !second.bytes().all(|c| c.is_ascii_digit())
            || second.parse::<u32>().ok()? > 59
            || millis.is_some_and(|m| m.len() != 3 || !m.bytes().all(|c| c.is_ascii_digit()))
        {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    crate::mdparse::parse_iso(s)
}

fn iso_day(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let m: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=days).contains(&d) {
        return None;
    }
    Some(crate::markdown::days_from_civil(y, m, d) * 86_400_000)
}

/// Daily-note identity is an all-day Date value, never a title or agenda row.
fn journal_day(json: &str) -> Option<i64> {
    match serde_json::from_str::<PropValue>(json).ok()? {
        PropValue::Date {
            start_ms,
            has_time: false,
            ..
        } => Some(start_ms),
        _ => None,
    }
}

/// Title for a new day note, such as "Tuesday 6 October 2026".
fn day_title(day_ms: i64) -> Result<String> {
    // Day stamps encode the calendar date at UTC midnight.
    let day = Utc
        .timestamp_millis_opt(day_ms)
        .single()
        .ok_or_else(|| invalid("invalid daily-note date"))?;
    Ok(format!(
        "{} {} {} {}",
        day.format("%A"),
        day.day(),
        day.format("%B"),
        day.year()
    ))
}

fn toggle_block(id: String, text: &str) -> NewBlock {
    NewBlock {
        id,
        kind: BlockKind::Toggle,
        text: text.into(),
        marks: vec![],
        checked: false,
    }
}

/// Whether the blocks, or any block below them, link to `page` with a
/// `LinkToPage` block or a `PageLink` mark on their text.
fn links_page(blocks: &[Block], page: &str) -> bool {
    blocks.iter().any(|b| {
        (b.kind == BlockKind::LinkToPage && b.target.as_deref() == Some(page))
            || b.marks
                .iter()
                .any(|m| matches!(&m.kind, MarkKind::PageLink(id) if id.as_str() == page))
            || links_page(&b.children, page)
    })
}

impl AgendaRole {
    fn stored(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Tasks => "tasks",
            Self::Events => "events",
            Self::Off => "off",
        }
    }
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("tasks") => Self::Tasks,
            Some("events") => Self::Events,
            Some("off") => Self::Off,
            _ => Self::Auto,
        }
    }
}

fn effective_role(schema: &[PropDef], role: AgendaRole) -> AgendaRole {
    if role != AgendaRole::Auto {
        return role;
    }
    if !schema.iter().any(|d| d.kind == PropKind::Date) {
        AgendaRole::Off
    } else if schema
        .iter()
        .any(|d| matches!(d.kind, PropKind::Status | PropKind::Checkbox))
    {
        AgendaRole::Tasks
    } else {
        AgendaRole::Events
    }
}
fn date_def<'a>(schema: &'a [PropDef], key: Option<&str>) -> Option<&'a PropDef> {
    key.and_then(|key| schema.iter().find(|d| d.key == key))
        .or_else(|| schema.iter().find(|d| d.kind == PropKind::Date))
}
fn done_def(schema: &[PropDef]) -> Option<&PropDef> {
    schema
        .iter()
        .find(|d| d.kind == PropKind::Status)
        .or_else(|| schema.iter().find(|d| d.kind == PropKind::Checkbox))
}

impl Index {
    fn agenda_config(&self, id: &str) -> Result<Option<(Vec<PropDef>, AgendaSettings)>> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self.conn.prepare_cached(
            "SELECT schema, agenda_role, agenda_date FROM pages WHERE id = ?1 AND page_type = 'database'")?
            .query_row([id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        Ok(row.map(|(schema, role, date_prop)| {
            (
                serde_json::from_str(schema.as_deref().unwrap_or("[]")).unwrap_or_default(),
                AgendaSettings {
                    role: AgendaRole::parse(role.as_deref()),
                    date_prop,
                },
            )
        }))
    }
    fn agenda_prop(&self, rowid: i64, key: &str) -> Result<PropValue> {
        let json: Option<String> = self
            .conn
            .prepare_cached("SELECT json FROM props WHERE page = ?1 AND key = ?2")?
            .query_row(params![rowid, key], |r| r.get(0))
            .optional()?;
        Ok(json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or(PropValue::Empty))
    }
    /// The one derivation path used by incremental indexing and bulk rebuilds.
    pub(crate) fn refresh_agenda(&self, id: &str, blocks: Option<&BlockAgenda>) -> Result<()> {
        // DELETE + INSERT must be indivisible across the foreground and writer
        // connections. Retain an enclosing metadata, writer or rebuild transaction.
        let tx = self
            .conn
            .is_autocommit()
            .then(|| self.write_tx())
            .transpose()?;
        self.refresh_agenda_rows(id, blocks)?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(())
    }

    fn refresh_agenda_rows(&self, id: &str, blocks: Option<&BlockAgenda>) -> Result<()> {
        let row: Option<(i64, String, Option<String>, String)> = self
            .conn
            .prepare_cached("SELECT rowid,title,parent,page_type FROM pages WHERE id = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional()?;
        let Some((rowid, title, parent, page_type)) = row else {
            return Ok(());
        };
        if let Some(blocks) = blocks {
            self.conn
                .prepare_cached(
                    "UPDATE pages SET todos_total = ?2, todos_done = ?3 WHERE rowid = ?1",
                )?
                .execute(params![rowid, blocks.total, blocks.done])?;
            self.conn
                .prepare_cached(
                    "DELETE FROM agenda WHERE page = ?1 AND (block != '' OR source = 1)",
                )?
                .execute([rowid])?;
            let mut st = self.conn.prepare_cached("INSERT INTO agenda(page,block,kind,d0,d1,has_time,done,title) VALUES (?1,?2,?3,?4,NULL,?5,?6,?7)")?;
            for b in &blocks.rows {
                st.execute(params![
                    rowid,
                    b.block,
                    if b.kind == AgendaKind::Task { 0 } else { 2 },
                    b.date,
                    b.has_time,
                    b.done,
                    b.title
                ])?;
            }
            // A task page can be persisted before its composer writes the
            // add-on fields. The app renders it with these defaults already.
            let fallback = (page_type == "task" && blocks.empty_task_data).then(default_task_note);
            if let Some(task) = blocks.task.as_ref().or(fallback.as_ref()) {
                // Source 1 keeps the task note independent of the page's
                // database date row (source 0), even with block = ''.
                self.conn.prepare_cached("INSERT INTO agenda(page,block,source,kind,d0,d1,has_time,done,title,task_status,calendar,assignee) VALUES (?1,'',1,0,?2,?3,?4,?5,?6,?7,?8,?9)")?
                    .execute(params![rowid, task.date, task.end, task.has_time, task.status == "done", title, task.status, task.calendar, task.assignee])?;
            }
        }
        // Metadata refreshes retain the task fields derived by the writer.
        self.conn
            .prepare_cached("UPDATE agenda SET title = ?2 WHERE page = ?1 AND source = 1")?
            .execute(params![rowid, title])?;
        // In bulk mode parents may not be present yet; the final pass derives rows.
        if self.defer.get() {
            return Ok(());
        }
        self.conn
            .prepare_cached("DELETE FROM agenda WHERE page = ?1 AND block = '' AND source = 0")?
            .execute([rowid])?;
        let mut task_done: Option<bool> = None;
        if let Some(parent) = parent {
            if let Some((schema, settings)) = self.agenda_config(&parent)? {
                // Off disables Today, while the independent calendar switch
                // still permits rows. Infer their kind from the schema.
                let today_role = effective_role(&schema, settings.role);
                let role = effective_role(
                    &schema,
                    if settings.role == AgendaRole::Off {
                        AgendaRole::Auto
                    } else {
                        settings.role
                    },
                );
                if matches!(role, AgendaRole::Tasks | AgendaRole::Events) {
                    let done = if role == AgendaRole::Tasks {
                        let value = match done_def(&schema) {
                            Some(d) => self.agenda_prop(rowid, &d.key)?,
                            None => PropValue::Empty,
                        };
                        match value {
                            PropValue::Status(option) => done_def(&schema)
                                .is_some_and(|d| status_group(d, &option) == StatusGroup::Complete),
                            PropValue::Checkbox(done) => done,
                            _ => false,
                        }
                    } else {
                        false
                    };
                    if today_role == AgendaRole::Tasks {
                        task_done = Some(done);
                    }
                    if let Some(date) = date_def(&schema, settings.date_prop.as_deref()) {
                        if let PropValue::Date {
                            start_ms,
                            end_ms,
                            has_time,
                        } = self.agenda_prop(rowid, &date.key)?
                        {
                            self.conn.prepare_cached("INSERT INTO agenda(page,block,kind,d0,d1,has_time,done,title) VALUES (?1,'',?2,?3,?4,?5,?6,?7)")?
                                .execute(params![rowid,i64::from(role == AgendaRole::Events),start_ms,end_ms,has_time,done,title])?;
                        }
                    }
                }
            }
        }
        self.conn
            .prepare_cached("UPDATE pages SET task_done = ?2 WHERE rowid = ?1")?
            .execute(params![rowid, task_done])?;
        Ok(())
    }
    pub(crate) fn refresh_collection_agenda(&self, collection: &str) -> Result<()> {
        for id in self.child_ids(collection)? {
            self.refresh_agenda(&id, None)?;
        }
        Ok(())
    }
    pub(crate) fn refresh_all_agenda(&self) -> Result<()> {
        let mut st = self.conn.prepare_cached("SELECT id FROM pages")?;
        let ids = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            self.refresh_agenda(&id, None)?;
        }
        Ok(())
    }
    fn agenda_items(
        &self,
        predicate: &str,
        bounds: &[i64],
        ordering: &str,
        limit: Option<u32>,
        settings: &CalendarSettings,
        calendar: bool,
    ) -> Result<Vec<AgendaItem>> {
        Ok(self
            .agenda_entries(predicate, bounds, ordering, limit, settings, calendar, true)?
            .into_iter()
            .map(|entry| entry.item)
            .collect())
    }

    pub(crate) fn agenda_entries(
        &self,
        predicate: &str,
        bounds: &[i64],
        ordering: &str,
        limit: Option<u32>,
        settings: &CalendarSettings,
        calendar: bool,
        respect_visibility: bool,
    ) -> Result<Vec<AgendaEntry>> {
        // Keep the date-filtered agenda outermost. Otherwise SQLite can
        // choose the live-pages index and probe every page in the vault.
        // Apply the cap in SQL, before decoding results and resolving spaces.
        let cap = limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
        let visibility = if respect_visibility {
            calendar_visibility(bounds.len() + 1, calendar)
        } else {
            "1".to_owned()
        };
        let sql = format!(
            "SELECT p.id,a.block,a.title,a.kind,a.d0,a.d1,a.has_time,a.done,
            CASE WHEN c.page_type = 'database' AND a.source = 0 THEN c.id END,p.parent,p.icon,p.updated_ms,p.rowid,
            a.task_status,a.calendar,a.assignee,a.source,
            a.kind != 2 AND (a.source != 0 OR a.block != '' OR c.agenda_role IS NOT 'off')
            FROM agenda a CROSS JOIN pages p ON p.rowid = a.page LEFT JOIN pages c ON c.id = p.parent
            WHERE p.in_trash = 0 AND {NOT_TEMPLATE} AND p.id IS NOT c.template
              AND ({predicate}) AND {visibility} ORDER BY {ordering}{cap}"
        );
        let mut values: Vec<rusqlite::types::Value> =
            bounds.iter().copied().map(Into::into).collect();
        if respect_visibility {
            values.push(serde_json::to_string(settings).unwrap().into());
        }
        let mut st = self.conn.prepare_cached(&sql)?;
        let mut items = st
            .query_map(rusqlite::params_from_iter(values), |r| {
                let block: String = r.get(1)?;
                let date_ms: Option<i64> = r.get(4)?;
                let source = if block.is_empty() {
                    if r.get::<_, i64>(16)? == 1 {
                        "task_note"
                    } else {
                        "row"
                    }
                } else if r.get::<_, i64>(3)? == 2 {
                    "mention"
                } else {
                    "todo"
                };
                Ok((
                    AgendaEntry {
                        date_ms,
                        source: source.into(),
                        calendar_name: None,
                        today_eligible: r.get(17)?,
                        item: AgendaItem {
                            page: r.get(0)?,
                            block: (!block.is_empty()).then_some(block),
                            date_mark: None,
                            title: r.get(2)?,
                            kind: match r.get::<_, i64>(3)? {
                                0 => AgendaKind::Task,
                                2 => AgendaKind::Mention,
                                _ => AgendaKind::Event,
                            },
                            start_ms: date_ms.unwrap_or_default(),
                            end_ms: r.get(5)?,
                            has_time: r.get(6)?,
                            done: r.get(7)?,
                            status: r.get(13)?,
                            status_group: r.get::<_, Option<String>>(13)?.map(|s| {
                                match s.as_str() {
                                    "doing" => "progress",
                                    "done" => "complete",
                                    _ => "todo",
                                }
                                .into()
                            }),
                            status_color: None,
                            calendar: r.get(14)?,
                            assignee: r.get(15)?,
                            collection: r.get(8)?,
                            space: None,
                            space_color: None,
                            icon: r.get(10)?,
                            updated_ms: r.get(11)?,
                        },
                    },
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, i64>(12)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut spaces = Spaces::new(self);
        let colors: HashMap<_, _> = settings
            .calendars
            .iter()
            .map(|c| (c.id.as_str(), c.color.as_str()))
            .collect();
        let mut status_defs: HashMap<String, Option<PropDef>> = HashMap::new();
        for (entry, parent, rowid) in &mut items {
            let item = &mut entry.item;
            entry.calendar_name = item.calendar.as_ref().and_then(|id| {
                settings
                    .calendars
                    .iter()
                    .find(|calendar| calendar.id == *id)
                    .map(|calendar| calendar.name.clone())
            });
            let (space, color) = spaces.of(&item.page, parent.as_deref())?;
            item.space = space;
            item.space_color = color;
            if let Some(color) = item.calendar.as_deref().and_then(|id| colors.get(id)) {
                item.space_color = Some((*color).to_owned());
            }
            if item.block.is_some() {
                continue;
            }
            let Some(collection) = item.collection.as_ref() else {
                continue;
            };
            // Status values already live in the property index. Resolve each
            // collection's definition once, after applying the query's cap.
            if !status_defs.contains_key(collection) {
                let def = self.agenda_config(collection)?.and_then(|(schema, _)| {
                    schema.into_iter().find(|def| def.kind == PropKind::Status)
                });
                status_defs.insert(collection.clone(), def);
            }
            let Some(def) = status_defs.get(collection).and_then(Option::as_ref) else {
                continue;
            };
            let PropValue::Status(option_id) = self.agenda_prop(*rowid, &def.key)? else {
                continue;
            };
            let Some(option) = def.options.iter().find(|option| option.id == option_id) else {
                continue;
            };
            item.status = Some(option.name.clone());
            item.status_group = Some(
                match status_group(def, &option_id) {
                    StatusGroup::Todo => "todo",
                    StatusGroup::InProgress => "progress",
                    StatusGroup::Complete => "complete",
                }
                .into(),
            );
            item.status_color = (!option.color.is_empty()).then(|| option.color.clone());
        }
        Ok(items.into_iter().map(|(item, _, _)| item).collect())
    }
}

type Space = (Option<String>, Option<String>);
/// Cache each shared ancestor once, rather than walking parents per result.
struct Spaces<'a> {
    db: &'a Index,
    cache: HashMap<String, Space>,
}
impl<'a> Spaces<'a> {
    fn new(db: &'a Index) -> Self {
        Self {
            db,
            cache: HashMap::new(),
        }
    }
    fn of(&mut self, page: &str, parent: Option<&str>) -> Result<Space> {
        let mut current = parent.unwrap_or(page).to_string();
        let mut path = Vec::new();
        let mut visited = HashSet::new();
        let space = loop {
            if let Some(s) = self.cache.get(&current) {
                break s.clone();
            }
            if !visited.insert(current.clone()) {
                break (None, None);
            }
            path.push(current.clone());
            let row: Option<(Option<String>, Option<String>)> = self
                .db
                .conn
                .prepare_cached("SELECT parent,color FROM pages WHERE id = ?1")?
                .query_row([&current], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            match row {
                Some((Some(parent), _)) => current = parent,
                Some((None, color)) => break (Some(current), color),
                None => break (None, None),
            }
        };
        for id in path {
            self.cache.insert(id, space.clone());
        }
        Ok(space)
    }
}

impl Vault {
    /// All indexed agenda sources for tools, including undated to-dos and
    /// task notes. UI category visibility does not restrict this query.
    /// Results have a stable date/page/block/source order; undated entries
    /// follow dated ones. Trashed pages and templates remain excluded.
    pub fn query_agenda(&self) -> Result<Vec<AgendaEntry>> {
        // Read-only discovery must not initialize the lazy Calendars document
        // or normalize its CRDT lists as the settings editor does.
        let settings = match self.find_by_role("calendars".into())? {
            Some(page) => {
                serde_json::from_str::<CalendarSettings>(&self.type_data(page, "tasks".into())?)
                    .map_err(|_| invalid("invalid calendar settings"))?
            }
            None => CalendarSettings::default(),
        };
        lock(&self.inner.db).agenda_entries(
            ALL_AGENDA_SOURCES,
            &[],
            AGENDA_ORDER,
            None,
            &settings,
            false,
            false,
        )
    }

    fn agenda_task_note(&self, page: &str) -> Result<Option<TaskNote>> {
        let json = self.type_data(page.into(), "tasks".into())?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
            return Ok(None);
        };
        if empty_task_data(&value)
            && self
                .inner
                .read(page, |doc| doc.meta().page_type == "task")?
        {
            return Ok(Some(default_task_note()));
        }
        Ok(task_note(&value))
    }

    pub fn set_page_color(&self, page: PageId, color: Option<String>) -> Result<()> {
        if color
            .as_ref()
            .is_some_and(|c| c != "none" && !SPACE_COLORS.contains(&c.as_str()))
        {
            return Err(invalid("unknown space colour"));
        }
        self.set_organizer_meta(page, "color", color, true)
    }
    pub fn set_page_role(&self, page: PageId, role: Option<String>) -> Result<()> {
        if role
            .as_ref()
            .is_some_and(|r| !matches!(r.as_str(), "journal" | "calendars" | "feedback"))
        {
            return Err(invalid("unknown page role"));
        }
        self.set_organizer_meta(page, "role", role, true)
    }
    fn set_organizer_meta(
        &self,
        page: PageId,
        key: &str,
        value: Option<String>,
        tree: bool,
    ) -> Result<()> {
        let db = lock(&self.inner.db);
        if !db.exists(&page)? {
            return Err(not_found("page", &page));
        }
        self.inner
            .edit(&page, |d| d.set_meta_str(key, value.as_deref()))?;
        self.inner.sync_meta(&db, &page)?;
        drop(db);
        if tree {
            self.inner.emit([Change::TreeChanged]);
        }
        Ok(())
    }
    pub fn set_agenda_settings(&self, collection: PageId, settings: AgendaSettings) -> Result<()> {
        let db = lock(&self.inner.db);
        if db.schema_of(&collection)?.is_none() {
            return Err(invalid("page is not a collection"));
        }
        self.inner.edit(&collection, |d| {
            d.set_meta_str("agenda_role", Some(settings.role.stored()))?;
            d.set_meta_str("agenda_date", settings.date_prop.as_deref())
        })?;
        self.inner.sync_meta(&db, &collection)
    }
    pub fn agenda_settings(&self, collection: PageId) -> Result<AgendaSettings> {
        lock(&self.inner.db)
            .agenda_config(&collection)?
            .map(|(_, settings)| settings)
            .ok_or_else(|| invalid("page is not a collection"))
    }
    pub fn find_by_role(&self, role: String) -> Result<Option<PageId>> {
        Ok(lock(&self.inner.db).conn.prepare_cached("SELECT id FROM pages WHERE role = ?1 AND in_trash = 0 ORDER BY created_ms,id LIMIT 1")?.query_row([role],|r|r.get(0)).optional()?)
    }
    pub fn ensure_space_colors(&self) -> Result<u32> {
        let db = lock(&self.inner.db);
        let mut st = db.conn.prepare_cached("SELECT p.id,p.color FROM pages p WHERE p.parent IS NULL AND p.in_trash = 0 AND p.role IS NOT 'calendars' AND p.role IS NOT 'nodes' AND p.role IS NOT 'feedback' ORDER BY p.sort_key,p.created_ms,p.id")?;
        let pages = st
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut count = 0;
        for (i, (id, color)) in pages.iter().enumerate() {
            if color.as_deref().is_none_or(str::is_empty) {
                self.inner
                    .edit(id, |d| d.set_meta_str("color", Some(SPACE_COLORS[i % 6])))?;
                self.inner.sync_meta(&db, id)?;
                count += 1;
            }
        }
        drop(st);
        drop(db);
        if count > 0 {
            self.inner.emit([Change::TreeChanged]);
        }
        Ok(count)
    }
    pub fn agenda(
        &self,
        timed_from_ms: i64,
        timed_to_ms: i64,
        day_from_ms: i64,
        day_to_ms: i64,
    ) -> Result<Vec<AgendaItem>> {
        // One broad upper bound selects the date index; the two branches
        // still apply their own instant/calendar bounds to every candidate.
        let settings = self.list_calendars()?;
        let items = lock(&self.inner.db).agenda_items("a.kind != 2 AND a.d0 < max(?2,?4) AND ((a.has_time = 1 AND a.d0 < ?2 AND coalesce(a.d1,a.d0) >= ?1) OR (a.has_time = 0 AND a.d0 < ?4 AND coalesce(a.d1,a.d0) >= ?3))", &[timed_from_ms,timed_to_ms,day_from_ms,day_to_ms], "a.has_time,a.d0,a.title,p.id,a.block,a.source", None, &settings, false)?;
        self.attach_date_mark_refs(items)
    }
    /// All dated sources, including non-task mentions, for the vault calendar.
    pub fn calendar_items(
        &self,
        timed_from_ms: i64,
        timed_to_ms: i64,
        day_from_ms: i64,
        day_to_ms: i64,
    ) -> Result<Vec<AgendaItem>> {
        let settings = self.list_calendars()?;
        let items = lock(&self.inner.db).agenda_items("a.d0 < max(?2,?4) AND ((a.has_time = 1 AND a.d0 < ?2 AND coalesce(a.d1,a.d0) >= ?1) OR (a.has_time = 0 AND a.d0 < ?4 AND coalesce(a.d1,a.d0) >= ?3))", &[timed_from_ms,timed_to_ms,day_from_ms,day_to_ms], "a.has_time,a.d0,a.title,p.id,a.block,a.source", None, &settings, true)?;
        self.attach_date_mark_refs(items)
    }

    fn attach_date_mark_refs(&self, mut items: Vec<AgendaItem>) -> Result<Vec<AgendaItem>> {
        // Release the index lock before reading documents. Resolve each page
        // once, and withhold a reference while its index date is out of date.
        let mut pages = HashMap::<PageId, Vec<usize>>::new();
        for (i, item) in items.iter().enumerate() {
            if item.kind == AgendaKind::Task && item.block.is_some() {
                pages.entry(item.page.clone()).or_default().push(i);
            }
        }
        for (page, indices) in pages {
            self.inner.read(&page, |doc| {
                let blocks = doc.blocks();
                for i in indices {
                    let item = &mut items[i];
                    if let Some((reference, date, timed)) = item
                        .block
                        .as_deref()
                        .and_then(|id| find_dated_block(&blocks, id))
                        .and_then(date_mark_ref)
                    {
                        if date == item.start_ms && timed == item.has_time {
                            item.date_mark = Some(reference);
                        }
                    }
                }
            })?;
        }
        Ok(items)
    }

    /// Move only the ISO date prefix of the selected inline to-do date.
    /// A stale reference fails before any document mutation.
    pub fn move_date_mark(
        &self,
        page: PageId,
        block: BlockId,
        mark_ref: DateMarkRef,
        new_day_ms: i64,
    ) -> Result<()> {
        let day = Utc
            .timestamp_millis_opt(new_day_ms)
            .single()
            .filter(|_| new_day_ms.rem_euclid(86_400_000) == 0)
            .map(|day| day.format("%Y-%m-%d").to_string())
            .filter(|day| iso_day(day) == Some(new_day_ms))
            .ok_or_else(|| invalid("new date must be a valid UTC calendar midnight"))?;
        self.inner.edit(&page, |doc| {
            if doc.meta().trashed.is_some() {
                return Err(invalid("stale date mark reference: page is in the trash"));
            }
            let blocks = doc.blocks();
            let b = find_dated_block(&blocks, &block)
                .ok_or_else(|| invalid("stale date mark reference: block is missing"))?;
            let (current, _, _) = date_mark_ref(b).ok_or_else(|| {
                invalid("stale date mark reference: block has no selected to-do date")
            })?;
            if current != mark_ref {
                return Err(invalid(
                    "stale date mark reference: selected date or range changed",
                ));
            }
            let iso = format!("{day}{}", &current.iso[10..]);
            if iso == current.iso {
                return Ok(());
            }
            let mut marks = b.marks.clone();
            let mark = marks
                .iter_mut()
                .find(|mark| {
                    mark.start == current.start
                        && mark.end == current.end
                        && mark.kind == MarkKind::Date(current.iso.clone())
                })
                .ok_or_else(|| invalid("stale date mark reference: date is missing"))?;
            mark.kind = MarkKind::Date(iso);
            doc.apply(&[BlockOp::SetText {
                id: block,
                text: b.text.clone(),
                marks,
            }])
        })
    }
    /// Undone tasks before their matching bound, oldest first, capped in SQL.
    pub fn overdue(
        &self,
        timed_before_ms: i64,
        day_before_ms: i64,
        limit: u32,
    ) -> Result<Vec<AgendaItem>> {
        // Valid ranges end on or after their start, so the broad start bound
        // also narrows the (done, d0) index without changing the end test.
        let settings = self.list_calendars()?;
        lock(&self.inner.db).agenda_items(
            OVERDUE_PREDICATE,
            &[timed_before_ms, day_before_ms],
            "a.d0,a.title,p.id,a.block",
            Some(limit),
            &settings,
            false,
        )
    }
    /// Done tasks dated before their matching bound, on pages edited since
    /// `edited_since_ms`. The page edit stamp is a proxy for completion time.
    pub fn done_since(
        &self,
        timed_before_ms: i64,
        day_before_ms: i64,
        edited_since_ms: i64,
        limit: u32,
    ) -> Result<Vec<AgendaItem>> {
        let settings = self.list_calendars()?;
        lock(&self.inner.db).agenda_items(
            DONE_SINCE_PREDICATE,
            &[timed_before_ms, day_before_ms, edited_since_ms],
            "p.updated_ms DESC,a.d0,a.title,p.id,a.block",
            Some(limit),
            &settings,
            false,
        )
    }
    /// Total matching overdue tasks, without materializing items or spaces.
    pub fn overdue_count(&self, timed_before_ms: i64, day_before_ms: i64) -> Result<u32> {
        let settings = self.list_calendars()?;
        let db = lock(&self.inner.db);
        let visibility = calendar_visibility(3, false);
        let sql = format!(
            "SELECT count(*) FROM agenda a CROSS JOIN pages p ON p.rowid = a.page
             LEFT JOIN pages c ON c.id = p.parent
             WHERE p.in_trash = 0 AND {NOT_TEMPLATE} AND p.id IS NOT c.template
               AND ({OVERDUE_PREDICATE}) AND {visibility}"
        );
        let count = db.conn.prepare_cached(&sql)?.query_row(
            params![
                timed_before_ms,
                day_before_ms,
                serde_json::to_string(&settings).unwrap()
            ],
            |r| r.get(0),
        )?;
        Ok(count)
    }
    pub fn set_agenda_done(&self, page: PageId, block: Option<BlockId>, done: bool) -> Result<()> {
        self.set_agenda_done_inner(page, block, done, None)
    }

    /// Mutates an explicit source when a page is both a task note and a
    /// database row. The legacy API keeps its task-note-first precedence.
    pub fn set_agenda_source_done(
        &self,
        page: PageId,
        block: Option<BlockId>,
        source: String,
        done: bool,
    ) -> Result<()> {
        self.validate_agenda_source(&page, &block, &source)?;
        self.set_agenda_done_inner(page, block, done, Some(&source))
    }

    fn validate_agenda_source(
        &self,
        page: &str,
        block: &Option<BlockId>,
        source: &str,
    ) -> Result<()> {
        match source {
            "todo" if block.is_some() => Ok(()),
            "task_note" if block.is_none() && self.agenda_task_note(page)?.is_some() => Ok(()),
            "row" if block.is_none() => Ok(()),
            _ => Err(invalid("invalid or read-only agenda source")),
        }
    }

    fn set_agenda_done_inner(
        &self,
        page: PageId,
        block: Option<BlockId>,
        done: bool,
        source: Option<&str>,
    ) -> Result<()> {
        if let Some(block) = block {
            let valid = self.inner.read(&page, |d| {
                block_agenda(&d.blocks()).rows.iter().any(|b| {
                    b.block == block
                        && b.kind == AgendaKind::Task
                        && (source.is_some() || b.date.is_some())
                })
            })?;
            if !valid {
                return Err(invalid("block is not an agenda task"));
            }
            return self.apply(
                page,
                vec![BlockOp::SetChecked {
                    id: block,
                    checked: done,
                }],
            );
        }
        // App callers target dated agenda items. Explicit tool sources also
        // support undated notes without changing legacy database precedence.
        if source != Some("row")
            && self
                .agenda_task_note(&page)?
                .is_some_and(|task| source.is_some() || task.date.is_some())
        {
            return self.type_data_set(
                page,
                "tasks".into(),
                vec!["status".into()],
                serde_json::to_string(if done { "done" } else { "todo" }).unwrap(),
            );
        }
        let (key, value) = {
            let db = lock(&self.inner.db);
            let parent = db
                .parent_of(&page)?
                .flatten()
                .ok_or_else(|| invalid("page is not an agenda task"))?;
            let (schema, settings) = db
                .agenda_config(&parent)?
                .ok_or_else(|| invalid("page is not an agenda task"))?;
            let role = if source == Some("row") && settings.role == AgendaRole::Off {
                AgendaRole::Auto
            } else {
                settings.role
            };
            if effective_role(&schema, role) != AgendaRole::Tasks {
                return Err(invalid("page is not an agenda task"));
            }
            let rowid = db.rowid(&page)?.ok_or_else(|| not_found("page", &page))?;
            let date = date_def(&schema, settings.date_prop.as_deref())
                .ok_or_else(|| invalid("page is not an agenda task"))?;
            if !matches!(db.agenda_prop(rowid, &date.key)?, PropValue::Date { .. }) {
                return Err(invalid("page is not an agenda task"));
            }
            let def =
                done_def(&schema).ok_or_else(|| invalid("task has no completion property"))?;
            let value = match def.kind {
                PropKind::Checkbox => PropValue::Checkbox(done),
                PropKind::Status => {
                    let group = if done {
                        StatusGroup::Complete
                    } else {
                        StatusGroup::Todo
                    };
                    let option = first_option_in(def, group)
                        .ok_or_else(|| invalid("status has no option in the requested group"))?;
                    PropValue::Status(option.id.clone())
                }
                _ => unreachable!(),
            };
            (def.key.clone(), value)
        };
        self.set_property(page, key, value)
    }
    /// Reschedules a to-do's first date mention (appending if absent), a task
    /// note's type-data date, or a database row's configured date property.
    /// Times are instants; date-only values are UTC calendar midnights.
    pub fn reschedule_agenda(
        &self,
        page: PageId,
        block: Option<BlockId>,
        start_ms: i64,
        has_time: bool,
    ) -> Result<()> {
        self.reschedule_agenda_inner(page, block, start_ms, has_time, None)
    }

    pub fn reschedule_agenda_source(
        &self,
        page: PageId,
        block: Option<BlockId>,
        source: String,
        start_ms: i64,
        has_time: bool,
    ) -> Result<()> {
        self.validate_agenda_source(&page, &block, &source)?;
        self.reschedule_agenda_inner(page, block, start_ms, has_time, Some(&source))
    }

    fn reschedule_agenda_inner(
        &self,
        page: PageId,
        block: Option<BlockId>,
        start_ms: i64,
        has_time: bool,
        source: Option<&str>,
    ) -> Result<()> {
        let iso = crate::markdown::date_text(start_ms, None, has_time);
        if mention_date(&iso).is_none() || (!has_time && start_ms.rem_euclid(86_400_000) != 0) {
            return Err(invalid("invalid agenda date"));
        }
        if self.load_page(page.clone())?.in_trash {
            return Err(invalid("page is in the trash"));
        }
        if let Some(block) = block {
            // Resolve and edit under the same document lock so a concurrent
            // text edit cannot be overwritten between loading and applying.
            return self.inner.edit(&page, |doc| {
                fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
                    for b in blocks {
                        if b.id == id {
                            return Some(b);
                        }
                        if let Some(b) = find(&b.children, id) {
                            return Some(b);
                        }
                    }
                    None
                }
                let blocks = doc.blocks();
                let b = find(&blocks, &block).ok_or_else(|| not_found("block", &block))?;
                if b.kind != BlockKind::Todo {
                    return Err(invalid("block is not a to-do"));
                }
                let chars: Vec<char> = b.text.chars().collect();
                let first = b
                    .marks
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| {
                        matches!(m.kind, MarkKind::Date(_))
                            && m.start < m.end
                            && m.end as usize <= chars.len()
                    })
                    .min_by_key(|(_, m)| m.start);
                let (start, end) = first
                    .map(|(_, m)| (m.start, m.end))
                    .unwrap_or((chars.len() as u32, chars.len() as u32));
                let space = first.is_none() && !b.text.is_empty();
                let replacement = format!("{}@{iso}", if space { " " } else { "" });
                let width = replacement.chars().count() as u32;
                let delta = width as i64 - (end - start) as i64;
                let text = chars[..start as usize]
                    .iter()
                    .chain(replacement.chars().collect::<Vec<_>>().iter())
                    .chain(chars[end as usize..].iter())
                    .collect();
                let mut marks = Vec::new();
                for (i, m) in b.marks.iter().enumerate() {
                    if first.is_some_and(|(at, _)| at == i) {
                        continue;
                    }
                    let mut m = m.clone();
                    if m.start >= end {
                        m.start = (m.start as i64 + delta) as u32;
                        m.end = (m.end as i64 + delta) as u32;
                    } else if m.end > start {
                        m.start = m.start.min(start);
                        m.end = if m.end >= end {
                            (m.end as i64 + delta) as u32
                        } else {
                            start + width
                        };
                    }
                    if m.start < m.end {
                        marks.push(m);
                    }
                }
                marks.push(Mark {
                    start: start + u32::from(space),
                    end: start + width,
                    kind: MarkKind::Date(iso),
                });
                marks.sort_by_key(|m| m.start);
                doc.apply(&[BlockOp::SetText {
                    id: block,
                    text,
                    marks,
                }])
            });
        }
        if source != Some("row")
            && self
                .agenda_task_note(&page)?
                .is_some_and(|task| source.is_some() || task.date.is_some())
        {
            return self.type_data_set(
                page,
                "tasks".into(),
                vec!["date".into()],
                serde_json::to_string(&iso).unwrap(),
            );
        }
        let key = {
            let db = lock(&self.inner.db);
            let parent = db
                .parent_of(&page)?
                .flatten()
                .ok_or_else(|| invalid("page is not a database row"))?;
            let (schema, settings) = db
                .agenda_config(&parent)?
                .ok_or_else(|| invalid("page is not a database row"))?;
            date_def(&schema, settings.date_prop.as_deref())
                .filter(|d| d.kind == PropKind::Date)
                .ok_or_else(|| invalid("database has no agenda date property"))?
                .key
                .clone()
        };
        self.set_property(
            page,
            key,
            PropValue::Date {
                start_ms,
                end_ms: None,
                has_time,
            },
        )
    }

    pub fn in_motion(&self, limit: u32) -> Result<Vec<Progress>> {
        let db = lock(&self.inner.db);
        let sql = format!("SELECT p.id,p.title,p.icon,0,p.todos_done,p.todos_total,p.parent,p.updated_ms FROM pages p WHERE p.in_trash = 0 AND {NOT_TEMPLATE} AND p.todos_total > 0 AND p.todos_done < p.todos_total
            UNION ALL SELECT p.id,p.title,p.icon,1,sum(r.task_done),count(*),p.parent,max(r.updated_ms) FROM pages p JOIN pages r ON r.parent = p.id WHERE p.in_trash = 0 AND {NOT_TEMPLATE} AND r.in_trash = 0 AND r.is_template = 0 AND r.id IS NOT p.template AND r.task_done IS NOT NULL GROUP BY p.id HAVING sum(r.task_done) < count(*) ORDER BY 8 DESC,1 LIMIT ?1");
        let mut st = db.conn.prepare_cached(&sql)?;
        let rows = st
            .query_map([limit], |r| {
                Ok((
                    Progress {
                        page: r.get(0)?,
                        title: r.get(1)?,
                        icon: r.get(2)?,
                        kind: if r.get::<_, i64>(3)? == 0 {
                            ProgressKind::Checklist
                        } else {
                            ProgressKind::Tasks
                        },
                        done: r.get(4)?,
                        total: r.get(5)?,
                        space: None,
                        space_color: None,
                        updated_ms: r.get(7)?,
                    },
                    r.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut spaces = Spaces::new(&db);
        rows.into_iter()
            .map(|(mut p, parent)| {
                let (space, color) = spaces.of(&p.page, parent.as_deref())?;
                p.space = space;
                p.space_color = color;
                Ok(p)
            })
            .collect()
    }
    pub fn daily_note(&self, day_ms: i64, title: String, create: bool) -> Result<Option<PageId>> {
        let iso = Utc
            .timestamp_millis_opt(day_ms)
            .single()
            .ok_or_else(|| invalid("invalid daily-note date"))?
            .format("%Y-%m-%d")
            .to_string();
        if iso_day(&iso) != Some(day_ms) {
            return Err(invalid("daily-note date must be a UTC calendar-day stamp"));
        }
        // Lock before looking up the Journal or date row so independent vault
        // handles recheck the shared index before creating either page.
        // The order is daily -> document -> flush, never flush -> daily.
        let _file_guard = if create {
            Some(self.inner.daily_notes_lock()?)
        } else {
            None
        };
        // Creation spans several ordinary page edits; serialize it per vault.
        let _guard = lock(&self.inner.daily_notes);
        let journal = match self.find_by_role("journal".into())? {
            Some(id) => id,
            None if !create => return Ok(None),
            None => {
                let schema = vec![PropDef {
                    key: "date".into(),
                    name: "Date".into(),
                    kind: PropKind::Date,
                    options: vec![],
                    config: String::new(),
                }];
                let (id, created) = self.create_identified_page(
                    JOURNAL_PAGE,
                    None,
                    "Journal",
                    |doc| doc.meta().role.as_deref() == Some("journal"),
                    |doc| {
                        doc.set_page_type(PAGE_TYPE_DATABASE)?;
                        doc.set_schema(&schema)?;
                        doc.set_meta_str("agenda_role", Some("off"))?;
                        doc.set_meta_str("role", Some("journal"))
                    },
                )?;
                if created {
                    // Retain the normal database's default view creation.
                    self.set_schema(id.clone(), schema)?;
                }
                id
            }
        };
        let schema = self.get_schema(journal.clone())?;
        let date = schema
            .iter()
            .find(|d| d.kind == PropKind::Date)
            .ok_or_else(|| invalid("Journal has no Date property"))?;
        {
            let db = lock(&self.inner.db);
            let mut st=db.conn.prepare_cached(&format!("SELECT p.id,v.json FROM pages p JOIN props v ON v.page=p.rowid AND v.key=?2 WHERE p.parent=?1 AND p.in_trash=0 AND {NOT_TEMPLATE} AND v.d0=?3 ORDER BY p.sort_key,p.id"))?;
            let rows = st.query_map(params![journal, date.key, day_ms], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, json) = row?;
                if journal_day(&json) == Some(day_ms) {
                    return Ok(Some(id));
                }
            }
        }
        if !create {
            return Ok(None);
        }
        // New daily notes ignore a user template: they start with only the
        // fixed Tasks section and Log toggle, in that order.
        let preferred = journal_day_id(&journal, &iso);
        let value = PropValue::Date {
            start_ms: day_ms,
            end_ms: None,
            has_time: false,
        };
        let tasks = new_id();
        let (id, _) = self.create_identified_page(
            &preferred,
            Some(&journal),
            &title,
            |doc| {
                let meta = doc.meta();
                meta.parent.as_deref() == Some(journal.as_str())
                    && !meta.is_template
                    && doc.props().iter().any(|p| {
                        p.key == date.key
                            && matches!(
                                p.value,
                                PropValue::Date { start_ms, has_time: false, .. }
                                    if start_ms == day_ms
                            )
                    })
            },
            |doc| {
                doc.set_prop(&date.key, &value)?;
                doc.apply(&[
                    BlockOp::Insert {
                        parent: None,
                        after: None,
                        block: toggle_block(tasks.clone(), "Tasks"),
                    },
                    BlockOp::Insert {
                        parent: None,
                        after: Some(tasks),
                        block: toggle_block(new_id(), "Log"),
                    },
                ])
            },
        )?;
        Ok(Some(id))
    }

    /// Existing Journal days in the half-open calendar-day range. Reads only
    /// indexed dates; it neither creates a Journal nor loads each note.
    pub fn day_notes_in_range(&self, from_ms: i64, to_ms: i64) -> Result<Vec<i64>> {
        if from_ms >= to_ms {
            return Ok(vec![]);
        }
        let Some(journal) = self.find_by_role("journal".into())? else {
            return Ok(vec![]);
        };
        let schema = self.get_schema(journal.clone())?;
        let date = schema
            .iter()
            .find(|d| d.kind == PropKind::Date)
            .ok_or_else(|| invalid("Journal has no Date property"))?;
        let db = lock(&self.inner.db);
        // Drive the props(key, d0) index, then check only matching pages.
        let mut st = db.conn.prepare_cached(&format!(
            "SELECT v.json FROM props v CROSS JOIN pages p ON p.rowid = v.page
             WHERE v.key = ?2 AND v.d0 >= ?3 AND v.d0 < ?4
               AND p.parent = ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE}
             ORDER BY v.d0"
        ))?;
        let rows = st.query_map(params![journal, date.key, from_ms, to_ms], |r| {
            r.get::<_, String>(0)
        })?;
        let mut days = Vec::new();
        for row in rows {
            if let Some(day) = journal_day(&row?) {
                if day >= from_ms && day < to_ms && days.last() != Some(&day) {
                    days.push(day);
                }
            }
        }
        Ok(days)
    }

    /// Appends a local-time-stamped paragraph to the day's top-level Log
    /// toggle, creating the daily note and Log toggle when needed.
    pub fn append_log(&self, day_ms: i64, text: String) -> Result<PageId> {
        // The log time is the device's wall clock, obtained through the
        // shared clock hook.
        let title = day_title(day_ms)?;
        let now = Local
            .timestamp_millis_opt(now_ms())
            .single()
            .ok_or_else(|| invalid("invalid log timestamp"))?;
        let page = self
            .daily_note(day_ms, title, true)?
            .ok_or_else(|| invalid("daily note was not created"))?;
        let line = format!("{}  {text}", now.format("%H:%M"));
        self.inner.edit(&page, |doc| {
            // Find/create and append under one document lock, so concurrent
            // captures cannot duplicate Log or insert against a stale tail.
            let roots = doc.blocks();
            let log = roots
                .iter()
                .find(|b| b.kind == BlockKind::Toggle && b.text == "Log");
            let mut ops = Vec::new();
            let (parent, after) = match log {
                Some(log) => (log.id.clone(), log.children.last().map(|b| b.id.clone())),
                None => {
                    let id = new_id();
                    ops.push(BlockOp::Insert {
                        parent: None,
                        after: roots.last().map(|b| b.id.clone()),
                        block: NewBlock {
                            id: id.clone(),
                            kind: BlockKind::Toggle,
                            text: "Log".into(),
                            marks: vec![],
                            checked: false,
                        },
                    });
                    (id, None)
                }
            };
            ops.push(BlockOp::Insert {
                parent: Some(parent),
                after,
                block: NewBlock {
                    id: new_id(),
                    kind: BlockKind::Paragraph,
                    text: line,
                    marks: vec![],
                    checked: false,
                },
            });
            doc.apply(&ops)
        })?;
        Ok(page)
    }

    /// Links a task page into the day's Tasks section, creating the daily
    /// note when needed. Returns false when that section already links the
    /// task; links elsewhere in the note do not count.
    pub fn link_task_to_day(&self, day_ms: i64, task_page_id: PageId) -> Result<bool> {
        // Validate before creating a note, so a rejected link leaves no trace.
        // Trashed and missing pages look the same here.
        if lock(&self.inner.db).in_trash(&task_page_id)? != Some(false) {
            return Err(not_found("page", &task_page_id));
        }
        // Any tasks data makes a task; a date is optional (tasks.status alone
        // is enough). Absent data reads as an empty object.
        if self.type_data(task_page_id.clone(), "tasks".into())? == "{}" {
            return Err(invalid("page has no tasks data"));
        }
        let title = day_title(day_ms)?;
        let page = self
            .daily_note(day_ms, title, true)?
            .ok_or_else(|| invalid("daily note was not created"))?;
        self.inner.edit(&page, |doc| {
            // Find or create the section and check for a duplicate under one
            // document lock, so concurrent links cannot add the task twice.
            let roots = doc.blocks();
            let mut ops = Vec::new();
            let (section, after) = match roots
                .iter()
                .find(|b| b.kind == BlockKind::Toggle && b.text == "Tasks")
            {
                Some(tasks) => {
                    if links_page(&tasks.children, &task_page_id) {
                        return Ok(false);
                    }
                    (
                        tasks.id.clone(),
                        tasks.children.last().map(|b| b.id.clone()),
                    )
                }
                None => {
                    // Older notes may lack the section. Put it just above
                    // Log; without a Log, put it at the end.
                    let before = match roots
                        .iter()
                        .position(|b| b.kind == BlockKind::Toggle && b.text == "Log")
                    {
                        Some(0) => None,
                        Some(i) => Some(roots[i - 1].id.clone()),
                        None => roots.last().map(|b| b.id.clone()),
                    };
                    let id = new_id();
                    ops.push(BlockOp::Insert {
                        parent: None,
                        after: before,
                        block: toggle_block(id.clone(), "Tasks"),
                    });
                    (id, None)
                }
            };
            let link = new_id();
            ops.push(BlockOp::Insert {
                parent: Some(section),
                after,
                block: NewBlock {
                    id: link.clone(),
                    kind: BlockKind::LinkToPage,
                    text: String::new(),
                    marks: vec![],
                    checked: false,
                },
            });
            ops.push(BlockOp::SetTarget {
                id: link,
                target: Some(task_page_id.clone()),
            });
            doc.apply(&ops)?;
            Ok(true)
        })
    }
}
