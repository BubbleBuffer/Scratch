use super::*;
use loro::{LoroDoc, LoroValue};

const DAY: i64 = 86_400;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("vault_cold_race_{}", new_id()));
        fs::create_dir_all(path.join(".vault")).unwrap();
        fs::write(
            path.join(".vault/cold-policy.json"),
            br#"{"idle_seconds":3600}"#,
        )
        .unwrap();
        Self(path)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn page(v: &Vault) -> String {
    let id = new_id();
    let doc = LoroDoc::new();
    doc.set_peer_id(42).unwrap();
    doc.set_record_timestamp(true);
    let meta = doc.get_map("page");
    meta.insert("id", id.as_str()).unwrap();
    meta.insert("parent", LoroValue::Null).unwrap();
    doc.set_next_commit_timestamp(now_ms() / 1000 - 50 * DAY);
    doc.commit();
    meta.insert("title", "Old").unwrap();
    doc.set_next_commit_timestamp(now_ms() / 1000 - 40 * DAY);
    doc.commit();
    v.import_updates(id.clone(), doc.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    v.flush().unwrap();
    id
}

fn run(v: &Vault, id: &str) -> bool {
    v.with_db(|db| {
        v.inner.cold_page(
            db,
            id,
            &ColdPolicy {
                minimum_bytes: 0,
                ..ColdPolicy::default()
            },
            now_ms(),
        )
    })
    .unwrap()
}

/// Stops the writer without its final flush, then the instance can be dropped.
/// Only what already reached the disk survives, as after a crash. Mirrors
/// `Dir::open_stopped` in vault.rs, which cannot be reused from here.
fn crash(vault: &Vault) {
    vault.inner.send(Msg::Crash);
    if let Some(writer) = lock(&vault.writer).take() {
        writer.join().unwrap();
    }
}

/// A device's edit on top of `base`, dated `at` (seconds), as a sync blob.
fn peer_edit(base: &[u8], peer: u64, at: i64, title: &str) -> Vec<u8> {
    let doc = LoroDoc::new();
    doc.import(base).unwrap();
    let before = doc.oplog_vv();
    doc.set_peer_id(peer).unwrap();
    doc.set_record_timestamp(true);
    doc.set_next_commit_timestamp(at);
    doc.get_map("page").insert("title", title).unwrap();
    doc.commit();
    doc.export(ExportMode::updates(&before)).unwrap()
}

/// Another instance (B) imports an edit dated 35 days back and then makes a
/// recent one, and publishes a cold root that covers the older edit. The first
/// instance keeps its own cache, which still holds its older shallow root.
fn advance_cold_root_from_second_instance(root: &Root, a: &Vault, id: &str) -> Vault {
    let t = now_ms() / 1000;
    let base = a
        .inner
        .read(id, |d| d.doc.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    let b = root.open();
    b.import_updates(id.to_owned(), peer_edit(&base, 77, t - 35 * DAY, "Old 35"))
        .unwrap();
    b.rename_page(id.to_owned(), "B recent".to_owned()).unwrap();
    b.flush().unwrap();
    assert!(
        run(&b, id),
        "the second instance must publish a root beyond the first cache"
    );
    b
}

#[test]
fn edit_between_preparation_and_publication_is_never_overwritten() {
    let root = Root::new();
    let v = root.open();
    let id = page(&v);
    let original = fs::read(v.inner.doc_path(&id)).unwrap();
    *lock(&v.inner.cold_before_publish) = Some(Box::new(|inner, id| {
        inner
            .edit(id, |doc| doc.set_title("Edited during compression"))
            .unwrap();
    }));
    assert!(!run(&v, &id));
    assert_eq!(fs::read(v.inner.doc_path(&id)).unwrap(), original);
    // Cancelled before the cold write, so no redundant cold file is left behind.
    assert!(!cold::path(&root.0, &id).exists());
    assert_eq!(
        v.load_page(id.clone()).unwrap().title,
        "Edited during compression"
    );
    v.flush().unwrap();
    let versions = v.list_versions(id.clone()).unwrap();
    drop(v);
    let v = root.open();
    assert_eq!(
        v.load_page(id.clone()).unwrap().title,
        "Edited during compression"
    );
    assert_eq!(v.list_versions(id.clone()).unwrap(), versions);
    assert!(run(&v, &id));
    assert_eq!(v.list_versions(id).unwrap(), versions);
}

#[test]
fn acquired_page_between_preparation_and_publication_cancels_split() {
    let root = Root::new();
    let v = root.open();
    let id = page(&v);
    let held = Arc::new(Mutex::new(None));
    let hold = held.clone();
    *lock(&v.inner.cold_before_publish) = Some(Box::new(move |inner, id| {
        *lock(&hold) = Some(inner.slot(id).unwrap());
    }));
    assert!(!run(&v, &id));
    assert!(!v.inner.read(&id, |d| d.doc.is_shallow()).unwrap());
    *lock(&held) = None;
    assert!(run(&v, &id));
    assert_eq!(v.load_page(id).unwrap().title, "Old");
}

#[test]
fn purge_between_cold_write_and_publication_never_resurrects_files() {
    let root = Root::new();
    let v = root.open();
    let id = page(&v);
    *lock(&v.inner.cold_before_publish) = Some(Box::new(|inner, id| {
        inner.add_purged(&[id.to_owned()]).unwrap();
        inner.remove_files(id, None).unwrap();
    }));
    // Purge cancels staging without a writer error. The hot file must remain
    // absent; sync must honour the durable tombstone.
    let result = v.with_db(|db| {
        v.inner.cold_page(
            db,
            &id,
            &ColdPolicy {
                minimum_bytes: 0,
                ..ColdPolicy::default()
            },
            now_ms(),
        )
    });
    assert!(!result.unwrap());
    assert!(!v.inner.doc_path(&id).exists());
    assert!(!cold::path(&root.0, &id).exists());
    assert!(v.purged_pages().contains(&id));
}

#[test]
fn second_vault_flush_during_preparation_must_not_be_overwritten() {
    let root = Root::new();
    let v = root.open();
    let id = page(&v);
    let second = Arc::new(root.open());
    assert!(!second.is_owner());
    second.load_page(id.clone()).unwrap();
    let other = second.clone();
    *lock(&v.inner.cold_before_publish) = Some(Box::new(move |_, id| {
        other
            .rename_page(id.to_owned(), "Flushed by second instance".to_owned())
            .unwrap();
        other.flush().unwrap();
    }));
    let published = run(&v, &id);
    drop(second);
    drop(v);
    let reopened = root.open();
    assert_eq!(
        reopened.load_page(id).unwrap().title,
        "Flushed by second instance"
    );
    assert!(
        !published,
        "publication must cancel after another instance flushes newer ops"
    );
}

#[test]
fn second_instance_append_during_preparation_survives_crash_and_reopen() {
    let root = Root::new();
    let v = root.open();
    let id = page(&v);
    v.rename_page(id.clone(), "A recent".to_owned()).unwrap();
    v.flush().unwrap();
    let base = v
        .inner
        .read(&id, |d| d.doc.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    let delta = peer_edit(&base, 77, now_ms() / 1000, "B appended");

    // The second instance appends a sync blob straight to the log: durable
    // without any flush, and it is not in the first instance's cache.
    let second = Arc::new(root.open());
    let other = second.clone();
    *lock(&v.inner.cold_before_publish) = Some(Box::new(move |_, id| {
        other.import_updates(id.to_owned(), delta).unwrap();
        crash(&other);
    }));
    assert!(
        !run(&v, &id),
        "publication must cancel after another instance appended"
    );
    crash(&v);
    assert!(
        fs::metadata(v.inner.log_path(&id)).unwrap().len() > 0,
        "the append must remain unflushed at the crash boundary"
    );
    // Cancelled before the cold write: the split leaves no cold file behind.
    assert!(!cold::path(&root.0, &id).exists());
    assert_eq!(second.load_page(id.clone()).unwrap().title, "B appended");

    // Crash both instances, with the second instance's append never flushed.
    let expected_vv = second
        .inner
        .read_history(&id, |d| d.doc.oplog_vv())
        .unwrap();
    let expected_versions = second.list_versions(id.clone()).unwrap();
    crash(&second);
    crash(&v);
    drop(second);
    drop(v);

    let reopened = root.open();
    assert_eq!(reopened.load_page(id.clone()).unwrap().title, "B appended");
    assert_eq!(
        reopened
            .inner
            .read_history(&id, |d| d.doc.oplog_vv())
            .unwrap(),
        expected_vv
    );
    assert_eq!(reopened.list_versions(id).unwrap(), expected_versions);
}

#[test]
fn full_cache_from_before_foreign_split_flushes_back_to_shallow_hot() {
    let root = Root::new();
    let a = root.open();
    let id = page(&a);
    let b = root.open();
    b.load_page(id.clone()).unwrap();
    assert!(!b.inner.read(&id, |d| d.doc.is_shallow()).unwrap());
    assert!(run(&a, &id));
    let cold_before = fs::read(cold::path(&root.0, &id)).unwrap();
    b.rename_page(id.clone(), "Edited from a full cache".into())
        .unwrap();
    b.flush().unwrap();
    let hot = LoroDoc::new();
    hot.import(&fs::read(b.inner.doc_path(&id)).unwrap())
        .unwrap();
    assert!(
        hot.is_shallow(),
        "a cached full doc must retain the foreign cold boundary"
    );
    assert_eq!(fs::read(cold::path(&root.0, &id)).unwrap(), cold_before);
    let versions = b.list_versions(id.clone()).unwrap();
    drop(b);
    drop(a);
    let reopened = root.open();
    assert_eq!(
        reopened.load_page(id.clone()).unwrap().title,
        "Edited from a full cache"
    );
    assert_eq!(reopened.list_versions(id).unwrap(), versions);
}

#[test]
fn foreign_dirty_log_before_preparation_is_left_for_index_recovery() {
    let root = Root::new();
    let a = root.open();
    let id = page(&a);
    let base = a.export_updates(id.clone(), Vec::new()).unwrap();
    let b = root.open();
    b.import_updates(
        id.clone(),
        peer_edit(&base, 77, now_ms() / 1000, "Pending foreign edit"),
    )
    .unwrap();
    crash(&b);
    lock(&a.inner.cache).remove(&id);
    let log = fs::read(a.inner.log_path(&id)).unwrap();
    assert!(!log.is_empty());
    assert!(!run(&a, &id));
    assert_eq!(fs::read(a.inner.log_path(&id)).unwrap(), log);
    assert!(!cold::path(&root.0, &id).exists());
    crash(&a);
    drop(b);
    drop(a);
    let reopened = root.open();
    reopened.flush().unwrap();
    assert_eq!(
        reopened.load_page(id.clone()).unwrap().title,
        "Pending foreign edit"
    );
    let disk_vv = reopened
        .inner
        .read(&id, |d| d.doc.oplog_vv().encode())
        .unwrap();
    assert_eq!(
        reopened
            .page_versions()
            .unwrap()
            .into_iter()
            .find(|p| p.page == id)
            .unwrap()
            .version,
        disk_vv
    );
}

#[test]
fn full_cache_missing_foreign_archived_operations_merges_before_flush() {
    let root = Root::new();
    let a = root.open();
    let id = page(&a);
    let b = root.open();
    b.load_page(id.clone()).unwrap();
    let base = a.export_updates(id.clone(), Vec::new()).unwrap();
    a.import_updates(
        id.clone(),
        peer_edit(&base, 77, now_ms() / 1000 - 35 * DAY, "Older foreign edit"),
    )
    .unwrap();
    a.rename_page(id.clone(), "Latest foreign state".into())
        .unwrap();
    a.flush().unwrap();
    assert!(run(&a, &id));
    let latest = a.inner.read_history(&id, |d| d.doc.oplog_vv()).unwrap();
    assert!(!b
        .inner
        .read(&id, |d| d.doc.oplog_vv().includes_vv(&latest))
        .unwrap());
    b.apply(
        id.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: crate::types::NewBlock {
                id: "cached-local-block".into(),
                kind: BlockKind::Paragraph,
                text: "Retained local edit".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    b.flush().unwrap();
    assert!(b
        .inner
        .read(&id, |d| d.doc.oplog_vv().includes_vv(&latest))
        .unwrap());
    let value = b.load_page(id.clone()).unwrap();
    assert_eq!(value.title, "Latest foreign state");
    assert_eq!(value.blocks[0].text, "Retained local edit");
    let hot = LoroDoc::new();
    hot.import(&fs::read(b.inner.doc_path(&id)).unwrap())
        .unwrap();
    assert!(hot.is_shallow());
    let versions = b.list_versions(id.clone()).unwrap();
    drop(b);
    drop(a);
    let reopened = root.open();
    assert_eq!(reopened.load_page(id.clone()).unwrap(), value);
    assert_eq!(reopened.list_versions(id).unwrap(), versions);
}

#[test]
fn clean_stale_cache_does_not_prevent_a_later_cold_split() {
    let root = Root::new();
    let a = root.open();
    let id = page(&a);
    let b = root.open();
    b.rename_page(id.clone(), "Already flushed foreign edit".into())
        .unwrap();
    b.flush().unwrap();
    assert!(run(&a, &id));
    assert_eq!(
        a.load_page(id.clone()).unwrap().title,
        "Already flushed foreign edit"
    );
    assert_eq!(
        a.list_versions(id.clone()).unwrap(),
        b.list_versions(id).unwrap()
    );
}

#[test]
fn stale_shallow_history_and_full_sync_see_root_advanced_by_second_instance() {
    let root = Root::new();
    let a = root.open();
    let id = page(&a);
    assert!(run(&a, &id));
    assert!(a.inner.read(&id, |d| d.doc.is_shallow()).unwrap());
    let stale_vv = a.inner.read(&id, |d| d.doc.oplog_vv()).unwrap();

    let b = advance_cold_root_from_second_instance(&root, &a, &id);
    let latest_vv = b.inner.read_history(&id, |d| d.doc.oplog_vv()).unwrap();
    assert_ne!(
        stale_vv, latest_vv,
        "the first instance must still be stale"
    );

    // History from the stale handle reads the disk B published.
    let versions = a.list_versions(id.clone()).unwrap();
    assert_eq!(versions, b.list_versions(id.clone()).unwrap());
    assert_eq!(a.inner.read(&id, |d| d.doc.oplog_vv()).unwrap(), latest_vv);
    assert_eq!(a.load_page(id.clone()).unwrap().title, "B recent");

    // Full sync from the stale handle carries every operation.
    let sync = a.export_updates(id.clone(), Vec::new()).unwrap();
    let mirror = LoroDoc::new();
    mirror.import(&sync).unwrap();
    assert_eq!(mirror.oplog_vv(), latest_vv, "full sync lost operations");
}

#[test]
fn stale_shallow_import_merges_on_top_of_latest_cold_root() {
    let root = Root::new();
    let a = root.open();
    let id = page(&a);
    let oldest = a
        .list_versions(id.clone())
        .unwrap()
        .last()
        .unwrap()
        .id
        .clone();
    let offline_base = a
        .inner
        .read(&id, |d| {
            d.fork_at(&crate::history::decode_version(&oldest).unwrap())
                .unwrap()
                .doc
                .export(ExportMode::Snapshot)
                .unwrap()
        })
        .unwrap();
    assert!(run(&a, &id));
    assert!(a.inner.read(&id, |d| d.doc.is_shallow()).unwrap());

    let b = advance_cold_root_from_second_instance(&root, &a, &id);
    let latest_vv = b.inner.read_history(&id, |d| d.doc.oplog_vv()).unwrap();
    assert_ne!(
        a.inner.read(&id, |d| d.doc.oplog_vv()).unwrap(),
        latest_vv,
        "the first instance must still be stale"
    );

    // An offline device depends on history before even A's cached root.
    // Rebuilding must also retain the newer hot/root published by B.
    let offline = LoroDoc::new();
    offline.import(&offline_base).unwrap();
    let since = offline.oplog_vv();
    offline.set_peer_id(88).unwrap();
    offline.get_map("offline").insert("marker", "kept").unwrap();
    offline.commit();
    let later = offline.export(ExportMode::updates(&since)).unwrap();
    let shallow = a
        .inner
        .read(&id, |d| d.doc.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    let probe = LoroDoc::new();
    probe.import(&shallow).unwrap();
    assert!(matches!(
        probe.import(&later),
        Err(loro::LoroError::ImportUpdatesThatDependsOnOutdatedVersion)
    ));
    a.import_updates(id.clone(), later).unwrap();
    assert_eq!(a.load_page(id.clone()).unwrap().title, "B recent");
    assert!(
        matches!(a.inner.read(&id, |d| d.doc.get_map("offline").get("marker")).unwrap(),
        Some(loro::ValueOrContainer::Value(LoroValue::String(value))) if value.to_string() == "kept")
    );

    // Neither the import nor the history may drop the second instance's ops.
    let history_vv = a.inner.read_history(&id, |d| d.doc.oplog_vv()).unwrap();
    assert!(
        history_vv.includes_vv(&latest_vv),
        "history lost the second instance's operations"
    );
    assert!(
        history_vv.get(&88).is_some(),
        "history lost the stale-handle import"
    );
    let sync = a.export_updates(id.clone(), Vec::new()).unwrap();
    let mirror = LoroDoc::new();
    mirror.import(&sync).unwrap();
    assert_eq!(
        mirror.oplog_vv(),
        history_vv,
        "full sync must include all operations"
    );
}
