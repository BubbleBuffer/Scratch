//! Real Loro 1.16.2 shallow-history boundaries (Cargo.lock), not a sync mock.
//! Scenarios 1--8 below correspond to the storage investigation assignment.
//! Raw LoroDoc is necessary to retain ImportStatus and inspect shallow/history
//! state: PageDoc is private and Vault discards ImportStatus. Where useful we
//! additionally exercise Vault's actual sync.rs export/import adapters.
//! PageDoc uses LoroText, LoroMap and LoroTree (doc.rs:3--26). These fixtures
//! exercise text plus page/props maps; the adapter baseline uses real blocks.
//!
//! Source agreement (paths relative to ~/.cargo/registry/src/*/):
//! - loro-1.16.2/src/lib.rs:1285 documents the old-history import limit.
//! - loro-internal-1.16.2/src/encoding/shallow_snapshot.rs:300--307 retains
//!   the root op itself; its predecessors are excluded from the operation log.
//! - loro-internal-1.16.2/src/oplog/loro_dag.rs:796--832 defines rejection
//!   by causal deps, including a branch concurrent with the root op.
//! - loro-internal-1.16.2/src/oplog/change_store.rs:726--730 clamps old exports.
//! - loro-internal-1.16.2/src/encoding/fast_snapshot.rs:350--354 decodes a full
//!   snapshot as updates for a nonempty doc; it does not restore omitted ops.
//! - loro-common-1.16.0/src/error.rs:92--97 defines the import/checkout errors.

use loro::{ExportMode, Frontiers, ImportStatus, LoroDoc, LoroError, VersionVector, ID};
use std::{fs, path::PathBuf};
use vault_core::{new_id, BlockKind, BlockOp, NewBlock, Vault, VaultError};

const D_PEER: u64 = 100;
const P_PEER: u64 = 200;
const O_PEER: u64 = 300;
const PAGE: &str = "shallow-investigation";

struct Peers {
    d: LoroDoc,
    p: LoroDoc,
    o: LoroDoc,
    old: Frontiers,
    old_vv: VersionVector,
    root: Frontiers,
    shallow: Vec<u8>,
}

fn full_copy(doc: &LoroDoc, peer: u64) -> LoroDoc {
    let copy = LoroDoc::new();
    let status = copy
        .import(&doc.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    assert_eq!(status.pending, None);
    copy.set_peer_id(peer).unwrap();
    copy.set_change_merge_interval(0);
    copy
}

fn edit(doc: &LoroDoc, marker: &str) {
    // All devices insert at the same position in the same text. Unique map
    // keys make acceptance visible independently of text merge ordering.
    doc.get_text("text").insert(0, marker).unwrap();
    doc.get_map("props").insert(marker, true).unwrap();
    doc.commit();
}

fn fixture() -> Peers {
    let d = LoroDoc::new();
    d.set_peer_id(D_PEER).unwrap();
    d.set_change_merge_interval(0);
    let page = d.get_map("page");
    page.insert("id", PAGE).unwrap();
    page.insert("title", "Investigation").unwrap();
    page.insert("created", 1_i64).unwrap();
    page.insert("updated", 1_i64).unwrap();
    d.get_text("text").insert(0, "base").unwrap();
    d.commit();
    let old = d.oplog_frontiers();
    let old_vv = d.oplog_vv();
    let o = full_copy(&d, O_PEER);
    edit(&d, "[before-R]");
    let root = d.oplog_frontiers();
    edit(&d, "[after-R]");
    let shallow = d.export(ExportMode::shallow_snapshot(&root)).unwrap();
    let p = LoroDoc::new();
    let status = p.import(&shallow).unwrap();
    assert_eq!(status.pending, None);
    p.set_peer_id(P_PEER).unwrap();
    p.set_change_merge_interval(0);
    assert!(
        p.is_shallow(),
        "must actually trim history, not fall back to full"
    );
    assert!(!p.shallow_since_vv().is_empty());
    assert!(p.get_change(ID::new(D_PEER, 0)).is_none());
    // 1.16.2 retains the frontier op itself, but trims its predecessors.
    let root_id = root.as_single().unwrap();
    assert!(p.get_change(root_id).is_some());
    assert!(p
        .get_change(ID::new(root_id.peer, root_id.counter - 1))
        .is_none());
    assert_eq!(p.oplog_vv(), d.oplog_vv());
    assert_eq!(p.oplog_frontiers(), d.oplog_frontiers());
    assert_eq!(p.get_deep_value(), d.get_deep_value());
    Peers {
        d,
        p,
        o,
        old,
        old_vv,
        root,
        shallow,
    }
}

fn updates(from: &LoroDoc, to: &LoroDoc) -> Vec<u8> {
    // Same nonempty-VV branch as Vault::export_updates in sync.rs:313--317.
    from.export(ExportMode::updates(&to.oplog_vv())).unwrap()
}

fn send(from: &LoroDoc, to: &LoroDoc) -> ImportStatus {
    let status = to.import(&updates(from, to)).unwrap();
    assert_eq!(status.pending, None);
    status
}

fn equal(a: &LoroDoc, b: &LoroDoc) {
    assert_eq!(
        a.get_text("text").to_string(),
        b.get_text("text").to_string()
    );
    assert_eq!(a.get_deep_value(), b.get_deep_value());
    assert_eq!(a.oplog_vv(), b.oplog_vv());
    assert_eq!(a.state_vv(), a.oplog_vv());
    assert_eq!(b.state_vv(), b.oplog_vv());
    assert_eq!(a.oplog_frontiers(), b.oplog_frontiers());
}

fn rejected(doc: &LoroDoc, bytes: &[u8]) {
    let state = doc.get_deep_value();
    let vv = doc.oplog_vv();
    let heads = doc.oplog_frontiers();
    let error = doc.import(bytes).unwrap_err();
    assert!(matches!(
        error,
        LoroError::ImportUpdatesThatDependsOnOutdatedVersion
    ));
    println!("rejected: {error}");
    assert_eq!(doc.get_deep_value(), state);
    assert_eq!(doc.oplog_vv(), vv);
    assert_eq!(doc.state_vv(), vv);
    assert_eq!(doc.oplog_frontiers(), heads);
}

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("vault_shallow_{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn vault_vv(vault: &Vault, page: &str) -> Vec<u8> {
    vault
        .page_versions()
        .unwrap()
        .into_iter()
        .find(|v| v.page == page)
        .unwrap()
        .version
}

/// 1: Shallow P's new edits retain enough causal context for a full D.
#[test]
fn phone_edits_converge() {
    let f = fixture();
    edit(&f.p, "[phone]");
    send(&f.p, &f.d);
    equal(&f.p, &f.d);
    assert!(f.d.get_text("text").to_string().contains("[phone]"));

    // Also use actual Vault sync.rs helpers with a real page/tree/text layout.
    let dt = TempDir::new();
    let pt = TempDir::new();
    let d = dt.open();
    let p = pt.open();
    let page = d.create_page(None, "Baseline".into()).unwrap();
    d.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: "a".into(),
                kind: BlockKind::Paragraph,
                text: "base".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    let raw = LoroDoc::new();
    raw.import(&d.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    let root = raw.oplog_frontiers();
    d.rename_page(page.clone(), "After root".into()).unwrap();
    raw.import(
        &d.export_updates(page.clone(), raw.oplog_vv().encode())
            .unwrap(),
    )
    .unwrap();
    let shallow = raw.export(ExportMode::shallow_snapshot(&root)).unwrap();
    let proof = LoroDoc::new();
    proof.import(&shallow).unwrap();
    assert!(proof.is_shallow());
    p.import_updates(page.clone(), shallow).unwrap();
    p.apply(
        page.clone(),
        vec![BlockOp::SetText {
            id: "a".into(),
            text: "phone edit".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    let bytes = p.export_updates(page.clone(), vault_vv(&d, &page)).unwrap();
    d.import_updates(page.clone(), bytes).unwrap();
    assert_eq!(
        d.load_page(page.clone()).unwrap().blocks[0].text,
        "phone edit"
    );
    assert_eq!(vault_vv(&d, &page), vault_vv(&p, &page));
    println!("scenario 1: raw Loro + Vault helpers converge, including block text");
}

/// 2: D's new changes causally descend from the shallow root.
#[test]
fn desktop_edits_converge() {
    let f = fixture();
    edit(&f.d, "[desktop]");
    send(&f.d, &f.p);
    equal(&f.d, &f.p);
    let dt = TempDir::new();
    let pt = TempDir::new();
    let d = dt.open();
    let p = pt.open();
    d.import_updates(PAGE.into(), f.d.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    p.import_updates(PAGE.into(), f.shallow).unwrap();
    d.rename_page(PAGE.into(), "desktop after R".into())
        .unwrap();
    let bytes = d.export_updates(PAGE.into(), vault_vv(&p, PAGE)).unwrap();
    p.import_updates(PAGE.into(), bytes).unwrap();
    assert_eq!(p.load_page(PAGE.into()).unwrap().title, "desktop after R");
    assert_eq!(vault_vv(&p, PAGE), vault_vv(&d, PAGE));
    println!("scenario 2: raw Loro + Vault VV deltas converge");
}

/// 3: Full D can merge O, but routing O through D does not rebase O's deps.
#[test]
fn pre_root_edit_via_desktop_rejected() {
    let f = fixture();
    edit(&f.o, "[offline]");
    send(&f.o, &f.d);
    send(&f.d, &f.o);
    equal(&f.d, &f.o);
    let bytes = updates(&f.d, &f.p);
    rejected(&f.p, &bytes);
    rejected(&f.p, &bytes); // A retry is another error, not eventual delivery.
    assert!(!f.p.get_text("text").to_string().contains("[offline]"));
    assert!(f.d.get_text("text").to_string().contains("[offline]"));
    assert_ne!(f.p.get_deep_value(), f.d.get_deep_value());

    // Actual cold/hot Vault import path propagates Loro's failure as Crdt.
    let pt = TempDir::new();
    let p = pt.open();
    p.import_updates(PAGE.into(), f.shallow).unwrap();
    let before = vault_vv(&p, PAGE);
    let error = p.import_updates(PAGE.into(), bytes).unwrap_err();
    assert!(
        matches!(&error, VaultError::Crdt(m) if m == &LoroError::ImportUpdatesThatDependsOnOutdatedVersion.to_string())
    );
    assert_eq!(vault_vv(&p, PAGE), before);
    println!("scenario 3: full D/O converge; P rejects, Vault reports {error}");
}

/// 4: P cannot supply old history; O's old branch cannot be merged into P.
#[test]
fn direct_sync_rejects_old_branch_and_leaves_suffix_pending() {
    let f = fixture();
    edit(&f.o, "[offline]");
    let before = f.o.get_deep_value();
    let vv = f.o.oplog_vv();
    let export = f.p.export(ExportMode::updates(&vv)).unwrap();
    let status = f.o.import(&export).unwrap();
    assert!(status.pending.is_some());
    assert_eq!(f.o.get_deep_value(), before);
    assert_eq!(f.o.oplog_vv(), vv);
    let retry = f.o.import(&export).unwrap();
    assert!(retry.pending.is_some());
    assert_eq!(f.o.oplog_vv(), vv);
    println!("scenario 4: P exports retained suffix; O receives {status:?}");
    rejected(&f.p, &updates(&f.o, &f.p));
    assert_ne!(f.p.get_deep_value(), f.o.get_deep_value());
    // Pending is diagnosable and recoverable: D has the omitted prefix.
    send(&f.d, &f.o);
    assert!(f.o.get_text("text").to_string().contains("[after-R]"));
    assert!(f.o.get_text("text").to_string().contains("[offline]"));
    send(&f.o, &f.d);
    equal(&f.o, &f.d);
}

/// 5: D can answer P's VV. A shallow P has no complete older-VV delta.
#[test]
fn old_vv_export_is_partial() {
    let f = fixture();
    let vv = f.p.oplog_vv();
    assert_eq!(
        f.d.frontiers_to_vv(&f.p.oplog_frontiers()),
        Some(vv.clone())
    );
    edit(&f.d, "[later]");
    let status =
        f.p.import(&f.d.export(ExportMode::updates(&vv)).unwrap())
            .unwrap();
    assert_eq!(status.pending, None);
    equal(&f.p, &f.d);
    println!(
        "scenario 5: root {:?}, shallow start {:?}",
        f.root,
        f.p.shallow_since_frontiers()
    );
    let partial = f.p.export(ExportMode::updates(&f.old_vv)).unwrap();
    let metadata = LoroDoc::decode_import_blob_meta(&partial, true).unwrap();
    let start = *metadata.partial_start_vv.get(&D_PEER).unwrap();
    assert_eq!(start, f.root.as_single().unwrap().counter);
    assert!(start > *f.old_vv.get(&D_PEER).unwrap());
    let status = f.o.import(&partial).unwrap();
    assert!(status.pending.is_some());
    println!("scenario 5: older-VV export silently clamps start to {start}; import: {status:?}");

    // Vault uses the same Updates mode for a nonempty encoded VV. Empty
    // bytes select Snapshot instead, but that snapshot also remains shallow.
    let pt = TempDir::new();
    let p = pt.open();
    p.import_updates(PAGE.into(), f.shallow).unwrap();
    let bytes = p.export_updates(PAGE.into(), f.old_vv.encode()).unwrap();
    let meta = LoroDoc::decode_import_blob_meta(&bytes, true).unwrap();
    assert_eq!(meta.partial_start_vv.get(&D_PEER), Some(&start));
    let copy = LoroDoc::new();
    copy.import(&p.export_updates(PAGE.into(), vec![]).unwrap())
        .unwrap();
    assert!(
        copy.is_shallow(),
        "Snapshot mode cannot fabricate pruned history"
    );
}

/// 6: Concurrent edits whose causal bases both include R are supported.
#[test]
fn post_root_concurrent_edits_converge() {
    let f = fixture();
    edit(&f.d, "[desktop-offline]");
    edit(&f.p, "[phone-offline]");
    send(&f.p, &f.d);
    send(&f.d, &f.p);
    equal(&f.p, &f.d);
    let text = f.p.get_text("text").to_string();
    assert!(text.contains("[desktop-offline]") && text.contains("[phone-offline]"));
    println!("scenario 6: raw Loro concurrent text/maps converge: {text}");
}

/// 7: Probe in-place hydration and lossless rebuild via a fresh full doc.
#[test]
fn recovery_requires_rebuild_preserving_phone_edits() {
    let f = fixture();
    edit(&f.p, "[unsent-phone]");
    let compatible = f.d.export(ExportMode::Snapshot).unwrap();
    let shallow_start = f.p.shallow_since_vv();
    let status = f.p.import(&compatible).unwrap();
    assert_eq!(status.pending, None);
    assert!(f.p.is_shallow());
    assert_eq!(f.p.shallow_since_vv(), shallow_start);
    assert!(f.p.get_change(ID::new(D_PEER, 0)).is_none());
    // Importing all compatible old updates also leaves the cutoff in place.
    let status =
        f.p.import(&f.d.export(ExportMode::all_updates()).unwrap())
            .unwrap();
    assert_eq!(status.pending, None);
    assert_eq!(f.p.shallow_since_vv(), shallow_start);
    edit(&f.o, "[offline]");
    send(&f.o, &f.d);
    let snapshot = f.d.export(ExportMode::Snapshot).unwrap();
    rejected(&f.p, &snapshot);
    assert!(f.p.is_shallow());
    assert!(matches!(
        f.p.checkout(&f.old),
        Err(LoroError::SwitchToVersionBeforeShallowRoot)
    ));
    f.p.checkout_to_latest();
    rejected(&f.p, &f.d.export(ExportMode::all_updates()).unwrap());
    assert!(f.p.is_shallow());
    // Fresh full state + P's unsent delta: no overwrite of local changes.
    let recovered = LoroDoc::new();
    recovered.import(&snapshot).unwrap();
    recovered.set_peer_id(P_PEER).unwrap();
    send(&f.p, &recovered);
    send(&recovered, &f.d);
    equal(&recovered, &f.d);
    assert!(!recovered.is_shallow());
    recovered.checkout(&f.old).unwrap();
    assert_eq!(recovered.get_text("text").to_string(), "base");
    recovered.checkout_to_latest();
    equal(&recovered, &f.d);
    assert!(recovered
        .get_text("text")
        .to_string()
        .contains("[unsent-phone]"));
    println!("scenario 7: in-place recovery rejected; fresh full doc + unsent phone delta restores full history");
}

/// 8: Compacted history is unavailable, not a bogus historical rendering.
#[test]
fn pre_root_checkout_rejected_and_listing_empty() {
    let f = fixture();
    let error = f.p.checkout(&f.old).unwrap_err();
    assert!(matches!(error, LoroError::SwitchToVersionBeforeShallowRoot));
    println!("scenario 8: {error}");
    f.p.checkout_to_latest();
    assert!(f.d.get_change(ID::new(D_PEER, 0)).is_some());
    assert!(f.p.get_change(ID::new(D_PEER, 0)).is_none());
    assert!(f.p.get_change(f.root.as_single().unwrap()).is_some());
    // len_changes() counts stored change metadata, not accessible versions;
    // test actual lookup/checkout and the public history listing instead.
    println!(
        "scenario 8: change counts: shallow={}, full={}",
        f.p.len_changes(),
        f.d.len_changes()
    );
    let pt = TempDir::new();
    let p = pt.open();
    p.import_updates(PAGE.into(), f.shallow).unwrap();
    let version = f
        .old
        .iter()
        .map(|id| format!("{}@{}", id.peer, id.counter))
        .collect::<Vec<_>>()
        .join(",");
    let error = p.load_page_at(PAGE.into(), version).unwrap_err();
    println!("scenario 8: Vault load_page_at error: {error:?}");
    // fork_at exports SnapshotAt first. Before the root this reports missing
    // frontiers, unlike checkout's dedicated shallow-boundary error.
    // loro-internal/src/fork.rs:38--43; encoding.rs:424--427;
    // loro-common/src/error.rs:204--208 (all versions pinned above).
    let fork_error = match f.p.fork_at(&f.old) {
        Err(error) => error,
        Ok(_) => panic!("fork_at unexpectedly recovered pruned history"),
    };
    let frontiers = format!("{:?}", f.old);
    assert!(matches!(&fork_error, LoroError::NotFoundError(m) if m.as_ref() == frontiers));
    assert!(matches!(&error, VaultError::Crdt(m) if m == &fork_error.to_string()));
    let listed = p.list_versions(PAGE.into()).unwrap();
    assert!(
        listed.is_empty(),
        "history.rs starts each peer at counter 0, now pruned"
    );
    println!("scenario 8: raw history has retained changes; Vault history listing is empty");
}

/// 9: Documentation's rejection is not an atomic transaction guarantee.
/// loro-internal-1.16.2/src/loro.rs:775--886 applies a valid sub-batch before
/// returning ImportUpdatesThatDependsOnOutdatedVersion for excluded deps.
#[test]
fn mixed_import_errors_after_partial_application() {
    let f = fixture();
    edit(&f.d, "[valid-desktop]");
    edit(&f.o, "[invalid-offline]");
    send(&f.o, &f.d);
    let error = f.p.import(&updates(&f.d, &f.p)).unwrap_err();
    assert!(matches!(
        error,
        LoroError::ImportUpdatesThatDependsOnOutdatedVersion
    ));
    let text = f.p.get_text("text").to_string();
    assert!(text.contains("[valid-desktop]"));
    assert!(!text.contains("[invalid-offline]"));
    assert_eq!(f.p.state_vv(), f.p.oplog_vv());
    assert_ne!(f.p.oplog_vv(), f.d.oplog_vv());
    // Repeating just the rejected branch fails again; it was not queued.
    rejected(&f.p, &updates(&f.d, &f.p));
    println!("scenario 9: valid text/map ops applied despite Err; invalid ops rejected: {text}");

    // Vault must persist the accepted portion even while propagating the
    // original import error. Replaying the rejected branch is not recovery.
    let pt = TempDir::new();
    let applied;
    {
        let p = pt.open();
        p.import_updates(PAGE.into(), f.shallow).unwrap();
        let original = vault_vv(&p, PAGE);
        let before = VersionVector::decode(&original).unwrap();
        let bytes = f.d.export(ExportMode::updates(&before)).unwrap();
        let error = p.import_updates(PAGE.into(), bytes).unwrap_err();
        assert!(matches!(
            &error,
            VaultError::Crdt(message)
                if message == &LoroError::ImportUpdatesThatDependsOnOutdatedVersion.to_string()
        ));
        applied = vault_vv(&p, PAGE);
        assert_ne!(applied, original);
        assert_eq!(VersionVector::decode(&applied).unwrap(), f.p.oplog_vv());
        let live = LoroDoc::new();
        live.import(&p.export_updates(PAGE.into(), vec![]).unwrap())
            .unwrap();
        let text = live.get_text("text").to_string();
        assert!(text.contains("[valid-desktop]"));
        assert!(!text.contains("[invalid-offline]"));
        p.flush().unwrap();
    }
    let reopened = pt.open();
    assert_eq!(vault_vv(&reopened, PAGE), applied);
    let persisted = LoroDoc::new();
    persisted
        .import(&reopened.export_updates(PAGE.into(), vec![]).unwrap())
        .unwrap();
    let text = persisted.get_text("text").to_string();
    assert!(text.contains("[valid-desktop]"));
    assert!(!text.contains("[invalid-offline]"));
    assert_eq!(persisted.get_deep_value(), f.p.get_deep_value());
    println!("scenario 9: Vault preserves only the accepted portion across flush/reopen");
}

/// 9a: The retained root must be in the edit's causal past. Merely having
/// the root's predecessor state is insufficient, even at that exact boundary.
#[test]
fn root_concurrent_edit_rejected_root_descendant_accepted() {
    let f = fixture();
    let root = f.root.as_single().unwrap();
    let predecessor = Frontiers::from(ID::new(root.peer, root.counter - 1));
    let concurrent = f.d.fork_at(&predecessor).unwrap();
    concurrent.set_peer_id(500).unwrap();
    edit(&concurrent, "[root-concurrent]");
    rejected(&f.p, &updates(&concurrent, &f.p));
    let descendant = f.d.fork_at(&f.root).unwrap();
    descendant.set_peer_id(600).unwrap();
    edit(&descendant, "[root-descendant]");
    send(&descendant, &f.p);
    send(&f.p, &f.d);
    equal(&f.p, &f.d);
    assert!(f
        .p
        .get_text("text")
        .to_string()
        .contains("[root-descendant]"));
    println!("scenario 9a: exact root boundary rejects concurrent, accepts descendant");
}

/// 9b: Missing retained deps produce pending, distinct from pruned deps.
#[test]
fn missing_post_root_deps_pending_then_converge() {
    let f = fixture();
    edit(&f.d, "[missing-post-root]");
    let middle = f.d.oplog_vv();
    edit(&f.d, "[dependent-post-root]");
    let before = f.p.oplog_vv();
    let status =
        f.p.import(&f.d.export(ExportMode::updates(&middle)).unwrap())
            .unwrap();
    assert!(status.pending.is_some());
    assert_eq!(f.p.oplog_vv(), before);
    assert_eq!(f.p.state_vv(), before);
    send(&f.d, &f.p);
    equal(&f.p, &f.d);
    println!("scenario 9b: retained out-of-order deps are pending, then converge");
}
