//! Computed properties: formulas and rollups.
//!
//! Their values are never stored in the documents. They are evaluated here
//! from the index and written to the index's `props` table only, so filters,
//! sorts, groups and row summaries treat them like any other property.
//!
//! A row is recomputed whenever its index row is written (`write_meta`), and
//! so are the rows whose relations point at it (rollups and relation titles
//! in formulas), a few levels deep. Bulk writes (reindex, sync import) defer
//! this and recompute every computed collection at the end.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::formula::{self, Env, EvalResult, Val};
use crate::index::{fold, Index, PropCols, SEP};
use crate::types::{PropDef, PropKind, PropValue, Property};

/// How far a change propagates through relations.
const MAX_DEPTH: u32 = 4;

pub(crate) fn is_computed(k: PropKind) -> bool {
    matches!(k, PropKind::Formula | PropKind::Rollup)
}

/// The schema JSON mentions a computed kind (a cheap test before parsing).
fn schema_has_computed(json: &str) -> bool {
    json.contains("\"Formula\"") || json.contains("\"Rollup\"")
}

fn config(def: &PropDef) -> serde_json::Value {
    serde_json::from_str(&def.config).unwrap_or(serde_json::Value::Null)
}

fn config_str(def: &PropDef, key: &str) -> Option<String> {
    config(def)
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// One page as the index knows it.
struct Row {
    rowid: i64,
    title: String,
    parent: Option<String>,
    created: i64,
    updated: i64,
    is_task: bool,
    props: HashMap<String, PropValue>,
}

/// Defers computed properties until dropped or [`DeferGuard::finish`]ed.
pub(crate) struct DeferGuard<'a>(&'a Index);

impl DeferGuard<'_> {
    /// Ends the deferral and recomputes every computed collection.
    pub(crate) fn finish(self) -> Result<()> {
        self.0.defer_computed(false)
    }
}

impl Drop for DeferGuard<'_> {
    fn drop(&mut self) {
        self.0.defer.set(false);
    }
}

impl Index {
    pub(crate) fn defer_guard(&self) -> DeferGuard<'_> {
        self.defer.set(true);
        DeferGuard(self)
    }

    /// Turns deferred mode on or off. Turning it off recomputes every
    /// computed collection.
    pub(crate) fn defer_computed(&self, on: bool) -> Result<()> {
        self.defer.set(on);
        if !on {
            self.refresh_all_computed()?;
            self.refresh_all_agenda()?;
        }
        Ok(())
    }

    /// Collections with a formula or rollup: (id, schema).
    fn computed_collections(&self) -> Result<Vec<(String, Vec<PropDef>)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, schema FROM pages WHERE page_type = 'database' AND schema IS NOT NULL
               AND (instr(schema, '\"Formula\"') > 0 OR instr(schema, '\"Rollup\"') > 0)",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (id, json) = r?;
            let schema: Vec<PropDef> = serde_json::from_str(&json).unwrap_or_default();
            if schema.iter().any(|d| is_computed(d.kind)) {
                out.push((id, schema));
            }
        }
        Ok(out)
    }

    /// Recomputes every row of every computed collection. Rollups over
    /// other formulas or rollups need their targets first, so collections
    /// with rollups get a second pass.
    pub(crate) fn refresh_all_computed(&self) -> Result<()> {
        let colls = self.computed_collections()?;
        for pass in 0..2 {
            for (id, schema) in &colls {
                if pass == 1 && !schema.iter().any(|d| d.kind == PropKind::Rollup) {
                    continue;
                }
                for row in self.child_ids(id)? {
                    self.refresh_row(&row)?;
                }
            }
        }
        Ok(())
    }

    /// After a page's index row was written: recomputes it (every row, if
    /// it is a collection whose schema changed) and the rows that relate
    /// to it.
    /// `old_schema` is the schema JSON before the write, when it changed.
    pub(crate) fn computed_after_write(
        &self,
        id: &str,
        old_schema: Option<Option<&str>>,
    ) -> Result<()> {
        if self.defer.get() {
            return Ok(());
        }
        if let Some(old) = old_schema {
            self.drop_stale_computed(id, old)?;
        }
        let colls = self.computed_collections()?;
        if colls.is_empty() {
            return Ok(());
        }
        if old_schema.is_some() && colls.iter().any(|(c, _)| c == id) {
            for row in self.child_ids(id)? {
                self.refresh_row(&row)?;
                self.touched.borrow_mut().push(row.clone());
                self.refresh_dependents(&row, &colls)?;
            }
        }
        self.refresh_row(id)?;
        self.refresh_dependents(id, &colls)
    }

    /// Before pages leave the index: the rows that relate to them, to
    /// refresh afterwards with [`Index::refresh_ids`].
    pub(crate) fn dependents_of(&self, ids: &[String]) -> Result<Vec<String>> {
        if self.defer.get() {
            return Ok(Vec::new());
        }
        let colls = self.computed_collections()?;
        let mut out = Vec::new();
        for id in ids {
            out.extend(self.direct_dependents(id, &colls)?);
        }
        out.sort();
        out.dedup();
        out.retain(|d| !ids.contains(d));
        Ok(out)
    }

    pub(crate) fn refresh_ids(&self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.computed_after_write(id, None)?;
            self.touched.borrow_mut().push(id.clone());
        }
        Ok(())
    }

    /// Rows refreshed through their relations since the last call.
    pub(crate) fn take_touched(&self) -> Vec<String> {
        let mut v = std::mem::take(&mut *self.touched.borrow_mut());
        v.sort();
        v.dedup();
        v
    }

    /// Drops the values of properties that were computed in `old` (a
    /// schema JSON) and are not any more: no document holds them.
    fn drop_stale_computed(&self, collection: &str, old: Option<&str>) -> Result<()> {
        let Some(old) = old.filter(|j| schema_has_computed(j)) else {
            return Ok(());
        };
        let old: Vec<PropDef> = serde_json::from_str(old).unwrap_or_default();
        let new: HashMap<String, PropKind> = self
            .schema_of(collection)?
            .unwrap_or_default()
            .into_iter()
            .map(|d| (d.key, d.kind))
            .collect();
        let mut del = self.conn.prepare_cached(
            "DELETE FROM props WHERE key = ?2 AND page IN (SELECT rowid FROM pages
               WHERE parent = ?1 AND (?2 != 'status' OR page_type != 'task'))",
        )?;
        for d in old.iter().filter(|d| is_computed(d.kind)) {
            if !new.get(&d.key).is_some_and(|k| is_computed(*k)) {
                del.execute(params![collection, d.key])?;
            }
        }
        Ok(())
    }

    /// Rows related to `id` in collections with computed properties.
    fn direct_dependents(&self, id: &str, colls: &[(String, Vec<PropDef>)]) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let needle = format!("{SEP}{id}{SEP}");
        for (coll, schema) in colls {
            for d in schema.iter().filter(|d| d.kind == PropKind::Relation) {
                let mut st = self.conn.prepare_cached(
                    "SELECT p.id FROM props x JOIN pages p ON p.rowid = x.page
                     WHERE x.key = ?1 AND instr(x.t, ?2) > 0 AND p.parent = ?3",
                )?;
                let ids = st
                    .query_map(params![d.key, needle, coll], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                out.extend(ids);
            }
        }
        Ok(out)
    }

    fn refresh_dependents(&self, id: &str, colls: &[(String, Vec<PropDef>)]) -> Result<()> {
        let mut seen: HashSet<String> = HashSet::from([id.to_string()]);
        let mut level = vec![id.to_string()];
        for _ in 0..MAX_DEPTH {
            let mut next = Vec::new();
            for x in &level {
                for d in self.direct_dependents(x, colls)? {
                    if seen.insert(d.clone()) {
                        self.refresh_row(&d)?;
                        self.touched.borrow_mut().push(d.clone());
                        next.push(d);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            level = next;
        }
        Ok(())
    }

    fn load_row(&self, id: &str) -> Result<Option<Row>> {
        let head: Option<(i64, String, Option<String>, i64, i64, bool)> = self
            .conn
            .prepare_cached(
                "SELECT rowid, title, parent, created_ms, updated_ms, page_type = 'task' FROM pages WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get::<_, i64>(5)? != 0,
                ))
            })
            .optional()?;
        let Some((rowid, title, parent, created, updated, is_task)) = head else {
            return Ok(None);
        };
        let mut props = HashMap::new();
        let mut st = self
            .conn
            .prepare_cached("SELECT key, json FROM props WHERE page = ?1")?;
        let rows = st.query_map([rowid], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (k, json) = r?;
            if let Ok(v) = serde_json::from_str::<PropValue>(&json) {
                props.insert(k, v);
            }
        }
        Ok(Some(Row {
            rowid,
            title,
            parent,
            created,
            updated,
            is_task,
            props,
        }))
    }

    fn row_in_trash(&self, id: &str) -> Result<bool> {
        Ok(self.in_trash(id)?.unwrap_or(true))
    }

    /// Recomputes the formulas and rollups of one row.
    pub(crate) fn refresh_row(&self, id: &str) -> Result<()> {
        let Some(row) = self.load_row(id)? else {
            return Ok(());
        };
        let Some(parent) = &row.parent else {
            return Ok(());
        };
        let schema = match self.schema_json(parent)? {
            Some(j) if schema_has_computed(&j) => {
                serde_json::from_str::<Vec<PropDef>>(&j).unwrap_or_default()
            }
            _ => return Ok(()),
        };
        // A task note's indexed status comes from type data. A same-key
        // formula/rollup may not overwrite or delete that projection.
        let computed: Vec<&PropDef> = schema
            .iter()
            .filter(|d| is_computed(d.kind) && !(row.is_task && d.key == "status"))
            .collect();
        if computed.is_empty() {
            return Ok(());
        }
        let mut ev = Evaluator {
            db: self,
            row: &row,
            schema: &schema,
            memo: HashMap::new(),
            stack: Vec::new(),
            now: crate::ids::now_ms(),
        };
        let values: Vec<(String, PropValue)> = computed
            .iter()
            .map(|d| (d.key.clone(), ev.value(d)))
            .collect();
        let mut del = self
            .conn
            .prepare_cached("DELETE FROM props WHERE page = ?1 AND key = ?2")?;
        let mut ins = self.conn.prepare_cached(
            "INSERT INTO props(page, key, t, tf, n, d0, d1, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for (key, v) in values {
            del.execute(params![row.rowid, key])?;
            if v == PropValue::Empty {
                continue;
            }
            let (t, tf, n, d0, d1) = computed_cols(&v);
            let json = serde_json::to_string(&v).unwrap_or_default();
            ins.execute(params![row.rowid, key, t, tf, n, d0, d1, json])?;
        }
        Ok(())
    }

    fn schema_json(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached("SELECT schema FROM pages WHERE id = ?1 AND page_type = 'database'")?
            .query_row([id], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten())
    }

    /// The computed property values of a page (for `load_page`).
    pub(crate) fn computed_values(&self, id: &str) -> Result<Vec<Property>> {
        let Some(parent) = self.parent_of(id)?.flatten() else {
            return Ok(Vec::new());
        };
        let keys: HashSet<String> = match self.schema_json(&parent)? {
            Some(j) if schema_has_computed(&j) => serde_json::from_str::<Vec<PropDef>>(&j)
                .unwrap_or_default()
                .into_iter()
                .filter(|d| is_computed(d.kind))
                .map(|d| d.key)
                .collect(),
            _ => return Ok(Vec::new()),
        };
        let mut st = self.conn.prepare_cached(
            "SELECT x.key, x.json FROM props x JOIN pages p ON p.rowid = x.page
               WHERE p.id = ?1 AND (x.key != 'status' OR p.page_type != 'task')",
        )?;
        let rows = st.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (key, json) = r?;
            if keys.contains(&key) {
                if let Ok(value) = serde_json::from_str(&json) {
                    out.push(Property { key, value });
                }
            }
        }
        Ok(out)
    }
}

/// Index columns of a computed value. Every result also gets its text in
/// `t`/`tf`, so text filters apply to any formula.
fn computed_cols(v: &PropValue) -> PropCols {
    let text = display(v);
    let (n, d0, d1) = match v {
        PropValue::Number(x) => (Some(*x), None, None),
        PropValue::Checkbox(b) => (Some(if *b { 1.0 } else { 0.0 }), None, None),
        PropValue::Date {
            start_ms, end_ms, ..
        } => (None, Some(*start_ms), *end_ms),
        _ => (None, None, None),
    };
    (Some(text.clone()), Some(fold(&text)), n, d0, d1)
}

fn display(v: &PropValue) -> String {
    match v {
        PropValue::Number(x) => formula::format_num(*x),
        PropValue::Checkbox(b) => b.to_string(),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => crate::markdown::date_text(*start_ms, *end_ms, *has_time),
        PropValue::Text(s) | PropValue::Url(s) | PropValue::Error(s) => s.clone(),
        _ => String::new(),
    }
}

/// A formula result as a property value.
fn to_prop(v: Val) -> PropValue {
    match v {
        Val::Empty => PropValue::Empty,
        Val::Num(x) => PropValue::Number(x),
        Val::Bool(b) => PropValue::Checkbox(b),
        Val::Date {
            start,
            end,
            has_time,
        } => PropValue::Date {
            start_ms: start,
            end_ms: end,
            has_time,
        },
        Val::Str(s) if s.is_empty() => PropValue::Empty,
        Val::Str(s) => PropValue::Text(s),
        Val::List(l) if l.is_empty() => PropValue::Empty,
        v @ Val::List(_) => PropValue::Text(formula::to_text(&v)),
    }
}

struct Evaluator<'a> {
    db: &'a Index,
    row: &'a Row,
    schema: &'a [PropDef],
    memo: HashMap<String, PropValue>,
    /// Computed keys being evaluated (to catch circular references).
    stack: Vec<String>,
    now: i64,
}

impl Evaluator<'_> {
    /// The value of a computed property (an `Error` value on failure).
    fn value(&mut self, def: &PropDef) -> PropValue {
        if let Some(v) = self.memo.get(&def.key) {
            return v.clone();
        }
        if self.stack.contains(&def.key) {
            return PropValue::Error(format!("Circular reference to {}", def.name));
        }
        self.stack.push(def.key.clone());
        let r = match def.kind {
            PropKind::Formula => self.formula(def),
            _ => self.rollup(def),
        };
        self.stack.pop();
        let v = match r {
            Ok(v) => to_prop(v),
            Err(e) => PropValue::Error(e),
        };
        self.memo.insert(def.key.clone(), v.clone());
        v
    }

    fn formula(&mut self, def: &PropDef) -> EvalResult {
        let src = config_str(def, "formula").unwrap_or_default();
        let expr = formula::parse(&src)?;
        formula::eval(&expr, self)
    }

    fn rollup(&mut self, def: &PropDef) -> EvalResult {
        let (Some(rel), Some(func)) = (
            config_str(def, "rollupRelation"),
            config_str(def, "rollupFunction"),
        ) else {
            return Ok(Val::Empty);
        };
        let target = config_str(def, "rollupProperty").unwrap_or_else(|| "$title".into());
        let Some(rel_def) = self
            .schema
            .iter()
            .find(|d| d.key == rel && d.kind == PropKind::Relation)
        else {
            return Err("The relation of this rollup no longer exists".into());
        };
        let ids = match self.row.props.get(&rel_def.key) {
            Some(PropValue::Relation(ids)) => ids.clone(),
            _ => Vec::new(),
        };
        // One value per related page (trashed pages are left out).
        let mut values: Vec<Val> = Vec::new();
        let mut schemas: HashMap<String, Vec<PropDef>> = HashMap::new();
        for id in &ids {
            if self.db.row_in_trash(id).map_err(|e| e.to_string())? {
                continue;
            }
            let Some(r) = self.db.load_row(id).map_err(|e| e.to_string())? else {
                continue;
            };
            if target == "$title" {
                values.push(Val::Str(r.title.clone()));
                continue;
            }
            let schema = match &r.parent {
                Some(p) => schemas
                    .entry(p.clone())
                    .or_insert_with(|| self.db.schema_of(p).ok().flatten().unwrap_or_default())
                    .clone(),
                None => Vec::new(),
            };
            let def = schema.iter().find(|d| d.key == target);
            values.push(match def {
                Some(d) => match d.kind {
                    PropKind::CreatedTime => date_val(r.created),
                    PropKind::UpdatedTime => date_val(r.updated),
                    _ => match r.props.get(&d.key) {
                        // A failed formula on the other side counts as empty.
                        Some(PropValue::Error(_)) | None => empty_of(d.kind),
                        Some(v) => convert(self.db, d, v),
                    },
                },
                None => Val::Empty,
            });
        }
        aggregate(&func, values)
    }
}

impl Env for Evaluator<'_> {
    fn prop(&mut self, name: &str) -> EvalResult {
        let lname = name.trim().to_lowercase();
        let def = self
            .schema
            .iter()
            .find(|d| d.name.trim().to_lowercase() == lname)
            .or_else(|| self.schema.iter().find(|d| d.key == name));
        let Some(def) = def else {
            return match lname.as_str() {
                "title" | "name" => Ok(Val::Str(self.row.title.clone())),
                _ => Err(format!("No property named {name}")),
            };
        };
        match def.kind {
            PropKind::Formula | PropKind::Rollup => {
                let def = def.clone();
                match self.value(&def) {
                    PropValue::Error(e) => Err(if e.starts_with("Circular") {
                        e
                    } else {
                        format!("{} has an error: {e}", def.name)
                    }),
                    v => Ok(prop_val(&v)),
                }
            }
            PropKind::CreatedTime => Ok(date_val(self.row.created)),
            PropKind::UpdatedTime => Ok(date_val(self.row.updated)),
            _ => Ok(match self.row.props.get(&def.key) {
                Some(v) => convert(self.db, def, v),
                None => empty_of(def.kind),
            }),
        }
    }

    fn now(&self) -> i64 {
        self.now
    }
}

fn date_val(ms: i64) -> Val {
    Val::Date {
        start: ms,
        end: None,
        has_time: true,
    }
}

fn empty_of(kind: PropKind) -> Val {
    match kind {
        PropKind::Checkbox => Val::Bool(false),
        PropKind::MultiSelect | PropKind::Relation | PropKind::Files => Val::List(Vec::new()),
        _ => Val::Empty,
    }
}

/// A computed value read back for another formula.
fn prop_val(v: &PropValue) -> Val {
    match v {
        PropValue::Number(x) => Val::Num(*x),
        PropValue::Checkbox(b) => Val::Bool(*b),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => Val::Date {
            start: *start_ms,
            end: *end_ms,
            has_time: *has_time,
        },
        PropValue::Text(s) | PropValue::Url(s) => Val::Str(s.clone()),
        _ => Val::Empty,
    }
}

/// A stored value as a formula value.
fn convert(db: &Index, def: &PropDef, v: &PropValue) -> Val {
    let option = |id: &str| {
        def.options
            .iter()
            .find(|o| o.id == id)
            .map(|o| Val::Str(o.name.clone()))
            .unwrap_or(Val::Empty)
    };
    match v {
        PropValue::Select(id) | PropValue::Status(id) => option(id),
        PropValue::MultiSelect(ids) => Val::List(ids.iter().map(|i| option(i)).collect()),
        PropValue::Relation(ids) => Val::List(
            ids.iter()
                .filter_map(|i| db.title_of(i).ok().flatten())
                .map(Val::Str)
                .collect(),
        ),
        PropValue::Files(ids) => Val::List(
            ids.iter()
                .map(|i| {
                    Val::Str(
                        db.attachment(i)
                            .ok()
                            .flatten()
                            .map(|a| a.name)
                            .unwrap_or_else(|| i.clone()),
                    )
                })
                .collect(),
        ),
        other => prop_val(other),
    }
}

fn is_blank(v: &Val) -> bool {
    match v {
        Val::Empty => true,
        Val::Str(s) => s.is_empty(),
        Val::List(l) => l.is_empty(),
        _ => false,
    }
}

/// A rollup's aggregation over one value per related page.
pub(crate) fn aggregate(func: &str, values: Vec<Val>) -> EvalResult {
    let total = values.len();
    let flat: Vec<Val> = values
        .iter()
        .flat_map(|v| match v {
            Val::List(l) => l.clone(),
            v => vec![v.clone()],
        })
        .filter(|v| !is_blank(v))
        .collect();
    let nums: Vec<f64> = flat
        .iter()
        .filter_map(|v| match v {
            Val::Num(x) => Some(*x),
            _ => None,
        })
        .collect();
    let dates: Vec<i64> = flat
        .iter()
        .filter_map(|v| match v {
            Val::Date { start, .. } => Some(*start),
            _ => None,
        })
        .collect();
    let empty = values.iter().filter(|v| is_blank(v)).count();
    let checked = values
        .iter()
        .filter(|v| matches!(v, Val::Bool(true)))
        .count();
    let pct = |n: usize| {
        if total == 0 {
            Val::Empty
        } else {
            Val::Num((n as f64 * 10_000.0 / total as f64).round() / 100.0)
        }
    };
    let num_or_date = |want_max: bool| -> Val {
        if nums.is_empty() && !dates.is_empty() {
            let d = if want_max {
                *dates.iter().max().unwrap_or(&0)
            } else {
                *dates.iter().min().unwrap_or(&0)
            };
            let has_time = flat
                .iter()
                .any(|v| matches!(v, Val::Date { has_time: true, .. }));
            Val::Date {
                start: d,
                end: None,
                has_time,
            }
        } else if nums.is_empty() {
            Val::Empty
        } else if want_max {
            Val::Num(nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        } else {
            Val::Num(nums.iter().cloned().fold(f64::INFINITY, f64::min))
        }
    };
    Ok(match func.to_ascii_lowercase().as_str() {
        "show_original" | "original" => Val::List(flat),
        "count" | "count_all" => Val::Num(total as f64),
        "count_values" => Val::Num(flat.len() as f64),
        "count_unique" | "count_unique_values" => {
            let set: HashSet<String> = flat.iter().map(formula::to_text).collect();
            Val::Num(set.len() as f64)
        }
        "count_empty" => Val::Num(empty as f64),
        "count_not_empty" => Val::Num((total - empty) as f64),
        "percent_empty" => pct(empty),
        "percent_not_empty" => pct(total - empty),
        "checked" | "count_checked" => Val::Num(checked as f64),
        "unchecked" | "count_unchecked" => Val::Num((total - checked) as f64),
        "percent_checked" => pct(checked),
        "percent_unchecked" => pct(total - checked),
        "sum" => Val::Num(nums.iter().sum()),
        "avg" | "average" => {
            if nums.is_empty() {
                Val::Empty
            } else {
                Val::Num(nums.iter().sum::<f64>() / nums.len() as f64)
            }
        }
        "median" => {
            if nums.is_empty() {
                Val::Empty
            } else {
                let mut s = nums.clone();
                s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let n = s.len();
                Val::Num(if n % 2 == 1 {
                    s[n / 2]
                } else {
                    (s[n / 2 - 1] + s[n / 2]) / 2.0
                })
            }
        }
        "min" | "earliest_date" => num_or_date(false),
        "max" | "latest_date" => num_or_date(true),
        "range" => {
            if nums.is_empty() {
                Val::Empty
            } else {
                let lo = nums.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                Val::Num(hi - lo)
            }
        }
        "date_range" => match (dates.iter().min(), dates.iter().max()) {
            (Some(a), Some(b)) => Val::Num(((b - a) / 86_400_000) as f64),
            _ => Val::Empty,
        },
        other => return Err(format!("Unknown rollup function {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregations() {
        let v = vec![Val::Num(2.0), Val::Num(4.0), Val::Empty, Val::Num(9.0)];
        assert_eq!(aggregate("count", v.clone()), Ok(Val::Num(4.0)));
        assert_eq!(aggregate("count_values", v.clone()), Ok(Val::Num(3.0)));
        assert_eq!(aggregate("sum", v.clone()), Ok(Val::Num(15.0)));
        assert_eq!(aggregate("average", v.clone()), Ok(Val::Num(5.0)));
        assert_eq!(aggregate("median", v.clone()), Ok(Val::Num(4.0)));
        assert_eq!(aggregate("min", v.clone()), Ok(Val::Num(2.0)));
        assert_eq!(aggregate("max", v.clone()), Ok(Val::Num(9.0)));
        assert_eq!(aggregate("range", v.clone()), Ok(Val::Num(7.0)));
        assert_eq!(aggregate("percent_empty", v.clone()), Ok(Val::Num(25.0)));
        let b = vec![Val::Bool(true), Val::Bool(false), Val::Bool(true)];
        assert_eq!(aggregate("percent_checked", b), Ok(Val::Num(66.67)));
        assert_eq!(aggregate("avg", vec![]), Ok(Val::Empty));
        assert!(aggregate("nope", vec![]).is_err());
    }
}
