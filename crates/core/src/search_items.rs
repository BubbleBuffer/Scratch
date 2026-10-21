//! Item search: to-dos, task notes, dated database rows and date mentions as
//! results of a query with `is:`, `done:`, `assignee:`, `calendar:` or
//! `undated`. Page operators (`in:`, `tag:`, properties, ...) filter the
//! item's page, text matches the item's own text, and `date:` its date.
//!
//! The app's search and live query cards show one hit per page (the first
//! matching item is the snippet); tools get every item.

use std::collections::HashSet;

use chrono::{Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::agenda::{AgendaEntry, AGENDA_ORDER, ALL_AGENDA_SOURCES};
use crate::error::{invalid, Result};
use crate::index::Index;
use crate::search_ops::ITEM_KINDS;
use crate::search_query::{alternatives, is_item_or_shape, strip_brackets, SearchSpec, ITEM_DATE};
use crate::types::{
    CalendarSettings, Filter, FilterOp, MatchedFilterValue, PageId, PropValue, SearchHit,
    SearchSpecResults, VAULT_SOURCE,
};
use crate::vault::{lock, Vault};

/// One item a query matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemHit {
    pub page: PageId,
    pub page_title: String,
    /// The to-do or mention block; none for task notes and rows.
    pub block: Option<String>,
    /// `todo`, `task`, `row` or `mention`.
    pub kind: String,
    /// The item's own text (a task note's or row's title).
    pub title: String,
    pub done: bool,
    /// None when the item has no date.
    pub date_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub has_time: bool,
    pub calendar_name: Option<String>,
    pub assignee: Option<String>,
    pub status: Option<String>,
    /// The database of a row.
    pub collection: Option<PageId>,
    /// The reference `update_task` accepts: `block:<page>/<block>`,
    /// `note:<page>`, `row:<database>/<row>`, or `mention:<page>/<block>`
    /// (read-only).
    pub reference: String,
}

/// A window of item hits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemResults {
    pub items: Vec<ItemHit>,
    /// All matching items, before the window.
    pub total: u32,
    pub warnings: Vec<String>,
}

fn kind_name(source: &str) -> &'static str {
    ITEM_KINDS
        .iter()
        .find(|(_, s)| *s == source)
        .map_or("todo", |(name, _)| name)
}

fn is_item_date(filter: &Option<Filter>) -> bool {
    fn walk(f: &Filter) -> bool {
        match f {
            Filter::Cond(c) => c.prop == ITEM_DATE,
            Filter::And(v) | Filter::Or(v) => v.iter().any(walk),
        }
    }
    filter.as_ref().is_some_and(walk)
}

/// The item's span in epoch ms. All-day dates are stored as UTC midnights;
/// they compare as the local day, as the agenda does.
fn span(entry: &AgendaEntry) -> Option<(i64, i64)> {
    let start = entry.date_ms?;
    let end = entry.item.end_ms.unwrap_or(start).max(start);
    if entry.item.has_time {
        return Some((start, end));
    }
    let local = |ms: i64| {
        let day = Utc.timestamp_millis_opt(ms).single()?.date_naive();
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
            .earliest()
            .map(|d| d.timestamp_millis())
    };
    Some((local(start)?, local(end)?))
}

/// The local midnight after the local day that starts at `ms`.
fn day_after(ms: i64) -> Option<i64> {
    let day = Local
        .timestamp_millis_opt(ms)
        .single()?
        .date_naive()
        .succ_opt()?;
    Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|d| d.timestamp_millis())
}

fn date_matches(filter: &Filter, entry: &AgendaEntry) -> bool {
    match filter {
        Filter::And(v) => v.iter().all(|f| date_matches(f, entry)),
        Filter::Or(v) => v.iter().any(|f| date_matches(f, entry)),
        Filter::Cond(c) => {
            let Some((start, end)) = span(entry) else {
                return false;
            };
            let PropValue::Date {
                start_ms,
                end_ms,
                has_time,
            } = c.value
            else {
                return false;
            };
            // Against an instant (`now`), an all-day item lasts its whole
            // day: it is before now only once its day is over.
            if has_time
                && !entry.item.has_time
                && matches!(c.op, FilterOp::Before | FilterOp::OnOrBefore)
            {
                return day_after(end).is_some_and(|after| after <= start_ms);
            }
            match c.op {
                FilterOp::WithinRange => end >= start_ms && end_ms.map_or(true, |e| start < e),
                FilterOp::Before => start < start_ms,
                FilterOp::OnOrBefore => start <= start_ms,
                FilterOp::After => start > start_ms,
                FilterOp::OnOrAfter => start >= start_ms,
                _ => false,
            }
        }
    }
}

fn contains_fold(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Whether an entry passes every item operator and the text terms.
fn item_matches(spec: &SearchSpec, entry: &AgendaEntry) -> bool {
    let item = &entry.item;
    for op in spec.operators.iter().filter(|op| !op.property) {
        let pass = match op.key.as_str() {
            "is" => alternatives(&op.value).iter().any(|kind| {
                ITEM_KINDS
                    .iter()
                    .any(|(name, source)| name == kind && *source == entry.source)
            }),
            "done" => item.done == (op.value == "yes"),
            "assignee" => {
                entry.source == "task_note"
                    && alternatives(&op.value)
                        .iter()
                        .any(|who| item.assignee.as_deref() == Some(who.as_str()))
            }
            "calendar" => alternatives(&op.value)
                .iter()
                .any(|id| item.calendar.as_deref() == Some(strip_brackets(id))),
            "undated" => entry.date_ms.is_none(),
            "date" if is_item_date(&op.filter) => {
                op.filter.as_ref().is_some_and(|f| date_matches(f, entry))
            }
            _ => continue,
        };
        if pass == op.negated {
            return false;
        }
    }
    spec.terms.iter().all(|term| {
        let words: Vec<&str> = if term.phrase {
            vec![term.value.as_str()]
        } else {
            term.value.split_whitespace().collect()
        };
        let found = words.iter().all(|w| contains_fold(&item.title, w));
        found != term.exclude
    })
}

/// The update_task reference of an entry.
fn reference(entry: &AgendaEntry) -> String {
    let item = &entry.item;
    match (&item.block, entry.source.as_str()) {
        (Some(block), "mention") => format!("mention:{}/{block}", item.page),
        (Some(block), _) => format!("block:{}/{block}", item.page),
        (None, "task_note") => format!("note:{}", item.page),
        (None, _) => format!(
            "row:{}/{}",
            item.collection.as_deref().unwrap_or_default(),
            item.page
        ),
    }
}

impl Index {
    /// Every item the spec matches, in agenda order (or by `sort:title` /
    /// `sort:updated`), with the warnings of its page operators.
    pub(crate) fn search_item_entries(
        &self,
        spec: &SearchSpec,
        settings: &CalendarSettings,
    ) -> Result<(Vec<AgendaEntry>, Vec<String>)> {
        let _snapshot = self
            .conn
            .is_autocommit()
            .then(|| self.conn.unchecked_transaction())
            .transpose()?;
        let mut page_spec = spec.clone();
        page_spec.terms.clear();
        page_spec.sort = None;
        page_spec
            .operators
            .retain(|op| !is_item_or_shape(op) && !is_item_date(&op.filter));
        let eligible = crate::search_exec::candidates(self, &page_spec)?;
        let mut warnings = spec.warnings.clone();
        if let Some(set) = &eligible {
            warnings.extend(set.warnings.iter().cloned());
        }
        let mut entries: Vec<AgendaEntry> = self
            .agenda_entries(
                ALL_AGENDA_SOURCES,
                &[],
                AGENDA_ORDER,
                None,
                settings,
                false,
                false,
            )?
            .into_iter()
            .filter(|e| {
                eligible
                    .as_ref()
                    .map_or(true, |set| set.ids.contains(&e.item.page))
            })
            .filter(|e| item_matches(spec, e))
            .collect();
        if let Some(sort) = spec.sort.as_deref() {
            let (key, direction) = crate::search_query::sort_direction(sort);
            match key {
                "title" => entries.sort_by_key(|e| e.item.title.to_lowercase()),
                "updated" => entries.sort_by(|a, b| b.item.updated_ms.cmp(&a.item.updated_ms)),
                _ => {}
            }
            let default_desc = key == "updated";
            if matches!(key, "title" | "updated") && direction.is_some_and(|d| d != default_desc) {
                entries.reverse();
            }
        }
        Ok((entries, warnings))
    }

    fn page_title(&self, page: &str) -> Result<String> {
        Ok(self
            .conn
            .prepare_cached("SELECT title FROM pages WHERE id = ?1")?
            .query_row([page], |r| r.get(0))
            .unwrap_or_default())
    }

    /// The app's view of an item search: one hit per page, in the order of
    /// its first matching item, whose text is the snippet.
    pub(crate) fn item_page_results(
        &self,
        spec: &SearchSpec,
        limit: u32,
        offset: u32,
    ) -> Result<SearchSpecResults> {
        let (entries, warnings) = self.search_item_entries(spec, &CalendarSettings::default())?;
        let mut seen = HashSet::new();
        let firsts: Vec<_> = entries
            .into_iter()
            .filter(|e| seen.insert(e.item.page.clone()))
            .collect();
        let total_bounded = firsts.len().min(201) as u32;
        let mut hits = Vec::new();
        for entry in firsts.iter().skip(offset as usize).take(limit as usize) {
            let mut matched = vec![MatchedFilterValue {
                key: "done".into(),
                value: if entry.item.done { "yes" } else { "no" }.into(),
            }];
            if let Some(date) = entry
                .date_ms
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
            {
                matched.push(MatchedFilterValue {
                    key: "date".into(),
                    value: date.format("%Y-%m-%d").to_string(),
                });
            }
            hits.push(SearchHit {
                id: entry.item.page.clone(),
                title: self.page_title(&entry.item.page)?,
                snippet: entry.item.title.clone(),
                fuzzy: false,
                corrected: None,
                matched_filters: matched,
                source: VAULT_SOURCE.to_string(),
                page_type: crate::types::default_page_type(),
                target: None,
            });
        }
        Ok(SearchSpecResults {
            hits,
            total_bounded,
            narrowed: false,
            warnings,
        })
    }
}

impl Vault {
    /// The calendar settings, read without initializing the Calendars page.
    pub(crate) fn calendar_settings_readonly(&self) -> Result<CalendarSettings> {
        match self.find_by_role("calendars".into())? {
            Some(page) => {
                serde_json::from_str::<CalendarSettings>(&self.type_data(page, "tasks".into())?)
                    .map_err(|_| invalid("invalid calendar settings"))
            }
            None => Ok(CalendarSettings::default()),
        }
    }

    /// Calendar categories as (id, name), for `calendar:`.
    pub(crate) fn calendar_names(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .calendar_settings_readonly()?
            .calendars
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect())
    }

    /// The items an item query matches (see [`SearchSpec::selects_items`]),
    /// windowed by `limit` and `offset`.
    pub fn search_items(&self, spec: &SearchSpec, limit: u32, offset: u32) -> Result<ItemResults> {
        let settings = self.calendar_settings_readonly()?;
        let db = lock(&self.inner.db);
        let (entries, warnings) = db.search_item_entries(spec, &settings)?;
        let total = entries.len() as u32;
        let mut items = Vec::new();
        for entry in entries.iter().skip(offset as usize).take(limit as usize) {
            let item = &entry.item;
            items.push(ItemHit {
                page: item.page.clone(),
                page_title: db.page_title(&item.page)?,
                block: item.block.clone(),
                kind: kind_name(&entry.source).to_string(),
                title: item.title.clone(),
                done: item.done,
                date_ms: entry.date_ms,
                end_ms: item.end_ms,
                has_time: item.has_time,
                calendar_name: entry.calendar_name.clone(),
                assignee: item.assignee.clone(),
                status: item.status.clone(),
                collection: item.collection.clone(),
                reference: reference(entry),
            });
        }
        Ok(ItemResults {
            items,
            total,
            warnings,
        })
    }
}
