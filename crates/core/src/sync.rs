//! Sync hooks, used by `crates/sync`: version vectors, update export and
//! import, purge tombstones, conflicts and blobs. Design and limits:
//! `wiki/sync/sync-state.md`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::{Duration, SystemTime};

use loro::{EncodedBlobMode, ExportMode, Frontiers, LoroDoc, ToJson, VersionVector, ID};

use crate::attach;
use crate::doc::{PageDoc, PageMeta};
use crate::error::{invalid, not_found, Result, VaultError};
#[cfg(feature = "storage-metrics")]
use crate::storage_metrics;
use crate::ids::{new_id, now_ms, stamp_ms};
use crate::index::{ContentRow, Index, MetaRow};
use crate::order;
use crate::types::{
    AppliedResolution, Attachment, AttachmentId, BeforeState, Block, Change,
    ChangeBatch, Conflict, ConflictId, ConflictResolution, ConflictScope, ConflictSide, ConflictStatus,
    PageId, PageUpdate, PageVersion, SideState, SyncImport,
};
#[cfg(test)]
use crate::types::{BlockKind, BlockOp, NewBlock};
use crate::vault::{lock, meta_row, Inner, PageState, Vault};

/// A page file changed more recently than this is synced to disk before
/// the page's changes are exported; older files are assumed written back
/// by the OS already.
const RECENT: Duration = Duration::from_secs(120);
/// Imported new pages per index transaction.
const COLD_BATCH: usize = 64;

/// Decodes an encoded version vector (empty bytes: the empty version).
pub(crate) fn decode_vv(bytes: &[u8]) -> Result<VersionVector> {
    if bytes.is_empty() {
        return Ok(VersionVector::default());
    }
    VersionVector::decode(bytes).map_err(|e| invalid(format!("bad version vector: {e}")))
}

/// Lowercase hex SHA-256 of `bytes`: the attachment id of those bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    #[cfg(feature = "storage-metrics")]
    let _h = storage_metrics::span("hash.sha256");
    #[cfg(feature = "storage-metrics")]
    storage_metrics::add_bytes("hash.sha256", bytes.len() as u64);
    attach::sha256_hex(bytes)
}

/// Whether version `a` includes every change in version `b` (both encoded
/// version vectors as returned by `page_versions`; empty means none).
pub fn version_includes(a: &[u8], b: &[u8]) -> Result<bool> {
    Ok(decode_vv(a)?.includes_vv(&decode_vv(b)?))
}

/// A purge's causal coverage. `None` is a legacy ID-only tombstone.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PurgeRecord {
    pub page: PageId,
    pub version: Option<Vec<u8>>,
    /// The device that made the purge, when known. Forwarded unchanged; a
    /// receiving device never stamps its own id. Absent on legacy records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

/// Durable proof that a full editor document superseded a purge.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RestoreRecord {
    pub page: PageId,
    pub purge_version: Option<Vec<u8>>,
    /// The authoritative document version after restoration normalization.
    pub version: Vec<u8>,
}

/// Crash recovery for an authoritative snapshot that excludes purge-side
/// history: old logs must never be replayed after installing that snapshot.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RestoreJournal {
    pub record: RestoreRecord,
    pub bytes: Vec<u8>,
    pub installed: bool,
}

fn covers_purge(restore: &RestoreRecord, purge: &PurgeRecord) -> Result<bool> {
    if restore.page != purge.page {
        return Ok(false);
    }
    match (&restore.purge_version, &purge.version) {
        (None, None) => Ok(true),
        (Some(a), Some(b)) => version_includes(a, b),
        _ => Ok(false),
    }
}

/// The purging device two tombstones of one generation, or two concurrent
/// purges, settle on: the greater id in byte order. A missing id yields to a
/// known one. The choice ignores delivery order. A purge record carries no
/// time, so id order is the only tiebreak.
fn selected_device(known: Option<String>, incoming: Option<String>) -> Option<String> {
    known.max(incoming)
}

/// Merges an incoming purge into the active tombstone of its page and returns
/// the coverage and purging device the page keeps. A strictly newer causal
/// purge owns its id (none when fieldless). A stale causal purge keeps the
/// active tombstone. Equal coverage and concurrent coverage merge their
/// versions and select the device with `selected_device`, so a fieldless copy
/// never erases a known id. An ID-only copy keeps the coverage it finds.
pub(crate) fn merge_purge_identity(
    active: Option<Vec<u8>>,
    known: Option<String>,
    incoming: &PurgeRecord,
) -> Result<(Option<Vec<u8>>, Option<String>)> {
    let Some(new) = &incoming.version else {
        // ID-only copies never weaken a causal tombstone; the caller skips them then.
        return Ok((active, selected_device(known, incoming.device.clone())));
    };
    let Some(old) = active else {
        // Replaces a legacy ID-only tombstone or starts one.
        return Ok((Some(new.clone()), incoming.device.clone()));
    };
    let (old_vv, new_vv) = (decode_vv(&old)?, decode_vv(new)?);
    let (new_covers_old, old_covers_new) = (new_vv.includes_vv(&old_vv), old_vv.includes_vv(&new_vv));
    Ok(match (new_covers_old, old_covers_new) {
        // The same purge generation.
        (true, true) => (Some(old), selected_device(known, incoming.device.clone())),
        // Strictly newer coverage names its own purging device.
        (true, false) => (Some(new.clone()), incoming.device.clone()),
        // Stale coverage: the active tombstone already covers it.
        (false, true) => (Some(old), known),
        // Concurrent coverage: merged, with the selected device.
        (false, false) => {
            let mut vv = old_vv;
            vv.merge(&new_vv);
            (Some(vv.encode()), selected_device(known, incoming.device.clone()))
        }
    })
}

fn full_restore_doc(page: &str, bytes: &[u8]) -> Result<PageDoc> {
    let blob = LoroDoc::decode_import_blob_meta(bytes, true)?;
    if blob.mode != EncodedBlobMode::Snapshot {
        return Err(invalid(format!(
            "page {page}: purge restoration requires a full snapshot with history"
        )));
    }
    let doc = PageDoc::from_bytes(bytes, &[])?;
    if doc.doc.is_shallow() {
        return Err(invalid(format!(
            "page {page}: purge restoration requires complete history"
        )));
    }
    if doc.meta().id != page {
        return Err(invalid(format!(
            "snapshot for page {page} holds another page"
        )));
    }
    Ok(doc)
}

fn derived_sync_ops(doc: &LoroDoc, start: ID, len: usize) -> bool {
    (0..len).all(|offset| {
        let id = ID::new(start.peer, start.counter + offset as i32);
        let containers = doc.get_changed_containers_in(id, 1);
        if containers.is_empty() {
            return false;
        }
        let adopted = doc
            .get_change(id)
            .and_then(|c| c.message)
            .is_some_and(|m| matches!(m.as_ref(), "sync:adopt-orphan" | "sync:restore-purge" | "sync:conflict-metadata" | "sync:resolve-conflict" | "sync:undo-conflict"));
        containers.iter().all(|container| {
            matches!(container,
            loro::ContainerID::Root { name, .. } if name.as_ref() == "conflicts"
                || name.as_ref() == "peers" || name.as_ref() == "conflict-lifecycle"
                || name.as_ref() == "conflict-history"
                || (adopted && name.as_ref() == "page"))
        })
    })
}

/// Only derived conflict/peer metadata may be excluded from the editor
/// snapshot in addition to operations covered by the purge itself.
pub(crate) fn restore_coverage_includes(doc: &PageDoc, coverage: &VersionVector) -> bool {
    doc.doc.oplog_vv().iter().all(|(peer, &end)| {
        let start = coverage.get(peer).copied().unwrap_or(0);
        end <= start || derived_sync_ops(&doc.doc, ID::new(*peer, start), (end - start) as usize)
    })
}

/// Lifecycle choices travel in the original page document, including archives.
/// `version` covers the actual content decision; later user edits supersede it.
#[derive(serde::Serialize, serde::Deserialize)]
struct ConflictLifecycle {
    purged: bool,
    version: Vec<u8>,
    purge_version: Option<Vec<u8>>,
    /// The device whose local decision purged the archive. Replicas keep it
    /// rather than naming themselves; absent on older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    purging_device: Option<String>,
}

fn lifecycle_decision(doc: &PageDoc) -> Option<ConflictLifecycle> {
    let value = doc.doc.get_map("conflict-lifecycle").get_value().to_json_value();
    serde_json::from_str(value.get("state")?.as_str()?).ok()
}

fn record_lifecycle(
    doc: &PageDoc,
    purged: bool,
    purge_version: Option<Vec<u8>>,
    purging_device: Option<String>,
) -> Result<()> {
    doc.doc.commit();
    let decision = ConflictLifecycle {
        purged,
        version: doc.doc.oplog_vv().encode(),
        purge_version,
        purging_device,
    };
    let json = serde_json::to_string(&decision).map_err(|e| invalid(e.to_string()))?;
    doc.doc.get_map("conflict-lifecycle").insert("state", json)?;
    Ok(())
}

fn imported_only_decisions(doc: &PageDoc, before: &VersionVector) -> bool {
    doc.doc.oplog_vv().iter().all(|(peer, &end)| {
        let start = before.get(peer).copied().unwrap_or(0);
        (start..end).all(|counter| {
            let id = ID::new(*peer, counter);
            derived_sync_ops(&doc.doc, id, 1) || doc.doc.get_change(id)
                .and_then(|change| change.message)
                .is_some_and(|message| matches!(message.as_ref(), "sync:resolve-conflict" | "sync:undo-conflict"))
        })
    })
}

/// Recover the causal editor branch rather than a receiver's merged state:
/// purge-only deletions may already have arrived before the tombstone.
fn editor_branch(doc: &PageDoc, purge: &PurgeRecord) -> Result<PageDoc> {
    // A post-root editor head can still fork to a shallow document. Recover
    // from complete history so the restoration journal outlives cold cleanup.
    if doc.doc.is_shallow() {
        let complete = PageDoc::from_bytes(&doc.full_snapshot()?, &[])?;
        return editor_branch(&complete, purge);
    }
    let Some(version) = &purge.version else {
        return full_restore_doc(&purge.page, &doc.full_snapshot()?);
    };
    let purge_vv = decode_vv(version)?;
    let local = doc.doc.oplog_vv();
    let mut heads = Vec::new();
    for (peer, &end) in local.iter() {
        let covered = purge_vv.get(peer).copied().unwrap_or(0);
        let mut head = end;
        // Receiver-generated conflicts/peer registration have both branches
        // as dependencies. They are derived metadata, not an editor head.
        while head > covered && derived_sync_ops(&doc.doc, ID::new(*peer, head - 1), 1) {
            head -= 1;
        }
        if head > covered {
            heads.push(ID::new(*peer, head - 1));
        }
    }
    if heads.is_empty() {
        return Err(invalid("restore has no causally new editor operations"));
    }
    // Loro fork_at contains only history in these heads' causal past.
    doc.fork_at(&Frontiers::from(heads))
}

/// Format of conflict records written to the page document. Records without
/// a `format` key are format 1: one block per side, read by `conflict_of`.
const CONFLICT_FORMAT: u32 = 2;

/// The stored form of a conflict (the page is the document's), format 2.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ConflictRecord {
    format: u32,
    /// The stable record id; also the document key.
    id: ConflictId,
    scope: ConflictScope,
    time_ms: i64,
    sides: Vec<ConflictSide>,
    status: ConflictStatus,
    #[serde(default)]
    applied: Vec<AppliedResolution>,
}

impl ConflictRecord {
    /// The anchor block of the conflict (see `ConflictScope::anchor_id`).
    fn anchor(&self) -> String {
        self.scope.anchor_id().to_string()
    }
}

/// The stored form of a format 1 conflict: one block per side, without
/// children, `None` where that side deleted it.
#[derive(serde::Deserialize)]
struct LegacyConflictRecord {
    block: String,
    time_ms: i64,
    sides: Vec<LegacyConflictSide>,
    #[serde(default)]
    status: Option<ConflictStatus>,
    #[serde(default)]
    applied: Vec<AppliedResolution>,
}

#[derive(serde::Deserialize)]
struct LegacyConflictSide {
    device: String,
    block: Option<Block>,
}

#[cfg(test)]
fn find_block(blocks: &[Block], id: &str) -> Option<Block> {
    for b in blocks {
        if b.id == id {
            return Some(Block {
                children: Vec::new(),
                ..b.clone()
            });
        }
        if let Some(x) = find_block(&b.children, id) {
            return Some(x);
        }
    }
    None
}

/// Format of records written before format 2: one block per side, no
/// versions, no applied history, not undoable.
const LEGACY_FORMAT: u32 = 1;

/// Reads a stored conflict. Format 2 records are read as written, unless
/// they break the format's invariants (then the conflict is unreadable).
/// Records without a `format` key are format 1: `Block` scope with
/// `Present` or `Deleted` sides, no frontiers, no anchor, id = block id.
fn conflict_of(page: &str, json: &str) -> Option<Conflict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if value.get("format").is_some_and(|format| format.as_u64() != Some(1)) {
        let r: ConflictRecord = serde_json::from_value(value).ok()?;
        if r.format != CONFLICT_FORMAT || r.id.is_empty() || !consistent(&r) {
            return None;
        }
        let block = r.anchor();
        return Some(
            Conflict {
                page: page.to_string(),
                id: r.id,
                format: CONFLICT_FORMAT,
                block,
                time_ms: r.time_ms,
                scope: r.scope,
                sides: r.sides,
                status: r.status,
                applied: r.applied,
            }
            .with_legacy_blocks(),
        );
    }
    let r: LegacyConflictRecord = serde_json::from_value(value).ok()?;
    let time_ms = r.time_ms;
    let sides = r
        .sides
        .into_iter()
        .map(|s| {
            let state = match s.block {
                // Format 1 blocks never carried children.
                Some(block) => SideState::Present {
                    content: vec![block],
                },
                None => SideState::Deleted,
            };
            ConflictSide::new(s.device, time_ms, None, state, None)
        })
        .collect();
    let block = r.block;
    Some(
        Conflict {
            page: page.to_string(),
            id: block.clone(),
            format: LEGACY_FORMAT,
            block: block.clone(),
            time_ms,
            scope: ConflictScope::Block { block },
            sides,
            status: r.status.unwrap_or(ConflictStatus::Pending),
            applied: r.applied,
        }
        .with_legacy_blocks(),
    )
}

pub(crate) fn read_conflict(page: &str, json: &str) -> Option<Conflict> {
    conflict_of(page, json)
}

pub(crate) fn write_conflict(doc: &PageDoc, conflict: &Conflict) -> Result<()> {
    retain_conflict_history(doc)?;
    if conflict.format == LEGACY_FORMAT {
        let sides: Vec<_> = conflict.sides.iter().map(|side| {
            serde_json::json!({ "device": side.device, "block": side.block })
        }).collect();
        let json = serde_json::json!({ "format": LEGACY_FORMAT,
            "block": conflict.block, "time_ms": conflict.time_ms,
            "sides": sides, "status": conflict.status, "applied": conflict.applied });
        return doc.set_conflict(&conflict.id, &json.to_string());
    }
    let record = ConflictRecord {
        format: CONFLICT_FORMAT,
        id: conflict.id.clone(),
        scope: conflict.scope.clone(),
        time_ms: conflict.time_ms,
        sides: conflict.sides.clone(),
        status: conflict.status.clone(),
        applied: conflict.applied.clone(),
    };
    let json = serde_json::to_string(&record).map_err(|e| invalid(e.to_string()))?;
    doc.set_conflict(&conflict.id, &json)
}

pub(crate) fn retain_conflict_history(doc: &PageDoc) -> Result<()> {
    doc.doc.get_map("conflict-history").insert("retained", true)?;
    Ok(())
}

pub(crate) fn has_conflict_history(doc: &PageDoc) -> bool {
    doc.doc.get_map("conflict-history").get_value().to_json_value()
        .get("retained").and_then(serde_json::Value::as_bool) == Some(true)
}

/// Format 2 invariants. A side has frontiers unless it is `Purged`, which
/// has no page frontier and never gets one made up. An applied resolution's
/// `before` is empty exactly when its prior state is `Purged`. A `Resolved`
/// status repeats the last applied entry.
fn consistent(r: &ConflictRecord) -> bool {
    let sides_ok = r.sides.iter().all(|s| {
        let purged = matches!(s.state, SideState::Purged { .. });
        purged == s.frontiers.is_none()
    });
    let applied_ok = r.applied.iter().all(|a| {
        a.before.is_empty() == matches!(a.before_state, Some(BeforeState::Purged { .. }))
    });
    let status_ok = match &r.status {
        ConflictStatus::Pending => true,
        ConflictStatus::Resolved(latest) => r.applied.last() == Some(latest),
    };
    sides_ok && applied_ok && status_ok
}

/// Rows for the index mirror of a page's open conflicts, from the document's
/// (key, JSON) pairs: (id, anchor, JSON, scope).
pub(crate) fn mirror_rows(
    page: &str,
    items: &[(String, String)],
) -> Vec<(String, String, String, &'static str)> {
    items
        .iter()
        .map(|(key, json)| {
            let (anchor, scope) = match conflict_of(page, json) {
                Some(c) => (c.block.clone(), c.scope.tag()),
                None => (key.clone(), "block"),
            };
            (key.clone(), anchor, json.clone(), scope)
        })
        .collect()
}

/// A new page read from sync updates, before it is written.
struct ColdPage {
    id: PageId,
    meta: PageMeta,
    row: MetaRow,
    blocks: Vec<Block>,
    version: Vec<u8>,
    vv: Vec<u8>,
    conflicts: Vec<(String, String)>,
    /// The document file's bytes, written by the batch's one install.
    /// Cleared once installed.
    snapshot: Vec<u8>,
}

/// Captured while the page is locked; applying the index/feed effects must
/// not acquire its state again or use another SQLite connection.
pub(crate) struct ImportEffect {
    before: PageMeta,
    after: PageMeta,
    row: MetaRow,
    conflicts: Vec<(String, String)>,
    pub recorded: u32,
    pub imported: Result<()>,
    normalized: bool,
}

fn file_is_recent(p: &std::path::Path) -> bool {
    #[cfg(feature = "storage-metrics")]
    let _s = storage_metrics::span("stat.file_is_recent");
    match fs::metadata(p).and_then(|m| m.modified()) {
        Ok(t) => SystemTime::now()
            .duration_since(t)
            .map(|d| d < RECENT)
            .unwrap_or(true),
        Err(_) => false,
    }
}

impl Inner {
    /// Shared merge and conflict handling for sync imports and disk refresh.
    /// The callback persists received ops and newly generated conflict ops.
    /// Disk refresh already has received ops on disk, so it only appends the
    /// generated ones, without recursively acquiring the advisory lock.
    pub(crate) fn merge_cached_import(
        &self,
        index: &crate::index::Index,
        id: &str,
        st: &mut PageState,
        blobs: &[Vec<u8>],
        mut persist: impl FnMut(&mut PageState, &[u8], bool) -> Result<()>,
    ) -> Result<Option<ImportEffect>> {
        let before_vv = st.doc.doc.oplog_vv();
        let before_f = st.doc.doc.oplog_frontiers();
        let before = st.doc.meta();
        // Loro may apply valid ops before rejecting the rest of a blob.
        let mut imported = Ok(());
        for bytes in blobs {
            imported = st.doc.import_updates(bytes);
            if imported.is_err() {
                break;
            }
        }
        let after_vv = st.doc.doc.oplog_vv();
        self.refresh_conflict_policy_from_doc(&st.doc);
        if after_vv == before_vv {
            return imported.map(|_| None);
        }
        let peer = self.page_peer(&st.doc);
        st.doc.set_peer(peer)?;
        let incoming = st.doc.doc.export(ExportMode::updates(&before_vv))?;
        persist(st, &incoming, true)?;
        self.mark_dirty(st, id);
        let mut recorded = 0;
        let vv = st.doc.doc.oplog_vv();
        let recovered = crate::day_repair::normalize(index, &mut st.doc)?;
        st.doc.doc.commit();
        let day_normalized = st.doc.doc.oplog_vv() != vv;
        let found = if imported_only_decisions(&st.doc, &before_vv) {
            Vec::new()
        } else { crate::conflict_detect::detect(
            &mut st.doc, &before_vv, &before_f, &after_vv, &recovered,
            self.conflict_auto_resolve_cached(),
        )? };
        let has_records = !found.is_empty();
        st.doc.doc.commit();
        for conflict in found {
            recorded += u32::from(conflict.status == ConflictStatus::Pending);
            write_conflict(&st.doc, &conflict)?;
        }
        let policy = crate::conflict_policy::apply(&st.doc, self.conflict_auto_resolve_cached(), now_ms())?;
        if has_records || day_normalized || policy.changed() {
            st.doc.register_peer(&self.device)?;
            if day_normalized {
                st.doc.set_updated(stamp_ms())?;
            }
            st.doc.doc.set_next_commit_message("sync:conflict-metadata");
            st.doc.doc.commit();
            let own = st.doc.doc.export(ExportMode::updates(&vv))?;
            persist(st, &own, false)?;
        }
        // sync_meta normally performs this as an ordinary edit. Imports
        // using a captured row must normalize before capturing that row (or
        // a flush snapshot), without reacquiring page or advisory locks.
        let mut normalized = day_normalized;
        if st.doc.meta().role.as_deref() == Some("calendars")
            && crate::calendars::needs_normalization(&st.doc)
        {
            let vv = st.doc.doc.oplog_vv();
            let result = crate::calendars::normalize_document(&mut st.doc);
            st.doc.doc.commit();
            if st.doc.doc.oplog_vv() != vv {
                // Match edit's successful metadata update, and its failed
                // edit path which still logs partially applied operations.
                if result.is_ok() {
                    st.doc.register_peer(&self.device)?;
                    st.doc.set_updated(stamp_ms())?;
                    st.doc.doc.commit();
                }
                let own = st.doc.doc.export(ExportMode::updates(&vv))?;
                persist(st, &own, false)?;
                normalized = true;
            }
            result?;
        }
        Ok(Some(ImportEffect {
            before,
            after: st.doc.meta(),
            row: meta_row(&st.doc),
            conflicts: st.doc.conflicts(),
            recorded,
            imported,
            normalized,
        }))
    }

    /// Apply imported metadata, conflict rows and feed effects using the
    /// caller's connection. Page state, io and advisory locks are released.
    pub(crate) fn apply_import_index(
        &self,
        db: &Index,
        id: &str,
        effect: &ImportEffect,
        was_in_trash: bool,
    ) -> Result<()> {
        let before = &effect.before;
        let after = &effect.after;
        if effect.normalized {
            // Ordinary normalization edits emit their own change as well as
            // the surrounding import. Defer it until page locks are free.
            self.emit([Change::PageChanged(id.to_string())]);
        }
        db.update_meta(&effect.row)?;
        db.set_conflicts(id, &mirror_rows(id, &effect.conflicts))?;
        let mut changes = vec![Change::PageChanged(id.to_string())];
        if before.title != after.title {
            for src in db.link_sources(id)? {
                self.rerender(&src);
            }
        }
        if before.title != after.title
            || before.parent != after.parent
            || before.order != after.order
            || before.icon != after.icon
            || before.favorite != after.favorite
            || before.trashed != after.trashed
        {
            changes.push(Change::TreeChanged);
        }
        let now_in_trash = db.in_trash(id)?.unwrap_or(false);
        if now_in_trash && !was_in_trash {
            for (pid, md) in db.subtree(id)? {
                self.remove_md(db, &pid, md.as_deref())?;
                changes.push(Change::PageDeleted(pid));
            }
        } else if was_in_trash && !now_in_trash {
            for (pid, _) in db.subtree(id)? {
                if db.in_trash(&pid)? == Some(false) {
                    self.rerender(&pid);
                    changes.push(Change::PageCreated(pid));
                }
            }
        }
        self.emit(changes);
        // flush_page also calls this inside the writer's transaction. That
        // transaction publishes once after its whole chunk commits instead.
        if db.conn.is_autocommit() {
            self.index_committed(db, db.subtree(id)?.into_iter().map(|(id, _)| id).collect());
        }
        Ok(())
    }

    /// Decodes a new page from sync updates and prepares its document bytes.
    /// Takes the update by value: a snapshot's bytes move into the page
    /// rather than being copied. Has no filesystem side effects and runs
    /// outside the flush lock; the batch installs the bytes and block caches
    /// later.
    fn read_cold(&self, update: PageUpdate) -> Result<ColdPage> {
        #[cfg(feature = "storage-metrics")]
        let _r = storage_metrics::span("sync.read_cold");
        let PageUpdate { page: id, bytes } = update;
        let mut doc = PageDoc::from_bytes(&bytes, &[])?;
        let meta = doc.meta();
        if meta.id != id {
            return Err(invalid(format!(
                "updates for page {id} hold page {:?}",
                meta.id
            )));
        }
        let peer = self.page_peer(&doc);
        doc.set_peer(peer)?;
        let is_snapshot = matches!(
            LoroDoc::decode_import_blob_meta(&bytes, false).map(|m| m.mode),
            Ok(EncodedBlobMode::Snapshot)
        );
        let snapshot = if is_snapshot { bytes } else { doc.snapshot()? };
        let blocks = doc.blocks();
        let version = doc.version();
        self.refresh_conflict_policy_from_doc(&doc);
        Ok(ColdPage {
            row: meta_row(&doc),
            meta,
            blocks,
            version,
            vv: doc.doc.oplog_vv().encode(),
            conflicts: doc.conflicts(),
            snapshot,
            id,
        })
    }
}

impl Vault {
    /// Complete retained history, including pages whose public load is purged.
    pub(crate) fn load_conflict_history_doc(&self, page: &str) -> Result<PageDoc> {
        if self.inner.is_purged(page) {
            self.inner.read_conflict_archive(page)
        } else {
            // read_history reconstructs an available local cold prefix. An
            // externally supplied shallow snapshot has no such source and
            // retains Loro's existing historical-read limitations.
            let bytes = self.inner.read_history(page,
                |doc| doc.doc.export(ExportMode::Snapshot))??;
            let active = PageDoc::from_bytes(&bytes, &[])?;
            match self.inner.read_conflict_archive(page) {
                Ok(archive) => crate::conflict_archive::merge(page, &active, Some(&archive)),
                Err(crate::error::VaultError::NotFound(_)) => Ok(active),
                Err(error) => Err(error),
            }
        }
    }

    pub(crate) fn rebuild_conflict_archive_mirror(&self) -> Result<()> {
        let pages = self.inner.archive_page_ids()?;
        let db = lock(&self.inner.db);
        for page in pages {
            if self.inner.is_purged(&page) || !db.exists(&page)? {
                let doc = self.inner.read_conflict_archive(&page)?;
                db.set_conflicts(&page, &mirror_rows(&page, &doc.conflicts()))?;
            }
        }
        Ok(())
    }

    /// Explicit page-history restore reuses the durable restoration journal;
    /// edit the retained document so every historical frontier remains readable.
    pub(crate) fn restore_archived_page_from_history<T>(
        &self,
        page: &str,
        edit: impl FnOnce(&mut PageDoc) -> Result<T>,
    ) -> Result<T> {
        let _purge = lock(&self.inner.purge_sync);
        let purge_version = lock(&self.inner.purged).versions.get(page).cloned().flatten();
        let (result, bytes, version) = self.inner.edit_conflict_archive(page, |doc| {
            doc.doc.set_next_commit_message("sync:restore-history");
            let result = edit(doc)?;
            doc.doc.set_next_commit_message("sync:restore-history");
            doc.doc.commit();
            record_lifecycle(doc, false, purge_version.clone(), None)?;
            doc.doc.set_next_commit_message("sync:restore-history");
            doc.doc.commit();
            Ok((result, doc.snapshot()?, doc.doc.oplog_vv().encode()))
        })?;
        let record = RestoreRecord { page: page.to_owned(), purge_version, version };
        self.install_restored_snapshot(record, bytes, true)?;
        Ok(result)
    }

    /// Merge retained metadata before the ordinary delete/edit recovery path.
    fn import_conflict_archive_update(&self, update: &PageUpdate) -> Result<Option<SyncImport>> {
        let purged = self.inner.is_purged(&update.page);
        let incoming = match PageDoc::from_bytes(&update.bytes, &[]) {
            Ok(doc) => doc,
            // Hot import owns valid-prefix recovery for a partially bad blob.
            Err(_) if !purged => return Ok(None),
            Err(error) => return Err(error),
        };
        let blob = LoroDoc::decode_import_blob_meta(&update.bytes, true)?;
        // A delta alone may have pending dependencies, so its lifecycle map
        // becomes readable only after merging it with the receiver's history.
        if !purged && blob.mode == EncodedBlobMode::Snapshot
            && lifecycle_decision(&incoming).is_none() {
            return Ok(None);
        }
        let current = if purged { self.inner.read_conflict_archive(&update.page) }
            else { self.inner.read(&update.page, |doc| doc.snapshot())
                .and_then(|bytes| bytes).and_then(|bytes| PageDoc::from_bytes(&bytes, &[])) };
        let mut doc = match current {
            Ok(doc) => doc,
            Err(crate::error::VaultError::NotFound(_)) if incoming.meta().id == update.page => incoming,
            Err(crate::error::VaultError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Err(error) = doc.import_updates(&update.bytes) {
            if !purged { return Ok(None); }
            return Err(error);
        }
        if !purged {
            // Probe recent updates against the hot state first. A live or
            // obsolete lifecycle marker belongs to ordinary import and must
            // not make every subsequent delta read the cold prefix.
            let Some(decision) = lifecycle_decision(&doc) else { return Ok(None); };
            if !decision.purged
                || !restore_coverage_includes(&doc, &decode_vv(&decision.version)?) {
                return Ok(None);
            }
            if doc.doc.is_shallow() {
                let bytes = self.inner.read_history(&update.page,
                    |source| source.full_snapshot())??;
                doc = PageDoc::from_bytes(&bytes, &[])?;
                doc.import_updates(&update.bytes)?;
            }
        }
        // Ordinary permanent deletion must not start retaining a stale copy.
        // Only conflict-bearing history participates in the archive protocol.
        if purged && doc.conflicts().is_empty() && !has_conflict_history(&doc)
            && lifecycle_decision(&doc).is_none() {
            return Ok(None);
        }
        crate::conflict_archive::validate(&update.page, &doc)?;
        if let Some(decision) = lifecycle_decision(&doc) {
            if restore_coverage_includes(&doc, &decode_vv(&decision.version)?) {
                let _purge = lock(&self.inner.purge_sync);
                if decision.purged {
                    self.install_conflict_purge(&update.page, &doc)?;
                } else if self.inner.is_purged(&update.page) {
                    let purge_version = lock(&self.inner.purged).versions
                        .get(&update.page).cloned().flatten();
                    let record = RestoreRecord { page: update.page.clone(), purge_version,
                        version: doc.doc.oplog_vv().encode() };
                    self.install_restored_snapshot(record, doc.snapshot()?, true)?;
                } else {
                    return Ok(None);
                }
                return Ok(Some(SyncImport { changed: 1, ..SyncImport::default() }));
            }
        }
        if purged {
            let coverage = lock(&self.inner.purged).versions.get(&update.page).cloned().flatten();
            if coverage.as_ref().map(|v| decode_vv(v)
                .map(|vv| restore_coverage_includes(&doc, &vv))).transpose()?.unwrap_or(false) {
                self.inner.store_conflict_archive(&update.page, &doc)?;
                lock(&self.inner.db).set_conflicts(&update.page,
                    &mirror_rows(&update.page, &doc.conflicts()))?;
                return Ok(Some(SyncImport { skipped: 1, ..SyncImport::default() }));
            }
        }
        Ok(None)
    }
    /// Every page's version (an encoded Loro version vector), pages in the
    /// trash included. Compare two versions with [`version_includes`].
    pub fn page_versions(&self) -> Result<Vec<PageVersion>> {
        let rows = lock(&self.inner.db).versions()?;
        let live: HashMap<PageId, Vec<u8>> = self.inner.live_versions().into_iter().collect();
        let mut out = Vec::with_capacity(rows.len());
        for (id, vv) in rows {
            let version = match (live.get(&id), vv) {
                (Some(v), _) => v.clone(),
                (None, Some(v)) => v,
                (None, None) => match self.inner.read_doc(&id) {
                    Ok(d) => d.doc.oplog_vv().encode(),
                    Err(_) => continue,
                },
            };
            out.push(PageVersion { page: id, version });
        }
        Ok(out)
    }

    /// The page's changes that are not in `since` (an encoded version
    /// vector; empty for the whole page, which is then a snapshot). The
    /// page's files are synced to disk first when they were written
    /// recently, so changes another device has are never lost here.
    pub fn export_updates(&self, page: PageId, since: Vec<u8>) -> Result<Vec<u8>> {
        #[cfg(feature = "storage-metrics")]
        let _x = storage_metrics::span("sync.export_updates");
        let since = decode_vv(&since)?;
        if self.inner.is_purged(&page) {
            let doc = self.inner.read_conflict_archive(&page)?;
            return Ok(if since.is_empty() { doc.doc.export(ExportMode::Snapshot)? }
                else { doc.doc.export(ExportMode::updates(&since))? });
        }
        let slot = self.inner.slot(&page)?;
        let mut st = {
            #[cfg(feature = "storage-metrics")]
            let _w = storage_metrics::span("wait.slot_state");
            lock(&slot.state)
        };
        if st.deleted {
            return Err(not_found("page", &page));
        }
        if since.is_empty() || st.doc.needs_cold_export(&since) {
            self.inner.refresh_cold_state(&page, &mut st)?;
        }
        let peer = st.doc.doc.peer_id();
        let own = st.doc.doc.oplog_vv().get(&peer).copied().unwrap_or(0)
            > since.get(&peer).copied().unwrap_or(0);
        if own {
            // Unflushed changes are in the logs; flushed ones in the
            // snapshot. The next flush writes its snapshot durably.
            st.exported = true;
            let paths = [self.inner.doc_path(&page), self.inner.log_path(&page)];
            if paths.iter().any(|p| file_is_recent(p)) {
                #[cfg(feature = "storage-metrics")]
                let _f = storage_metrics::span("sync.sync_files");
                self.inner.sync_files(&page)?;
            }
        }
        let bytes = if since.is_empty() {
            #[cfg(feature = "storage-metrics")]
            let _e = storage_metrics::span("loro.export_snapshot");
            st.doc.doc.export(ExportMode::Snapshot)?
        } else {
            #[cfg(feature = "storage-metrics")]
            let _e = storage_metrics::span("loro.export_updates");
            st.doc.doc.export(ExportMode::updates(&since))?
        };
        #[cfg(feature = "storage-metrics")]
        storage_metrics::add_bytes(if since.is_empty() {
            "loro.export_snapshot"
        } else { "loro.export_updates" }, bytes.len() as u64);
        Ok(bytes)
    }

    /// Imports one page's updates from another device; see
    /// `import_updates_batch`.
    pub fn import_updates(&self, page: PageId, bytes: Vec<u8>) -> Result<SyncImport> {
        self.import_updates_batch(vec![PageUpdate { page, bytes }])
    }

    /// Imports updates from another device (as made by `export_updates`
    /// there). New pages are written with their markdown files and index
    /// rows; existing pages are merged, their files rewritten by the
    /// writer, and concurrent changes to the same block recorded as
    /// conflicts. Causally new full snapshots restore purged pages. Reports each page
    /// to the change feed.
    pub fn import_updates_batch(&self, updates: Vec<PageUpdate>) -> Result<SyncImport> {
        #[cfg(feature = "storage-metrics")]
        let _b = storage_metrics::span("sync.import_batch");
        let updated: Vec<_> = updates.iter().map(|u| u.page.clone()).collect();
        let mut report = SyncImport::default();
        let mut cold: Vec<PageUpdate> = Vec::new();
        for u in updates {
            if let Some(imported) = self.import_conflict_archive_update(&u)? {
                report.created += imported.created;
                report.changed += imported.changed;
                report.conflicts += imported.conflicts;
                report.skipped += imported.skipped;
                continue;
            }
            if self.inner.is_purged(&u.page) {
                let _purge = lock(&self.inner.purge_sync);
                if !self.inner.is_purged(&u.page) {
                    let (changed, conflicts) = self.import_hot(&u.page, &u.bytes)?;
                    report.changed += changed as u32;
                    report.conflicts += conflicts;
                    continue;
                }
                let purge = {
                    let p = lock(&self.inner.purged);
                    PurgeRecord {
                        page: u.page.clone(),
                        version: p.versions.get(&u.page).cloned().flatten(),
                        device: p.devices.get(&u.page).cloned(),
                    }
                };
                let blob = LoroDoc::decode_import_blob_meta(&u.bytes, true)?;
                if purge
                    .version
                    .as_ref()
                    .map(|vv| decode_vv(vv).map(|v| v.includes_vv(&blob.partial_end_vv)))
                    .transpose()?
                    .unwrap_or(false)
                {
                    report.skipped += 1;
                    continue;
                }
                let full = full_restore_doc(&u.page, &u.bytes)?;
                if purge
                    .version
                    .as_ref()
                    .map(|v| decode_vv(v).map(|v| restore_coverage_includes(&full, &v)))
                    .transpose()?
                    .unwrap_or(false)
                {
                    report.skipped += 1;
                    continue;
                }
                let mut doc = editor_branch(&full, &purge)?;
                self.normalize_restored_doc(&mut doc, &purge)?;
                let record = RestoreRecord {
                    page: u.page.clone(),
                    purge_version: purge.version,
                    version: doc.doc.oplog_vv().encode(),
                };
                let bytes = doc.snapshot()?;
                let r = self.install_restored_snapshot(record, bytes, false)?;
                report.created += r.created;
                report.changed += r.changed;
                report.conflicts += r.conflicts;
                report.skipped += r.skipped;
                continue;
            }
            // A document file without an index row is recovered before
            // merging, even when identical updates make no CRDT change.
            let exists =
                lock(&self.inner.db).exists(&u.page)? || self.inner.doc_path(&u.page).exists();
            if exists {
                let (changed, conflicts) = self.import_hot(&u.page, &u.bytes)?;
                report.changed += changed as u32;
                report.conflicts += conflicts;
            } else if cold.iter().any(|c| c.page == u.page) {
                // Two updates for one new page: merge them into the first.
                let first = cold.iter_mut().find(|c| c.page == u.page).expect("page");
                let doc = LoroDoc::new();
                doc.import(&first.bytes)?;
                doc.import(&u.bytes)?;
                first.bytes = doc.export(ExportMode::Snapshot)?;
            } else {
                cold.push(u);
            }
        }
        if !cold.is_empty() {
            report.created += self.import_cold(cold)?;
        }
        self.repair_imported_day_notes(&updated)?;
        self.sweep_conflict_policy()?;
        Ok(report)
    }

    /// Version-one compatibility: even causal tombstones remain delete-wins.
    pub fn import_updates_batch_legacy(&self, updates: Vec<PageUpdate>) -> Result<SyncImport> {
        let mut skipped = 0;
        let updates = updates
            .into_iter()
            .filter(|u| {
                if self.inner.is_purged(&u.page) {
                    skipped += 1;
                    false
                } else {
                    true
                }
            })
            .collect();
        let mut report = self.import_updates_batch(updates)?;
        report.skipped += skipped;
        Ok(report)
    }

    /// Complete active tombstones; offsets are unsuitable because restores
    /// remove IDs from this list.
    pub fn purge_records(&self) -> Vec<PurgeRecord> {
        let p = lock(&self.inner.purged);
        p.list
            .iter()
            .map(|page| PurgeRecord {
                page: page.clone(),
                version: p.versions.get(page).cloned().flatten(),
                device: p.devices.get(page).cloned(),
            })
            .collect()
    }

    /// Applied acknowledgements whose pages still exist. Older generations
    /// remain persisted to reject delayed tombstone replays after reopening.
    pub fn restore_records(&self) -> Vec<RestoreRecord> {
        let p = lock(&self.inner.purged);
        let mut seen = HashSet::new();
        p.restored
            .iter()
            .rev()
            .filter(|r| !p.set.contains(&r.page) && seen.insert(r.page.clone()))
            .cloned()
            .collect()
    }

    pub fn needs_restore_snapshot(&self, record: &RestoreRecord) -> Result<bool> {
        let version = decode_vv(&record.version)?;
        if let Some(v) = &record.purge_version {
            decode_vv(v)?;
        }
        let known = {
            let p = lock(&self.inner.purged);
            if p.restored.contains(record) {
                return Ok(false);
            }
            let purge = PurgeRecord {
                page: record.page.clone(),
                version: record.purge_version.clone(),
                device: None,
            };
            let mut known = false;
            for r in &p.restored {
                known |= covers_purge(r, &purge)?;
            }
            known && !p.set.contains(&record.page)
        };
        if known && self.inner.doc_path(&record.page).exists() {
            let slot = self.inner.slot(&record.page)?;
            return Ok(!lock(&slot.state).doc.doc.oplog_vv().includes_vv(&version));
        }
        Ok(true)
    }

    fn acknowledge_restore(&self, record: RestoreRecord) -> Result<()> {
        let mut p = lock(&self.inner.purged);
        let mut next = p.clone();
        next.set.remove(&record.page);
        next.list.retain(|id| id != &record.page);
        next.versions.remove(&record.page);
        next.devices.remove(&record.page);
        if !next.restored.contains(&record) {
            next.restored.push(record);
        }
        self.inner.save_purged(&next)?;
        *p = next;
        Ok(())
    }

    fn normalize_restored_doc(&self, doc: &mut PageDoc, purge: &PurgeRecord) -> Result<()> {
        let editing_version = doc.version();
        let content = doc.blocks();
        let peers = doc.peers();
        let editing_device = doc.doc.oplog_frontiers().iter()
            .max_by_key(|id| doc.doc.get_change(*id).map(|change| change.timestamp).unwrap_or(0))
            .map(|id| peers.get(&id.peer.to_string()).cloned().unwrap_or_else(|| id.peer.to_string()))
            .unwrap_or_else(|| "unknown".into());
        let time_ms = now_ms();
        // A clone can race with live edits by this device. Restoration uses
        // its own peer so it cannot reuse a concurrently assigned counter.
        doc.set_peer(crate::vault::peer_of(&new_id()))?;
        doc.register_peer(&self.inner.device)?;
        doc.set_meta_str("purge_restore", Some(&new_id()))?;
        if purge.version.is_some() {
            doc.set_meta_i64("trashed", None)?;
            doc.set_meta_str("purge_restore_warning", None)?;
        } else {
            doc.set_meta_i64("trashed", Some(stamp_ms()))?;
            doc.set_meta_str(
                "purge_restore_warning",
                Some("came back after permanent delete"),
            )?;
        }
        let parent = doc.meta().parent;
        if let Some(parent) = parent {
            let db = lock(&self.inner.db);
            if self.inner.is_purged(&parent) || db.in_trash(&parent)? != Some(false) {
                doc.set_meta_str("parent", None)?;
            }
        }
        doc.set_updated(stamp_ms())?;
        doc.doc.set_next_commit_message("sync:restore-purge");
        doc.doc.commit();
        let scope = ConflictScope::Page;
        let sides = vec![
            ConflictSide::new(editing_device, time_ms, Some(editing_version),
                SideState::Present { content }, None),
            // The purging side is the tombstone's device; fieldless records fall back.
            ConflictSide::new(purge.device.clone().unwrap_or_else(|| "unknown".into()), time_ms, None,
                SideState::Purged { tombstone_version: purge.version.clone().unwrap_or_default() }, None),
        ];
        let mut conflict = Conflict {
            page: purge.page.clone(),
            id: crate::conflict_detect::event_id(&scope, &sides)?,
            format: CONFLICT_FORMAT,
            block: String::new(), time_ms, scope, sides,
            status: ConflictStatus::Pending, applied: Vec::new(),
        };
        // Causal purge repair already applied the editing side. Legacy
        // restores deliberately remain in Trash and retain their warning.
        if purge.version.is_some() {
            conflict.applied.push(AppliedResolution {
            undone_at: None,
                time_ms,
                by: crate::types::ResolutionBy::Auto { rule: "edit-beats-delete".into() },
                before: Vec::new(),
                before_state: Some(BeforeState::Purged {
                    tombstone_version: purge.version.clone().unwrap_or_default(),
                }),
                fallback: false, restored_ancestors: Vec::new(),
                // The editing side is sides[0].
                resolution: Some(ConflictResolution::UseSide(0)),
            });
        }
        if !doc.conflicts().iter().any(|(id, _)| id == &conflict.id) {
            if self.inner.conflict_auto_resolve_cached() {
                if let Some(latest) = conflict.applied.last() {
                    conflict.status = ConflictStatus::Resolved(latest.clone());
                }
            }
            write_conflict(doc, &conflict)?;
            doc.doc.set_next_commit_message("sync:conflict");
            doc.doc.commit();
        }
        Ok(())
    }

    /// IDs carrying the persisted legacy-restoration warning in their page
    /// metadata. The marker travels with the document without bridge changes.
    pub fn restored_after_purge_pages(&self) -> Result<Vec<PageId>> {
        let mut out = Vec::new();
        for v in self.page_versions()? {
            let slot = self.inner.slot(&v.page)?;
            let st = lock(&slot.state);
            if matches!(
                st.doc.doc.get_map("page").get("purge_restore_warning"),
                Some(loro::ValueOrContainer::Value(loro::LoroValue::String(_)))
            ) {
                out.push(v.page);
            }
        }
        Ok(out)
    }

    /// Applies causal tombstones before merging ordinary page updates. An
    /// edited local page is kept whole and made live; legacy copies go to trash.
    pub fn import_purge_records(&self, mut records: Vec<PurgeRecord>) -> Result<u32> {
        let _purge = lock(&self.inner.purge_sync);
        let incoming: HashSet<PageId> = records.iter().map(|r| r.page.clone()).collect();
        // Process ancestors first, including reversed wire batches. A restored
        // child can then decide whether its original parent still exists.
        let mut depths = HashMap::new();
        {
            let db = lock(&self.inner.db);
            for r in &records {
                let mut parent = db.parent_of(&r.page)?.flatten();
                let mut seen = HashSet::new();
                let mut depth = 0;
                while let Some(id) = parent {
                    if !incoming.contains(&id) || !seen.insert(id.clone()) {
                        break;
                    }
                    depth += 1;
                    parent = db.parent_of(&id)?.flatten();
                }
                depths.insert(r.page.clone(), depth);
            }
        }
        records.sort_by_key(|r| depths.get(&r.page).copied().unwrap_or(0));
        let mut removed = 0;
        for mut record in records {
            if let Some(v) = &record.version {
                decode_vv(v)?;
            }
            // An ID-only tombstone cannot say which edits it covers, so a page
            // that was ever restored keeps its restored copy against it. Such
            // a record is ignored: nothing is removed, and nothing is stored.
            let restored_before = lock(&self.inner.purged)
                .restored
                .iter()
                .any(|r| r.page == record.page);
            if record.version.is_none() && restored_before {
                continue;
            }
            let acknowledged = {
                let p = lock(&self.inner.purged);
                let mut covered = false;
                for r in &p.restored {
                    covered |= covers_purge(r, &record)?;
                }
                covered
            };
            if acknowledged {
                continue;
            }
            // The effective tombstone is the one merge rule applied to the active
            // tombstone. It serves the restore checks below and the persisted
            // record, so a fieldless replay keeps the device the page already names.
            let incoming_record = record.clone();
            record = lock(&self.inner.purged).effect(&incoming_record)?;
            // Re-deriving the effect on persistence yields the same tombstone, so
            // an ID-only copy stays ID-only and never weakens a causal tombstone.
            let persisted = PurgeRecord {
                version: incoming_record.version.as_ref().and(record.version.clone()),
                ..record.clone()
            };
            let legacy_leftover = record.version.is_none() && self.inner.is_purged(&record.page);
            let exists = !legacy_leftover
                && (lock(&self.inner.db).exists(&record.page)?
                    || self.inner.doc_path(&record.page).exists());
            if exists {
                let slot = self.inner.slot(&record.page)?;
                let snapshot = {
                    let st = lock(&slot.state);
                    if record
                        .version
                        .as_ref()
                        .map(|v| decode_vv(v).map(|v| restore_coverage_includes(&st.doc, &v)))
                        .transpose()?
                        .unwrap_or(false)
                    {
                        None
                    } else {
                        Some(editor_branch(&st.doc, &record)?.snapshot()?)
                    }
                };
                if let Some(snapshot) = snapshot {
                    let mut doc = full_restore_doc(&record.page, &snapshot)?;
                    self.normalize_restored_doc(&mut doc, &record)?;
                    let restored = RestoreRecord {
                        page: record.page.clone(),
                        purge_version: record.version.clone(),
                        version: doc.doc.oplog_vv().encode(),
                    };
                    self.install_restored_snapshot(restored, doc.snapshot()?, true)?;
                    continue;
                }
            }
            self.inner.add_purge_records(&[persisted])?;
            let db = lock(&self.inner.db);
            let children = db.child_ids(&record.page)?;
            if db.exists(&record.page)? {
                let (md, _) = db.md(&record.page)?;
                let coverage = record
                    .version
                    .as_ref()
                    .map(|v| decode_vv(v))
                    .transpose()?
                    .ok_or_else(|| invalid("legacy local copies must restore before removal"))?;
                self.inner
                    .purge_files(&record.page, md.as_deref(), Some(&coverage))?;
                db.delete(&[record.page.clone()])?;
                self.inner.index_committed(&db, vec![record.page.clone()]);
                removed += 1;
                self.inner.emit([
                    Change::PageDeleted(record.page.clone()),
                    Change::TreeChanged,
                ]);
            }
            self.adopt_orphans(
                &db,
                &children
                    .into_iter()
                    .filter(|id| !incoming.contains(id) && !self.inner.is_purged(id))
                    .collect::<Vec<_>>(),
            )?;
            self.inner.emit_touched(&db);
        }
        Ok(removed)
    }

    /// Installs a full authoritative editor snapshot. Unknown restoration
    /// proofs replace purge-side history only when no independent edits vanish.
    pub fn import_restored_updates(
        &self,
        record: RestoreRecord,
        bytes: Vec<u8>,
    ) -> Result<SyncImport> {
        let _purge = lock(&self.inner.purge_sync);
        self.install_restored_snapshot(record, bytes, false)
    }

    fn install_restored_snapshot(
        &self,
        record: RestoreRecord,
        bytes: Vec<u8>,
        local: bool,
    ) -> Result<SyncImport> {
        let incoming = full_restore_doc(&record.page, &bytes)?;
        let incoming_vv = incoming.doc.oplog_vv();
        let proof_vv = decode_vv(&record.version)?;
        if !incoming_vv.includes_vv(&proof_vv) {
            return Err(invalid(
                "restore snapshot does not contain its authoritative version",
            ));
        }
        let (known_generation, active) = {
            let p = lock(&self.inner.purged);
            let purge = PurgeRecord {
                page: record.page.clone(),
                version: record.purge_version.clone(),
                device: None,
            };
            let mut known = false;
            for r in &p.restored {
                known |= covers_purge(r, &purge)?;
            }
            (known, p.versions.get(&record.page).cloned())
        };
        if let Some(Some(v)) = active {
            let explicit_restore = lifecycle_decision(&incoming).is_some_and(|decision| {
                !decision.purged && decode_vv(&decision.version).is_ok_and(|coverage|
                    restore_coverage_includes(&incoming, &coverage))
            });
            if !explicit_restore && restore_coverage_includes(&incoming, &decode_vv(&v)?) {
                return Ok(SyncImport {
                    skipped: 1,
                    ..SyncImport::default()
                });
            }
        }
        let exists = lock(&self.inner.db).exists(&record.page)?
            || self.inner.doc_path(&record.page).exists();
        if exists && known_generation && !self.inner.is_purged(&record.page) && !local {
            let (changed, conflicts) = self.import_hot(&record.page, &bytes)?;
            self.inner.sync_files(&record.page)?;
            self.acknowledge_restore(record)?;
            return Ok(SyncImport {
                changed: changed as u32,
                conflicts,
                ..SyncImport::default()
            });
        }
        let mut coverage = incoming_vv;
        if let Some(v) = &record.purge_version {
            coverage.merge(&decode_vv(v)?);
        }
        self.inner.replace_page_doc(&record, &bytes, &coverage)?;
        self.import_cold(vec![PageUpdate {
            page: record.page.clone(),
            bytes,
        }])?;
        self.acknowledge_restore(record.clone())?;
        self.inner.finish_restore_journal(&record.page)?;
        Ok(SyncImport {
            created: (!exists) as u32,
            changed: exists as u32,
            ..SyncImport::default()
        })
    }

    pub(crate) fn recover_restorations(&self) -> Result<()> {
        let directory = self.inner.root.join(".vault/restores");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let journal: RestoreJournal = serde_json::from_slice(&fs::read(&path)?)
                .map_err(|e| invalid(format!("bad restoration journal: {e}")))?;
            let _purge = lock(&self.inner.purge_sync);
            if journal.installed {
                let doc = self.inner.read_doc(&journal.record.page)?;
                if !doc
                    .doc
                    .oplog_vv()
                    .includes_vv(&decode_vv(&journal.record.version)?)
                {
                    return Err(invalid("installed restoration journal does not match page"));
                }
                self.import_cold(vec![PageUpdate {
                    page: journal.record.page.clone(),
                    bytes: doc.snapshot()?,
                }])?;
                self.acknowledge_restore(journal.record.clone())?;
                self.inner.finish_restore_journal(&journal.record.page)?;
            } else {
                self.install_restored_snapshot(journal.record, journal.bytes, true)?;
            }
        }
        Ok(())
    }

    /// A persisted causal purge may have raced with an edit already durable
    /// in the snapshot or either log. Recover that editor branch through the
    /// same normalization and journal as an incoming edited copy. Covered
    /// history and ID-only legacy leftovers remain permanently deleted.
    pub(crate) fn recover_purged_leftovers(&self) -> Result<()> {
        let mut records = Vec::new();
        let mut parents = HashMap::new();
        for record in self.purge_records() {
            let Some(version) = &record.version else {
                continue;
            };
            if !self.inner.doc_path(&record.page).exists() {
                continue;
            }
            let doc = self.inner.read_doc(&record.page)?;
            if !restore_coverage_includes(&doc, &decode_vv(version)?) {
                parents.insert(
                    record.page.clone(),
                    editor_branch(&doc, &record)?.meta().parent,
                );
                records.push(record);
            }
        }
        // These pages need not have index rows yet. Use their document
        // parents to recover ancestors first, preserving a live hierarchy.
        records.sort_by_key(|record| {
            let mut parent = parents.get(&record.page).cloned().flatten();
            let mut seen = HashSet::new();
            let mut depth = 0;
            while let Some(id) = parent {
                if !seen.insert(id.clone()) || !parents.contains_key(&id) {
                    break;
                }
                depth += 1;
                parent = parents.get(&id).cloned().flatten();
            }
            depth
        });
        for record in records {
            let _purge = lock(&self.inner.purge_sync);
            let (active, device) = {
                let p = lock(&self.inner.purged);
                (p.versions.get(&record.page).cloned().flatten(), p.devices.get(&record.page).cloned())
            };
            let Some(version) = active else {
                continue;
            };
            let record = PurgeRecord {
                page: record.page,
                version: Some(version.clone()),
                device,
            };
            let doc = self.inner.read_doc(&record.page)?;
            if restore_coverage_includes(&doc, &decode_vv(&version)?) {
                continue;
            }
            let mut doc = editor_branch(&doc, &record)?;
            self.normalize_restored_doc(&mut doc, &record)?;
            let restored = RestoreRecord {
                page: record.page,
                purge_version: record.version,
                version: doc.doc.oplog_vv().encode(),
            };
            self.install_restored_snapshot(restored, doc.snapshot()?, true)?;
        }
        Ok(())
    }

    /// Merges updates into an existing page. Returns (changed, conflicts).
    fn import_hot(&self, id: &str, bytes: &[u8]) -> Result<(bool, u32)> {
        #[cfg(feature = "storage-metrics")]
        let _h = storage_metrics::span("sync.import_hot");
        let inner = &self.inner;
        let db = lock(&inner.db);
        if !db.exists(id)? {
            inner.recover_missing_rows(&db, &[id.to_string()])?;
        }
        let was_in_trash = db.in_trash(id)?.unwrap_or(false);
        let slot = inner.slot(id)?;
        let mut st = {
            #[cfg(feature = "storage-metrics")]
            let _w = storage_metrics::span("wait.slot_state");
            lock(&slot.state)
        };
        if st.deleted {
            return Ok((false, 0));
        }
        let effect = {
            #[cfg(feature = "storage-metrics")]
            let _m = storage_metrics::span("sync.merge_import");
            inner.merge_sync_import(&db, id, &mut st, bytes)?
        };
        drop(st);
        let Some(effect) = effect else {
            return Ok((false, 0));
        };
        inner.apply_import_index(&db, id, &effect, was_in_trash)?;
        effect.imported.map(|_| (true, effect.recorded))
    }
    /// Writes pages that are new here. Returns how many.
    fn import_cold(&self, updates: Vec<PageUpdate>) -> Result<u32> {
        #[cfg(feature = "storage-metrics")]
        let _c = storage_metrics::span("sync.import_cold");
        let inner = &self.inner;
        // Decoding dominates, so workers decode owned groups in parallel.
        // A worker stops at its first error but keeps the pages it decoded
        // before it.
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 8)
            .min(updates.len().div_ceil(16).max(1));
        let per = updates.len().div_ceil(threads).max(1);
        let mut groups: Vec<Vec<PageUpdate>> = Vec::with_capacity(threads);
        for (i, update) in updates.into_iter().enumerate() {
            if i % per == 0 {
                groups.push(Vec::with_capacity(per));
            }
            groups.last_mut().expect("group").push(update);
        }
        let parts: Vec<(Vec<ColdPage>, Option<VaultError>)> = std::thread::scope(|sc| {
            let handles: Vec<_> = groups
                .into_iter()
                .map(|group| {
                    sc.spawn(move || {
                        let mut pages = Vec::with_capacity(group.len());
                        for update in group {
                            match inner.read_cold(update) {
                                Ok(page) => pages.push(page),
                                Err(e) => return (pages, Some(e)),
                            }
                        }
                        (pages, None)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                .collect()
        });
        let mut pages = Vec::with_capacity(per * parts.len());
        let mut decode_error = None;
        for (decoded, error) in parts {
            pages.extend(decoded);
            decode_error = decode_error.or(error);
        }
        if pages.is_empty() {
            return decode_error.map_or(Ok(0), Err);
        }
        {
            // The SQLite write transaction starts before the flush lock, as
            // the background writer does. One install writes every valid
            // snapshot under one lock acquisition.
            let db = lock(&inner.db);
            let tx = db.write_tx()?;
            let docs = pages.iter().map(|p| (p.id.as_str(), p.snapshot.as_slice()));
            // Block caches are written after the snapshots are durable and
            // before any index row commits. A cache failure therefore leaves
            // the snapshots durable and unindexed, as on master.
            let store_caches = || -> Result<()> {
                for p in &pages {
                    inner.store_block_cache(&p.id, &p.version, &p.blocks)?;
                }
                Ok(())
            };
            if let Some(error) = decode_error {
                // The valid snapshots stay durable with no index rows, so
                // open or an identical retry recovers them.
                inner.write_page_docs(docs, || {
                    store_caches()?;
                    Ok(tx.commit()?)
                })?;
                return Err(error);
            }
            // The index rows commit while the batch lock is still held, so
            // another connection sees them before any other flush can run.
            let defer = db.defer_guard();
            inner.write_page_docs(docs, || {
                store_caches()?;
                for p in &pages {
                    let content = ContentRow {
                        vv: Some(p.vv.clone()),
                        ..ContentRow::default()
                    };
                    db.upsert(&p.row, &content)?;
                    db.set_conflicts(&p.id, &mirror_rows(&p.id, &p.conflicts))?;
                }
                defer.finish()?;
                Ok(tx.commit()?)
            })?;
            inner.index_committed(&db, pages.iter().map(|p| p.id.clone()).collect());
        }
        for p in &mut pages {
            p.snapshot = Vec::new();
        }
        let new: HashSet<&str> = pages.iter().map(|p| p.id.as_str()).collect();
        // Documents and index rows came first, so titles, parents and schemas
        // of the whole batch are known when the markdown is rendered.
        // Markdown files and content, in short transactions.
        for chunk in pages.chunks(COLD_BATCH) {
            let db = lock(&inner.db);
            let tx = db.write_tx()?;
            for p in chunk {
                inner.finish_flush(&db, &p.id, &p.version, None, &p.meta, &p.row, &p.blocks)?;
            }
            tx.commit()?;
            inner.index_committed(&db, chunk.iter().map(|p| p.id.clone()).collect());
        }
        // Pages here that render these pages' titles or schemas.
        let db = lock(&inner.db);
        let mut again = HashSet::new();
        for p in &pages {
            for src in db.link_sources(&p.id)? {
                if !new.contains(src.as_str()) {
                    again.insert(src);
                }
            }
            if p.meta.page_type == crate::types::PAGE_TYPE_DATABASE {
                for c in db.child_ids(&p.id)? {
                    if !new.contains(c.as_str()) {
                        again.insert(c);
                    }
                }
            }
        }
        let orphans: Vec<PageId> = pages
            .iter()
            .filter(|p| p.meta.parent.as_deref().is_some_and(|x| inner.is_purged(x)))
            .map(|p| p.id.clone())
            .collect();
        self.adopt_orphans(&db, &orphans)?;
        drop(db);
        for id in again {
            inner.rerender(&id);
        }
        let n = pages.len() as u32;
        inner.emit(
            pages
                .into_iter()
                .map(|p| Change::PageCreated(p.id))
                .chain([Change::TreeChanged]),
        );
        Ok(n)
    }

    /// Moves pages whose parent was purged to the top level.
    fn adopt_orphans(&self, db: &Index, ids: &[PageId]) -> Result<()> {
        for id in ids {
            let key = order::between(db.last_child_key(None)?.as_deref(), None);
            self.inner.edit_sync(id, |d| {
                d.set_meta_str("parent", None)?;
                d.set_meta_str("order", Some(&key))
            })?;
            self.inner.sync_meta(db, id)?;
        }
        Ok(())
    }

    /// Active purge IDs, oldest first. A causal editor restoration removes
    /// the ID while its acknowledged purge generation stays persisted.
    pub fn purged_pages(&self) -> Vec<PageId> {
        lock(&self.inner.purged).list.clone()
    }

    /// Applies purges from another device: records the tombstones and
    /// deletes those pages here (in the trash or not; a purge wins over
    /// edits). Pages created here under a purged page move to the top
    /// level. Returns the number of pages deleted. An ID-only purge of a
    /// page with restoration history is ignored: it is a replay of an old
    /// client's tombstone, and the page stays as restored.
    pub fn import_purged(&self, pages: Vec<PageId>) -> Result<u32> {
        let _purge = lock(&self.inner.purge_sync);
        let inner = &self.inner;
        let fresh: Vec<PageId> = {
            let p = lock(&inner.purged);
            pages
                .into_iter()
                .filter(|id| !p.set.contains(id) && !p.restored.iter().any(|r| &r.page == id))
                .collect()
        };
        if fresh.is_empty() {
            return Ok(0);
        }
        inner.add_purged(&fresh)?;
        let db = lock(&inner.db);
        let mut removed = Vec::new();
        for id in &fresh {
            if db.exists(id)? {
                let (md, _) = db.md(id)?;
                inner.remove_files(id, md.as_deref())?;
                removed.push(id.clone());
            } else if let Some(s) = inner.cached_slot(id) {
                // Loaded but not indexed yet: drop it all the same.
                drop(s);
                inner.remove_files(id, None)?;
            }
        }
        let mut orphans = Vec::new();
        for id in &removed {
            for c in db.child_ids(id)? {
                if !inner.is_purged(&c) {
                    orphans.push(c);
                }
            }
        }
        db.delete(&removed)?;
        if !removed.is_empty() {
            inner.index_committed(&db, removed.clone());
        }
        inner.emit_touched(&db);
        self.adopt_orphans(&db, &orphans)?;
        drop(db);
        let n = removed.len() as u32;
        if n > 0 {
            inner.emit(
                removed
                    .into_iter()
                    .map(Change::PageDeleted)
                    .chain([Change::TreeChanged]),
            );
        }
        Ok(n)
    }

    /// `poll_changes`, waiting up to `timeout_ms` for a change after
    /// `since_seq` (returns at once if there is one). For long-polls.
    pub fn wait_changes(&self, since_seq: u32, timeout_ms: u32) -> ChangeBatch {
        self.inner
            .wait(since_seq, Duration::from_millis(timeout_ms as u64))
    }

    /// Document changes for sync transport. This cursor is independent from
    /// `poll_changes`: index commits neither wake sync nor age its history.
    /// Use the returned sequence only with these two sync-feed methods.
    pub fn poll_sync_changes(&self, since_seq: u32) -> ChangeBatch {
        self.inner.poll_documents(since_seq)
    }

    /// Wait for document changes, ignoring index-only notifications for the
    /// entire timeout. An actual edit retains the normal immediate wake.
    pub fn wait_sync_changes(&self, since_seq: u32, timeout_ms: u32) -> ChangeBatch {
        self.inner.wait_documents(since_seq, Duration::from_millis(timeout_ms as u64))
    }

    /// Open sync conflicts of one page, or of every page, by page and block.
    pub fn list_conflicts(&self, page: Option<PageId>) -> Result<Vec<Conflict>> {
        self.sweep_conflict_policy()?;
        let rows = lock(&self.inner.db).conflicts(page.as_deref())?;
        Ok(rows
            .into_iter()
            .filter_map(|(p, j)| conflict_of(&p, &j))
            .filter(|conflict| conflict.status == ConflictStatus::Pending)
            .collect())
    }

    /// Resolved conflict records retained for review and scoped undo.
    pub fn conflict_log(&self, page: Option<PageId>) -> Result<Vec<Conflict>> {
        self.sweep_conflict_policy()?;
        let rows = lock(&self.inner.db).conflicts(page.as_deref())?;
        Ok(rows.into_iter().filter_map(|(page, json)| conflict_of(&page, &json))
            .filter(|conflict| matches!(conflict.status, ConflictStatus::Resolved(_))
                || conflict.applied.iter().any(|application| application.undone_at.is_some()))
            .collect())
    }

    /// Transport versions include retained conflict history for purged pages.
    /// Ordinary `page_versions` continues to describe loadable pages.
    pub fn sync_page_versions(&self) -> Result<Vec<PageVersion>> {
        let mut versions = self.page_versions()?;
        let active: HashSet<_> = versions.iter().map(|v| v.page.clone()).collect();
        for page in self.inner.archive_page_ids()? {
            if !active.contains(&page) {
                let doc = self.inner.read_conflict_archive(&page)?;
                versions.push(PageVersion { page, version: doc.doc.oplog_vv().encode() });
            }
        }
        versions.sort_by(|left, right| left.page.cmp(&right.page));
        Ok(versions)
    }

    /// Confirm the current result or restore one historical side of a stable record.
    pub fn resolve_conflict(
        &self,
        page: PageId,
        conflict: ConflictId,
        resolution: ConflictResolution,
    ) -> Result<()> {
        self.apply_conflict_choice(page, conflict, Some(resolution))
    }

    /// Undo the latest active application once, retaining its log and history.
    pub fn undo_conflict(&self, page: PageId, conflict: ConflictId) -> Result<()> {
        self.apply_conflict_choice(page, conflict, None)
    }

    fn apply_conflict_choice(
        &self,
        page: PageId,
        id: ConflictId,
        resolution: Option<ConflictResolution>,
    ) -> Result<()> {
        use crate::conflict_resolution::PageAction;
        let _purge = lock(&self.inner.purge_sync);
        let purged = self.inner.is_purged(&page);
        let purge_version = lock(&self.inner.purged).versions.get(&page).cloned().flatten();
        let purged_by = lock(&self.inner.purged).devices.get(&page).cloned();
        let message = if resolution.is_some() { "sync:resolve-conflict" } else { "sync:undo-conflict" };
        let edit = |doc: &mut PageDoc| -> Result<(PageAction, Vec<u8>, Vec<(String, String)>)> {
            // Reconstruct before reading the record or capturing the undo
            // frontier: the checked cold source can include newer disk edits.
            doc.rehydrate()?;
            let mut conflict = doc.conflicts().into_iter()
                .find_map(|(key, json)| (key == id).then(|| conflict_of(&page, &json)).flatten())
                .ok_or_else(|| not_found("conflict", &id))?;
            doc.doc.set_next_commit_message(message);
            let action = if let Some(resolution) = &resolution {
                if conflict.status != ConflictStatus::Pending {
                    return Err(not_found("pending conflict", &id));
                }
                let before_state = (purged && matches!(conflict.scope, ConflictScope::Page)).then(|| BeforeState::Purged {
                    tombstone_version: purge_version.clone().unwrap_or_default(),
                });
                let outcome = crate::conflict_resolution::resolve(doc, &conflict, resolution,
                    crate::types::ResolutionBy::User, now_ms(), before_state)?;
                conflict.applied.push(outcome.applied.clone());
                conflict.status = ConflictStatus::Resolved(outcome.applied);
                outcome.page_action
            } else {
                let outcome = crate::conflict_resolution::undo(doc, &conflict)?;
                let journal = conflict.applied.iter().any(|application| {
                    matches!(&application.by, crate::types::ResolutionBy::Auto { rule }
                        if rule == crate::conflict_detect::JOURNAL_RULE)
                });
                let latest = conflict.applied.last_mut().expect("undo checked application");
                latest.undone_at = Some(now_ms());
                conflict.status = if journal { ConflictStatus::Resolved(latest.clone()) }
                    else { ConflictStatus::Pending };
                outcome.page_action
            };
            write_conflict(doc, &conflict)?;
            doc.doc.set_next_commit_message(message);
            doc.doc.commit();
            let remains_purged = matches!(action, PageAction::Purged { .. })
                || (purged && matches!(action, PageAction::Unchanged));
            // A purge decided here names this device. A refresh of an existing
            // purge keeps the device that made it.
            let purging_device = match action {
                PageAction::Purged { .. } => Some(self.inner.device.clone()),
                _ if remains_purged => purged_by.clone(),
                _ => None,
            };
            let superseded_purge = if let PageAction::Purged { tombstone_version } = &action {
                let mut coverage = decode_vv(tombstone_version)?;
                if let Some(active) = &purge_version {
                    coverage.merge(&decode_vv(active)?);
                }
                Some(coverage.encode())
            } else {
                purge_version.clone()
            };
            record_lifecycle(doc, remains_purged, superseded_purge, purging_device)?;
            doc.doc.set_next_commit_message(message);
            doc.doc.commit();
            Ok((action, doc.full_snapshot()?, doc.conflicts()))
        };
        let (action, bytes, conflicts) = if purged {
            self.inner.edit_conflict_archive(&page, edit)?
        } else {
            self.inner.edit_tagged(&page, Some(message), edit)?
        };
        let doc = PageDoc::from_bytes(&bytes, &[])?;
        match action {
            PageAction::Purged { .. } => self.install_conflict_purge(&page, &doc)?,
            PageAction::Present | PageAction::Trashed { .. } if purged => {
                let record = RestoreRecord { page: page.clone(), purge_version,
                    version: doc.doc.oplog_vv().encode() };
                self.install_restored_snapshot(record, bytes, true)?;
            }
            PageAction::Unchanged if purged => {
                let device = lifecycle_decision(&doc).and_then(|decision| decision.purging_device);
                self.inner.add_purge_records(&[PurgeRecord { page: page.clone(),
                    version: Some(doc.doc.oplog_vv().encode()), device }])?;
            }
            _ => {
                let db = lock(&self.inner.db);
                self.inner.sync_meta(&db, &page)?;
            }
        }
        lock(&self.inner.db).set_conflicts(&page, &mirror_rows(&page, &conflicts))?;
        Ok(())
    }

    /// Install an explicit purge with causal coverage of the synced decision.
    /// The purging device is the one the decision names, so replay keeps the
    /// origin id; a decision without one (older records) names none.
    fn install_conflict_purge(&self, page: &str, doc: &PageDoc) -> Result<()> {
        self.inner.store_conflict_archive(page, doc)?;
        let decision = lifecycle_decision(doc);
        let mut coverage = doc.doc.oplog_vv();
        // Restoration excludes the old purge branch from the document. An
        // explicit successor still acknowledges that tombstone's coverage.
        if let Some(previous) = decision.as_ref().and_then(|state| state.purge_version.as_deref()) {
            coverage.merge(&decode_vv(previous)?);
        }
        let device = decision.and_then(|decision| decision.purging_device);
        self.inner.add_purge_records(&[PurgeRecord { page: page.to_owned(),
            version: Some(coverage.encode()), device }])?;
        let db = lock(&self.inner.db);
        let children = db.child_ids(page)?;
        if db.exists(page)? || self.inner.doc_path(page).exists() {
            let (md, _) = db.md(page)?;
            self.inner.purge_files(page, md.as_deref(), Some(&coverage))?;
            db.delete(&[page.to_owned()])?;
            self.inner.index_committed(&db, vec![page.to_owned()]);
            self.inner.emit([Change::PageDeleted(page.to_owned()), Change::TreeChanged]);
            self.adopt_orphans(&db, &children)?;
        }
        db.set_conflicts(page, &mirror_rows(page, &doc.conflicts()))?;
        Ok(())
    }
    /// Ids of the attachments stored here.
    pub fn attachment_ids(&self) -> Result<Vec<AttachmentId>> {
        Ok(attach::list_blobs(&self.inner.root)?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
    }

    /// Attachments that pages use but that are not stored here (not
    /// fetched from the hub yet). Flushes first.
    pub fn missing_attachments(&self) -> Result<Vec<AttachmentId>> {
        self.flush()?;
        let used = lock(&self.inner.db).referenced_blobs()?;
        let mut out: Vec<AttachmentId> = used
            .into_iter()
            .filter(|id| {
                attach::is_attachment_id(id) && !attach::blob_path(&self.inner.root, id).exists()
            })
            .collect();
        out.sort();
        Ok(out)
    }

    /// Stores an attachment received from another device under its id,
    /// which must be the SHA-256 of `bytes`.
    pub fn store_attachment(&self, id: AttachmentId, name: String, bytes: Vec<u8>) -> Result<()> {
        if sha256_hex(&bytes) != id {
            return Err(invalid(format!("attachment {id}: the bytes do not match")));
        }
        let (id, size) = attach::store_bytes(&self.inner.root, &bytes)?;
        let name = if name.is_empty() { id.clone() } else { name };
        lock(&self.inner.db).put_attachment(
            &Attachment {
                mime: attach::mime_of(&name).to_string(),
                id,
                name,
                size,
            },
            now_ms(),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod public_resolution_regressions {
    use super::*;
    use crate::types::{Anchor, ResolutionBy};

    fn insert(id: &str, value: &str) -> BlockOp {
        BlockOp::Insert { parent: None, after: None,
            block: NewBlock { id: id.into(), kind: BlockKind::Paragraph,
                text: value.into(), marks: vec![], checked: false } }
    }

    #[test]
    fn legacy_keep_and_side_choice_remain_readable_after_reopen() {
        let path = std::env::temp_dir().join(format!("legacy_conflict_pick_{}", new_id()));
        {
            let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
            let page = vault.create_page(None, "Legacy".into()).unwrap();
            vault.apply(page.clone(), vec![insert("keep", "Merged"), insert("side", "Merged")]).unwrap();
            vault.inner.edit(&page, |doc| {
                for id in ["keep", "side"] {
                    let block = find_block(&doc.blocks(), id).unwrap();
                    let mut chosen = block.clone(); chosen.text = "Chosen".into();
                    let json = serde_json::json!({"block": id, "time_ms": now_ms(),
                        "sides": [{"device": "left", "block": block},
                            {"device": "right", "block": chosen}]}).to_string();
                    doc.set_conflict(id, &json)?;
                }
                Ok(())
            }).unwrap();
            vault.resolve_conflict(page.clone(), "keep".into(), ConflictResolution::Keep).unwrap();
            vault.resolve_conflict(page.clone(), "side".into(), ConflictResolution::UseSide(1)).unwrap();
            assert_eq!(vault.conflict_log(Some(page.clone())).unwrap().len(), 2);
            assert_eq!(find_block(&vault.load_page(page).unwrap().blocks, "side").unwrap().text, "Chosen");
            vault.flush().unwrap();
        }
        {
            let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
            let records = vault.conflict_log(None).unwrap();
            assert_eq!(records.len(), 2);
            assert!(records.iter().all(|record| record.format == 1 && !record.undoable()));
            assert!(records.iter().all(|record| matches!(record.status, ConflictStatus::Resolved(_))));
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn public_pick_logs_fallback_when_no_history_can_rebuild_the_anchor() {
        let path = std::env::temp_dir().join(format!("fallback_conflict_pick_{}", new_id()));
        {
            let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
            let page = vault.create_page(None, "Fallback".into()).unwrap();
            vault.apply(page.clone(), vec![insert("target", "Chosen")]).unwrap();
            vault.inner.edit(&page, |doc| {
                doc.doc.commit();
                let side = ConflictSide::new("history".into(), now_ms(), Some(doc.version()),
                    SideState::Present { content: doc.blocks() },
                    Some(Anchor { parent: Some("never-existed".into()), previous: None }));
                let conflict = Conflict { page: page.clone(), id: "fallback".into(), format: 2,
                    block: "target".into(), time_ms: now_ms(),
                    scope: ConflictScope::Block { block: "target".into() },
                    sides: vec![side], status: ConflictStatus::Pending, applied: vec![] };
                doc.apply(&[BlockOp::Delete { id: "target".into() }])?;
                write_conflict(doc, &conflict)
            }).unwrap();
            vault.resolve_conflict(page.clone(), "fallback".into(), ConflictResolution::UseSide(0)).unwrap();
            let log = vault.conflict_log(Some(page.clone())).unwrap();
            assert!(log[0].applied[0].fallback);
            assert_eq!(log[0].applied[0].by, ResolutionBy::User);
            assert!(log[0].applied[0].restored_ancestors.is_empty());
            assert_eq!(vault.load_page(page).unwrap().blocks[0].id, "target");
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod conflict_format_tests {
    use super::*;
    use crate::types::{Anchor, AppliedResolution, BeforeState, ConflictResolution, ResolutionBy};

    fn block(id: &str, text: &str, children: Vec<Block>) -> Block {
        Block {
            id: id.into(),
            kind: BlockKind::Paragraph,
            text: text.into(),
            marks: Vec::new(),
            checked: false,
            children,
            target: None,
            color: None,
            width: None,
            numbered_start: None,
        }
    }

    /// A side. A purged side has no page frontiers, so it gets none.
    fn side(device: &str, state: SideState, anchor: Option<Anchor>) -> ConflictSide {
        let frontiers = match state {
            SideState::Purged { .. } => None,
            _ => Some(vec![1, 2, 3]),
        };
        ConflictSide::new(device.into(), 42, frontiers, state, anchor)
    }

    fn applied(time_ms: i64, by: ResolutionBy, before: Vec<u8>) -> AppliedResolution {
        AppliedResolution {
            undone_at: None,
            time_ms,
            by,
            before,
            before_state: None,
            fallback: false,
            restored_ancestors: Vec::new(),
            resolution: None,
        }
    }

    /// Format 2 records covering every scope, side state, anchor, status and
    /// applied-history shape. Each record's id is distinct.
    fn variants() -> Vec<ConflictRecord> {
        let present = SideState::Present {
            content: vec![block("s", "one", vec![block("c", "child", Vec::new())])],
        };
        let anchor = Some(Anchor {
            parent: Some("p".into()),
            previous: None,
        });
        let latest = applied(20, ResolutionBy::User, vec![4]);
        vec![
            ConflictRecord {
                format: CONFLICT_FORMAT,
                id: "r-block".into(),
                scope: ConflictScope::Block { block: "b".into() },
                time_ms: 5,
                sides: vec![
                    side("a", SideState::Present { content: vec![block("b", "x", Vec::new())] }, None),
                    side("b", SideState::Deleted, None),
                ],
                status: ConflictStatus::Pending,
                applied: Vec::new(),
            },
            ConflictRecord {
                format: CONFLICT_FORMAT,
                id: "r-subtree".into(),
                scope: ConflictScope::Subtree { root: "s".into() },
                time_ms: 6,
                sides: vec![side("a", present.clone(), anchor.clone()), side("b", SideState::Deleted, None)],
                status: ConflictStatus::Pending,
                applied: Vec::new(),
            },
            ConflictRecord {
                format: CONFLICT_FORMAT,
                id: "r-page".into(),
                scope: ConflictScope::Page,
                time_ms: 7,
                sides: vec![
                    side("a", SideState::Trashed { at: 99 }, None),
                    side("b", SideState::Purged { tombstone_version: vec![9, 8] }, None),
                ],
                status: ConflictStatus::Pending,
                applied: Vec::new(),
            },
            ConflictRecord {
                format: CONFLICT_FORMAT,
                id: "r-resolved".into(),
                scope: ConflictScope::Page,
                time_ms: 8,
                sides: vec![side("a", present, None), side("b", SideState::Deleted, None)],
                status: ConflictStatus::Resolved(latest.clone()),
                applied: vec![latest],
            },
            ConflictRecord {
                format: CONFLICT_FORMAT,
                id: "r-purged".into(),
                scope: ConflictScope::Block { block: "b".into() },
                time_ms: 9,
                sides: vec![side("a", SideState::Deleted, None), side("b", SideState::Deleted, None)],
                // Applied while the page was purged: no frontiers, so `before`
                // is empty and `before_state` says why.
                status: ConflictStatus::Pending,
                applied: vec![AppliedResolution {
                    before_state: Some(BeforeState::Purged {
                        tombstone_version: vec![7],
                    }),
                    ..applied(10, ResolutionBy::Auto { rule: "journal".into() }, Vec::new())
                }],
            },
        ]
    }

    #[test]
    fn format_2_round_trips_every_scope_state_and_status() {
        for record in variants() {
            let json = serde_json::to_string(&record).unwrap();
            let back = conflict_of("page-1", &json).unwrap();
            assert_eq!(back.page, "page-1");
            assert_eq!(back.id, record.id);
            assert_eq!(back.format, CONFLICT_FORMAT);
            assert_eq!(back.undoable(), !record.applied.is_empty());
            assert_eq!(back.block, record.anchor());
            assert_eq!(back.scope, record.scope);
            assert_eq!(back.time_ms, record.time_ms);
            assert_eq!(back.status, record.status);
            assert_eq!(back.applied, record.applied);
            assert_eq!(back.sides.len(), record.sides.len());
            for (got, want) in back.sides.iter().zip(&record.sides) {
                assert_eq!(got.device, want.device);
                assert_eq!(got.time_ms, want.time_ms);
                assert_eq!(got.frontiers, want.frontiers);
                assert_eq!(got.state, want.state);
                assert_eq!(got.anchor, want.anchor);
            }
        }
    }

    #[test]
    fn block_scope_exposes_the_legacy_block_view_and_nothing_else() {
        let records = variants();
        let block_json = serde_json::to_string(&records[0]).unwrap();
        let block_conflict = conflict_of("p", &block_json).unwrap();
        assert_eq!(block_conflict.sides[0].block, Some(block("b", "x", Vec::new())));
        assert_eq!(block_conflict.sides[1].block, None);

        let subtree_json = serde_json::to_string(&records[1]).unwrap();
        let subtree = conflict_of("p", &subtree_json).unwrap();
        assert_eq!(subtree.block, "s");
        assert!(subtree.sides.iter().all(|s| s.block.is_none()));
    }

    #[test]
    fn format_1_records_read_as_block_scope_without_version() {
        let edited = block("b1", "mine", Vec::new());
        let json = serde_json::json!({
            "block": "b1",
            "time_ms": 11,
            "sides": [
                { "device": "dev-a", "block": serde_json::to_value(&edited).unwrap() },
                { "device": "dev-b", "block": null },
            ],
        })
        .to_string();
        let c = conflict_of("p", &json).unwrap();
        assert_eq!(c.block, "b1");
        assert_eq!(c.id, "b1", "a format 1 record's id is its block");
        assert_eq!(c.format, LEGACY_FORMAT);
        assert!(!c.undoable(), "a format 1 record is never undoable");
        assert!(c.applied.is_empty(), "format 1 has no versions or history");
        assert_eq!(c.scope, ConflictScope::Block { block: "b1".into() });
        assert_eq!(c.status, ConflictStatus::Pending);
        assert_eq!(c.sides[0].device, "dev-a");
        assert_eq!(c.sides[0].time_ms, 11);
        assert_eq!(c.sides[0].frontiers, None);
        assert_eq!(c.sides[0].anchor, None);
        assert_eq!(
            c.sides[0].state,
            SideState::Present { content: vec![edited.clone()] }
        );
        assert_eq!(c.sides[0].block, Some(edited));
        assert_eq!(c.sides[1].device, "dev-b");
        assert_eq!(c.sides[1].state, SideState::Deleted);
        assert_eq!(c.sides[1].block, None);
    }

    #[test]
    fn unknown_or_damaged_records_are_unreadable() {
        let future = r#"{"format":3,"scope":"Page","time_ms":1,"sides":[],"status":"Pending"}"#;
        assert!(conflict_of("p", future).is_none());
        assert!(conflict_of("p", "not json").is_none());
        assert!(conflict_of("p", r#"{"format":2}"#).is_none());

        // Each format 2 invariant, broken on an otherwise valid record.
        let value = |i: usize| serde_json::to_value(&variants()[i]).unwrap();
        let mut empty_id = value(0);
        empty_id["id"] = "".into();
        let mut purged_with_frontiers = value(2);
        purged_with_frontiers["sides"][1]["frontiers"] = serde_json::json!([1]);
        let mut live_side_without_frontiers = value(0);
        live_side_without_frontiers["sides"][0]["frontiers"] = serde_json::Value::Null;
        let mut live_before_empty = value(3);
        live_before_empty["applied"][0]["before"] = serde_json::json!([]);
        let mut resolved_not_latest = value(3);
        resolved_not_latest["status"]["Resolved"]["time_ms"] = 999.into();
        for broken in [
            empty_id,
            purged_with_frontiers,
            live_side_without_frontiers,
            live_before_empty,
            resolved_not_latest,
        ] {
            assert!(conflict_of("p", &broken.to_string()).is_none(), "{broken}");
        }
    }

    /// Rule-applied and awaiting OK: the record stays Pending and keeps the
    /// applied resolution, with its fallback and restored ancestors.
    #[test]
    fn applied_pending_round_trips_with_its_history() {
        let mut pending = variants()[0].clone();
        pending.applied = vec![AppliedResolution {
            before_state: Some(BeforeState::Trashed { at: 30 }),
            fallback: true,
            restored_ancestors: vec!["a".into(), "b".into()],
            ..applied(25, ResolutionBy::Auto { rule: "journal".into() }, vec![1, 2])
        }];
        let back = conflict_of("p", &serde_json::to_string(&pending).unwrap()).unwrap();
        assert_eq!(back.status, ConflictStatus::Pending);
        assert_eq!(back.applied, pending.applied);

        // The Purged variant: no frontiers, and `before_state` says why.
        let purged = variants().remove(4);
        let back = conflict_of("p", &serde_json::to_string(&purged).unwrap()).unwrap();
        assert_eq!(back.status, ConflictStatus::Pending);
        assert_eq!(back.applied, purged.applied);
        assert!(back.applied[0].before.is_empty());
    }

    /// A resolved record keeps every application, oldest first, and repeats
    /// the latest as its status.
    #[test]
    fn resolved_history_keeps_every_application_and_repeats_the_latest() {
        let first = applied(20, ResolutionBy::Auto { rule: "journal".into() }, vec![1]);
        let latest = AppliedResolution {
            undone_at: None,
            restored_ancestors: vec!["a".into(), "b".into()],
            ..applied(30, ResolutionBy::User, vec![5])
        };
        let mut record = variants()[0].clone();
        record.status = ConflictStatus::Resolved(latest.clone());
        record.applied = vec![first.clone(), latest.clone()];
        let back = conflict_of("p", &serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(back.applied, vec![first, latest.clone()]);
        assert_eq!(back.status, ConflictStatus::Resolved(latest));
    }

    /// Fields added after format 2 shipped default when absent: no history,
    /// no anchor, no restored ancestors, not a fallback, live `before_state`.
    #[test]
    fn new_fields_default_when_absent() {
        let json = r#"{"format":2,"id":"r1","scope":"Page","time_ms":3,
            "sides":[
              {"device":"a","time_ms":3,"frontiers":[1],"state":"Deleted"},
              {"device":"b","time_ms":3,"frontiers":[2],"state":"Deleted"}],
            "status":"Pending"}"#;
        let c = conflict_of("p", json).unwrap();
        assert!(c.applied.is_empty());
        assert_eq!(c.scope, ConflictScope::Page);
        assert_eq!(c.sides[0].anchor, None);

        let with_applied = r#"{"format":2,"id":"r1","scope":"Page","time_ms":3,
            "sides":[
              {"device":"a","time_ms":3,"frontiers":[1],"state":"Deleted"},
              {"device":"b","time_ms":3,"frontiers":[2],"state":"Deleted"}],
            "status":"Pending",
            "applied":[{"time_ms":4,"by":"User","before":[1]}]}"#;
        let c = conflict_of("p", with_applied).unwrap();
        assert_eq!(
            c.applied,
            vec![AppliedResolution {
            undone_at: None,
                time_ms: 4,
                by: ResolutionBy::User,
                before: vec![1],
                before_state: None,
                fallback: false,
                restored_ancestors: Vec::new(),
                resolution: None,
            }]
        );
    }

    #[test]
    fn resolution_and_purging_device_default_to_none_and_round_trip() {
        let old: AppliedResolution =
            serde_json::from_str(r#"{"time_ms":4,"by":"User","before":[1]}"#).unwrap();
        assert_eq!(old.resolution, None);
        for resolution in [ConflictResolution::Keep, ConflictResolution::UseSide(1)] {
            let applied = AppliedResolution { resolution: Some(resolution), ..old.clone() };
            let json = serde_json::to_string(&applied).unwrap();
            assert_eq!(serde_json::from_str::<AppliedResolution>(&json).unwrap(), applied);
        }

        let legacy: PurgeRecord = serde_json::from_str(r#"{"page":"p","version":null}"#).unwrap();
        assert_eq!(legacy.device, None);
        let json = serde_json::to_string(&legacy).unwrap();
        assert!(!json.contains("device"), "fieldless records stay fieldless: {json}");
        let stamped = PurgeRecord { device: Some("dev-a".into()), ..legacy };
        let json = serde_json::to_string(&stamped).unwrap();
        assert_eq!(serde_json::from_str::<PurgeRecord>(&json).unwrap(), stamped);
    }

    #[test]
    fn mirror_rows_carry_the_id_anchor_and_scope_of_each_record() {
        let legacy = serde_json::json!({
            "block": "b1",
            "time_ms": 1,
            "sides": [{ "device": "dev-a", "block": null }],
        })
        .to_string();
        let subtree = serde_json::to_string(&variants()[1]).unwrap();
        let rows = mirror_rows(
            "p",
            &[("b1".to_string(), legacy), ("r-subtree".to_string(), subtree)],
        );
        assert_eq!(rows[0].0, "b1");
        assert_eq!(rows[0].1, "b1");
        assert_eq!(rows[0].3, "block");
        assert_eq!(rows[1].0, "r-subtree");
        assert_eq!(rows[1].1, "s");
        assert_eq!(rows[1].3, "subtree");
        assert_eq!(ConflictScope::Page.anchor_id(), "");
        assert_eq!(ConflictScope::Page.tag(), "page");
    }

    fn coverage(entries: &[(u64, i32)]) -> Vec<u8> {
        let mut vv = VersionVector::new();
        for &(peer, end) in entries {
            vv.set_end(ID::new(peer, end));
        }
        vv.encode()
    }

    fn purge(version: Option<Vec<u8>>, device: Option<&str>) -> PurgeRecord {
        PurgeRecord { page: "p".into(), version, device: device.map(str::to_owned) }
    }

    #[test]
    fn newer_purge_takes_its_own_device_and_a_fieldless_newer_purge_takes_none() {
        let (old, newer) = (coverage(&[(7, 2)]), coverage(&[(7, 5)]));
        let (version, device) = merge_purge_identity(
            Some(old.clone()),
            Some("a".into()),
            &purge(Some(newer.clone()), Some("b")),
        )
        .unwrap();
        assert_eq!(version, Some(newer.clone()));
        assert_eq!(device.as_deref(), Some("b"));
        let (_, device) =
            merge_purge_identity(Some(old), Some("a".into()), &purge(Some(newer), None)).unwrap();
        assert_eq!(device, None, "a fieldless newer purge must not inherit the old device");
    }

    #[test]
    fn same_purge_duplicates_and_stale_replays_keep_the_known_device() {
        let old = coverage(&[(7, 2)]);
        let (_, device) = merge_purge_identity(
            Some(old.clone()),
            Some("a".into()),
            &purge(Some(old.clone()), None),
        )
        .unwrap();
        assert_eq!(device.as_deref(), Some("a"));
        // A copy of the same purge that names a device teaches an unknown one.
        let (_, device) = merge_purge_identity(
            Some(old.clone()),
            None,
            &purge(Some(old.clone()), Some("b")),
        )
        .unwrap();
        assert_eq!(device.as_deref(), Some("b"));
        // Stale coverage from an older purge never takes over the newer identity.
        let newer = coverage(&[(7, 5)]);
        let (version, device) = merge_purge_identity(
            Some(newer.clone()),
            Some("a".into()),
            &purge(Some(old), Some("b")),
        )
        .unwrap();
        assert_eq!(version, Some(newer));
        assert_eq!(device.as_deref(), Some("a"));
    }

    #[test]
    fn legacy_tombstones_hand_over_to_causal_purges_without_inheriting() {
        let (version, device) =
            merge_purge_identity(None, Some("a".into()), &purge(None, None)).unwrap();
        assert_eq!(version, None);
        assert_eq!(device.as_deref(), Some("a"));
        let causal = coverage(&[(7, 3)]);
        let (version, device) = merge_purge_identity(
            None,
            Some("a".into()),
            &purge(Some(causal.clone()), None),
        )
        .unwrap();
        assert_eq!(version, Some(causal));
        assert_eq!(device, None);
    }

    #[test]
    fn concurrent_purges_select_the_greater_device_in_either_order() {
        let (left, right) = (coverage(&[(7, 2)]), coverage(&[(8, 3)]));
        for (first, second) in [("a", "b"), ("b", "a")] {
            let (version, device) = merge_purge_identity(
                Some(left.clone()),
                Some(first.into()),
                &purge(Some(right.clone()), Some(second)),
            )
            .unwrap();
            let merged = decode_vv(&version.unwrap()).unwrap();
            assert!(merged.includes_vv(&decode_vv(&left).unwrap()));
            assert!(merged.includes_vv(&decode_vv(&right).unwrap()));
            assert_eq!(device.as_deref(), Some("b"), "first {first}, second {second}");
        }
    }

    #[test]
    fn a_missing_device_yields_to_a_known_one_and_equal_coverage_selects_the_greater() {
        let (left, right) = (coverage(&[(7, 2)]), coverage(&[(8, 3)]));
        let (_, device) =
            merge_purge_identity(Some(left.clone()), Some("a".into()), &purge(Some(right.clone()), None))
                .unwrap();
        assert_eq!(device.as_deref(), Some("a"));
        let (_, device) =
            merge_purge_identity(Some(left.clone()), None, &purge(Some(right), Some("b"))).unwrap();
        assert_eq!(device.as_deref(), Some("b"));
        for (first, second) in [("a", "b"), ("b", "a")] {
            let (_, device) = merge_purge_identity(
                Some(left.clone()),
                Some(first.into()),
                &purge(Some(left.clone()), Some(second)),
            )
            .unwrap();
            assert_eq!(device.as_deref(), Some("b"), "equal coverage, first {first}");
        }
    }

    #[test]
    fn several_concurrent_purgers_settle_in_every_delivery_order() {
        let records = [
            (coverage(&[(7, 2)]), "dev-c"),
            (coverage(&[(8, 3)]), "dev-a"),
            (coverage(&[(9, 1)]), "dev-b"),
        ];
        let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        for order in orders {
            let mut state: (Option<Vec<u8>>, Option<String>) = (None, None);
            for i in order {
                let (cov, device) = &records[i];
                state = merge_purge_identity(
                    state.0,
                    state.1,
                    &purge(Some(cov.clone()), Some(*device)),
                )
                .unwrap();
            }
            assert_eq!(state.1.as_deref(), Some("dev-c"), "order {order:?}");
            let merged = decode_vv(state.0.as_ref().unwrap()).unwrap();
            for (cov, _) in &records {
                assert!(merged.includes_vv(&decode_vv(cov).unwrap()), "order {order:?}");
            }
        }
    }

    #[test]
    fn a_strictly_newer_purge_owns_its_id_over_concurrent_history() {
        let (left, right) = (coverage(&[(7, 2)]), coverage(&[(8, 3)]));
        let newest = coverage(&[(7, 2), (8, 3), (9, 1)]);
        let (_, device) = merge_purge_identity(
            Some(newest.clone()),
            Some("a".into()),
            &purge(Some(left.clone()), Some("b")),
        )
        .unwrap();
        assert_eq!(device.as_deref(), Some("a"), "stale left does not take over");
        let (_, device) = merge_purge_identity(
            Some(right),
            Some("b".into()),
            &purge(Some(newest), Some("z")),
        )
        .unwrap();
        assert_eq!(device.as_deref(), Some("z"), "newer purge owns its id");
    }

    #[test]
    fn lifecycle_records_the_purging_device_and_old_records_read_none() {
        let old = r#"{"purged":true,"version":[],"purge_version":null}"#;
        let decision: ConflictLifecycle = serde_json::from_str(old).unwrap();
        assert!(decision.purging_device.is_none());
        let stamped = ConflictLifecycle {
            purged: true,
            version: vec![],
            purge_version: None,
            purging_device: Some("dev-a".into()),
        };
        let json = serde_json::to_string(&stamped).unwrap();
        let read: ConflictLifecycle = serde_json::from_str(&json).unwrap();
        assert_eq!(read.purging_device.as_deref(), Some("dev-a"));
        let fieldless = serde_json::to_string(&ConflictLifecycle { purging_device: None, ..stamped })
            .unwrap();
        assert!(!fieldless.contains("purging_device"), "{fieldless}");
    }
}
