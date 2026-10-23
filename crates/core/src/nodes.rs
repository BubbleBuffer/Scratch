//! Node records: one page per runner device, under a hidden Nodes page. The
//! scheduler supplies each record as JSON and this module only stores it, so
//! every client has the list offline. The Nodes page is a hidden collection
//! like the calendar page (`index.rs` `NOT_TEMPLATE`): it is left out of
//! lists, search, counts and agenda. Its records are ordinary pages of type
//! `node`, so `type:node` search and listing the Nodes page find them.
use serde_json::{Map, Value};

use crate::doc::PageDoc;
use crate::error::{invalid, Result};
use crate::ids::{node_record_id, stamp_ms};
use crate::types::PageId;
use crate::vault::{lock, Vault};

// Two devices must create the same Nodes page, as the calendar page does.
const NODES_PAGE: &str = "000000000000000000000000ND";
const ROLE_PAGE: &str = "nodes";
const PAGE_TYPE: &str = "node";
// The record's fields live under this add-on key of the page's type data.
const ADDON: &str = "node";

/// Writes the record's title, type and `node` fields, changing only what
/// differs. A `node` key the record no longer has is removed. Other add-on data
/// on the page is left alone.
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
        .filter(|k| !fields.contains_key(*k))
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

    /// The hidden Nodes page, created on first use. Called with the nodes
    /// mutex held.
    fn nodes_page(&self) -> Result<PageId> {
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
