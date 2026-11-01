//! One page's Loro document.
//!
//! Layout inside the document:
//! - map `page`: `id`, `title`, `parent` (string or null), `created`, `updated` (ms).
//!   Also `icon`, `cover` (attachment id), `favorite` (ms when favorited),
//!   `trashed` (ms when moved to the trash), `order` (sibling order key),
//!   `type` (string: the page type; absent for a document, `database` for a
//!   database, other values kept as they are; the old `collection` bool reads
//!   as `database` when no `type` is stored), `template` (page id: the default
//!   template for new children) and `istemplate` (bool: a template of its
//!   parent), each absent
//!   or null when unset.
//!   Display options: `small_text`, `full_width`, `locked` (bool) and
//!   `cover_position` (per mille), absent or null for the defaults.
//! - tree `blocks`: a movable tree (`LoroTree`, fractional index on) with one
//!   node per block. Each node's meta map holds `id` (the caller's BlockId),
//!   `kind` (string), `checked` (bool), `target` (string, optional) and `text`
//!   (a `LoroText` container whose style keys are `bold`, `italic`, `strike`,
//!   `code`, `link`, `pagelink` and `query`).
//! - map `props`: property key -> the `PropValue` as a JSON string.
//! - map `schema` (key -> `PropDef` JSON) and list `schema_order` (keys), for
//!   collections.
//! - map `views` (id -> `ViewDef` JSON) and list `view_order` (ids).
//! - map `peers`: Loro peer id (decimal string) -> device id, written by each
//!   session on its first change, for page history. The peer id is derived
//!   from the device id, so one device keeps one peer.
//! - map `conflicts`: record id -> a sync conflict as JSON (format 2, see
//!   `sync.rs`: the scope, the sides and their frontiers, the applied
//!   resolutions, without the page), removed when resolved. The record id is
//!   not the block: several records may share a block over time. Format 1
//!   records are keyed by their block, which is also their id.
//! - map `type_data`: add-on id -> a map of JSON-like data (maps, lists and
//!   scalars), plus reserved `$markdown` -> a readable body string. See
//!   `type_data.rs`.

use std::collections::{HashMap, HashSet};

use loro::{
    Container, ExpandType, ExportMode, LoroDoc, LoroList, LoroMap, LoroText, LoroTree, LoroValue,
    StyleConfig, StyleConfigMap, TreeID, TreeParentId, UpdateOptions, ValueOrContainer,
};

use crate::error::{invalid, not_found, Result, VaultError};
#[cfg(feature = "storage-metrics")]
use crate::storage_metrics;
use crate::types::{
    Block, BlockId, BlockKind, BlockOp, Mark, MarkKind, NewBlock, PagePrefs, PropDef, PropValue,
    Property, ViewDef,
};

/// Loro merges a peer's consecutive commits into one change while they are
/// less than this many seconds apart (from the change's first commit). Kept
/// well below the 5-minute history grouping so idle gaps stay visible.
const CHANGE_MERGE_SECS: i64 = 60;

pub(crate) struct PageDoc {
    pub doc: LoroDoc,
    /// Present only for a locally split page; never part of a sync blob.
    cold: Option<ColdSource>,
    meta: LoroMap,
    tree: LoroTree,
    props: LoroMap,
    ids: HashMap<BlockId, TreeID>,
    rev: HashMap<TreeID, BlockId>,
    /// `ids`/`rev` are filled lazily (first `blocks()` or edit), which keeps
    /// the cold open of a big page to one traversal.
    ids_ready: bool,
    /// This session's peer id is recorded in the `peers` map.
    peer_registered: bool,
}

#[derive(Clone)]
struct ColdSource {
    vault: std::path::PathBuf,
    page: String,
    root: loro::Frontiers,
}

/// Plain page metadata read from the document.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageMeta {
    pub id: String,
    pub title: String,
    pub parent: Option<String>,
    pub created: i64,
    pub updated: i64,
    pub icon: Option<String>,
    pub cover: Option<String>,
    pub favorite: Option<i64>,
    pub trashed: Option<i64>,
    pub order: String,
    /// The page type (`crate::types::PAGE_TYPE_DOCUMENT` when none is stored).
    pub page_type: String,
    pub template: Option<String>,
    /// A template for new children of the parent (`templates.rs`).
    pub is_template: bool,
    pub prefs: PagePrefs,
    pub color: Option<String>,
    pub role: Option<String>,
    pub agenda_role: Option<String>,
    pub agenda_date: Option<String>,
    /// The page's add-on data and optional `$markdown` body string as one
    /// JSON object (`""` when absent). Markdown writes the body separately
    /// and replaces the string with a true marker in `type-data`.
    pub type_data: String,
}

fn style_config() -> StyleConfigMap {
    let mut styles = StyleConfigMap::new();
    for key in ["bold", "italic", "strike", "underline", "color"] {
        styles.insert(
            key.into(),
            StyleConfig {
                expand: ExpandType::After,
            },
        );
    }
    for key in ["code", "link", "pagelink", "query", "date"] {
        styles.insert(
            key.into(),
            StyleConfig {
                expand: ExpandType::None,
            },
        );
    }
    styles
}

#[cfg(test)]
mod day_restore_api_tests {
    use loro::{ExportMode, LoroDoc, TreeParentId};

    #[test]
    fn deleted_root_json_move_restores_same_physical_node_concurrently() {
        let original = LoroDoc::new();
        original.set_peer_id(1).unwrap();
        let tree = original.get_tree("blocks");
        tree.enable_fractional_index(0);
        let root = tree.create(TreeParentId::Root).unwrap();
        tree.get_meta(root).unwrap().insert("id", "root").unwrap();
        let child = tree.create(root).unwrap();
        tree.get_meta(child).unwrap().insert("id", "child").unwrap();
        original.commit();
        let original_nodes = tree.get_nodes(false);
        tree.delete(root).unwrap();
        original.commit();
        let snapshot = original.export(ExportMode::Snapshot).unwrap();
        let a = LoroDoc::new();
        let b = LoroDoc::new();
        for (doc, peer) in [(&a, 100), (&b, 200)] {
            doc.import(&snapshot).unwrap();
            doc.set_peer_id(peer).unwrap();
            doc.set_record_timestamp(true);
            doc.get_map("before").insert("peer", peer as i64).unwrap();
            doc.commit();
            crate::day_repair::move_nodes(doc, &original_nodes).unwrap();
            assert_eq!(doc.get_tree("blocks").roots(), vec![root]);
            assert_eq!(doc.get_tree("blocks").children(root).unwrap(), vec![child]);
            doc.get_map("after").insert("peer", peer as i64).unwrap();
            doc.commit();
        }
        a.import(&b.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        b.import(&a.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        for doc in [&a, &b] {
            assert_eq!(doc.get_tree("blocks").roots(), vec![root]);
            assert_eq!(doc.get_tree("blocks").nodes().len(), 2);
            assert_eq!(doc.get_tree("blocks").children(root).unwrap(), vec![child]);
        }
        assert_eq!(a.get_deep_value(), b.get_deep_value());
    }
}

impl PageDoc {
    pub(crate) fn invalidate_block_ids(&mut self) {
        self.ids_ready = false;
        self.ids.clear();
        self.rev.clear();
    }

    /// A new page document whose changes are made by `peer` (the vault's
    /// peer id, see `Inner::peer`).
    pub fn new_page(
        peer: u64,
        id: &str,
        title: &str,
        parent: Option<&str>,
        now: i64,
    ) -> Result<PageDoc> {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer)?;
        let mut pd = PageDoc::setup(doc);
        pd.meta.insert("id", id)?;
        pd.meta.insert("title", title)?;
        match parent {
            Some(p) => pd.meta.insert("parent", p)?,
            None => pd.meta.insert("parent", LoroValue::Null)?,
        }
        pd.meta.insert("created", now)?;
        pd.meta.insert("updated", now)?;
        pd.doc.commit();
        pd.ids_ready = true;
        Ok(pd)
    }

    /// Loads a snapshot followed by any number of update blobs.
    pub fn from_bytes(snapshot: &[u8], updates: &[Vec<u8>]) -> Result<PageDoc> {
        #[cfg(feature = "storage-metrics")]
        let _m = storage_metrics::span("pagedoc.from_bytes");
        let doc = LoroDoc::new();
        {
            #[cfg(feature = "storage-metrics")]
            let _s = storage_metrics::span("loro.import_initial_blob");
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("loro.import_initial_blob", snapshot.len() as u64);
            doc.import(snapshot)?;
        }
        if !updates.is_empty() {
            #[cfg(feature = "storage-metrics")]
            let _b = storage_metrics::span("loro.import_batch");
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("loro.import_batch", updates.iter().map(|b| b.len() as u64).sum());
            doc.import_batch(updates)?;
        }
        Ok(PageDoc::setup(doc))
    }

    fn setup(doc: LoroDoc) -> PageDoc {
        doc.config_text_style(style_config());
        doc.set_record_timestamp(true);
        doc.set_change_merge_interval(CHANGE_MERGE_SECS);
        let meta = doc.get_map("page");
        let tree = doc.get_tree("blocks");
        let props = doc.get_map("props");
        tree.enable_fractional_index(0);
        PageDoc {
            doc,
            cold: None,
            meta,
            tree,
            props,
            ids: HashMap::new(),
            rev: HashMap::new(),
            ids_ready: false,
            peer_registered: false,
        }
    }

    /// A read-only copy of the document as of `frontiers`.
    pub fn fork_at(&self, frontiers: &loro::Frontiers) -> Result<PageDoc> {
        if self.doc.is_shallow() && self.cold.is_some() {
            // Loro cannot fork shallow documents, even at a post-root head.
            return self.full_copy()?.fork_at(frontiers);
        }
        let doc = {
            #[cfg(feature = "storage-metrics")]
            let _f = storage_metrics::span("loro.fork_at");
            self.doc.fork_at(frontiers)?
        };
        Ok(PageDoc::setup(doc))
    }

    /// Imports Loro updates (from a fork of this document or another
    /// device). Block ids are re-read on the next read or edit.
    pub fn import_updates(&mut self, bytes: &[u8]) -> Result<()> {
        let mut imported = {
            #[cfg(feature = "storage-metrics")]
            let _i = storage_metrics::span("loro.import_update");
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("loro.import_update", bytes.len() as u64);
            self.doc.import(bytes)
        };
        // Even a rejected import may have applied ops to the block tree.
        self.ids_ready = false;
        self.ids.clear();
        self.rev.clear();
        if matches!(imported, Err(loro::LoroError::ImportUpdatesThatDependsOnOutdatedVersion))
            && self.cold.is_some()
        {
            // Rebuild includes any ops already applied by the failed import.
            // Replaying the batch is idempotent by original operation ID.
            self.rehydrate()?;
            #[cfg(feature = "storage-metrics")]
            let _r = storage_metrics::span("loro.import_update");
            imported = self.doc.import(bytes);
        }
        imported?;
        Ok(())
    }

    /// Makes further changes come from `peer`. Loro continues the peer's
    /// counter from the ops the document already has.
    pub fn set_peer(&mut self, peer: u64) -> Result<()> {
        if self.doc.peer_id() != peer {
            self.doc.set_peer_id(peer)?;
            self.peer_registered = false;
        }
        Ok(())
    }

    /// Open sync conflicts: record id -> the conflict as JSON.
    pub fn conflicts(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let LoroValue::Map(m) = self.doc.get_map("conflicts").get_value() {
            for (k, v) in m.iter() {
                if let LoroValue::String(s) = v {
                    out.push((k.clone(), s.to_string()));
                }
            }
        }
        out.sort();
        out
    }

    /// Stores a conflict record under its record id.
    pub fn set_conflict(&self, id: &str, json: &str) -> Result<()> {
        self.doc.get_map("conflicts").insert(id, json)?;
        Ok(())
    }

    /// Removes a conflict record by its record id.
    pub fn remove_conflict(&self, id: &str) -> Result<()> {
        let m = self.doc.get_map("conflicts");
        if m.get(id).is_some() {
            m.delete(id)?;
        }
        Ok(())
    }

    /// Records this session's peer id -> `device` (once per session).
    pub fn register_peer(&mut self, device: &str) -> Result<()> {
        if self.peer_registered {
            return Ok(());
        }
        let peers = self.doc.get_map("peers");
        let key = self.doc.peer_id().to_string();
        if map_str(&peers, &key).as_deref() != Some(device) {
            peers.insert(&key, device)?;
        }
        self.peer_registered = true;
        Ok(())
    }

    /// Peer id (decimal) -> device id.
    pub fn peers(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        if let LoroValue::Map(m) = self.doc.get_map("peers").get_value() {
            for (k, v) in m.iter() {
                if let LoroValue::String(s) = v {
                    out.insert(k.clone(), s.to_string());
                }
            }
        }
        out
    }

    fn rebuild_ids(&mut self) {
        self.ids_ready = true;
        self.ids.clear();
        self.rev.clear();
        let mut stack: Vec<TreeID> = self.tree.children(TreeParentId::Root).unwrap_or_default();
        while let Some(node) = stack.pop() {
            if let Ok(m) = self.tree.get_meta(node) {
                if let Some(id) = map_str(&m, "id") {
                    self.ids.insert(id.clone(), node);
                    self.rev.insert(node, id);
                }
            }
            if let Some(ch) = self.tree.children(node) {
                stack.extend(ch);
            }
        }
    }

    /// Whether the block tree has been read since load (the Loro state of
    /// every block is decoded then).
    pub fn is_warm(&self) -> bool {
        self.ids_ready
    }

    /// Encoded oplog frontiers: identifies the document version without
    /// decoding its state.
    pub fn version(&self) -> Vec<u8> {
        self.doc.oplog_frontiers().encode()
    }

    pub fn snapshot(&self) -> Result<Vec<u8>> {
        if !self.doc.is_shallow() {
            if let Some(cold) = &self.cold {
                return crate::history::hot_snapshot(self, &cold.root);
            }
        }
        #[cfg(feature = "storage-metrics")]
        let _s = storage_metrics::span("loro.export_snapshot");
        Ok(self.doc.export(ExportMode::Snapshot)?)
    }

    /// Complete history for detached historical views and retained archives.
    /// Ordinary hot writes use `snapshot`, which preserves the cold split.
    pub(crate) fn full_snapshot(&self) -> Result<Vec<u8>> {
        if self.doc.is_shallow() {
            return Ok(self.full_copy()?.doc.export(ExportMode::Snapshot)?);
        }
        Ok(self.doc.export(ExportMode::Snapshot)?)
    }

    pub(crate) fn attach_cold(&mut self, vault: &std::path::Path, page: &str) {
        if self.doc.is_shallow() {
            self.cold = Some(ColdSource {
                vault: vault.to_owned(),
                page: page.to_owned(),
                root: self.doc.shallow_since_frontiers(),
            });
        }
    }

    pub(crate) fn needs_cold_export(&self, since: &loro::VersionVector) -> bool {
        let root = if self.doc.is_shallow() {
            Some(self.doc.shallow_since_frontiers())
        } else { self.cold.as_ref().map(|cold| cold.root.clone()) };
        root.is_some_and(|root| self.doc.frontiers_to_vv(&root)
            .is_none_or(|vv| !since.includes_vv(&vv)))
    }

    pub(crate) fn retain_cold_root(&mut self, vault: &std::path::Path, page: &str, root: loro::Frontiers) {
        self.cold = Some(ColdSource { vault: vault.to_owned(), page: page.to_owned(), root });
    }

    pub(crate) fn materialize_roots_in(&self, target: &LoroDoc) {
        // Root containers exist implicitly in Loro. Merely accessing an empty
        // root materializes it in snapshots/deep values without producing ops,
        // so updates alone need not recreate it. Match the loaded hot doc's
        // root handles generically, without changing content or operation IDs.
        if let LoroValue::Map(roots) = self.doc.get_value() {
            for value in roots.values() {
                if let LoroValue::Container(id) = value {
                    target.get_container(id.clone());
                }
            }
        }
    }

    pub(crate) fn cold_state(&self) -> LoroValue {
        let value = {
            #[cfg(feature = "storage-metrics")]
            let _d = storage_metrics::span("loro.deep_value");
            self.doc.get_deep_value()
        };
        let LoroValue::Map(roots) = &value else { return value; };
        LoroValue::Map(roots.iter().filter(|(_, v)| match v {
            LoroValue::Map(m) => !m.is_empty(),
            LoroValue::List(l) => !l.is_empty(),
            LoroValue::String(s) => !s.is_empty(),
            _ => true,
        }).map(|(name, v)| (name.clone(), v.clone())).collect())
    }

    fn full_copy(&self) -> Result<PageDoc> {
        let cold = self.cold.as_ref().ok_or_else(|| invalid("missing cold history"))?;
        crate::vault::with_cold_read_lock(&cold.vault, || self.full_copy_locked())
    }

    fn full_copy_locked(&self) -> Result<PageDoc> {
        #[cfg(feature = "storage-metrics")]
        let _t = storage_metrics::span("full_copy.total");
        let cold = self.cold.as_ref().ok_or_else(|| invalid("missing cold history"))?;
        let part = crate::vault::cold::read(&cold.vault, &cold.page)?;
        let hot = {
            #[cfg(feature = "storage-metrics")]
            let _e = storage_metrics::span("full_copy.export_cached_hot");
            self.doc.export(ExportMode::all_updates())?
        };
        #[cfg(feature = "storage-metrics")]
        storage_metrics::add_bytes("full_copy.export_cached_hot", hot.len() as u64);
        // Callers hold the vault flush lock. Another instance may have
        // advanced the cold root beyond this cached hot doc's version; the
        // newest disk hot snapshot supplies its complete retained suffix.
        let docs = cold.vault.join(".vault/docs");
        let logs = cold.vault.join(".vault/logs");
        let disk = {
            #[cfg(feature = "storage-metrics")]
            let _d = storage_metrics::span("full_copy.read_disk_hot");
            std::fs::read(docs.join(format!("{}.loro", cold.page)))?
        };
        #[cfg(feature = "storage-metrics")]
        storage_metrics::add_bytes("full_copy.read_disk_hot", disk.len() as u64);
        let mut updates = vec![disk, hot];
        {
            #[cfg(feature = "storage-metrics")]
            let _l = storage_metrics::span("full_copy.read_logs");
            for suffix in ["log.old", "log"] {
                let bytes = match std::fs::read(logs.join(format!("{}.{suffix}", cold.page))) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                #[cfg(feature = "storage-metrics")]
                storage_metrics::add_bytes("full_copy.read_logs", bytes.len() as u64);
                crate::vault::decode_frames(&bytes, &mut updates);
            }
        }
        let mut full = {
            #[cfg(feature = "storage-metrics")]
            let _r = storage_metrics::span("full_copy.rebuild_import");
            Self::from_bytes(&part.updates, &updates)?
        };
        self.materialize_roots_in(&full.doc);
        // Verify the original state at its exact frontiers even if the disk
        // supplied newer operations. All history remains in the returned doc.
        let head = self.doc.oplog_frontiers();
        // Usually the rebuilt head is the cached head: no newer disk operations,
        // and the rebuilt state sits at that head. Then the rebuilt state is
        // the state a fork at `head` would hold, so comparing it directly gives
        // the same answer without exporting and re-importing it. Any other
        // version or checked-out frontier keeps the fork below.
        let at_head = {
            #[cfg(feature = "storage-metrics")]
            let _h = storage_metrics::span("full_copy.verify_head");
            !full.doc.is_shallow()
                && full.doc.oplog_vv() == self.doc.oplog_vv()
                && full.doc.oplog_frontiers() == head
                && full.doc.state_frontiers() == head
        };
        let same = if at_head {
            #[cfg(feature = "storage-metrics")]
            let _c = storage_metrics::span("full_copy.verify_compare");
            // The same canonical state comparison as the fork path below.
            full.cold_state() == self.cold_state()
        } else {
            let original = {
                #[cfg(feature = "storage-metrics")]
                let _f = storage_metrics::span("full_copy.verify_fork");
                full.fork_at(&head)?
            };
            #[cfg(feature = "storage-metrics")]
            let _c = storage_metrics::span("full_copy.verify_compare");
            !full.doc.is_shallow() && original.doc.oplog_vv() == self.doc.oplog_vv()
                && original.doc.oplog_frontiers() == self.doc.oplog_frontiers()
                // Updates may also recreate old roots that are now empty and
                // omitted by the shallow exporter. Ignore only empty root values;
                // nested values and every nonempty root must still match exactly.
                && original.cold_state() == self.cold_state()
        };
        if !same {
            return Err(invalid("cold history does not reconstruct the hot page"));
        }
        full.set_peer(self.doc.peer_id())?;
        full.cold = self.cold.clone();
        Ok(full)
    }

    pub(crate) fn rehydrate(&mut self) -> Result<()> {
        if self.doc.is_shallow() && self.cold.is_some() {
            *self = self.full_copy()?;
        }
        Ok(())
    }

    pub fn meta(&self) -> PageMeta {
        PageMeta {
            id: map_str(&self.meta, "id").unwrap_or_default(),
            title: map_str(&self.meta, "title").unwrap_or_default(),
            parent: map_str(&self.meta, "parent"),
            created: map_i64(&self.meta, "created"),
            updated: map_i64(&self.meta, "updated"),
            icon: map_str(&self.meta, "icon"),
            cover: map_str(&self.meta, "cover"),
            color: map_str(&self.meta, "color"),
            role: map_str(&self.meta, "role"),
            agenda_role: map_str(&self.meta, "agenda_role"),
            agenda_date: map_str(&self.meta, "agenda_date"),
            type_data: self.type_data_text(),
            favorite: map_opt_i64(&self.meta, "favorite"),
            trashed: map_opt_i64(&self.meta, "trashed"),
            order: map_str(&self.meta, "order").unwrap_or_default(),
            page_type: read_page_type(&self.meta),
            template: map_str(&self.meta, "template"),
            is_template: matches!(
                self.meta.get("istemplate"),
                Some(ValueOrContainer::Value(LoroValue::Bool(true)))
            ),
            prefs: PagePrefs {
                small_text: map_bool(&self.meta, "small_text"),
                full_width: map_bool(&self.meta, "full_width"),
                locked: map_bool(&self.meta, "locked"),
                cover_position: map_opt_i64(&self.meta, "cover_position")
                    .map(|v| v.clamp(0, 1000) as u32),
            },
        }
    }

    /// Writes the display options that differ from the stored ones (false
    /// and None are stored as null).
    pub fn set_prefs(&self, p: &PagePrefs) -> Result<()> {
        for (key, on) in [
            ("small_text", p.small_text),
            ("full_width", p.full_width),
            ("locked", p.locked),
        ] {
            if map_bool(&self.meta, key) != on {
                if on {
                    self.meta.insert(key, true)?;
                } else {
                    self.meta.insert(key, LoroValue::Null)?;
                }
            }
        }
        self.set_meta_i64("cover_position", p.cover_position.map(i64::from))
    }

    pub fn set_title(&self, title: &str) -> Result<()> {
        self.meta.insert("title", title)?;
        Ok(())
    }

    pub fn set_updated(&self, ms: i64) -> Result<()> {
        self.meta.insert("updated", ms)?;
        Ok(())
    }

    /// Sets a string field of the `page` map (null when `None`), if it changed.
    pub fn set_meta_str(&self, key: &str, value: Option<&str>) -> Result<()> {
        if map_str(&self.meta, key).as_deref() == value {
            return Ok(());
        }
        match value {
            Some(v) => self.meta.insert(key, v)?,
            None => self.meta.insert(key, LoroValue::Null)?,
        }
        Ok(())
    }

    /// Sets an integer field of the `page` map (null when `None`), if it changed.
    pub fn set_meta_i64(&self, key: &str, value: Option<i64>) -> Result<()> {
        if map_opt_i64(&self.meta, key) == value {
            return Ok(());
        }
        match value {
            Some(v) => self.meta.insert(key, v)?,
            None => self.meta.insert(key, LoroValue::Null)?,
        }
        Ok(())
    }

    /// Marks the page as a template of its parent (or not), if it changed.
    pub fn set_is_template(&self, on: bool) -> Result<()> {
        if self.meta().is_template != on {
            self.meta.insert("istemplate", on)?;
        }
        Ok(())
    }

    /// Sets the page type. A document stores no `type` (the key is removed),
    /// and the old `collection` key is removed too, so the stored type is the
    /// only source. Writes only what changed.
    pub fn set_page_type(&self, page_type: &str) -> Result<()> {
        let stored = map_str(&self.meta, "type");
        if page_type == crate::types::PAGE_TYPE_DOCUMENT {
            if stored.is_some() {
                self.meta.delete("type")?;
            }
        } else if stored.as_deref() != Some(page_type) {
            self.meta.insert("type", page_type)?;
        }
        if self.meta.get("collection").is_some() {
            self.meta.delete("collection")?;
        }
        Ok(())
    }

    // ------------------------------------------------------------- properties

    /// Stored properties, sorted by key. Values that do not parse are skipped.
    pub fn props(&self) -> Vec<Property> {
        let mut out = Vec::new();
        if let LoroValue::Map(m) = self.props.get_value() {
            for (k, v) in m.iter() {
                if let LoroValue::String(s) = v {
                    if let Ok(value) = serde_json::from_str::<PropValue>(s) {
                        out.push(Property {
                            key: k.clone(),
                            value,
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Stores a (normalized, non-empty) value, if it changed.
    pub fn set_prop(&self, key: &str, value: &PropValue) -> Result<()> {
        let json = serde_json::to_string(value).map_err(|e| VaultError::Invalid(e.to_string()))?;
        if map_str(&self.props, key).as_deref() != Some(json.as_str()) {
            self.props.insert(key, json)?;
        }
        Ok(())
    }

    pub fn remove_prop(&self, key: &str) -> Result<()> {
        if self.props.get(key).is_some() {
            self.props.delete(key)?;
        }
        Ok(())
    }

    // ------------------------------------------------------- schema and views

    /// The collection schema in order (`None` if the page is no collection).
    pub fn schema(&self) -> Option<Vec<PropDef>> {
        if self.meta().page_type != crate::types::PAGE_TYPE_DATABASE {
            return None;
        }
        Some(read_ordered(&self.doc, "schema", "schema_order"))
    }

    /// Replaces the schema, writing only entries that changed.
    pub fn set_schema(&self, defs: &[PropDef]) -> Result<()> {
        let items: Vec<(String, String)> = defs
            .iter()
            .map(|d| Ok((d.key.clone(), to_json(d)?)))
            .collect::<Result<_>>()?;
        write_ordered(&self.doc, "schema", "schema_order", &items)
    }

    pub fn views(&self) -> Vec<ViewDef> {
        read_ordered(&self.doc, "views", "view_order")
    }

    pub fn set_views(&self, views: &[ViewDef]) -> Result<()> {
        let items: Vec<(String, String)> = views
            .iter()
            .map(|v| Ok((v.id.clone(), to_json(v)?)))
            .collect::<Result<_>>()?;
        write_ordered(&self.doc, "views", "view_order", &items)
    }

    // ---------------------------------------------------------------- reading

    pub fn blocks(&mut self) -> Vec<Block> {
        let mut seen = if self.ids_ready {
            None
        } else {
            Some(Vec::new())
        };
        let blocks = self.read_children(TreeParentId::Root, &mut seen);
        if let Some(seen) = seen {
            self.ids.clear();
            self.rev.clear();
            for (id, node) in seen {
                self.ids.insert(id.clone(), node);
                self.rev.insert(node, id);
            }
            self.ids_ready = true;
        }
        blocks
    }

    fn read_children(
        &self,
        parent: TreeParentId,
        seen: &mut Option<Vec<(BlockId, TreeID)>>,
    ) -> Vec<Block> {
        let Some(children) = self.tree.children(parent) else {
            return Vec::new();
        };
        children
            .into_iter()
            .filter_map(|n| self.read_block(n, seen))
            .collect()
    }

    /// Reads one physical block without decoding unrelated page contents.
    /// Metadata of deleted nodes remains addressable, so verify its whole
    /// ancestor chain is visible before exposing the historical block.
    pub(crate) fn block_at_tree_node(&self, node: TreeID) -> Option<Block> {
        let mut ancestor = node;
        loop {
            match self.tree.parent(ancestor) {
                Some(TreeParentId::Node(parent)) => ancestor = parent,
                Some(TreeParentId::Root) => return self.read_block(node, &mut None),
                _ => return None,
            }
        }
    }

    fn read_block(&self, node: TreeID, seen: &mut Option<Vec<(BlockId, TreeID)>>) -> Option<Block> {
        let m = self.tree.get_meta(node).ok()?;
        let id = map_str(&m, "id")?;
        if let Some(seen) = seen.as_mut() {
            seen.push((id.clone(), node));
        }
        let kind = BlockKind::parse(&map_str(&m, "kind").unwrap_or_else(|| "paragraph".into()));
        let checked = matches!(
            m.get("checked"),
            Some(ValueOrContainer::Value(LoroValue::Bool(true)))
        );
        // Embeds store no text; query specs are plain strings even if a
        // different writer left rich-text attributes on them.
        let (text, marks) = match text_of(&m) {
            Some(t) if kind == BlockKind::Query => (t.to_string(), Vec::new()),
            Some(t) if kind != BlockKind::Embed => read_text(&t),
            _ => (String::new(), Vec::new()),
        };
        Some(Block {
            id,
            kind,
            text,
            marks,
            checked,
            children: self.read_children(TreeParentId::Node(node), seen),
            target: map_str(&m, "target"),
            color: map_str(&m, "color"),
            width: match m.get("width") {
                Some(ValueOrContainer::Value(LoroValue::I64(w))) if w > 0 && w < 100 => {
                    Some(w as u32)
                }
                _ => None,
            },
            numbered_start: match m.get("numbered_start") {
                Some(ValueOrContainer::Value(LoroValue::I64(n)))
                    if (0..=u32::MAX as i64).contains(&n) && n != 1 => Some(n as u32),
                _ => None,
            },
        })
    }

    // ---------------------------------------------------------------- editing

    /// Validates every op against the state the earlier ops of the batch
    /// produce, then applies them. On a validation error nothing is changed.
    /// Does not commit.
    pub fn apply(&mut self, ops: &[BlockOp]) -> Result<()> {
        if !self.ids_ready {
            self.rebuild_ids();
        }
        self.validate(ops)?;
        let before = self.doc.state_frontiers();
        let res = ops.iter().try_for_each(|op| self.apply_one(op));
        if let Err(e) = res {
            // Should not happen after validation; undo whatever was applied.
            self.doc.commit();
            let _ = self.doc.revert_to(&before);
            self.doc.commit();
            self.rebuild_ids();
            return Err(e);
        }
        Ok(())
    }

    fn validate(&self, ops: &[BlockOp]) -> Result<()> {
        let mut sim = Sim {
            pd: self,
            parent: HashMap::new(),
            kind: HashMap::new(),
            added: HashSet::new(),
            removed: HashSet::new(),
        };
        let mut check_parents = HashSet::new();
        for op in ops {
            match op {
                BlockOp::Insert {
                    parent,
                    after,
                    block,
                } => {
                    if block.id.is_empty() {
                        return Err(invalid("block id must not be empty"));
                    }
                    if sim.known(&block.id) {
                        return Err(invalid(format!("block {} already exists", block.id)));
                    }
                    sim.check_position(parent.as_ref(), after.as_ref())?;
                    check_marks(&block.text, &block.marks)?;
                    sim.added.insert(block.id.clone());
                    sim.parent.insert(block.id.clone(), parent.clone());
                    sim.kind.insert(block.id.clone(), block.kind.clone());
                    check_parents.extend(parent.iter().cloned());
                }
                BlockOp::Delete { id } => {
                    sim.require(id)?;
                    sim.removed.insert(id.clone());
                }
                BlockOp::Move { id, parent, after } => {
                    sim.require(id)?;
                    if after.as_deref() == Some(id.as_str()) {
                        return Err(invalid("a block cannot move after itself"));
                    }
                    sim.check_position(parent.as_ref(), after.as_ref())?;
                    let mut cur = parent.clone();
                    while let Some(p) = cur {
                        if &p == id {
                            return Err(invalid(format!("moving {id} into its own subtree")));
                        }
                        cur = sim.parent_of(&p);
                    }
                    sim.parent.insert(id.clone(), parent.clone());
                    check_parents.extend(parent.iter().cloned());
                }
                BlockOp::SetText { id, text, marks } => {
                    sim.require(id)?;
                    check_marks(text, marks)?;
                }
                BlockOp::SetKind { id, kind } => {
                    sim.require(id)?;
                    sim.kind.insert(id.clone(), kind.clone());
                    check_parents.insert(id.clone());
                }
                BlockOp::SetChecked { id, .. }
                | BlockOp::SetTarget { id, .. }
                | BlockOp::SetColor { id, .. }
                | BlockOp::SetWidth { id, .. }
                | BlockOp::SetNumberedStart { id, .. } => sim.require(id)?,
            }
        }
        // Check the final tree so a replacement can turn a parent into a
        // query before deleting or moving its former children in this batch.
        check_parents.retain(|id| {
            sim.exists(id) && sim.kind_of(id) == Some(BlockKind::Query)
        });
        if !check_parents.is_empty()
            && self.ids.keys().chain(sim.added.iter()).any(|child| {
                sim.exists(child)
                    && sim.parent_of(child).is_some_and(|p| check_parents.contains(&p))
            })
        {
            return Err(invalid("a query block cannot have children"));
        }
        Ok(())
    }

    fn node(&self, id: &str) -> Result<TreeID> {
        self.ids
            .get(id)
            .copied()
            .ok_or_else(|| not_found("block", id))
    }

    fn parent_node(&self, parent: Option<&BlockId>) -> Result<TreeParentId> {
        Ok(match parent {
            Some(p) => TreeParentId::Node(self.node(p)?),
            None => TreeParentId::Root,
        })
    }

    fn apply_one(&mut self, op: &BlockOp) -> Result<()> {
        match op {
            BlockOp::Insert {
                parent,
                after,
                block,
            } => self.insert(parent.as_ref(), after.as_ref(), block),
            BlockOp::Delete { id } => {
                let node = self.node(id)?;
                let mut stack = vec![node];
                while let Some(n) = stack.pop() {
                    if let Some(bid) = self.rev.remove(&n) {
                        self.ids.remove(&bid);
                    }
                    stack.extend(self.tree.children(n).unwrap_or_default());
                }
                self.tree.delete(node)?;
                Ok(())
            }
            BlockOp::Move { id, parent, after } => {
                let node = self.node(id)?;
                let parent = self.parent_node(parent.as_ref())?;
                let siblings = self.tree.children(parent).unwrap_or_default();
                let cur = siblings.iter().position(|&n| n == node);
                match after {
                    None => {
                        if cur != Some(0) {
                            self.tree.mov_to(node, parent, 0)?;
                        }
                    }
                    Some(a) => {
                        let a = self.node(a)?;
                        let ai = siblings.iter().position(|&n| n == a);
                        let already = matches!((cur, ai), (Some(c), Some(i)) if c == i + 1);
                        if !already {
                            self.tree.mov_after(node, a)?;
                        }
                    }
                }
                Ok(())
            }
            BlockOp::SetText { id, text, marks } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                let t = match text_of(&m) {
                    Some(t) => t,
                    None => m.insert_container("text", LoroText::new())?,
                };
                if map_str(&m, "kind").as_deref() == Some(BlockKind::Embed.as_str()) {
                    return set_text(&t, "", &[]);
                }
                if map_str(&m, "kind").as_deref() == Some(BlockKind::Query.as_str()) {
                    return set_text(&t, text, &[]);
                }
                set_text(&t, text, marks)
            }
            BlockOp::SetKind { id, kind } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                if map_str(&m, "kind").as_deref() != Some(kind.as_str()) {
                    m.insert("kind", kind.as_str())?;
                }
                if *kind == BlockKind::Embed {
                    if let Some(t) = text_of(&m) {
                        set_text(&t, "", &[])?;
                    }
                } else if *kind == BlockKind::Query {
                    if let Some(t) = text_of(&m) {
                        set_text(&t, &t.to_string(), &[])?;
                    }
                }
                Ok(())
            }
            BlockOp::SetChecked { id, checked } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                m.insert("checked", *checked)?;
                Ok(())
            }
            BlockOp::SetTarget { id, target } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                if map_str(&m, "target") != *target {
                    match target {
                        Some(t) => m.insert("target", t.as_str())?,
                        None => m.delete("target")?,
                    }
                }
                Ok(())
            }
            BlockOp::SetColor { id, color } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                let color = color
                    .as_deref()
                    .filter(|c| !c.is_empty() && *c != "default");
                if map_str(&m, "color").as_deref() != color {
                    match color {
                        Some(c) => m.insert("color", c)?,
                        None => m.delete("color")?,
                    }
                }
                Ok(())
            }
            BlockOp::SetWidth { id, width } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                let w = width.filter(|w| *w > 0 && *w < 100);
                let cur = match m.get("width") {
                    Some(ValueOrContainer::Value(LoroValue::I64(v))) => Some(v),
                    _ => None,
                };
                if cur != w.map(|w| w as i64) {
                    match w {
                        Some(w) => m.insert("width", w as i64)?,
                        None => m.delete("width")?,
                    }
                }
                Ok(())
            }
            BlockOp::SetNumberedStart { id, numbered_start } => {
                let m = self.tree.get_meta(self.node(id)?)?;
                let start = numbered_start.filter(|start| *start != 1);
                let cur = match m.get("numbered_start") {
                    Some(ValueOrContainer::Value(LoroValue::I64(n))) => Some(n),
                    _ => None,
                };
                if cur != start.map(i64::from) {
                    match start {
                        Some(start) => m.insert("numbered_start", i64::from(start))?,
                        None => m.delete("numbered_start")?,
                    }
                }
                Ok(())
            }
        }
    }

    fn insert(
        &mut self,
        parent: Option<&BlockId>,
        after: Option<&BlockId>,
        b: &NewBlock,
    ) -> Result<()> {
        let parent = self.parent_node(parent)?;
        let index = match after {
            None => 0,
            Some(a) => {
                let a = self.node(a)?;
                let siblings = self.tree.children(parent).unwrap_or_default();
                siblings
                    .iter()
                    .position(|&n| n == a)
                    .ok_or_else(|| invalid("`after` is not a child of `parent`"))?
                    + 1
            }
        };
        let node = self.tree.create_at(parent, index)?;
        let m = self.tree.get_meta(node)?;
        m.insert("id", b.id.as_str())?;
        m.insert("kind", b.kind.as_str())?;
        m.insert("checked", b.checked)?;
        let t = m.insert_container("text", LoroText::new())?;
        if b.kind == BlockKind::Embed {
            set_text(&t, "", &[])?;
        } else if b.kind == BlockKind::Query {
            set_text(&t, &b.text, &[])?;
        } else {
            set_text(&t, &b.text, &b.marks)?;
        }
        self.ids.insert(b.id.clone(), node);
        self.rev.insert(node, b.id.clone());
        Ok(())
    }
}

/// The title an embed shows: its target's live title, or empty while the
/// target is unknown. Never the target id. One line, for Markdown and search.
pub(crate) fn embed_title(target: Option<&str>, titles: &dyn Fn(&str) -> Option<String>) -> String {
    target
        .and_then(titles)
        .unwrap_or_default()
        .replace(['\r', '\n'], " ")
}

/// All block texts joined by newlines, in document order (for search). An
/// embed contributes its target's title, as drawn; queries and view settings
/// contribute no text.
pub(crate) fn plain_text(blocks: &[Block], titles: &dyn Fn(&str) -> Option<String>) -> String {
    fn walk(b: &[Block], titles: &dyn Fn(&str) -> Option<String>, out: &mut String) {
        for x in b {
            if x.kind == BlockKind::Query {
                continue;
            }
            let text = if x.kind == BlockKind::View {
                std::borrow::Cow::Borrowed("")
            } else if x.kind == BlockKind::Embed {
                std::borrow::Cow::Owned(embed_title(x.target.as_deref(), titles))
            } else {
                std::borrow::Cow::Borrowed(x.text.as_str())
            };
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&text);
            }
            walk(&x.children, titles, out);
        }
    }
    let mut out = String::new();
    walk(blocks, titles, &mut out);
    out
}

/// Batch validation state: the page as the earlier ops of the batch leave it.
struct Sim<'a> {
    pd: &'a PageDoc,
    parent: HashMap<BlockId, Option<BlockId>>,
    kind: HashMap<BlockId, BlockKind>,
    added: HashSet<BlockId>,
    removed: HashSet<BlockId>,
}

impl Sim<'_> {
    fn known(&self, id: &str) -> bool {
        self.added.contains(id) || self.pd.ids.contains_key(id)
    }

    fn parent_of(&self, id: &str) -> Option<BlockId> {
        if let Some(p) = self.parent.get(id) {
            return p.clone();
        }
        let node = *self.pd.ids.get(id)?;
        match self.pd.tree.parent(node) {
            Some(TreeParentId::Node(p)) => self.pd.rev.get(&p).cloned(),
            _ => None,
        }
    }

    fn kind_of(&self, id: &str) -> Option<BlockKind> {
        if let Some(kind) = self.kind.get(id) {
            return Some(kind.clone());
        }
        let node = *self.pd.ids.get(id)?;
        let meta = self.pd.tree.get_meta(node).ok()?;
        map_str(&meta, "kind").map(|kind| BlockKind::parse(&kind))
    }

    /// Exists and neither it nor an ancestor was deleted earlier in the batch.
    fn exists(&self, id: &str) -> bool {
        if !self.known(id) {
            return false;
        }
        let mut cur = Some(id.to_string());
        let mut depth = 0;
        while let Some(c) = cur {
            if self.removed.contains(&c) || depth > 10_000 {
                return false;
            }
            depth += 1;
            cur = self.parent_of(&c);
        }
        true
    }

    fn require(&self, id: &str) -> Result<()> {
        if self.exists(id) {
            Ok(())
        } else {
            Err(not_found("block", id))
        }
    }

    fn check_position(&self, parent: Option<&BlockId>, after: Option<&BlockId>) -> Result<()> {
        if let Some(p) = parent {
            self.require(p)?;
        }
        if let Some(a) = after {
            self.require(a)?;
            if self.parent_of(a).as_ref() != parent {
                return Err(invalid(format!(
                    "`after` block {a} is not a child of the given parent"
                )));
            }
        }
        Ok(())
    }
}

fn check_marks(text: &str, marks: &[Mark]) -> Result<()> {
    let len = text.chars().count() as u32;
    for m in marks {
        if m.start > m.end || m.end > len {
            return Err(invalid(format!(
                "mark {:?} {}..{} out of range for text of {} characters",
                m.kind, m.start, m.end, len
            )));
        }
    }
    Ok(())
}

/// The page type stored in a page's meta map. A `type` string wins; without
/// one, the old `collection: true` flag reads as a database; otherwise a
/// document.
pub(crate) fn read_page_type(m: &LoroMap) -> String {
    if let Some(t) = map_str(m, "type") {
        return t;
    }
    if matches!(
        m.get("collection"),
        Some(ValueOrContainer::Value(LoroValue::Bool(true)))
    ) {
        return crate::types::PAGE_TYPE_DATABASE.to_string();
    }
    crate::types::PAGE_TYPE_DOCUMENT.to_string()
}

fn map_str(m: &LoroMap, key: &str) -> Option<String> {
    match m.get(key) {
        Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

fn map_bool(m: &LoroMap, key: &str) -> bool {
    matches!(
        m.get(key),
        Some(ValueOrContainer::Value(LoroValue::Bool(true)))
    )
}

fn map_i64(m: &LoroMap, key: &str) -> i64 {
    map_opt_i64(m, key).unwrap_or(0)
}

fn map_opt_i64(m: &LoroMap, key: &str) -> Option<i64> {
    match m.get(key) {
        Some(ValueOrContainer::Value(LoroValue::I64(v))) => Some(v),
        Some(ValueOrContainer::Value(LoroValue::Double(v))) => Some(v as i64),
        _ => None,
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| VaultError::Invalid(e.to_string()))
}

/// Items of a map (id -> JSON) in the order of a list of ids. Duplicate or
/// dangling ids in the list are skipped; map entries missing from the list
/// (after concurrent edits) follow, sorted by id.
fn read_ordered<T: serde::de::DeserializeOwned>(doc: &LoroDoc, map: &str, order: &str) -> Vec<T> {
    let LoroValue::Map(m) = doc.get_map(map).get_value() else {
        return Vec::new();
    };
    let mut ids: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    if let LoroValue::List(l) = doc.get_list(order).get_value() {
        for v in l.iter() {
            if let LoroValue::String(s) = v {
                if m.contains_key(s.as_str()) && seen.insert(s.to_string()) {
                    ids.push(s.to_string());
                }
            }
        }
    }
    let mut rest: Vec<&String> = m.keys().filter(|k| !seen.contains(*k)).collect();
    rest.sort();
    ids.extend(rest.into_iter().cloned());
    ids.iter()
        .filter_map(|id| match m.get(id) {
            Some(LoroValue::String(s)) => serde_json::from_str(s).ok(),
            _ => None,
        })
        .collect()
}

/// Makes the map hold exactly `items` (id, JSON) and the list their order,
/// writing only what changed.
fn write_ordered(doc: &LoroDoc, map: &str, order: &str, items: &[(String, String)]) -> Result<()> {
    let m = doc.get_map(map);
    let keep: HashSet<&str> = items.iter().map(|(k, _)| k.as_str()).collect();
    if let LoroValue::Map(cur) = m.get_value() {
        for k in cur.keys() {
            if !keep.contains(k.as_str()) {
                m.delete(k)?;
            }
        }
    }
    for (k, json) in items {
        if map_str(&m, k).as_deref() != Some(json.as_str()) {
            m.insert(k, json.as_str())?;
        }
    }
    let list: LoroList = doc.get_list(order);
    let cur: Vec<String> = match list.get_value() {
        LoroValue::List(l) => l
            .iter()
            .map(|v| match v {
                LoroValue::String(s) => s.to_string(),
                _ => String::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let want: Vec<&String> = items.iter().map(|(k, _)| k).collect();
    if cur.iter().collect::<Vec<_>>() != want {
        if !cur.is_empty() {
            list.delete(0, cur.len())?;
        }
        for (i, k) in want.iter().enumerate() {
            list.insert(i, k.as_str())?;
        }
    }
    Ok(())
}

fn text_of(m: &LoroMap) -> Option<LoroText> {
    match m.get("text") {
        Some(ValueOrContainer::Container(Container::Text(t))) => Some(t),
        _ => None,
    }
}

fn mark_from_attr(key: &str, value: &LoroValue) -> Option<MarkKind> {
    match (key, value) {
        ("bold", LoroValue::Bool(true)) => Some(MarkKind::Bold),
        ("italic", LoroValue::Bool(true)) => Some(MarkKind::Italic),
        ("strike", LoroValue::Bool(true)) => Some(MarkKind::Strike),
        ("code", LoroValue::Bool(true)) => Some(MarkKind::Code),
        ("link", LoroValue::String(s)) => Some(MarkKind::Link(s.to_string())),
        ("pagelink", LoroValue::String(s)) => Some(MarkKind::PageLink(s.to_string())),
        ("query", LoroValue::String(s)) => Some(MarkKind::Query(s.to_string())),
        ("underline", LoroValue::Bool(true)) => Some(MarkKind::Underline),
        ("color", LoroValue::String(s)) => Some(MarkKind::Color(s.to_string())),
        ("date", LoroValue::String(s)) => Some(MarkKind::Date(s.to_string())),
        _ => None,
    }
}

fn kind_rank(k: &MarkKind) -> u8 {
    match k {
        MarkKind::Bold => 0,
        MarkKind::Italic => 1,
        MarkKind::Strike => 2,
        MarkKind::Code => 3,
        MarkKind::Link(_) => 4,
        MarkKind::PageLink(_) => 5,
        MarkKind::Query(_) => 6,
        MarkKind::Underline => 7,
        MarkKind::Color(_) => 8,
        MarkKind::Date(_) => 9,
    }
}

/// Sorts marks, drops empty ones and merges overlapping or touching ranges of
/// the same kind (for links: the same URL).
pub(crate) fn normalize_marks(marks: &[Mark]) -> Vec<Mark> {
    let mut v: Vec<Mark> = marks.iter().filter(|m| m.start < m.end).cloned().collect();
    v.sort_by(|a, b| {
        (kind_rank(&a.kind), &a.kind, a.start, a.end).cmp(&(
            kind_rank(&b.kind),
            &b.kind,
            b.start,
            b.end,
        ))
    });
    let mut out: Vec<Mark> = Vec::with_capacity(v.len());
    for m in v {
        if let Some(last) = out.last_mut() {
            if last.kind == m.kind && m.start <= last.end {
                last.end = last.end.max(m.end);
                continue;
            }
        }
        out.push(m);
    }
    out.sort_by_key(|m| (m.start, kind_rank(&m.kind), m.end));
    out
}

/// Text and marks (offsets in Unicode scalar values) of a Loro text. Reads
/// the rich-text value directly (cheaper than `to_delta`, which clones).
fn read_text(t: &LoroText) -> (String, Vec<Mark>) {
    let LoroValue::List(items) = t.get_richtext_value() else {
        return (t.to_string(), Vec::new());
    };
    let mut text = String::new();
    let mut marks: Vec<Mark> = Vec::new();
    let mut pos: u32 = 0;
    for item in items.iter() {
        let LoroValue::Map(m) = item else { continue };
        let Some(LoroValue::String(insert)) = m.get("insert") else {
            continue;
        };
        let len = insert.chars().count() as u32;
        if let Some(LoroValue::Map(attrs)) = m.get("attributes") {
            for (k, v) in attrs.iter() {
                if let Some(kind) = mark_from_attr(k, v) {
                    marks.push(Mark {
                        start: pos,
                        end: pos + len,
                        kind,
                    });
                }
            }
        }
        text.push_str(insert);
        pos += len;
    }
    if marks.is_empty() {
        return (text, marks);
    }
    (text, normalize_marks(&marks))
}

/// Diffs the new text into `t`, then brings its marks in line with `marks`.
fn set_text(t: &LoroText, text: &str, marks: &[Mark]) -> Result<()> {
    if t.to_string() != text {
        t.update(text, UpdateOptions::default())
            .map_err(|e| crate::error::VaultError::Crdt(format!("text update: {e:?}")))?;
    }
    let want = normalize_marks(marks);
    let (_, have) = read_text(t);
    if want == have {
        return Ok(());
    }
    let len = text.chars().count();
    // Re-mark only the keys whose ranges differ.
    let mut keys: Vec<&'static str> = Vec::new();
    for key in [
        "bold",
        "italic",
        "strike",
        "code",
        "link",
        "pagelink",
        "query",
        "underline",
        "color",
        "date",
    ] {
        let w: Vec<&Mark> = want.iter().filter(|m| m.kind.key() == key).collect();
        let h: Vec<&Mark> = have.iter().filter(|m| m.kind.key() == key).collect();
        if w != h {
            keys.push(key);
        }
    }
    for key in keys {
        if len > 0 && have.iter().any(|m| m.kind.key() == key) {
            t.unmark(0..len, key)?;
        }
        for m in want.iter().filter(|m| m.kind.key() == key) {
            let value: LoroValue = match &m.kind {
                MarkKind::Link(url) => url.as_str().into(),
                MarkKind::PageLink(id) => id.as_str().into(),
                MarkKind::Query(query) => query.as_str().into(),
                MarkKind::Color(c) => c.as_str().into(),
                MarkKind::Date(d) => d.as_str().into(),
                _ => true.into(),
            };
            t.mark(m.start as usize..m.end as usize, key, value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod page_type_tests {
    use super::*;
    use crate::types::{PAGE_TYPE_DATABASE, PAGE_TYPE_DOCUMENT};

    fn page() -> PageDoc {
        PageDoc::new_page(1, "p1", "Title", None, 0).unwrap()
    }

    #[test]
    fn a_new_page_is_a_document_and_stores_no_type() {
        let d = page();
        assert_eq!(d.meta().page_type, PAGE_TYPE_DOCUMENT);
        assert!(d.meta.get("type").is_none());
        assert!(d.schema().is_none());
    }

    #[test]
    fn a_database_stores_its_type_string() {
        let d = page();
        d.set_page_type(PAGE_TYPE_DATABASE).unwrap();
        assert_eq!(d.meta().page_type, PAGE_TYPE_DATABASE);
        assert_eq!(map_str(&d.meta, "type").as_deref(), Some("database"));
        assert!(d.meta.get("collection").is_none());
        assert_eq!(d.schema(), Some(Vec::new()));
    }

    #[test]
    fn the_old_collection_flag_reads_as_a_database() {
        let d = page();
        d.meta.insert("collection", true).unwrap();
        assert_eq!(d.meta().page_type, PAGE_TYPE_DATABASE);
        // A document drops the old flag as well as any type.
        d.set_page_type(PAGE_TYPE_DOCUMENT).unwrap();
        assert_eq!(d.meta().page_type, PAGE_TYPE_DOCUMENT);
        assert!(d.meta.get("collection").is_none());
        assert!(d.meta.get("type").is_none());
    }

    #[test]
    fn a_stored_type_wins_and_unknown_types_are_kept() {
        let d = page();
        d.meta.insert("collection", true).unwrap();
        d.meta.insert("type", "kanban").unwrap();
        assert_eq!(d.meta().page_type, "kanban");
        d.set_page_type("kanban").unwrap();
        assert_eq!(d.meta().page_type, "kanban");
        assert!(d.meta.get("collection").is_none());
        assert!(d.schema().is_none());
    }

    #[test]
    fn a_legacy_collection_snapshot_reopens_and_rebuilds_as_a_database() {
        let root = std::env::temp_dir().join(format!("legacy_collection_{}", crate::new_id()));
        let id = {
            let vault = crate::Vault::open(root.to_string_lossy().into_owned()).unwrap();
            let id = vault.create_page(None, "Legacy collection".into()).unwrap();
            vault.flush().unwrap();
            id
        };
        let path = root.join(".vault/docs").join(format!("{id}.loro"));
        let old = PageDoc::from_bytes(&std::fs::read(&path).unwrap(), &[]).unwrap();
        old.meta.insert("collection", true).unwrap();
        assert!(old.meta.get("type").is_none());
        std::fs::write(&path, old.snapshot().unwrap()).unwrap();
        let index = rusqlite::Connection::open(root.join(".vault/index.sqlite")).unwrap();
        index.pragma_update(None, "user_version", 8).unwrap();
        drop(index);
        let vault = crate::Vault::open(root.to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            vault.load_page(id.clone()).unwrap().page_type,
            PAGE_TYPE_DATABASE
        );
        let row = vault.list_children(None, 0, 10).unwrap().pop().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.page_type, PAGE_TYPE_DATABASE);
        assert!(row.is_database());
        drop(vault);
        std::fs::remove_dir_all(root).unwrap();
    }
}
