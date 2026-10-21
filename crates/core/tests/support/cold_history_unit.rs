//! Cold history regression tests. This file is included as a child module of
//! `vault.rs` (`#[path = "../tests/support/cold_history_unit.rs"]`, gated by
//! `#[cfg(test)]` there), so it can reach `Inner`, `ColdPolicy`, `PageDoc` and
//! the private `cold_page`, `cold_pass` and `stage_cold_hot`. It lives under
//! `tests/support/` because cargo compiles every file directly in `tests/` as
//! an integration test, and this one only compiles inside the crate.
//!
//! Fixtures are small temp vaults with injected Loro commit timestamps. The
//! history device creates each page with a change dated 101 days before the
//! test starts, then makes three edits dated 100, 95 and 40 days back. The
//! cold cutoff is 30 days back, so those four changes are a historical prefix.
//! The vault adds one recent block with the real clock. Pages are imported as
//! new pages, so no recent creation change can stop the cold root early.
//! The cold-policy file sets idle_seconds to 3600 so the writer's own cold
//! pass never runs during a test.

use super::*;
use crate::types::{NewBlock, Version};
use loro::{Frontiers, LoroDoc, LoroError, LoroValue, VersionVector};

const DAY: i64 = 86_400;
/// Devices: the history writer, an offline device and a post-root device.
const PEER_H: u64 = 0x1001;
const PEER_D: u64 = 0x2001;
const PEER_B: u64 = 0x3001;

struct TempVault(PathBuf);

impl TempVault {
    /// A vault whose writer never runs a cold pass on its own (idle 3600 s).
    fn new(tag: &str) -> TempVault {
        Self::with_policy(tag, r#"{"idle_seconds": 3600}"#)
    }
    /// A vault with the given `cold-policy.json` text, read by `Vault::open`.
    fn with_policy(tag: &str, policy_json: &str) -> TempVault {
        let p = std::env::temp_dir().join(format!("vault_cold_{tag}_{}", new_id()));
        fs::create_dir_all(p.join(".vault")).unwrap();
        fs::write(p.join(".vault").join("cold-policy.json"), policy_json).unwrap();
        TempVault(p)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).expect("open")
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Policy for manual `cold_page` / `cold_pass` calls. `minimum_bytes` 0 means
/// the size check never skips a page.
fn policy(minimum_bytes: usize) -> ColdPolicy {
    ColdPolicy { age_seconds: 30 * DAY, minimum_bytes, pages_per_pass: 2, idle_seconds: 3600 }
}

/// Cutoff (ms) of a split run at `t` seconds: changes at or before it are old.
fn cutoff_ms(t: i64) -> i64 {
    (t - 30 * DAY) * 1000
}

/// A device's copy of `base` with one title change committed at `ts` seconds.
fn device_edit(base: &[u8], peer: u64, ts: i64, title: &str) -> PageDoc {
    let mut pd = PageDoc::from_bytes(base, &[]).unwrap();
    pd.set_peer(peer).unwrap();
    pd.doc.set_next_commit_timestamp(ts);
    pd.set_title(title).unwrap();
    pd.doc.commit();
    pd
}

/// A page document whose creation change is dated `ts` seconds, as a history
/// device would have made it. `PageDoc::new_page` always stamps the real clock,
/// so this mirrors its meta layout with an injected timestamp.
fn historical_page(id: &str, title: &str, parent: Option<&str>, ts: i64) -> Vec<u8> {
    let doc = LoroDoc::new();
    doc.set_peer_id(PEER_H).unwrap();
    doc.set_record_timestamp(true);
    doc.set_next_commit_timestamp(ts);
    let meta = doc.get_map("page");
    meta.insert("id", id).unwrap();
    meta.insert("title", title).unwrap();
    match parent {
        Some(p) => meta.insert("parent", p).unwrap(),
        None => meta.insert("parent", LoroValue::Null).unwrap(),
    }
    meta.insert("created", ts * 1000).unwrap();
    meta.insert("updated", ts * 1000).unwrap();
    doc.commit();
    doc.export(ExportMode::Snapshot).unwrap()
}

/// The history device's states for one page. `s1` is its state after the
/// first edit (a device that synced only that far, before the root). `s3` is
/// its full history: everything before the cold root.
struct Chain {
    s1: Vec<u8>,
    s3: Vec<u8>,
}

fn historical_chain(id: &str, title: &str, parent: Option<&str>, t: i64) -> Chain {
    let init = historical_page(id, title, parent, t - 101 * DAY);
    let s1 = device_edit(&init, PEER_H, t - 100 * DAY, "Old 1").doc.export(ExportMode::Snapshot).unwrap();
    let s2 = device_edit(&s1, PEER_H, t - 95 * DAY, "Old 2").doc.export(ExportMode::Snapshot).unwrap();
    let s3 = device_edit(&s2, PEER_H, t - 40 * DAY, "Old 3").doc.export(ExportMode::Snapshot).unwrap();
    Chain { s1, s3 }
}

/// Imports the history device's page as a new page (its first change is old).
fn import_history(v: &Vault, page: &str, title: &str, parent: Option<&str>, t: i64) -> Chain {
    let chain = historical_chain(page, title, parent, t);
    v.import_updates(page.to_string(), chain.s3.clone()).unwrap();
    chain
}

/// One recent block, made with the real clock.
fn recent(v: &Vault, page: &str, id: &str, text: &str) {
    let block = NewBlock {
        id: id.into(),
        kind: BlockKind::Paragraph,
        text: text.into(),
        marks: vec![],
        checked: false,
    };
    v.apply(page.to_string(), vec![BlockOp::Insert { parent: None, after: None, block }]).unwrap();
}

fn raw(snapshot: &[u8]) -> LoroDoc {
    let doc = LoroDoc::new();
    doc.import(snapshot).unwrap();
    doc
}

fn raw_view(doc: &LoroDoc) -> (LoroValue, Frontiers, VersionVector) {
    (canonical_deep(doc), doc.oplog_frontiers(), doc.oplog_vv())
}

/// Deep value with unused empty root containers removed: a top-level empty Map,
/// List or String. Loro omits such roots from shallow and update exports, and
/// touching a root makes no op, so they are not history. Nested empty values and
/// non-empty roots are kept as they are. Frontiers and VV are compared exactly
/// beside this, never through it.
fn canonical_deep(doc: &LoroDoc) -> LoroValue {
    let LoroValue::Map(roots) = doc.get_deep_value() else {
        panic!("the deep value of a doc is a map of roots");
    };
    let kept: std::collections::HashMap<String, LoroValue> = roots
        .iter()
        .filter(|(_, value)| !is_empty_root(value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    LoroValue::from(kept)
}

fn is_empty_root(value: &LoroValue) -> bool {
    match value {
        LoroValue::Map(map) => map.is_empty(),
        LoroValue::List(list) => list.is_empty(),
        LoroValue::String(text) => text.is_empty(),
        _ => false,
    }
}

/// The full history of `page` (rehydrated from cold): deep value, frontiers,
/// version vector and an exported full snapshot.
fn full_view(v: &Vault, page: &str) -> (LoroValue, Frontiers, VersionVector, Vec<u8>) {
    v.inner()
        .read_history(page, |pd| {
            let snap = pd.doc.export(ExportMode::Snapshot).unwrap();
            (canonical_deep(&pd.doc), pd.doc.oplog_frontiers(), pd.doc.oplog_vv(), snap)
        })
        .unwrap()
}

/// Flushes, then runs `cold_page` at `at_ms`.
fn split(v: &Vault, page: &str, policy: &ColdPolicy, at_ms: i64) -> bool {
    v.flush().unwrap();
    cold_now(v, page, policy, at_ms)
}

/// `cold_page` without a flush first, so a deliberately dirty page stays dirty.
fn cold_now(v: &Vault, page: &str, policy: &ColdPolicy, at_ms: i64) -> bool {
    v.with_db(|db| v.inner().cold_page(db, page, policy, at_ms)).unwrap()
}

/// Versions whose last change is before `cut_ms`: the old, pre-root part of a list.
fn old_versions(versions: &[Version], cut_ms: i64) -> Vec<Version> {
    versions.iter().filter(|x| x.time_ms < cut_ms).cloned().collect()
}

fn reopen(tv: &TempVault, v: Vault) -> Vault {
    drop(v);
    tv.open()
}

fn cold_file(tv: &TempVault, page: &str) -> PathBuf {
    cold::path(&tv.0, page)
}

/// Asserts that the page is cached as a shallow document. Reads never
/// rehydrate, so the test's next import really lands in the shallow hot doc.
fn assert_shallow_cached(v: &Vault, page: &str) {
    assert!(
        v.inner().read(page, |pd| pd.doc.is_shallow()).unwrap(),
        "the import must land in a shallow cached page"
    );
}

/// Shared fixture: a page with historical history and one recent block, flushed.
/// Returns the page id and the history device's states.
fn fixture(v: &Vault, title: &str, t: i64) -> (PageId, Chain) {
    let page = new_id();
    let chain = import_history(v, &page, title, None, t);
    recent(v, &page, "r1", "Recent");
    v.flush().unwrap();
    (page, chain)
}

/// An imported conflict whose side and initial undo frontiers predate the
/// cold root. Commit clocks are injected independently of review timestamps.
fn conflict_fixture(v: &Vault, resolved: bool) -> (PageId, String) {
    use crate::types::{AppliedResolution, Conflict, ConflictScope, ConflictSide,
        ConflictStatus, ResolutionBy, SideState};
    let t = now_ms() / 1000;
    let page = new_id();
    let mut doc = PageDoc::from_bytes(
        &historical_page(&page, "Conflict history", None, t - 101 * DAY), &[]).unwrap();
    doc.doc.set_next_commit_timestamp(t - 100 * DAY);
    doc.apply(&[BlockOp::Insert { parent: None, after: None, block: NewBlock {
        id: "target".into(), kind: BlockKind::Paragraph, text: "Left".into(),
        marks: vec![], checked: false,
    }}]).unwrap();
    doc.doc.commit();
    let left = ConflictSide::new("left".into(), (t - 100 * DAY) * 1000,
        Some(doc.version()), SideState::Present { content: doc.blocks() }, None);
    doc.doc.set_next_commit_timestamp(t - 95 * DAY);
    doc.apply(&[BlockOp::SetText { id: "target".into(), text: "Right".into(), marks: vec![] }]).unwrap();
    doc.doc.commit();
    let before = doc.version();
    let right = ConflictSide::new("right".into(), (t - 95 * DAY) * 1000,
        Some(before.clone()), SideState::Present { content: doc.blocks() }, None);
    let applied = AppliedResolution { time_ms: now_ms(), undone_at: None,
        by: ResolutionBy::User, before, before_state: None,
        fallback: false, restored_ancestors: vec![], resolution: None };
    if resolved {
        doc.doc.set_next_commit_timestamp(t - 40 * DAY);
        doc.doc.set_next_commit_message("sync:resolve-conflict");
        doc.apply(&[BlockOp::SetText { id: "target".into(), text: "Left".into(), marks: vec![] }]).unwrap();
        doc.doc.commit();
    }
    let id = "old-conflict".to_owned();
    let record = Conflict { page: page.clone(), id: id.clone(), format: 2,
        block: "target".into(), time_ms: now_ms(),
        scope: ConflictScope::Block { block: "target".into() }, sides: vec![left, right],
        status: if resolved { ConflictStatus::Resolved(applied.clone()) } else { ConflictStatus::Pending },
        applied: if resolved { vec![applied] } else { vec![] } };
    doc.doc.set_next_commit_timestamp(t - 40 * DAY);
    crate::sync::write_conflict(&doc, &record).unwrap();
    doc.doc.commit();
    v.import_updates(page.clone(), doc.full_snapshot().unwrap()).unwrap();
    recent(v, &page, "outside", "Recent unrelated content");
    assert!(split(v, &page, &policy(0), now_ms()));
    (page, id)
}

fn assert_conflict_content(v: &Vault, page: &str, target: &str) {
    let blocks = v.load_page(page.to_owned()).unwrap().blocks;
    assert_eq!(blocks.iter().find(|b| b.id == "target").unwrap().text, target);
    assert_eq!(blocks.iter().find(|b| b.id == "outside").unwrap().text,
        "Recent unrelated content");
}

fn assert_conflict_versions_before_root(v: &Vault, page: &str) {
    let record = v.list_conflicts(Some(page.to_owned())).unwrap().into_iter().next()
        .or_else(|| v.conflict_log(Some(page.to_owned())).unwrap().into_iter().next()).unwrap();
    v.inner.read(page, |hot| {
        assert!(hot.doc.is_shallow());
        let root = hot.doc.frontiers_to_vv(&hot.doc.shallow_since_frontiers()).unwrap();
        let full = PageDoc::from_bytes(&hot.full_snapshot().unwrap(), &[]).unwrap();
        let versions = record.sides.iter().map(|side| side.frontiers.as_ref().unwrap())
            .chain(record.applied.iter().map(|application| &application.before));
        for version in versions {
            let frontiers = Frontiers::decode(version).unwrap();
            let old = full.doc.frontiers_to_vv(&frontiers).unwrap();
            assert!(root.includes_vv(&old), "recorded version must precede the actual root");
            assert!(!old.includes_vv(&root), "recorded version must be strictly before the root");
        }
    }).unwrap();
}

#[test]
fn cold_conflict_diff_pick_and_undo_read_pre_root_sides() {
    use crate::types::{ConflictResolution, ConflictStatus};
    let tv = TempVault::new("conflict-pick");
    let v = tv.open();
    let (page, id) = conflict_fixture(&v, false);
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);
    assert_conflict_versions_before_root(&v, &page);
    let overlay = v.conflict_diff(page.clone(), id.clone()).unwrap();
    assert_eq!(overlay.sides[0].blocks[0].block.text, "Left");
    assert_eq!(overlay.sides[1].blocks[0].block.text, "Right");
    // Historical review and the following flush keep the on-disk hot split.
    v.flush().unwrap();
    assert!(raw(&fs::read(v.inner.doc_path(&page)).unwrap()).is_shallow());
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);
    v.resolve_conflict(page.clone(), id.clone(), ConflictResolution::UseSide(0)).unwrap();
    assert_conflict_content(&v, &page, "Left");
    v.undo_conflict(page.clone(), id.clone()).unwrap();
    assert_conflict_content(&v, &page, "Right");
    let pending = v.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, ConflictStatus::Pending);
    assert!(pending[0].applied[0].undone_at.is_some());
    assert_eq!(v.conflict_log(Some(page)).unwrap().len(), 1);
}

#[test]
fn cold_conflict_undo_reads_pre_root_before_frontier() {
    let tv = TempVault::new("conflict-undo");
    let v = tv.open();
    let (page, id) = conflict_fixture(&v, true);
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);
    assert_conflict_versions_before_root(&v, &page);
    assert_conflict_content(&v, &page, "Left");
    v.undo_conflict(page.clone(), id.clone()).unwrap();
    assert_conflict_content(&v, &page, "Right");
    let pending = v.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(!pending[0].undoable());
    assert!(v.undo_conflict(page, id).is_err());
}

#[test]
fn cold_conflict_purge_retains_complete_sides_and_page_history() {
    use crate::types::ConflictResolution;
    let tv = TempVault::new("conflict-purge");
    let v = tv.open();
    let (page, id) = conflict_fixture(&v, false);
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);
    assert_conflict_versions_before_root(&v, &page);
    v.delete_page(page.clone()).unwrap();
    v.purge_page(page.clone()).unwrap();
    assert!(!cold_file(&tv, &page).exists());
    assert!(matches!(v.load_page(page.clone()), Err(VaultError::NotFound(_))));
    let archive = v.inner.read_conflict_archive(&page).unwrap();
    assert!(!archive.doc.is_shallow());
    let left_frontiers = Frontiers::decode(
        &v.list_conflicts(Some(page.clone())).unwrap()[0].sides[0].frontiers.clone().unwrap()).unwrap();
    let left_version = left_frontiers.iter().map(|id| format!("{}@{}", id.peer, id.counter))
        .collect::<Vec<_>>().join(",");
    assert_eq!(v.load_page_at(page.clone(), left_version.clone()).unwrap().blocks[0].text, "Left");
    assert!(!v.list_versions(page.clone()).unwrap().is_empty());
    let overlay = v.conflict_diff(page.clone(), id.clone()).unwrap();
    assert_eq!(overlay.sides[1].blocks[0].block.text, "Right");
    v.resolve_conflict(page.clone(), id.clone(), ConflictResolution::UseSide(0)).unwrap();
    v.undo_conflict(page.clone(), id).unwrap();
    assert!(matches!(v.load_page(page.clone()), Err(VaultError::NotFound(_))));
    v.restore_version(page.clone(), left_version).unwrap();
    let restored = v.load_page(page).unwrap();
    assert_eq!(restored.blocks[0].text, "Left");
    assert!(!restored.in_trash);
}

#[test]
fn cold_recent_delta_with_live_conflict_decision_does_not_read_prefix() {
    use crate::types::ConflictResolution;
    let tv = TempVault::new("conflict-recent");
    let v = tv.open();
    let (page, id) = conflict_fixture(&v, false);
    v.resolve_conflict(page.clone(), id, ConflictResolution::Keep).unwrap();
    let bytes = v.export_updates(page.clone(), vec![]).unwrap();
    v.flush().unwrap();
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);
    let mut peer = PageDoc::from_bytes(&bytes, &[]).unwrap();
    let before = peer.doc.oplog_vv();
    peer.set_peer(PEER_B).unwrap();
    peer.set_title("Recent peer with a saved decision").unwrap();
    peer.doc.commit();
    let delta = peer.doc.export(ExportMode::updates(&before)).unwrap();
    let path = cold_file(&tv, &page);
    let aside = tv.0.join("conflict-cold.aside");
    fs::rename(&path, &aside).unwrap();
    let report = v.import_updates(page.clone(), delta).unwrap();
    assert_eq!((report.created, report.skipped, report.changed), (0, 0, 1));
    assert_eq!(v.load_page(page).unwrap().title, "Recent peer with a saved decision");
    assert!(!path.exists());
    fs::rename(aside, path).unwrap();
}

#[test]
fn cold_causal_purge_restores_post_root_editor_with_complete_history() {
    let tv = TempVault::new("purge-editor");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold edited page", t);
    let historical_version = v.list_versions(page.clone()).unwrap().into_iter()
        .find(|version| version.time_ms == (t - 100 * DAY) * 1000).unwrap().id;
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);
    // Loro cannot fork this shallow post-root editor head directly. The
    // PageDoc wrapper must reconstruct it before the recovery journal exports.
    v.inner.read(&page, |doc| {
        assert!(doc.doc.fork_at(&doc.doc.oplog_frontiers()).is_err());
        assert!(!doc.fork_at(&doc.doc.oplog_frontiers()).unwrap().doc.is_shallow());
    }).unwrap();
    let records = vec![crate::sync::PurgeRecord { page: page.clone(),
        version: Some(raw(&chain.s3).oplog_vv().encode()), device: None }];
    v.import_purge_records(records.clone()).unwrap();
    assert!(!v.purged_pages().contains(&page));
    assert_eq!(v.load_page(page.clone()).unwrap().blocks[0].text, "Recent");
    assert_eq!(v.restore_records().len(), 1);
    assert!(!raw(&v.export_updates(page.clone(), vec![]).unwrap()).is_shallow());
    assert_eq!(v.load_page_at(page.clone(), historical_version).unwrap().title, "Old 1");
    v.import_purge_records(records).unwrap();
    assert!(!v.purged_pages().contains(&page));
    let v = reopen(&tv, v);
    assert_eq!(v.load_page(page).unwrap().blocks[0].text, "Recent");
}

#[test]
fn cold_concurrent_post_root_text_edits_record_both_sides() {
    let tv = TempVault::new("conflict-post-root");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold concurrent edits", t);
    let bytes = v.export_updates(page.clone(), vec![]).unwrap();
    let mut peer = PageDoc::from_bytes(&bytes, &[]).unwrap();
    let base = peer.doc.oplog_vv();
    peer.set_peer(PEER_B).unwrap();
    peer.apply(&[BlockOp::SetText { id: "r1".into(), text: "Remote edit".into(), marks: vec![] }]).unwrap();
    peer.doc.commit();
    let delta = peer.doc.export(ExportMode::updates(&base)).unwrap();
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);
    v.apply(page.clone(), vec![BlockOp::SetText {
        id: "r1".into(), text: "Local edit".into(), marks: vec![],
    }]).unwrap();
    assert_shallow_cached(&v, &page);
    v.import_updates(page.clone(), delta).unwrap();
    let conflicts = v.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(conflicts.len(), 1);
    let overlay = v.conflict_diff(page, conflicts[0].id.clone()).unwrap();
    let mut sides = overlay.sides.into_iter().map(|side| side.blocks[0].block.text.clone()).collect::<Vec<_>>();
    sides.sort();
    assert_eq!(sides, ["Local edit", "Remote edit"]);
}

#[test]
fn split_and_reopen_keep_deep_value_frontiers_vv_and_exact_versions() {
    let tv = TempVault::new("reopen");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);

    let pre_snap = v.export_updates(page.clone(), Vec::new()).unwrap();
    let (deep, frontiers, vv) = raw_view(&raw(&pre_snap));
    let before = v.list_versions(page.clone()).unwrap();
    assert!(before.len() > 1, "fixture needs several versions: {before:?}");

    assert!(split(&v, &page, &policy(0), now_ms()));
    assert!(cold_file(&tv, &page).exists());
    let v = reopen(&tv, v);

    // The hot file is shallow, and the page is still whole.
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap());
    assert_eq!(v.load_page(page.clone()).unwrap().title, "Old 3");

    let (deep2, frontiers2, vv2, snap2) = full_view(&v, &page);
    assert_eq!(deep2, deep);
    assert_eq!(frontiers2, frontiers);
    assert_eq!(vv2, vv);
    assert_eq!(raw_view(&raw(&snap2)), (deep, frontiers, vv));
    assert_eq!(v.list_versions(page.clone()).unwrap(), before);
}

#[test]
fn recent_open_edit_and_delta_do_not_read_cold() {
    let tv = TempVault::new("recent");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold", t);
    let before = v.list_versions(page.clone()).unwrap();
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);

    // Move the cold file aside: anything that reads it now fails.
    let cold_path = cold_file(&tv, &page);
    let aside = tv.0.join("cold.aside");
    fs::rename(&cold_path, &aside).unwrap();

    assert_eq!(v.load_page(page.clone()).unwrap().title, "Old 3");
    let since = v
        .page_versions()
        .unwrap()
        .into_iter()
        .find(|p| p.page == page)
        .expect("page version")
        .version;
    v.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: "e1".into(),
                kind: BlockKind::Paragraph,
                text: "Edited while split".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    v.flush().unwrap();
    let delta = v.export_updates(page.clone(), since).unwrap();
    assert!(!delta.is_empty());
    assert!(!cold_path.exists(), "recent paths must not recreate or read cold");

    // A peer that already has the vault's recent state makes an edit that
    // depends only on post-root changes. Importing it must not read cold
    // either. The peer's base is the history device's state plus the vault's
    // changes since then, which `export_updates` returns without rehydrating.
    let base_vv = raw(&chain.s3).oplog_vv();
    let vault_post = v.export_updates(page.clone(), base_vv.encode()).unwrap();
    let mut peer = PageDoc::from_bytes(&chain.s3, &[vault_post]).unwrap();
    let vault_vv = peer.doc.oplog_vv();
    peer.set_peer(PEER_B).unwrap();
    peer.doc.set_next_commit_timestamp(t);
    peer.set_title("Recent peer edit").unwrap();
    peer.doc.commit();
    let blob = peer.doc.export(ExportMode::updates(&vault_vv)).unwrap();
    let report = v.import_updates(page.clone(), blob).unwrap();
    assert_eq!((report.created, report.skipped, report.changed), (0, 0, 1));
    assert_eq!(v.load_page(page.clone()).unwrap().title, "Recent peer edit");
    assert!(!cold_path.exists(), "a recent peer import must not recreate or read cold");

    // Old history paths work again once the file is back.
    fs::rename(&aside, &cold_path).unwrap();
    let after = v.list_versions(page.clone()).unwrap();
    let cut = cutoff_ms(t);
    assert_eq!(old_versions(&after, cut), old_versions(&before, cut));
}

#[test]
fn pre_root_versions_load_and_restore_after_reopen() {
    let tv = TempVault::new("preroot");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);
    let before = v.list_versions(page.clone()).unwrap();
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);

    assert_eq!(v.list_versions(page.clone()).unwrap(), before);
    let at = |ts: i64| {
        before
            .iter()
            .find(|x| x.time_ms == ts * 1000)
            .unwrap_or_else(|| panic!("no version at {ts}"))
            .clone()
    };
    let o1 = at(t - 100 * DAY);
    let o2 = at(t - 95 * DAY);
    assert_eq!(v.load_page_at(page.clone(), o1.id.clone()).unwrap().title, "Old 1");
    assert_eq!(v.load_page_at(page.clone(), o2.id.clone()).unwrap().title, "Old 2");
    // The oldest version is the creation, before any cold data.
    let oldest = before.last().unwrap();
    assert_eq!(v.load_page_at(page.clone(), oldest.id.clone()).unwrap().title, "Cold");

    v.restore_version(page.clone(), o2.id.clone()).unwrap();
    assert_eq!(v.load_page(page.clone()).unwrap().title, "Old 2");
    v.flush().unwrap();
    let hot = PageDoc::from_bytes(&fs::read(v.inner().doc_path(&page)).unwrap(), &[]).unwrap();
    assert!(hot.doc.is_shallow(), "restore must not make the hot file full");
}

#[test]
fn fresh_full_export_and_older_vv_are_complete_and_converge() {
    let tv = TempVault::new("export");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold", t);
    let pre = raw(&v.export_updates(page.clone(), Vec::new()).unwrap());
    let (deep, frontiers, vv) = raw_view(&pre);
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);

    // A fresh device that asks for everything gets the whole history.
    let full = v.export_updates(page.clone(), Vec::new()).unwrap();
    let fresh = raw(&full);
    assert_eq!(raw_view(&fresh), (deep.clone(), frontiers.clone(), vv.clone()));

    // A device that synced before the root (only the first change) gets the
    // missing prefix and the rest, and converges to the same state.
    let old = raw(&chain.s1);
    assert!(!old.oplog_vv().includes_vv(&vv));
    let delta = v.export_updates(page.clone(), old.oplog_vv().encode()).unwrap();
    old.import(&delta).unwrap();
    assert_eq!(raw_view(&old), (deep, frontiers, vv));
}

#[test]
fn offline_device_before_root_merges_into_cold_history() {
    let tv = TempVault::new("merge");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold", t);
    // The device synced before the root and goes offline there. Its edit
    // depends only on the first change, which is before the root.
    let d = device_edit(&chain.s1, PEER_D, t - 60 * DAY, "Offline D");
    let reference = offline_mixed_import(&v, &page, &d, &chain.s3, t).reference;

    v.flush().unwrap();
    let v = reopen(&tv, v);
    let hot = PageDoc::from_bytes(&fs::read(v.inner().doc_path(&page)).unwrap(), &[]).unwrap();
    assert!(hot.doc.is_shallow(), "flush after the merge keeps the hot file shallow");
    let (deep, frontiers, vv, _) = full_view(&v, &page);
    assert_eq!(deep, canonical_deep(&reference));
    assert_eq!(frontiers, reference.oplog_frontiers());
    assert_eq!(vv, reference.oplog_vv());
    assert!(vv.get(&PEER_D).is_some());
    assert!(vv.get(&PEER_B).is_some());
}

/// Copies the files that exist among `paths` from `src` into `dst`, keeping
/// their paths relative to the vault root.
fn copy_files(src: &TempVault, dst: &TempVault, paths: &[PathBuf]) {
    for p in paths {
        if p.exists() {
            let to = dst.0.join(p.strip_prefix(&src.0).unwrap());
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(p, to).unwrap();
        }
    }
}

/// Recovery before any flush. The merged changes exist only in the page log
/// (and in memory). A copy of the on-disk files, taken while the page's io and
/// state locks are held, must rebuild the full merged state by itself.
#[test]
fn offline_merge_survives_reopen_before_writer_flush() {
    let tv = TempVault::new("replay");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold", t);
    let d = device_edit(&chain.s1, PEER_D, t - 60 * DAY, "Offline D");
    let mixed = offline_mixed_import(&v, &page, &d, &chain.s3, t);
    let reference = mixed.reference;

    // Source view and version list, read before the copy. Reads rehydrate the
    // cached doc in memory only; the files on disk stay as they are.
    let src_view = full_view(&v, &page);
    let src_versions = v.list_versions(page.clone()).unwrap();
    assert_eq!(src_view.0, canonical_deep(&reference));
    assert_eq!(src_view.2, reference.oplog_vv());

    let copy = TempVault::new("replay_copy");
    {
        // Holding io (no flush phase 2 or cold publish) and state (no flush
        // phase 1 or log rotation) freezes the page's files for the copy.
        let slot = v.inner().slot(&page).unwrap();
        let _io = lock(&slot.io);
        let st = lock(&slot.state);
        assert!(st.dirty, "the merge must still be unflushed; a flush would replace the evidence");
        let inner = v.inner();
        copy_files(
            &tv,
            &copy,
            &[
                inner.doc_path(&page),
                inner.log_path(&page),
                inner.old_log_path(&page),
                inner.cache_path(&page),
                cold_file(&tv, &page),
            ],
        );
    }

    // The copied hot file is still the shallow split. It excludes both
    // devices' merged changes, so the copied log is the only recovery path.
    let hot_path = copy.0.join(v.inner().doc_path(&page).strip_prefix(&tv.0).unwrap());
    let hot = raw(&fs::read(hot_path).unwrap());
    assert!(hot.is_shallow(), "the on-disk hot file must still be shallow");
    assert!(hot.oplog_vv().get(&PEER_D).is_none(), "hot file must not hold the offline merge");
    assert!(hot.oplog_vv().get(&PEER_B).is_none(), "hot file must not hold device B's merge");

    // Opening the copy replays the logs. Its full state equals the merged state
    // and the source's state, down to frontiers, VV and version list.
    let copy_v = copy.open();
    let (deep, frontiers, vv, _) = full_view(&copy_v, &page);
    assert_eq!(deep, canonical_deep(&reference));
    assert_eq!(frontiers, reference.oplog_frontiers());
    assert_eq!(vv, reference.oplog_vv());
    assert_eq!((deep, frontiers, vv), (src_view.0, src_view.1, src_view.2));
    assert_eq!(copy_v.list_versions(page.clone()).unwrap(), src_versions);
}

/// Probe for the partial-application claim. A blob whose pre-root part is
/// rejected may still apply its valid part first. Scenario 9 in
/// `tests/shallow_snapshot_limits.rs` shows this for a different mixing order.
/// This test checks it on the vault's own shallow snapshot. If it fails, the
/// mixing order must be rebuilt along scenario 9, and no partial coverage is
/// claimed for this blob.
#[test]
fn mixed_blob_applies_valid_ops_before_outdated_rejection() {
    let tv = TempVault::new("partial");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold", t);
    let d = device_edit(&chain.s1, PEER_D, t - 60 * DAY, "Offline D");
    let mixed = offline_mixed_import(&v, &page, &d, &chain.s3, t);
    assert!(
        mixed.raw_advanced,
        "a raw shallow copy did not apply B's valid op before rejecting D's pre-root op"
    );
    assert!(mixed.reference.oplog_vv().get(&PEER_B).is_some());
}

#[test]
fn pre_root_only_blob_merges_through_cold_import() {
    let tv = TempVault::new("preonly");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, chain) = fixture(&v, "Cold", t);
    let before_split = v.export_updates(page.clone(), Vec::new()).unwrap();
    let d = device_edit(&chain.s1, PEER_D, t - 60 * DAY, "Offline D");
    assert!(split(&v, &page, &policy(0), now_ms()));

    let only_d = d.doc.export(ExportMode::updates(&raw(&before_split).oplog_vv())).unwrap();
    v.import_updates(page.clone(), only_d.clone()).unwrap();

    let reference = raw(&before_split);
    reference.import(&only_d).unwrap();
    let (deep, frontiers, vv, _) = full_view(&v, &page);
    assert_eq!(deep, canonical_deep(&reference));
    assert_eq!(frontiers, reference.oplog_frontiers());
    assert_eq!(vv, reference.oplog_vv());
}

#[test]
fn trashed_page_and_inherited_child_go_cold_and_round_trip() {
    let tv = TempVault::new("trash");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (parent, _) = fixture(&v, "Parent", t);
    let child = new_id();
    import_history(&v, &child, "Child", Some(parent.as_str()), t);
    recent(&v, &child, "c1", "Child recent");
    v.flush().unwrap();
    let parent_before = v.list_versions(parent.clone()).unwrap();
    let child_before = v.list_versions(child.clone()).unwrap();

    v.delete_page(parent.clone()).unwrap();
    assert!(v.list_trash().unwrap().iter().any(|x| x.page.id == parent));

    // Trashed pages go fully cold, so the minimum size does not apply, and
    // the child goes with its trashed parent.
    assert!(split(&v, &parent, &policy(usize::MAX), now_ms()));
    assert!(split(&v, &child, &policy(usize::MAX), now_ms()));
    assert!(cold_file(&tv, &parent).exists());
    assert!(cold_file(&tv, &child).exists());
    let v = reopen(&tv, v);
    assert!(v.inner().read(&child, |pd| pd.doc.is_shallow()).unwrap());

    // Restore brings the subtree back with its whole history.
    v.restore_page(parent.clone()).unwrap();
    assert!(v.list_trash().unwrap().is_empty());
    let cut = cutoff_ms(t);
    assert_eq!(old_versions(&v.list_versions(parent.clone()).unwrap(), cut), old_versions(&parent_before, cut));
    assert_eq!(old_versions(&v.list_versions(child.clone()).unwrap(), cut), old_versions(&child_before, cut));

    // Sync of the restored child gives the full history.
    let synced = v.export_updates(child.clone(), Vec::new()).unwrap();
    assert_eq!(canonical_deep(&raw(&synced)), full_view(&v, &child).0);

    // Purging removes the cold files and tombstones both pages, so the same
    // old updates coming back from another device are skipped.
    v.delete_page(parent.clone()).unwrap();
    v.purge_page(parent.clone()).unwrap();
    assert!(!cold_file(&tv, &parent).exists());
    assert!(!cold_file(&tv, &child).exists());
    assert!(v.purged_pages().contains(&parent));
    assert!(v.purged_pages().contains(&child));
    assert_eq!(v.import_updates(child.clone(), synced).unwrap().skipped, 1);
}

#[test]
fn skips_pages_below_cutoff_minimum_dirty_and_in_use() {
    let tv = TempVault::new("skip");
    let v = tv.open();
    let t = now_ms() / 1000;
    let recent_only = v.create_page(None, "Recent".into()).unwrap();
    recent(&v, &recent_only, "r1", "Only recent");
    let (old, _) = fixture(&v, "Old", t);
    let at = now_ms();

    // Cutoff: nothing is older than 30 days, so there is no root.
    assert!(!split(&v, &recent_only, &policy(0), at));
    assert!(!cold_file(&tv, &recent_only).exists());

    // Minimum: the old prefix is smaller than the minimum.
    assert!(!split(&v, &old, &policy(usize::MAX), at));
    assert!(!cold_file(&tv, &old).exists());

    // In use: an extra handle to the page blocks the split.
    let held = v.inner().slot(&old).unwrap();
    assert!(!split(&v, &old, &policy(0), at));
    drop(held);

    // Dirty: a changed page is not split. Set the flag without flushing, so
    // the writer cannot clear it first.
    {
        let slot = v.inner().slot(&old).unwrap();
        lock(&slot.state).dirty = true;
    }
    assert!(!cold_now(&v, &old, &policy(0), at));
    assert!(!cold_file(&tv, &old).exists());
    {
        let slot = v.inner().slot(&old).unwrap();
        lock(&slot.state).dirty = false;
    }

    // Clean and unused: the split happens, and a second split is a no-op.
    assert!(split(&v, &old, &policy(0), at));
    assert!(cold_file(&tv, &old).exists());
    assert!(!split(&v, &old, &policy(0), at));
}

#[test]
fn cold_pass_splits_each_eligible_page_once() {
    let tv = TempVault::new("pass");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (a, _) = fixture(&v, "A", t);
    let (b, _) = fixture(&v, "B", t);
    let c = v.create_page(None, "C".into()).unwrap();
    recent(&v, &c, "r1", "Only recent");
    v.flush().unwrap();
    let at = now_ms();
    let one_per_pass = ColdPolicy { pages_per_pass: 1, ..policy(0) };

    let mut cursor = None;
    let mut moved = 0;
    for _ in 0..8 {
        moved += v.with_db(|db| v.inner().cold_pass(db, &mut cursor, &one_per_pass, at)).unwrap();
    }
    assert_eq!(moved, 2);
    assert!(cold_file(&tv, &a).exists());
    assert!(cold_file(&tv, &b).exists());
    assert!(!cold_file(&tv, &c).exists());

    // A second sweep finds nothing new to do.
    let mut again = 0;
    for _ in 0..8 {
        again += v.with_db(|db| v.inner().cold_pass(db, &mut cursor, &one_per_pass, at)).unwrap();
    }
    assert_eq!(again, 0);
}

#[test]
fn flush_after_rehydration_keeps_the_root() {
    let tv = TempVault::new("rehydrate");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);

    let root = v.inner().read(&page, |pd| pd.doc.shallow_since_frontiers()).unwrap();
    // Reading history rehydrates the page in memory.
    assert!(!v.list_versions(page.clone()).unwrap().is_empty());
    recent(&v, &page, "r2", "After rehydrate");
    v.flush().unwrap();

    let hot = PageDoc::from_bytes(&fs::read(v.inner().doc_path(&page)).unwrap(), &[]).unwrap();
    assert!(hot.doc.is_shallow());
    assert_eq!(hot.doc.shallow_since_frontiers(), root);
}

/// Crash after the cold prefix is durable but before the shallow hot file is
/// published. The cold file is a redundant prefix, so the page must stay whole
/// and the next split must succeed.
#[test]
fn crash_after_cold_write_before_hot_publish_keeps_page_whole() {
    let tv = TempVault::new("crash");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);
    let hot_file = v.inner().doc_path(&page);
    let hot_before = fs::read(&hot_file).unwrap();
    let before = v.list_versions(page.clone()).unwrap();
    let whole_doc = raw(&v.export_updates(page.clone(), Vec::new()).unwrap());
    let whole = raw_view(&whole_doc);

    assert!(split(&v, &page, &policy(0), now_ms()));
    drop(v);
    // Crash: the cold prefix is durable, but the rename of the shallow hot
    // file never ran. Put back the hot file as it was before the split.
    assert!(cold_file(&tv, &page).exists());
    fs::write(&hot_file, &hot_before).unwrap();
    let v = tv.open();

    assert!(
        !v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap(),
        "an unpublished split must leave the hot file full"
    );
    assert_eq!(v.list_versions(page.clone()).unwrap(), before);
    let (deep, frontiers, vv, _) = full_view(&v, &page);
    assert_eq!((deep, frontiers, vv), whole);

    // The page recovers: a later split replaces the stale cold prefix.
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap());
    let (deep2, frontiers2, vv2, _) = full_view(&v, &page);
    assert_eq!((deep2, frontiers2, vv2), whole);
}

/// Reverse order: a shallow hot file may only be staged after a durable cold
/// prefix covers its root. Missing prefixes and prefixes that stop short of
/// the root are both refused. The matching prefix is accepted.
#[test]
fn stage_refuses_shallow_hot_unless_cold_prefix_covers_its_root() {
    let tv = TempVault::new("reverse");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);
    let stage_file = v.inner().docs_dir().join(format!("{page}.cold-stage"));
    let full = PageDoc::from_bytes(&v.export_updates(page.clone(), Vec::new()).unwrap(), &[]).unwrap();

    // Root at 30 days (after Old 3): the hot file for it has no cold prefix yet.
    let root_30 = crate::history::cutoff_root(&full, t - 30 * DAY).unwrap().expect("root");
    let hot_30 = crate::history::hot_snapshot(&full, &root_30).unwrap();
    assert!(v.inner().stage_cold_hot(&page, &hot_30).is_err(), "no cold prefix must refuse");
    assert!(!stage_file.exists(), "a refused stage must leave nothing behind");

    // A cold prefix at 50 days (before Old 3) covers only the older root.
    assert!(split(&v, &page, &policy(0), (t - 20 * DAY) * 1000));
    assert!(v.inner().stage_cold_hot(&page, &hot_30).is_err(), "a shorter cold prefix must refuse");
    assert!(!stage_file.exists());

    // The hot file for the 50-day root is covered, so it stages.
    let root_50 = crate::history::cutoff_root(&full, t - 50 * DAY).unwrap().expect("root");
    let hot_50 = crate::history::hot_snapshot(&full, &root_50).unwrap();
    let staged = v.inner().stage_cold_hot(&page, &hot_50).unwrap();
    assert!(staged.exists());
    fs::remove_file(&staged).unwrap();
}

/// Serial history that alternates between two peers, every change depending
/// on the one before. The requested root must come back exactly, with the
/// original operation IDs, and the root must hold exactly the six old changes.
#[test]
fn serial_alternating_peers_keep_original_ids_and_root() {
    let t = now_ms() / 1000;
    let init = historical_page("serial", "Serial 0", None, t - 101 * DAY);
    let mut pd = PageDoc::from_bytes(&init, &[]).unwrap();
    for i in 0..6 {
        let peer = if i % 2 == 0 { PEER_H } else { PEER_D };
        pd.set_peer(peer).unwrap();
        pd.doc.set_next_commit_timestamp(t - (100 - 5 * i) * DAY);
        pd.set_title(&format!("Serial {i}")).unwrap();
        pd.doc.commit();
    }
    // One recent change by a third device, which depends on all of them.
    pd.set_peer(PEER_B).unwrap();
    pd.doc.set_next_commit_timestamp(t);
    pd.set_title("Post-root").unwrap();
    pd.doc.commit();

    let requested = crate::history::cutoff_root(&pd, t - 30 * DAY).unwrap().expect("root");
    let hot_bytes = crate::history::hot_snapshot(&pd, &requested).unwrap();
    let hot = LoroDoc::new();
    hot.import(&hot_bytes).unwrap();
    assert!(hot.is_shallow());
    // Root distance zero: Loro kept the requested boundary.
    assert_eq!(hot.shallow_since_frontiers(), requested, "root moved");
    // Original IDs and state survive the trim.
    assert_eq!(hot.oplog_vv(), pd.doc.oplog_vv(), "operation IDs changed");
    assert_eq!(hot.oplog_frontiers(), pd.doc.oplog_frontiers());
    assert_eq!(canonical_deep(&hot), canonical_deep(&pd.doc));
    // The root holds exactly the six serial changes, not the recent one.
    let vv = pd.doc.oplog_vv();
    let root_vv = pd.doc.frontiers_to_vv(&requested).unwrap();
    assert_eq!(root_vv.get(&PEER_H), vv.get(&PEER_H));
    assert_eq!(root_vv.get(&PEER_D), vv.get(&PEER_D));
    assert_eq!(root_vv.get(&PEER_B), None);
}

/// History whose `count` edits each set a title of `bytes` pseudo-random
/// letters (an LCG, so LZ4 cannot shrink it much). Edits are dated 90 days back
/// and onward, one day apart, so every change is older than the 30-day cutoff.
fn import_padded_history(v: &Vault, page: &str, t: i64, count: usize, bytes: usize) -> Chain {
    let mut cur = historical_page(page, "Padded", None, t - 101 * DAY);
    let mut s1 = Vec::new();
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    for k in 0..count {
        let mut title = String::with_capacity(bytes);
        for _ in 0..bytes {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            title.push(char::from(b'a' + ((seed >> 33) % 26) as u8));
        }
        cur = device_edit(&cur, PEER_H, t - 90 * DAY + k as i64 * DAY, &title)
            .doc
            .export(ExportMode::Snapshot)
            .unwrap();
        if k == 0 {
            s1 = cur.clone();
        }
    }
    v.import_updates(page.to_string(), cur.clone()).unwrap();
    Chain { s1, s3: cur }
}

/// Registers an empty `type_data` root on a cached shallow page without any op,
/// then rehydrates it. The page must come back exact, with the same versions,
/// frontiers and VV, and the root must stay empty.
#[test]
fn materialized_empty_type_data_root_survives_split_and_rehydrate() {
    let tv = TempVault::new("typedata");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);
    v.inner().read(&page, |pd| { pd.doc.get_map("type_data"); }).unwrap();
    let before = raw_view(&raw(&v.export_updates(page.clone(), Vec::new()).unwrap()));
    let before_versions = v.list_versions(page.clone()).unwrap();
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);

    // Materialize the root on the cached shallow doc: no op is recorded.
    v.inner()
        .read(&page, |pd| {
            let _ = pd.doc.get_map("type_data");
        })
        .unwrap();
    let (hot_deep, hot_frontiers, hot_vv, hot_snap) = v
        .inner()
        .read(&page, |pd| {
            (pd.doc.get_deep_value(), pd.doc.oplog_frontiers(), pd.doc.oplog_vv(), pd.doc.export(ExportMode::Snapshot).unwrap())
        })
        .unwrap();
    assert_eq!(hot_frontiers, before.1, "materializing a root must not add an op");
    assert_eq!(hot_vv, before.2, "materializing a root must not add an op");
    assert_eq!(raw(&hot_snap).get_map("type_data").len(), 0, "the hot root must stay empty");

    // Rehydrating checks the full copy against the cached page's deep value.
    // Exact on purpose: this regression proves the production guard keeps the
    // root materialized, so the rebuilt deep value is compared without
    // canonicalization.
    let (deep, frontiers, vv, snap) = v
        .inner()
        .read_history(&page, |pd| {
            (pd.doc.get_deep_value(), pd.doc.oplog_frontiers(), pd.doc.oplog_vv(), pd.doc.export(ExportMode::Snapshot).unwrap())
        })
        .unwrap();
    assert_eq!(deep, hot_deep, "rehydrated page must match the cached page exactly");
    assert_eq!(frontiers, before.1);
    assert_eq!(vv, before.2);
    assert_eq!(raw(&snap).get_map("type_data").len(), 0, "the full root must stay empty");
    assert_eq!(v.load_page(page.clone()).unwrap().title, "Old 3");
    assert_eq!(v.list_versions(page.clone()).unwrap(), before_versions);
}

#[test]
fn deleted_empty_root_recreated_by_old_updates_is_not_a_state_mismatch() {
    let tv = TempVault::new("retired_root");
    let v = tv.open();
    let t = now_ms() / 1000;
    let page = new_id();
    let base = historical_page(&page, "Root history", None, t - 101 * DAY);
    let pd = PageDoc::from_bytes(&base, &[]).unwrap();
    let retired = pd.doc.get_map("retired");
    retired.insert("old", true).unwrap();
    pd.doc.set_next_commit_timestamp(t - 90 * DAY);
    pd.doc.commit();
    retired.delete("old").unwrap();
    let visible = pd.doc.get_map("visible");
    visible.insert("nested_empty", LoroValue::from(std::collections::HashMap::<String, LoroValue>::new())).unwrap();
    pd.doc.set_next_commit_timestamp(t - 40 * DAY);
    pd.doc.commit();
    v.import_updates(page.clone(), pd.doc.export(ExportMode::Snapshot).unwrap()).unwrap();
    recent(&v, &page, "r1", "Recent");
    v.flush().unwrap();
    let expected = full_view(&v, &page);
    let versions = v.list_versions(page.clone()).unwrap();
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);
    let rebuilt = full_view(&v, &page);
    assert_eq!((rebuilt.0, rebuilt.1, rebuilt.2), (expected.0, expected.1, expected.2));
    assert_eq!(v.list_versions(page.clone()).unwrap(), versions);
    v.inner().read_history(&page, |pd| {
        assert_eq!(pd.doc.get_map("retired").len(), 0);
        assert_eq!(pd.doc.get_map("visible").get("nested_empty").unwrap().get_deep_value(),
            LoroValue::from(std::collections::HashMap::<String, LoroValue>::new()));
    }).unwrap();
}

/// Eligibility under the shipped default policy. Its 16 KiB floor skips a small
/// aged prefix and accepts a large one. A zero-floor control proves the small
/// page is otherwise eligible.
#[test]
fn default_minimum_bytes_skips_small_aged_prefix_and_splits_large_one() {
    let tv = TempVault::new("minbytes");
    let v = tv.open();
    let t = now_ms() / 1000;
    let (small, _) = fixture(&v, "Small", t);
    let big = new_id();
    import_padded_history(&v, &big, t, 12, 4096);
    recent(&v, &big, "r1", "Recent");
    v.flush().unwrap();
    let at = now_ms();

    let default = ColdPolicy::default();
    assert_eq!(default.minimum_bytes, 16 * 1024, "the shipped floor is 16 KiB");
    assert!(!split(&v, &small, &default, at), "a small aged prefix must be below the floor");
    assert!(!cold_file(&tv, &small).exists());
    assert!(split(&v, &small, &policy(0), at), "control: the small prefix is otherwise eligible");
    assert!(cold_file(&tv, &small).exists());

    assert!(split(&v, &big, &default, at), "a 12 x 4 KiB aged prefix must clear the floor");
    assert!(cold_file(&tv, &big).exists());
}

/// The writer's own idle pass, driven by a local `cold-policy.json` with a
/// 1 s idle bound and no size floor. The aged page must be split without any
/// explicit `cold_page` call, within a bounded wait, and must reopen whole.
#[test]
fn writer_idle_pass_splits_aged_page_under_local_policy() {
    let tv = TempVault::with_policy("idle", r#"{"idle_seconds": 1, "minimum_bytes": 0}"#);
    let v = tv.open();
    let t = now_ms() / 1000;
    let (page, _) = fixture(&v, "Cold", t);
    let before_doc = raw(&v.export_updates(page.clone(), Vec::new()).unwrap());
    let before = raw_view(&before_doc);
    let before_versions = v.list_versions(page.clone()).unwrap();

    // Poll the hot file on disk: a shallow file means the writer published.
    let hot_path = v.inner().doc_path(&page);
    let mut published = false;
    for _ in 0..150 {
        if let Ok(bytes) = fs::read(&hot_path) {
            let doc = LoroDoc::new();
            if doc.import(&bytes).is_ok() && doc.is_shallow() {
                published = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(published, "the writer's idle pass did not split the aged page within 15 s");
    assert!(cold_file(&tv, &page).exists());

    let v = reopen(&tv, v);
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap());
    let (deep, frontiers, vv, _) = full_view(&v, &page);
    assert_eq!((deep, frontiers, vv), before);
    assert_eq!(v.list_versions(page.clone()).unwrap(), before_versions);
}

/// What `offline_mixed_import` returns: the merged reference document, and
/// whether a raw copy of the shallow hot snapshot applied some valid ops
/// before it rejected the blob.
struct Mixed {
    reference: LoroDoc,
    raw_advanced: bool,
}

/// Imports `blob` into a raw copy of the shallow snapshot. Returns whether
/// Loro rejected it as depending on an outdated version, and whether the copy's
/// version vector advanced anyway. Raw copies show Loro's behaviour without
/// the vault's error handling.
fn raw_shallow_import(shallow: &[u8], blob: &[u8]) -> (bool, bool) {
    let copy = raw(shallow);
    assert!(copy.is_shallow(), "the probe must start from a shallow snapshot");
    let before = copy.oplog_vv();
    let rejected = matches!(
        copy.import(blob),
        Err(LoroError::ImportUpdatesThatDependsOnOutdatedVersion)
    );
    (rejected, copy.oplog_vv() != before)
}

/// Split at `now`, then import into the vault a mixed blob: the offline
/// device's pre-root edit `d` plus a post-root edit from device B that
/// depends on the full current state. `pre_root` is the history device's full
/// state before the cold root, `Chain::s3`.
fn offline_mixed_import(v: &Vault, page: &str, d: &PageDoc, pre_root: &[u8], t: i64) -> Mixed {
    assert!(split(v, page, &policy(0), now_ms()));
    // The vault's full state, read without rehydrating. `pre_root` already
    // covers the cold root, so the export asks only for the later vault
    // changes. An empty `since` would rehydrate the shallow cache.
    let pre = raw(pre_root);
    let post = v.export_updates(page.to_string(), pre.oplog_vv().encode()).unwrap();
    pre.import(&post).unwrap();
    let vault_full = pre.export(ExportMode::Snapshot).unwrap();
    let vault_vv = pre.oplog_vv();
    assert_shallow_cached(v, page);
    // Device B edits after the root, on the vault's full state. Its change does
    // not depend on D's pre-root change.
    let b = device_edit(&vault_full, PEER_B, t, "Post-root B");
    let w = b.doc.export(ExportMode::updates(&vault_vv)).unwrap();
    // D catches up with the vault and with B, then sends what the vault lacks:
    // its pre-root edit (rejected by the shallow hot doc) and B's post-root
    // edit (valid). The import has to merge both through cold history.
    d.doc.import(&vault_full).unwrap();
    d.doc.import(&w).unwrap();
    let blob = d.doc.export(ExportMode::updates(&vault_vv)).unwrap();
    // Probe the exact rejection on the shallow snapshot the vault holds, before
    // the real import changes anything.
    let shallow = v.inner().read(page, |pd| pd.doc.export(ExportMode::Snapshot).unwrap()).unwrap();
    let (rejected, raw_advanced) = raw_shallow_import(&shallow, &blob);
    assert!(rejected, "the pre-root edit must hit the outdated-dependency rejection");
    let report = v.import_updates(page.to_string(), blob).unwrap();
    assert_eq!((report.created, report.skipped), (0, 0));
    Mixed { reference: d.doc.clone(), raw_advanced }
}

/// Imports the page as a history device's creation only: one change dated before
/// the cutoff, with no later title edits. The root is then that creation change.
fn import_creation(v: &Vault, page: &str, title: &str, t: i64) {
    v.import_updates(page.to_string(), historical_page(page, title, None, t - 101 * DAY)).unwrap();
}

/// Creation-only page with one recent block, split and reopened. The reopened
/// page is cached as shallow with its cold file attached, so the next full read
/// rebuilds from that file. Its cached head is the rebuilt head.
fn creation_only_split(tv: &TempVault, t: i64) -> (Vault, PageId, LoroValue, Frontiers, VersionVector) {
    let v = tv.open();
    let page = new_id();
    import_creation(&v, &page, "Cold", t);
    recent(&v, &page, "r1", "Recent");
    v.flush().unwrap();
    let whole = raw_view(&raw(&v.export_updates(page.clone(), Vec::new()).unwrap()));
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(tv, v);
    assert_shallow_cached(&v, &page);
    (v, page, whole.0, whole.1, whole.2)
}

/// Same operation IDs, root and version vector as the real cold prefix, but a
/// different creation title. A rebuild over it reaches the cached VV and
/// frontiers with the wrong state, so only the state comparison at the cached
/// head can refuse it. The file itself stays valid and decodes to the same root.
#[test]
fn same_head_rebuild_refuses_wrong_prefix_state_with_matching_versions() {
    let tv = TempVault::new("wrong_prefix");
    let t = now_ms() / 1000;
    let (v, page, deep, frontiers, vv) = creation_only_split(&tv, t);
    let good = cold::read(&tv.0, &page).unwrap();

    let wrong_snapshot = historical_page(&page, "Wrong", None, t - 101 * DAY);
    let wrong_doc = raw(&wrong_snapshot);
    assert_eq!(wrong_doc.oplog_frontiers(), good.root.frontiers, "the wrong prefix must keep the root frontiers");
    assert_eq!(wrong_doc.oplog_vv(), good.root.vv, "the wrong prefix must keep the root version vector");
    assert_ne!(canonical_deep(&wrong_doc), canonical_deep(&raw(&good.updates)), "the wrong prefix must differ in state");
    cold::write(&tv.0, &page, &cold::ColdPart { root: good.root.clone(), updates: wrong_snapshot }).unwrap();

    // The rewritten file reads back with the same root, and its updates decode to the wrong state.
    let rewritten = cold::read(&tv.0, &page).unwrap();
    assert_eq!(rewritten.root.frontiers, good.root.frontiers);
    assert_eq!(rewritten.root.vv, good.root.vv);
    assert_eq!(canonical_deep(&raw(&rewritten.updates)), canonical_deep(&wrong_doc));

    // The cached head is the rebuilt head, so this takes the same-head path.
    // Prove this is a complete rebuild at the same head with different state.
    // This uses the same prefix/disk/cached-hot inputs as full_copy_locked.
    let disk = fs::read(v.inner().doc_path(&page)).unwrap();
    let hot = v.inner().read(&page, |pd| pd.doc.export(ExportMode::all_updates()).unwrap()).unwrap();
    let unchecked = PageDoc::from_bytes(&rewritten.updates, &[disk, hot]).unwrap();
    assert!(!unchecked.doc.is_shallow());
    assert_eq!(unchecked.doc.oplog_vv(), vv);
    assert_eq!(unchecked.doc.oplog_frontiers(), frontiers);
    assert_eq!(unchecked.doc.state_frontiers(), frontiers);
    assert_ne!(canonical_deep(&unchecked.doc), deep);
    assert_eq!(unchecked.meta().title, "Wrong");

    // The refusal must be the state comparison's own error. If Loro dropped the
    // wrong title during import, the rebuild would succeed and this would fail.
    let err = v.inner().read_history(&page, |_| ()).unwrap_err();
    assert_eq!(err, VaultError::Invalid("cold history does not reconstruct the hot page".into()));
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap(), "a refused rebuild must leave the page shallow");

    // Restoring the real prefix rebuilds the whole page exactly.
    cold::write(&tv.0, &page, &good).unwrap();
    let (deep2, frontiers2, vv2, _) = full_view(&v, &page);
    assert_eq!((deep2, frontiers2, vv2), (deep, frontiers, vv));
}

/// The same wrong prefix, but the on-disk hot file holds one newer operation, so
/// the rebuilt head is not the cached head and the same-head check does not
/// apply. The rebuild forks at the cached head. That fork matches the cached VV
/// and frontiers, so only the fallback's state comparison can refuse it. The
/// rehydrate runs on the cached page itself: `read_history` would first refresh
/// the cache from the newer disk file and take the same-head path.
#[test]
fn newer_disk_op_takes_fallback_and_refuses_wrong_prefix_state() {
    let tv = TempVault::new("fallback_wrong_prefix");
    let t = now_ms() / 1000;
    let (v, page, _deep, frontiers, _vv) = creation_only_split(&tv, t);
    let good = cold::read(&tv.0, &page).unwrap();

    // One newer operation appended to the on-disk hot file, after the cached head.
    let disk_before = fs::read(v.inner().doc_path(&page)).unwrap();
    let newer = device_edit(&disk_before, PEER_D, t, "Newer").doc.export(ExportMode::all_updates()).unwrap();
    fs::write(v.inner().doc_path(&page), &newer).unwrap();

    let wrong_snapshot = historical_page(&page, "Wrong", None, t - 101 * DAY);
    let wrong_doc = raw(&wrong_snapshot);
    assert_eq!(wrong_doc.oplog_frontiers(), good.root.frontiers, "the wrong prefix must keep the root frontiers");
    assert_eq!(wrong_doc.oplog_vv(), good.root.vv, "the wrong prefix must keep the root version vector");
    assert_ne!(canonical_deep(&wrong_doc), canonical_deep(&raw(&good.updates)), "the wrong prefix must differ in state");
    cold::write(&tv.0, &page, &cold::ColdPart { root: good.root.clone(), updates: wrong_snapshot }).unwrap();

    // The rebuild's inputs are the wrong prefix, the newer disk file and the
    // cached hot doc. Their merged head is not the cached head, so the cheap
    // same-head comparison is skipped and the fallback forks.
    let rewritten = cold::read(&tv.0, &page).unwrap();
    let disk_now = fs::read(v.inner().doc_path(&page)).unwrap();
    let hot = v.inner().read(&page, |pd| pd.doc.export(ExportMode::all_updates()).unwrap()).unwrap();
    let unchecked = PageDoc::from_bytes(&rewritten.updates, &[disk_now, hot]).unwrap();
    assert!(!unchecked.doc.is_shallow());
    assert_ne!(unchecked.doc.oplog_frontiers(), frontiers, "the newer disk op must move the rebuilt head off the cached head");

    // The fallback's fork at the cached head has the wrong state, so the refusal
    // is the state comparison's own error. Without that comparison the rebuild
    // would succeed here.
    let err = v.inner().read(&page, |pd| pd.rehydrate()).unwrap().unwrap_err();
    assert_eq!(err, VaultError::Invalid("cold history does not reconstruct the hot page".into()));
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap(), "a refused rebuild must leave the page shallow");

    // Restoring the real prefix lets the rebuild succeed and keeps the newer op.
    cold::write(&tv.0, &page, &good).unwrap();
    v.inner().read(&page, |pd| pd.rehydrate()).unwrap().unwrap();
    assert!(!v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap());
    assert_eq!(v.inner().read(&page, |pd| pd.meta().title).unwrap(), "Newer");
}

/// A cold prefix that lacks versions the root covers (creation and the first
/// edit only; the root also covers the second and third edits) must not rebuild.
/// The rebuilt version does not match the cached one, and the rebuild is refused.
#[test]
fn missing_prefix_versions_refuse_rebuild() {
    let tv = TempVault::new("missing_prefix");
    let v = tv.open();
    let t = now_ms() / 1000;
    let page = new_id();
    let chain = import_history(&v, &page, "Cold", None, t);
    recent(&v, &page, "r1", "Recent");
    v.flush().unwrap();
    let whole = raw_view(&raw(&v.export_updates(page.clone(), Vec::new()).unwrap()));
    assert!(split(&v, &page, &policy(0), now_ms()));
    let v = reopen(&tv, v);
    assert_shallow_cached(&v, &page);

    let good = cold::read(&tv.0, &page).unwrap();
    cold::write(&tv.0, &page, &cold::ColdPart { root: good.root.clone(), updates: chain.s1.clone() }).unwrap();
    assert!(v.inner().read_history(&page, |_| ()).is_err(), "a prefix missing root versions must refuse the rebuild");
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap(), "a refused rebuild must leave the page shallow");

    cold::write(&tv.0, &page, &good).unwrap();
    let (deep, frontiers, vv, _) = full_view(&v, &page);
    assert_eq!((deep, frontiers, vv), (whole.0, whole.1, whole.2));
}

/// A flipped byte in the cold file's payload is refused by the checksum before
/// any rebuild happens. The page stays cached as shallow, and the restored file
/// rebuilds the whole page.
#[test]
fn corrupt_cold_prefix_refuses_rebuild_and_keeps_page_shallow() {
    let tv = TempVault::new("corrupt_prefix");
    let t = now_ms() / 1000;
    let (v, page, deep, frontiers, vv) = creation_only_split(&tv, t);
    let path = cold_file(&tv, &page);
    let good_bytes = fs::read(&path).unwrap();
    let mut bad_bytes = good_bytes.clone();
    let last = bad_bytes.len() - 1;
    bad_bytes[last] ^= 0x01;
    fs::write(&path, &bad_bytes).unwrap();

    let err = v.inner().read_history(&page, |_| ()).unwrap_err();
    assert!(err.to_string().contains("checksum"), "expected a checksum refusal, got {err}");
    assert!(v.inner().read(&page, |pd| pd.doc.is_shallow()).unwrap(), "a refused rebuild must leave the page shallow");

    fs::write(&path, &good_bytes).unwrap();
    let (deep2, frontiers2, vv2, _) = full_view(&v, &page);
    assert_eq!((deep2, frontiers2, vv2), (deep, frontiers, vv));
}
