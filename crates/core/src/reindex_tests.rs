//! Breaking reindex: batched, interrupted, resumed.

use super::*;
use std::sync::atomic::Ordering;

/// The hooks are process-wide: tests that use them run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("reindex_{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn index(&self) -> PathBuf {
        self.0.join(".vault/index.sqlite")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn seed(dir: &Dir, pages: usize) -> Vec<PageId> {
    let v = Vault::open(dir.root()).unwrap();
    let ids = (0..pages)
        .map(|n| v.create_page(None, format!("Planning note {n}")).unwrap())
        .collect();
    v.flush().unwrap();
    ids
}

/// Marks the index as of an older breaking generation.
fn make_old(dir: &Dir) {
    rusqlite::Connection::open(dir.index())
        .unwrap()
        .pragma_update(None, "user_version", 1)
        .unwrap();
}

fn indexed(dir: &Dir) -> i64 {
    rusqlite::Connection::open(dir.index())
        .unwrap()
        .query_row("SELECT count(*) FROM pages", [], |r| r.get(0))
        .unwrap()
}

fn pending(dir: &Dir) -> bool {
    rusqlite::Connection::open(dir.index())
        .unwrap()
        .query_row("SELECT count(*) FROM meta WHERE key = 'reindex'", [], |r| r.get::<_, i64>(0))
        .unwrap()
        > 0
}

#[test]
fn an_interrupted_rebuild_continues_where_it_stopped() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let ids = seed(&dir, 25);
    make_old(&dir);
    REINDEX_TEST_BATCH.store(5, Ordering::SeqCst);
    // The app exits after the second batch of the rebuild.
    REINDEX_TEST_STOP.store(2, Ordering::SeqCst);
    let first = Vault::open(dir.root());
    REINDEX_TEST_STOP.store(usize::MAX, Ordering::SeqCst);
    assert!(first.is_err(), "the interrupted open reports the stop");
    drop(first);
    assert!(pending(&dir), "the unfinished rebuild is marked in the index");
    assert_eq!(indexed(&dir), 10, "two batches of five pages are committed");

    // The next open finishes it: later pages only.
    let v = Vault::open(dir.root()).unwrap();
    REINDEX_TEST_BATCH.store(0, Ordering::SeqCst);
    assert!(!pending(&dir));
    assert_eq!(indexed(&dir), 25);
    assert_eq!(v.page_count().unwrap(), 25);
    let hits = v.search("Planning note 17".into(), 25).unwrap();
    assert!(hits.iter().any(|h| h.id == ids[17]), "search finds a page from the last batch");
    let hits = v.search("Planning note 2".into(), 25).unwrap();
    assert!(hits.iter().any(|h| h.id == ids[2]), "and one from the first");
    assert!(!v.inner.index_update_file().exists());
}

#[test]
fn a_rebuild_that_was_never_interrupted_clears_its_marker() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    seed(&dir, 7);
    make_old(&dir);
    REINDEX_TEST_BATCH.store(3, Ordering::SeqCst);
    let v = Vault::open(dir.root()).unwrap();
    REINDEX_TEST_BATCH.store(0, Ordering::SeqCst);
    assert!(!pending(&dir));
    assert_eq!(v.page_count().unwrap(), 7);
    drop(v);
    // And an index that is current opens without a rebuild.
    let v = Vault::open(dir.root()).unwrap();
    assert_eq!(v.page_count().unwrap(), 7);
}
