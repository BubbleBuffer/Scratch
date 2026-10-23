//! Candidate compilation shared by every search path. Text is deliberately
//! absent from the cache key, except for hard phrases and exclusions.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{Local, TimeZone, Utc};
use rusqlite::{params, types::Value, OptionalExtension};

use crate::error::{Result, VaultError};
use crate::index::{fold, Index, NOT_FEEDBACK, NOT_TEMPLATE};
use crate::search_query::{SearchOperator, SearchSpec};
use crate::types::{Filter, MatchedFilterValue, PropDef, PropKind, PropValue, SearchHit};

pub(crate) const SMALL_CANDIDATES: usize = 5_000;

#[derive(Default)]
pub(crate) struct Cache {
    guard: Option<(i64, u64)>,
    sets: HashMap<String, Arc<Candidates>>,
    // Keep the full definition order; the parser's merged catalog differs.
    schemas: Option<Arc<Vec<PropDef>>>,
    tree_epoch: Option<i64>,
    subtrees: HashMap<String, String>,
}

impl Cache {
    fn refresh(&mut self, db: &Index) -> Result<()> {
        let version: i64 = db
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?;
        let guard = (version, db.conn.total_changes());
        if self.guard != Some(guard) {
            self.sets.clear();
            self.schemas = None;
            self.guard = Some(guard);
        }
        Ok(())
    }
}

/// A caller-owned transaction may roll back without changing total_changes.
/// Its eligibility and structural values must never enter the durable cache.
pub(crate) struct IsolatedCache<'a> {
    cell: &'a RefCell<Cache>,
    previous: Option<Cache>,
}

impl<'a> IsolatedCache<'a> {
    pub fn new(cell: &'a RefCell<Cache>) -> Self {
        Self {
            cell,
            previous: Some(cell.replace(Cache::default())),
        }
    }
}

impl Drop for IsolatedCache<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            self.cell.replace(previous);
        }
    }
}

pub(crate) struct Candidates {
    pub rows: HashSet<i64>,
    pub ids: HashSet<String>,
    pub array: Vec<Value>,
    pub warnings: Vec<String>,
}

impl Candidates {
    pub fn small(&self) -> bool {
        self.rows.len() <= SMALL_CANDIDATES
    }
    pub fn clause(&self, column: &str, parameter: usize) -> String {
        if self.small() {
            // FTS5 does not consume IN as one rowid set. Without unary +,
            // SQLite executes MATCH once per candidate and rebuilds long
            // prefix posting unions each time. Keep the indexed-text scan
            // first, then apply the same rarray constraint before ranking.
            let column = if column == "fts.rowid" {
                "+fts.rowid"
            } else {
                column
            };
            format!(" AND {column} IN rarray(?{parameter})")
        } else {
            format!(" AND search_candidate({column})")
        }
    }
}

/// Unlike the eventual feed generation, this clock changes only when the
/// indexed parent graph changes. Writer content commits leave subtrees warm.
pub(crate) fn initialize(conn: &rusqlite::Connection) -> Result<()> {
    rusqlite::vtab::array::load_module(conn)?;
    conn.create_scalar_function(
        "search_fold",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|value| fold(&value))),
    )?;
    let flags = rusqlite::functions::FunctionFlags::SQLITE_UTF8
        | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC;
    conn.create_scalar_function("search_probe_tag_options", 2, flags, |ctx| {
        let defs = probe_schemas(&ctx.get::<String>(0)?);
        let mut pairs = Vec::new();
        for tag in alternatives(&ctx.get::<String>(1)?) {
            for (key, ids) in tag_options(&fold(&tag), &defs) {
                pairs.extend(ids.into_iter().map(|id| (key.clone(), id)));
            }
        }
        serde_json::to_string(&pairs)
            .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))
    })?;
    conn.create_scalar_function("search_probe_timestamp", 2, flags, |ctx| {
        let defs = probe_schemas(&ctx.get::<String>(0)?);
        let key = ctx.get::<String>(1)?;
        Ok(defs.iter().any(|d| {
            d.key == key && matches!(d.kind, PropKind::CreatedTime | PropKind::UpdatedTime)
        }))
    })?;
    conn.create_scalar_function("search_date_start", 2, flags, |ctx| {
        Ok(calendar_boundary(
            ctx.get::<Option<i64>>(0)?,
            ctx.get::<bool>(1)?,
            false,
            &Local,
        ))
    })?;
    conn.create_scalar_function("search_date_end", 2, flags, |ctx| {
        Ok(calendar_boundary(
            ctx.get::<Option<i64>>(0)?,
            ctx.get::<bool>(1)?,
            true,
            &Local,
        ))
    })?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS pages_created ON pages(in_trash, created_ms DESC, id DESC);
         CREATE TABLE IF NOT EXISTS search_tree_epoch(value INTEGER NOT NULL);
         INSERT INTO search_tree_epoch(value) SELECT 0 WHERE NOT EXISTS(SELECT 1 FROM search_tree_epoch);
         CREATE TRIGGER IF NOT EXISTS search_tree_insert AFTER INSERT ON pages BEGIN
           UPDATE search_tree_epoch SET value = value + 1;
         END;
         CREATE TRIGGER IF NOT EXISTS search_tree_delete AFTER DELETE ON pages BEGIN
           UPDATE search_tree_epoch SET value = value + 1;
         END;
         CREATE TRIGGER IF NOT EXISTS search_tree_parent AFTER UPDATE OF parent ON pages
         WHEN old.parent IS NOT new.parent BEGIN
           UPDATE search_tree_epoch SET value = value + 1;
         END;"
    )?;
    initialize_multi_values(conn)?;
    initialize_search_rows(conn)?;
    Ok(())
}

/// The search projection: one row per searchable item, keyed by the FTS
/// rowid (`pages.rowid`, or `-search_external.rowid` for external rows). It
/// is a real table, so joining it to FTS is a rowid lookup; a UNION ALL view
/// was re-materialized per query and made warm searches several times slower.
/// Triggers keep it in step with `pages` and `search_external`. Like
/// `search_multi`, its presence is the completion marker of an additive
/// migration: creation, backfill and triggers commit together.
fn initialize_search_rows(conn: &rusqlite::Connection) -> Result<()> {
    let exists = |conn: &rusqlite::Connection| -> rusqlite::Result<bool> {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'search_rows')", [], |r| r.get(0))
    };
    if exists(conn)? {
        return Ok(());
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    if !exists(&tx)? {
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS search_rows_insert;
             DROP TRIGGER IF EXISTS search_rows_update;
             DROP TRIGGER IF EXISTS search_rows_delete;
             DROP TRIGGER IF EXISTS search_rows_external_insert;
             DROP TRIGGER IF EXISTS search_rows_external_update;
             DROP TRIGGER IF EXISTS search_rows_external_delete;
             CREATE TABLE search_rows(
               rowid INTEGER PRIMARY KEY,
               id TEXT NOT NULL UNIQUE,
               title TEXT NOT NULL,
               title_fold TEXT NOT NULL,
               parent TEXT,
               template TEXT,
               created_ms INTEGER NOT NULL,
               updated_ms INTEGER NOT NULL,
               in_trash INTEGER NOT NULL,
               is_template INTEGER NOT NULL,
               page_type TEXT NOT NULL,
               schema TEXT,
               source TEXT NOT NULL
             );
             CREATE INDEX search_rows_updated ON search_rows(in_trash, updated_ms DESC, id DESC);
             CREATE INDEX search_rows_created ON search_rows(in_trash, created_ms DESC, id DESC);
             INSERT INTO search_rows(rowid, id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema, source)
               SELECT rowid, id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema, 'vault'
               FROM pages;
             INSERT INTO search_rows(rowid, id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema, source)
               SELECT -rowid, source || char(31) || id, title, search_fold(title), NULL, NULL, updated_ms, updated_ms, 0, 0, page_type, NULL, source
               FROM search_external;
             CREATE TRIGGER search_rows_insert AFTER INSERT ON pages BEGIN
               INSERT INTO search_rows(rowid, id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema, source)
               VALUES (new.rowid, new.id, new.title, new.title_fold, new.parent, new.template, new.created_ms, new.updated_ms, new.in_trash, new.is_template, new.page_type, new.schema, 'vault');
             END;
             CREATE TRIGGER search_rows_update AFTER UPDATE OF id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema ON pages BEGIN
               UPDATE search_rows SET id = new.id, title = new.title, title_fold = new.title_fold, parent = new.parent,
                 template = new.template, created_ms = new.created_ms, updated_ms = new.updated_ms, in_trash = new.in_trash,
                 is_template = new.is_template, page_type = new.page_type, schema = new.schema
               WHERE rowid = new.rowid;
             END;
             CREATE TRIGGER search_rows_delete AFTER DELETE ON pages BEGIN
               DELETE FROM search_rows WHERE rowid = old.rowid;
             END;
             CREATE TRIGGER search_rows_external_insert AFTER INSERT ON search_external BEGIN
               INSERT INTO search_rows(rowid, id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema, source)
               VALUES (-new.rowid, new.source || char(31) || new.id, new.title, search_fold(new.title), NULL, NULL, new.updated_ms, new.updated_ms, 0, 0, new.page_type, NULL, new.source);
             END;
             CREATE TRIGGER search_rows_external_update AFTER UPDATE ON search_external BEGIN
               DELETE FROM search_rows WHERE rowid = -old.rowid;
               INSERT INTO search_rows(rowid, id, title, title_fold, parent, template, created_ms, updated_ms, in_trash, is_template, page_type, schema, source)
               VALUES (-new.rowid, new.source || char(31) || new.id, new.title, search_fold(new.title), NULL, NULL, new.updated_ms, new.updated_ms, 0, 0, new.page_type, NULL, new.source);
             END;
             CREATE TRIGGER search_rows_external_delete AFTER DELETE ON search_external BEGIN
               DELETE FROM search_rows WHERE rowid = -old.rowid;
             END;",
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// An additive, versionless index migration. Its presence is the completion
/// marker: creation, backfill and maintenance triggers commit together.
fn initialize_multi_values(conn: &rusqlite::Connection) -> Result<()> {
    let exists = |conn: &rusqlite::Connection| -> rusqlite::Result<bool> {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'search_multi')", [], |r| r.get(0))
    };
    if exists(conn)? {
        return Ok(());
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    // The foreground and writer may open simultaneously. The write lock
    // serializes migration, then this recheck observes the first commit.
    if !exists(&tx)? {
        tx.execute_batch(
            "CREATE TABLE search_multi(key TEXT NOT NULL, value TEXT NOT NULL, page INTEGER NOT NULL,
               PRIMARY KEY(key, value, page)) WITHOUT ROWID;
             CREATE INDEX search_multi_page ON search_multi(page, key);
             INSERT OR IGNORE INTO search_multi(key, value, page)
               SELECT p.key, search_fold(v.value), p.page
               FROM props p, json_each(p.json, '$.MultiSelect') v;
             CREATE TRIGGER IF NOT EXISTS search_multi_insert AFTER INSERT ON props BEGIN
               INSERT OR IGNORE INTO search_multi(key, value, page)
                 SELECT new.key, search_fold(v.value), new.page
                 FROM json_each(new.json, '$.MultiSelect') v;
             END;
             CREATE TRIGGER IF NOT EXISTS search_multi_update AFTER UPDATE OF page, key, json ON props BEGIN
               DELETE FROM search_multi WHERE page = old.page AND key = old.key;
               INSERT OR IGNORE INTO search_multi(key, value, page)
                 SELECT new.key, search_fold(v.value), new.page
                 FROM json_each(new.json, '$.MultiSelect') v;
             END;
             CREATE TRIGGER IF NOT EXISTS search_multi_delete AFTER DELETE ON props BEGIN
               DELETE FROM search_multi WHERE page = old.page AND key = old.key;
             END;
             CREATE TRIGGER IF NOT EXISTS search_multi_page_delete AFTER DELETE ON pages BEGIN
               DELETE FROM search_multi WHERE page = old.rowid;
             END;"
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// All-day values encode a calendar date at UTC midnight, independent of
/// the current zone. Search compares that calendar day's local interval;
/// timed values already encode an instant and pass through unchanged.
fn calendar_boundary<Z: TimeZone>(
    value: Option<i64>,
    has_time: bool,
    end: bool,
    tz: &Z,
) -> Option<i64> {
    let value = value?;
    if has_time {
        return Some(value);
    }
    let mut day = chrono::DateTime::<Utc>::from_timestamp_millis(value)?.date_naive();
    if end {
        day = day.succ_opt()?;
    }
    let value = [0, 1, 2]
        .into_iter()
        .find_map(|hour| {
            day.and_hms_opt(hour, 0, 0)
                .and_then(|at| tz.from_local_datetime(&at).earliest())
        })?
        .timestamp_millis();
    Some(if end { value.checked_sub(1)? } else { value })
}

pub(crate) fn alternatives(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut part = String::new();
    let mut quote = false;
    let mut escaped = false;
    let mut brackets = 0u32;
    for c in value.chars() {
        if escaped {
            part.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote {
            escaped = true;
            continue;
        }
        if c == '"' {
            quote = !quote;
            continue;
        }
        if !quote {
            if c == '[' {
                brackets += 1;
            }
            if c == ']' {
                brackets = brackets.saturating_sub(1);
            }
            if c == ',' && brackets == 0 {
                if !part.trim().is_empty() {
                    out.push(part.trim().to_string());
                }
                part.clear();
                continue;
            }
        }
        part.push(c);
    }
    if !part.trim().is_empty() {
        out.push(part.trim().to_string());
    }
    out
}

fn bind(p: &mut Vec<Value>, value: impl Into<Value>) -> String {
    p.push(value.into());
    format!("?{}", p.len())
}

fn schemas(db: &Index) -> Result<Arc<Vec<PropDef>>> {
    let mut cache = db.search_filter_cache.borrow_mut();
    cache.refresh(db)?;
    cached_schemas(db, &mut cache)
}

fn cached_schemas(db: &Index, cache: &mut Cache) -> Result<Arc<Vec<PropDef>>> {
    if let Some(defs) = &cache.schemas {
        return Ok(Arc::clone(defs));
    }
    let values = db
        .conn
        .prepare_cached("SELECT schema FROM pages WHERE schema IS NOT NULL")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut defs = Vec::new();
    for value in values {
        defs.extend(serde_json::from_str::<Vec<PropDef>>(&value).unwrap_or_default());
    }
    let defs = Arc::new(defs);
    cache.schemas = Some(Arc::clone(&defs));
    Ok(defs)
}

fn normalized_key(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn probe_schemas(encoded: &str) -> Vec<PropDef> {
    serde_json::from_str::<Vec<String>>(encoded)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|schema| serde_json::from_str::<Vec<PropDef>>(&schema).unwrap_or_default())
        .collect()
}

fn tag_options(tag: &str, defs: &[PropDef]) -> Vec<(String, Vec<String>)> {
    defs.iter()
        .filter(|d| {
            d.kind == PropKind::MultiSelect
                && (normalized_key(&d.name) == "tags" || normalized_key(&d.key) == "tags")
        })
        .map(|d| {
            let ids = d
                .options
                .iter()
                .filter_map(|option| {
                    let name = fold(&option.name);
                    (name == tag || name.starts_with(&format!("{tag}/")) || fold(&option.id) == tag)
                        .then(|| fold(&option.id))
                })
                .collect();
            (d.key.clone(), ids)
        })
        .collect()
}

fn subtree(db: &Index, cache: &mut Cache, id: &str) -> Result<String> {
    let epoch: i64 = db
        .conn
        .query_row("SELECT value FROM search_tree_epoch", [], |r| r.get(0))?;
    if cache.tree_epoch != Some(epoch) {
        cache.subtrees.clear();
        cache.tree_epoch = Some(epoch);
    }
    if let Some(value) = cache.subtrees.get(id) {
        return Ok(value.clone());
    }
    let rows = db
        .conn
        .prepare_cached(
            "WITH RECURSIVE tree(id) AS (SELECT id FROM pages WHERE id = ?1
         UNION SELECT p.id FROM pages p JOIN tree t ON p.parent = t.id)
         SELECT p.rowid FROM pages p JOIN tree t ON p.id = t.id",
        )?
        .query_map([id], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let value = serde_json::to_string(&rows).map_err(|e| VaultError::Index(e.to_string()))?;
    if cache.subtrees.len() >= 32 {
        cache.subtrees.clear();
    }
    cache.subtrees.insert(id.to_string(), value.clone());
    Ok(value)
}

fn page_alternatives(op: &SearchOperator) -> Vec<String> {
    let values = alternatives(&op.value);
    if let Some(ids) = &op.resolved_id {
        if ids.contains(',') && !values.iter().all(|value| value.starts_with("[[")) {
            return alternatives(ids);
        }
    }
    if values.len() <= 1 {
        if let Some(id) = &op.resolved_id {
            return vec![id.clone()];
        }
    }
    values
        .into_iter()
        .map(|v| {
            v.strip_prefix("[[")
                .and_then(|v| v.strip_suffix("]]"))
                .unwrap_or(&v)
                .to_string()
        })
        .collect()
}

/// Page types compare case-insensitively. External rows carry their own
/// type (`chat`); a vault page without a stored type is a `document`.
fn type_sql(op: &SearchOperator, p: &mut Vec<Value>) -> String {
    let choices: Vec<String> = alternatives(&op.value)
        .into_iter()
        .map(|t| bind(p, fold(&t)))
        .collect();
    if choices.is_empty() {
        "0".into()
    } else {
        format!("search_fold(p.page_type) IN ({})", choices.join(", "))
    }
}

fn tag_sql(op: &SearchOperator, defs: &[PropDef], p: &mut Vec<Value>) -> String {
    let mut choices = Vec::new();
    for tag in alternatives(&op.value) {
        let tag = fold(&tag);
        let exact = bind(p, tag.clone());
        choices.push(format!(
            "SELECT page FROM search_multi WHERE key = 'tags' AND value = {exact}"
        ));
        // The slash's next binary character is 0. This interval includes
        // exactly descendants, and treats percent/underscore literally.
        let lower = bind(p, format!("{tag}/"));
        let upper = bind(p, format!("{tag}0"));
        choices.push(format!("SELECT page FROM search_multi WHERE key = 'tags' AND value >= {lower} AND value < {upper}"));
        for (key, ids) in tag_options(&tag, defs) {
            let matches: Vec<_> = ids.into_iter().map(|id| bind(p, id)).collect();
            if !matches.is_empty() {
                let key = bind(p, key);
                choices.push(format!(
                    "SELECT page FROM search_multi WHERE key = {key} AND value IN ({})",
                    matches.join(", ")
                ));
            }
        }
    }
    if choices.is_empty() {
        "0".into()
    } else {
        format!("p.rowid IN ({})", choices.join(" UNION "))
    }
}

fn operator_sql(
    db: &Index,
    cache: &mut Cache,
    op: &SearchOperator,
    defs: &[PropDef],
    p: &mut Vec<Value>,
    inline: bool,
) -> Result<Option<String>> {
    if let Some(filter) = &op.filter {
        return crate::query::filter_sql(filter, p).map(Some);
    }
    if op.property {
        return Ok(None);
    }
    let sql = match op.key.as_str() {
        "tag" if inline => {
            let literal = tag_sql(op, &[], p);
            let value = bind(p, op.value.clone());
            format!("({literal} OR EXISTS(SELECT 1 FROM search_multi m
                JOIN json_each(search_probe_tag_options((SELECT value FROM probe_schemas), {value})) option
                  ON m.key = json_extract(option.value, '$[0]')
                 AND m.value = json_extract(option.value, '$[1]') WHERE m.page = p.rowid))")
        }
        "tag" => tag_sql(op, defs, p),
        "type" => type_sql(op, p),
        "in" => {
            let mut parts = Vec::new();
            // Path patterns (`in:Work/*`) resolve to their exact pages; the
            // other alternatives keep the subtree path below.
            let raws = crate::search_path::split_raw(&op.value, ',');
            let (patterns, plain): (Vec<_>, Vec<_>) = raws
                .iter()
                .partition(|raw| crate::search_path::is_pattern(raw));
            for raw in &patterns {
                let pattern = crate::search_path::parse(raw, |t| Some(t.to_string()), None)
                    .map_err(VaultError::Index)?;
                let ids = serde_json::to_string(&pattern.page_ids(db)?)
                    .map_err(|e| VaultError::Index(e.to_string()))?;
                let parameter = bind(p, ids);
                parts.push(format!(
                    "p.id IN (SELECT value FROM json_each({parameter}))"
                ));
            }
            let plain_ids = if patterns.is_empty() {
                page_alternatives(op)
            } else {
                plain
                    .iter()
                    .map(|raw| crate::search_query::strip_brackets(raw).to_string())
                    .collect()
            };
            for id in plain_ids {
                if inline {
                    let parameter = bind(p, id);
                    parts.push(format!(
                        "p.id IN (WITH RECURSIVE tree(id) AS (
                        SELECT id FROM pages WHERE id = {parameter}
                        UNION SELECT child.id FROM pages child JOIN tree ON child.parent = tree.id)
                        SELECT id FROM tree)"
                    ));
                    continue;
                }
                let tree = subtree(db, cache, &id)?;
                let parameter = bind(p, tree);
                parts.push(format!(
                    "p.rowid IN (SELECT value FROM json_each({parameter}))"
                ));
            }
            format!(
                "({})",
                if parts.is_empty() {
                    "0".into()
                } else {
                    parts.join(" OR ")
                }
            )
        }
        "links" | "from" => {
            let mut parts = Vec::new();
            for id in page_alternatives(op) {
                let parameter = bind(p, id);
                parts.push(if op.key == "links" {
                    format!(
                        "EXISTS(SELECT 1 FROM links l WHERE l.src = p.id AND l.dst = {parameter})"
                    )
                } else {
                    format!(
                        "EXISTS(SELECT 1 FROM links l WHERE l.dst = p.id AND l.src = {parameter})"
                    )
                });
            }
            format!(
                "({})",
                if parts.is_empty() {
                    "0".into()
                } else {
                    parts.join(" OR ")
                }
            )
        }
        "has" => {
            let mut parts = Vec::new();
            let mut values = alternatives(&op.value);
            if values.len() == 1 && !matches!(values[0].as_str(), "link" | "attachment") {
                if let Some(key) = &op.resolved_id {
                    values[0] = key.clone();
                }
            }
            for value in values {
                parts.push(match value.as_str() {
                    "link" => "EXISTS(SELECT 1 FROM links l WHERE l.src = p.id)".into(),
                    "attachment" => "EXISTS(SELECT 1 FROM refs r WHERE r.page = p.id)".into(),
                    key if defs.iter().any(|d| d.key == key && matches!(d.kind, PropKind::CreatedTime | PropKind::UpdatedTime)) => "1".into(),
                    key => {
                        let parameter = bind(p, key.to_string());
                        let stored = format!("EXISTS(SELECT 1 FROM props x WHERE x.page = p.rowid AND x.key = {parameter})");
                        if inline {
                            format!("({stored} OR search_probe_timestamp((SELECT value FROM probe_schemas), {parameter}))")
                        } else {
                            stored
                        }
                    }
                });
            }
            format!(
                "({})",
                if parts.is_empty() {
                    "0".into()
                } else {
                    parts.join(" OR ")
                }
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(sql))
}

/// The same hard constraints for ordinary searches and commit probes. Inline
/// mode resolves schema aliases and subtrees inside the probe's one statement.
fn eligibility_sql(
    db: &Index,
    cache: &mut Cache,
    spec: &SearchSpec,
    defs: &[PropDef],
    p: &mut Vec<Value>,
    inline: bool,
) -> Result<(String, Vec<String>)> {
    let mut clauses = vec![format!(
        "p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{}",
        db.scope_clause("p.rowid")
    )];
    let mut warnings = Vec::new();
    // Item operators run in the item search; size controls never filter.
    for op in spec
        .operators
        .iter()
        .filter(|op| !crate::search_query::is_item_or_shape(op))
    {
        let start = p.len();
        match operator_sql(db, cache, op, defs, p, inline) {
            Ok(Some(sql)) => clauses.push(if op.negated {
                format!("NOT ({sql})")
            } else {
                format!("({sql})")
            }),
            Ok(None) => {
                p.truncate(start);
                warnings.push(format!(
                    "Ignored unreadable operator {}:{}",
                    op.key, op.value
                ));
            }
            Err(VaultError::Invalid(message)) => {
                p.truncate(start);
                warnings.push(format!("Ignored {}:{}: {message}", op.key, op.value));
            }
            Err(error) => return Err(error),
        }
    }
    for term in spec.terms.iter().filter(|t| t.exclude || t.phrase) {
        let words = crate::fuzzy::query_words(&term.value);
        if words.is_empty() {
            continue;
        }
        let query = format!("\"{}\"", words.join(" "));
        let parameter = bind(p, query);
        let restriction = if inline {
            " AND +rowid IN (SELECT rowid FROM committed)"
        } else {
            ""
        };
        clauses.push(format!(
            "p.rowid {}IN (SELECT rowid FROM fts WHERE fts MATCH {parameter}{restriction})",
            if term.exclude { "NOT " } else { "" }
        ));
    }
    Ok((clauses.join(" AND "), warnings))
}

pub(crate) fn probe_predicate(db: &Index, spec: &SearchSpec, p: &mut Vec<Value>) -> Result<String> {
    eligibility_sql(db, &mut Cache::default(), spec, &[], p, true).map(|(sql, _)| sql)
}

pub(crate) fn candidates(db: &Index, spec: &SearchSpec) -> Result<Option<Arc<Candidates>>> {
    let hard_terms: Vec<_> = spec
        .terms
        .iter()
        .filter(|t| t.exclude || t.phrase)
        .collect();
    let filters: Vec<_> = spec
        .operators
        .iter()
        .filter(|op| !crate::search_query::is_item_or_shape(op))
        .collect();
    if filters.is_empty() && hard_terms.is_empty() {
        return Ok(None);
    }
    // Repeated keys are conjunctions; raw display strings and chip ordering
    // do not change eligibility. Relative date bounds remain exact values.
    let mut keys = Vec::new();
    for op in filters {
        let key = if let Some(filter) = &op.filter {
            serde_json::to_string(&(op.negated, crate::query::normalize_search_filter(filter)))
        } else {
            let mut values = if matches!(op.key.as_str(), "in" | "links" | "from") {
                page_alternatives(op)
            } else if op.key == "tag" {
                alternatives(&op.value)
                    .into_iter()
                    .map(|v| fold(&v))
                    .collect()
            } else {
                alternatives(&op.value)
            };
            values.sort();
            values.dedup();
            serde_json::to_string(&(op.key.as_str(), op.negated, op.property, values))
        }
        .map_err(|e| VaultError::Index(e.to_string()))?;
        keys.push(key);
    }
    keys.sort();
    let hard_keys: Vec<_> = hard_terms
        .iter()
        .map(|t| (t.exclude, crate::fuzzy::query_words(&t.value)))
        .collect();
    // Page-only and all-row candidate sets differ, so the scope is part of the key.
    let key = serde_json::to_string(&(keys, hard_keys, db.page_scope.get()))
        .map_err(|e| VaultError::Index(e.to_string()))?;
    let mut cache = db.search_filter_cache.borrow_mut();
    cache.refresh(db)?;
    if let Some(set) = cache.sets.get(&key) {
        return Ok(Some(set.clone()));
    }
    let defs = if spec
        .operators
        .iter()
        .any(|o| matches!(o.key.as_str(), "tag" | "has") && !o.property)
    {
        cached_schemas(db, &mut cache)?
    } else {
        Arc::new(Vec::new())
    };
    let mut p = Vec::new();
    let (predicate, warnings) = eligibility_sql(db, &mut cache, spec, &defs, &mut p, false)?;
    let sql = format!(
        "SELECT p.rowid, p.id FROM search_rows p WHERE {}",
        predicate
    );
    let pairs = db
        .conn
        .prepare_cached(&sql)?
        .query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let set = Arc::new(Candidates {
        rows: pairs.iter().map(|(row, _)| *row).collect(),
        ids: pairs.iter().map(|(_, id)| id.clone()).collect(),
        array: pairs.iter().map(|(row, _)| Value::Integer(*row)).collect(),
        warnings,
    });
    if cache.sets.len() >= 16 {
        cache.sets.clear();
    }
    cache.sets.insert(key, set.clone());
    Ok(Some(set))
}

fn date_fields(filter: &Filter, fields: &mut HashSet<String>) {
    match filter {
        Filter::Cond(c) if matches!(c.value, PropValue::Date { .. }) => {
            fields.insert(c.prop.clone());
        }
        Filter::And(v) | Filter::Or(v) => {
            for f in v {
                date_fields(f, fields);
            }
        }
        _ => {}
    }
}

pub(crate) fn order_sql(spec: &SearchSpec, p: &mut Vec<Value>) -> String {
    let mut fields = HashSet::new();
    let mut future = false;
    for op in spec.operators.iter().filter(|o| !o.negated) {
        if let Some(f) = &op.filter {
            date_fields(f, &mut fields);
            future |= op.date_future;
        }
    }
    let mut descending = true;
    let field = if let Some(sort) = &spec.sort {
        let (key, direction) = crate::search_query::sort_direction(sort);
        descending = direction.unwrap_or(matches!(
            key,
            "updated" | "created" | "$updated" | "$created"
        ));
        key.to_string()
    } else if fields.len() == 1 {
        descending = !future;
        fields.into_iter().next().unwrap()
    } else {
        "$updated".into()
    };
    let expr = match field.as_str() {
        "updated" | "$updated" => "p.updated_ms".to_string(),
        "created" | "$created" => "p.created_ms".to_string(),
        "title" | "$title" => "p.title_fold".to_string(),
        key => {
            let parameter = bind(p, key.to_string());
            format!("(SELECT coalesce(x.n, search_date_start(x.d0, coalesce(json_extract(x.json, '$.Date.has_time'), 1)), x.tf, x.t) FROM props x WHERE x.page = p.rowid AND x.key = {parameter})")
        }
    };
    let direction = if descending { "DESC" } else { "ASC" };
    match field.as_str() {
        // These columns are non-null. A redundant NULL expression prevents
        // SQLite from using the timestamp/title index for a bounded sort.
        "updated" | "$updated" => format!("{expr} {direction}, p.id"),
        "created" | "$created" | "title" | "$title" => {
            format!("{expr} {direction}, p.updated_ms DESC, p.id")
        }
        // Empty property values sort last in either direction.
        _ => format!("({expr} IS NULL), {expr} {direction}, p.updated_ms DESC, p.id"),
    }
}

fn display(value: &PropValue, defs: &[PropDef], key: &str) -> String {
    let name = |id: &str| {
        defs.iter()
            .filter(|d| d.key == key)
            .flat_map(|d| d.options.iter())
            .find(|o| o.id == id)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match value {
        PropValue::Select(id) | PropValue::Status(id) => name(id),
        PropValue::MultiSelect(ids) => ids.iter().map(|id| name(id)).collect::<Vec<_>>().join(", "),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => crate::markdown::date_text(*start_ms, *end_ms, *has_time),
        PropValue::Number(v) => crate::formula::format_num(*v),
        PropValue::Checkbox(v) => v.to_string(),
        PropValue::Text(v) | PropValue::Url(v) | PropValue::Error(v) => v.clone(),
        PropValue::Relation(v) | PropValue::Files(v) => v.join(", "),
        PropValue::Empty => String::new(),
    }
}

pub(crate) fn decorate(db: &Index, spec: &SearchSpec, hits: &mut [SearchHit]) -> Result<()> {
    if spec.operators.is_empty() || hits.is_empty() {
        return Ok(());
    }
    let defs = schemas(db)?;
    for hit in hits {
        // External rows (a source-scoped id) have no page, props or dates.
        let Some((rowid, created, updated)) = db
            .conn
            .query_row(
                "SELECT rowid, created_ms, updated_ms FROM pages WHERE id = ?1",
                [&hit.id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            continue;
        };
        let props = db
            .conn
            .prepare_cached("SELECT key, json FROM props WHERE page = ?1")?
            .query_map([rowid], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let props: HashMap<String, PropValue> = props
            .into_iter()
            .filter_map(|(k, v)| serde_json::from_str(&v).ok().map(|v| (k, v)))
            .collect();
        for op in &spec.operators {
            if op.negated {
                continue;
            }
            // A status alias can match a real database property with a UUID
            // key as well as the task projection. Resolve labels from the
            // actual matching branch, even when the row stores both keys.
            if op.property && op.key == "status" {
                if let Some(filter) = &op.filter {
                    if let Some(values) =
                        status_alias_values(db, filter, &props, &defs, rowid, (created, updated))?
                    {
                        for (key, value) in values {
                            let item = MatchedFilterValue { key, value };
                            if !hit.matched_filters.contains(&item) {
                                hit.matched_filters.push(item);
                            }
                        }
                        continue;
                    }
                }
            }
            let values: Vec<(String, String)> = if op.property {
                let mut conditions = Vec::new();
                if let Some(filter) = &op.filter {
                    flatten_conditions(filter, &mut conditions);
                }
                let value = props.get(&op.key).cloned().or_else(|| {
                    for filter in &conditions {
                        let Filter::Cond(c) = filter else {
                            continue;
                        };
                        match c.prop.as_str() {
                            "$created" => {
                                return Some(PropValue::Date {
                                    start_ms: created,
                                    end_ms: None,
                                    has_time: true,
                                })
                            }
                            "$updated" => {
                                return Some(PropValue::Date {
                                    start_ms: updated,
                                    end_ms: None,
                                    has_time: true,
                                })
                            }
                            _ => {}
                        }
                        if c.value == PropValue::Empty || c.op == crate::types::FilterOp::IsEmpty {
                            return Some(PropValue::Empty);
                        }
                        if c.value == PropValue::Checkbox(false)
                            || c.op == crate::types::FilterOp::Unchecked
                        {
                            return Some(PropValue::Checkbox(false));
                        }
                    }
                    None
                });
                value
                    .map(|mut value| {
                        if let PropValue::MultiSelect(ids) = &mut value {
                            let mut selected = HashSet::new();
                            for filter in &conditions {
                                let Filter::Cond(c) = filter else {
                                    continue;
                                };
                                match &c.value {
                                    PropValue::Select(id) | PropValue::Status(id) => {
                                        selected.insert(fold(id));
                                    }
                                    PropValue::MultiSelect(ids) => {
                                        selected.extend(ids.iter().map(|id| fold(id)));
                                    }
                                    _ => {}
                                }
                            }
                            if !selected.is_empty() {
                                ids.retain(|id| selected.contains(&fold(id)));
                            }
                        }
                        let label = if value == PropValue::Empty {
                            "none".into()
                        } else {
                            display(&value, &defs, &op.key)
                        };
                        vec![(op.key.clone(), label)]
                    })
                    .unwrap_or_default()
            } else {
                match op.key.as_str() {
                    "created" => vec![(
                        op.key.clone(),
                        crate::markdown::date_text(created, None, true),
                    )],
                    "updated" => vec![(
                        op.key.clone(),
                        crate::markdown::date_text(updated, None, true),
                    )],
                    "title" => vec![(op.key.clone(), hit.title.clone())],
                    "tag" => {
                        let wanted = alternatives(&op.value)
                            .into_iter()
                            .map(|v| fold(&v))
                            .collect::<Vec<_>>();
                        let mut values = Vec::new();
                        for (key, value) in &props {
                            let tag_key = key == "tags"
                                || defs
                                    .iter()
                                    .any(|d| d.key == *key && normalized_key(&d.name) == "tags");
                            if !tag_key {
                                continue;
                            }
                            if let PropValue::MultiSelect(ids) = value {
                                for id in ids {
                                    let label = display(&PropValue::Select(id.clone()), &defs, key);
                                    let folded = fold(&label);
                                    if wanted.iter().any(|v| {
                                        folded == *v
                                            || folded.starts_with(&format!("{v}/"))
                                            || fold(id) == *v
                                    }) {
                                        values.push(("tag".into(), label));
                                    }
                                }
                            }
                        }
                        values
                    }
                    "date" => {
                        let mut values = Vec::new();
                        if let Some(filter) = &op.filter {
                            // Read each matching date source rather than echoing the range.
                            let mut parts = Vec::new();
                            flatten_conditions(filter, &mut parts);
                            let mut seen = HashSet::new();
                            for f in parts {
                                let Filter::Cond(c) = f else {
                                    continue;
                                };
                                if !seen.insert(c.prop.clone()) {
                                    continue;
                                }
                                let source = extract_source_filter(filter, &c.prop);
                                let mut parameters = vec![Value::Integer(rowid)];
                                let predicate = crate::query::filter_sql(&source, &mut parameters)?;
                                let matches = db.conn.query_row(
                                    &format!(
                                        "SELECT ({predicate}) FROM pages p WHERE p.rowid = ?1"
                                    ),
                                    rusqlite::params_from_iter(parameters.iter()),
                                    |r| r.get::<_, bool>(0),
                                )?;
                                if !matches {
                                    continue;
                                }
                                match c.prop.as_str() {
                                    "$created" => values.push((
                                        "created".into(),
                                        crate::markdown::date_text(created, None, true),
                                    )),
                                    "$updated" => values.push((
                                        "updated".into(),
                                        crate::markdown::date_text(updated, None, true),
                                    )),
                                    "*" => {
                                        for (key, value) in &props {
                                            if !matches!(value, PropValue::Date { .. }) {
                                                continue;
                                            }
                                            let test = replace_any(&source, key);
                                            let mut parameters = vec![Value::Integer(rowid)];
                                            let predicate =
                                                crate::query::filter_sql(&test, &mut parameters)?;
                                            if db.conn.query_row(&format!("SELECT ({predicate}) FROM pages p WHERE p.rowid = ?1"), rusqlite::params_from_iter(parameters.iter()), |r| r.get::<_, bool>(0))? { values.push((key.clone(), display(value, &defs, key))); }
                                        }
                                    }
                                    key => {
                                        if let Some(v) = props.get(key) {
                                            values.push((key.into(), display(v, &defs, key)));
                                        }
                                    }
                                }
                            }
                        }
                        values
                    }
                    "in" | "links" | "from" => {
                        let mut values = Vec::new();
                        for id in page_alternatives(op) {
                            let matched = match op.key.as_str() {
                                "links" => db.conn.query_row("SELECT EXISTS(SELECT 1 FROM links WHERE src = ?1 AND dst = ?2)", params![hit.id, id], |r| r.get::<_, bool>(0))?,
                                "from" => db.conn.query_row("SELECT EXISTS(SELECT 1 FROM links WHERE dst = ?1 AND src = ?2)", params![hit.id, id], |r| r.get::<_, bool>(0))?,
                                _ => {
                                    let mut cache = db.search_filter_cache.borrow_mut();
                                    let tree = subtree(db, &mut cache, &id)?;
                                    db.conn.query_row("SELECT EXISTS(SELECT 1 FROM json_each(?1) WHERE value = ?2)", params![tree, rowid], |r| r.get::<_, bool>(0))?
                                }
                            };
                            if matched {
                                if let Some(title) = db.title_of(&id)? {
                                    values.push((op.key.clone(), title));
                                }
                            }
                        }
                        values
                    }
                    "has" => {
                        let mut values = Vec::new();
                        for key in alternatives(&op.value) {
                            match key.as_str() {
                                "link" => {
                                    let count: u32 = db.conn.query_row(
                                        "SELECT count(*) FROM links WHERE src = ?1",
                                        [&hit.id],
                                        |r| r.get(0),
                                    )?;
                                    if count > 0 {
                                        values.push(("has".into(), format!("{count} links")));
                                    }
                                }
                                "attachment" => {
                                    let names = db.conn.prepare_cached("SELECT coalesce(a.name, r.blob) FROM refs r LEFT JOIN attachments a ON a.id = r.blob WHERE r.page = ?1")?.query_map([&hit.id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                                    if !names.is_empty() {
                                        values.push(("has".into(), names.join(", ")));
                                    }
                                }
                                key => {
                                    let timestamp =
                                        defs.iter().find(|d| d.key == key).and_then(|d| {
                                            match d.kind {
                                                PropKind::CreatedTime => Some(created),
                                                PropKind::UpdatedTime => Some(updated),
                                                _ => None,
                                            }
                                        });
                                    if let Some(timestamp) = timestamp {
                                        values.push((
                                            key.into(),
                                            crate::markdown::date_text(timestamp, None, true),
                                        ));
                                    } else if let Some(v) = props.get(key) {
                                        values.push((key.into(), display(v, &defs, key)));
                                    }
                                }
                            }
                        }
                        values
                    }
                    _ => Vec::new(),
                }
            };
            for (key, value) in values {
                let item = MatchedFilterValue { key, value };
                if !hit.matched_filters.contains(&item) {
                    hit.matched_filters.push(item);
                }
            }
        }
    }
    Ok(())
}

fn flatten_conditions<'a>(filter: &'a Filter, out: &mut Vec<&'a Filter>) {
    match filter {
        Filter::Cond(_) => out.push(filter),
        Filter::And(v) | Filter::Or(v) => {
            for f in v {
                flatten_conditions(f, out);
            }
        }
    }
}

fn status_alias_values(
    db: &Index,
    filter: &Filter,
    props: &HashMap<String, PropValue>,
    defs: &[PropDef],
    rowid: i64,
    timestamps: (i64, i64),
) -> Result<Option<Vec<(String, String)>>> {
    let mut conditions = Vec::new();
    flatten_conditions(filter, &mut conditions);
    let mut keys = Vec::new();
    for condition in &conditions {
        if let Filter::Cond(condition) = condition {
            if !keys.contains(&condition.prop) {
                keys.push(condition.prop.clone());
            }
        }
    }
    if !keys.iter().any(|key| key != "status") {
        return Ok(None);
    }
    let mut values = Vec::new();
    for key in keys {
        let mut parameters = vec![Value::Integer(rowid)];
        let branch = extract_source_filter(filter, &key);
        let predicate = crate::query::filter_sql(&branch, &mut parameters)?;
        let matches: bool = db.conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pages p WHERE p.rowid = ?1 AND {predicate})"),
            rusqlite::params_from_iter(parameters.iter()),
            |row| row.get(0),
        )?;
        if !matches {
            continue;
        }
        let mut value = props
            .get(&key)
            .cloned()
            .unwrap_or_else(|| match key.as_str() {
                "$created" | "$updated" => PropValue::Date {
                    start_ms: if key == "$created" {
                        timestamps.0
                    } else {
                        timestamps.1
                    },
                    end_ms: None,
                    has_time: true,
                },
                _ => PropValue::Empty,
            });
        if let PropValue::MultiSelect(ids) = &mut value {
            let selected: HashSet<_> = conditions
                .iter()
                .filter_map(|filter| match filter {
                    Filter::Cond(condition) if condition.prop == key => Some(&condition.value),
                    _ => None,
                })
                .flat_map(|value| match value {
                    PropValue::Select(id) | PropValue::Status(id) => vec![fold(id)],
                    PropValue::MultiSelect(ids) => ids.iter().map(|id| fold(id)).collect(),
                    _ => Vec::new(),
                })
                .collect();
            if !selected.is_empty() {
                ids.retain(|id| selected.contains(&fold(id)));
            }
        }
        let label = if value == PropValue::Empty {
            "none".into()
        } else {
            display(&value, defs, &key)
        };
        values.push((key, label));
    }
    Ok(Some(values))
}

fn extract_source_filter(filter: &Filter, source: &str) -> Filter {
    match filter {
        Filter::Cond(c) if c.prop == source => filter.clone(),
        Filter::Cond(_) => Filter::Or(Vec::new()),
        Filter::And(v) => Filter::And(
            v.iter()
                .filter(|f| has_source(f, source))
                .map(|f| extract_source_filter(f, source))
                .collect(),
        ),
        Filter::Or(v) => Filter::Or(
            v.iter()
                .filter(|f| has_source(f, source))
                .map(|f| extract_source_filter(f, source))
                .collect(),
        ),
    }
}
fn has_source(filter: &Filter, source: &str) -> bool {
    match filter {
        Filter::Cond(c) => c.prop == source,
        Filter::And(v) | Filter::Or(v) => v.iter().any(|f| has_source(f, source)),
    }
}
fn replace_any(filter: &Filter, key: &str) -> Filter {
    match filter {
        Filter::Cond(c) => {
            let mut c = c.clone();
            if c.prop == "*" {
                c.prop = key.into();
            }
            Filter::Cond(c)
        }
        Filter::And(v) => Filter::And(v.iter().map(|f| replace_any(f, key)).collect()),
        Filter::Or(v) => Filter::Or(v.iter().map(|f| replace_any(f, key)).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn all_day_calendar_dates_use_local_boundaries_west_of_utc() {
        let zone = FixedOffset::west_opt(8 * 3_600).unwrap();
        let stored = Utc
            .with_ymd_and_hms(2026, 10, 10, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            calendar_boundary(Some(stored), false, false, &zone),
            Some(stored + 8 * 3_600_000)
        );
        assert_eq!(
            calendar_boundary(Some(stored), false, true, &zone),
            Some(stored + 32 * 3_600_000 - 1)
        );
        assert_eq!(
            calendar_boundary(Some(stored), true, false, &zone),
            Some(stored)
        );
        assert_eq!(
            calendar_boundary(Some(stored), true, true, &zone),
            Some(stored)
        );
        assert_eq!(calendar_boundary(None, false, false, &zone), None);
    }
}
