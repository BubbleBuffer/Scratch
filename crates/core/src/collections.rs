//! Properties, collections (schema, views, default template) and queries.
//!
//! Values live in each page's document (`props` map), the schema and views in
//! the collection page's document. The index keeps one typed row per page and
//! property, written on the caller's thread so queries see edits at once.

use std::collections::{HashMap, HashSet};

use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::ids::new_id;
use crate::index::{fold, Index};
use crate::mdparse::parse_iso;
use crate::query;
use crate::types::{
    Filter, Group, PageId, PropDef, PropKind, PropValue, QueryResult, RangeEntry, Sort, ViewDef,
    ViewKind, PAGE_TYPE_DATABASE, PAGE_TYPE_DOCUMENT,
};
use crate::vault::{lock, Vault};

/// Keys every scope has.
const BUILTIN: &[&str] = &["$title", "$created", "$updated", "$cover"];

/// Turns a document into a database and returns whether it changed. A
/// database stays one; any other page type (a mod's) is kept and refused,
/// since its own type decides what it is. A page promoted now shows its
/// children through view blocks: its saved views stay for older clients
/// and are never migrated into blocks.
fn make_database(d: &mut PageDoc) -> Result<bool> {
    match d.meta().page_type.as_str() {
        PAGE_TYPE_DATABASE => Ok(false),
        PAGE_TYPE_DOCUMENT => {
            d.set_page_type(PAGE_TYPE_DATABASE)?;
            d.mark_view_blocks_current()?;
            Ok(true)
        }
        other => Err(invalid(format!("page type {other} is not a database"))),
    }
}

fn exists(db: &Index, id: &str) -> Result<()> {
    if db.exists(id)? {
        Ok(())
    } else {
        Err(not_found("page", id))
    }
}

/// Empty text, URLs and lists become `Empty`; ids are deduplicated.
pub(crate) fn normalize(v: PropValue) -> Result<PropValue> {
    let v = match v {
        PropValue::MultiSelect(ids) => PropValue::MultiSelect(dedup(ids)),
        PropValue::Relation(ids) => PropValue::Relation(dedup(ids)),
        PropValue::Files(ids) => PropValue::Files(dedup(ids)),
        PropValue::Error(_) => return Err(invalid("an error value cannot be set")),
        v => v,
    };
    Ok(match v {
        PropValue::Text(s) | PropValue::Url(s) | PropValue::Select(s) | PropValue::Status(s)
            if s.is_empty() =>
        {
            PropValue::Empty
        }
        PropValue::MultiSelect(v) | PropValue::Relation(v) | PropValue::Files(v)
            if v.is_empty() =>
        {
            PropValue::Empty
        }
        PropValue::Number(x) if !x.is_finite() => return Err(invalid("numbers must be finite")),
        PropValue::Date {
            start_ms,
            end_ms: Some(e),
            ..
        } if e < start_ms => return Err(invalid("a date range must not end before it starts")),
        v => v,
    })
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    v.into_iter()
        .filter(|x| !x.is_empty() && seen.insert(x.clone()))
        .collect()
}

/// Checks a (non-empty) value against its schema definition.
fn check(def: &PropDef, v: &PropValue) -> Result<()> {
    if matches!(
        def.kind,
        PropKind::CreatedTime | PropKind::UpdatedTime | PropKind::Formula | PropKind::Rollup
    ) {
        return Err(invalid(format!("property {} is computed", def.name)));
    }
    let stored = match def.kind {
        // Email and phone numbers are stored as text.
        PropKind::Email | PropKind::Phone => Some(PropKind::Text),
        k => Some(k),
    };
    if v.kind() != stored {
        return Err(invalid(format!(
            "property {} is {:?}, not {:?}",
            def.name,
            def.kind,
            v.kind()
        )));
    }
    let ids: Vec<&String> = match v {
        PropValue::Select(id) | PropValue::Status(id) => vec![id],
        PropValue::MultiSelect(ids) => ids.iter().collect(),
        _ => return Ok(()),
    };
    for id in ids {
        if !def.options.iter().any(|o| o.id == *id) {
            return Err(invalid(format!("property {} has no option {id}", def.name)));
        }
    }
    Ok(())
}

/// The value as display text (option names from `def`).
fn as_text(v: &PropValue, def: &PropDef) -> String {
    let name = |id: &String| {
        def.options
            .iter()
            .find(|o| o.id == *id)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| id.clone())
    };
    match v {
        PropValue::Select(id) | PropValue::Status(id) => name(id),
        PropValue::MultiSelect(ids) => ids.iter().map(name).collect::<Vec<_>>().join(", "),
        PropValue::Relation(ids) | PropValue::Files(ids) => ids.join(", "),
        PropValue::Text(s) | PropValue::Url(s) | PropValue::Error(s) => s.clone(),
        PropValue::Number(x) => {
            if x.fract() == 0.0 && x.abs() < 1e15 {
                format!("{x:.0}")
            } else {
                x.to_string()
            }
        }
        PropValue::Checkbox(b) => b.to_string(),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => crate::markdown::date_text(*start_ms, *end_ms, *has_time),
        PropValue::Empty => String::new(),
    }
}

/// Converts a value after its property changed from `old` to `new` (kind or
/// options). `Empty` means the value is dropped.
pub(crate) fn convert(v: &PropValue, old: &PropDef, new: &PropDef) -> PropValue {
    let by_id = |id: &String| new.options.iter().find(|o| o.id == *id);
    let by_name = |name: &str| {
        let f = fold(name.trim());
        new.options.iter().find(|o| fold(&o.name) == f)
    };
    // Option ids of the value, mapped to the new options (by id, else name).
    let option_ids = |ids: &[String]| -> Vec<String> {
        ids.iter()
            .filter_map(|id| {
                by_id(id)
                    .or_else(|| {
                        old.options
                            .iter()
                            .find(|o| o.id == *id)
                            .and_then(|o| by_name(&o.name))
                    })
                    .map(|o| o.id.clone())
            })
            .collect()
    };
    let text = as_text(v, old);
    let out = match (new.kind, v) {
        (
            PropKind::CreatedTime | PropKind::UpdatedTime | PropKind::Formula | PropKind::Rollup,
            _,
        ) => PropValue::Empty,
        (PropKind::Files, PropValue::Files(ids)) => PropValue::Files(ids.clone()),
        (PropKind::Select | PropKind::Status, PropValue::Select(id) | PropValue::Status(id)) => {
            match option_ids(std::slice::from_ref(id)).pop() {
                Some(id) if new.kind == PropKind::Select => PropValue::Select(id),
                Some(id) => PropValue::Status(id),
                None => PropValue::Empty,
            }
        }
        (PropKind::Select | PropKind::Status, PropValue::MultiSelect(ids)) => {
            match option_ids(ids).into_iter().next() {
                Some(id) if new.kind == PropKind::Select => PropValue::Select(id),
                Some(id) => PropValue::Status(id),
                None => PropValue::Empty,
            }
        }
        (PropKind::Select | PropKind::Status, PropValue::Text(s)) => match by_name(s) {
            Some(o) if new.kind == PropKind::Select => PropValue::Select(o.id.clone()),
            Some(o) => PropValue::Status(o.id.clone()),
            None => PropValue::Empty,
        },
        (PropKind::MultiSelect, PropValue::Select(id) | PropValue::Status(id)) => {
            PropValue::MultiSelect(option_ids(std::slice::from_ref(id)))
        }
        (PropKind::MultiSelect, PropValue::MultiSelect(ids)) => {
            PropValue::MultiSelect(option_ids(ids))
        }
        (PropKind::MultiSelect, PropValue::Text(s)) => PropValue::MultiSelect(
            s.split(',')
                .filter_map(|n| by_name(n).map(|o| o.id.clone()))
                .collect(),
        ),
        (PropKind::Relation, PropValue::Relation(ids)) => PropValue::Relation(ids.clone()),
        (PropKind::Text, _) => PropValue::Text(text),
        (PropKind::Url, PropValue::Url(s) | PropValue::Text(s)) => PropValue::Url(s.clone()),
        (PropKind::Email | PropKind::Phone, PropValue::Text(s) | PropValue::Url(s)) => {
            PropValue::Text(s.clone())
        }
        (PropKind::Number, PropValue::Number(x)) => PropValue::Number(*x),
        (PropKind::Number, PropValue::Checkbox(b)) => PropValue::Number(if *b { 1.0 } else { 0.0 }),
        (PropKind::Number, PropValue::Text(s)) => match s.trim().parse::<f64>() {
            Ok(x) if x.is_finite() => PropValue::Number(x),
            _ => PropValue::Empty,
        },
        (PropKind::Checkbox, PropValue::Checkbox(b)) => PropValue::Checkbox(*b),
        (PropKind::Checkbox, PropValue::Number(x)) => PropValue::Checkbox(*x != 0.0),
        (PropKind::Checkbox, PropValue::Text(s)) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "x" | "1" => PropValue::Checkbox(true),
            "false" | "no" | "0" => PropValue::Checkbox(false),
            _ => PropValue::Empty,
        },
        (PropKind::Date, PropValue::Date { .. }) => v.clone(),
        (PropKind::Date, PropValue::Text(s)) => match parse_iso(s.trim()) {
            Some((ms, has_time)) => PropValue::Date {
                start_ms: ms,
                end_ms: None,
                has_time,
            },
            None => PropValue::Empty,
        },
        _ => PropValue::Empty,
    };
    normalize(out).unwrap_or(PropValue::Empty)
}

/// Assigns missing keys and option ids and checks names are unique.
fn normalize_schema(defs: Vec<PropDef>) -> Result<Vec<PropDef>> {
    let mut keys = HashSet::new();
    let mut names = HashSet::new();
    let mut out = Vec::with_capacity(defs.len());
    for mut d in defs {
        d.key = d.key.trim().to_string();
        if d.key.is_empty() {
            d.key = new_id();
        }
        if d.key.starts_with('$') || d.key == "*" {
            return Err(invalid(format!("property key {} is reserved", d.key)));
        }
        d.name = d.name.trim().to_string();
        if d.name.is_empty() {
            return Err(invalid("a property needs a name"));
        }
        if !keys.insert(d.key.clone()) {
            return Err(invalid(format!("duplicate property key {}", d.key)));
        }
        if !names.insert(fold(&d.name)) {
            return Err(invalid(format!("duplicate property name {}", d.name)));
        }
        if matches!(
            d.kind,
            PropKind::Select | PropKind::MultiSelect | PropKind::Status
        ) {
            let mut ids = HashSet::new();
            let mut onames = HashSet::new();
            for o in &mut d.options {
                o.name = o.name.trim().to_string();
                if o.id.is_empty() {
                    o.id = new_id();
                }
                if o.name.is_empty() {
                    return Err(invalid(format!("an option of {} needs a name", d.name)));
                }
                if o.color.is_empty() {
                    o.color = "default".into();
                }
                if !ids.insert(o.id.clone()) || !onames.insert(fold(&o.name)) {
                    return Err(invalid(format!(
                        "duplicate option {} in {}",
                        o.name, d.name
                    )));
                }
            }
        } else {
            d.options.clear();
        }
        out.push(d);
    }
    Ok(out)
}

/// Whether changing a property from `o` to `n` changes the rows' values or
/// their markdown (kind, name, or an option removed or renamed).
fn affects_rows(o: &PropDef, n: &PropDef) -> bool {
    o.kind != n.kind
        || o.name != n.name
        || o.options.iter().any(|oo| {
            !n.options
                .iter()
                .any(|no| no.id == oo.id && no.name == oo.name)
        })
}

fn default_view() -> ViewDef {
    ViewDef {
        id: new_id(),
        name: "Table".into(),
        kind: ViewKind::Table,
        filter: None,
        sorts: Vec::new(),
        group_by: None,
        date_prop: None,
        visible_props: Vec::new(),
        card_cover: None,
        config: String::new(),
    }
}

fn known(key: &str, schema: &[PropDef]) -> bool {
    BUILTIN.contains(&key) || schema.iter().any(|d| d.key == key)
}

fn prune(f: Filter, schema: &[PropDef]) -> Option<Filter> {
    match f {
        Filter::Cond(c) => known(&c.prop, schema).then_some(Filter::Cond(c)),
        // A group that loses all its conditions goes; one that had none stays.
        Filter::And(v) => {
            let was_empty = v.is_empty();
            let v: Vec<Filter> = v.into_iter().filter_map(|x| prune(x, schema)).collect();
            (was_empty || !v.is_empty()).then_some(Filter::And(v))
        }
        Filter::Or(v) => {
            let was_empty = v.is_empty();
            let v: Vec<Filter> = v.into_iter().filter_map(|x| prune(x, schema)).collect();
            (was_empty || !v.is_empty()).then_some(Filter::Or(v))
        }
    }
}

/// Drops references to properties no longer in the schema.
fn clean_view(v: &mut ViewDef, schema: &[PropDef]) {
    v.sorts.retain(|s| known(&s.prop, schema));
    v.visible_props.retain(|k| known(k, schema));
    v.group_by = v.group_by.take().filter(|k| known(k, schema));
    v.date_prop = v.date_prop.take().filter(|k| known(k, schema));
    v.card_cover = v.card_cover.take().filter(|k| known(k, schema));
    v.filter = v.filter.take().and_then(|f| prune(f, schema));
}

fn check_view(v: &ViewDef, schema: &[PropDef]) -> Result<()> {
    let mut c = v.clone();
    clean_view(&mut c, schema);
    if c != *v {
        return Err(invalid(format!(
            "view {} refers to a property not in the schema",
            v.name
        )));
    }
    Ok(())
}

impl Vault {
    /// Sets a property of a page (`Empty`, an empty text or an empty list
    /// removes it). Rows of a collection take only the schema's keys, with
    /// values of the property's kind and existing option ids; other pages
    /// take any key not starting with `$`.
    pub fn set_property(&self, page: PageId, key: String, value: PropValue) -> Result<()> {
        let db = lock(&self.inner.db);
        exists(&db, &page)?;
        let v = normalize(value)?;
        let schema = match db.parent_of(&page)?.flatten() {
            Some(p) => db.schema_of(&p)?,
            None => None,
        };
        match &schema {
            Some(s) => {
                let def = s
                    .iter()
                    .find(|d| d.key == key)
                    .ok_or_else(|| invalid(format!("no property {key} in the collection")))?;
                if v != PropValue::Empty {
                    check(def, &v)?;
                }
            }
            None => {
                if key.trim().is_empty() || key.starts_with('$') || key == "*" {
                    return Err(invalid(format!("property key {key:?} is reserved")));
                }
            }
        }
        self.inner.edit(&page, |d| {
            if v == PropValue::Empty {
                d.remove_prop(&key)
            } else {
                d.set_prop(&key, &v)
            }
        })?;
        self.inner.sync_meta(&db, &page)
    }

    pub fn remove_property(&self, page: PageId, key: String) -> Result<()> {
        let db = lock(&self.inner.db);
        exists(&db, &page)?;
        self.inner.edit(&page, |d| d.remove_prop(&key))?;
        self.inner.sync_meta(&db, &page)
    }

    /// Makes the page a database (its children are the rows), with an
    /// empty schema and a table view. Does nothing if it already is one.
    /// A page of another type (not a document) is refused.
    pub fn make_collection(&self, page: PageId) -> Result<()> {
        let db = lock(&self.inner.db);
        exists(&db, &page)?;
        self.inner.edit(&page, |d| {
            if make_database(d)? {
                d.set_schema(&[])?;
            }
            if d.views().is_empty() {
                d.set_views(&[default_view()])?;
            }
            Ok(())
        })?;
        self.inner.sync_meta(&db, &page)
    }

    /// The collection's schema (empty if the page is no collection).
    pub fn get_schema(&self, collection: PageId) -> Result<Vec<PropDef>> {
        self.inner
            .read(&collection, |d| d.schema().unwrap_or_default())
    }

    /// Replaces the collection's schema (making the page a collection) and
    /// returns it with keys and option ids assigned. Keys identify
    /// properties: a def with a known key and a new name is a rename. Rows
    /// lose the values of removed properties and removed options; values of
    /// a property whose kind changed are converted where possible (else
    /// dropped). Views lose references to removed properties.
    pub fn set_schema(&self, collection: PageId, schema: Vec<PropDef>) -> Result<Vec<PropDef>> {
        let new = normalize_schema(schema)?;
        let db = lock(&self.inner.db);
        exists(&db, &collection)?;
        let old = self
            .inner
            .read(&collection, |d| d.schema().unwrap_or_default())?;
        self.inner.edit(&collection, |d| {
            make_database(d)?;
            d.set_schema(&new)?;
            let mut views = d.views();
            if views.is_empty() {
                views.push(default_view());
            }
            for v in &mut views {
                clean_view(v, &new);
            }
            d.set_views(&views)
        })?;
        self.inner.sync_meta(&db, &collection)?;
        if !old.iter().any(|o| {
            new.iter()
                .find(|n| n.key == o.key)
                .is_none_or(|n| affects_rows(o, n))
        }) {
            // Added properties or options, new colours or settings: the rows'
            // values and markdown stay as they are.
            return Ok(new);
        }
        let old_by: HashMap<&str, &PropDef> = old.iter().map(|d| (d.key.as_str(), d)).collect();
        let new_by: HashMap<&str, &PropDef> = new.iter().map(|d| (d.key.as_str(), d)).collect();
        let tx = db.write_tx()?;
        for row in db.child_ids(&collection)? {
            let props = self.inner.read(&row, |d| d.props())?;
            let mut changes: Vec<(String, PropValue)> = Vec::new();
            for p in props {
                match (old_by.get(p.key.as_str()), new_by.get(p.key.as_str())) {
                    (Some(_), None) => changes.push((p.key, PropValue::Empty)),
                    (Some(o), Some(n)) if o != n => {
                        let v = convert(&p.value, o, n);
                        if v != p.value {
                            changes.push((p.key, v));
                        }
                    }
                    _ => {}
                }
            }
            if changes.is_empty() {
                // Names in the markdown may have changed.
                self.inner.rerender(&row);
                continue;
            }
            self.inner.edit(&row, |d| {
                for (k, v) in &changes {
                    if *v == PropValue::Empty {
                        d.remove_prop(k)?;
                    } else {
                        d.set_prop(k, v)?;
                    }
                }
                Ok(())
            })?;
            self.inner.sync_meta(&db, &row)?;
        }
        tx.commit()?;
        self.inner.index_committed(&db, Vec::new());
        Ok(new)
    }

    fn collection_schema(&self, db: &Index, collection: &str) -> Result<Vec<PropDef>> {
        exists(db, collection)?;
        db.schema_of(collection)?
            .ok_or_else(|| invalid(format!("page {collection} is not a collection")))
    }

    /// The collection's views in order.
    pub fn list_views(&self, collection: PageId) -> Result<Vec<ViewDef>> {
        self.inner.read(&collection, |d| d.views())
    }

    /// Adds a view at the end; returns its id (assigned if empty).
    pub fn add_view(&self, collection: PageId, mut view: ViewDef) -> Result<String> {
        let db = lock(&self.inner.db);
        let schema = self.collection_schema(&db, &collection)?;
        if view.id.is_empty() {
            view.id = new_id();
        }
        check_view(&view, &schema)?;
        let id = view.id.clone();
        self.inner.edit(&collection, |d| {
            let mut views = d.views();
            if views.iter().any(|v| v.id == view.id) {
                return Err(invalid(format!("view {} exists", view.id)));
            }
            views.push(view);
            d.set_views(&views)
        })?;
        Ok(id)
    }

    /// Replaces the view with the same id.
    pub fn update_view(&self, collection: PageId, view: ViewDef) -> Result<()> {
        let db = lock(&self.inner.db);
        let schema = self.collection_schema(&db, &collection)?;
        check_view(&view, &schema)?;
        self.inner.edit(&collection, |d| {
            let mut views = d.views();
            let slot = views
                .iter_mut()
                .find(|v| v.id == view.id)
                .ok_or_else(|| not_found("view", &view.id))?;
            *slot = view;
            d.set_views(&views)
        })
    }

    pub fn delete_view(&self, collection: PageId, view_id: String) -> Result<()> {
        let db = lock(&self.inner.db);
        self.collection_schema(&db, &collection)?;
        self.inner.edit(&collection, |d| {
            let mut views = d.views();
            let n = views.len();
            views.retain(|v| v.id != view_id);
            if views.len() == n {
                return Err(not_found("view", &view_id));
            }
            d.set_views(&views)
        })
    }

    /// Moves a view to position `index` (clamped).
    pub fn move_view(&self, collection: PageId, view_id: String, index: u32) -> Result<()> {
        let db = lock(&self.inner.db);
        self.collection_schema(&db, &collection)?;
        self.inner.edit(&collection, |d| {
            let mut views = d.views();
            let from = views
                .iter()
                .position(|v| v.id == view_id)
                .ok_or_else(|| not_found("view", &view_id))?;
            let v = views.remove(from);
            views.insert((index as usize).min(views.len()), v);
            d.set_views(&views)
        })
    }

    /// Sets (or clears) the default template of a page's new children (a
    /// collection's: its new rows): one of its children, which becomes a
    /// template (`templates.rs`) and is copied into each new child.
    /// Clearing the default keeps the template.
    pub fn set_default_template(&self, collection: PageId, template: Option<PageId>) -> Result<()> {
        let db = lock(&self.inner.db);
        if !db.exists(&collection)? {
            return Err(not_found("page", &collection));
        }
        if let Some(t) = &template {
            if db.parent_of(t)?.flatten().as_deref() != Some(collection.as_str()) {
                return Err(invalid(format!("page {t} is not a child of {collection}")));
            }
            self.inner.edit(t, |d| d.set_is_template(true))?;
            self.inner.sync_meta(&db, t)?;
        }
        self.inner.edit(&collection, |d| {
            d.set_meta_str("template", template.as_deref())
        })?;
        self.inner.sync_meta(&db, &collection)?;
        drop(db);
        self.inner.emit([crate::types::Change::TreeChanged]);
        Ok(())
    }

    pub fn default_template(&self, collection: PageId) -> Result<Option<PageId>> {
        let db = lock(&self.inner.db);
        exists(&db, &collection)?;
        db.template_of(&collection)
    }

    /// Pages matching `filter`, sorted by `sorts` then by the scope's order
    /// (a collection: sibling order; the vault: last update, newest first).
    /// `scope` is a collection (its rows, template excluded) or `None` (every
    /// page not in the trash). Condition and sort props are property keys
    /// or `$title`, `$created`, `$updated`; `*` in a condition means any
    /// property. Empty values sort last.
    pub fn query(
        &self,
        scope: Option<PageId>,
        filter: Option<Filter>,
        sorts: Vec<Sort>,
        offset: u32,
        limit: u32,
    ) -> Result<QueryResult> {
        let db = lock(&self.inner.db);
        query::query(
            &db,
            scope.as_deref(),
            filter.as_ref(),
            &sorts,
            offset,
            limit,
        )
    }

    /// Board columns: the matching pages counted per value of `group_by`.
    /// Select, status and multi-select give an empty-value column first, then
    /// one per option in schema order (a multi-select row counts in each of
    /// its options); a checkbox gives false and true; other kinds one column
    /// per distinct value, sorted.
    pub fn query_groups(
        &self,
        scope: Option<PageId>,
        filter: Option<Filter>,
        group_by: String,
    ) -> Result<Vec<Group>> {
        let db = lock(&self.inner.db);
        query::groups(&db, scope.as_deref(), filter.as_ref(), &group_by)
    }

    /// Calendar and timeline entries: pages whose date `date_prop` overlaps
    /// `[from_ms, to_ms)`, ordered by start. `date_prop` is a date property
    /// key, `$created`, `$updated` or `*` (any date property: one entry per
    /// matching property).
    pub fn query_range(
        &self,
        scope: Option<PageId>,
        date_prop: String,
        from_ms: i64,
        to_ms: i64,
        filter: Option<Filter>,
    ) -> Result<Vec<RangeEntry>> {
        let db = lock(&self.inner.db);
        query::range(
            &db,
            scope.as_deref(),
            &date_prop,
            from_ms,
            to_ms,
            filter.as_ref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SelectOption;

    fn def(kind: PropKind, opts: &[(&str, &str)]) -> PropDef {
        PropDef {
            key: "k".into(),
            name: "K".into(),
            kind,
            options: opts
                .iter()
                .map(|(id, name)| SelectOption {
                    id: id.to_string(),
                    name: name.to_string(),
                    color: "default".into(),
                })
                .collect(),
            config: String::new(),
        }
    }

    #[test]
    fn converts_between_kinds() {
        let sel = def(PropKind::Select, &[("a", "Alpha"), ("b", "Beta")]);
        let text = def(PropKind::Text, &[]);
        let multi = def(PropKind::MultiSelect, &[("x", "alpha")]);
        assert_eq!(
            convert(&PropValue::Select("a".into()), &sel, &text),
            PropValue::Text("Alpha".into())
        );
        assert_eq!(
            convert(&PropValue::Text("beta".into()), &text, &sel),
            PropValue::Select("b".into())
        );
        assert_eq!(
            convert(&PropValue::Select("a".into()), &sel, &multi),
            PropValue::MultiSelect(vec!["x".into()])
        );
        assert_eq!(
            convert(
                &PropValue::Text("12.5".into()),
                &text,
                &def(PropKind::Number, &[])
            ),
            PropValue::Number(12.5)
        );
        assert_eq!(
            convert(
                &PropValue::Text("soon".into()),
                &text,
                &def(PropKind::Date, &[])
            ),
            PropValue::Empty
        );
        let email = def(PropKind::Email, &[]);
        assert_eq!(
            convert(&PropValue::Url("a@b.c".into()), &text, &email),
            PropValue::Text("a@b.c".into())
        );
        assert_eq!(
            convert(
                &PropValue::Text("555".into()),
                &email,
                &def(PropKind::Phone, &[])
            ),
            PropValue::Text("555".into())
        );
        let mut coloured = sel.clone();
        coloured.options[0].color = "red".into();
        coloured.config = "{\"x\":1}".into();
        assert!(!affects_rows(&sel, &coloured));
        assert!(affects_rows(
            &sel,
            &def(PropKind::Select, &[("a", "Alpha")])
        ));
        assert!(affects_rows(&sel, &text));
        let fewer = def(PropKind::Select, &[("a", "Alpha")]);
        assert_eq!(
            convert(&PropValue::Select("b".into()), &sel, &fewer),
            PropValue::Empty
        );
    }
}
