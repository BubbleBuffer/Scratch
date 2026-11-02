//! Breaking reindex: batched, interrupted, resumed.

use super::*;

/// Batch size and batches before a simulated exit for the vault in `dir`.
fn hook(dir: &Dir, batch: usize, stop: usize) {
    lock(&REINDEX_TEST).insert(test_key(&dir.0), (batch, stop));
}

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
    let dir = Dir::new();
    let ids = seed(&dir, 25);
    make_old(&dir);
    // The app exits after the second batch of five pages.
    hook(&dir, 5, 2);
    let first = Vault::open(dir.root());
    hook(&dir, 5, 0);
    assert!(first.is_err(), "the interrupted open reports the stop");
    drop(first);
    assert!(pending(&dir), "the unfinished rebuild is marked in the index");
    assert_eq!(indexed(&dir), 10, "two batches of five pages are committed");

    // The next open finishes it: later pages only.
    let v = Vault::open(dir.root()).unwrap();
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
    let dir = Dir::new();
    seed(&dir, 7);
    make_old(&dir);
    hook(&dir, 3, 0);
    let v = Vault::open(dir.root()).unwrap();
    assert!(!pending(&dir));
    assert_eq!(v.page_count().unwrap(), 7);
    drop(v);
    // And an index that is current opens without a rebuild.
    let v = Vault::open(dir.root()).unwrap();
    assert_eq!(v.page_count().unwrap(), 7);
}

fn hold_rebuild_lock(dir: &Dir) -> File {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.0.join(".vault/reindex.lock"))
        .unwrap();
    assert!(try_lock_exclusive(&f).unwrap());
    f
}

/// Another process is rebuilding: this opener waits, leaves the progress file
/// alone, and rebuilds itself only once the lock is free and the rebuild is
/// still unfinished (the other one died).
#[test]
fn an_opener_waits_for_the_process_that_is_rebuilding() {
    let dir = Dir::new();
    seed(&dir, 6);
    make_old(&dir);
    // The first Index::open of a rebuild marks it unfinished.
    drop(Index::open(&dir.index()).unwrap());
    let progress = dir.0.join(".vault/index-update.json");
    fs::write(&progress, r#"{"done":3,"total":6}"#).unwrap();
    let held = hold_rebuild_lock(&dir);
    let root = dir.root();
    let opener = std::thread::spawn(move || Vault::open(root).map(|v| v.page_count().unwrap()));
    std::thread::sleep(Duration::from_millis(600));
    assert!(!opener.is_finished(), "the opener waits for the lock");
    assert!(progress.exists(), "a waiting opener leaves the progress file alone");
    assert_eq!(indexed(&dir), 0);
    drop(held);
    assert_eq!(opener.join().unwrap().unwrap(), 6);
    assert!(!pending(&dir));
    assert!(!progress.exists(), "the run that rebuilt removes it");
}

/// The rebuilding process finished while this opener waited: nothing is
/// rebuilt again.
#[test]
fn an_opener_that_finds_the_rebuild_finished_does_not_redo_it() {
    let dir = Dir::new();
    seed(&dir, 4);
    make_old(&dir);
    drop(Index::open(&dir.index()).unwrap());
    let held = hold_rebuild_lock(&dir);
    let root = dir.root();
    let opener = std::thread::spawn(move || Vault::open(root).map(|_| ()));
    std::thread::sleep(Duration::from_millis(400));
    assert!(!opener.is_finished());
    // The other process commits its last batch and clears the marker.
    rusqlite::Connection::open(dir.index())
        .unwrap()
        .execute("DELETE FROM meta WHERE key = 'reindex'", [])
        .unwrap();
    drop(held);
    opener.join().unwrap().unwrap();
    assert_eq!(indexed(&dir), 0, "the waiter did not clear and rebuild the index");
}

/// A crash during the schema rebuild cannot leave an empty index that looks
/// current: the rebuild is one transaction.
#[test]
fn a_failed_schema_rebuild_leaves_the_old_index_alone() {
    let dir = Dir::new();
    seed(&dir, 3);
    make_old(&dir);
    let conn = rusqlite::Connection::open(dir.index()).unwrap();
    // A view where the rebuild drops the agenda table makes it fail half way,
    // after it has dropped the pages table.
    conn.execute_batch("DROP TABLE agenda; CREATE VIEW agenda AS SELECT 1 AS x;").unwrap();
    drop(conn);
    assert!(Index::open(&dir.index()).is_err());
    let conn = rusqlite::Connection::open(dir.index()).unwrap();
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(version, 1, "still the old generation, so the next open rebuilds");
    let pages: i64 = conn.query_row("SELECT count(*) FROM pages", [], |r| r.get(0)).unwrap();
    assert_eq!(pages, 3, "the old rows were not dropped");
}

/// The rebuild keeps external search rows (their FTS text is restored in the
/// same transaction) and marks itself unfinished.
#[test]
fn the_schema_rebuild_marks_itself_and_restores_external_rows() {
    let dir = Dir::new();
    seed(&dir, 1);
    {
        let (index, _) = Index::open(&dir.index()).unwrap();
        index.set_reindex_pending(false).unwrap();
        index
            .conn
            .execute(
                "INSERT INTO search_external(source, id, title, body, page_type, updated_ms)
                 VALUES ('s', 'e1', 'External thing', 'body', 'document', 0)",
                [],
            )
            .unwrap();
        index
            .conn
            .execute("INSERT INTO fts(rowid, title, body) SELECT -rowid, title, body FROM search_external", [])
            .unwrap();
    }
    make_old(&dir);
    let (index, fresh) = Index::open(&dir.index()).unwrap();
    assert!(fresh);
    assert!(index.reindex_pending().unwrap());
    let in_fts: i64 = index
        .conn
        .query_row("SELECT count(*) FROM fts WHERE rowid < 0", [], |r| r.get(0))
        .unwrap();
    assert_eq!(in_fts, 1);
}
