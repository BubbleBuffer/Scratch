//! Generic per-page add-on data (`type_data`).
//!
//! Layout in a page's Loro document: root map `type_data`, add-on id -> a
//! map. Inside an add-on map, a value is a scalar (JSON null, bool, number or
//! string), a map (JSON object) or a list (JSON array); maps and lists nest to
//! any depth. A list's maps are selected by their string `id` entry, so a
//! path can run through `cards` -> `c1` -> `items`.
//!
//! Paths start at the add-on's map. A segment is a key while the container is
//! a map; while it is a list, the segment selects the first map item whose
//! `id` is that string. Set and delete work on map keys: the last segment must
//! name a key, and list items change through the list operations. Writes
//! validate the whole path before they change anything. Set creates the
//! missing intermediate maps; every other missing or wrong-kind part is an
//! error.
//!
//! Maps and lists created for a key (an add-on's map, an intermediate map, a
//! key's object or array) are Loro mergeable children, so two replicas that
//! create the same absent part keep their disjoint edits when they sync.
//!
//! Whole-add-on writes (the Markdown import) update the existing data in
//! place: only the differing keys and list items change, so concurrent edits
//! to other parts of the same add-on data are kept.
//!
//! The reserved root key `$markdown` holds a verbatim readable body string.
//! Add-on ids cannot start with `$`; this body uses a separate API.

use loro::{Container, LoroList, LoroMap, LoroValue, ValueOrContainer};
use serde_json::{Map, Number, Value};

use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::mdparse::Yaml;
use crate::vault::Vault;

/// The Loro root map holding every add-on's data.
const ROOT: &str = "type_data";
/// The Markdown front matter key holding the data as one JSON object.
pub(crate) const FM_KEY: &str = "type-data";
/// Reserved readable body, stored as a string and exported as a true marker.
pub(crate) const MARKDOWN_KEY: &str = "$markdown";

/// Selects one item of a list in add-on data: by its position, or by the
/// string `id` of a map item (a string item matches its own text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListItem {
    Index(u32),
    Id(String),
}

/// A container reached by a path.
enum Node {
    Map(LoroMap),
    List(LoroList),
}

// ------------------------------------------------------------- JSON <-> Loro

fn loro_json(v: &LoroValue) -> Value {
    match v {
        LoroValue::Null => Value::Null,
        LoroValue::Bool(b) => Value::Bool(*b),
        LoroValue::Double(d) => Number::from_f64(*d).map_or(Value::Null, Value::Number),
        LoroValue::I64(i) => Value::Number((*i).into()),
        LoroValue::String(s) => Value::String(s.to_string()),
        LoroValue::List(items) => Value::Array(items.iter().map(loro_json).collect()),
        LoroValue::Map(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.to_string(), loro_json(v)))
                .collect(),
        ),
        // Binary values and container references are not add-on data.
        _ => Value::Null,
    }
}

fn node_json(v: &ValueOrContainer) -> Value {
    match v {
        ValueOrContainer::Value(x) => loro_json(x),
        ValueOrContainer::Container(Container::Map(m)) => map_json(m),
        ValueOrContainer::Container(Container::List(l)) => list_json(l),
        ValueOrContainer::Container(_) => Value::Null,
    }
}

fn map_json(m: &LoroMap) -> Value {
    Value::Object(
        m.keys()
            .filter_map(|k| {
                let k = k.to_string();
                let v = m.get(&k)?;
                Some((k, node_json(&v)))
            })
            .collect(),
    )
}

fn list_json(l: &LoroList) -> Value {
    Value::Array(
        (0..l.len())
            .map(|i| l.get(i).map_or(Value::Null, |v| node_json(&v)))
            .collect(),
    )
}

/// A JSON scalar as a Loro value (integers stay integers).
fn scalar(v: &Value) -> LoroValue {
    match v {
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().unwrap_or(0.0).into(),
        },
        Value::String(s) => s.as_str().into(),
        _ => LoroValue::Null,
    }
}

/// The map at `key`, created as a mergeable child when absent. A deleted
/// mergeable child keeps its old keys (its id is fixed by the parent and key),
/// so a map created again starts empty.
fn child_map(parent: &LoroMap, key: &str) -> Result<LoroMap> {
    match parent.get(key) {
        Some(ValueOrContainer::Container(Container::Map(m))) => return Ok(m),
        Some(ValueOrContainer::Value(LoroValue::Null)) | None => {}
        Some(_) => parent.delete(key)?,
    }
    let m = parent.ensure_mergeable_map(key)?;
    let stale: Vec<String> = m.keys().map(|k| k.to_string()).collect();
    for k in stale {
        m.delete(&k)?;
    }
    Ok(m)
}

/// The list at `key`, created as a mergeable child when absent (emptied, as
/// `child_map` does).
fn child_list(parent: &LoroMap, key: &str) -> Result<LoroList> {
    match parent.get(key) {
        Some(ValueOrContainer::Container(Container::List(l))) => return Ok(l),
        Some(ValueOrContainer::Value(LoroValue::Null)) | None => {}
        Some(_) => parent.delete(key)?,
    }
    let l = parent.ensure_mergeable_list(key)?;
    if l.len() > 0 {
        l.delete(0, l.len())?;
    }
    Ok(l)
}

fn write_map_key(m: &LoroMap, key: &str, v: &Value) -> Result<()> {
    match v {
        Value::Object(o) => reconcile_map(&child_map(m, key)?, o)?,
        Value::Array(a) => reconcile_list(&child_list(m, key)?, a)?,
        _ => m.insert(key, scalar(v))?,
    }
    Ok(())
}

fn write_list_item(l: &LoroList, pos: usize, v: &Value) -> Result<()> {
    match v {
        Value::Object(o) => {
            let c = l.insert_container(pos, LoroMap::new())?;
            for (k, x) in o {
                write_map_key(&c, k, x)?;
            }
        }
        Value::Array(a) => {
            let c = l.insert_container(pos, LoroList::new())?;
            for (i, x) in a.iter().enumerate() {
                write_list_item(&c, i, x)?;
            }
        }
        _ => l.insert(pos, scalar(v))?,
    }
    Ok(())
}

// --------------------------------------------------- minimal in-place updates

/// Makes the map hold exactly the keys of `o`, changing only what differs.
fn reconcile_map(m: &LoroMap, o: &Map<String, Value>) -> Result<()> {
    let stale: Vec<String> = m
        .keys()
        .map(|k| k.to_string())
        .filter(|k| !o.contains_key(k))
        .collect();
    for k in stale {
        m.delete(&k)?;
    }
    for (k, v) in o {
        set_map_key(m, k, v)?;
    }
    Ok(())
}

/// Sets one key. A map or list of the same kind is updated in place; any
/// other value is written fresh, and nothing is written when it is equal.
fn set_map_key(m: &LoroMap, key: &str, v: &Value) -> Result<()> {
    match (m.get(key), v) {
        (Some(ValueOrContainer::Container(Container::Map(c))), Value::Object(o)) => {
            reconcile_map(&c, o)
        }
        (Some(ValueOrContainer::Container(Container::List(c))), Value::Array(a)) => {
            reconcile_list(&c, a)
        }
        (cur, _) => {
            if cur.as_ref().is_some_and(|c| node_json(c) == *v) {
                return Ok(());
            }
            write_map_key(m, key, v)
        }
    }
}

/// Makes the list hold exactly `a`. The common prefix and suffix stay; the
/// middle is updated item by item when both sides have the same length
/// (so an edited map keeps its identity), else replaced.
fn reconcile_list(l: &LoroList, a: &[Value]) -> Result<()> {
    let cur: Vec<Value> = match list_json(l) {
        Value::Array(v) => v,
        _ => Vec::new(),
    };
    let (n, m) = (cur.len(), a.len());
    let mut p = 0;
    while p < n.min(m) && cur[p] == a[p] {
        p += 1;
    }
    let mut s = 0;
    while s < n.min(m) - p && cur[n - 1 - s] == a[m - 1 - s] {
        s += 1;
    }
    let (cur_mid, new_mid) = (n - p - s, m - p - s);
    if cur_mid == new_mid {
        for (i, v) in a.iter().enumerate().take(p + new_mid).skip(p) {
            set_list_item(l, i, v)?;
        }
    } else {
        if cur_mid > 0 {
            l.delete(p, cur_mid)?;
        }
        for (k, v) in a[p..p + new_mid].iter().enumerate() {
            write_list_item(l, p + k, v)?;
        }
    }
    Ok(())
}

fn set_list_item(l: &LoroList, i: usize, v: &Value) -> Result<()> {
    match (l.get(i), v) {
        (Some(ValueOrContainer::Container(Container::Map(c))), Value::Object(o)) => {
            reconcile_map(&c, o)
        }
        (Some(ValueOrContainer::Container(Container::List(c))), Value::Array(a)) => {
            reconcile_list(&c, a)
        }
        (cur, _) => {
            if cur.as_ref().is_some_and(|c| node_json(c) == *v) {
                return Ok(());
            }
            if cur.is_some() {
                l.delete(i, 1)?;
            }
            write_list_item(l, i, v)
        }
    }
}

// ----------------------------------------------------------------- navigation

fn map_id(m: &LoroMap) -> Option<String> {
    match m.get("id")? {
        ValueOrContainer::Value(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// The first map item of `l` whose `id` is `id`, with its position.
fn find_map_item(l: &LoroList, id: &str) -> Option<(usize, LoroMap)> {
    (0..l.len()).find_map(|i| match l.get(i)? {
        ValueOrContainer::Container(Container::Map(m)) if map_id(&m).as_deref() == Some(id) => {
            Some((i, m))
        }
        _ => None,
    })
}

/// The position of a list item: an index in range, or the first item that is
/// the string itself or a map with that `id`.
fn find_item(l: &LoroList, item: &ListItem) -> Option<usize> {
    match item {
        ListItem::Index(i) => ((*i as usize) < l.len()).then_some(*i as usize),
        ListItem::Id(id) => (0..l.len()).find(|&i| match l.get(i) {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => s.to_string() == *id,
            Some(ValueOrContainer::Container(Container::Map(m))) => {
                map_id(&m).as_deref() == Some(id.as_str())
            }
            _ => false,
        }),
    }
}

fn step(cur: Node, seg: &str) -> Result<Node> {
    match cur {
        Node::Map(m) => match m.get(seg) {
            Some(ValueOrContainer::Container(Container::Map(c))) => Ok(Node::Map(c)),
            Some(ValueOrContainer::Container(Container::List(c))) => Ok(Node::List(c)),
            Some(_) => Err(invalid(format!("type data {seg} is not a map or a list"))),
            None => Err(not_found("type data", seg)),
        },
        Node::List(l) => match find_map_item(&l, seg) {
            Some((_, m)) => Ok(Node::Map(m)),
            None => Err(not_found("list item", seg)),
        },
    }
}

fn check_addon(addon: &str) -> Result<()> {
    if addon.is_empty() {
        return Err(invalid("the add-on id must not be empty"));
    }
    if addon.starts_with('$') {
        return Err(invalid("add-on ids starting with $ are reserved"));
    }
    Ok(())
}

fn parse_json(json: &str) -> Result<Value> {
    serde_json::from_str(json).map_err(|e| invalid(format!("invalid JSON: {e}")))
}

/// `mdparse::split_frontmatter`, except that the `type-data` entry is the raw
/// JSON text on its line. The shared scalar reader would cut the JSON at a
/// ` #` or mangle it, so only the front matter prefix is scanned for it. A
/// value that does not start with `{` keeps the reader's entry.
pub(crate) fn split_frontmatter(src: &str) -> (Vec<(String, Yaml)>, &str) {
    let (mut fm, body) = crate::mdparse::split_frontmatter(src);
    let prefix = &src[..src.len() - body.len()];
    let raw = prefix.lines().find_map(|l| {
        let rest = l.strip_prefix(FM_KEY)?.strip_prefix(':')?;
        Some(rest.trim())
    });
    if let Some(raw) = raw.filter(|r| r.starts_with('{')) {
        match fm.iter_mut().find(|(k, _)| k == FM_KEY) {
            Some((_, v)) => *v = Yaml::Str(raw.to_string()),
            None => fm.push((FM_KEY.to_string(), Yaml::Str(raw.to_string()))),
        }
    }
    (fm, body)
}

/// The add-on data a Markdown file declares: the front matter `type-data`
/// key as a JSON object whose add-on values are objects. The reserved
/// `$markdown: true` marker becomes the exact body string. `None` when the
/// key is absent or invalid, so the page's data stays as it is.
pub(crate) fn fm_type_data(fm: &[(String, Yaml)], body: &str) -> Option<Map<String, Value>> {
    let text = fm.iter().find(|(k, _)| k == FM_KEY)?.1.as_str()?;
    let Value::Object(mut o) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    if !o.iter().all(|(k, v)| {
        if k == MARKDOWN_KEY {
            *v == Value::Bool(true)
        } else {
            !k.is_empty() && !k.starts_with('$') && v.is_object()
        }
    }) {
        return None;
    }
    if o.contains_key(MARKDOWN_KEY) {
        o.insert(MARKDOWN_KEY.to_string(), Value::String(body.to_string()));
    }
    Some(o)
}

/// The stored add-on data as the page metadata's JSON text (`""` when none).
pub(crate) fn meta_type_data(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::Object(Map::new()))
}

// -------------------------------------------------------------- page document

impl PageDoc {
    fn type_data_root(&self) -> LoroMap {
        self.doc.get_map(ROOT)
    }

    /// Every add-on's map and the reserved readable body as one JSON object.
    pub(crate) fn type_data_value(&self) -> Value {
        let root = self.type_data_root();
        Value::Object(
            root.keys()
                .filter_map(|k| {
                    let k = k.to_string();
                    match root.get(&k)? {
                        ValueOrContainer::Container(Container::Map(m)) => Some((k, map_json(&m))),
                        ValueOrContainer::Value(LoroValue::String(s)) if k == MARKDOWN_KEY => {
                            Some((k, Value::String(s.to_string())))
                        }
                        _ => None,
                    }
                })
                .collect(),
        )
    }

    /// One add-on's data as a JSON object (`{}` when absent).
    fn type_data_addon(&self, addon: &str) -> Value {
        match self.type_data_root().get(addon) {
            Some(ValueOrContainer::Container(Container::Map(m))) => map_json(&m),
            _ => Value::Object(Map::new()),
        }
    }

    /// The page metadata's copy of the add-on data and readable body: JSON
    /// text, `""` when absent, so an empty page stays free of it.
    pub(crate) fn type_data_text(&self) -> String {
        let root = self.type_data_root();
        if root.is_empty() {
            return String::new();
        }
        self.type_data_value().to_string()
    }

    /// Makes the page's add-on data equal `o` with minimal edits (rescans
    /// and imports).
    pub(crate) fn type_data_reconcile(&self, o: &Map<String, Value>) -> Result<()> {
        reconcile_map(&self.type_data_root(), o)
    }

    fn type_markdown_body(&self) -> Option<String> {
        match self.type_data_root().get(MARKDOWN_KEY) {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
            _ => None,
        }
    }

    fn set_type_markdown_body(&self, body: Option<&str>) -> Result<()> {
        let root = self.type_data_root();
        match body {
            Some(body) => set_map_key(&root, MARKDOWN_KEY, &Value::String(body.to_string()))?,
            None if root.get(MARKDOWN_KEY).is_some() => root.delete(MARKDOWN_KEY)?,
            None => {}
        }
        Ok(())
    }

    /// The container at `path` under `addon`. Every part must exist.
    fn type_data_node(&self, addon: &str, path: &[String]) -> Result<Node> {
        let mut cur = match self.type_data_root().get(addon) {
            Some(ValueOrContainer::Container(Container::Map(m))) => Node::Map(m),
            Some(_) => return Err(invalid(format!("type data {addon} is not a map"))),
            None => return Err(not_found("type data", addon)),
        };
        for seg in path {
            cur = step(cur, seg)?;
        }
        Ok(cur)
    }

    /// The list at `path` under `addon`. Every part must exist.
    fn type_data_list(&self, addon: &str, path: &[String]) -> Result<LoroList> {
        if path.is_empty() {
            return Err(invalid("the path must name a list"));
        }
        match self.type_data_node(addon, path)? {
            Node::List(l) => Ok(l),
            Node::Map(_) => Err(invalid("the path does not name a list")),
        }
    }

    /// Fails for a path that set could not write (checked before any write).
    /// A missing key in a map starts a new branch, which cannot fail. The
    /// last segment must name a map key, not a list item.
    fn check_type_data_set(&self, addon: &str, path: &[String]) -> Result<()> {
        let mut cur = match self.type_data_root().get(addon) {
            Some(ValueOrContainer::Container(Container::Map(m))) => Node::Map(m),
            Some(_) => return Err(invalid(format!("type data {addon} is not a map"))),
            None => return Ok(()),
        };
        for (i, seg) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            cur = match cur {
                Node::Map(m) => match m.get(seg) {
                    Some(ValueOrContainer::Container(Container::Map(c))) => Node::Map(c),
                    Some(ValueOrContainer::Container(Container::List(c))) => Node::List(c),
                    Some(_) if last => return Ok(()),
                    Some(_) => {
                        return Err(invalid(format!("type data {seg} is not a map or a list")))
                    }
                    None => return Ok(()),
                },
                Node::List(l) => match find_map_item(&l, seg) {
                    Some(_) if last => return Err(list_item_path_error()),
                    Some((_, m)) => Node::Map(m),
                    None => return Err(not_found("list item", seg)),
                },
            };
        }
        Ok(())
    }

    /// Sets the map key at `path` under `addon` to `v`, creating the missing
    /// intermediate maps. A list item is changed through the list operations,
    /// so the path must not end at one; an empty path is an error.
    pub(crate) fn type_data_set(&self, addon: &str, path: &[String], v: &Value) -> Result<()> {
        check_addon(addon)?;
        let Some((last, parents)) = path.split_last() else {
            return Err(invalid("the path must name a map key"));
        };
        self.check_type_data_set(addon, path)?;
        let root = self.type_data_root();
        let mut cur = Node::Map(child_map(&root, addon)?);
        for seg in parents {
            cur = match cur {
                Node::Map(m) => match m.get(seg) {
                    Some(ValueOrContainer::Container(Container::Map(c))) => Node::Map(c),
                    Some(ValueOrContainer::Container(Container::List(c))) => Node::List(c),
                    _ => Node::Map(child_map(&m, seg)?),
                },
                Node::List(l) => match find_map_item(&l, seg) {
                    Some((_, m)) => Node::Map(m),
                    None => return Err(not_found("list item", seg)),
                },
            };
        }
        match cur {
            Node::Map(m) => set_map_key(&m, last, v),
            Node::List(_) => Err(list_item_path_error()),
        }
    }

    /// Deletes the map key at `path` under `addon`. A list item is deleted
    /// through the list operations, so the path must not end at one; an empty
    /// path is an error.
    pub(crate) fn type_data_delete(&self, addon: &str, path: &[String]) -> Result<()> {
        check_addon(addon)?;
        let Some((last, parents)) = path.split_last() else {
            return Err(invalid("the path must name a map key"));
        };
        match self.type_data_node(addon, parents)? {
            Node::Map(m) => {
                if m.get(last).is_none() {
                    return Err(not_found("type data", last));
                }
                m.delete(last)?;
            }
            Node::List(l) => match find_map_item(&l, last) {
                Some(_) => return Err(list_item_path_error()),
                None => return Err(not_found("list item", last)),
            },
        }
        Ok(())
    }

    /// Inserts `v` into the list at `path`: at `index`, or at the end.
    pub(crate) fn type_data_list_insert(
        &self,
        addon: &str,
        path: &[String],
        index: Option<u32>,
        v: &Value,
    ) -> Result<()> {
        check_addon(addon)?;
        let list = self.type_data_list(addon, path)?;
        let len = list.len();
        let pos = match index {
            None => len,
            Some(i) if (i as usize) <= len => i as usize,
            Some(i) => return Err(invalid(format!("index {i} is past the end of the list"))),
        };
        write_list_item(&list, pos, v)
    }

    /// Deletes one item of the list at `path`.
    pub(crate) fn type_data_list_delete(
        &self,
        addon: &str,
        path: &[String],
        item: &ListItem,
    ) -> Result<()> {
        check_addon(addon)?;
        let list = self.type_data_list(addon, path)?;
        let i = find_item(&list, item).ok_or_else(|| not_found("list item", &item_text(item)))?;
        list.delete(i, 1)?;
        Ok(())
    }

    /// Moves one item of the list at `path` so it ends at `to_index`. This is
    /// a delete and an insert of the item's JSON, so the moved copy is new:
    /// edits another replica made to the old item are not carried over.
    pub(crate) fn type_data_list_move(
        &self,
        addon: &str,
        path: &[String],
        item: &ListItem,
        to_index: u32,
    ) -> Result<()> {
        check_addon(addon)?;
        let list = self.type_data_list(addon, path)?;
        let i = find_item(&list, item).ok_or_else(|| not_found("list item", &item_text(item)))?;
        let to = to_index as usize;
        if to >= list.len() {
            return Err(invalid(format!("index {to_index} is past the end of the list")));
        }
        if i == to {
            return Ok(());
        }
        let v = list
            .get(i)
            .map_or(Value::Null, |x| node_json(&x));
        list.delete(i, 1)?;
        write_list_item(&list, to, &v)
    }
}

fn list_item_path_error() -> crate::error::VaultError {
    invalid("the path names a list item; use the list operations for list items")
}

fn item_text(item: &ListItem) -> String {
    match item {
        ListItem::Index(i) => i.to_string(),
        ListItem::Id(id) => id.clone(),
    }
}

// ------------------------------------------------------------------ vault API

impl Vault {
    /// One add-on's data on a page, as a JSON object (`{}` when absent).
    pub fn type_data(&self, page: String, addon: String) -> Result<String> {
        check_addon(&addon)?;
        let v = self.inner.read(&page, |d| d.type_data_addon(&addon))?;
        Ok(v.to_string())
    }

    /// All add-on data and the optional `$markdown` body string, as a JSON
    /// object (`{}` when there is none).
    pub fn type_data_all(&self, page: String) -> Result<String> {
        let v = self.inner.read(&page, |d| d.type_data_value())?;
        Ok(v.to_string())
    }

    /// The verbatim readable Markdown body for a page's type, if stored.
    pub fn type_markdown_body(&self, page: String) -> Result<Option<String>> {
        self.inner.read(&page, |d| d.type_markdown_body())
    }

    /// Stores a verbatim readable body through the page's CRDT. `None`
    /// removes it. Markdown export uses it when the page has no blocks, or
    /// only empty paragraph placeholders without children.
    pub fn set_type_markdown_body(&self, page: String, body: Option<String>) -> Result<()> {
        self.inner
            .edit(&page, |d| d.set_type_markdown_body(body.as_deref()))
    }

    /// Sets the map key at `path` (from the add-on's root) to the JSON value.
    /// Missing intermediate maps are created. The path must not be empty or
    /// end at a list item (use the list operations for those).
    pub fn type_data_set(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        json_value: String,
    ) -> Result<()> {
        let v = parse_json(&json_value)?;
        self.inner
            .edit(&page, |d| d.type_data_set(&addon, &path, &v))
    }

    /// Deletes the map key at `path`. The path must not be empty or end at a
    /// list item (use `type_data_list_delete`).
    pub fn type_data_delete(&self, page: String, addon: String, path: Vec<String>) -> Result<()> {
        self.inner
            .edit(&page, |d| d.type_data_delete(&addon, &path))
    }

    /// Inserts a JSON value into the list at `path`, at `index` or at the end.
    pub fn type_data_list_insert(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        index: Option<u32>,
        json_value: String,
    ) -> Result<()> {
        let v = parse_json(&json_value)?;
        self.inner
            .edit(&page, |d| d.type_data_list_insert(&addon, &path, index, &v))
    }

    /// Deletes one item of the list at `path`, by position or by id.
    pub fn type_data_list_delete(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        item: ListItem,
    ) -> Result<()> {
        self.inner
            .edit(&page, |d| d.type_data_list_delete(&addon, &path, &item))
    }

    /// Moves one item of the list at `path` so it ends at `to_index` (a
    /// delete and an insert; see `PageDoc::type_data_list_move`).
    pub fn type_data_list_move(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        item: ListItem,
        to_index: u32,
    ) -> Result<()> {
        self.inner
            .edit(&page, |d| d.type_data_list_move(&addon, &path, &item, to_index))
    }
}
