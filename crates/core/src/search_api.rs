//! Vault-facing search parsing and completion. Metadata is cached independently
//! of typed text; SQLite's guards observe foreground and writer changes.

use std::collections::BTreeMap;

use crate::error::{invalid, Result};
use crate::index::Index;
use crate::search_query::{
    is_task_status_catalog, ParsedQuery, SearchExtras, SearchPage, SearchSpec,
    TASK_STATUS_CATALOG_CONFIG,
};
use crate::types::{
    PropDef, PropKind, PropValue, SearchCompletion, SearchSpecResults, SelectOption,
};
use crate::vault::{lock, Vault};

#[derive(Default)]
pub(crate) struct SearchMetadata {
    guard: Option<(i64, u64)>,
    properties: Vec<PropDef>,
    pages: Vec<SearchPage>,
    /// The top-level `Memory` page that `in:memory` names.
    memory_root: Option<String>,
}

fn normalized(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

impl SearchMetadata {
    fn refresh(&mut self, db: &Index) -> Result<()> {
        let version: i64 = db
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?;
        let guard = (version, db.conn.total_changes());
        if self.guard == Some(guard) {
            return Ok(());
        }
        let mut properties = BTreeMap::<String, PropDef>::new();
        let schemas = db
            .conn
            .prepare_cached("SELECT schema FROM pages WHERE schema IS NOT NULL ORDER BY id")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for schema in schemas {
            if let Ok(defs) = serde_json::from_str::<Vec<PropDef>>(&schema) {
                for def in defs {
                    let entry = properties
                        .entry(def.key.clone())
                        .or_insert_with(|| def.clone());
                    for option in def.options {
                        if !entry.options.iter().any(|o| o.id == option.id) {
                            entry.options.push(option);
                        }
                    }
                }
            }
        }
        let mut genuine_status = properties.contains_key("status");
        let values = db
            .conn
            .prepare_cached("SELECT DISTINCT x.key, x.json, p.page_type = 'task'
                FROM props x JOIN pages p ON p.rowid = x.page ORDER BY x.key, p.page_type = 'task'")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, json, task) in values {
            genuine_status |= key == "status" && !task;
            let Ok(value) = serde_json::from_str::<PropValue>(&json) else {
                continue;
            };
            let Some(kind) = value.kind() else {
                continue;
            };
            let def = properties.entry(key.clone()).or_insert_with(|| PropDef {
                key: key.clone(),
                name: key,
                kind,
                options: Vec::new(),
                config: String::new(),
            });
            let ids = match value {
                PropValue::Select(id) | PropValue::Status(id) => vec![id],
                PropValue::MultiSelect(ids) => ids,
                _ => Vec::new(),
            };
            for id in ids {
                if !def.options.iter().any(|o| o.id == id) {
                    def.options.push(SelectOption {
                        name: id.clone(),
                        id,
                        color: String::new(),
                    });
                }
            }
        }
        // Task status is a stable searchable field even before the vault has
        // any task pages. Preserve existing database/property kinds and their
        // option IDs when they share this key.
        let status = properties.entry("status".into()).or_insert_with(|| PropDef {
            key: "status".into(),
            name: "status".into(),
            kind: PropKind::Status,
            options: Vec::new(),
            config: String::new(),
        });
        if !genuine_status {
            status.config = TASK_STATUS_CATALOG_CONFIG.into();
        }
        for value in ["todo", "doing", "done"] {
            if !status.options.iter().any(|option| option.id == value) {
                status.options.push(SelectOption {
                    id: value.into(), name: value.into(), color: String::new(),
                });
            }
        }
        self.properties = properties.into_values().collect();
        self.pages = db.conn.prepare_cached(
            "SELECT id,title,updated_ms FROM pages p WHERE p.in_trash = 0 AND p.is_template = 0 ORDER BY p.updated_ms DESC,p.id"
        )?.query_map([], |r| Ok(SearchPage { id: r.get(0)?, title: r.get(1)?, updated_ms: r.get(2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        self.memory_root = db
            .conn
            .prepare_cached(
                "SELECT id FROM pages WHERE parent IS NULL AND title = 'Memory' AND in_trash = 0 ORDER BY id LIMIT 1",
            )?
            .query_map([], |r| r.get::<_, String>(0))?
            .next()
            .transpose()?;
        self.guard = Some(guard);
        Ok(())
    }
}

impl Vault {
    /// Parse text into an executable spec and byte-spanned tokens for chips.
    pub fn parse_search(&self, query: String) -> Result<ParsedQuery> {
        // Ordinary text needs no schema/title scan, even on its first keystroke.
        if !query.contains(':') && !query.contains('#') {
            return Ok(crate::search_query::parse_search(&query));
        }
        // Calendar names are read only for queries that name one.
        let calendars = if query.to_ascii_lowercase().contains("calendar") {
            self.calendar_names()?
        } else {
            Vec::new()
        };
        let db = lock(&self.inner.db);
        let mut metadata = lock(&self.inner.search_metadata);
        metadata.refresh(&db)?;
        let extras = SearchExtras {
            memory_root: metadata.memory_root.clone(),
            calendars,
        };
        Ok(crate::search_query::parse_search_with_metadata(
            &query,
            &metadata.properties,
            &metadata.pages,
            crate::ids::now_ms(),
            &extras,
        ))
    }

    /// Execute a stored spec; resolved page ids survive title changes.
    pub fn search_spec(
        &self,
        spec: SearchSpec,
        limit: u32,
        offset: u32,
    ) -> Result<SearchSpecResults> {
        lock(&self.inner.db).search_spec(&spec, limit, offset, false)
    }

    /// Complete keys and operator values at a byte cursor. Suggestions replace
    /// [start, end), and page suggestions also carry their stable page id.
    pub fn complete_search(&self, query: String, cursor: u32) -> Result<Vec<SearchCompletion>> {
        let cursor = cursor as usize;
        if cursor > query.len() || !query.is_char_boundary(cursor) {
            return Err(invalid(
                "search completion cursor must be a UTF-8 byte boundary",
            ));
        }
        let parsed = self.parse_search(query.clone())?;
        let token = parsed
            .tokens
            .iter()
            .find(|t| t.start as usize <= cursor && cursor <= t.end as usize);
        let start = token.map_or(cursor, |t| t.start as usize);
        let end = token.map_or(cursor, |t| t.end as usize);
        let typed = &query[start..cursor];
        let unsigned = typed.strip_prefix('-').unwrap_or(typed);
        let value_offset = operator_colon(unsigned)
            .map(|colon| colon + 1)
            .or_else(|| unsigned.starts_with('#').then_some(1));
        let replacement_end = value_offset.map_or(end, |offset| {
            let value_start = start + usize::from(typed.starts_with('-')) + offset;
            commas(&query[value_start..end])
                .into_iter()
                .map(|i| value_start + i)
                .find(|i| *i >= cursor)
                .unwrap_or(end)
        });
        let calendars = if typed.to_ascii_lowercase().contains("calendar") {
            self.calendar_names()?
        } else {
            Vec::new()
        };
        let db = lock(&self.inner.db);
        let mut metadata = lock(&self.inner.search_metadata);
        metadata.refresh(&db)?;
        let mut out = Vec::new();
        let mut push =
            |value: String, label: String, replacement_start: usize, id: Option<String>| {
                out.push(SearchCompletion {
                    value,
                    label,
                    start: replacement_start as u32,
                    end: replacement_end as u32,
                    resolved_id: id,
                });
            };
        let typed = typed.strip_prefix('-').unwrap_or(typed);
        if let Some(tag) = typed.strip_prefix('#') {
            let segment = last_comma(tag).map_or(0, |i| i + 1);
            let prefix = unquoted_prefix(&tag[segment..]).to_lowercase();
            for def in metadata
                .properties
                .iter()
                .filter(|d| normalized(&d.key) == "tags" || normalized(&d.name) == "tags")
            {
                for option in &def.options {
                    if option.name.to_lowercase().starts_with(&prefix) {
                        push(
                            quoted(&option.name),
                            option.name.clone(),
                            cursor - tag.len() + segment,
                            None,
                        );
                    }
                }
            }
        } else if let Some(colon) = operator_colon(typed) {
            let key = unquoted_prefix(&typed[..colon]);
            let raw_value = &typed[colon + 1..];
            let segment = last_comma(raw_value).map_or(0, |i| i + 1);
            let prefix = unquoted_prefix(&raw_value[segment..])
                .trim_start_matches("[[")
                .to_lowercase();
            let value_start = cursor - raw_value.len() + segment;
            let def = metadata
                .properties
                .iter()
                .find(|d| normalized(&d.key) == normalized(&key))
                .or_else(|| {
                    metadata
                        .properties
                        .iter()
                        .find(|d| normalized(&d.name) == normalized(&key))
                });
            let builtin = !typed.starts_with('"');
            let normalized_key = normalized(&key);
            let builtin_key = builtin
                && crate::search_ops::keyed_operator(&normalized_key).is_some();
            let fixed = crate::search_ops::keyed_operator(&normalized_key)
                .filter(|_| builtin)
                .map(|op| op.completions)
                .unwrap_or(&[]);
            for value in fixed {
                if value.starts_with(&prefix) {
                    push((*value).into(), (*value).into(), value_start, None);
                }
            }
            if builtin && normalized_key == "calendar" {
                for (_, name) in &calendars {
                    if name.to_lowercase().starts_with(&prefix) {
                        push(quoted(name), name.clone(), value_start, None);
                    }
                }
            }
            if builtin && matches!(normalized_key.as_str(), "in" | "links" | "from") {
                for page in &metadata.pages {
                    if page.title.to_lowercase().starts_with(&prefix) {
                        push(
                            format!("[[{}]]", page.id),
                            page.title.clone(),
                            value_start,
                            Some(page.id.clone()),
                        );
                    }
                }
            } else if (builtin && matches!(normalized_key.as_str(), "created" | "updated" | "date"))
                || (!builtin_key
                    && def.is_some_and(|d| {
                        matches!(
                            d.kind,
                            PropKind::Date | PropKind::CreatedTime | PropKind::UpdatedTime
                        )
                    }))
            {
                for word in [
                    "today",
                    "yesterday",
                    "tomorrow",
                    "week",
                    "lastweek",
                    "month",
                    "lastmonth",
                    "year",
                    "7d",
                    "+7d",
                ] {
                    if word.starts_with(&prefix) {
                        push(word.into(), word.into(), value_start, None);
                    }
                }
            } else if builtin && normalized_key == "sort" {
                for key in ["updated", "created", "title"] {
                    if key.starts_with(&prefix) {
                        push(key.into(), key.into(), value_start, None);
                    }
                }
                for def in &metadata.properties {
                    if def.name.to_lowercase().starts_with(&prefix) {
                        push(quoted(&def.name), def.name.clone(), value_start, None);
                    }
                }
            } else if builtin && normalized_key == "type" {
                for kind in [crate::types::PAGE_TYPE_DOCUMENT, crate::types::PAGE_TYPE_DATABASE, "chat"] {
                    if kind.starts_with(&prefix) {
                        push(kind.into(), kind.into(), value_start, None);
                    }
                }
            } else if builtin && normalized_key == "has" {
                for key in ["link", "attachment"] {
                    if key.starts_with(&prefix) {
                        push(key.into(), key.into(), value_start, None);
                    }
                }
                for def in &metadata.properties {
                    if def.name.to_lowercase().starts_with(&prefix) {
                        push(quoted(&def.name), def.name.clone(), value_start, None);
                    }
                }
            } else {
                let definitions: Vec<_> = if builtin && normalized_key == "tag" {
                    metadata
                        .properties
                        .iter()
                        .filter(|d| normalized(&d.key) == "tags" || normalized(&d.name) == "tags")
                        .collect()
                } else if builtin_key {
                    Vec::new()
                } else if builtin && normalized_key == "status"
                    && def.is_some_and(is_task_status_catalog) {
                    metadata.properties.iter()
                        .filter(|d| normalized(&d.key) == "status" || normalized(&d.name) == "status")
                        .collect()
                } else {
                    def.into_iter().collect()
                };
                for def in definitions {
                    for option in &def.options {
                        if option.name.to_lowercase().starts_with(&prefix) {
                            push(quoted(&option.name), option.name.clone(), value_start, None);
                        }
                    }
                    if def.kind == PropKind::Checkbox {
                        for word in ["yes", "no", "true", "false"] {
                            if word.starts_with(&prefix) {
                                push(word.into(), word.into(), value_start, None);
                            }
                        }
                    }
                }
            }
        } else {
            let prefix = normalized(&unquoted_prefix(typed));
            let key_start = if query[start..cursor].starts_with('-') {
                start + 1
            } else {
                start
            };
            for key in crate::search_ops::keyed_names() {
                if key.starts_with(&prefix) {
                    push(format!("{key}:"), key.into(), key_start, None);
                }
            }
            for def in &metadata.properties {
                if normalized(&def.name).starts_with(&prefix)
                    || normalized(&def.key).starts_with(&prefix)
                {
                    // Quoting also reaches properties named after built-ins.
                    push(
                        format!("\"{}\":", escaped(&def.name)),
                        def.name.clone(),
                        key_start,
                        None,
                    );
                }
            }
        }
        out.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then(a.value.cmp(&b.value))
        });
        out.dedup_by(|a, b| a.value == b.value);
        out.truncate(50);
        Ok(out)
    }
}

fn escaped(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
fn unquoted_prefix(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"') else {
        return s.into();
    };
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(c) = chars.next() {
                    out.push(c);
                }
            }
            '"' => break,
            _ => out.push(c),
        }
    }
    out
}
fn quoted(s: &str) -> String {
    if s.chars()
        .any(|c| c.is_whitespace() || matches!(c, ',' | '"' | '\\' | ':'))
    {
        format!("\"{}\"", escaped(s))
    } else {
        s.into()
    }
}
fn operator_colon(s: &str) -> Option<usize> {
    let mut quote = false;
    let mut escape = false;
    for (i, c) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
        } else if c == '"' {
            quote = !quote;
        } else if c == ':' && !quote {
            return Some(i);
        }
    }
    None
}
fn last_comma(s: &str) -> Option<usize> {
    commas(s).last().copied()
}
fn commas(s: &str) -> Vec<usize> {
    let mut quote = false;
    let mut escape = false;
    let mut result = Vec::new();
    let mut brackets = 0usize;
    for (i, c) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
        } else if c == '"' {
            quote = !quote;
        } else if !quote {
            if c == '[' {
                brackets += 1;
            } else if c == ']' {
                brackets = brackets.saturating_sub(1);
            } else if c == ',' && brackets == 0 {
                result.push(i);
            }
        }
    }
    result
}
