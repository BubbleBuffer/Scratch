//! `search_notes` results in an rg-like shape, and the hidden aliases that
//! map `agenda`, `query_database` and `memory_search` onto a query.
//!
//! Each page hit gives its title, id and path, then the matching blocks
//! with `context` blocks around them, each block with its id (the form
//! `edit_page` takes). A page of at most `full` characters is shown whole;
//! a filter-only query shows the start of each page. Item queries (`is:`,
//! `done:` ...) list to-dos, task notes, rows and mentions with their done
//! state, date and `update_task` reference. The whole result stays within
//! [`OUTPUT_BUDGET_CHARS`]: previews shrink before hits are dropped, and the
//! result says what was trimmed.

use std::collections::HashMap;

use chrono::{Local, TimeZone, Utc};
use serde_json::{json, Value};
use vault_core::search_ops::{
    DEFAULT_CONTEXT, DEFAULT_FULL_CHARS, DEFAULT_LIMIT, MAX_LIMIT, OUTPUT_BUDGET_CHARS,
};
use vault_core::{Block, ItemHit, Page, ParsedQuery, SearchHit, TokenKind, Vault, VAULT_SOURCE};

use crate::{err, Result, ToolError};

/// Characters of a page's start shown for a filter-only hit.
const PREVIEW_CHARS: usize = 300;
/// A block line's length at the tightest detail level.
const TIGHT_LINE_CHARS: usize = 160;

/// How much of each hit to show.
#[derive(Clone, Copy)]
struct Detail {
    context: usize,
    full: usize,
    line_chars: Option<usize>,
}

/// One block of a page in document order.
struct Line {
    id: String,
    depth: usize,
    markdown: String,
    plain: String,
}

fn flatten(v: &Vault, blocks: &[Block], depth: usize, out: &mut Vec<Line>) {
    for b in blocks {
        let mut own = b.clone();
        own.children.clear();
        out.push(Line {
            id: b.id.clone(),
            depth,
            markdown: v.blocks_markdown(vec![own]),
            plain: b.text.to_lowercase(),
        });
        flatten(v, &b.children, depth + 1, out);
    }
}

fn shorten(text: &str, max: Option<usize>) -> String {
    match max {
        Some(max) if text.chars().count() > max => {
            let mut s: String = text.chars().take(max).collect();
            s.push('…');
            s
        }
        _ => text.to_string(),
    }
}

fn block_line(line: &Line, detail: Detail) -> String {
    let indent = "  ".repeat(line.depth);
    let body = shorten(&line.markdown, detail.line_chars);
    let pad = format!("\n    {indent}");
    format!("  `{}` | {indent}{}\n", line.id, body.replace('\n', &pad))
}

/// Titles of the page's ancestors, outermost first.
pub(crate) fn path_of(
    v: &Vault,
    page: &Page,
    cache: &mut HashMap<String, Option<(String, Option<String>)>>,
) -> String {
    let mut titles = Vec::new();
    let mut parent = page.parent.clone();
    let mut guard = 0;
    while let Some(id) = parent {
        guard += 1;
        if guard > 64 {
            break;
        }
        let entry = cache
            .entry(id.clone())
            .or_insert_with(|| v.load_page(id.clone()).ok().map(|p| (p.title, p.parent)))
            .clone();
        let Some((title, next)) = entry else { break };
        titles.push(if title.trim().is_empty() {
            "Untitled".to_string()
        } else {
            title
        });
        parent = next;
    }
    titles.reverse();
    titles.join(" / ")
}

fn title_or(t: &str) -> &str {
    if t.trim().is_empty() {
        "Untitled"
    } else {
        t
    }
}

fn header(title: &str, id: &str, path: &str, extra: &str) -> String {
    let mut out = format!("- **{}** `{id}`", title_or(title));
    if !path.is_empty() {
        out.push_str(&format!(" · in {path}"));
    }
    out.push_str(extra);
    out.push('\n');
    out
}

/// The words a block must contain to count as a match.
fn needles(parsed: &ParsedQuery) -> Vec<String> {
    let mut out = Vec::new();
    for term in parsed.spec.terms.iter().filter(|t| !t.exclude) {
        if term.phrase {
            out.push(term.value.to_lowercase());
        } else {
            out.extend(term.value.split_whitespace().map(str::to_lowercase));
        }
    }
    out
}

struct PageHit<'a> {
    hit: &'a SearchHit,
    page: Option<Page>,
    path: String,
    lines: Vec<Line>,
}

fn render_page(h: &PageHit, needles: &[String], detail: Detail) -> String {
    let hit = h.hit;
    let mut extra = String::new();
    if hit.page_type == "database" {
        extra.push_str(" (database)");
    }
    if !hit.matched_filters.is_empty() {
        let matched: Vec<String> = hit
            .matched_filters
            .iter()
            .map(|m| format!("{}: {}", m.key, m.value))
            .collect();
        extra.push_str(&format!(" [{}]", matched.join(", ")));
    }
    if let Some(corrected) = hit.corrected.as_deref().filter(|_| hit.fuzzy) {
        extra.push_str(&format!(" (typo match for \"{corrected}\")"));
    }
    let mut out = header(&hit.title, &hit.id, &h.path, &extra);
    if h.page.is_none() {
        // External rows (agent chats) have no blocks; show the snippet.
        let snip = hit.snippet.replace('\n', " ");
        if !snip.trim().is_empty() {
            out.push_str(&format!(
                "  {}\n",
                shorten(snip.trim(), detail.line_chars.or(Some(PREVIEW_CHARS)))
            ));
        }
        return out;
    }
    let lines = &h.lines;
    if lines.is_empty() {
        out.push_str("  (empty page)\n");
        return out;
    }
    let size: usize = lines.iter().map(|l| l.markdown.len()).sum();
    if detail.full > 0 && size <= detail.full {
        for line in lines {
            out.push_str(&block_line(line, detail));
        }
        return out;
    }
    let matches: Vec<usize> = if needles.is_empty() {
        Vec::new()
    } else {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| needles.iter().any(|n| l.plain.contains(n.as_str())))
            .map(|(i, _)| i)
            .collect()
    };
    if matches.is_empty() {
        // Filter-only, or matched by title or a property: the page's start.
        let mut shown = 0;
        for (i, line) in lines.iter().enumerate() {
            if i > 0 && shown + line.markdown.len() > PREVIEW_CHARS {
                out.push_str(&format!("  … {} more blocks\n", lines.len() - i));
                break;
            }
            shown += line.markdown.len();
            out.push_str(&block_line(
                line,
                Detail {
                    line_chars: detail.line_chars.or(Some(PREVIEW_CHARS)),
                    ..detail
                },
            ));
        }
        return out;
    }
    let mut keep = vec![false; lines.len()];
    for &m in &matches {
        let from = m.saturating_sub(detail.context);
        let to = (m + detail.context).min(lines.len() - 1);
        for k in keep.iter_mut().take(to + 1).skip(from) {
            *k = true;
        }
    }
    let mut last: Option<usize> = None;
    for (i, line) in lines.iter().enumerate().filter(|(i, _)| keep[*i]) {
        if last.is_some_and(|l| i > l + 1) || (last.is_none() && i > 0) {
            out.push_str("  …\n");
        }
        out.push_str(&block_line(line, detail));
        last = Some(i);
    }
    if last.is_some_and(|l| l + 1 < lines.len()) {
        out.push_str("  …\n");
    }
    out
}

/// "How the query was read" lines: the canonical query, resolved pages and warnings.
pub(crate) fn reading(parsed: &ParsedQuery, extra_warnings: &[String]) -> String {
    let mut out = format!("Read as: `{}`\n", parsed.spec.to_query_string());
    let resolved: Vec<String> = parsed
        .spec
        .operators
        .iter()
        .filter_map(|op| {
            let id = op.resolved_id.as_ref()?;
            let sign = if op.negated { "-" } else { "" };
            Some(format!("`{sign}{}:{}` → `{id}`", op.key, op.value))
        })
        .collect();
    if !resolved.is_empty() {
        out.push_str(&format!("Resolved: {}\n", resolved.join(", ")));
    }
    let mut warnings = parsed.spec.warnings.clone();
    for t in &parsed.tokens {
        if let TokenKind::Error { message } = &t.kind {
            if !warnings.contains(message) {
                warnings.push(message.clone());
            }
        }
    }
    for w in extra_warnings {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    if !warnings.is_empty() {
        out.push_str("Warnings:\n");
        for w in &warnings {
            out.push_str(&format!("- {w}\n"));
        }
    }
    out
}

/// Fits rendered hits into the budget: tighter detail first, then fewer hits.
fn fit<F: Fn(usize, Detail) -> String>(
    count: usize,
    head: usize,
    requested: Detail,
    render: F,
) -> (Vec<String>, Option<String>) {
    let levels = [
        requested,
        Detail {
            context: 0,
            full: 0,
            line_chars: None,
        },
        Detail {
            context: 0,
            full: 0,
            line_chars: Some(TIGHT_LINE_CHARS),
        },
    ];
    let budget = OUTPUT_BUDGET_CHARS.saturating_sub(head);
    let mut last = Vec::new();
    for (n, detail) in levels.iter().enumerate() {
        let parts: Vec<String> = (0..count).map(|i| render(i, *detail)).collect();
        let size: usize = parts.iter().map(String::len).sum();
        if size <= budget {
            let note = match n {
                0 => None,
                1 => Some("previews shortened to the matching blocks (no context, no whole pages)".to_string()),
                _ => Some(format!("previews shortened to matching blocks of at most {TIGHT_LINE_CHARS} characters")),
            };
            return (parts, note);
        }
        last = parts;
    }
    let mut size = 0;
    let mut kept = Vec::new();
    for part in last {
        if size + part.len() > budget && !kept.is_empty() {
            break;
        }
        size += part.len();
        kept.push(part);
    }
    let dropped = count - kept.len();
    (
        kept,
        Some(format!(
            "previews shortened to matching blocks of at most {TIGHT_LINE_CHARS} characters, and the last {dropped} hit(s) left out"
        )),
    )
}

fn trimmed_note(note: Option<String>, next_offset: usize) -> String {
    match note {
        Some(what) => format!(
            "\n_Trimmed to fit {OUTPUT_BUDGET_CHARS} characters: {what}. For more: narrow the query, use limit:N or offset={next_offset}, or read_page._"
        ),
        None => String::new(),
    }
}

/// Resolves a memory scope (`project` or `project/subproject`) to the
/// container page, for the memory_search alias.
fn with_memory_scope(v: &Vault, query: String, a: &Value) -> Result<String> {
    match a.get("__memory_scope").and_then(Value::as_str) {
        None | Some("all") => Ok(query),
        Some(scope) => {
            let id = crate::memory::scope_container(v, scope)?;
            Ok(format!("{query} in:[[{id}]]"))
        }
    }
}

/// search_notes with a query.
pub(crate) fn search(v: &Vault, a: &Value) -> Result<String> {
    let q = a
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError("missing argument: query".into()))?;
    let q = with_memory_scope(v, q.to_string(), a)?;
    let offset = a
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(10_000) as u32;
    // Content edits reach the index through the background writer.
    let _ = v.flush();
    let parsed = v.parse_search(q.clone())?;
    let spec = &parsed.spec;
    let arg_limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n.clamp(1, MAX_LIMIT as u64) as u32);
    let limit = spec.shape("limit").or(arg_limit).unwrap_or(DEFAULT_LIMIT);
    let detail = Detail {
        context: spec.shape("context").unwrap_or(DEFAULT_CONTEXT) as usize,
        full: spec.shape("full").unwrap_or(DEFAULT_FULL_CHARS) as usize,
        line_chars: None,
    };
    if spec.selects_items() {
        return items(v, &q, &parsed, limit, offset, detail);
    }
    // Tools need body recall even for broad queries; narrowing is for typing.
    let hits = v.search_full(q.clone(), offset + limit + 1)?;
    let window: Vec<&SearchHit> = hits
        .iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let mut out = reading(&parsed, &[]);
    if window.is_empty() {
        out.push_str(&format!("No pages match {q:?}."));
        return Ok(out.trim_end().to_string());
    }
    let mut cache = HashMap::new();
    let page_hits: Vec<PageHit> = window
        .iter()
        .map(|hit| {
            let page = (hit.source == VAULT_SOURCE)
                .then(|| v.load_page(hit.id.clone()).ok())
                .flatten();
            let path = page
                .as_ref()
                .map(|p| path_of(v, p, &mut cache))
                .unwrap_or_default();
            let mut lines = Vec::new();
            if let Some(p) = &page {
                flatten(v, &p.blocks, 0, &mut lines);
            }
            PageHit {
                hit,
                page,
                path,
                lines,
            }
        })
        .collect();
    let needles = needles(&parsed);
    out.push('\n');
    let (parts, note) = fit(page_hits.len(), out.len(), detail, |i, d| {
        render_page(&page_hits[i], &needles, d)
    });
    let shown = parts.len();
    for part in parts {
        out.push_str(&part);
    }
    let next = offset as usize + shown;
    if hits.len() > (offset + limit) as usize || shown < window.len() {
        out.push_str(&format!("\n_More: offset={next}_"));
    }
    out.push_str(&trimmed_note(note, next));
    Ok(out.trim_end().to_string())
}

fn item_date(item: &ItemHit) -> Option<String> {
    let ms = item.date_ms?;
    if item.has_time {
        let at = Local.timestamp_millis_opt(ms).single()?;
        Some(at.format("%Y-%m-%d %H:%M").to_string())
    } else {
        Some(
            Utc.timestamp_millis_opt(ms)
                .single()?
                .format("%Y-%m-%d")
                .to_string(),
        )
    }
}

fn render_item(item: &ItemHit, path: &str, with_header: bool, detail: Detail) -> String {
    let mut out = if with_header {
        header(&item.page_title, &item.page, path, "")
    } else {
        String::new()
    };
    let id = item.block.as_deref().unwrap_or("-");
    let mut line = format!(
        "  `{id}` | [{}] {}",
        if item.done { "x" } else { " " },
        shorten(&item.title.replace('\n', " "), detail.line_chars)
    );
    let mut facts = vec![item.kind.clone()];
    match item_date(item) {
        Some(date) => facts.push(format!("due {date}")),
        None => facts.push("no date".into()),
    }
    if let Some(status) = &item.status {
        facts.push(format!("status {status}"));
    }
    if let Some(who) = &item.assignee {
        facts.push(format!("assignee {who}"));
    }
    if let Some(calendar) = &item.calendar_name {
        facts.push(format!("calendar {calendar}"));
    }
    facts.push(format!("ref {}", item.reference));
    line.push_str(&format!(" · {}\n", facts.join(" · ")));
    out.push_str(&line);
    out
}

fn items(
    v: &Vault,
    q: &str,
    parsed: &ParsedQuery,
    limit: u32,
    offset: u32,
    detail: Detail,
) -> Result<String> {
    let results = v.search_items(&parsed.spec, limit, offset)?;
    let mut out = reading(parsed, &results.warnings);
    if results.items.is_empty() {
        out.push_str(&format!("No items match {q:?}."));
        return Ok(out.trim_end().to_string());
    }
    let mut cache = HashMap::new();
    let mut paths: HashMap<String, String> = HashMap::new();
    for item in &results.items {
        if !paths.contains_key(&item.page) {
            let path = v
                .load_page(item.page.clone())
                .map(|p| path_of(v, &p, &mut cache))
                .unwrap_or_default();
            paths.insert(item.page.clone(), path);
        }
    }
    let first = offset as usize + 1;
    out.push_str(&format!(
        "\n{} item(s) match; {}–{} below.\n",
        results.total,
        first,
        offset as usize + results.items.len()
    ));
    let items = &results.items;
    let (parts, note) = fit(items.len(), out.len(), detail, |i, d| {
        let with_header = i == 0 || items[i - 1].page != items[i].page;
        render_item(&items[i], &paths[&items[i].page], with_header, d)
    });
    let shown = parts.len();
    for part in parts {
        out.push_str(&part);
    }
    let next = offset as usize + shown;
    if (next as u32) < results.total {
        out.push_str(&format!("\n_More: offset={next}_"));
    }
    out.push_str(&trimmed_note(note, next));
    Ok(out.trim_end().to_string())
}

/// Lines naming pages whose titles contain the words of `text`, for misses.
pub(crate) fn title_matches(v: &Vault, text: &str, limit: u32) -> Vec<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| format!("title:{}", quote(w)))
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<_> = v
        .search_full(words.join(" "), limit)
        .unwrap_or_default()
        .into_iter()
        .filter(|h| h.source == VAULT_SOURCE)
        .collect();
    if hits.is_empty() && words.len() > 1 {
        // No title has every word (a typo, or an extra word): rank titles by
        // how many of the words they contain.
        let mut ranked: Vec<(usize, SearchHit)> = Vec::new();
        for word in &words {
            for h in v.search_full(word.clone(), limit).unwrap_or_default() {
                if h.source != VAULT_SOURCE {
                    continue;
                }
                match ranked.iter_mut().find(|(_, r)| r.id == h.id) {
                    Some((n, _)) => *n += 1,
                    None => ranked.push((1, h)),
                }
            }
        }
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        hits = ranked
            .into_iter()
            .take(limit as usize)
            .map(|(_, h)| h)
            .collect();
    }
    let mut cache = HashMap::new();
    hits.iter()
        .map(|h| {
            let path = v
                .load_page(h.id.clone())
                .map(|p| path_of(v, &p, &mut cache))
                .unwrap_or_default();
            header(&h.title, &h.id, &path, "").trim_end().to_string()
        })
        .collect()
}

/// read_page on a miss: a title search, labelled as search results.
pub(crate) fn read_page_miss(v: &Vault, text: &str) -> String {
    let lines = title_matches(v, text, 10);
    if lines.is_empty() {
        return format!("No page has the id or exact title {text:?}, and no titles contain those words. Use search_notes to look further.");
    }
    format!(
        "No page has the id or exact title {text:?}. Search results for pages whose titles contain those words (not opened; read_page one of these ids):\n\n{}",
        lines.join("\n")
    )
}

// ------------------------------------------------------------ aliases

/// A value quoted for the query language when it needs quotes.
pub(crate) fn quote(s: &str) -> String {
    if !s.is_empty()
        && !s
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, ',' | '"' | '\\' | '[' | ']' | ':'))
    {
        return s.to_string();
    }
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn page_ref(s: &str) -> String {
    format!("[[{}]]", s.trim())
}

fn strings(a: &Value, key: &str) -> Result<Vec<String>> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ToolError(format!("{key} must be an array of strings")))
            })
            .collect(),
        Some(_) => err(format!("{key} must be an array of strings")),
    }
}

/// A strict YYYY-MM-DD date.
fn iso_day(s: &str) -> Result<chrono::NaiveDate> {
    let shaped = s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-';
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .filter(|_| shaped)
        .ok_or_else(|| ToolError(format!("invalid ISO date: {s}; expected YYYY-MM-DD")))
}

fn day_before(s: &str) -> Result<String> {
    let day = iso_day(s)?;
    Ok(day.pred_opt().unwrap_or(day).format("%Y-%m-%d").to_string())
}

fn opt_str<'a>(a: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => err(format!("{key} must be a string")),
    }
}

fn opt_bool(a: &Value, key: &str) -> Result<Option<bool>> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => err(format!("{key} must be true or false")),
    }
}

fn paging(a: &Value, query: Vec<String>) -> Result<Value> {
    let mut out = json!({ "query": query.join(" ") });
    match a.get("offset") {
        None | Some(Value::Null) => {}
        Some(offset) if offset.as_u64().is_some() => out["offset"] = offset.clone(),
        Some(_) => return err("offset must be a non-negative integer"),
    }
    Ok(out)
}

/// Item kinds the agenda views list when no kinds are given: mentions are
/// references, not work, so today and overdue leave them out.
const WORK_KINDS: [&str; 3] = ["todo", "task", "row"];

/// agenda's arguments as a query.
pub(crate) fn agenda_query(a: &Value) -> Result<Value> {
    let mut q = Vec::new();
    let given = a.get("kinds").is_some_and(|k| !k.is_null());
    let kinds = strings(a, "kinds")?;
    if given && kinds.is_empty() {
        return err("kinds must not be empty; leave it out for every kind");
    }
    let mut is = Vec::new();
    for kind in &kinds {
        is.push(match kind.as_str() {
            "todo" => "todo",
            "task_note" => "task",
            "row" => "row",
            "mention" => "mention",
            _ => return err("kinds must contain todo, task_note, row or mention"),
        });
    }
    let view = opt_str(a, "view")?;
    if is.is_empty() {
        is = if view == Some("overdue") {
            // Overdue is what can be ticked off: past events and mentions
            // are not late.
            vec!["todo", "task"]
        } else if view.is_some() {
            WORK_KINDS.to_vec()
        } else {
            vec!["todo", "task", "row", "mention"]
        };
    }
    q.push(format!("is:{}", is.join(",")));
    match view {
        None => {}
        Some("today") => q.push("date:today".into()),
        Some("overdue") => q.push("date:<now done:no".into()),
        Some(_) => return err("view must be today or overdue"),
    }
    let from = opt_str(a, "from")?;
    let to = opt_str(a, "to")?;
    if let Some(f) = from {
        iso_day(f)?;
    }
    if let Some(t) = to {
        iso_day(t)?;
    }
    match (from, to) {
        (Some(f), Some(t)) => {
            if f >= t {
                return err("to must be after from (to is exclusive)");
            }
            q.push(format!("date:{f}..{}", day_before(t)?));
        }
        (Some(f), None) => q.push(format!("date:>={f}")),
        (None, Some(t)) => q.push(format!("date:<{t}")),
        (None, None) => {}
    }
    if opt_bool(a, "undated")? == Some(true) {
        q.push("undated".into());
    }
    let calendars = strings(a, "calendar")?;
    if a.get("calendar").is_some_and(|c| !c.is_null()) && calendars.is_empty() {
        return err("calendar must not be empty; leave it out for every calendar");
    }
    if !calendars.is_empty() {
        q.push(format!(
            "calendar:{}",
            calendars
                .iter()
                .map(|c| quote(c))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    match opt_str(a, "done")?.unwrap_or("any") {
        "any" => {}
        "open" => q.push("done:no".into()),
        "done" => q.push("done:yes".into()),
        _ => return err("done must be open, done or any"),
    }
    if let Some(who) = opt_str(a, "assignee")? {
        if !matches!(who, "me" | "agent") {
            return err("assignee must be me or agent");
        }
        q.push(format!("assignee:{who}"));
    }
    if let Some(tag) = opt_str(a, "tag")? {
        q.push(format!("tag:{}", quote(tag)));
    }
    let subtree = opt_bool(a, "subtree")?;
    if let Some(page) = opt_str(a, "page")? {
        let page = page_ref(page);
        q.push(format!("in:{page}"));
        if subtree != Some(true) {
            // That page only: its subtree minus everything below it.
            q.push(format!("-in:{page}/**"));
        }
    } else if subtree == Some(true) {
        return err("subtree requires page");
    }
    match a.get("limit") {
        None | Some(Value::Null) => {}
        Some(limit) => match limit.as_u64() {
            Some(limit) if (1..=200).contains(&limit) => q.push(format!("limit:{limit}")),
            _ => return err("limit must be an integer between 1 and 200"),
        },
    }
    paging(a, q)
}

/// The old overdue tool: unfinished items due before now (all-day items
/// before today).
pub(crate) fn overdue_query(_: &Value) -> Result<Value> {
    Ok(json!({ "query": "is:todo,task date:<now done:no" }))
}

/// query_database's arguments as a query.
pub(crate) fn database_query(a: &Value) -> Result<Value> {
    let database = a
        .get("database")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError("missing argument: database".into()))?;
    let mut q = vec![
        format!("in:{}", page_ref(database)),
        "-type:database".to_string(),
    ];
    let filters = a
        .get("filter")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if filters.len() > 1 && a.get("match").and_then(Value::as_str) == Some("any") {
        return err("match any is not in the query language; run one query per filter");
    }
    for f in &filters {
        let prop = f
            .get("property")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("filter needs property".into()))?;
        let op = f
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("filter needs op".into()))?;
        let value = match f.get("value") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        };
        let key = quote_key(prop);
        let v = quote(&value);
        q.push(match op {
            "is" | "contains" => format!("{key}:{v}"),
            "is_not" | "not_contains" => format!("-{key}:{v}"),
            "is_empty" => format!("{key}:none"),
            "is_not_empty" => format!("has:{}", quote(prop)),
            ">" | "after" => format!("{key}:>{value}"),
            ">=" | "on_or_after" => format!("{key}:>={value}"),
            "<" | "before" => format!("{key}:<{value}"),
            "<=" | "on_or_before" => format!("{key}:<={value}"),
            "checked" => format!("{key}:yes"),
            "unchecked" => format!("{key}:no"),
            "starts_with" | "ends_with" => {
                return err(format!("{op} is not in the query language; use contains"))
            }
            other => return err(format!("unknown op {other:?}")),
        });
    }
    if let Some(sort) = a
        .get("sort")
        .and_then(Value::as_array)
        .and_then(|s| s.first())
    {
        let prop = sort
            .get("property")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("sort needs property".into()))?;
        let desc = sort
            .get("descending")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        q.push(format!(
            "sort:{}{}",
            quote(prop),
            if desc { ",desc" } else { ",asc" }
        ));
    }
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200);
    q.push(format!("limit:{limit}"));
    paging(a, q)
}

fn quote_key(prop: &str) -> String {
    format!("\"{}\"", prop.replace('\\', "\\\\").replace('"', "\\\""))
}

/// memory_search's arguments as a query over the memory pages.
pub(crate) fn memory_query(a: &Value) -> Result<Value> {
    let text = a.get("query").and_then(Value::as_str).unwrap_or("").trim();
    let mut out = json!({ "query": if text.is_empty() { "in:memory -type:database".to_string() } else { format!("in:memory -type:database {text}") } });
    if let Some(scope) = a.get("scope").and_then(Value::as_str) {
        out["__memory_scope"] = json!(scope);
    }
    Ok(out)
}
