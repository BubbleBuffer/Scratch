//! Recovery after an interrupted new-page import. A batch import writes each
//! new page's snapshot file before its index transaction. If a sibling update
//! in the same batch is invalid, the batch fails, so the good page can be left
//! as a durable file with no index row. Opening the vault, or an identical
//! retry, must make such pages visible again, with their latest versions, and
//! must not touch rows that were already indexed.
//!
//! Everything here goes through the public API and the files under the vault
//! root (`.vault/docs`, `.vault/logs`, `.vault/purged`, `.vault/index.sqlite`).

use std::fs;
use std::path::PathBuf;

use loro::LoroDoc;
use vault_core::*;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("import_recovery_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
    fn doc_file(&self, id: &str) -> PathBuf {
        self.0
            .join(".vault")
            .join("docs")
            .join(format!("{id}.loro"))
    }
    fn log_file(&self, id: &str, old: bool) -> PathBuf {
        let name = if old {
            format!("{id}.log.old")
        } else {
            format!("{id}.log")
        };
        self.0.join(".vault").join("logs").join(name)
    }
    /// Writes one update as a log frame (u32 LE length, then the bytes), as the
    /// writer appends it.
    fn write_log(&self, id: &str, update: &[u8], old: bool) {
        let mut buf = (update.len() as u32).to_le_bytes().to_vec();
        buf.extend_from_slice(update);
        fs::create_dir_all(self.0.join(".vault").join("logs")).unwrap();
        fs::write(self.log_file(id, old), buf).unwrap();
    }
    /// Records purge tombstones in `.vault/purged`, one id per line.
    fn write_purged(&self, ids: &[&PageId]) {
        let text: String = ids.iter().map(|id| format!("{id}\n")).collect();
        fs::write(self.0.join(".vault").join("purged"), text).unwrap();
    }
    fn write_purge_records(&self, records: &[PurgeRecord]) {
        self.write_purged(&records.iter().map(|r| &r.page).collect::<Vec<_>>());
        fs::write(
            self.0.join(".vault/purge-state.json"),
            serde_json::to_vec(&serde_json::json!({ "records": records })).unwrap(),
        )
        .unwrap();
    }
    /// Deletes the SQLite index, so the next open starts from a fresh index.
    fn remove_index(&self) {
        let db = self.0.join(".vault").join("index.sqlite");
        for suffix in ["", "-wal", "-shm"] {
            let mut p = db.clone().into_os_string();
            p.push(suffix);
            let _ = fs::remove_file(PathBuf::from(p));
        }
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// True when the page is in the index (listed under the root).
fn listed(v: &Vault, id: &str) -> bool {
    title_of(v, id).is_some()
}

/// The page's title as the index lists it, if it is listed.
fn title_of(v: &Vault, id: &str) -> Option<String> {
    v.list_pages(None, 0, 10_000)
        .unwrap()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.title)
}

/// Every listed page id, sorted.
fn listed_ids(v: &Vault) -> Vec<PageId> {
    let mut ids: Vec<PageId> = v
        .list_pages(None, 0, 10_000)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    ids.sort();
    ids
}

/// The page's version as `page_versions` reports it, if present.
fn version_of(v: &Vault, id: &str) -> Option<Vec<u8>> {
    v.page_versions()
        .unwrap()
        .into_iter()
        .find(|p| p.page == id)
        .map(|p| p.version)
}

/// A flushed page on a source vault, ready to be sent as a snapshot.
struct Snap {
    id: PageId,
    bytes: Vec<u8>,
    page: Page,
    version: Vec<u8>,
}

/// Snapshots a page the source has already flushed.
fn snap(src: &Vault, id: &PageId) -> Snap {
    Snap {
        id: id.clone(),
        bytes: src.export_updates(id.clone(), Vec::new()).unwrap(),
        page: src.load_page(id.clone()).unwrap(),
        version: version_of(src, id).unwrap(),
    }
}

/// A page with a snapshot that a source vault exports for sync.
fn source_snapshot(title: &str) -> (Dir, Snap) {
    let dir = Dir::new("source");
    let src = dir.open();
    let id = src.create_page(None, title.to_owned()).unwrap();
    src.apply(
        id.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: new_id(),
                kind: BlockKind::Paragraph,
                text: "durable recovery body".to_owned(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    src.flush().unwrap();
    let s = snap(&src, &id);
    (dir, s)
}

/// A later rename of the snapshot's page on the source, as a sync update
/// over the snapshot. Returns the update and the source's latest version.
fn later_update(src: &Vault, s: &Snap, title: &str) -> (Vec<u8>, Vec<u8>) {
    let base = LoroDoc::new();
    base.import(&s.bytes).unwrap();
    let since = base.oplog_vv().encode();
    src.rename_page(s.id.clone(), title.to_owned()).unwrap();
    src.flush().unwrap();
    let update = src.export_updates(s.id.clone(), since).unwrap();
    (update, version_of(src, &s.id).unwrap())
}

/// A receiver with one indexed page, flushed and closed. Returns its id and
/// version, which must survive every later open unchanged.
fn receiver_with_existing(dir: &Dir) -> (PageId, Vec<u8>) {
    let v = dir.open();
    let id = v.create_page(None, "Existing page".to_owned()).unwrap();
    v.flush().unwrap();
    let version = version_of(&v, &id).unwrap();
    (id, version)
}

#[test]
fn interrupted_new_page_batch_indexes_durable_snapshot_on_retry() {
    let (_source_dir, s) = source_snapshot("Imported page");

    let dir = Dir::new("retry");
    let (existing, existing_version) = receiver_with_existing(&dir);
    let receiver = dir.open();

    // The good page is written first, then the invalid sibling fails the batch.
    let batch = vec![
        PageUpdate {
            page: s.id.clone(),
            bytes: s.bytes.clone(),
        },
        PageUpdate {
            page: "bad-sibling".to_owned(),
            bytes: b"not a loro document".to_vec(),
        },
    ];
    assert!(
        receiver.import_updates_batch(batch).is_err(),
        "the invalid sibling must fail the batch"
    );
    assert!(
        dir.doc_file(&s.id).exists(),
        "the good snapshot must be durable before the failure"
    );
    assert!(
        !listed(&receiver, &s.id),
        "the interrupted page must have no index row yet"
    );

    // Identical retry: visible at once, before any flush.
    receiver
        .import_updates(s.id.clone(), s.bytes.clone())
        .unwrap();
    assert!(
        listed(&receiver, &s.id),
        "identical retry must index the snapshot before flush"
    );
    assert_eq!(title_of(&receiver, &s.id).as_deref(), Some("Imported page"));
    assert_eq!(
        version_of(&receiver, &s.id),
        Some(s.version.clone()),
        "version before flush"
    );
    assert_eq!(receiver.load_page(s.id.clone()).unwrap(), s.page);

    receiver.flush().unwrap();
    assert!(
        listed(&receiver, &s.id),
        "page must stay indexed after flush"
    );
    assert_eq!(
        version_of(&receiver, &s.id),
        Some(s.version.clone()),
        "version after flush"
    );
    assert_eq!(
        version_of(&receiver, &existing),
        Some(existing_version.clone())
    );
    drop(receiver);

    let reopened = dir.open();
    assert!(
        listed(&reopened, &s.id),
        "page must stay indexed after reopen"
    );
    assert_eq!(
        version_of(&reopened, &s.id),
        Some(s.version.clone()),
        "version after reopen"
    );
    assert_eq!(reopened.load_page(s.id.clone()).unwrap(), s.page);
    assert_eq!(version_of(&reopened, &existing), Some(existing_version));
}

/// Crash after the snapshot reached the disk and before the index commit.
/// Open must index the durable snapshot, with its version, and keep the
/// existing row exactly as it was.
#[test]
fn snapshot_left_without_index_row_is_recovered_on_open() {
    let (_source_dir, s) = source_snapshot("Crash page");
    let dir = Dir::new("crash");
    let (existing, existing_version) = receiver_with_existing(&dir);
    fs::write(dir.doc_file(&s.id), &s.bytes).unwrap();

    let reopened = dir.open();
    assert_eq!(title_of(&reopened, &s.id).as_deref(), Some("Crash page"));
    assert_eq!(version_of(&reopened, &s.id), Some(s.version.clone()));
    assert!(
        reopened
            .search_full("durable recovery body".to_owned(), 10)
            .unwrap()
            .iter()
            .any(|hit| hit.id == s.id),
        "recovered body must be searchable before flush"
    );
    assert_eq!(reopened.load_page(s.id.clone()).unwrap(), s.page);
    assert_eq!(
        title_of(&reopened, &existing).as_deref(),
        Some("Existing page")
    );
    assert_eq!(version_of(&reopened, &existing), Some(existing_version));
    let mut want = vec![existing, s.id];
    want.sort();
    assert_eq!(
        listed_ids(&reopened),
        want,
        "recovery must add only the orphan"
    );
}

/// Same crash, plus a pending operation in the page's log that is not in the
/// snapshot. The `.log` and `.log.old` names are both recovered. The index
/// must list the latest title and hold the latest version, not only the disk
/// document, which loads even without the fix.
fn pending_log_recovers_on_open(old: bool) {
    let (source_dir, s) = source_snapshot("Crash page");
    let src = source_dir.open();
    let (update, latest) = later_update(&src, &s, "Pending title");
    drop(src);

    let dir = Dir::new(if old { "crash_log_old" } else { "crash_log" });
    receiver_with_existing(&dir);
    fs::write(dir.doc_file(&s.id), &s.bytes).unwrap();
    dir.write_log(&s.id, &update, old);

    let reopened = dir.open();
    assert_eq!(
        title_of(&reopened, &s.id).as_deref(),
        Some("Pending title"),
        "index title"
    );
    assert_eq!(
        version_of(&reopened, &s.id),
        Some(latest.clone()),
        "index version"
    );
    assert_eq!(
        reopened.load_page(s.id.clone()).unwrap().title,
        "Pending title"
    );
    assert_eq!(reopened.page_versions().unwrap().len(), 2);
    drop(reopened);

    // The recovered version survives another open.
    let again = dir.open();
    assert_eq!(title_of(&again, &s.id).as_deref(), Some("Pending title"));
    assert_eq!(version_of(&again, &s.id), Some(latest));
}

#[test]
fn unindexed_snapshot_with_pending_log_recovers_on_open() {
    pending_log_recovers_on_open(false);
}

#[test]
fn unindexed_snapshot_with_pending_old_log_recovers_on_open() {
    pending_log_recovers_on_open(true);
}

/// A purge tombstone plus leftover nonempty logs (`.log` and `.log.old`) and
/// snapshots must not bring the pages back on open. An incoming legacy copy
/// is recoverable in marked Trash, without inheriting deleted disk edits.
#[test]
fn purged_leftover_nonempty_logs_are_not_resurrected() {
    let source_dir = Dir::new("source_purged");
    let src = source_dir.open();
    let a_id = src.create_page(None, "Purged log page".to_owned()).unwrap();
    let b_id = src
        .create_page(None, "Purged old log page".to_owned())
        .unwrap();
    src.flush().unwrap();
    let a = snap(&src, &a_id);
    let b = snap(&src, &b_id);
    let (ua, _) = later_update(&src, &a, "Purged log edit");
    let (ub, _) = later_update(&src, &b, "Purged old edit");
    drop(src);

    let dir = Dir::new("purged_logs");
    receiver_with_existing(&dir);
    fs::write(dir.doc_file(&a.id), &a.bytes).unwrap();
    fs::write(dir.doc_file(&b.id), &b.bytes).unwrap();
    dir.write_log(&a.id, &ua, false);
    dir.write_log(&b.id, &ub, true);
    dir.write_purged(&[&a.id, &b.id]);

    let receiver = dir.open();
    for id in [&a.id, &b.id] {
        assert!(!listed(&receiver, id), "open must not index a purged page");
        assert_eq!(version_of(&receiver, id), None);
        assert!(receiver.purged_pages().contains(id));
    }
    receiver
        .import_purge_records(receiver.purge_records())
        .unwrap();
    assert!(
        receiver.restore_records().is_empty(),
        "replayed IDs are not edits"
    );
    drop(receiver);
    let receiver = dir.open();
    assert_eq!(version_of(&receiver, &a.id), None);
    assert_eq!(version_of(&receiver, &b.id), None);
    let report = receiver
        .import_updates(a.id.clone(), a.bytes.clone())
        .unwrap();
    assert_eq!(report.skipped, 0);
    let restored = receiver.load_page(a.id.clone()).unwrap();
    assert!(restored.in_trash);
    assert_eq!(
        restored.title, a.page.title,
        "deleted log must not be merged"
    );
    assert_eq!(
        receiver.restored_after_purge_pages().unwrap(),
        vec![a.id.clone()]
    );
    assert!(!receiver.purged_pages().contains(&a.id));
    assert!(!dir.log_file(&a.id, false).exists());
    assert!(!dir.log_file(&a.id, true).exists());
    receiver.flush().unwrap();
    let restored_version = version_of(&receiver, &a.id).unwrap();
    drop(receiver);

    let reopened = dir.open();
    assert!(
        !listed(&reopened, &a.id),
        "restored legacy copy remains in Trash"
    );
    assert_eq!(
        reopened.load_page(a.id.clone()).unwrap().title,
        a.page.title
    );
    assert_eq!(version_of(&reopened, &a.id), Some(restored_version));
    assert!(!reopened.purged_pages().contains(&a.id));
    assert_eq!(reopened.restored_after_purge_pages().unwrap(), vec![a.id]);
    assert!(
        !listed(&reopened, &b.id),
        "unimported legacy leftover stays deleted"
    );
    assert_eq!(version_of(&reopened, &b.id), None);
    assert!(reopened.purged_pages().contains(&b.id));
}

#[test]
fn causal_covered_leftover_logs_allow_incoming_editor_restore() {
    for old in [false, true] {
        let (source_dir, s) = source_snapshot("Covered leftover");
        let src = source_dir.open();
        let purger_dir = Dir::new("covered_purger");
        let purger = purger_dir.open();
        purger
            .import_updates(s.id.clone(), s.bytes.clone())
            .unwrap();
        purger
            .apply(
                s.id.clone(),
                vec![BlockOp::Delete {
                    id: s.page.blocks[0].id.clone(),
                }],
            )
            .unwrap();
        purger.delete_page(s.id.clone()).unwrap();
        let purge_delta = purger
            .export_updates(s.id.clone(), s.version.clone())
            .unwrap();
        purger.purge_page(s.id.clone()).unwrap();
        let records = purger.purge_records();

        let dir = Dir::new("covered_leftover");
        receiver_with_existing(&dir);
        fs::write(dir.doc_file(&s.id), &s.bytes).unwrap();
        dir.write_log(&s.id, &purge_delta, old);
        dir.write_purge_records(&records);
        for _ in 0..2 {
            let receiver = dir.open();
            assert_eq!(version_of(&receiver, &s.id), None);
            assert!(receiver.purged_pages().contains(&s.id));
            assert!(receiver.restore_records().is_empty());
            assert_eq!(
                receiver
                    .import_updates(s.id.clone(), s.bytes.clone())
                    .unwrap()
                    .skipped,
                1
            );
        }
        src.rename_page(s.id.clone(), "New editor title".to_owned())
            .unwrap();
        src.import_purge_records(records.clone()).unwrap();
        let record = src.restore_records().remove(0);
        let incoming = src.export_updates(s.id.clone(), Vec::new()).unwrap();
        let receiver = dir.open();
        receiver
            .import_restored_updates(record.clone(), incoming.clone())
            .unwrap();
        let restored = receiver.load_page(s.id.clone()).unwrap();
        assert!(!restored.in_trash);
        assert_eq!(restored.title, "New editor title");
        assert_eq!(
            restored.blocks, s.page.blocks,
            "purge-side block deletion must not replay"
        );
        assert_eq!(receiver.restore_records(), vec![record.clone()]);
        assert!(receiver.restored_after_purge_pages().unwrap().is_empty());
        assert!(!dir.log_file(&s.id, false).exists());
        assert!(!dir.log_file(&s.id, true).exists());
        receiver.flush().unwrap();
        let version = version_of(&receiver, &s.id).unwrap();
        assert!(version_includes(&version, &record.version).unwrap());
        drop(receiver);
        let receiver = dir.open();
        assert_eq!(receiver.load_page(s.id.clone()).unwrap(), restored);
        assert_eq!(version_of(&receiver, &s.id), Some(version));
        assert!(!receiver.purged_pages().contains(&s.id));
        receiver.import_purge_records(records).unwrap();
        assert!(
            !receiver.purged_pages().contains(&s.id),
            "delayed purge stays acknowledged"
        );
    }
}

#[test]
fn causal_uncovered_leftover_logs_restore_live_on_open() {
    for old in [false, true] {
        let (source_dir, s) = source_snapshot("Uncovered leftover");
        let src = source_dir.open();
        let (update, latest) = later_update(&src, &s, "Durable new editor title");
        let dir = Dir::new("uncovered_leftover");
        receiver_with_existing(&dir);
        fs::write(dir.doc_file(&s.id), &s.bytes).unwrap();
        dir.write_log(&s.id, &update, old);
        let records = vec![PurgeRecord {
            page: s.id.clone(),
            version: Some(s.version.clone()),
            device: None,
        }];
        dir.write_purge_records(&records);

        let receiver = dir.open();
        let restored = receiver.load_page(s.id.clone()).unwrap();
        assert_eq!(
            title_of(&receiver, &s.id).as_deref(),
            Some("Durable new editor title")
        );
        assert!(!restored.in_trash);
        assert_eq!(restored.blocks, s.page.blocks);
        assert!(!receiver.purged_pages().contains(&s.id));
        assert!(receiver.restored_after_purge_pages().unwrap().is_empty());
        let proof = receiver.restore_records().remove(0);
        assert_eq!(proof.purge_version, Some(s.version.clone()));
        assert!(
            version_includes(&proof.version, &latest).unwrap(),
            "all editor history is retained"
        );
        assert!(!dir.log_file(&s.id, false).exists());
        assert!(!dir.log_file(&s.id, true).exists());
        let doc = LoroDoc::new();
        doc.import(&receiver.export_updates(s.id.clone(), Vec::new()).unwrap())
            .unwrap();
        assert!(
            doc.get_map("page").get("purge_restore").is_some(),
            "same restore generation marker as incoming edits"
        );
        assert!(doc.get_map("page").get("purge_restore_warning").is_none());
        receiver.flush().unwrap();
        let version = version_of(&receiver, &s.id).unwrap();
        drop(receiver);
        let receiver = dir.open();
        assert_eq!(receiver.load_page(s.id.clone()).unwrap(), restored);
        assert_eq!(version_of(&receiver, &s.id), Some(version));
        assert_eq!(receiver.restore_records(), vec![proof]);
        receiver.import_purge_records(records).unwrap();
        assert!(!receiver.purged_pages().contains(&s.id));
    }
}

#[test]
fn incoming_restore_still_rejects_independent_live_disk_log() {
    for old in [false, true] {
        let (source_dir, s) = source_snapshot("Live base");
        let src = source_dir.open();
        later_update(&src, &s, "Incoming title");
        let incoming = src.export_updates(s.id.clone(), Vec::new()).unwrap();
        let record = RestoreRecord {
            page: s.id.clone(),
            purge_version: Some(s.version.clone()),
            version: version_of(&src, &s.id).unwrap(),
        };

        let dir = Dir::new("live_disk_guard");
        let receiver = dir.open();
        receiver
            .import_updates(s.id.clone(), s.bytes.clone())
            .unwrap();
        receiver.flush().unwrap();
        receiver.load_page(s.id.clone()).unwrap(); // Cache predates the durable foreign log.
        let disk_editor_dir = Dir::new("disk_editor");
        let disk_editor = disk_editor_dir.open();
        disk_editor
            .import_updates(s.id.clone(), s.bytes.clone())
            .unwrap();
        let (delta, _) = later_update(&disk_editor, &s, "Independent disk title");
        dir.write_log(&s.id, &delta, old);
        let before = fs::read(dir.doc_file(&s.id)).unwrap();
        let error = receiver
            .import_restored_updates(record, incoming)
            .unwrap_err();
        assert!(
            error.to_string().contains("independent disk edits"),
            "{error}"
        );
        assert_eq!(fs::read(dir.doc_file(&s.id)).unwrap(), before);
        assert!(dir.log_file(&s.id, old).exists());
        assert!(receiver.restore_records().is_empty());
        drop(receiver);
        let receiver = dir.open();
        assert_eq!(
            receiver.load_page(s.id.clone()).unwrap().title,
            "Independent disk title"
        );
    }
}

#[test]
fn reindex_recovers_uncovered_log_arriving_after_open_with_a_cached_leftover() {
    for old in [false, true] {
        let (source_dir, s) = source_snapshot("Covered at open");
        let src = source_dir.open();
        let (delta, latest) = later_update(&src, &s, "Durable after open");
        let dir = Dir::new("reindex_purged_log");
        receiver_with_existing(&dir);
        fs::write(dir.doc_file(&s.id), &s.bytes).unwrap();
        dir.write_purge_records(&[PurgeRecord {
            page: s.id.clone(),
            version: Some(s.version.clone()),
            device: None,
        }]);
        let receiver = dir.open();
        assert_eq!(version_of(&receiver, &s.id), None);
        // Loading the physical leftover creates a covered cache entry; the
        // independent durable edit subsequently appears only in the log.
        receiver.load_page(s.id.clone()).unwrap();
        dir.write_log(&s.id, &delta, old);
        receiver.reindex().unwrap();
        assert_eq!(
            title_of(&receiver, &s.id).as_deref(),
            Some("Durable after open")
        );
        assert!(!receiver.load_page(s.id.clone()).unwrap().in_trash);
        assert!(version_includes(&version_of(&receiver, &s.id).unwrap(), &latest).unwrap());
        assert!(!receiver.purged_pages().contains(&s.id));
        receiver.flush().unwrap();
        drop(receiver);
        assert_eq!(
            title_of(&dir.open(), &s.id).as_deref(),
            Some("Durable after open")
        );
    }
}

#[test]
fn causal_uncovered_snapshot_family_recovers_parent_before_child() {
    let source_dir = Dir::new("leftover_family_source");
    let src = source_dir.open();
    let parent = src.create_page(None, "Parent".to_owned()).unwrap();
    let child = src
        .create_page(Some(parent.clone()), "Child".to_owned())
        .unwrap();
    src.flush().unwrap();
    let p = snap(&src, &parent);
    let c = snap(&src, &child);
    src.rename_page(parent.clone(), "Edited parent".to_owned())
        .unwrap();
    src.rename_page(child.clone(), "Edited child".to_owned())
        .unwrap();
    src.flush().unwrap();
    let records = vec![
        PurgeRecord {
            page: child.clone(),
            version: Some(c.version),
            device: None,
        },
        PurgeRecord {
            page: parent.clone(),
            version: Some(p.version),
            device: None,
        },
    ];
    for fresh in [false, true] {
        let dir = Dir::new("leftover_family");
        receiver_with_existing(&dir);
        if fresh {
            dir.remove_index();
        }
        fs::write(
            dir.doc_file(&parent),
            src.export_updates(parent.clone(), Vec::new()).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.doc_file(&child),
            src.export_updates(child.clone(), Vec::new()).unwrap(),
        )
        .unwrap();
        dir.write_purge_records(&records);
        let receiver = dir.open();
        assert_eq!(
            receiver.load_page(child.clone()).unwrap().parent,
            Some(parent.clone())
        );
        assert_eq!(
            receiver.load_page(parent.clone()).unwrap().title,
            "Edited parent"
        );
        assert_eq!(
            receiver.load_page(child.clone()).unwrap().title,
            "Edited child"
        );
        assert!(receiver.purged_pages().is_empty());
        receiver.flush().unwrap();
        drop(receiver);
        let receiver = dir.open();
        assert_eq!(
            receiver.load_page(child.clone()).unwrap().parent,
            Some(parent.clone())
        );
        assert_eq!(receiver.restore_records().len(), 2);
    }
}

/// Fresh index (deleted SQLite file) with a purged page and an unpurged
/// orphan: the rebuild must index the orphan and skip the purged page.
#[test]
fn fresh_index_recovery_skips_purged_and_indexes_orphans() {
    let (_orphan_dir, orphan) = source_snapshot("Orphan page");
    let (_purged_dir, purged) = source_snapshot("Purged page");

    let dir = Dir::new("fresh_purge");
    let (existing, existing_version) = receiver_with_existing(&dir);
    dir.remove_index();
    fs::write(dir.doc_file(&orphan.id), &orphan.bytes).unwrap();
    fs::write(dir.doc_file(&purged.id), &purged.bytes).unwrap();
    dir.write_purged(&[&purged.id]);

    let reopened = dir.open();
    assert_eq!(
        title_of(&reopened, &existing).as_deref(),
        Some("Existing page")
    );
    assert_eq!(version_of(&reopened, &existing), Some(existing_version));
    assert_eq!(
        title_of(&reopened, &orphan.id).as_deref(),
        Some("Orphan page")
    );
    assert_eq!(
        version_of(&reopened, &orphan.id),
        Some(orphan.version.clone())
    );
    assert!(
        !listed(&reopened, &purged.id),
        "fresh-index rebuild must not list a purged page"
    );
    assert_eq!(version_of(&reopened, &purged.id), None);
    assert!(reopened.purged_pages().contains(&purged.id));
    let mut want = vec![existing, orphan.id];
    want.sort();
    assert_eq!(listed_ids(&reopened), want);
}

/// Proves recovery is not a wholesale rebuild. A known indexed page whose
/// snapshot file was removed must keep its index row, unchanged, when open
/// recovers another page. A rebuild from disk would drop it.
#[test]
fn indexed_page_with_missing_snapshot_keeps_its_row_when_another_page_recovers() {
    let (_orphan_dir, orphan) = source_snapshot("Orphan page");

    let dir = Dir::new("no_rebuild");
    let (known, known_version) = receiver_with_existing(&dir);
    fs::remove_file(dir.doc_file(&known)).unwrap();
    fs::write(dir.doc_file(&orphan.id), &orphan.bytes).unwrap();

    let reopened = dir.open();
    assert_eq!(
        title_of(&reopened, &known).as_deref(),
        Some("Existing page"),
        "row must stay"
    );
    assert_eq!(
        version_of(&reopened, &known),
        Some(known_version),
        "row version must stay"
    );
    assert_eq!(
        title_of(&reopened, &orphan.id).as_deref(),
        Some("Orphan page")
    );
    assert_eq!(
        version_of(&reopened, &orphan.id),
        Some(orphan.version.clone())
    );
    let mut want = vec![known, orphan.id];
    want.sort();
    assert_eq!(listed_ids(&reopened), want);
}

/// 226 missing snapshots in a nonempty index. Every one must be recovered
/// with its version, exactly once, and a second open must not duplicate them.
#[test]
fn many_missing_snapshots_recover_once_on_repeated_open() {
    const COUNT: usize = 226;
    let source_dir = Dir::new("source_many");
    let src = source_dir.open();
    let ids: Vec<PageId> = (0..COUNT)
        .map(|i| src.create_page(None, format!("Page {i}")).unwrap())
        .collect();
    src.flush().unwrap();
    let snaps: Vec<Snap> = ids.iter().map(|id| snap(&src, id)).collect();
    drop(src);

    let dir = Dir::new("many");
    let (existing, existing_version) = receiver_with_existing(&dir);
    for s in &snaps {
        fs::write(dir.doc_file(&s.id), &s.bytes).unwrap();
    }
    let mut want: Vec<PageId> = ids.clone();
    want.push(existing.clone());
    want.sort();

    for pass in 0..2 {
        let v = dir.open();
        assert_eq!(
            listed_ids(&v),
            want,
            "pass {pass}: each page listed exactly once"
        );
        for s in &snaps {
            assert_eq!(
                title_of(&v, &s.id),
                Some(s.page.title.clone()),
                "pass {pass}: title"
            );
            assert_eq!(
                version_of(&v, &s.id),
                Some(s.version.clone()),
                "pass {pass}: version"
            );
        }
        assert_eq!(
            version_of(&v, &existing),
            Some(existing_version.clone()),
            "pass {pass}"
        );
    }
}
