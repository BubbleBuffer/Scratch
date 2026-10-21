//! Calendar tools use the same calendar-day / instant split as the app.
use chrono::{Datelike, Local, LocalResult, NaiveDate, NaiveTime, TimeZone};
use serde_json::{json, Value};
use vault_core::{Block, BlockKind, Vault};

use crate::{err, Result, ToolError};

/// The concrete page and optional block addressed by an agenda reference.
/// Row references also retain their database, after checking membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaTarget {
    pub page: String,
    pub block: Option<String>,
    pub database: Option<String>,
    /// Date mentions can be opened, but update_task must never mutate them.
    pub writable: bool,
}

/// Resolves the stable references returned by agenda / overdue. Undated
/// to-dos are accepted, allowing `reschedule` to add their first mention.
pub fn resolve_agenda_item(v: &Vault, item: &str) -> Result<AgendaTarget> {
    let (kind, ids) = item.split_once(':').ok_or_else(|| {
        ToolError("item must be block:<page>/<block>, note:<page>, row:<database>/<row> or mention:<page>/<block>".into())
    })?;
    if kind == "note" {
        if ids.is_empty() || ids.contains('/') {
            return err("item must be note:<page>");
        }
        let page = v.load_page(ids.into())?;
        if page.in_trash {
            return err("item's page is in the trash");
        }
        let data: Value = serde_json::from_str(&v.type_data(ids.into(), "tasks".into())?)
            .map_err(|e| ToolError(e.to_string()))?;
        if page.page_type != "task"
            && !data.as_object().is_some_and(|o| {
                [
                    "date",
                    "status",
                    "calendar",
                    "assignee",
                    "checkbox",
                    "duration_min",
                ]
                .iter()
                .any(|k| o.contains_key(*k))
            })
        {
            return err("item's page is not a task note");
        }
        return Ok(AgendaTarget {
            page: ids.into(),
            block: None,
            database: None,
            writable: true,
        });
    }
    let (left, right) = ids
        .split_once('/')
        .filter(|(l, r)| !l.is_empty() && !r.is_empty() && !r.contains('/'))
        .ok_or_else(|| ToolError("item must contain two nonempty ids separated by /".into()))?;
    let page_id = if matches!(kind, "block" | "mention") {
        left
    } else {
        right
    };
    if !matches!(kind, "block" | "row" | "mention") {
        return err("unknown item reference kind");
    }
    let page = v.load_page(page_id.into())?;
    if page.in_trash {
        return err("item's page is in the trash");
    }
    if matches!(kind, "block" | "mention") {
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
        let b = find(&page.blocks, right)
            .ok_or_else(|| ToolError(format!("no block {right} on page {left}")))?;
        if kind == "block" && b.kind != BlockKind::Todo {
            return err("item's block is not a to-do");
        }
        if kind == "mention"
            && (b.kind == BlockKind::Todo
                || !b
                    .marks
                    .iter()
                    .any(|m| matches!(m.kind, vault_core::MarkKind::Date(_))))
        {
            return err("item's block is not a date mention");
        }
        Ok(AgendaTarget {
            page: left.into(),
            block: Some(right.into()),
            database: None,
            writable: kind == "block",
        })
    } else {
        let database = v.load_page(left.into())?;
        if !database.is_database() || database.in_trash || page.parent.as_deref() != Some(left) {
            return err("item is not a live row of its referenced database");
        }
        Ok(AgendaTarget {
            page: right.into(),
            block: None,
            database: Some(left.into()),
            writable: true,
        })
    }
}

fn required<'a>(a: &'a Value, key: &str) -> Result<&'a str> {
    a.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError(format!("missing string argument: {key}")))
}

fn day(s: &str) -> Result<NaiveDate> {
    if s.len() != 10
        || s.as_bytes()[4] != b'-'
        || s.as_bytes()[7] != b'-'
        || !s
            .bytes()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return err("date must use YYYY-MM-DD");
    }
    let parsed = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| ToolError(format!("invalid ISO date: {s}; expected YYYY-MM-DD")))?;
    if parsed.format("%Y-%m-%d").to_string() != s {
        return err("date must use YYYY-MM-DD");
    }
    Ok(parsed)
}

fn day_ms(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

fn local_ms(date: NaiveDate, time: NaiveTime) -> Result<i64> {
    match Local.from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(date) => Ok(date.timestamp_millis()),
        LocalResult::Ambiguous(_, _) => {
            err("local time is ambiguous at a daylight-saving transition; choose another time")
        }
        LocalResult::None => {
            err("local time does not exist at a daylight-saving transition; choose another time")
        }
    }
}

pub(crate) fn set_done(v: &Vault, a: &Value) -> Result<String> {
    let reference = required(a, "item")?;
    let done = a
        .get("done")
        .and_then(Value::as_bool)
        .ok_or_else(|| ToolError("missing boolean argument: done".into()))?;
    let target = resolve_agenda_item(v, reference)?;
    if !target.writable {
        return err("date mentions are read-only; edit their page to change them");
    }
    let source = if target.block.is_some() {
        "todo"
    } else if target.database.is_some() {
        "row"
    } else {
        "task_note"
    };
    v.set_agenda_source_done(target.page, target.block, source.into(), done)?;
    Ok(json!({ "item": reference, "done": done }).to_string())
}

pub(crate) fn reschedule(v: &Vault, a: &Value) -> Result<String> {
    let reference = required(a, "item")?;
    let date = day(required(a, "date")?)?;
    let time = match a.get("time") {
        None => None,
        Some(Value::String(time)) => {
            let parsed = NaiveTime::parse_from_str(time, "%H:%M")
                .map_err(|_| ToolError("time must use HH:MM (00:00 through 23:59)".into()))?;
            if parsed.format("%H:%M").to_string() != *time {
                return err("time must use HH:MM");
            }
            Some(parsed)
        }
        Some(_) => return err("time must be an HH:MM string or omitted"),
    };
    let stamp = match time {
        Some(time) => local_ms(date, time)?,
        None => day_ms(date),
    };
    let target = resolve_agenda_item(v, reference)?;
    if !target.writable {
        return err("date mentions are read-only; edit their page to change them");
    }
    let source = if target.block.is_some() {
        "todo"
    } else if target.database.is_some() {
        "row"
    } else {
        "task_note"
    };
    v.reschedule_agenda_source(
        target.page,
        target.block,
        source.into(),
        stamp,
        time.is_some(),
    )?;
    Ok(
        json!({ "item": reference, "date": date.format("%Y-%m-%d").to_string(),
        "time": time.map(|t| t.format("%H:%M").to_string()) })
        .to_string(),
    )
}

fn daily_date(a: &Value) -> Result<NaiveDate> {
    match a.get("date") {
        None => Ok(Local::now().date_naive()),
        Some(Value::String(date)) => day(date),
        Some(_) => err("date must be a YYYY-MM-DD string or omitted"),
    }
}

/// Looks up a daily note without creating it, using the tool's local-date
/// default and the same core lookup as the app.
pub fn resolve_daily_note(v: &Vault, a: &Value) -> Result<Option<String>> {
    let date = daily_date(a)?;
    Ok(v.daily_note(day_ms(date), String::new(), false)?)
}

pub(crate) fn daily_note(v: &Vault, a: &Value) -> Result<String> {
    let date = daily_date(a)?;
    // Match daily_capture.dart's weekday/day/month/year title and core's
    // serialized create-or-reuse path with Tasks and Log sections.
    let title = format!(
        "{} {} {} {}",
        date.format("%A"),
        date.day(),
        date.format("%B"),
        date.year()
    );
    let existing = v.daily_note(day_ms(date), title.clone(), false)?;
    let created = existing.is_none();
    let id = match existing {
        Some(id) => id,
        None => v
            .daily_note(day_ms(date), title, true)?
            .ok_or_else(|| ToolError("daily note was not created".into()))?,
    };
    let page = v.load_page(id.clone())?;
    Ok(json!({ "id": id, "title": page.title, "created": created }).to_string())
}
