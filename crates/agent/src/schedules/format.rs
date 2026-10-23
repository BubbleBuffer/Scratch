//! Schedule format v1: parsing, acceptance hash, occurrences and titles.
//!
//! A schedule's definition is the object stored as `type_data.schedule`. Parsing
//! refuses what this runner cannot run; nothing is ignored silently. Unknown keys
//! stay in `raw`. The acceptance hash covers `runner`, `tools` and `steps`, with
//! object keys sorted and array order kept.
//!
//! `tools` limits what the steps may use: a built-in tool set (`read`, `write` or
//! `everything`, the same sets as agent tokens) or, as first written, a list of
//! tool names. Every step's tool must be in it; a run checks again before acting.
//!
//! Times: `trigger.at` is a wall time in `trigger.tz`. A wall time in a DST gap runs
//! at the transition, the first valid instant after the gap. A wall time in an
//! overlap runs at the earlier instant.
//!
//! Recurrence is a subset of RRULE: `FREQ` DAILY, WEEKLY or MONTHLY, with
//! `INTERVAL`, plus `COUNT` or `UNTIL` (mutually exclusive), `BYDAY` (weekly, plain
//! weekday codes) and `BYMONTHDAY` (monthly, 1..31 or -1). Omitted BYDAY and
//! BYMONTHDAY take DTSTART's weekday and day. A month without the day is skipped,
//! not clamped. Occurrences are the rule's matches on or after DTSTART's date, at
//! DTSTART's wall time; the interval counts periods from DTSTART's period. DTSTART
//! is an occurrence only when the rule matches it. Without a rule the schedule is a
//! one-off at DTSTART. COUNT and UNTIL limit the whole sequence, not the query window.

use anyhow::{bail, ensure, Context, Result};
use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::Timestamp;
use notes_agent_wire::{ScheduleOccurrence, SchedulePreview};
use serde_json::{json, Value};
use vault_tools::ToolSet;

const FORMAT_VERSION: u64 = 1;
const STEP_KIND_NOTE: &str = "note.from_template";
/// The vault tool the `note.from_template` step runs as.
pub const NOTE_STEP_TOOL: &str = "create_page";
/// Limit on explicitly requested previews. Catch-up scans include every due date.
const MAX_OCCURRENCES: usize = 10_000;

/// A validated schedule. `raw` is the definition as stored, unknown keys included.
#[derive(Clone, Debug)]
pub struct Schedule {
    pub raw: Value,
    pub runner: String,
    /// `latest` or `skip`.
    pub missed: String,
    pub tools: ScheduleTools,
    pub step: NoteStep,
    trigger: Trigger,
}

/// What a schedule's steps may use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleTools {
    /// A built-in tool set, as agent tokens use.
    Set(ToolSet),
    /// Tool names, as the format was first written. Old names count as the
    /// tool they run.
    List(Vec<String>),
}

impl ScheduleTools {
    /// Reads the stored `tools` value: a set name or a list of names.
    fn parse(value: Option<&Value>) -> Result<Self> {
        match value {
            Some(Value::String(name)) => name
                .parse::<ToolSet>()
                .map(ScheduleTools::Set)
                .map_err(|e| anyhow::anyhow!("tools: {e}")),
            Some(Value::Array(items)) => items
                .iter()
                .map(|t| {
                    t.as_str()
                        .map(str::to_owned)
                        .context("tools must be strings")
                })
                .collect::<Result<Vec<_>>>()
                .map(ScheduleTools::List),
            _ => bail!("tools must be a tool set (read, write or everything) or a list"),
        }
    }

    /// Whether a step may run tool `name`.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            ScheduleTools::Set(set) => set.allows(name),
            ScheduleTools::List(names) => {
                let wanted = vault_tools::canonical(name);
                names.iter().any(|t| vault_tools::canonical(t) == wanted)
            }
        }
    }
}

/// The `note.from_template` step. Template and parent are link text as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteStep {
    pub template: String,
    pub parent: String,
    /// Raw title with placeholders, rendered per occurrence by `render_title`.
    pub title: String,
}

#[derive(Clone, Debug)]
struct Trigger {
    tz: TimeZone,
    /// The DTSTART wall time, as written in `trigger.at`.
    start: DateTime,
    /// Day number (days since 1970-01-01) of DTSTART's local date.
    start_day: i64,
    /// `None` for a one-off.
    rule: Option<Rule>,
    count: Option<u64>,
    until: Option<Timestamp>,
}

#[derive(Clone, Debug)]
struct Rule {
    kind: Kind,
    interval: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Daily,
    /// Weekday indexes, Monday = 0, sorted and distinct.
    Weekly(Vec<i64>),
    /// Sorted and distinct.
    Monthly(Vec<MonthDay>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonthDay {
    Day(i64),
    Last,
}

/// The RRULE after parsing. `count` and `until` are the global limits.
struct Parsed {
    rule: Rule,
    count: Option<u64>,
    until: Option<Timestamp>,
}

impl Schedule {
    /// Validates a stored definition. The error text is shown on the schedule card.
    pub fn parse(raw: Value) -> Result<Self> {
        let version = raw
            .get("version")
            .context("schedule has no version")?
            .as_u64()
            .context("version must be a whole number")?;
        ensure!(
            version <= FORMAT_VERSION,
            "schedule format version {version} is newer than this runner supports ({FORMAT_VERSION})"
        );
        ensure!(
            version == FORMAT_VERSION,
            "unsupported schedule format version {version}"
        );
        let runner = text(&raw, "runner")?;
        let tools = ScheduleTools::parse(raw.get("tools"))?;
        match &tools {
            ScheduleTools::Set(set) => ensure!(
                tools.allows(NOTE_STEP_TOOL),
                "the {set} tool set cannot create pages; the note step needs write or everything"
            ),
            ScheduleTools::List(_) => ensure!(
                tools.allows(NOTE_STEP_TOOL),
                "tools must include create_page; the note step creates a page"
            ),
        }
        let (trigger, missed) = parse_trigger(raw.get("trigger").context("trigger is missing")?)?;
        let steps = raw
            .get("steps")
            .and_then(Value::as_array)
            .context("steps must be a list")?;
        ensure!(!steps.is_empty(), "steps must have one step");
        ensure!(steps.len() == 1, "needs a newer version");
        let step = parse_step(&steps[0])?;
        Ok(Self {
            runner,
            missed,
            tools,
            step,
            trigger,
            raw,
        })
    }

    /// Acceptance hash over `runner`, `tools` and `steps`.
    pub fn hash(&self) -> String {
        canonical_hash(&self.raw)
    }

    /// Occurrences after `after` and up to `through` (inclusive), in order.
    /// COUNT and UNTIL apply to the whole sequence from DTSTART.
    pub fn occurrences(&self, after: Timestamp, through: Timestamp) -> Result<Vec<Timestamp>> {
        if through <= after {
            return Ok(vec![]);
        }
        self.scan(after, Some(through), usize::MAX)
    }

    /// The next `count` occurrences after `after`, in order. COUNT and UNTIL apply to
    /// the whole sequence from DTSTART.
    pub fn next(&self, after: Timestamp, count: usize) -> Result<Vec<Timestamp>> {
        ensure!(
            count <= MAX_OCCURRENCES,
            "at most {MAX_OCCURRENCES} occurrences can be listed at once"
        );
        self.scan(after, None, count)
    }

    /// The title for the occurrence at `at`, in the schedule's time zone. Known
    /// placeholders are filled; anything else stays literal.
    pub fn render_title(&self, at: Timestamp) -> Result<String> {
        let zoned = at.to_zoned(self.trigger.tz.clone());
        let date = zoned.date();
        let time = zoned.time();
        let iso = date.iso_week_date();
        Ok(self
            .step
            .title
            .replace(
                "{{date}}",
                &format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()),
            )
            .replace(
                "{{time}}",
                &format!("{:02}:{:02}", time.hour(), time.minute()),
            )
            .replace("{{weekday}}", english_weekday(date.weekday()))
            .replace("{{week}}", &iso.week().to_string())
            .replace("{{year}}", &iso.year().to_string()))
    }

    /// Walks the occurrences in order, from the first day that can still be after `after`.
    /// `position` counts occurrences from DTSTART's day, so COUNT holds globally.
    fn scan(
        &self,
        after: Timestamp,
        through: Option<Timestamp>,
        limit: usize,
    ) -> Result<Vec<Timestamp>> {
        let mut found = vec![];
        if limit == 0 {
            return Ok(found);
        }
        let from = self.first_day(after);
        let position = if self.trigger.count.is_some() {
            self.count_before(from)
        } else {
            0
        };
        for (current, day) in (position..).zip(self.days_from(from)) {
            if self.trigger.count.is_some_and(|count| current >= count) {
                break;
            }
            let at = self.instant(day)?;
            if self.trigger.until.is_some_and(|until| at > until) {
                break;
            }
            if through.is_some_and(|through| at > through) {
                break;
            }
            if at > after {
                found.push(at);
                if found.len() == limit {
                    break;
                }
            }
        }
        Ok(found)
    }

    /// Two days before the local date of `after`. Every occurrence before that day is
    /// before `after`, even after a gap or an overlap moves its instant.
    fn first_day(&self, after: Timestamp) -> i64 {
        let local = after.to_zoned(self.trigger.tz.clone()).date();
        day_of(local) - 2
    }

    /// Occurrences dated from DTSTART's day up to `day` (exclusive), by arithmetic.
    fn count_before(&self, day: i64) -> u64 {
        let start = self.trigger.start_day;
        if day <= start {
            return 0;
        }
        // matches_before(start) counts the matches before DTSTART's day, which
        // matches_before(day) also includes, so they cancel out.
        (self.matches_before(day) - self.matches_before(start)) as u64
    }

    /// Rule matches in active periods dated before `day`, including any before DTSTART
    /// inside its own period. Whole periods are counted by arithmetic. A monthly rule
    /// counts each month, since month lengths differ.
    fn matches_before(&self, day: i64) -> i64 {
        let Some(rule) = &self.trigger.rule else {
            return 0;
        };
        let (n, start) = (rule.interval, self.trigger.start_day);
        match &rule.kind {
            Kind::Daily => {
                if day <= start {
                    0
                } else {
                    (day - start - 1) / n + 1
                }
            }
            Kind::Weekly(days) => {
                let (first, last) = (week_of(start), week_of(day));
                let whole = if last <= first {
                    0
                } else {
                    (last - first - 1) / n + 1
                };
                let mut total = whole * days.len() as i64;
                if last >= first && (last - first) % n == 0 {
                    let monday = monday_of(last);
                    total += days.iter().filter(|&&index| monday + index < day).count() as i64;
                }
                total
            }
            Kind::Monthly(specs) => {
                let (start_year, start_month, _) = civil(start);
                let (year, month, _) = civil(day);
                let first = start_year * 12 + start_month - 1;
                let last = year * 12 + month - 1;
                let mut total = 0;
                let mut index = first;
                while index < last {
                    let (y, m) = (index.div_euclid(12), index.rem_euclid(12) + 1);
                    total += month_days(specs, month_length(y, m)).len() as i64;
                    let Some(next) = index.checked_add(n) else {
                        return total;
                    };
                    index = next;
                }
                if index == last {
                    let days = month_days(specs, month_length(year, month));
                    total += days
                        .iter()
                        .filter(|&&d| day_number(year, month, d) < day)
                        .count() as i64;
                }
                total
            }
        }
    }

    /// Occurrence dates from `from` on, in order. Never before DTSTART's date.
    fn days_from(&self, from: i64) -> impl Iterator<Item = i64> + '_ {
        let start = self.trigger.start_day;
        let mut cursor = Some(from.max(start));
        std::iter::from_fn(move || {
            let current = cursor?;
            let found = self.next_day(current)?;
            if !in_range(found) {
                cursor = None;
                return None;
            }
            cursor = Some(found + 1);
            Some(found)
        })
    }

    /// The first occurrence date on or after `from`. A one-off has only DTSTART's day.
    fn next_day(&self, from: i64) -> Option<i64> {
        if self.trigger.rule.is_none() {
            return (from == self.trigger.start_day).then_some(self.trigger.start_day);
        }
        self.next_match(from)
    }

    /// The first rule match on or after `from` and on or after DTSTART's date. Jumps
    /// over inactive periods instead of stepping through them.
    fn next_match(&self, from: i64) -> Option<i64> {
        let rule = self.trigger.rule.as_ref()?;
        let (n, start) = (rule.interval, self.trigger.start_day);
        let from = from.max(start);
        match &rule.kind {
            Kind::Daily => from.checked_add((start - from).rem_euclid(n)),
            Kind::Weekly(days) => {
                let first = week_of(start);
                let mut week = week_of(from);
                loop {
                    if week > week_of(day_number(9999, 12, 31)) {
                        return None;
                    }
                    if (week - first) % n == 0 {
                        let found = days
                            .iter()
                            .map(|&index| monday_of(week) + index)
                            .filter(|&day| day >= from)
                            .min();
                        if found.is_some() {
                            return found;
                        }
                    }
                    week = first.checked_add(((week - first) / n + 1).checked_mul(n)?)?;
                }
            }
            Kind::Monthly(specs) => {
                let (start_year, start_month, _) = civil(start);
                let first = start_year * 12 + start_month - 1;
                let (year, month, _) = civil(from);
                let mut index = year * 12 + month - 1;
                loop {
                    if index.div_euclid(12) > 9999 {
                        return None;
                    }
                    if (index - first) % n == 0 {
                        let (y, m) = (index.div_euclid(12), index.rem_euclid(12) + 1);
                        let found = month_days(specs, month_length(y, m))
                            .into_iter()
                            .map(|d| day_number(y, m, d))
                            .find(|&day| day >= from);
                        if found.is_some() {
                            return found;
                        }
                    }
                    index = first.checked_add(((index - first) / n + 1).checked_mul(n)?)?;
                }
            }
        }
    }

    /// The instant of the occurrence on `day`, at the DTSTART wall time.
    fn instant(&self, day: i64) -> Result<Timestamp> {
        let (year, month, date) = civil(day);
        let time = self.trigger.start.time();
        let wall = DateTime::new(
            year as i16,
            month as i8,
            date as i8,
            time.hour(),
            time.minute(),
            time.second(),
            0,
        )?;
        local_to_instant(&self.trigger.tz, wall)
    }
}

/// Lowercase hex SHA-256 over the canonical JSON of `runner`, `tools` and `steps`.
/// Object keys are sorted; array order is kept. Works on refused definitions too.
/// Errors only when `raw` is not an object.
pub fn definition_hash(raw: &Value) -> Result<String> {
    ensure!(raw.is_object(), "schedule must be an object");
    Ok(canonical_hash(raw))
}

/// Checks a definition (JSON text) and lists its next three occurrences after
/// `now`. Pure: it saves nothing and reads no vault, so every host can call it.
/// Invalid input is reported in the result, never as an error.
pub fn preview(json_text: &str, now: Timestamp) -> SchedulePreview {
    let raw: Value = match serde_json::from_str(json_text) {
        Ok(raw) => raw,
        Err(e) => {
            return SchedulePreview {
                valid: false,
                current_hash: None,
                reason: Some(format!("invalid JSON: {e}")),
                occurrences: vec![],
            }
        }
    };
    let current_hash = definition_hash(&raw).ok();
    let result = (|| -> Result<Vec<ScheduleOccurrence>> {
        let schedule = Schedule::parse(raw)?;
        schedule
            .next(now, 3)?
            .into_iter()
            .map(|at| {
                Ok(ScheduleOccurrence {
                    occurrence: at.to_string(),
                    title: schedule.render_title(at)?,
                })
            })
            .collect()
    })();
    match result {
        Ok(occurrences) => SchedulePreview {
            valid: true,
            current_hash,
            reason: None,
            occurrences,
        },
        Err(error) => SchedulePreview {
            valid: false,
            current_hash,
            reason: Some(error.to_string()),
            occurrences: vec![],
        },
    }
}

fn canonical_hash(raw: &Value) -> String {
    let field = |key: &str| raw.get(key).cloned().unwrap_or(Value::Null);
    let definition =
        json!({"runner": field("runner"), "steps": field("steps"), "tools": field("tools")});
    let mut canonical = String::new();
    write_canonical(&definition, &mut canonical);
    vault_core::sha256_hex(canonical.as_bytes())
}

/// JSON with object keys sorted at every level, independent of key order in memory.
fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.to_string()).to_string());
                out.push(':');
                write_canonical(&map[key.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

fn text(value: &Value, key: &str) -> Result<String> {
    match value.get(key).and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ => bail!("{key} must be a non-empty string"),
    }
}

fn parse_step(value: &Value) -> Result<NoteStep> {
    let id = text(value, "id")?;
    ensure!(value.get("if").is_none(), "needs a newer version");
    let kind = text(value, "do")?;
    ensure!(
        kind == STEP_KIND_NOTE,
        "step {id:?} has unsupported kind {kind:?}"
    );
    Ok(NoteStep {
        template: text(value, "template")?,
        parent: text(value, "parent")?,
        title: text(value, "title")?,
    })
}

fn parse_trigger(value: &Value) -> Result<(Trigger, String)> {
    let zone = text(value, "tz")?;
    let tz = TimeZone::get(&zone).with_context(|| format!("unknown time zone {zone:?}"))?;
    let start = parse_at(&text(value, "at")?)?;
    local_to_instant(&tz, start)?;
    let start_day = day_of(start.date());
    let missed = match value.get("missed") {
        None => "latest".to_string(),
        Some(missed) => match missed.as_str() {
            Some(choice @ ("latest" | "skip")) => choice.to_string(),
            _ => bail!("missed must be latest or skip"),
        },
    };
    let trigger = match value.get("rrule") {
        None => Trigger {
            tz,
            start,
            start_day,
            rule: None,
            count: None,
            until: None,
        },
        Some(rule) => {
            let rule = rule.as_str().context("rrule must be a string")?;
            let weekday = (start_day + 3).rem_euclid(7);
            let parsed = parse_rrule(rule, weekday, i64::from(start.date().day()))?;
            Trigger {
                tz,
                start,
                start_day,
                rule: Some(parsed.rule),
                count: parsed.count,
                until: parsed.until,
            }
        }
    };
    Ok((trigger, missed))
}

/// A local wall time with no offset, `YYYY-MM-DDTHH:MM` or with `:SS`.
fn parse_at(text: &str) -> Result<DateTime> {
    ensure!(
        text.bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b':' | b'T'))
            && text.matches('T').count() == 1,
        "trigger.at {text:?} must be a local time like 2026-10-19T08:00 with no offset"
    );
    let time = text.split_once('T').map_or("", |(_, time)| time);
    let full = if time.matches(':').count() == 1 {
        format!("{text}:00")
    } else {
        text.to_string()
    };
    full.parse::<DateTime>()
        .with_context(|| format!("trigger.at {text:?} is not a valid date and time"))
}

/// Parses the supported RRULE subset. Any other part is refused, never ignored.
/// `start_weekday` (Monday = 0) and `start_monthday` are DTSTART's, the defaults for
/// an omitted BYDAY or BYMONTHDAY.
fn parse_rrule(text: &str, start_weekday: i64, start_monthday: i64) -> Result<Parsed> {
    let mut freq: Option<&str> = None;
    let mut interval: Option<&str> = None;
    let mut count: Option<&str> = None;
    let mut until: Option<&str> = None;
    let mut byday: Option<&str> = None;
    let mut bymonthday: Option<&str> = None;
    for part in text.split(';') {
        let (key, value) = part
            .split_once('=')
            .with_context(|| format!("RRULE part {part:?} is not KEY=VALUE"))?;
        let slot = match key {
            "FREQ" => &mut freq,
            "INTERVAL" => &mut interval,
            "COUNT" => &mut count,
            "UNTIL" => &mut until,
            "BYDAY" => &mut byday,
            "BYMONTHDAY" => &mut bymonthday,
            other => bail!("RRULE part {other} is not supported in schedule format v1"),
        };
        ensure!(slot.is_none(), "RRULE part {key} appears twice");
        *slot = Some(value);
    }
    ensure!(
        !(count.is_some() && until.is_some()),
        "COUNT and UNTIL cannot both be set"
    );
    let interval = match interval {
        None => 1,
        Some(value) => parse_positive(value, "INTERVAL")?,
    };
    let count = count
        .map(|value| parse_positive(value, "COUNT"))
        .transpose()?
        .map(|n| n as u64);
    let until = until.map(parse_until).transpose()?;
    let kind = match freq.context("RRULE needs FREQ")? {
        "DAILY" => {
            ensure!(
                byday.is_none() && bymonthday.is_none(),
                "FREQ=DAILY takes no BYDAY or BYMONTHDAY"
            );
            Kind::Daily
        }
        "WEEKLY" => {
            ensure!(bymonthday.is_none(), "FREQ=WEEKLY takes no BYMONTHDAY");
            match byday {
                Some(value) => Kind::Weekly(parse_byday(value)?),
                None => Kind::Weekly(vec![start_weekday]),
            }
        }
        "MONTHLY" => {
            ensure!(byday.is_none(), "FREQ=MONTHLY takes no BYDAY");
            match bymonthday {
                Some(value) => Kind::Monthly(parse_bymonthday(value)?),
                None => Kind::Monthly(vec![MonthDay::Day(start_monthday)]),
            }
        }
        other => bail!("FREQ={other} is not supported; use DAILY, WEEKLY or MONTHLY"),
    };
    Ok(Parsed {
        rule: Rule { kind, interval },
        count,
        until,
    })
}

fn parse_positive(value: &str, name: &str) -> Result<i64> {
    let number: i64 = value
        .parse()
        .with_context(|| format!("{name} must be a whole number, not {value:?}"))?;
    ensure!(number >= 1, "{name} must be at least 1");
    Ok(number)
}

/// A UTC iCalendar timestamp in basic form, `YYYYMMDDTHHMMSSZ`.
fn parse_until(text: &str) -> Result<Timestamp> {
    let shape = text.len() == 16
        && text.bytes().enumerate().all(|(index, b)| match index {
            8 => b == b'T',
            15 => b == b'Z',
            _ => b.is_ascii_digit(),
        });
    ensure!(
        shape,
        "UNTIL {text:?} must be a UTC timestamp like 20261231T235959Z"
    );
    let iso = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &text[0..4],
        &text[4..6],
        &text[6..8],
        &text[9..11],
        &text[11..13],
        &text[13..15]
    );
    iso.parse::<Timestamp>()
        .with_context(|| format!("UNTIL {text:?} is not a valid date and time"))
}

fn parse_byday(value: &str) -> Result<Vec<i64>> {
    let mut days = vec![];
    for code in value.split(',') {
        let index = match code {
            "MO" => 0,
            "TU" => 1,
            "WE" => 2,
            "TH" => 3,
            "FR" => 4,
            "SA" => 5,
            "SU" => 6,
            other => bail!("BYDAY value {other:?} is not a plain weekday (MO, TU, ... SU)"),
        };
        days.push(index);
    }
    days.sort_unstable();
    days.dedup();
    Ok(days)
}

fn parse_bymonthday(value: &str) -> Result<Vec<MonthDay>> {
    let mut days = vec![];
    for item in value.split(',') {
        let day = match item.parse::<i64>() {
            Ok(-1) => MonthDay::Last,
            Ok(day @ 1..=31) => MonthDay::Day(day),
            _ => bail!("BYMONTHDAY value {item:?} must be 1 to 31 or -1"),
        };
        days.push(day);
    }
    days.sort_unstable_by_key(|day| match day {
        MonthDay::Day(day) => *day,
        MonthDay::Last => i64::MAX,
    });
    days.dedup();
    Ok(days)
}

fn english_weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

/// Resolves a wall time in `tz`. An overlap gives the earlier instant. A gap gives
/// the transition instant, the first valid instant after the gap.
fn local_to_instant(tz: &TimeZone, wall: DateTime) -> Result<Timestamp> {
    let ambiguous = tz.to_ambiguous_timestamp(wall);
    if matches!(ambiguous.offset(), AmbiguousOffset::Gap { .. }) {
        // `earlier` is before the gap. The next transition is its first valid
        // instant, rather than shifting the requested minute by the gap size.
        return tz
            .following(ambiguous.earlier()?)
            .next()
            .map(|transition| transition.timestamp())
            .context("no transition after a nonexistent local time");
    }
    Ok(ambiguous.earlier()?)
}

/// Days since 1970-01-01 of a jiff date.
fn day_of(date: Date) -> i64 {
    day_number(
        i64::from(date.year()),
        i64::from(date.month()),
        i64::from(date.day()),
    )
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
fn day_number(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// (year, month, day) of a day number. Inverse of `day_number`.
fn civil(day: i64) -> (i64, i64, i64) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Whether a day number is within jiff's date range.
fn in_range(day: i64) -> bool {
    (day_number(-9999, 1, 1)..=day_number(9999, 12, 31)).contains(&day)
}

/// ISO weeks start on Monday. 1970-01-01 was a Thursday.
fn week_of(day: i64) -> i64 {
    (day + 3).div_euclid(7)
}

fn monday_of(week: i64) -> i64 {
    week * 7 - 3
}

fn month_length(year: i64, month: i64) -> i64 {
    match month {
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The distinct days a monthly rule picks in a month of `length` days, sorted.
fn month_days(specs: &[MonthDay], length: i64) -> Vec<i64> {
    let mut days: Vec<i64> = specs
        .iter()
        .filter_map(|spec| match spec {
            MonthDay::Day(day) if *day <= length => Some(*day),
            MonthDay::Day(_) => None,
            MonthDay::Last => Some(length),
        })
        .collect();
    days.sort_unstable();
    days.dedup();
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn raw(at: &str, rrule: Option<&str>) -> Value {
        let mut trigger = json!({"at": at, "tz": "Europe/Berlin"});
        if let Some(rule) = rrule {
            trigger["rrule"] = json!(rule);
        }
        json!({
            "version": 1,
            "trigger": trigger,
            "runner": "hub",
            "tools": ["create_page", "read_page"],
            "steps": [{
                "id": "note", "do": "note.from_template",
                "template": "[[Weekly note]]", "parent": "[[Journal]]",
                "title": "Week {{week}} {{year}}"
            }]
        })
    }

    fn next_strings(value: Value, after: &str, count: usize) -> Vec<String> {
        let schedule = Schedule::parse(value).unwrap();
        schedule
            .next(ts(after), count)
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn between(value: Value, after: &str, through: &str) -> Vec<String> {
        let schedule = Schedule::parse(value).unwrap();
        schedule
            .occurrences(ts(after), ts(through))
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn unknown_keys_are_kept_and_outside_keys_do_not_change_the_hash() {
        let mut value = raw("2026-10-19T08:00", None);
        value["x_future"] = json!({"a": 1});
        value["trigger"]["x_hint"] = json!(true);
        value["steps"][0]["x_color"] = json!("red");
        let schedule = Schedule::parse(value.clone()).unwrap();
        assert_eq!(schedule.raw, value);
        assert_eq!(schedule.runner, "hub");
        assert_eq!(schedule.step.title, "Week {{week}} {{year}}");
        assert_eq!(schedule.step.parent, "[[Journal]]");

        let base = raw("2026-10-19T08:00", None);
        let mut extra_top = base.clone();
        extra_top["x_future"] = json!(1);
        assert_eq!(
            definition_hash(&base).unwrap(),
            definition_hash(&extra_top).unwrap()
        );
        // Step keys are part of the hashed steps.
        let mut extra_step = base.clone();
        extra_step["steps"][0]["x_color"] = json!("red");
        assert_ne!(
            definition_hash(&base).unwrap(),
            definition_hash(&extra_step).unwrap()
        );
    }

    #[test]
    fn refuses_newer_versions_and_unsupported_steps() {
        let mut newer = raw("2026-10-19T08:00", None);
        newer["version"] = json!(2);
        let error = Schedule::parse(newer.clone()).unwrap_err().to_string();
        assert!(error.contains("newer"), "{error}");
        assert!(
            definition_hash(&newer).is_ok(),
            "refused definitions still hash"
        );

        let mut missing_version = raw("2026-10-19T08:00", None);
        missing_version.as_object_mut().unwrap().remove("version");
        assert!(Schedule::parse(missing_version).is_err());

        let mut unknown_do = raw("2026-10-19T08:00", None);
        unknown_do["steps"][0]["do"] = json!("agent.prompt");
        assert!(Schedule::parse(unknown_do).is_err());

        let mut no_create_page = raw("2026-10-19T08:00", None);
        no_create_page["tools"] = json!(["read_page"]);
        let error = Schedule::parse(no_create_page).unwrap_err().to_string();
        assert!(error.contains("create_page"), "{error}");

        // Tool sets: Write and Everything allow the note step, Read does not,
        // and an unknown set name is refused.
        for set in ["write", "everything"] {
            let mut with_set = raw("2026-10-19T08:00", None);
            with_set["tools"] = json!(set);
            let parsed = Schedule::parse(with_set).unwrap();
            assert_eq!(parsed.tools, ScheduleTools::Set(set.parse().unwrap()));
        }
        let mut read_set = raw("2026-10-19T08:00", None);
        read_set["tools"] = json!("read");
        let error = Schedule::parse(read_set).unwrap_err().to_string();
        assert!(
            error.contains("read tool set cannot create pages"),
            "{error}"
        );
        let mut unknown_set = raw("2026-10-19T08:00", None);
        unknown_set["tools"] = json!("admin");
        let error = Schedule::parse(unknown_set).unwrap_err().to_string();
        assert!(error.contains("read, write or everything"), "{error}");
        let mut not_a_list = raw("2026-10-19T08:00", None);
        not_a_list["tools"] = json!(3);
        assert!(Schedule::parse(not_a_list).is_err());

        let mut two_steps = raw("2026-10-19T08:00", None);
        let copy = two_steps["steps"][0].clone();
        two_steps["steps"].as_array_mut().unwrap().push(copy);
        assert_eq!(
            Schedule::parse(two_steps).unwrap_err().to_string(),
            "needs a newer version"
        );

        let mut with_if = raw("2026-10-19T08:00", None);
        with_if["steps"][0]["if"] = json!("{{steps.note.created}}");
        assert_eq!(
            Schedule::parse(with_if).unwrap_err().to_string(),
            "needs a newer version"
        );

        let mut no_steps = raw("2026-10-19T08:00", None);
        no_steps["steps"] = json!([]);
        assert!(Schedule::parse(no_steps).is_err());
    }

    #[test]
    fn refuses_bad_trigger_fields() {
        let mut bad_zone = raw("2026-10-19T08:00", None);
        bad_zone["trigger"]["tz"] = json!("Mars/Olympus");
        assert!(Schedule::parse(bad_zone).is_err());

        for at in [
            "2026-10-19T08:00Z",
            "2026-10-19T08:00+02:00",
            "2026-10-19 08:00",
            "2026-13-19T08:00",
        ] {
            assert!(
                Schedule::parse(raw(at, None)).is_err(),
                "{at} should be refused"
            );
        }
        assert!(Schedule::parse(raw("2026-10-19T08:00:00", None)).is_ok());

        let mut bad_missed = raw("2026-10-19T08:00", None);
        bad_missed["trigger"]["missed"] = json!("soon");
        assert!(Schedule::parse(bad_missed).is_err());

        let schedule = Schedule::parse(raw("2026-10-19T08:00", None)).unwrap();
        assert_eq!(schedule.missed, "latest");
        let mut skip = raw("2026-10-19T08:00", None);
        skip["trigger"]["missed"] = json!("skip");
        assert_eq!(Schedule::parse(skip).unwrap().missed, "skip");
    }

    #[test]
    fn refuses_unsupported_rrule_parts() {
        let bad = [
            "",
            "BYDAY=MO",
            "freq=daily",
            "RRULE:FREQ=DAILY",
            "FREQ=HOURLY",
            "FREQ=YEARLY",
            "FREQ=DAILY;BYHOUR=8",
            "FREQ=DAILY;BYSETPOS=1",
            "FREQ=DAILY;WKST=SU",
            "FREQ=DAILY;FREQ=WEEKLY",
            "FREQ=DAILY;BYDAY=MO",
            "FREQ=DAILY;BYMONTHDAY=1",
            "FREQ=WEEKLY;BYDAY=1MO",
            "FREQ=WEEKLY;BYDAY=-1FR",
            "FREQ=WEEKLY;BYDAY=MO,XX",
            "FREQ=WEEKLY;BYDAY=",
            "FREQ=WEEKLY;BYDAY=MO;BYDAY=TU",
            "FREQ=WEEKLY;BYDAY=MO;BYMONTHDAY=1",
            "FREQ=WEEKLY;BYDAY=MO;BYHOUR=8",
            "FREQ=MONTHLY;BYMONTHDAY=0",
            "FREQ=MONTHLY;BYMONTHDAY=32",
            "FREQ=MONTHLY;BYMONTHDAY=-2",
            "FREQ=MONTHLY;BYMONTHDAY=1,x",
            "FREQ=MONTHLY;BYDAY=MO",
            "FREQ=MONTHLY;BYMONTHDAY=1;BYDAY=MO",
            "FREQ=MONTHLY;BYYEARDAY=1",
        ];
        for rule in bad {
            assert!(
                Schedule::parse(raw("2026-10-19T08:00", Some(rule))).is_err(),
                "{rule} should be refused"
            );
        }
    }

    #[test]
    fn refuses_invalid_interval_count_and_until() {
        let bad = [
            "FREQ=DAILY;INTERVAL=0",
            "FREQ=DAILY;INTERVAL=-2",
            "FREQ=DAILY;INTERVAL=two",
            "FREQ=DAILY;INTERVAL=1.5",
            "FREQ=DAILY;INTERVAL=",
            "FREQ=DAILY;INTERVAL=9223372036854775808",
            "FREQ=DAILY;INTERVAL=2;INTERVAL=3",
            "FREQ=DAILY;COUNT=0",
            "FREQ=DAILY;COUNT=-3",
            "FREQ=DAILY;COUNT=x",
            "FREQ=DAILY;COUNT=2;COUNT=3",
            "FREQ=DAILY;COUNT=2;UNTIL=20261021T060000Z",
            "FREQ=DAILY;UNTIL=2026-10-21",
            "FREQ=DAILY;UNTIL=20261021T060000",
            "FREQ=DAILY;UNTIL=20261321T060000Z",
            "FREQ=DAILY;UNTIL=20261021T060000Zjunk",
        ];
        for rule in bad {
            assert!(
                Schedule::parse(raw("2026-10-19T08:00", Some(rule))).is_err(),
                "{rule} should be refused"
            );
        }
    }

    #[test]
    fn one_off_has_no_rule_and_respects_both_bounds() {
        let value = raw("2026-10-19T08:00", None);
        assert_eq!(
            between(
                value.clone(),
                "2026-10-18T00:00:00Z",
                "2026-10-19T06:00:00Z"
            ),
            ["2026-10-19T06:00:00Z"]
        );
        assert!(between(
            value.clone(),
            "2026-10-19T06:00:00Z",
            "2026-12-31T00:00:00Z"
        )
        .is_empty());
        assert_eq!(
            next_strings(value.clone(), "2026-10-19T05:59:59Z", 5),
            ["2026-10-19T06:00:00Z"]
        );
        assert!(next_strings(value.clone(), "2026-10-19T06:00:00Z", 5).is_empty());
        assert!(next_strings(value, "2026-01-01T00:00:00Z", 0).is_empty());
    }

    #[test]
    fn daily_follows_wall_time_across_dst_end() {
        let value = raw("2026-10-24T08:00", Some("FREQ=DAILY"));
        assert_eq!(
            next_strings(value.clone(), "2026-10-19T00:00:00Z", 3),
            [
                "2026-10-24T06:00:00Z",
                "2026-10-25T07:00:00Z",
                "2026-10-26T07:00:00Z"
            ]
        );
        assert_eq!(
            between(value, "2026-10-24T00:00:00Z", "2026-10-26T00:00:00Z"),
            ["2026-10-24T06:00:00Z", "2026-10-25T07:00:00Z"]
        );
    }

    #[test]
    fn daily_interval_and_count_are_global_from_dtstart() {
        // Oct 19, 21, 23 at 08:00 CEST (06:00Z). COUNT=3 ends the sequence on Oct 23.
        let value = raw("2026-10-19T08:00", Some("FREQ=DAILY;INTERVAL=2;COUNT=3"));
        assert_eq!(
            next_strings(value.clone(), "2026-10-01T00:00:00Z", 10),
            [
                "2026-10-19T06:00:00Z",
                "2026-10-21T06:00:00Z",
                "2026-10-23T06:00:00Z"
            ]
        );
        // Querying later sees only the members of the sequence that are left.
        assert_eq!(
            next_strings(value.clone(), "2026-10-22T00:00:00Z", 10),
            ["2026-10-23T06:00:00Z"]
        );
        assert!(next_strings(value.clone(), "2026-10-23T06:00:00Z", 10).is_empty());
        assert_eq!(
            between(value, "2026-10-20T00:00:00Z", "2026-10-31T00:00:00Z"),
            ["2026-10-21T06:00:00Z", "2026-10-23T06:00:00Z"]
        );

        let uncounted = raw("2026-10-19T08:00", Some("FREQ=DAILY;INTERVAL=2"));
        assert_eq!(
            next_strings(uncounted, "2026-10-20T00:00:00Z", 2),
            ["2026-10-21T06:00:00Z", "2026-10-23T06:00:00Z"]
        );
    }

    #[test]
    fn weekly_occurrences_are_rule_matches() {
        let value = raw("2026-10-16T09:00", Some("FREQ=WEEKLY;BYDAY=MO,FR"));
        assert_eq!(
            next_strings(value, "2026-10-14T00:00:00Z", 4),
            [
                "2026-10-16T07:00:00Z",
                "2026-10-19T07:00:00Z",
                "2026-10-23T07:00:00Z",
                "2026-10-26T08:00:00Z"
            ]
        );
        // DTSTART is a Thursday, not a BYDAY day: it is not an occurrence.
        let off_day = raw("2026-10-15T09:00", Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(
            next_strings(off_day, "2026-10-14T00:00:00Z", 2),
            ["2026-10-19T07:00:00Z", "2026-10-26T08:00:00Z"]
        );
    }

    #[test]
    fn weekly_byday_tuesday_from_monday_starts_next_day() {
        // DTSTART Mon Oct 19 08:00 CEST. The first Tuesday is the next day, and the
        // Monday itself is not an occurrence.
        let value = raw("2026-10-19T08:00", Some("FREQ=WEEKLY;BYDAY=TU"));
        assert_eq!(
            next_strings(value.clone(), "2026-10-18T00:00:00Z", 2),
            ["2026-10-20T06:00:00Z", "2026-10-27T07:00:00Z"]
        );
        assert!(between(value, "2026-10-18T00:00:00Z", "2026-10-19T23:59:59Z").is_empty());
    }

    #[test]
    fn dtstart_is_not_added_unless_the_rule_matches_it() {
        // DTSTART Thu Oct 15 09:00, BYDAY=MO, COUNT=2: the members are Oct 19 and Oct 26.
        let weekly = raw("2026-10-15T09:00", Some("FREQ=WEEKLY;BYDAY=MO;COUNT=2"));
        assert_eq!(
            next_strings(weekly.clone(), "2026-10-14T00:00:00Z", 5),
            ["2026-10-19T07:00:00Z", "2026-10-26T08:00:00Z"]
        );
        assert_eq!(
            next_strings(weekly.clone(), "2026-10-20T00:00:00Z", 5),
            ["2026-10-26T08:00:00Z"]
        );
        assert!(next_strings(weekly, "2026-10-27T00:00:00Z", 5).is_empty());

        // DTSTART Jan 15 09:00, BYMONTHDAY=1, COUNT=2: Jan 1 is before DTSTART and Jan 15
        // does not match, so the members are Feb 1 and Mar 1.
        let monthly = raw(
            "2026-01-15T09:00",
            Some("FREQ=MONTHLY;BYMONTHDAY=1;COUNT=2"),
        );
        assert_eq!(
            next_strings(monthly.clone(), "2026-01-01T00:00:00Z", 5),
            ["2026-02-01T08:00:00Z", "2026-03-01T08:00:00Z"]
        );
        assert_eq!(
            next_strings(monthly, "2026-02-15T00:00:00Z", 5),
            ["2026-03-01T08:00:00Z"]
        );
    }

    #[test]
    fn weekly_interval_byday_count_from_dtstart_despite_later_query() {
        // DTSTART Mon Oct 19. Active weeks: Oct 19 and Nov 2 (every two weeks).
        // Sequence: Oct 19, Oct 21, Nov 2, Nov 4. COUNT=4 ends it there.
        let value = raw(
            "2026-10-19T09:00",
            Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=4"),
        );
        assert_eq!(
            next_strings(value.clone(), "2026-10-01T00:00:00Z", 10),
            [
                "2026-10-19T07:00:00Z",
                "2026-10-21T07:00:00Z",
                "2026-11-02T08:00:00Z",
                "2026-11-04T08:00:00Z"
            ]
        );
        assert_eq!(
            next_strings(value.clone(), "2026-10-30T00:00:00Z", 10),
            ["2026-11-02T08:00:00Z", "2026-11-04T08:00:00Z"]
        );
        assert!(between(value, "2026-11-05T00:00:00Z", "2027-06-01T00:00:00Z").is_empty());

        // Without COUNT the sequence goes on: Nov 16 is the next active Monday.
        let open = raw(
            "2026-10-19T09:00",
            Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE"),
        );
        assert_eq!(
            next_strings(open, "2026-10-30T00:00:00Z", 3),
            [
                "2026-11-02T08:00:00Z",
                "2026-11-04T08:00:00Z",
                "2026-11-16T08:00:00Z"
            ]
        );
    }

    #[test]
    fn weekly_defaults_to_dtstart_weekday() {
        // Oct 21, 2026 is a Wednesday.
        let value = raw("2026-10-21T09:00", Some("FREQ=WEEKLY"));
        assert_eq!(
            next_strings(value, "2026-10-20T00:00:00Z", 3),
            [
                "2026-10-21T07:00:00Z",
                "2026-10-28T08:00:00Z",
                "2026-11-04T08:00:00Z"
            ]
        );
    }

    #[test]
    fn monthly_skips_short_months_and_clamps_nothing() {
        let value = raw("2026-01-31T08:00", Some("FREQ=MONTHLY;BYMONTHDAY=31"));
        assert_eq!(
            next_strings(value, "2026-01-01T00:00:00Z", 6),
            [
                "2026-01-31T07:00:00Z",
                "2026-03-31T06:00:00Z",
                "2026-05-31T06:00:00Z",
                "2026-07-31T06:00:00Z",
                "2026-08-31T06:00:00Z",
                "2026-10-31T07:00:00Z",
            ]
        );
    }

    #[test]
    fn monthly_interval_skips_short_months() {
        // Active months from Jan 2026: Jan, Mar, May, Jul, Sep, Nov, then Jan 2027...
        // Sep and Nov have no 31st, so they are skipped.
        let value = raw(
            "2026-01-31T08:00",
            Some("FREQ=MONTHLY;INTERVAL=2;BYMONTHDAY=31"),
        );
        assert_eq!(
            next_strings(value.clone(), "2026-01-01T00:00:00Z", 4),
            [
                "2026-01-31T07:00:00Z",
                "2026-03-31T06:00:00Z",
                "2026-05-31T06:00:00Z",
                "2026-07-31T06:00:00Z"
            ]
        );
        assert!(between(
            value.clone(),
            "2026-07-31T07:00:00Z",
            "2027-01-30T23:59:00Z"
        )
        .is_empty());
        assert_eq!(
            next_strings(value.clone(), "2026-08-01T00:00:00Z", 2),
            ["2027-01-31T07:00:00Z", "2027-03-31T06:00:00Z"]
        );
        assert_eq!(
            between(value, "2026-08-01T00:00:00Z", "2027-03-31T06:00:00Z"),
            ["2027-01-31T07:00:00Z", "2027-03-31T06:00:00Z"]
        );
    }

    #[test]
    fn monthly_defaults_to_dtstart_day() {
        let value = raw("2026-01-15T09:00", Some("FREQ=MONTHLY"));
        assert_eq!(
            next_strings(value, "2026-01-01T00:00:00Z", 3),
            [
                "2026-01-15T08:00:00Z",
                "2026-02-15T08:00:00Z",
                "2026-03-15T08:00:00Z"
            ]
        );
    }

    #[test]
    fn monthly_last_day_is_valid_in_every_month() {
        let value = raw("2026-01-31T08:00", Some("FREQ=MONTHLY;BYMONTHDAY=-1"));
        assert_eq!(
            next_strings(value, "2026-02-01T00:00:00Z", 3),
            [
                "2026-02-28T07:00:00Z",
                "2026-03-31T06:00:00Z",
                "2026-04-30T06:00:00Z"
            ]
        );
    }

    #[test]
    fn monthly_leap_day_skips_common_years() {
        let value = raw("2027-01-29T08:00", Some("FREQ=MONTHLY;BYMONTHDAY=29"));
        assert_eq!(
            next_strings(value.clone(), "2027-01-30T00:00:00Z", 1),
            ["2027-03-29T06:00:00Z"]
        );
        assert!(between(
            value.clone(),
            "2027-02-01T00:00:00Z",
            "2027-02-28T23:59:59Z"
        )
        .is_empty());
        assert_eq!(
            next_strings(value, "2028-01-31T00:00:00Z", 1),
            ["2028-02-29T07:00:00Z"]
        );
    }

    #[test]
    fn until_is_inclusive_and_global() {
        // Oct 19, 20, 21 at 06:00Z. UNTIL is inclusive, so Oct 21 06:00Z is the last.
        let value = raw(
            "2026-10-19T08:00",
            Some("FREQ=DAILY;UNTIL=20261021T060000Z"),
        );
        assert_eq!(
            next_strings(value.clone(), "2026-10-18T00:00:00Z", 10),
            [
                "2026-10-19T06:00:00Z",
                "2026-10-20T06:00:00Z",
                "2026-10-21T06:00:00Z"
            ]
        );
        assert_eq!(
            next_strings(value, "2026-10-20T12:00:00Z", 10),
            ["2026-10-21T06:00:00Z"]
        );

        // One second earlier excludes Oct 21.
        let earlier = raw(
            "2026-10-19T08:00",
            Some("FREQ=DAILY;UNTIL=20261021T055959Z"),
        );
        assert_eq!(
            next_strings(earlier, "2026-10-18T00:00:00Z", 10),
            ["2026-10-19T06:00:00Z", "2026-10-20T06:00:00Z"]
        );
    }

    #[test]
    fn far_future_queries_use_arithmetic() {
        let count_three = raw("2026-10-19T08:00", Some("FREQ=DAILY;COUNT=3"));
        assert!(Schedule::parse(count_three)
            .unwrap()
            .next(ts("2500-01-01T00:00:00Z"), 5)
            .unwrap()
            .is_empty());

        let huge_daily = raw("2026-10-19T08:00", Some("FREQ=DAILY;COUNT=1000000000"));
        let far = Schedule::parse(huge_daily)
            .unwrap()
            .next(ts("2500-01-01T00:00:00Z"), 2)
            .unwrap();
        assert_eq!(far.len(), 2);
        assert!(far[0] > ts("2500-01-01T00:00:00Z") && far[0] < far[1]);

        let huge_weekly = raw(
            "2026-10-19T08:00",
            Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=1000000"),
        );
        assert_eq!(
            Schedule::parse(huge_weekly)
                .unwrap()
                .next(ts("9000-01-01T00:00:00Z"), 2)
                .unwrap()
                .len(),
            2
        );

        let huge_monthly = raw(
            "2026-10-19T08:00",
            Some("FREQ=MONTHLY;COUNT=5000000;BYMONTHDAY=-1"),
        );
        assert_eq!(
            Schedule::parse(huge_monthly)
                .unwrap()
                .next(ts("9000-01-01T00:00:00Z"), 1)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn berlin_gap_runs_at_the_transition_and_overlap_at_the_earlier_instant() {
        // 2026-03-29 02:00 to 03:00 CET->CEST does not exist: runs at 03:00 CEST.
        let gap = raw("2026-03-29T02:30", None);
        assert_eq!(
            next_strings(gap, "2026-03-28T00:00:00Z", 1),
            ["2026-03-29T01:00:00Z"]
        );
        // 2026-10-25 02:00 to 03:00 CEST->CET happens twice: runs at the first, CEST.
        let overlap = raw("2026-10-25T02:30", None);
        assert_eq!(
            next_strings(overlap, "2026-10-24T00:00:00Z", 1),
            ["2026-10-25T00:30:00Z"]
        );
        // A daily rule goes through the gap day, then returns to normal times.
        let daily = raw("2026-03-28T02:30", Some("FREQ=DAILY"));
        assert_eq!(
            next_strings(daily, "2026-03-28T12:00:00Z", 2),
            ["2026-03-29T01:00:00Z", "2026-03-30T00:30:00Z"]
        );
    }

    #[test]
    fn catch_up_can_exceed_preview_limit() {
        let value = raw("2026-01-01T08:00", Some("FREQ=DAILY"));
        let schedule = Schedule::parse(value).unwrap();
        assert!(
            schedule
                .occurrences(ts("2000-01-01T00:00:00Z"), ts("2100-01-01T00:00:00Z"))
                .unwrap()
                .len()
                > MAX_OCCURRENCES
        );
        assert!(schedule
            .next(ts("2026-01-01T00:00:00Z"), MAX_OCCURRENCES + 1)
            .is_err());
    }

    #[test]
    fn titles_use_iso_week_and_iso_year_at_the_boundary() {
        let schedule = Schedule::parse(raw("2026-12-28T09:00", None)).unwrap();
        assert_eq!(
            schedule.render_title(ts("2026-12-28T08:00:00Z")).unwrap(),
            "Week 53 2026"
        );
        assert_eq!(
            schedule.render_title(ts("2027-01-01T08:00:00Z")).unwrap(),
            "Week 53 2026"
        );
        assert_eq!(
            schedule.render_title(ts("2027-01-04T08:00:00Z")).unwrap(),
            "Week 1 2027"
        );
    }

    #[test]
    fn titles_fill_date_fields_and_leave_unknown_placeholders_literal() {
        let mut value = raw("2027-01-04T09:00", None);
        value["steps"][0]["title"] =
            json!("{{foo}} {{weekday}} {{date}} {{time}} {{week}} {{ year }}");
        let schedule = Schedule::parse(value).unwrap();
        assert_eq!(
            schedule.render_title(ts("2027-01-04T08:00:00Z")).unwrap(),
            "{{foo}} Monday 2027-01-04 09:00 1 {{ year }}"
        );
    }

    #[test]
    fn acceptance_hash_covers_runner_tools_and_steps_only() {
        let base = raw("2026-10-19T08:00", Some("FREQ=DAILY"));
        let hash = Schedule::parse(base.clone()).unwrap().hash();
        assert_eq!(hash, definition_hash(&base).unwrap());

        // A trigger edit keeps acceptance.
        let mut moved = base.clone();
        moved["trigger"]["at"] = json!("2026-10-20T09:00");
        assert_eq!(definition_hash(&moved).unwrap(), hash);

        // Tool order is part of the hash: reordering tools pauses the schedule.
        let mut reordered = base.clone();
        reordered["tools"] = json!(["read_page", "create_page"]);
        assert_ne!(definition_hash(&reordered).unwrap(), hash);
        let mut more_tools = base.clone();
        more_tools["tools"] = json!(["create_page", "read_page", "search_notes"]);
        assert_ne!(definition_hash(&more_tools).unwrap(), hash);
        let mut other_runner = base.clone();
        other_runner["runner"] = json!("desktop");
        assert_ne!(definition_hash(&other_runner).unwrap(), hash);
        let mut other_title = base.clone();
        other_title["steps"][0]["title"] = json!("Day {{date}}");
        assert_ne!(definition_hash(&other_title).unwrap(), hash);
    }

    #[test]
    fn canonical_hash_ignores_object_key_order() {
        let a: Value = serde_json::from_str(
            r#"{"runner":"hub","tools":["create_page"],"steps":[{"do":"note.from_template","id":"n"}]}"#,
        )
        .unwrap();
        let b: Value = serde_json::from_str(
            r#"{"steps":[{"id":"n","do":"note.from_template"}],"tools":["create_page"],"runner":"hub"}"#,
        )
        .unwrap();
        assert_eq!(definition_hash(&a).unwrap(), definition_hash(&b).unwrap());
        assert!(definition_hash(&json!("not an object")).is_err());
    }

    #[test]
    fn end_before_start_is_empty_and_large_intervals_do_not_overflow() {
        let ended = raw(
            "2026-10-19T08:00",
            Some("FREQ=DAILY;UNTIL=20261018T000000Z"),
        );
        assert!(next_strings(ended, "2026-10-01T00:00:00Z", 3).is_empty());
        for freq in ["DAILY", "WEEKLY", "MONTHLY"] {
            let rule = format!("FREQ={freq};INTERVAL=9223372036854775807");
            assert!(next_strings(
                raw("2026-10-19T08:00", Some(&rule)),
                "2026-10-20T00:00:00Z",
                3
            )
            .is_empty());
        }
    }

    #[test]
    fn day_arithmetic_matches_jiff_calendar() {
        assert_eq!(day_number(1970, 1, 1), 0);
        for day in (-800_000..800_000).step_by(7919) {
            let (year, month, date) = civil(day);
            assert_eq!(day_number(year, month, date), day);
            let jiff_date = Date::new(year as i16, month as i8, date as i8).unwrap();
            const NAMES: [&str; 7] = [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ];
            assert_eq!(
                english_weekday(jiff_date.weekday()),
                NAMES[(day + 3).rem_euclid(7) as usize]
            );
        }
    }

    #[test]
    fn schedule_tools_allow_by_set_or_list() {
        let list = ScheduleTools::List(vec!["create_page".into(), "append_blocks".into()]);
        assert!(list.allows("create_page"));
        // An old name counts as the tool it runs.
        assert!(list.allows("edit_page"));
        assert!(!list.allows("trash_page"));
        assert!(!ScheduleTools::Set(ToolSet::Read).allows("create_page"));
        assert!(ScheduleTools::Set(ToolSet::Write).allows("trash_page"));
        assert!(!ScheduleTools::Set(ToolSet::Write).allows("js"));
        assert!(ScheduleTools::Set(ToolSet::Everything).allows("js"));
    }
}
