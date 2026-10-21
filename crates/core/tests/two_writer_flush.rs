//! Two writable Vault instances on one folder. Only the first instance open
//! is the owner; the second one is a writer without the device's peer id.
//! Each writer's flush must keep the other's edits on disk, in the merged
//! cache and in the metadata and search index.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Barrier;

use vault_core::*;

/// Rounds of concurrent edits and flushes in the two-thread test.
const ROUNDS: usize = 25;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("two_writer_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn nb(id: &str, text: &str) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind: BlockKind::Paragraph,
        text: text.into(),
        marks: vec![],
        checked: false,
    }
}

fn ins(after: Option<&str>, block: NewBlock) -> BlockOp {
    BlockOp::Insert {
        parent: None,
        after: after.map(Into::into),
        block,
    }
}

fn set_text(id: &str, text: &str) -> BlockOp {
    BlockOp::SetText {
        id: id.into(),
        text: text.into(),
        marks: vec![],
    }
}

/// The owner creates page P with one block "b1", flushed to disk.
fn setup(dir: &Dir) -> (Vault, PageId) {
    let a = dir.open();
    assert!(a.is_owner());
    let p = a.create_page(None, "Old".into()).unwrap();
    a.apply(
        p.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: nb("b1", "old text"),
        }],
    )
    .unwrap();
    a.flush().unwrap();
    (a, p)
}

/// The page after a fresh open.
fn reopen(dir: &Dir, p: &PageId) -> Page {
    dir.open().load_page(p.clone()).unwrap()
}

/// Text of the top-level block `id`, if the page has it.
fn block_text(page: &Page, id: &str) -> Option<String> {
    page.blocks
        .iter()
        .find(|b| b.id == id)
        .map(|b| b.text.clone())
}

/// Top-level blocks as id to text.
fn block_map(page: &Page) -> HashMap<String, String> {
    page.blocks
        .iter()
        .map(|b| (b.id.clone(), b.text.clone()))
        .collect()
}

/// The page state after `run_concurrent_rounds`, with the expected values:
/// the title of the last round, b1 from the last round, and every block
/// inserted by either writer with its text.
fn assert_full_page(page: &Page) {
    let last = ROUNDS - 1;
    assert_eq!(page.title, format!("A title {last}"));
    let mut expected: HashMap<String, String> = HashMap::new();
    expected.insert("b1".into(), format!("B text {last}"));
    for r in 0..ROUNDS {
        expected.insert(format!("a{r}"), format!("alpha {r}"));
        expected.insert(format!("c{r}"), format!("bravo {r}"));
    }
    assert_eq!(page.blocks.len(), expected.len(), "block count");
    assert_eq!(block_map(page), expected);
}

/// I1: B renames P and flushes; A (owner, no dirty state) flushes afterwards.
#[test]
fn i1_owner_flushes_after_second_writer_without_own_edit() {
    let dir = Dir::new("i1");
    let (a, p) = setup(&dir);
    let b = dir.open();
    assert!(!b.is_owner());
    b.load_page(p.clone()).unwrap();
    b.rename_page(p.clone(), "B title".into()).unwrap();
    b.flush().unwrap();
    drop(b);
    a.flush().unwrap();
    drop(a);
    let page = reopen(&dir, &p);
    println!("I1 title={:?} b1={:?}", page.title, block_text(&page, "b1"));
    assert_eq!(page.title, "B title");
    assert_eq!(block_text(&page, "b1").as_deref(), Some("old text"));
    assert_eq!(page.blocks.len(), 1, "no block lost or added");
}

/// I2: B renames P and flushes; A edits a different block (b1) and flushes.
#[test]
fn i2_owner_edits_other_part_after_second_writer_flush() {
    let dir = Dir::new("i2");
    let (a, p) = setup(&dir);
    let b = dir.open();
    assert!(!b.is_owner());
    b.load_page(p.clone()).unwrap();
    b.rename_page(p.clone(), "B title".into()).unwrap();
    b.flush().unwrap();
    drop(b);
    a.apply(p.clone(), vec![set_text("b1", "A text")]).unwrap();
    a.flush().unwrap();
    drop(a);
    let page = reopen(&dir, &p);
    println!("I2 title={:?} b1={:?}", page.title, block_text(&page, "b1"));
    assert_eq!(page.title, "B title");
    assert_eq!(block_text(&page, "b1").as_deref(), Some("A text"));
    assert_eq!(page.blocks.len(), 1, "no block lost or added");
}

/// I3: B edits block b1 and flushes; A renames P (other part) and flushes.
#[test]
fn i3_owner_renames_after_second_writer_block_edit_flush() {
    let dir = Dir::new("i3");
    let (a, p) = setup(&dir);
    let b = dir.open();
    assert!(!b.is_owner());
    b.load_page(p.clone()).unwrap();
    b.apply(p.clone(), vec![set_text("b1", "B text")]).unwrap();
    b.flush().unwrap();
    drop(b);
    a.rename_page(p.clone(), "A title".into()).unwrap();
    a.flush().unwrap();
    drop(a);
    let page = reopen(&dir, &p);
    println!("I3 title={:?} b1={:?}", page.title, block_text(&page, "b1"));
    assert_eq!(page.title, "A title");
    assert_eq!(block_text(&page, "b1").as_deref(), Some("B text"));
    assert_eq!(page.blocks.len(), 1, "no block lost or added");
}

/// I4: overlap. A edits P (dirty, unflushed), B edits b1 and flushes, then A flushes.
#[test]
fn i4_owner_dirty_then_flush_after_second_writer_flush() {
    let dir = Dir::new("i4");
    let (a, p) = setup(&dir);
    let b = dir.open();
    assert!(!b.is_owner());
    b.load_page(p.clone()).unwrap();
    a.rename_page(p.clone(), "A title".into()).unwrap();
    b.apply(p.clone(), vec![set_text("b1", "B text")]).unwrap();
    b.flush().unwrap();
    drop(b);
    a.flush().unwrap();
    drop(a);
    let page = reopen(&dir, &p);
    println!("I4 title={:?} b1={:?}", page.title, block_text(&page, "b1"));
    assert_eq!(page.title, "A title");
    assert_eq!(block_text(&page, "b1").as_deref(), Some("B text"));
    assert_eq!(page.blocks.len(), 1, "no block lost or added");
}

/// Both instances open and cached on page P. Each round, A renames P and
/// inserts block `a{r}`, while B sets b1 and inserts block `c{r}`. Barriers
/// line up the edits, then the two flushes, so the flushes overlap.
/// Errors are collected, not unwrapped, so a failing thread cannot leave the
/// other one waiting at a barrier.
///
/// Returns both instances still open (A is the owner), with the page id.
fn run_concurrent_rounds(dir: &Dir) -> (Vault, Vault, PageId) {
    let (a, p) = setup(dir);
    let b = dir.open();
    assert!(!b.is_owner());
    a.load_page(p.clone()).unwrap();
    b.load_page(p.clone()).unwrap();

    let barrier = Barrier::new(2);
    let (errs_a, errs_b) = std::thread::scope(|s| {
        let (a, b, p, barrier) = (&a, &b, &p, &barrier);
        let owner = s.spawn(move || {
            let mut errs = Vec::new();
            for r in 0..ROUNDS {
                barrier.wait();
                let edit = a
                    .rename_page(p.clone(), format!("A title {r}"))
                    .and_then(|()| {
                        a.apply(
                            p.clone(),
                            vec![ins(None, nb(&format!("a{r}"), &format!("alpha {r}")))],
                        )
                    });
                barrier.wait();
                let flush = a.flush();
                barrier.wait();
                if let Err(e) = edit {
                    errs.push(format!("A edit round {r}: {e:?}"));
                }
                if let Err(e) = flush {
                    errs.push(format!("A flush round {r}: {e:?}"));
                }
            }
            errs
        });
        let writer = s.spawn(move || {
            let mut errs = Vec::new();
            for r in 0..ROUNDS {
                barrier.wait();
                let edit = b.apply(
                    p.clone(),
                    vec![
                        set_text("b1", &format!("B text {r}")),
                        ins(None, nb(&format!("c{r}"), &format!("bravo {r}"))),
                    ],
                );
                barrier.wait();
                let flush = b.flush();
                barrier.wait();
                if let Err(e) = edit {
                    errs.push(format!("B edit round {r}: {e:?}"));
                }
                if let Err(e) = flush {
                    errs.push(format!("B flush round {r}: {e:?}"));
                }
            }
            errs
        });
        (owner.join().unwrap(), writer.join().unwrap())
    });
    assert!(errs_a.is_empty(), "owner errors: {errs_a:?}");
    assert!(errs_b.is_empty(), "writer errors: {errs_b:?}");
    (a, b, p)
}

/// Concurrent flushes from two cached instances: after both are closed and
/// the vault is reopened, every edit from both writers is there.
#[test]
fn concurrent_flush_two_instances_all_edits_survive_reopen() {
    let dir = Dir::new("concurrent");
    let (a, b, p) = run_concurrent_rounds(&dir);
    drop(b);
    drop(a);
    let page = reopen(&dir, &p);
    assert_full_page(&page);
}

/// While both instances are still open after their concurrent flushes, the
/// merged cache, the page list and search reflect the other instance's edits.
#[test]
fn concurrent_flush_open_instances_see_each_others_edits() {
    let dir = Dir::new("concurrent_views");
    let (a, b, p) = run_concurrent_rounds(&dir);
    let last = ROUNDS - 1;

    // Merged cache, one view per instance.
    assert_full_page(&a.load_page(p.clone()).unwrap());
    assert_full_page(&b.load_page(p.clone()).unwrap());

    // Metadata: the owner's page list has the title from both writers' rounds.
    let listed: Vec<PageSummary> = a.list_pages(None, 0, 50).unwrap();
    let summary = listed.iter().find(|s| s.id == p).expect("page listed");
    assert_eq!(summary.title, format!("A title {last}"));

    // Search: body text written by each instance finds the page.
    for query in ["alpha", "bravo"] {
        let hits = a.search_pages(query.into(), 10).unwrap();
        assert!(
            hits.iter().any(|h| h.id == p),
            "search for {query:?} misses the page: {hits:?}"
        );
    }
}

/// The page's live log: `.vault/logs/{id}.log`. It must stay on disk across
/// flushes, truncated in place after the snapshot is written.
fn live_log(dir: &Dir, p: &PageId) -> PathBuf {
    dir.0.join(".vault").join("logs").join(format!("{p}.log"))
}

/// Length of the live log. Panics if the file is gone.
fn log_len(path: &Path) -> u64 {
    fs::metadata(path).expect("live log stays present").len()
}

/// Identity (device, inode) of the live log. Unix only; elsewhere the
/// presence and length checks still run and identity is `None` on both sides.
#[cfg(unix)]
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path).expect("live log stays present");
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    fs::metadata(path).expect("live log stays present");
    None
}

/// Asserts the page has exactly these top-level blocks, by id and text.
fn assert_blocks(page: &Page, expected: &[(&str, &str)]) {
    let want: HashMap<String, String> = expected
        .iter()
        .map(|(id, text)| (id.to_string(), text.to_string()))
        .collect();
    assert_eq!(page.blocks.len(), want.len(), "block count");
    assert_eq!(block_map(page), want);
}

/// A caches its live-log handle with one append. B flushes, appends and
/// flushes again (each flush truncates the log in place). Then A appends
/// through its cached handle and flushes. The log file keeps its identity
/// throughout, and every edit survives reopen.
#[test]
fn live_log_stays_and_truncates_in_place_across_repeated_merged_flushes() {
    let dir = Dir::new("live_log_repeat");
    let (a, p) = setup(&dir);
    let log = live_log(&dir, &p);

    // Unflushed: the edit exists only in A's live log, which A now holds open.
    a.apply(p.clone(), vec![ins(None, nb("a1", "alpha 1"))])
        .unwrap();
    let id = file_identity(&log);
    assert!(log_len(&log) > 0, "A's edit is in the live log");

    // B loads after A's append, so its merged view includes a1.
    let b = dir.open();
    b.load_page(p.clone()).unwrap();
    b.apply(p.clone(), vec![set_text("b1", "B text")]).unwrap();
    b.flush().unwrap();
    assert_eq!(file_identity(&log), id, "B's flush keeps the live log file");
    assert_eq!(log_len(&log), 0, "B's flush truncates the live log");

    b.apply(p.clone(), vec![ins(None, nb("c1", "bravo 1"))])
        .unwrap();
    assert!(
        log_len(&log) > 0,
        "B's append after truncation is in the live log"
    );
    b.flush().unwrap();
    assert_eq!(
        file_identity(&log),
        id,
        "second flush keeps the live log file"
    );
    assert_eq!(log_len(&log), 0, "second flush truncates the live log");
    drop(b);

    // A appends through its cached handle, then flushes.
    a.apply(p.clone(), vec![ins(None, nb("a2", "alpha 2"))])
        .unwrap();
    assert_eq!(
        file_identity(&log),
        id,
        "A's append reuses the live log file"
    );
    assert!(log_len(&log) > 0, "A's append is in the live log");
    a.flush().unwrap();
    assert_eq!(file_identity(&log), id, "A's flush keeps the live log file");
    assert_eq!(log_len(&log), 0, "A's flush truncates the live log");
    drop(a);

    let page = reopen(&dir, &p);
    assert_eq!(page.title, "Old");
    assert_blocks(
        &page,
        &[
            ("b1", "B text"),
            ("a1", "alpha 1"),
            ("c1", "bravo 1"),
            ("a2", "alpha 2"),
        ],
    );
}

/// After another instance truncates the live log, A's cached handle must
/// append at the new end: the edit survives reopen and no earlier edit is
/// lost. Separate from the repeated-flush case above.
#[test]
fn cached_live_log_handle_appends_at_new_end_after_other_truncation() {
    let dir = Dir::new("live_log_newend");
    let (a, p) = setup(&dir);
    let log = live_log(&dir, &p);

    a.apply(p.clone(), vec![ins(None, nb("a1", "alpha 1"))])
        .unwrap();
    let id = file_identity(&log);
    assert!(log_len(&log) > 0, "A's edit is in the live log");

    let b = dir.open();
    b.load_page(p.clone()).unwrap();
    b.apply(p.clone(), vec![ins(None, nb("c1", "bravo 1"))])
        .unwrap();
    b.flush().unwrap();
    drop(b);
    assert_eq!(file_identity(&log), id, "B's flush keeps the live log file");
    assert_eq!(log_len(&log), 0, "B's flush truncates the live log");

    // The next append through A's cached handle goes to the new end.
    a.apply(p.clone(), vec![ins(None, nb("a2", "alpha 2"))])
        .unwrap();
    assert_eq!(
        file_identity(&log),
        id,
        "A's append reuses the live log file"
    );
    assert!(
        log_len(&log) > 0,
        "A's append after truncation is in the live log"
    );
    drop(a);

    let page = reopen(&dir, &p);
    assert_blocks(
        &page,
        &[
            ("b1", "old text"),
            ("a1", "alpha 1"),
            ("c1", "bravo 1"),
            ("a2", "alpha 2"),
        ],
    );
}
