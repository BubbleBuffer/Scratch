use super::*;
use crate::types::PageUpdate;

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("flush_batch_{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn open(&self) -> Vault {
        stopped(&self.0)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stopped(path: &Path) -> Vault {
    let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
    vault.inner.send(Msg::Crash);
    if let Some(writer) = lock(&vault.writer).take() {
        writer.join().unwrap();
    }
    vault
}

fn update(n: usize) -> PageUpdate {
    let id = new_id();
    let doc = PageDoc::new_page(1, &id, &format!("batch page {n}"), None, now_ms()).unwrap();
    PageUpdate {
        page: id,
        bytes: doc.snapshot().unwrap(),
    }
}

#[test]
fn cold_import_bumps_sequence_once_for_the_whole_batch() {
    let dir = Dir::new();
    let vault = dir.open();
    let updates: Vec<_> = (0..40).map(update).collect();
    let ids: Vec<_> = updates.iter().map(|u| u.page.clone()).collect();
    let report = vault.import_updates_batch(updates).unwrap();
    assert_eq!(report.created, 40);
    {
        let flush = vault.inner.flush_lock().unwrap();
        assert_eq!(read_sequence(&flush).unwrap().unwrap().counter, 1);
    }
    let holds = lock(&vault.inner.disk_io.batch_holds);
    assert_eq!(holds.len(), 1, "one install acquisition for the batch");
    assert_eq!(holds[0].0, 40);
    drop(holds);
    drop(vault);
    let reopened = dir.open();
    assert_eq!(reopened.page_count().unwrap(), 40);
    for (n, id) in ids.into_iter().enumerate() {
        assert_eq!(
            reopened.load_page(id).unwrap().title,
            format!("batch page {n}")
        );
    }
}

#[test]
fn batch_advisory_lock_covers_snapshot_and_index_commit() {
    let dir = Dir::new();
    let vault = dir.open();
    let u = update(0);
    let doc = PageDoc::from_bytes(&u.bytes, &[]).unwrap();
    let external = open_flush_file(&dir.0).unwrap();
    {
        let db = lock(&vault.inner.db);
        let tx = db.write_tx().unwrap();
        vault
            .inner
            .write_page_docs([(u.page.as_str(), u.bytes.as_slice())], || {
                assert!(vault.inner.doc_path(&u.page).is_file());
                assert!(!try_lock_exclusive(&external)?);
                db.upsert(&meta_row(&doc), &ContentRow::default())?;
                tx.commit()?;
                assert!(
                    !try_lock_exclusive(&external)?,
                    "index commit is still protected"
                );
                Ok(())
            })
            .unwrap();
    }
    assert!(try_lock_exclusive(&external).unwrap());
    unlock_exclusive(&external).unwrap();
    drop(vault);
    assert_eq!(dir.open().load_page(u.page).unwrap().title, "batch page 0");
}

#[test]
fn cold_import_commits_index_before_releasing_batch_lock() {
    let dir = Dir::new();
    let vault = dir.open();
    let (observer, _) = Index::open(&dir.0.join(".vault/index.sqlite")).unwrap();
    let updates: Vec<_> = (0..40).map(update).collect();
    let ids: Vec<_> = updates.iter().map(|u| u.page.clone()).collect();
    let external = open_flush_file(&dir.0).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    *lock(&vault.inner.disk_io.pause_after_batch_finish) = Some(FlushPause {
        entered: entered_tx,
        resume: resume_rx,
    });
    let (paused, excluded, committed, imported) = std::thread::scope(|sc| {
        let importer = sc.spawn(|| vault.import_updates_batch(updates));
        let paused = entered_rx.recv_timeout(Duration::from_secs(10));
        let excluded = try_lock_exclusive(&external).map(|acquired| !acquired);
        let committed = ids
            .iter()
            .map(|id| observer.exists(id))
            .collect::<std::result::Result<Vec<_>, _>>();
        // Every fallible observation is checked only after releasing both
        // possible holders and joining the import thread.
        drop(external);
        let _ = resume_tx.send(());
        let imported = importer.join().expect("importer panicked");
        (paused, excluded, committed, imported)
    });
    paused.unwrap();
    assert!(excluded.unwrap(), "independent OS holder entered the batch");
    assert!(
        committed.unwrap().into_iter().all(|exists| exists),
        "index rows must be visible to another connection before unlock"
    );
    assert_eq!(imported.unwrap().created, 40);
}

#[test]
fn snapshot_write_failure_leaves_earlier_batch_pages_recoverable() {
    let dir = Dir::new();
    let vault = dir.open();
    let good = update(0);
    let bad = update(1);
    // Replacement of a directory by a snapshot must fail after the first
    // valid snapshot has reached durable storage.
    fs::create_dir(vault.inner.doc_path(&bad.page)).unwrap();
    let db = lock(&vault.inner.db);
    let tx = db.write_tx().unwrap();
    let result = vault.inner.write_page_docs(
        [
            (good.page.as_str(), good.bytes.as_slice()),
            (bad.page.as_str(), bad.bytes.as_slice()),
        ],
        || panic!("index commit must not run after a snapshot failure"),
    );
    assert!(result.is_err());
    assert!(vault.inner.doc_path(&good.page).is_file());
    assert!(!db.exists(&good.page).unwrap());
    drop(tx);
    drop(db);
    fs::remove_dir(vault.inner.doc_path(&bad.page)).unwrap();
    drop(vault);
    let reopened = dir.open();
    assert_eq!(reopened.page_count().unwrap(), 1);
    assert_eq!(reopened.load_page(good.page).unwrap().title, "batch page 0");
}

#[test]
fn cache_write_failure_keeps_the_durable_unindexed_snapshot() {
    let dir = Dir::new();
    let vault = dir.open();
    let u = update(0);
    let id = u.page.clone();
    let cache = vault.inner.cache_path(&id);
    fs::create_dir(&cache).unwrap();
    let failed = vault.import_updates_batch(vec![u]).is_err();
    let durable = vault.inner.doc_path(&id).is_file();
    let unindexed = !lock(&vault.inner.db).exists(&id).unwrap();
    fs::remove_dir(&cache).unwrap();
    drop(vault);
    assert!(failed, "the obstructed block cache must fail the import");
    assert!(
        durable,
        "cache failure must leave the snapshot durable, as on master"
    );
    assert!(unindexed);
    let reopened = dir.open();
    assert_eq!(reopened.page_count().unwrap(), 1);
    assert_eq!(reopened.load_page(id).unwrap().title, "batch page 0");
}

/// The caller supplies an owned copy, never the original aged fixture:
/// FLUSH_LOCK_FIXTURE=<copy> cargo test -p vault_core --lib
///     large_aged_fixture_batch_hold -- --ignored --nocapture
#[test]
#[ignore = "requires an owned copy of the 10,001-page aged fixture on the Linux machine"]
fn large_aged_fixture_batch_hold() {
    let path = PathBuf::from(std::env::var("FLUSH_LOCK_FIXTURE").expect("owned fixture copy"));
    let source = stopped(&path);
    let versions = source.page_versions().unwrap();
    assert_eq!(versions.len(), 10_001);
    let updates: Vec<_> = versions
        .iter()
        .map(|v| PageUpdate {
            page: v.page.clone(),
            bytes: source.export_updates(v.page.clone(), vec![]).unwrap(),
        })
        .collect();
    for size in [500, 10_001] {
        let dir = Dir::new();
        let vault = dir.open();
        let bytes: usize = updates[..size].iter().map(|u| u.bytes.len()).sum();
        let started = Instant::now();
        let report = vault
            .import_updates_batch(updates[..size].to_vec())
            .unwrap();
        let total = started.elapsed();
        assert_eq!(report.created as usize, size);
        assert_eq!(vault.page_count().unwrap() as usize, size);
        let holds = lock(&vault.inner.disk_io.batch_holds);
        assert_eq!(holds.len(), 1);
        let (installed, held) = holds[0];
        assert_eq!(installed, size);
        println!(
            "cold batch pages={size} snapshot_bytes={bytes} lock_hold_ms={:.2} total_ms={:.2}",
            held.as_secs_f64() * 1000.0,
            total.as_secs_f64() * 1000.0
        );
        drop(holds);
        assert!(held < FLUSH_LOCK_TIMEOUT,
            "batch holds the advisory lock past the other process's deadline; report a chunk-size choice");
        let received: HashMap<_, _> = vault
            .page_versions()
            .unwrap()
            .into_iter()
            .map(|v| (v.page, v.version))
            .collect();
        for expected in &versions[..size] {
            assert!(crate::sync::version_includes(
                received.get(&expected.page).unwrap(),
                &expected.version
            )
            .unwrap());
        }
        drop(vault);
        assert_eq!(dir.open().page_count().unwrap() as usize, size);
    }
}
