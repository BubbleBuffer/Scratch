//! Node records: one page per runner device, under a hidden Nodes page. The
//! app publishes device presence and the scheduler merges its own fields, so
//! every client has the list offline. The Nodes page is a hidden collection
//! like the calendar page (`index.rs` `NOT_TEMPLATE`): it is left out of
//! lists, search, counts and agenda. Its records are ordinary pages of type
//! `node`, so `type:node` search and listing the Nodes page find them.
use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::ids::{node_record_id, now_ms, stamp_ms};
use crate::types::PageId;
use crate::vault::{lock, Vault};

// Two devices must create the same Nodes page, as the calendar page does.
const NODES_PAGE: &str = "000000000000000000000000ND";
const ROLE_PAGE: &str = "nodes";
const PAGE_TYPE: &str = "node";
// The record's fields live under this add-on key of the page's type data.
const ADDON: &str = "node";
// The record's projects, an array of `{id, name}`. Only publish_device_projects
// writes them, so no other write path can drop or replace them.
const PROJECTS: &str = "projects";

/// A runner counts as online while its device's last heartbeat is this recent.
pub const RUNNER_ONLINE_WINDOW_MS: i64 = 300_000;
/// The shortest gap between two heartbeats written for one vault's device.
pub const DEVICE_ACTIVITY_INTERVAL_MS: i64 = 60_000;
// The kind a device record gets when its vault publishes it, and the name of a
// machine whose host name cannot be read.
const DEFAULT_KIND: &str = "desktop";
const FALLBACK_NAME: &str = "This device";

/// One device as the device list shows it. `id` is the public device id
/// (`.vault/device`), the record's `device_id`; the storage page id is
/// `node_record_id(id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// The last heartbeat, from the record's RFC 3339 `last_seen`.
    pub last_seen_ms: Option<i64>,
    /// The projects this device has open, for a task's project. Empty when the
    /// device has published none.
    #[serde(default)]
    pub projects: Vec<DeviceProject>,
}

/// A project a device has open, as its record publishes it: the id and the
/// display name only, never the project's path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceProject {
    pub id: String,
    pub name: String,
}

/// The stored form of `projects`: `{id, name}` entries in the order given. An
/// id must not be blank or repeated. A blank name is stored as the id.
fn project_entries(projects: &[DeviceProject]) -> Result<Value> {
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(projects.len());
    for project in projects {
        let id = project.id.trim();
        if id.is_empty() {
            return Err(invalid("a project needs an id"));
        }
        if !seen.insert(id) {
            return Err(invalid(format!("project {id:?} is listed twice")));
        }
        let name = project.name.trim();
        let name = if name.is_empty() { id } else { name };
        entries.push(json!({ "id": id, "name": name }));
    }
    Ok(Value::Array(entries))
}

/// The projects a record stores. Entries without a usable id are skipped.
fn parse_projects(value: Option<&Value>) -> Vec<DeviceProject> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            Some(DeviceProject {
                id: id.to_owned(),
                name: name.to_owned(),
            })
        })
        .collect()
}

/// Writes the record's title, type and `node` fields, changing only what
/// differs. A `node` key the record no longer has is removed, except
/// `projects`, which the record keeps. Other add-on data on the page is left
/// alone.
fn apply_record(doc: &mut PageDoc, name: &str, fields: &Map<String, Value>) -> Result<()> {
    if doc.meta().title != name {
        doc.set_title(name)?;
    }
    doc.set_page_type(PAGE_TYPE)?;
    let stale: Vec<String> = doc
        .type_data_value()
        .get(ADDON)
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.keys())
        .filter(|k| k.as_str() != PROJECTS && !fields.contains_key(*k))
        .cloned()
        .collect();
    for key in stale {
        doc.type_data_delete(ADDON, &[key])?;
    }
    for (key, value) in fields {
        doc.type_data_set(ADDON, std::slice::from_ref(key), value)?;
    }
    Ok(())
}

/// The fields of a JSON object built with `json!`.
fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

/// This machine's host name, for a device that has no name yet. The core crate
/// has no host-name dependency, so this reads the environment, then
/// `/etc/hostname`, then the system hostname command.
fn hostname() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .chain(std::fs::read_to_string("/etc/hostname").ok())
        .find_map(|name| {
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_owned())
        })
        .or_else(|| {
            std::process::Command::new("hostname")
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| FALLBACK_NAME.to_owned())
}

/// `ms` as RFC 3339 UTC, preserving the timestamp's millisecond precision.
fn rfc3339(ms: i64) -> Result<String> {
    let at = DateTime::<Utc>::from_timestamp_millis(ms)
        .ok_or_else(|| invalid("the device activity time is out of range"))?;
    Ok(at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// A record's `last_seen` as milliseconds, `None` when it is missing or not RFC 3339.
fn parse_last_seen(value: Option<&Value>) -> Option<i64> {
    let text = value?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// Whether a heartbeat at `last` still covers `now`. A stored time after `now`
/// (a clock that moved back, or another machine's clock) does not hold the
/// device back.
fn heartbeat_recent(last: Option<i64>, now: i64) -> bool {
    last.is_some_and(|last| (0..DEVICE_ACTIVITY_INTERVAL_MS).contains(&now.saturating_sub(last)))
}

/// A field that defaults apply to: missing, or not a non-empty string.
fn is_blank(value: &Value) -> bool {
    value.as_str().is_none_or(|s| s.trim().is_empty())
}

impl Vault {
    /// Creates or updates the node record of `device` and returns its page id,
    /// `node_record_id(device)`. `json` is an object whose `device_id` equals
    /// `device` and whose `name` is non-empty; its other fields are stored as
    /// the record. A record in the trash is restored. A purged record cannot be
    /// written again.
    pub fn write_node_record(&self, device: String, json: String) -> Result<String> {
        let record: Value = serde_json::from_str(&json)
            .map_err(|e| invalid(format!("invalid node record JSON: {e}")))?;
        let Value::Object(fields) = record else {
            return Err(invalid("a node record must be a JSON object"));
        };
        if device.trim().is_empty() {
            return Err(invalid("a node record needs a device id"));
        }
        if fields.get("device_id").and_then(Value::as_str) != Some(device.as_str()) {
            return Err(invalid("the node record's device_id does not match"));
        }
        if fields.contains_key(PROJECTS) {
            return Err(invalid(
                "projects are published with publish_device_projects",
            ));
        }
        let name = match fields.get("name").and_then(Value::as_str) {
            Some(name) if !name.trim().is_empty() => name.to_owned(),
            _ => return Err(invalid("a node record needs a name")),
        };
        let preferred = node_record_id(&device);
        if self.inner.is_purged(&preferred) {
            return Err(invalid(format!(
                "node record {preferred} was purged and cannot be written again"
            )));
        }
        let _guard = lock(&self.inner.nodes);
        let nodes = self.nodes_page()?;
        let (id, created) = self.create_unused_page(&preferred, &nodes, &name, None, |doc| {
            apply_record(doc, &name, &fields)
        })?;
        if created {
            return Ok(id);
        }
        // The page at this id must be a node record under the Nodes page.
        let meta = self.inner.read(&id, |doc| doc.meta())?;
        if meta.page_type != PAGE_TYPE || meta.parent.as_deref() != Some(nodes.as_str()) {
            return Err(invalid(format!("page {id} is not a node record")));
        }
        let in_trash = {
            let db = lock(&self.inner.db);
            db.in_trash(&id)? == Some(true)
        };
        if in_trash {
            self.restore_page(id.clone())?;
        }
        self.inner
            .edit(&id, |doc| apply_record(doc, &name, &fields))?;
        let db = lock(&self.inner.db);
        self.inner.sync_meta(&db, &id)?;
        Ok(id)
    }

    /// Merges the supplied fields into this vault's own device record, field by
    /// field: fields `json` omits are kept. An existing name is kept; a supplied
    /// name only names a record that does not exist yet (the host name when
    /// none is supplied). Only `device`, this vault's device id, may be written
    /// through here. Returns the record's page id.
    pub fn merge_node_record(&self, device: String, json: String) -> Result<String> {
        let record: Value = serde_json::from_str(&json)
            .map_err(|e| invalid(format!("invalid node record JSON: {e}")))?;
        let Value::Object(mut fields) = record else {
            return Err(invalid("a node record must be a JSON object"));
        };
        if device != self.device_id() {
            return Err(invalid("only this vault's own device record can be merged"));
        }
        if fields
            .get("device_id")
            .is_some_and(|id| id.as_str() != Some(device.as_str()))
        {
            return Err(invalid("the node record's device_id does not match"));
        }
        if fields.contains_key(PROJECTS) {
            return Err(invalid(
                "projects are published with publish_device_projects",
            ));
        }
        let name = match fields.remove("name") {
            None => hostname(),
            Some(Value::String(name)) if !name.trim().is_empty() => name.trim().to_owned(),
            Some(_) => return Err(invalid("a node record needs a name")),
        };
        fields.insert("device_id".into(), json!(device));
        let defaults = object(json!({ "name": name }));
        let _guard = lock(&self.inner.nodes);
        // Opening or toggling a scheduler may publish its other fields soon
        // after app open/sync. Share their persisted heartbeat throttle rather
        // than advancing last_seen again for the same minute of activity.
        if let Some(at) = parse_last_seen(fields.get("last_seen")) {
            if heartbeat_recent(self.stored_last_seen(&node_record_id(&device))?, at) {
                fields.remove("last_seen");
            }
        }
        let (id, _) = self.merge_record_locked(&device, &name, &defaults, &fields)?;
        Ok(id)
    }

    /// Records a heartbeat for this vault's device: its `last_seen` is set to
    /// `now_ms`, unless the record already has one within
    /// `DEVICE_ACTIVITY_INTERVAL_MS`. A new record is named after the host and
    /// is of kind `desktop`; an existing name and kind are kept. Returns whether
    /// the record changed.
    pub fn publish_device_activity_at(&self, now_ms: i64) -> Result<bool> {
        let device = self.device_id();
        let _guard = lock(&self.inner.nodes);
        let last = self.stored_last_seen(&node_record_id(&device))?;
        if heartbeat_recent(last, now_ms) {
            return Ok(false);
        }
        let name = hostname();
        let defaults = object(json!({ "name": name, "kind": DEFAULT_KIND }));
        let updates = object(json!({ "device_id": device, "last_seen": rfc3339(now_ms)? }));
        let (_, changed) = self.merge_record_locked(&device, &name, &defaults, &updates)?;
        Ok(changed)
    }

    /// `publish_device_activity_at` with the current time.
    pub fn publish_device_activity(&self) -> Result<bool> {
        self.publish_device_activity_at(now_ms())
    }

    /// Publishes the projects this vault's device has open, replacing the
    /// record's list. Each entry is an id and a name. Heartbeats and merges keep
    /// the list. A record that does not exist yet is created without a
    /// heartbeat, so it stays offline. Returns whether the record changed.
    pub fn publish_device_projects(&self, projects: Vec<DeviceProject>) -> Result<bool> {
        let device = self.device_id();
        let entries = project_entries(&projects)?;
        let _guard = lock(&self.inner.nodes);
        let name = hostname();
        let defaults = object(json!({ "name": name, "kind": DEFAULT_KIND }));
        let updates = object(json!({ "device_id": device, "projects": entries }));
        let (_, changed) = self.merge_record_locked(&device, &name, &defaults, &updates)?;
        Ok(changed)
    }

    /// Every device that has a node record, by name then id. Reading never
    /// creates the Nodes page.
    pub fn device_records(&self) -> Result<Vec<DeviceRecord>> {
        let Some(nodes) = self.find_by_role(ROLE_PAGE.into())? else {
            return Ok(Vec::new());
        };
        let mut devices = Vec::new();
        for child in self.list_children(Some(nodes), 0, u32::MAX)? {
            let (meta, data) = self
                .inner
                .read(&child.id, |doc| (doc.meta(), doc.type_data_value()))?;
            if meta.page_type != PAGE_TYPE {
                continue;
            }
            let node = data.get(ADDON).and_then(Value::as_object);
            let text = |key: &str| {
                node.and_then(|n| n.get(key))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            };
            let Some(id) = text("device_id") else {
                continue;
            };
            devices.push(DeviceRecord {
                id: id.to_owned(),
                name: text("name").map_or(meta.title, str::to_owned),
                kind: text("kind").unwrap_or(DEFAULT_KIND).to_owned(),
                last_seen_ms: parse_last_seen(node.and_then(|n| n.get("last_seen"))),
                projects: parse_projects(node.and_then(|n| n.get(PROJECTS))),
            });
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(devices)
    }

    /// Renames a device: its record's name and title, and nothing else. The
    /// name is trimmed and must not be empty. A device with no live record is
    /// `NotFound`.
    pub fn rename_device(&self, device: String, name: String) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("a device needs a name"));
        }
        let _guard = lock(&self.inner.nodes);
        let id = node_record_id(&device);
        let live = match self.find_by_role(ROLE_PAGE.into())? {
            Some(nodes) => self.live_record(&id, &nodes)?,
            None => false,
        };
        if !live {
            return Err(not_found("device", &device));
        }
        self.inner.edit(&id, |doc| {
            doc.set_title(name)?;
            doc.type_data_set(ADDON, &["name".to_owned()], &json!(name))
        })?;
        let db = lock(&self.inner.db);
        self.inner.sync_meta(&db, &id)?;
        Ok(())
    }

    /// Whether `id` is a live (not trashed) node record under the Nodes page `nodes`.
    fn live_record(&self, id: &str, nodes: &str) -> Result<bool> {
        let in_trash = lock(&self.inner.db).in_trash(id)?;
        if in_trash != Some(false) {
            return Ok(false);
        }
        let meta = self.inner.read(id, |doc| doc.meta())?;
        Ok(meta.page_type == PAGE_TYPE && meta.parent.as_deref() == Some(nodes))
    }

    /// The `last_seen` a record holds, `None` when there is no record or no
    /// usable time. Called with the nodes mutex held.
    fn stored_last_seen(&self, id: &str) -> Result<Option<i64>> {
        if lock(&self.inner.db).in_trash(id)?.is_none() {
            return Ok(None);
        }
        let data = self.inner.read(id, |doc| doc.type_data_value())?;
        Ok(parse_last_seen(
            data.get(ADDON).and_then(|node| node.get("last_seen")),
        ))
    }

    /// Writes `updates` and the `defaults` the record lacks, field by field,
    /// and returns the page id and whether anything changed. A new record is
    /// titled `title`; an existing record keeps its title unless a default
    /// supplies its name. Called with the nodes mutex held.
    fn merge_record_locked(
        &self,
        device: &str,
        title: &str,
        defaults: &Map<String, Value>,
        updates: &Map<String, Value>,
    ) -> Result<(String, bool)> {
        let preferred = node_record_id(device);
        if self.inner.is_purged(&preferred) {
            return Err(invalid(format!(
                "node record {preferred} was purged and cannot be written again"
            )));
        }
        let nodes = self.nodes_page()?;
        let (id, created) = self.create_unused_page(&preferred, &nodes, title, None, |doc| {
            doc.set_page_type(PAGE_TYPE)?;
            for (key, value) in defaults.iter().chain(updates) {
                doc.type_data_set(ADDON, std::slice::from_ref(key), value)?;
            }
            Ok(())
        })?;
        if created {
            return Ok((id, true));
        }
        let meta = self.inner.read(&id, |doc| doc.meta())?;
        if meta.page_type != PAGE_TYPE || meta.parent.as_deref() != Some(nodes.as_str()) {
            return Err(invalid(format!("page {id} is not a node record")));
        }
        let was_trashed = lock(&self.inner.db).in_trash(&id)? == Some(true);
        if was_trashed {
            self.restore_page(id.clone())?;
        }
        let edited = self.inner.edit(&id, |doc| {
            let current = doc.type_data_value();
            let current = current
                .get(ADDON)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut changed = false;
            for (key, value) in defaults {
                if current.get(key).is_none_or(is_blank) {
                    doc.type_data_set(ADDON, std::slice::from_ref(key), value)?;
                    if key.as_str() == "name" {
                        doc.set_title(value.as_str().unwrap_or(title))?;
                    }
                    changed = true;
                }
            }
            for (key, value) in updates {
                if current.get(key) != Some(value) {
                    doc.type_data_set(ADDON, std::slice::from_ref(key), value)?;
                    changed = true;
                }
            }
            Ok(changed)
        })?;
        let db = lock(&self.inner.db);
        self.inner.sync_meta(&db, &id)?;
        Ok((id, was_trashed || edited))
    }

    /// The hidden Nodes page, created on first use. Called with the nodes
    /// mutex held.
    pub(crate) fn nodes_page(&self) -> Result<PageId> {
        if let Some(id) = self.find_by_role(ROLE_PAGE.into())? {
            return Ok(id);
        }
        let db = lock(&self.inner.db);
        if db.exists(NODES_PAGE)? {
            return Err(invalid(
                "node records page has lost its role or is in trash",
            ));
        }
        let doc = PageDoc::new_page(
            self.inner.actor_peer(),
            NODES_PAGE,
            "Nodes",
            None,
            stamp_ms(),
        )?;
        doc.set_meta_str("role", Some(ROLE_PAGE))?;
        self.inner.install(&db, doc)
    }
}
