//! Tests of the sync hooks (Core: MVP scope, Part B, internal wiki): versions, export and
//! import, purges, conflicts, blobs and the change feed wait. The
//! protocol and hub are tested in `crates/sync`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use loro::{ExportMode, LoroDoc, LoroError, LoroText, TreeParentId, VersionVector};
use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_sync_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).expect("open")
    }
}

impl Drop for TempDir {
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

/// Sends `from`'s changes that `to` lacks, and its purges.
fn push(from: &Vault, to: &Vault) -> SyncImport {
    to.import_purge_records(from.purge_records()).unwrap();
    for record in from.restore_records() {
        if to.needs_restore_snapshot(&record).unwrap() {
            let bytes = from.export_updates(record.page.clone(), vec![]).unwrap();
            to.import_restored_updates(record, bytes).unwrap();
        }
    }
    let have: HashMap<String, Vec<u8>> = to
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    let mut updates = Vec::new();
    for v in from.page_versions().unwrap() {
        let theirs = have.get(&v.page).cloned().unwrap_or_default();
        if !version_includes(&theirs, &v.version).unwrap() {
            let bytes = from.export_updates(v.page.clone(), theirs).unwrap();
            updates.push(PageUpdate {
                page: v.page,
                bytes,
            });
        }
    }
    to.import_updates_batch(updates).unwrap()
}

fn sync(a: &Vault, b: &Vault) {
    push(a, b);
    push(b, a);
}

fn texts(v: &Vault, p: &str) -> Vec<String> {
    v.load_page(p.to_string())
        .unwrap()
        .blocks
        .into_iter()
        .map(|b| b.text)
        .collect()
}

fn md_exists(t: &TempDir, id: &str) -> bool {
    fs::read_dir(t.0.join("pages")).unwrap().any(|e| {
        e.unwrap()
            .path()
            .to_string_lossy()
            .ends_with(&format!("-{id}.md"))
    })
}

#[test]
fn stable_peer_keeps_version_vectors_small() {
    let t = TempDir::new("peer");
    let p = {
        let v = t.open();
        let p = v.create_page(None, "P".into()).unwrap();
        v.apply(p.clone(), vec![ins(None, nb("a", "one"))]).unwrap();
        p
    };
    for i in 0..3 {
        let v = t.open();
        v.apply(p.clone(), vec![set_text("a", &format!("edit {i}"))])
            .unwrap();
    }
    let v = t.open();
    let versions = v.page_versions().unwrap();
    assert_eq!(versions.len(), 1);
    // One peer: a version vector of one entry encodes in a few bytes.
    let one_peer = versions[0].version.len();
    assert!(one_peer < 24, "version vector of {one_peer} bytes");
    assert_eq!(v.list_versions(p).unwrap().len(), 1);
}

#[test]
fn new_pages_arrive_with_files_index_and_feed() {
    let (ta, tb) = (TempDir::new("a"), TempDir::new("b"));
    let (a, b) = (ta.open(), tb.open());
    let parent = a.create_page(None, "Parent".into()).unwrap();
    let child = a.create_page(Some(parent.clone()), "Child".into()).unwrap();
    a.apply(
        child.clone(),
        vec![
            ins(None, nb("x", "zebra crossing")),
            ins(Some("x"), nb("y", "see also")),
        ],
    )
    .unwrap();
    let other = a.create_page(None, "Other".into()).unwrap();
    a.apply(
        other.clone(),
        vec![ins(
            None,
            NewBlock {
                id: "l".into(),
                kind: BlockKind::Paragraph,
                text: "Child".into(),
                marks: vec![Mark {
                    start: 0,
                    end: 5,
                    kind: MarkKind::PageLink(child.clone()),
                }],
                checked: false,
            },
        )],
    )
    .unwrap();
    let seq = b.poll_changes(0).seq;
    let r = push(&a, &b);
    assert_eq!(r.created, 3);
    assert_eq!(b.page_count().unwrap(), 3);
    assert_eq!(texts(&b, &child), vec!["zebra crossing", "see also"]);
    assert_eq!(
        b.load_page(child.clone()).unwrap().parent.as_deref(),
        Some(parent.as_str())
    );
    assert_eq!(b.search("zebra".into(), 5).unwrap()[0].id, child);
    assert_eq!(b.backlinks(child.clone()).unwrap()[0].id, other);
    assert!(md_exists(&tb, &child));
    let feed = b.poll_changes(seq);
    assert!(feed.changes.contains(&Change::PageCreated(child.clone())));
    assert!(feed.changes.contains(&Change::TreeChanged));
    // Nothing left to send either way.
    let r = push(&a, &b);
    assert_eq!((r.created, r.changed), (0, 0));
    let r = push(&b, &a);
    assert_eq!((r.created, r.changed), (0, 0));
    // Versions agree.
    let va = a.page_versions().unwrap();
    let vb: HashMap<_, _> = b
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    for v in va {
        assert!(version_includes(&vb[&v.page], &v.version).unwrap());
    }
}

#[test]
fn offline_edits_merge_without_conflict() {
    let (ta, tb) = (TempDir::new("a"), TempDir::new("b"));
    let (a, b) = (ta.open(), tb.open());
    let p = a.create_page(None, "Shared".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(None, nb("1", "first")),
            ins(Some("1"), nb("2", "second")),
        ],
    )
    .unwrap();
    sync(&a, &b);
    // Both edit different blocks and metadata while apart.
    a.apply(p.clone(), vec![set_text("1", "first, edited on a")])
        .unwrap();
    b.apply(p.clone(), vec![set_text("2", "second, edited on b")])
        .unwrap();
    b.rename_page(p.clone(), "Shared (renamed on b)".into())
        .unwrap();
    let seq = a.poll_changes(0).seq;
    sync(&a, &b);
    for v in [&a, &b] {
        assert_eq!(
            texts(v, &p),
            vec!["first, edited on a", "second, edited on b"]
        );
        assert_eq!(
            v.load_page(p.clone()).unwrap().title,
            "Shared (renamed on b)"
        );
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
    let feed = a.poll_changes(seq);
    assert!(feed.changes.contains(&Change::PageChanged(p.clone())));
    assert!(feed.changes.contains(&Change::TreeChanged));
    a.flush().unwrap();
    assert_eq!(
        a.list_pages(None, 0, 5).unwrap()[0].title,
        "Shared (renamed on b)"
    );
}

/// There is no public create-with-id API. Import a fresh, independent page
/// snapshot into each vault, then seed it through the ordinary block API.
fn independent_page(v: &Vault, page: &str, peer: u64, tasks: &str, log: &str, side: &str) {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer).unwrap();
    let meta = doc.get_map("page");
    meta.insert("id", page).unwrap();
    meta.insert("title", "2026-10-13").unwrap();
    meta.insert("parent", loro::LoroValue::Null).unwrap();
    meta.insert("created", 1_i64).unwrap();
    meta.insert("updated", 1_i64).unwrap();
    doc.commit();
    assert_eq!(
        v.import_updates(page.into(), doc.export(ExportMode::Snapshot).unwrap())
            .unwrap()
            .created,
        1
    );
    let task_content = new_id();
    let log_content = new_id();
    v.apply(
        page.into(),
        vec![
            ins(
                None,
                NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb(tasks, "Tasks")
                },
            ),
            BlockOp::Insert {
                parent: Some(tasks.into()),
                after: None,
                block: NewBlock {
                    kind: BlockKind::Todo,
                    ..nb(&task_content, &format!("task on {side}"))
                },
            },
            ins(
                Some(tasks),
                NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb(log, "Log")
                },
            ),
            BlockOp::Insert {
                parent: Some(log.into()),
                after: None,
                block: nb(&log_content, &format!("log on {side}")),
            },
        ],
    )
    .unwrap();
}

fn assert_independent_page_merge(v: &Vault, page: &str) {
    assert_eq!(v.page_count().unwrap(), 1);
    assert_eq!(v.page_versions().unwrap().len(), 1);
    assert_eq!(v.list_pages(None, 0, 10).unwrap().len(), 1);
    let p = v.load_page(page.into()).unwrap();
    assert_eq!(p.id, page);
    assert_eq!(p.blocks.len(), 4, "raw roots: {:?}", p.blocks);
    for name in ["Tasks", "Log"] {
        let roots: Vec<_> = p.blocks.iter().filter(|b| b.text == name).collect();
        assert_eq!(roots.len(), 2);
        assert!(roots
            .iter()
            .all(|b| b.kind == BlockKind::Toggle && b.children.len() == 1));
    }
    let mut contents: Vec<_> = p
        .blocks
        .iter()
        .flat_map(|b| &b.children)
        .map(|b| b.text.as_str())
        .collect();
    contents.sort();
    assert_eq!(contents, ["log on a", "log on b", "task on a", "task on b"]);
    assert!(v.list_conflicts(None).unwrap().is_empty());
}

#[test]
fn independent_same_id_pages_merge_random_seed_blocks() {
    const PAGE: &str = "independent-day-random-seeds";
    let (ta, tb) = (TempDir::new("independent_a"), TempDir::new("independent_b"));
    let (a, b) = (ta.open(), tb.open());
    let (tasks_a, log_a, tasks_b, log_b) = (new_id(), new_id(), new_id(), new_id());
    independent_page(&a, PAGE, 100, &tasks_a, &log_a, "a");
    independent_page(&b, PAGE, 200, &tasks_b, &log_b, "b");
    let ab = push(&a, &b);
    let ba = push(&b, &a);
    println!("random seed imports: a->b={ab:?}, b->a={ba:?}");
    for v in [&a, &b] {
        assert_independent_page_merge(v, PAGE);
    }
    assert_eq!(
        a.load_page(PAGE.into()).unwrap(),
        b.load_page(PAGE.into()).unwrap()
    );
    println!("random seeds: one page, four Toggle roots, four children, zero conflicts");
    a.apply(
        PAGE.into(),
        vec![BlockOp::Delete {
            id: tasks_a.clone(),
        }],
    )
    .unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        let page = v.load_page(PAGE.into()).unwrap();
        assert_eq!(page.blocks.len(), 3);
        assert!(page.blocks.iter().all(|b| b.id != tasks_a));
        assert!(page
            .blocks
            .iter()
            .any(|b| b.id == tasks_b && b.children[0].text == "task on b"));
        assert_eq!(
            page.blocks.iter().map(|b| b.children.len()).sum::<usize>(),
            3
        );
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
    println!("random seed delete-by-id: exactly a's Tasks subtree removed; three roots and three children remain");
}

#[test]
fn independent_same_id_pages_preserve_duplicate_seed_ids() {
    const PAGE: &str = "independent-day-fixed-seeds";
    let (ta, tb) = (TempDir::new("fixed_a"), TempDir::new("fixed_b"));
    let (a, b) = (ta.open(), tb.open());
    independent_page(&a, PAGE, 100, "tasks", "log", "a");
    independent_page(&b, PAGE, 200, "tasks", "log", "b");
    let ab = push(&a, &b);
    let ba = push(&b, &a);
    println!("fixed seed imports: a->b={ab:?}, b->a={ba:?}");
    for v in [&a, &b] {
        assert_independent_page_merge(v, PAGE);
        let page = v.load_page(PAGE.into()).unwrap();
        assert_eq!(page.blocks.iter().filter(|b| b.id == "tasks").count(), 2);
        assert_eq!(page.blocks.iter().filter(|b| b.id == "log").count(), 2);
    }
    assert_eq!(
        a.load_page(PAGE.into()).unwrap(),
        b.load_page(PAGE.into()).unwrap()
    );
    println!("fixed seeds: one page, four Toggle roots, four children, duplicate tasks/log ids, zero conflicts");
    a.apply(PAGE.into(), vec![BlockOp::Delete { id: "tasks".into() }])
        .unwrap();
    let after = a.load_page(PAGE.into()).unwrap();
    assert_eq!(after.blocks.len(), 3);
    assert_eq!(after.blocks.iter().filter(|b| b.id == "tasks").count(), 1);
    assert_eq!(
        after.blocks.iter().map(|b| b.children.len()).sum::<usize>(),
        3
    );
    let second_delete = a.apply(PAGE.into(), vec![BlockOp::Delete { id: "tasks".into() }]);
    println!("fixed seed delete-by-id: one Tasks subtree remains; second delete result={second_delete:?}");
    assert!(matches!(second_delete, Err(VaultError::NotFound(_))));
    sync(&a, &b);
    for v in [&a, &b] {
        let page = v.load_page(PAGE.into()).unwrap();
        assert_eq!(page.blocks.len(), 3);
        assert_eq!(page.blocks.iter().filter(|b| b.id == "tasks").count(), 1);
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
}

const SYNC_DAY: i64 = 1_791_244_800_000;

/// Explicit fixture ids make the seed keeper deterministic; production seeds
/// still use random ids. Each device has only its own section before syncing.
fn independent_day_notes(a: &Vault, b: &Vault) -> PageId {
    let page = a
        .daily_note(SYNC_DAY, "Day on a".into(), true)
        .unwrap()
        .unwrap();
    assert_eq!(
        b.daily_note(SYNC_DAY, "Day on b".into(), true).unwrap(),
        Some(page.clone())
    );
    for (v, side) in [(a, "a"), (b, "z")] {
        let roots = v.load_page(page.clone()).unwrap().blocks;
        let mut ops: Vec<_> = roots
            .iter()
            .map(|r| BlockOp::Delete { id: r.id.clone() })
            .collect();
        for name in ["Tasks", "Log"] {
            let id = format!("{side}-{name}");
            let after = (name == "Log").then(|| format!("{side}-Tasks"));
            ops.push(ins(
                after.as_deref(),
                NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb(&id, name)
                },
            ));
            ops.push(BlockOp::Insert {
                parent: Some(id),
                after: None,
                block: nb(&format!("{name}-{side}"), &format!("{name} on {side}")),
            });
        }
        v.apply(page.clone(), ops).unwrap();
    }
    page
}

fn assert_reachable(v: &Vault, page: &str, blocks: &[Block]) {
    for block in blocks {
        v.apply(
            page.into(),
            vec![BlockOp::SetText {
                id: block.id.clone(),
                text: block.text.clone(),
                marks: block.marks.clone(),
            }],
        )
        .unwrap();
        assert_reachable(v, page, &block.children);
    }
}

fn assert_day_read_idempotent(v: &Vault, page: &str) {
    let before = v.load_page(page.into()).unwrap();
    // Loro's encoded vector entries need not have a stable order. Compare
    // every page's decoded peer/counter map, retaining exact version equality.
    let vv = decoded_versions(v);
    for _ in 0..3 {
        assert_eq!(v.load_page(page.into()).unwrap(), before);
        assert_eq!(
            v.daily_note(SYNC_DAY, "Ignored".into(), false).unwrap(),
            Some(page.into())
        );
    }
    assert_eq!(decoded_versions(v), vv);
}

#[test]
fn day_section_move_then_delete_preserves_known_children() {
    let (ta, tb) = (TempDir::new("section_a"), TempDir::new("section_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = independent_day_notes(&a, &b);
    sync(&a, &b);
    for v in [&a, &b] {
        let repaired = v.load_page(page.clone()).unwrap();
        assert_eq!(repaired.blocks.len(), 2);
        for name in ["Tasks", "Log"] {
            let section = repaired.blocks.iter().find(|b| b.text == name).unwrap();
            assert_eq!(section.id, format!("a-{name}"));
            assert_eq!(section.children.len(), 2);
            let mut content: Vec<_> = section.children.iter().map(|b| b.text.clone()).collect();
            content.sort();
            assert_eq!(content, [format!("{name} on a"), format!("{name} on z")]);
        }
        assert!(v.list_conflicts(None).unwrap().is_empty());
        assert_day_read_idempotent(v, &page);
    }
    assert_eq!(
        a.load_page(page.clone()).unwrap(),
        b.load_page(page).unwrap()
    );
}

#[test]
fn day_section_merge_keeps_shared_logical_item_once_at_its_keeper_position() {
    let (ta, tb) = (TempDir::new("shared_seed_a"), TempDir::new("shared_seed_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = independent_day_notes(&a, &b);
    for (v, root, side, first) in [
        (&a, "a-Tasks", "a", "Tasks-a"),
        (&b, "z-Tasks", "z", "Tasks-z"),
    ] {
        v.apply(
            page.clone(),
            vec![
                BlockOp::Insert {
                    parent: Some(root.into()),
                    after: Some(first.into()),
                    block: nb("shared", "Shared item"),
                },
                BlockOp::Insert {
                    parent: Some("shared".into()),
                    after: None,
                    block: nb(&format!("nested-{side}"), &format!("Nested {side}")),
                },
                BlockOp::Insert {
                    parent: Some(root.into()),
                    after: Some("shared".into()),
                    block: nb(&format!("tail-{side}"), &format!("Tail {side}")),
                },
            ],
        )
        .unwrap();
    }
    sync(&a, &b);
    for v in [&a, &b] {
        let p = v.load_page(page.clone()).unwrap();
        assert_eq!(p.blocks.len(), 2);
        let tasks = p.blocks.iter().find(|b| b.text == "Tasks").unwrap();
        assert_eq!(tasks.id, "a-Tasks");
        assert_eq!(
            tasks
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["Tasks-a", "shared", "tail-a", "Tasks-z", "tail-z"]
        );
        assert_eq!(
            tasks.children[1]
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["nested-a", "nested-z"]
        );
        assert_reachable(v, &page, &p.blocks);
        assert!(v.list_conflicts(None).unwrap().is_empty());
        v.flush().unwrap();
    }
    let expected = a.load_page(page.clone()).unwrap().blocks;
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
    }
}

#[test]
fn restored_section_records_competing_content_edits_in_both_import_directions() {
    for deleting_first in [false, true] {
        let (ta, tb) = (TempDir::new("content_a"), TempDir::new("content_b"));
        let (a, b) = (ta.open(), tb.open());
        let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
        let root = a.load_page(page.clone()).unwrap().blocks[0].id.clone();
        a.apply(
            page.clone(),
            vec![BlockOp::Insert {
                parent: Some(root.clone()),
                after: None,
                block: nb("edited", "Common content"),
            }],
        )
        .unwrap();
        sync(&a, &b);
        a.apply(
            page.clone(),
            vec![
                set_text("edited", "Deleting-side edit"),
                BlockOp::Delete { id: root.clone() },
            ],
        )
        .unwrap();
        b.apply(
            page.clone(),
            vec![set_text("edited", "Offline editing-side edit")],
        )
        .unwrap();
        let result = if deleting_first {
            push(&a, &b)
        } else {
            push(&b, &a)
        };
        assert_eq!(result.conflicts, 1);
        sync(&a, &b);
        for v in [&a, &b] {
            let p = v.load_page(page.clone()).unwrap();
            assert_eq!(p.blocks.len(), 2);
            let conflicts = v.list_conflicts(Some(page.clone())).unwrap();
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].block, "edited");
            let mut sides: Vec<_> = conflicts[0]
                .sides
                .iter()
                .map(|s| s.block.as_ref().unwrap().text.as_str())
                .collect();
            sides.sort();
            assert_eq!(sides, ["Deleting-side edit", "Offline editing-side edit"]);
            assert_reachable(v, &page, &p.blocks);
            v.flush().unwrap();
        }
        drop(a);
        drop(b);
        for v in [ta.open(), tb.open()] {
            assert_eq!(v.list_conflicts(Some(page.clone())).unwrap().len(), 1);
        }
    }
}

#[test]
fn day_section_offline_edits_restore_full_deleted_seed_section() {
    let (ta, tb) = (
        TempDir::new("late_section_a"),
        TempDir::new("late_section_b"),
    );
    let (a, b) = (ta.open(), tb.open());
    let page = independent_day_notes(&a, &b);
    push(&b, &a); // A repairs duplicates; B still sees its original z sections.
    assert_eq!(a.load_page(page.clone()).unwrap().blocks.len(), 2);
    a.apply(
        page.clone(),
        vec![
            BlockOp::Delete {
                id: "Tasks-z".into(),
            },
            BlockOp::Insert {
                parent: Some("a-Tasks".into()),
                after: Some("Tasks-a".into()),
                block: nb("keeper-tail-Tasks", "Keeper task tail"),
            },
            BlockOp::Insert {
                parent: Some("a-Log".into()),
                after: Some("Log-z".into()),
                block: nb("keeper-tail-Log", "Keeper log tail"),
            },
        ],
    )
    .unwrap();
    b.apply(
        page.clone(),
        vec![
            set_text("Log-z", "Edited offline log"),
            BlockOp::Insert {
                parent: Some("z-Tasks".into()),
                after: Some("Tasks-z".into()),
                block: nb("late-task", "New offline task"),
            },
            BlockOp::Insert {
                parent: Some("z-Log".into()),
                after: Some("Log-z".into()),
                block: NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb("late-log", "New offline log")
                },
            },
            BlockOp::Insert {
                parent: Some("late-log".into()),
                after: None,
                block: nb("nested-late", "Nested offline text"),
            },
        ],
    )
    .unwrap();
    let expected = b.load_page(page.clone()).unwrap().blocks;
    let result = push(&b, &a);
    assert_eq!(result.conflicts, 0);
    sync(&a, &b);
    for v in [&a, &b] {
        let merged = v.load_page(page.clone()).unwrap();
        assert_eq!(merged.blocks.len(), 2);
        for name in ["Tasks", "Log"] {
            let keeper = merged
                .blocks
                .iter()
                .find(|b| b.id == format!("a-{name}"))
                .unwrap();
            let order: Vec<_> = keeper.children.iter().map(|b| b.id.as_str()).collect();
            assert_eq!(
                order,
                if name == "Tasks" {
                    vec!["Tasks-a", "keeper-tail-Tasks", "Tasks-z", "late-task"]
                } else {
                    vec!["Log-a", "Log-z", "keeper-tail-Log", "late-log"]
                }
            );
            let restored = expected.iter().find(|b| b.text == name).unwrap();
            for child in &restored.children {
                assert_eq!(
                    keeper.children.iter().find(|b| b.id == child.id),
                    Some(child)
                );
            }
        }
        assert!(v.list_conflicts(None).unwrap().is_empty());
        assert_reachable(v, &page, &merged.blocks);
        assert_day_read_idempotent(v, &page);
        v.flush().unwrap();
    }
    let merged = a.load_page(page.clone()).unwrap();
    assert_eq!(merged, b.load_page(page.clone()).unwrap());
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, merged.blocks);
        assert_reachable(&v, &page, &merged.blocks);
        assert_day_read_idempotent(&v, &page);
    }
}

#[test]
fn restored_sections_keep_visible_items_first_whichever_heading_survives() {
    for (restored_prefix, visible_prefix) in [("a", "z"), ("z", "a")] {
        let (ta, tb) = (TempDir::new("order_a"), TempDir::new("order_b"));
        let (a, b) = (ta.open(), tb.open());
        let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
        let mut ops: Vec<_> = a
            .load_page(page.clone())
            .unwrap()
            .blocks
            .into_iter()
            .map(|root| BlockOp::Delete { id: root.id })
            .collect();
        for name in ["Tasks", "Log"] {
            let root = format!("{restored_prefix}-{name}");
            let shared = format!("shared-{name}");
            ops.push(ins(
                (name == "Log")
                    .then(|| format!("{restored_prefix}-Tasks"))
                    .as_deref(),
                NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb(&root, name)
                },
            ));
            ops.push(BlockOp::Insert {
                parent: Some(root.clone()),
                after: None,
                block: nb(&shared, "Shared old content"),
            });
            ops.push(BlockOp::Insert {
                parent: Some(root),
                after: Some(shared),
                block: nb(&format!("restored-old-{name}"), "Restored old content"),
            });
        }
        a.apply(page.clone(), ops).unwrap();
        sync(&a, &b);
        let mut ops = Vec::new();
        for name in ["Tasks", "Log"] {
            let restored = format!("{restored_prefix}-{name}");
            let visible = format!("{visible_prefix}-{name}");
            let before = format!("visible-before-{name}");
            let shared = format!("shared-{name}");
            ops.push(ins(
                Some(&restored),
                NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb(&visible, name)
                },
            ));
            ops.push(BlockOp::Insert {
                parent: Some(visible.clone()),
                after: None,
                block: nb(&before, "Visible before"),
            });
            ops.push(BlockOp::Move {
                id: shared.clone(),
                parent: Some(visible.clone()),
                after: Some(before),
            });
            ops.push(BlockOp::Insert {
                parent: Some(visible),
                after: Some(shared),
                block: nb(&format!("visible-tail-{name}"), "Visible tail"),
            });
            ops.push(BlockOp::Delete { id: restored });
        }
        a.apply(page.clone(), ops).unwrap();
        let mut ops = Vec::new();
        for name in ["Tasks", "Log"] {
            let shared = format!("shared-{name}");
            let old = format!("restored-old-{name}");
            ops.push(set_text(&shared, &format!("Shared offline edit {name}")));
            ops.push(set_text(&old, &format!("Restored offline edit {name}")));
            ops.push(BlockOp::Insert {
                parent: Some(format!("{restored_prefix}-{name}")),
                after: Some(old),
                block: nb(&format!("late-{name}"), "Late restored item"),
            });
            ops.push(BlockOp::Insert {
                parent: Some(shared),
                after: None,
                block: nb(&format!("nested-{name}"), "Late nested edit"),
            });
        }
        b.apply(page.clone(), ops).unwrap();
        assert_eq!(push(&b, &a).conflicts, 0);
        sync(&a, &b);
        for v in [&a, &b] {
            let p = v.load_page(page.clone()).unwrap();
            assert_eq!(p.blocks.len(), 2);
            for name in ["Tasks", "Log"] {
                let root = p.blocks.iter().find(|b| b.text == name).unwrap();
                assert_eq!(root.id, format!("a-{name}"));
                assert_eq!(
                    root.children
                        .iter()
                        .map(|b| b.id.clone())
                        .collect::<Vec<_>>(),
                    [
                        format!("visible-before-{name}"),
                        format!("shared-{name}"),
                        format!("visible-tail-{name}"),
                        format!("restored-old-{name}"),
                        format!("late-{name}"),
                    ]
                );
                assert_eq!(root.children[1].text, format!("Shared offline edit {name}"));
                assert_eq!(root.children[1].children[0].id, format!("nested-{name}"));
                assert_eq!(
                    root.children[3].text,
                    format!("Restored offline edit {name}")
                );
            }
            assert_reachable(v, &page, &p.blocks);
            assert_day_read_idempotent(v, &page);
            assert!(v.list_conflicts(None).unwrap().is_empty());
        }
        let versions = (decoded_versions(&a), decoded_versions(&b));
        sync(&a, &b);
        sync(&b, &a);
        assert_eq!((decoded_versions(&a), decoded_versions(&b)), versions);
        let expected = a.load_page(page.clone()).unwrap().blocks;
        a.flush().unwrap();
        b.flush().unwrap();
        drop(a);
        drop(b);
        for v in [ta.open(), tb.open()] {
            assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
            assert_day_read_idempotent(&v, &page);
        }
    }
}

#[test]
fn explicit_section_delete_loses_to_full_offline_edit_without_conflicts() {
    let (ta, tb) = (
        TempDir::new("explicit_section_a"),
        TempDir::new("explicit_section_b"),
    );
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let root = a.load_page(page.clone()).unwrap().blocks[0].id.clone();
    a.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent: Some(root.clone()),
                after: None,
                block: NewBlock {
                    id: "old-removed".into(),
                    kind: BlockKind::Todo,
                    text: "Old content".into(),
                    checked: true,
                    marks: vec![Mark {
                        start: 0,
                        end: 3,
                        kind: MarkKind::Bold,
                    }],
                },
            },
            BlockOp::SetColor {
                id: "old-removed".into(),
                color: Some("blue_background".into()),
            },
            BlockOp::SetWidth {
                id: "old-removed".into(),
                width: Some(30),
            },
            BlockOp::Insert {
                parent: Some("old-removed".into()),
                after: None,
                block: NewBlock {
                    kind: BlockKind::LinkToPage,
                    ..nb("kept-link", "")
                },
            },
            BlockOp::SetTarget {
                id: "kept-link".into(),
                target: Some(page.clone()),
            },
        ],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![
            BlockOp::Delete {
                id: "old-removed".into(),
            },
            BlockOp::Delete { id: root.clone() },
        ],
    )
    .unwrap();
    b.apply(
        page.clone(),
        vec![
            set_text(&root, "Tasks edited offline"),
            BlockOp::Insert {
                parent: Some(root.clone()),
                after: Some("old-removed".into()),
                block: nb("new-content", "Offline addition"),
            },
        ],
    )
    .unwrap();
    let expected = b.load_page(page.clone()).unwrap().blocks;
    sync(&a, &b);
    for v in [&a, &b] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
        assert!(v.list_conflicts(None).unwrap().is_empty());
        assert_reachable(v, &page, &expected);
        assert_day_read_idempotent(v, &page);
        v.flush().unwrap();
    }
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
        assert_reachable(&v, &page, &expected);
    }
}

#[test]
fn whole_day_trash_loses_to_full_offline_note_edit_without_conflicts() {
    let (ta, tb) = (TempDir::new("trashed_day_a"), TempDir::new("trashed_day_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = a
        .daily_note(SYNC_DAY, "Original title".into(), true)
        .unwrap()
        .unwrap();
    let roots = a.load_page(page.clone()).unwrap().blocks;
    a.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent: Some(roots[0].id.clone()),
                after: None,
                block: NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb("old-parent", "Old parent")
                },
            },
            BlockOp::Insert {
                parent: Some("old-parent".into()),
                after: None,
                block: nb("old-removed", "Keep this old nested item"),
            },
        ],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![
            BlockOp::Delete {
                id: "old-removed".into(),
            },
            BlockOp::Delete {
                id: roots[0].id.clone(),
            },
        ],
    )
    .unwrap();
    a.rename_page(page.clone(), "Deleting-side title".into())
        .unwrap();
    a.delete_page(page.clone()).unwrap();
    b.rename_page(page.clone(), "Editing-side full title".into())
        .unwrap();
    b.apply(
        page.clone(),
        vec![
            set_text("old-removed", "Edited old nested item"),
            set_text(&roots[1].id, "Log edited offline"),
            BlockOp::Insert {
                parent: Some(roots[1].id.clone()),
                after: None,
                block: nb("new-log", "Offline log"),
            },
        ],
    )
    .unwrap();
    b.set_property(
        page.clone(),
        "date".into(),
        PropValue::Date {
            start_ms: SYNC_DAY + 86_400_000,
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();
    let expected = b.load_page(page.clone()).unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        let actual = v.load_page(page.clone()).unwrap();
        assert!(!actual.in_trash);
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.properties, expected.properties);
        assert_eq!(actual.blocks, expected.blocks);
        assert!(v.list_conflicts(None).unwrap().is_empty());
        assert_reachable(v, &page, &actual.blocks);
        let vv = v.page_versions().unwrap();
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected.blocks);
        assert_eq!(v.page_versions().unwrap(), vv);
        v.flush().unwrap();
    }
    let actual = a.load_page(page.clone()).unwrap();
    assert_eq!(actual, b.load_page(page.clone()).unwrap());
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        let reopened = v.load_page(page.clone()).unwrap();
        assert_eq!(reopened.title, expected.title);
        assert_eq!(reopened.properties, expected.properties);
        assert_eq!(reopened.blocks, expected.blocks);
        assert_reachable(&v, &page, &expected.blocks);
        assert!(!reopened.in_trash);
    }
}

#[test]
fn two_peers_recover_same_section_independently_and_keep_unique_addresses() {
    let (ta, tb, tc, td) = (
        TempDir::new("recover_a"),
        TempDir::new("recover_b"),
        TempDir::new("recover_c"),
        TempDir::new("recover_d"),
    );
    let (a, b, c, d) = (ta.open(), tb.open(), tc.open(), td.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let root = a.load_page(page.clone()).unwrap().blocks[0].id.clone();
    a.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: Some(root.clone()),
            after: None,
            block: nb("old", "Old content"),
        }],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![
            set_text("old", "Deleting side text"),
            BlockOp::Delete { id: root.clone() },
        ],
    )
    .unwrap();
    b.apply(
        page.clone(),
        vec![
            set_text("old", "Old content, offline edit"),
            BlockOp::Insert {
                parent: Some(root.clone()),
                after: Some("old".into()),
                block: nb("late", "New offline item"),
            },
        ],
    )
    .unwrap();
    push(&a, &c);
    push(&b, &c);
    push(&a, &d);
    push(&b, &d);
    assert_eq!(c.load_page(page.clone()).unwrap().blocks.len(), 2);
    assert_eq!(d.load_page(page.clone()).unwrap().blocks.len(), 2);
    sync(&c, &d);
    for v in [&c, &d] {
        let p = v.load_page(page.clone()).unwrap();
        assert_eq!(p.blocks.len(), 2);
        let restored = p.blocks.iter().find(|r| r.id == root).unwrap();
        assert_eq!(restored.children.len(), 2);
        assert_eq!(restored.children[0].text, "Old content, offline edit");
        assert_reachable(v, &page, &p.blocks);
        assert_day_read_idempotent(v, &page);
        let conflicts = v.list_conflicts(Some(page.clone())).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].block, "old");
        let mut sides: Vec<_> = conflicts[0]
            .sides
            .iter()
            .map(|s| s.block.as_ref().unwrap().text.as_str())
            .collect();
        sides.sort();
        assert_eq!(sides, ["Deleting side text", "Old content, offline edit"]);
    }
    c.apply(
        page.clone(),
        vec![set_text("old", "Edited on c after recovery")],
    )
    .unwrap();
    d.apply(
        page.clone(),
        vec![set_text("late", "Edited on d after recovery")],
    )
    .unwrap();
    sync(&c, &d);
    let expected = c.load_page(page.clone()).unwrap();
    assert_eq!(expected, d.load_page(page.clone()).unwrap());
    let restored = expected.blocks.iter().find(|r| r.id == root).unwrap();
    assert_eq!(restored.children[0].text, "Edited on c after recovery");
    assert_eq!(restored.children[1].text, "Edited on d after recovery");
    c.flush().unwrap();
    d.flush().unwrap();
    drop(c);
    drop(d);
    for v in [tc.open(), td.open()] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected.blocks);
        assert_reachable(&v, &page, &expected.blocks);
    }
}

#[test]
fn recovery_keeps_known_child_moved_out_by_the_offline_editor() {
    let (ta, tb) = (
        TempDir::new("moved_editor_a"),
        TempDir::new("moved_editor_b"),
    );
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let roots = a.load_page(page.clone()).unwrap().blocks;
    a.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: Some(roots[0].id.clone()),
            after: None,
            block: nb("moved-out", "Keep the moved item"),
        }],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![BlockOp::Delete {
            id: roots[0].id.clone(),
        }],
    )
    .unwrap();
    b.apply(
        page.clone(),
        vec![
            BlockOp::Move {
                id: "moved-out".into(),
                parent: Some(roots[1].id.clone()),
                after: None,
            },
            set_text(&roots[0].id, "Tasks edited offline"),
        ],
    )
    .unwrap();
    let expected = b.load_page(page.clone()).unwrap().blocks;
    sync(&a, &b);
    for v in [&a, &b] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
        assert_reachable(v, &page, &expected);
        assert!(v.list_conflicts(None).unwrap().is_empty());
        v.flush().unwrap();
    }
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
        assert_reachable(&v, &page, &expected);
    }
}

#[test]
fn duplicated_sections_repair_on_import_or_open_and_second_open_is_read_only() {
    let (ta, tb) = (
        TempDir::new("legacy_seeds_a"),
        TempDir::new("legacy_seeds_b"),
    );
    let (a, b) = (ta.open(), tb.open());
    let page = independent_day_notes(&a, &b);
    let journal = a.find_by_role("journal".into()).unwrap().unwrap();
    let merged = LoroDoc::new();
    merged
        .import(&a.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    merged
        .import(&b.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    assert_eq!(merged.get_tree("blocks").roots().len(), 4);
    let bytes = merged.export(ExportMode::Snapshot).unwrap();
    for trigger in ["open", "import", "failed_flush"] {
        let t = TempDir::new(trigger);
        let mut v = t.open();
        v.import_updates(
            journal.clone(),
            a.export_updates(journal.clone(), vec![]).unwrap(),
        )
        .unwrap();
        match trigger {
            "open" | "failed_flush" => {
                fs::write(t.0.join(".vault/docs").join(format!("{page}.loro")), &bytes).unwrap();
                v.reindex().unwrap();
                assert_eq!(repair_marker(&t), None);
                // Reads do not repair externally supplied legacy snapshots.
                let vv = v.page_versions().unwrap();
                assert_eq!(v.load_page(page.clone()).unwrap().blocks.len(), 4);
                assert_eq!(
                    v.daily_note(SYNC_DAY, String::new(), false).unwrap(),
                    Some(page.clone())
                );
                assert_eq!(v.page_versions().unwrap(), vv);
                drop(v);
                // A pre-marker index already has the current schema. Its
                // additive migration gains meta without rebuilding pages.
                rusqlite::Connection::open(t.0.join(".vault/index.sqlite"))
                    .unwrap()
                    .execute("DROP TABLE meta", [])
                    .unwrap();
                if trigger == "failed_flush" {
                    let blocked = t.0.join(".vault/docs").join(format!("{page}.loro.tmp"));
                    fs::create_dir(&blocked).unwrap();
                    assert!(Vault::open(t.root()).is_err());
                    assert_eq!(
                        repair_marker(&t),
                        None,
                        "failed durable snapshot must not advance marker"
                    );
                    fs::remove_dir(blocked).unwrap();
                }
                v = t.open();
            }
            "import" => {
                v.import_updates(page.clone(), bytes.clone()).unwrap();
            }
            _ => unreachable!(),
        }
        let repaired = v.load_page(page.clone()).unwrap();
        let marker = repair_marker(&t).expect("successful open writes repair marker");
        assert_eq!(repaired.blocks.len(), 2);
        assert!(repaired.blocks.iter().all(|root| root.children.len() == 2));
        assert_reachable(&v, &page, &repaired.blocks);
        assert_day_read_idempotent(&v, &page);
        assert!(v.list_conflicts(None).unwrap().is_empty());
        v.flush().unwrap();
        let versions = decoded_versions(&v);
        drop(v);
        // Compare file bytes and modification times, not just a rendered page:
        // no repair log append, snapshot rewrite or markdown rewrite on reopen.
        let disk = day_files(&t);
        let v = t.open();
        assert_eq!(v.load_page(page.clone()).unwrap().blocks, repaired.blocks);
        assert_eq!(
            v.daily_note(SYNC_DAY, String::new(), false).unwrap(),
            Some(page.clone())
        );
        assert_eq!(decoded_versions(&v), versions);
        assert_eq!(repair_marker(&t), Some(marker));
        v.flush().unwrap();
        assert_eq!(day_files(&t), disk);
        drop(v);
        assert_eq!(day_files(&t), disk);
    }
}

fn repair_marker(t: &TempDir) -> Option<u32> {
    use rusqlite::OptionalExtension;
    rusqlite::Connection::open(t.0.join(".vault/index.sqlite"))
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE key='day_notes_repaired_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .unwrap()
        .map(|v| v.parse().unwrap())
}

#[test]
fn startup_marker_skips_the_pass_and_older_or_rebuilt_markers_rerun_once() {
    let (ta, tb, t) = (
        TempDir::new("marker_source_a"),
        TempDir::new("marker_source_b"),
        TempDir::new("marker_copy"),
    );
    let (a, b, v) = (ta.open(), tb.open(), t.open());
    let version = repair_marker(&t).unwrap();
    let page = independent_day_notes(&a, &b);
    let journal = a.find_by_role("journal".into()).unwrap().unwrap();
    let merged = LoroDoc::new();
    merged
        .import(&a.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    merged
        .import(&b.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    let bytes = merged.export(ExportMode::Snapshot).unwrap();
    v.import_updates(journal.clone(), a.export_updates(journal, vec![]).unwrap())
        .unwrap();
    fs::write(t.0.join(".vault/docs").join(format!("{page}.loro")), &bytes).unwrap();
    v.reindex().unwrap();
    assert_eq!(repair_marker(&t), None);
    let versions = decoded_versions(&v);
    drop(v);
    let db = rusqlite::Connection::open(t.0.join(".vault/index.sqlite")).unwrap();
    // Deliberately leave a legacy duplicate snapshot under a valid marker:
    // an erroneously repeated repair pass would visibly consolidate it.
    db.execute(
        "INSERT INTO meta(key,value) VALUES ('day_notes_repaired_version',?1)",
        [version.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO meta(key,value) VALUES ('future_key','keep me')",
        [],
    )
    .unwrap();
    drop(db);
    let disk = day_files(&t);
    let v = t.open();
    assert_eq!(v.load_page(page.clone()).unwrap().blocks.len(), 4);
    assert_eq!(decoded_versions(&v), versions);
    v.flush().unwrap();
    drop(v);
    assert_eq!(day_files(&t), disk);
    // Simulate raising N: the stored version is now older than this binary.
    let db = rusqlite::Connection::open(t.0.join(".vault/index.sqlite")).unwrap();
    db.execute(
        "UPDATE meta SET value=?1 WHERE key='day_notes_repaired_version'",
        [(version - 1).to_string()],
    )
    .unwrap();
    drop(db);
    let v = t.open();
    assert_eq!(v.load_page(page.clone()).unwrap().blocks.len(), 2);
    assert_eq!(repair_marker(&t), Some(version));
    let versions = decoded_versions(&v);
    v.flush().unwrap();
    drop(v);
    let disk = day_files(&t);
    let v = t.open();
    assert_eq!(decoded_versions(&v), versions);
    assert_eq!(v.load_page(page.clone()).unwrap().blocks.len(), 2);
    v.flush().unwrap();
    assert_eq!(day_files(&t), disk);
    assert_eq!(
        rusqlite::Connection::open(t.0.join(".vault/index.sqlite"))
            .unwrap()
            .query_row("SELECT value FROM meta WHERE key='future_key'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "keep me"
    );
    // The existing explicit rebuild loses local metadata. Next open repairs
    // once and writes a fresh marker, even though the sweep itself is a no-op.
    v.reindex().unwrap();
    assert_eq!(repair_marker(&t), None);
    drop(v);
    let v = t.open();
    assert_eq!(repair_marker(&t), Some(version));
    assert_eq!(decoded_versions(&v), versions);
    v.flush().unwrap();
    drop(v);
    let disk = day_files(&t);
    let v = t.open();
    assert_eq!(repair_marker(&t), Some(version));
    assert_eq!(decoded_versions(&v), versions);
    v.flush().unwrap();
    drop(v);
    assert_eq!(day_files(&t), disk);
}

#[test]
fn repair_marker_is_local_and_does_not_gate_cold_sync_imports() {
    let (ta, tb, tc) = (
        TempDir::new("local_marker_a"),
        TempDir::new("local_marker_b"),
        TempDir::new("local_marker_c"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let page = independent_day_notes(&a, &b);
    let journal = a.find_by_role("journal".into()).unwrap().unwrap();
    let merged = LoroDoc::new();
    merged
        .import(&a.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    merged
        .import(&b.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    rusqlite::Connection::open(tc.0.join(".vault/index.sqlite"))
        .unwrap()
        .execute(
            "UPDATE meta SET value='999' WHERE key='day_notes_repaired_version'",
            [],
        )
        .unwrap();
    // Page first, Journal later: identity-dependent cold repair still runs
    // on the completed import batch despite the destination's newer marker.
    c.import_updates(page.clone(), merged.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    assert_eq!(c.load_page(page.clone()).unwrap().blocks.len(), 4);
    c.import_updates(journal.clone(), a.export_updates(journal, vec![]).unwrap())
        .unwrap();
    assert_eq!(c.load_page(page.clone()).unwrap().blocks.len(), 2);
    assert_eq!(repair_marker(&tc), Some(999));
    push(&c, &a);
    assert_ne!(repair_marker(&ta), Some(999));
    c.flush().unwrap();
    drop(c);
    let c = tc.open();
    assert_eq!(repair_marker(&tc), Some(999));
    assert_eq!(c.load_page(page).unwrap().blocks.len(), 2);
}

fn day_files(t: &TempDir) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut files = Vec::new();
    for dir in [".vault/docs", ".vault/logs", "pages"] {
        for entry in fs::read_dir(t.0.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                files.push((
                    path.clone(),
                    fs::read(&path).unwrap(),
                    fs::metadata(&path).unwrap().modified().unwrap(),
                ));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn decoded_versions(v: &Vault) -> HashMap<String, VersionVector> {
    v.page_versions()
        .unwrap()
        .into_iter()
        .map(|p| (p.page, VersionVector::decode(&p.version).unwrap()))
        .collect()
}

#[test]
fn single_day_item_deletion_without_later_edits_stays_deleted_after_sync_and_reopen() {
    let (ta, tb) = (TempDir::new("single_item_a"), TempDir::new("single_item_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let root = a.load_page(page.clone()).unwrap().blocks[0].id.clone();
    a.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent: Some(root.clone()),
                after: None,
                block: nb("keep", "Keep"),
            },
            BlockOp::Insert {
                parent: Some(root.clone()),
                after: Some("keep".into()),
                block: nb("remove", "Remove"),
            },
        ],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![BlockOp::Delete {
            id: "remove".into(),
        }],
    )
    .unwrap();
    sync(&a, &b);
    sync(&b, &a);
    for v in [&a, &b] {
        let p = v.load_page(page.clone()).unwrap();
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(
            p.blocks[0]
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["keep"]
        );
        assert!(v.list_conflicts(None).unwrap().is_empty());
        v.flush().unwrap();
    }
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        let p = v.load_page(page.clone()).unwrap();
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(
            p.blocks[0]
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["keep"]
        );
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
}

#[test]
fn deleted_day_identity_restores_an_editor_move_and_unseen_edits_revive_purges() {
    let (ta, tb) = (TempDir::new("moved_day_a"), TempDir::new("moved_day_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let roots = a.load_page(page.clone()).unwrap().blocks;
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![BlockOp::Delete {
            id: roots[0].id.clone(),
        }],
    )
    .unwrap();
    a.delete_page(page.clone()).unwrap();
    let outside = b.create_page(None, "Outside Journal".into()).unwrap();
    b.move_page(page.clone(), Some(outside.clone()), None)
        .unwrap();
    b.rename_page(page.clone(), "Moved editing-side note".into())
        .unwrap();
    let expected = b.load_page(page.clone()).unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        let actual = v.load_page(page.clone()).unwrap();
        assert_eq!(actual.parent, Some(outside.clone()));
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.blocks, expected.blocks);
        assert_eq!(actual.properties, expected.properties);
        assert!(!actual.in_trash);
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
    a.delete_page(page.clone()).unwrap();
    a.purge_page(page.clone()).unwrap();
    b.apply(
        page.clone(),
        vec![set_text(&roots[1].id, "Edited after purge")],
    )
    .unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        let revived = v.load_page(page.clone()).unwrap();
        assert!(revived
            .blocks
            .iter()
            .any(|block| block.id == roots[1].id && block.text == "Edited after purge"));
        assert!(!v.purged_pages().contains(&page));
    }
}

#[test]
fn stale_day_note_replay_from_before_purge_stays_purged() {
    let (ta, tb) = (TempDir::new("stale_day_a"), TempDir::new("stale_day_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let roots = a.load_page(page.clone()).unwrap().blocks;
    sync(&a, &b);
    b.apply(
        page.clone(),
        vec![set_text(&roots[1].id, "Edited before purge")],
    )
    .unwrap();
    sync(&a, &b);
    // B's edit is already on A, so it is inside the purge's version vector.
    let stale = b.export_updates(page.clone(), vec![]).unwrap();
    a.delete_page(page.clone()).unwrap();
    a.purge_page(page.clone()).unwrap();
    assert_eq!(a.import_updates(page.clone(), stale).unwrap().skipped, 1);
    sync(&a, &b);
    for v in [&a, &b] {
        assert!(matches!(
            v.load_page(page.clone()),
            Err(VaultError::NotFound(_))
        ));
        assert!(v.purged_pages().contains(&page));
    }
}

#[test]
fn timestamp_only_offline_change_does_not_restore_a_trashed_day() {
    let (ta, tb) = (TempDir::new("timestamp_a"), TempDir::new("timestamp_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    sync(&a, &b);
    a.delete_page(page.clone()).unwrap();
    let updated = LoroDoc::new();
    updated
        .import(&b.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    updated.get_map("page").insert("updated", 987_i64).unwrap();
    updated.commit();
    b.import_updates(page.clone(), updated.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        assert!(v.load_page(page.clone()).unwrap().in_trash);
        assert_eq!(v.daily_note(SYNC_DAY, String::new(), false).unwrap(), None);
        v.flush().unwrap();
    }
    drop(a);
    drop(b);
    assert!(ta.open().load_page(page.clone()).unwrap().in_trash);
    assert!(tb.open().load_page(page).unwrap().in_trash);
}

#[test]
fn ordinary_tasks_section_deletion_retains_generic_conflicts_with_a_journal_present() {
    let (ta, tb) = (
        TempDir::new("ordinary_day_a"),
        TempDir::new("ordinary_day_b"),
    );
    let (a, b) = (ta.open(), tb.open());
    a.daily_note(SYNC_DAY, "Day".into(), true).unwrap();
    let page = a.create_page(None, "Ordinary page".into()).unwrap();
    a.apply(
        page.clone(),
        vec![
            ins(
                None,
                NewBlock {
                    kind: BlockKind::Toggle,
                    ..nb("tasks", "Tasks")
                },
            ),
            BlockOp::Insert {
                parent: Some("tasks".into()),
                after: None,
                block: nb("old", "Original"),
            },
        ],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(page.clone(), vec![BlockOp::Delete { id: "tasks".into() }])
        .unwrap();
    b.apply(page.clone(), vec![set_text("old", "Edited offline")])
        .unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        let restored = v.load_page(page.clone()).unwrap();
        assert_eq!(restored.blocks.len(), 1);
        assert_eq!(restored.blocks[0].id, "tasks");
        assert_eq!(restored.blocks[0].children[0].id, "old");
        assert_eq!(restored.blocks[0].children[0].text, "Edited offline");
        let pending = v.list_conflicts(Some(page.clone())).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].scope,
            ConflictScope::Subtree {
                root: "tasks".into()
            }
        );
        assert_eq!(pending[0].status, ConflictStatus::Pending);
        assert!(
            matches!(&pending[0].applied[0].by, ResolutionBy::Auto { rule }
            if rule == "edit-beats-delete")
        );
    }
}

#[test]
fn intentional_day_deletions_and_unrelated_section_edits_do_not_restore() {
    let (ta, tb) = (TempDir::new("intentional_a"), TempDir::new("intentional_b"));
    let (a, b) = (ta.open(), tb.open());
    let page = a.daily_note(SYNC_DAY, "Day".into(), true).unwrap().unwrap();
    let roots = a.load_page(page.clone()).unwrap().blocks;
    sync(&a, &b);
    a.apply(
        page.clone(),
        vec![BlockOp::Delete {
            id: roots[0].id.clone(),
        }],
    )
    .unwrap();
    // The local write updates the page timestamp, but is outside Tasks.
    b.apply(page.clone(), vec![set_text(&roots[1].id, "Other log name")])
        .unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        assert_eq!(v.load_page(page.clone()).unwrap().blocks.len(), 1);
        assert_day_read_idempotent(v, &page);
    }
    a.delete_page(page.clone()).unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        assert!(v.load_page(page.clone()).unwrap().in_trash);
        assert_eq!(v.daily_note(SYNC_DAY, String::new(), false).unwrap(), None);
        assert!(v.load_page(page.clone()).unwrap().in_trash);
    }
}

#[test]
fn concurrent_edit_of_one_block_is_a_conflict() {
    let (ta, tb) = (TempDir::new("a"), TempDir::new("b"));
    let (a, b) = (ta.open(), tb.open());
    let p = a.create_page(None, "Doc".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(None, nb("1", "the cat sat")),
            ins(Some("1"), nb("2", "x")),
        ],
    )
    .unwrap();
    sync(&a, &b);
    a.apply(p.clone(), vec![set_text("1", "the black cat sat")])
        .unwrap();
    b.apply(p.clone(), vec![set_text("1", "the cat sat down")])
        .unwrap();
    // A block deleted on one side and edited on the other.
    a.apply(p.clone(), vec![BlockOp::Delete { id: "2".into() }])
        .unwrap();
    b.apply(p.clone(), vec![set_text("2", "x changed")])
        .unwrap();
    push(&a, &b);
    let cs = b.list_conflicts(Some(p.clone())).unwrap();
    assert_eq!(cs.len(), 2, "{cs:?}");
    let c1 = cs.iter().find(|c| c.block == "1").unwrap();
    assert_eq!(c1.sides[0].device, b.device_id());
    assert_eq!(c1.sides[1].device, a.device_id());
    assert_eq!(c1.sides[0].block.as_ref().unwrap().text, "the cat sat down");
    assert_eq!(
        c1.sides[1].block.as_ref().unwrap().text,
        "the black cat sat"
    );
    let c2 = cs.iter().find(|c| c.block == "2").unwrap();
    assert!(c2.sides[1].block.is_none());
    // The text rule shows one side, the later edit (device id breaks ties),
    // until the user confirms or switches.
    let later = c1
        .sides
        .iter()
        .max_by(|x, y| {
            x.time_ms
                .cmp(&y.time_ms)
                .then_with(|| x.device.as_bytes().cmp(y.device.as_bytes()))
        })
        .unwrap();
    assert_eq!(texts(&b, &p)[0], later.block.as_ref().unwrap().text);
    // The conflict syncs as recorded; the other side records no second one.
    push(&b, &a);
    assert_eq!(a.list_conflicts(None).unwrap(), cs);
    b.resolve_conflict(p.clone(), c1.id.clone(), ConflictResolution::UseSide(1))
        .unwrap();
    b.resolve_conflict(p.clone(), c2.id.clone(), ConflictResolution::UseSide(0))
        .unwrap();
    assert!(b.list_conflicts(None).unwrap().is_empty());
    sync(&a, &b);
    for v in [&a, &b] {
        assert!(v.list_conflicts(None).unwrap().is_empty());
        assert_eq!(texts(v, &p), vec!["the black cat sat", "x changed"]);
    }
    // Edit the note normally, then confirm Keep; a consumed record is NotFound.
    a.apply(p.clone(), vec![set_text("1", "A")]).unwrap();
    b.apply(p.clone(), vec![set_text("1", "B")]).unwrap();
    push(&a, &b);
    let pending = b.list_conflicts(None).unwrap();
    assert_eq!(pending.len(), 1);
    let id = pending[0].id.clone();
    b.apply(p.clone(), vec![set_text("1", "A and B")]).unwrap();
    b.resolve_conflict(p.clone(), id.clone(), ConflictResolution::Keep)
        .unwrap();
    assert_eq!(texts(&b, &p)[0], "A and B");
    assert!(matches!(
        b.resolve_conflict(p.clone(), id, ConflictResolution::Keep),
        Err(VaultError::NotFound(_))
    ));
}

#[test]
fn trash_moves_and_purges_sync() {
    let (ta, tb) = (TempDir::new("a"), TempDir::new("b"));
    let (a, b) = (ta.open(), tb.open());
    let x = a.create_page(None, "X".into()).unwrap();
    let y = a.create_page(None, "Y".into()).unwrap();
    let z = a.create_page(Some(x.clone()), "Z".into()).unwrap();
    sync(&a, &b);
    b.flush().unwrap();
    assert!(md_exists(&tb, &z));
    // Move on a, trash on b.
    a.move_page(y.clone(), Some(x.clone()), None).unwrap();
    b.delete_page(x.clone()).unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        assert_eq!(
            v.load_page(y.clone()).unwrap().parent.as_deref(),
            Some(x.as_str())
        );
        let trash: Vec<String> = v
            .list_trash()
            .unwrap()
            .into_iter()
            .map(|t| t.page.id)
            .collect();
        assert_eq!(trash, vec![x.clone()]);
        assert_eq!(v.page_count().unwrap(), 0);
    }
    a.flush().unwrap();
    assert!(!md_exists(&ta, &z));
    // Restore on a comes back on b.
    a.restore_page(x.clone()).unwrap();
    sync(&a, &b);
    assert_eq!(b.page_count().unwrap(), 3);
    b.flush().unwrap();
    assert!(md_exists(&tb, &z));
    // Purge on b; a edits Z meanwhile and creates a page under X.
    b.delete_page(x.clone()).unwrap();
    b.purge_page(x.clone()).unwrap();
    a.rename_page(z.clone(), "Z edited".into()).unwrap();
    let w = a.create_page(Some(x.clone()), "W".into()).unwrap();
    sync(&a, &b);
    for v in [&a, &b] {
        let restored = v.load_page(z.clone()).unwrap();
        assert_eq!(restored.title, "Z edited");
        assert!(!restored.in_trash);
        assert_eq!(restored.parent, None);
        assert!(v.purged_pages().contains(&x));
        // W was created under X on a: it moves to the top level.
        let wp = v.load_page(w.clone()).unwrap();
        assert_eq!(wp.parent, None);
        assert!(!wp.in_trash);
    }
    assert_eq!(a.page_count().unwrap(), 2);
    assert_eq!(b.page_count().unwrap(), 2);
    // Tombstones survive a reopen.
    drop(a);
    let a = ta.open();
    assert!(a.purged_pages().contains(&x));
    let r = a
        .import_updates_batch_legacy(vec![PageUpdate {
            page: x,
            bytes: vec![1, 2, 3],
        }])
        .expect("legacy purged pages are skipped before decoding");
    assert_eq!(r.skipped, 1);
}

#[test]
fn attachments_travel_by_id() {
    let (ta, tb) = (TempDir::new("a"), TempDir::new("b"));
    let (a, b) = (ta.open(), tb.open());
    let bytes = b"not really a png".to_vec();
    let id = a
        .add_attachment_bytes("pic.png".into(), bytes.clone())
        .unwrap();
    assert_eq!(sha256_hex(&bytes), id);
    let p = a.create_page(None, "With image".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(
                None,
                NewBlock {
                    kind: BlockKind::Image,
                    ..nb("i", "")
                },
            ),
            BlockOp::SetTarget {
                id: "i".into(),
                target: Some(id.clone()),
            },
        ],
    )
    .unwrap();
    assert_eq!(a.attachment_ids().unwrap(), vec![id.clone()]);
    push(&a, &b);
    b.flush().unwrap();
    assert_eq!(b.missing_attachments().unwrap(), vec![id.clone()]);
    assert!(b.attachment_path(id.clone()).is_err());
    assert!(matches!(
        b.store_attachment(id.clone(), "pic.png".into(), b"other".to_vec()),
        Err(VaultError::Invalid(_))
    ));
    b.store_attachment(id.clone(), "pic.png".into(), bytes.clone())
        .unwrap();
    assert!(b.missing_attachments().unwrap().is_empty());
    assert_eq!(
        fs::read(b.attachment_path(id.clone()).unwrap()).unwrap(),
        bytes
    );
    let info = b.attachment_info(id).unwrap();
    assert_eq!(info.mime, "image/png");
}

#[test]
fn bad_updates_are_rejected() {
    let (ta, tb) = (TempDir::new("a"), TempDir::new("b"));
    let (a, b) = (ta.open(), tb.open());
    let p = a.create_page(None, "P".into()).unwrap();
    let bytes = a.export_updates(p.clone(), vec![]).unwrap();
    // Updates of one page presented as another.
    assert!(b.import_updates(new_id(), bytes.clone()).is_err());
    assert!(b.import_updates(p.clone(), vec![0, 1, 2]).is_err());
    assert!(b.export_updates(p.clone(), vec![]).is_err());
    assert!(version_includes(&[9, 9, 9], &[]).is_err());
    b.import_updates(p.clone(), bytes).unwrap();
    assert_eq!(b.load_page(p).unwrap().title, "P");
}

#[test]
fn partial_import_error_preserves_blocks_metadata_and_feed() {
    const PAGE: &str = "partial-import";
    // Fixed peers and separate changes make the compatible branch and the
    // branch based on pruned history deterministic, using real Loro imports.
    let desktop = LoroDoc::new();
    desktop.set_peer_id(100).unwrap();
    desktop.set_change_merge_interval(0);
    let meta = desktop.get_map("page");
    meta.insert("id", PAGE).unwrap();
    meta.insert("title", "Before root").unwrap();
    meta.insert("created", 1_i64).unwrap();
    meta.insert("updated", 1_i64).unwrap();
    let insert_block = |doc: &LoroDoc, id: &str, text: &str| {
        let tree = doc.get_tree("blocks");
        tree.enable_fractional_index(0);
        let node = tree.create(TreeParentId::Root).unwrap();
        let block = tree.get_meta(node).unwrap();
        block.insert("id", id).unwrap();
        block.insert("kind", "paragraph").unwrap();
        block.insert("checked", false).unwrap();
        block
            .insert_container("text", LoroText::new())
            .unwrap()
            .insert(0, text)
            .unwrap();
    };
    insert_block(&desktop, "base", "base text");
    desktop.commit();
    let offline = LoroDoc::new();
    offline
        .import(&desktop.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    offline.set_peer_id(300).unwrap();
    offline.set_change_merge_interval(0);
    meta.insert("title", "At root").unwrap();
    desktop.commit();
    let root = desktop.oplog_frontiers();
    meta.insert("title", "Retained title").unwrap();
    desktop.commit();
    let shallow = desktop.export(ExportMode::shallow_snapshot(&root)).unwrap();
    let proof = LoroDoc::new();
    proof.import(&shallow).unwrap();
    assert!(proof.is_shallow());

    let t = TempDir::new("partial_import");
    let v = t.open();
    v.import_updates(PAGE.into(), shallow).unwrap();
    // Warm PageDoc's block-ID maps before the partial import adds a node.
    assert_eq!(texts(&v, PAGE), vec!["base text"]);
    let before = v.page_versions().unwrap()[0].version.clone();
    let seq = v.poll_changes(0).seq;
    meta.insert("title", "Accepted title").unwrap();
    insert_block(&desktop, "accepted", "accepted text");
    desktop.commit();
    insert_block(&offline, "rejected", "rejected text");
    offline.commit();
    desktop
        .import(&offline.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    let bytes = desktop
        .export(ExportMode::updates(
            &VersionVector::decode(&before).unwrap(),
        ))
        .unwrap();
    let error = v.import_updates(PAGE.into(), bytes.clone()).unwrap_err();
    assert!(matches!(
        &error,
        VaultError::Crdt(message)
            if message == &LoroError::ImportUpdatesThatDependsOnOutdatedVersion.to_string()
    ));
    let applied = v.page_versions().unwrap()[0].version.clone();
    assert_ne!(applied, before);
    assert!(!version_includes(&applied, &desktop.oplog_vv().encode()).unwrap());
    // Metadata and notifications follow the normal successful-import path
    // for the portion that applied, before a flush or another local edit.
    assert_eq!(v.list_pages(None, 0, 5).unwrap()[0].title, "Accepted title");
    let feed = v.poll_changes(seq);
    assert!(feed.changes.contains(&Change::PageChanged(PAGE.into())));
    assert!(feed.changes.contains(&Change::TreeChanged));
    let retry_error = v.import_updates(PAGE.into(), bytes).unwrap_err();
    assert!(matches!(
        &retry_error,
        VaultError::Crdt(message)
            if message == &LoroError::ImportUpdatesThatDependsOnOutdatedVersion.to_string()
    ));
    assert_eq!(v.page_versions().unwrap()[0].version, applied);
    assert!(v.poll_changes(feed.seq).changes.is_empty());
    // Edit by the imported ID before reading blocks again: stale ID maps
    // would reject this even though Loro already contains the new node.
    v.apply(
        PAGE.into(),
        vec![set_text("accepted", "edited after error")],
    )
    .unwrap();
    let page = v.load_page(PAGE.into()).unwrap();
    assert_eq!(page.title, "Accepted title");
    assert_eq!(page.blocks.len(), 2);
    assert_eq!(
        page.blocks
            .iter()
            .find(|b| b.id == "accepted")
            .unwrap()
            .text,
        "edited after error"
    );
    assert!(page.blocks.iter().all(|b| b.id != "rejected"));
    v.flush().unwrap();
    let version = v.page_versions().unwrap()[0].version.clone();
    drop(v);
    let reopened = t.open();
    assert_eq!(reopened.load_page(PAGE.into()).unwrap(), page);
    assert_eq!(reopened.page_versions().unwrap()[0].version, version);
    assert!(reopened.search("rejected".into(), 5).unwrap().is_empty());
    assert_eq!(
        reopened.search("edited after error".into(), 5).unwrap()[0].id,
        PAGE
    );
}

#[test]
fn wait_changes_wakes_on_a_change() {
    let t = TempDir::new("wait");
    let v = std::sync::Arc::new(t.open());
    let seq = v.poll_changes(0).seq;
    let start = Instant::now();
    let b = v.wait_changes(seq, 100);
    assert!(b.changes.is_empty());
    assert!(start.elapsed() >= Duration::from_millis(90));
    let v2 = v.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        v2.create_page(None, "Wake".into()).unwrap()
    });
    let start = Instant::now();
    let b = v.wait_changes(seq, 5000);
    let id = h.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(4));
    assert!(b.changes.contains(&Change::PageCreated(id)));
}

#[test]
fn sync_feed_ignores_index_only_wakes_but_retains_document_changes() {
    let t = TempDir::new("wait_sync");
    let v = std::sync::Arc::new(t.open());
    let seq = v.poll_sync_changes(0).seq;
    let raw_seq = v.poll_changes(0).seq;
    let writer = v.clone();
    let indexed = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        writer
            .upsert_search_row(
                "test".into(),
                ExternalSearchRow {
                    id: "local-index-only".into(),
                    title: "External".into(),
                    body: "preview".into(),
                    page_type: "chat".into(),
                    target: None,
                    updated_ms: 1,
                },
            )
            .unwrap();
    });
    let start = Instant::now();
    let unchanged = v.wait_sync_changes(seq, 250);
    indexed.join().unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(200),
        "index-only work must not end a sync wait"
    );
    assert_eq!(
        unchanged,
        ChangeBatch {
            seq,
            changes: Vec::new(),
            reset: false
        }
    );
    assert!(v
        .poll_changes(raw_seq)
        .changes
        .iter()
        .any(|change| matches!(change, Change::IndexCommitted { .. })));
    let editor = v.clone();
    let edit = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        editor.create_page(None, "Wake transport".into()).unwrap()
    });
    let start = Instant::now();
    let changed = v.wait_sync_changes(seq, 5000);
    let id = edit.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(4));
    assert!(changed.changes.contains(&Change::PageCreated(id)));
    assert!(changed
        .changes
        .iter()
        .all(|change| !matches!(change, Change::IndexCommitted { .. })));
}

#[test]
fn purge_restore_preserves_whole_editor_state_in_both_orders() {
    for purge_first in [false, true] {
        let (ta, tb) = (TempDir::new("whole_editor"), TempDir::new("purger"));
        let (a, b) = (ta.open(), tb.open());
        let p = a.create_page(None, "Whole page".into()).unwrap();
        a.apply(
            p.clone(),
            vec![
                ins(None, nb("kept", "original")),
                ins(Some("kept"), nb("edited", "before")),
            ],
        )
        .unwrap();
        sync(&a, &b);
        // The deleting branch removes content before permanent deletion.
        b.apply(p.clone(), vec![BlockOp::Delete { id: "kept".into() }])
            .unwrap();
        b.delete_page(p.clone()).unwrap();
        b.purge_page(p.clone()).unwrap();
        let tombstones = b.purge_records();
        a.apply(p.clone(), vec![set_text("edited", "offline edit")])
            .unwrap();
        if purge_first {
            push(&b, &a);
            push(&a, &b);
        } else {
            push(&a, &b);
            push(&b, &a);
        }
        for v in [&a, &b] {
            assert_eq!(texts(v, &p), vec!["original", "offline edit"]);
            assert!(!v.load_page(p.clone()).unwrap().in_trash);
            assert!(!v.purged_pages().contains(&p));
            assert!(!v.list_versions(p.clone()).unwrap().is_empty());
            v.import_purge_records(tombstones.clone()).unwrap();
            assert_eq!(texts(v, &p), vec!["original", "offline edit"]);
            v.flush().unwrap();
        }
        drop(b);
        let b = tb.open();
        b.import_purge_records(tombstones).unwrap();
        assert_eq!(texts(&b, &p), vec!["original", "offline edit"]);
        assert!(!b.load_page(p).unwrap().in_trash);
    }
}

/// Purger B deletes and purges a page that editor A then edits offline.
/// Returns the page and the tombstones B forwards.
fn purge_against_offline_edit(
    editor: &Vault,
    purger: &Vault,
    tag: &str,
) -> (PageId, Vec<PurgeRecord>) {
    let p = editor.create_page(None, tag.into()).unwrap();
    editor
        .apply(p.clone(), vec![ins(None, nb("body", "original"))])
        .unwrap();
    sync(editor, purger);
    purger.delete_page(p.clone()).unwrap();
    purger.purge_page(p.clone()).unwrap();
    let tombstones = purger.purge_records();
    editor
        .apply(p.clone(), vec![set_text("body", "offline edit")])
        .unwrap();
    (p, tombstones)
}

#[test]
fn purge_conflict_names_the_purging_device_on_each_replica() {
    let (ta, tb) = (
        TempDir::new("purge_side_editor"),
        TempDir::new("purge_side_purger"),
    );
    let (editor, purger) = (ta.open(), tb.open());
    let (p, tombstones) = purge_against_offline_edit(&editor, &purger, "Purge side");
    let purged_by = purger.device_id();
    assert_eq!(tombstones[0].device.as_deref(), Some(purged_by.as_str()));
    purger
        .import_updates(p.clone(), editor.export_updates(p.clone(), vec![]).unwrap())
        .unwrap();
    editor.import_purge_records(tombstones).unwrap();
    for vault in [&purger, &editor] {
        let conflicts = vault.list_conflicts(Some(p.clone())).unwrap();
        assert_eq!(conflicts.len(), 1);
        // Side 0 is the editing side and side 1 the purge; the auto rule
        // applied side 0.
        assert!(matches!(
            conflicts[0].sides[0].state,
            SideState::Present { .. }
        ));
        assert!(matches!(
            conflicts[0].sides[1].state,
            SideState::Purged { .. }
        ));
        assert_eq!(conflicts[0].sides[1].device, purged_by);
        assert_eq!(
            conflicts[0].applied[0].resolution,
            Some(ConflictResolution::UseSide(0))
        );
    }
}

#[test]
fn fieldless_tombstone_names_the_purging_side_unknown() {
    let (ta, tb) = (
        TempDir::new("fieldless_editor"),
        TempDir::new("fieldless_purger"),
    );
    let (editor, purger) = (ta.open(), tb.open());
    let (p, tombstones) = purge_against_offline_edit(&editor, &purger, "Fieldless purge");
    let fieldless: Vec<PurgeRecord> = tombstones
        .into_iter()
        .map(|record| PurgeRecord {
            device: None,
            ..record
        })
        .collect();
    editor.import_purge_records(fieldless).unwrap();
    let conflicts = editor.list_conflicts(Some(p)).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].sides[1].device, "unknown");
}

#[test]
fn purging_device_persists_and_fieldless_duplicates_keep_it() {
    let (ta, tb, tc) = (
        TempDir::new("device_editor"),
        TempDir::new("device_purger"),
        TempDir::new("device_relay"),
    );
    let (editor, purger, relay) = (ta.open(), tb.open(), tc.open());
    let (p, tombstones) = purge_against_offline_edit(&editor, &purger, "Persisted device");
    let purged_by = purger.device_id();
    let tombstone = tombstones[0].clone();
    // A relay forwards the original identity; it never stamps its own device.
    relay.import_purge_records(vec![tombstone.clone()]).unwrap();
    assert_eq!(
        relay.purge_records()[0].device.as_deref(),
        Some(purged_by.as_str())
    );
    drop(purger);
    let purger = tb.open();
    assert_eq!(
        purger.purge_records()[0].device.as_deref(),
        Some(purged_by.as_str())
    );
    // A fieldless copy and an ID-only legacy copy of the same purge keep the
    // known device and do not add a second tombstone.
    purger
        .import_purge_records(vec![
            PurgeRecord {
                device: None,
                ..tombstone.clone()
            },
            PurgeRecord {
                page: p.clone(),
                version: None,
                device: None,
            },
        ])
        .unwrap();
    assert_eq!(purger.purge_records().len(), 1);
    assert_eq!(
        purger.purge_records()[0].device.as_deref(),
        Some(purged_by.as_str())
    );
    drop(purger);
    let purger = tb.open();
    assert_eq!(purger.purge_records().len(), 1);
    assert_eq!(
        purger.purge_records()[0].device.as_deref(),
        Some(purged_by.as_str())
    );
}

#[test]
fn purge_state_without_device_field_reads_as_unknown() {
    let t = TempDir::new("purge_legacy_state");
    let v = t.open();
    let p = v.create_page(None, "Legacy state".into()).unwrap();
    v.delete_page(p.clone()).unwrap();
    v.purge_page(p).unwrap();
    let device = v.device_id();
    drop(v);
    let path = t.0.join(".vault/purge-state.json");
    let text = fs::read_to_string(&path).unwrap();
    let fieldless = text.replace(&format!(",\"device\":\"{device}\""), "");
    assert_ne!(fieldless, text);
    fs::write(&path, fieldless).unwrap();
    let v = t.open();
    let records = v.purge_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].device, None);
}

/// Three purgers delete and purge one page from their own histories, so their
/// tombstones are concurrent. Returns each purger's tombstone and device id.
fn concurrent_purgers(tag: &str) -> (Vec<PurgeRecord>, Vec<String>) {
    let origin = TempDir::new(&format!("{tag}_origin"));
    let editor = origin.open();
    let p = editor.create_page(None, tag.into()).unwrap();
    editor
        .apply(p.clone(), vec![ins(None, nb("body", "original"))])
        .unwrap();
    let (mut tombstones, mut devices) = (Vec::new(), Vec::new());
    for i in 0..3 {
        let dir = TempDir::new(&format!("{tag}_purger{i}"));
        let purger = dir.open();
        sync(&editor, &purger);
        purger.delete_page(p.clone()).unwrap();
        purger.purge_page(p.clone()).unwrap();
        tombstones.push(purger.purge_records().remove(0));
        devices.push(purger.device_id());
    }
    (tombstones, devices)
}

/// Imports `records` one call at a time into a fresh vault, then checks the
/// settled tombstone survives reopening and a full replay.
fn settle_in_order(tag: &str, records: &[PurgeRecord]) -> PurgeRecord {
    let dir = TempDir::new(tag);
    let v = dir.open();
    for record in records {
        v.import_purge_records(vec![record.clone()]).unwrap();
    }
    let settled = v.purge_records().remove(0);
    drop(v);
    let v = dir.open();
    assert_eq!(v.purge_records(), vec![settled.clone()], "reopened {tag}");
    v.import_purge_records(records.to_vec()).unwrap();
    assert_eq!(v.purge_records(), vec![settled.clone()], "replayed {tag}");
    settled
}

#[test]
fn concurrent_purgers_settle_on_the_greater_device_in_every_order() {
    let (tombstones, devices) = concurrent_purgers("concurrent_orders");
    let greatest = devices.iter().max().unwrap().clone();
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for (n, order) in orders.iter().enumerate() {
        let records: Vec<PurgeRecord> = order.iter().map(|&i| tombstones[i].clone()).collect();
        let settled = settle_in_order(&format!("concurrent_order_{n}"), &records);
        assert_eq!(
            settled.device.as_deref(),
            Some(greatest.as_str()),
            "order {order:?}"
        );
    }
}

#[test]
fn equal_coverage_from_different_purgers_settles_in_both_orders() {
    let (tombstones, devices) = concurrent_purgers("equal_coverage");
    let mine = tombstones[0].clone();
    let theirs = PurgeRecord {
        device: Some(devices[1].clone()),
        ..tombstones[0].clone()
    };
    let greater = devices[0].clone().max(devices[1].clone());
    for (n, records) in [vec![mine.clone(), theirs.clone()], vec![theirs, mine]]
        .iter()
        .enumerate()
    {
        let settled = settle_in_order(&format!("equal_coverage_{n}"), records);
        assert_eq!(settled.device.as_deref(), Some(greater.as_str()));
    }
}

#[test]
fn a_fieldless_concurrent_copy_yields_to_the_known_purger() {
    let (tombstones, devices) = concurrent_purgers("fieldless_concurrent");
    let known = tombstones[0].clone();
    let fieldless = PurgeRecord {
        device: None,
        ..tombstones[1].clone()
    };
    for (n, records) in [
        vec![known.clone(), fieldless.clone()],
        vec![fieldless, known],
    ]
    .iter()
    .enumerate()
    {
        let settled = settle_in_order(&format!("fieldless_concurrent_{n}"), records);
        assert_eq!(settled.device.as_deref(), Some(devices[0].as_str()));
    }
}

#[test]
fn id_only_records_with_known_devices_settle_in_both_orders() {
    let (tombstones, devices) = concurrent_purgers("id_only_orders");
    let page = tombstones[0].page.clone();
    let named = |device: &str| PurgeRecord {
        page: page.clone(),
        version: None,
        device: Some(device.into()),
    };
    let greater = devices[0].clone().max(devices[1].clone());
    for (n, records) in [
        vec![named(&devices[0]), named(&devices[1])],
        vec![named(&devices[1]), named(&devices[0])],
    ]
    .iter()
    .enumerate()
    {
        let settled = settle_in_order(&format!("id_only_{n}"), records);
        assert_eq!(settled.version, None);
        assert_eq!(settled.device.as_deref(), Some(greater.as_str()));
    }
}

#[test]
fn fieldless_replay_on_a_live_page_keeps_the_purging_side_named() {
    let (ta, tb) = (
        TempDir::new("replay_live_editor"),
        TempDir::new("replay_live_purger"),
    );
    let (editor, purger) = (ta.open(), tb.open());
    let (p, tombstones) = purge_against_offline_edit(&editor, &purger, "Live replay");
    let purged_by = purger.device_id();
    editor.import_purge_records(tombstones.clone()).unwrap();
    // The same purge again, fieldless, reaches the page through the live path.
    let fieldless: Vec<PurgeRecord> = tombstones
        .into_iter()
        .map(|record| PurgeRecord {
            device: None,
            ..record
        })
        .collect();
    editor.import_purge_records(fieldless).unwrap();
    let conflicts = editor.list_conflicts(Some(p)).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].sides[1].device, purged_by);
}

#[test]
fn stale_snapshot_is_skipped_but_new_delta_requires_full_history() {
    let (ta, tb) = (TempDir::new("delta_editor"), TempDir::new("delta_purger"));
    let (a, b) = (ta.open(), tb.open());
    let p = a.create_page(None, "Delta".into()).unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    sync(&a, &b);
    let base = a
        .page_versions()
        .unwrap()
        .into_iter()
        .find(|v| v.page == p)
        .unwrap()
        .version;
    let stale = a.export_updates(p.clone(), vec![]).unwrap();
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    assert_eq!(b.import_updates(p.clone(), stale).unwrap().skipped, 1);
    a.apply(p.clone(), vec![set_text("body", "new")]).unwrap();
    let delta = a.export_updates(p.clone(), base).unwrap();
    let error = b.import_updates(p.clone(), delta).unwrap_err();
    assert!(error.to_string().contains("full snapshot"));
    assert!(b.purged_pages().contains(&p));
    b.import_updates(p.clone(), a.export_updates(p.clone(), vec![]).unwrap())
        .unwrap();
    assert_eq!(texts(&b, &p), vec!["new"]);
}

#[test]
fn legacy_tombstone_restores_to_marked_trash_and_repurge_covers_copy() {
    for tombstone_first in [false, true] {
        let (ta, tb) = (TempDir::new("legacy_editor"), TempDir::new("legacy_purger"));
        let (a, b) = (ta.open(), tb.open());
        let p = a.create_page(None, "Legacy".into()).unwrap();
        a.apply(p.clone(), vec![ins(None, nb("body", "precious"))])
            .unwrap();
        let full = a.export_updates(p.clone(), vec![]).unwrap();
        let legacy = PurgeRecord {
            page: p.clone(),
            version: None,
            device: None,
        };
        if tombstone_first {
            a.import_purge_records(vec![legacy.clone()]).unwrap();
            push(&a, &b);
        } else {
            b.import_purge_records(vec![legacy.clone()]).unwrap();
            b.import_updates(p.clone(), full).unwrap();
            push(&b, &a);
        }
        for v in [&a, &b] {
            assert_eq!(texts(v, &p), vec!["precious"]);
            assert!(v.load_page(p.clone()).unwrap().in_trash);
            assert!(v.restored_after_purge_pages().unwrap().contains(&p));
            assert!(!v.purged_pages().contains(&p));
            v.import_purge_records(vec![legacy.clone()]).unwrap();
            assert_eq!(texts(v, &p), vec!["precious"]);
        }
        let stale = b.export_updates(p.clone(), vec![]).unwrap();
        b.purge_page(p.clone()).unwrap();
        assert!(b.purge_records()[0].version.is_some());
        assert_eq!(b.import_updates(p.clone(), stale).unwrap().skipped, 1);
        drop(a);
        drop(b);
        let a = ta.open();
        assert!(a.restored_after_purge_pages().unwrap().contains(&p));
        a.import_purge_records(vec![legacy]).unwrap();
        assert_eq!(texts(&a, &p), vec!["precious"]);
    }
}

#[test]
fn legacy_disk_ids_are_recoverable_but_old_api_is_delete_wins() {
    let (ta, tb) = (
        TempDir::new("legacy_disk_editor"),
        TempDir::new("legacy_disk"),
    );
    let a = ta.open();
    let p = a.create_page(None, "Legacy disk".into()).unwrap();
    let full = a.export_updates(p.clone(), vec![]).unwrap();
    {
        let b = tb.open();
        b.flush().unwrap();
    }
    fs::write(tb.0.join(".vault/purged"), format!("{p}\n")).unwrap();
    let b = tb.open();
    b.import_updates(p.clone(), full.clone()).unwrap();
    assert!(b.load_page(p.clone()).unwrap().in_trash);
    let old_dir = TempDir::new("old_api");
    let old = old_dir.open();
    old.import_purged(vec![p.clone()]).unwrap();
    assert_eq!(
        old.import_updates_batch_legacy(vec![PageUpdate {
            page: p,
            bytes: full
        }])
        .unwrap()
        .skipped,
        1
    );
}

#[test]
fn unknown_restore_replaces_only_covered_purge_branch_ops() {
    let (ta, tb, tc) = (
        TempDir::new("proof_editor"),
        TempDir::new("proof_purger"),
        TempDir::new("proof_hub"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Proof".into()).unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    push(&a, &b);
    push(&a, &c);
    b.apply(p.clone(), vec![BlockOp::Delete { id: "body".into() }])
        .unwrap();
    push(&b, &c); // Hub has already merged deletion-side history.
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    a.apply(p.clone(), vec![set_text("body", "saved")]).unwrap();
    a.import_purge_records(b.purge_records()).unwrap();
    let record = a.restore_records().remove(0);
    let snapshot = a.export_updates(p.clone(), vec![]).unwrap();
    assert!(c.needs_restore_snapshot(&record).unwrap());
    c.import_restored_updates(record.clone(), snapshot.clone())
        .unwrap();
    assert_eq!(texts(&c, &p), vec!["saved"]);
    c.apply(p.clone(), vec![set_text("body", "new local edit")])
        .unwrap();
    c.import_restored_updates(record, snapshot).unwrap();
    assert_eq!(texts(&c, &p), vec!["new local edit"]);
}

#[test]
fn unknown_restore_rejects_uncovered_independent_local_edits() {
    let (ta, tb, tc) = (
        TempDir::new("safe_editor"),
        TempDir::new("safe_purger"),
        TempDir::new("safe_hub"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Safe".into()).unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    push(&a, &b);
    push(&a, &c);
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    a.apply(p.clone(), vec![set_text("body", "editor")])
        .unwrap();
    a.import_purge_records(b.purge_records()).unwrap();
    c.apply(p.clone(), vec![set_text("body", "independent")])
        .unwrap();
    let record = a.restore_records().remove(0);
    let error = c
        .import_restored_updates(record, a.export_updates(p.clone(), vec![]).unwrap())
        .unwrap_err();
    assert!(error.to_string().contains("independent local edits"));
    assert_eq!(texts(&c, &p), vec!["independent"]);
}

#[test]
fn tombstone_after_merged_branches_recovers_editor_causal_state() {
    let (ta, tb, tc) = (
        TempDir::new("merged_editor"),
        TempDir::new("merged_purger"),
        TempDir::new("merged_hub"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Merged first".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(None, nb("kept", "precious")),
            ins(Some("kept"), nb("edited", "before")),
        ],
    )
    .unwrap();
    push(&a, &b);
    push(&a, &c);
    b.apply(p.clone(), vec![BlockOp::Delete { id: "kept".into() }])
        .unwrap();
    push(&b, &c);
    b.delete_page(p.clone()).unwrap();
    push(&b, &c);
    a.apply(p.clone(), vec![set_text("edited", "editor state")])
        .unwrap();
    push(&a, &c); // Page delete/edit already restores the entire editing side.
    assert_eq!(texts(&c, &p), vec!["precious", "editor state"]);
    b.purge_page(p.clone()).unwrap();
    c.import_purge_records(b.purge_records()).unwrap();
    assert_eq!(texts(&c, &p), vec!["precious", "editor state"]);
    assert!(!c.load_page(p.clone()).unwrap().in_trash);
    let record = c.restore_records().remove(0);
    let snapshot = c.export_updates(p.clone(), vec![]).unwrap();
    a.import_restored_updates(record, snapshot).unwrap();
    assert_eq!(texts(&a, &p), vec!["precious", "editor state"]);
}

#[test]
fn conflicts_do_not_make_purge_deletions_part_of_editor_snapshot() {
    let (ta, tb, tc) = (
        TempDir::new("conflict_editor"),
        TempDir::new("conflict_purger"),
        TempDir::new("conflict_hub"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Conflict before purge".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(None, nb("kept", "precious")),
            ins(Some("kept"), nb("edited", "before")),
        ],
    )
    .unwrap();
    push(&a, &b);
    push(&a, &c);
    b.apply(
        p.clone(),
        vec![
            BlockOp::Delete { id: "kept".into() },
            BlockOp::Delete {
                id: "edited".into(),
            },
        ],
    )
    .unwrap();
    push(&b, &c);
    b.delete_page(p.clone()).unwrap();
    push(&b, &c);
    a.apply(p.clone(), vec![set_text("edited", "editor state")])
        .unwrap();
    push(&a, &c);
    assert!(!c.list_conflicts(Some(p.clone())).unwrap().is_empty());
    b.purge_page(p.clone()).unwrap();
    c.import_purge_records(b.purge_records()).unwrap();
    assert_eq!(texts(&c, &p), vec!["precious", "editor state"]);
    assert!(!c.load_page(p).unwrap().in_trash);
}

#[test]
fn restored_purger_uses_fresh_stable_peer_for_future_edits() {
    let (ta, tb) = (TempDir::new("fresh_editor"), TempDir::new("fresh_purger"));
    let (a, b) = (ta.open(), tb.open());
    let p = a.create_page(None, "Fresh peer".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(None, nb("kept", "precious")),
            ins(Some("kept"), nb("edited", "before")),
        ],
    )
    .unwrap();
    push(&a, &b);
    b.apply(p.clone(), vec![BlockOp::Delete { id: "kept".into() }])
        .unwrap();
    let old = LoroDoc::new();
    old.import(&b.export_updates(p.clone(), vec![]).unwrap())
        .unwrap();
    let old_vv = old.oplog_vv();
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    a.apply(p.clone(), vec![set_text("edited", "saved")])
        .unwrap();
    sync(&a, &b);
    b.apply(p.clone(), vec![set_text("edited", "new on purger")])
        .unwrap();
    let new = LoroDoc::new();
    new.import(&b.export_updates(p.clone(), vec![]).unwrap())
        .unwrap();
    assert!(new
        .oplog_vv()
        .iter()
        .any(|(peer, &end)| old_vv.get(peer).is_none() && end > 0));
    push(&b, &a);
    assert_eq!(texts(&a, &p), vec!["precious", "new on purger"]);
    let before_reopen = new.oplog_vv();
    b.flush().unwrap();
    drop(b);
    let b = tb.open();
    b.apply(p.clone(), vec![set_text("edited", "after reopen")])
        .unwrap();
    let reopened = LoroDoc::new();
    reopened
        .import(&b.export_updates(p.clone(), vec![]).unwrap())
        .unwrap();
    assert_eq!(reopened.oplog_vv().len(), before_reopen.len());
    push(&b, &a);
    assert_eq!(texts(&a, &p), vec!["precious", "after reopen"]);
}

#[test]
fn restore_rejects_uncovered_edits_in_another_vault_instance() {
    let (ta, tb, tc) = (
        TempDir::new("disk_editor"),
        TempDir::new("disk_purger"),
        TempDir::new("disk_receiver"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Disk safety".into()).unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    push(&a, &b);
    push(&a, &c);
    c.flush().unwrap();
    c.load_page(p.clone()).unwrap(); // Cache the version before the other writer.
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    a.apply(p.clone(), vec![set_text("body", "editor")])
        .unwrap();
    a.import_purge_records(b.purge_records()).unwrap();
    let other = tc.open();
    other
        .apply(p.clone(), vec![set_text("body", "other instance")])
        .unwrap();
    other.flush().unwrap();
    let error = c
        .import_restored_updates(
            a.restore_records().remove(0),
            a.export_updates(p.clone(), vec![]).unwrap(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("independent disk edits"));
    assert_eq!(texts(&other, &p), vec!["other instance"]);
    drop(other);
    drop(c);
    let c = tc.open();
    assert_eq!(texts(&c, &p), vec!["other instance"]);
}

#[test]
fn replacement_journal_recovers_snapshot_without_replaying_purge_logs() {
    let (ta, tb, tc) = (
        TempDir::new("journal_editor"),
        TempDir::new("journal_purger"),
        TempDir::new("journal_receiver"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Journal".into()).unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    push(&a, &b);
    push(&a, &c);
    let base = a
        .page_versions()
        .unwrap()
        .into_iter()
        .find(|v| v.page == p)
        .unwrap()
        .version;
    b.apply(p.clone(), vec![BlockOp::Delete { id: "body".into() }])
        .unwrap();
    b.delete_page(p.clone()).unwrap();
    let purge_delta = b.export_updates(p.clone(), base).unwrap();
    b.purge_page(p.clone()).unwrap();
    a.apply(p.clone(), vec![set_text("body", "saved")]).unwrap();
    a.import_purge_records(b.purge_records()).unwrap();
    let record = a.restore_records().remove(0);
    let snapshot = a.export_updates(p.clone(), vec![]).unwrap();
    c.flush().unwrap();
    drop(c);
    // Simulate interruption after the authoritative snapshot rename but
    // before the incompatible old log was removed.
    fs::write(tc.0.join(format!(".vault/docs/{p}.loro")), &snapshot).unwrap();
    let mut frame = (purge_delta.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&purge_delta);
    fs::write(tc.0.join(format!(".vault/logs/{p}.log")), frame).unwrap();
    let journals = tc.0.join(".vault/restores");
    fs::create_dir_all(&journals).unwrap();
    let journal_path = journals.join(format!("{p}.json"));
    fs::write(
        &journal_path,
        serde_json::to_vec(&serde_json::json!({
            "record": record, "bytes": snapshot, "installed": false,
        }))
        .unwrap(),
    )
    .unwrap();
    let c = tc.open();
    assert_eq!(texts(&c, &p), vec!["saved"]);
    assert!(!c.load_page(p.clone()).unwrap().in_trash);
    assert!(!journal_path.exists());
    let current_record = c.restore_records().remove(0);
    let old_snapshot = c.export_updates(p.clone(), vec![]).unwrap();
    c.apply(p.clone(), vec![set_text("body", "later user edit")])
        .unwrap();
    c.flush().unwrap();
    drop(c);
    // A journal left after completed installation must preserve later edits.
    fs::write(
        &journal_path,
        serde_json::to_vec(&serde_json::json!({
            "record": current_record, "bytes": old_snapshot, "installed": true,
        }))
        .unwrap(),
    )
    .unwrap();
    let c = tc.open();
    assert_eq!(texts(&c, &p), vec!["later user edit"]);
    assert!(!journal_path.exists());
}

#[test]
fn derived_only_unseen_conflicts_do_not_restore_stale_user_state() {
    let (ta, tb, tc) = (
        TempDir::new("derived_a"),
        TempDir::new("derived_b"),
        TempDir::new("derived_c"),
    );
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let p = a.create_page(None, "Derived only".into()).unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    push(&a, &b);
    push(&a, &c);
    a.apply(p.clone(), vec![set_text("body", "a edit")])
        .unwrap();
    b.apply(p.clone(), vec![set_text("body", "b edit")])
        .unwrap();
    push(&a, &c);
    push(&b, &c);
    assert!(!c.list_conflicts(Some(p.clone())).unwrap().is_empty());
    push(&a, &b); // Purger knows all user ops, but not c's conflict ops.
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    c.import_purge_records(b.purge_records()).unwrap();
    assert!(matches!(
        c.load_page(p.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert!(c.purged_pages().contains(&p));
}

#[test]
fn independent_legacy_recovery_metadata_does_not_defeat_repurge() {
    let (ta, tb) = (
        TempDir::new("double_legacy_a"),
        TempDir::new("double_legacy_b"),
    );
    let (a, b) = (ta.open(), tb.open());
    let p = a
        .create_page(None, "Double legacy recovery".into())
        .unwrap();
    a.apply(p.clone(), vec![ins(None, nb("body", "original"))])
        .unwrap();
    push(&a, &b);
    let legacy = PurgeRecord {
        page: p.clone(),
        version: None,
        device: None,
    };
    a.import_purge_records(vec![legacy.clone()]).unwrap();
    b.import_purge_records(vec![legacy]).unwrap();
    let stale = b.export_updates(p.clone(), vec![]).unwrap();
    let stale_proof = b.restore_records().remove(0);
    a.purge_page(p.clone()).unwrap();
    let repurge = a.purge_records();
    assert_eq!(
        a.import_updates(p.clone(), stale.clone()).unwrap().skipped,
        1
    );
    assert_eq!(
        a.import_restored_updates(stale_proof, stale)
            .unwrap()
            .skipped,
        1
    );
    b.import_purge_records(repurge).unwrap();
    assert!(matches!(
        b.load_page(p.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert!(b.purged_pages().contains(&p));
}

#[test]
fn cached_second_instance_edit_after_purge_returns_error_without_orphan_log() {
    let directory = TempDir::new("purge_append_guard");
    let owner = directory.open();
    let p = owner.create_page(None, "Append guard".into()).unwrap();
    owner
        .apply(p.clone(), vec![ins(None, nb("body", "before"))])
        .unwrap();
    owner.flush().unwrap();
    let other = directory.open();
    other.load_page(p.clone()).unwrap();
    owner.delete_page(p.clone()).unwrap();
    owner.purge_page(p.clone()).unwrap();
    let result = other.apply(p.clone(), vec![set_text("body", "must not succeed")]);
    assert!(matches!(result, Err(VaultError::NotFound(_))));
    assert!(!directory.0.join(format!(".vault/logs/{p}.log")).exists());
}

/// An editor and a purger. The purger deleted and purged the page; the
/// editor's offline edit was restored live on both vaults, whole.
fn restored_page(ta: &TempDir, tb: &TempDir) -> (Vault, Vault, PageId) {
    let (a, b) = (ta.open(), tb.open());
    let p = a.create_page(None, "Restored".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(None, nb("kept", "original")),
            ins(Some("kept"), nb("edited", "before")),
        ],
    )
    .unwrap();
    sync(&a, &b);
    b.apply(p.clone(), vec![BlockOp::Delete { id: "kept".into() }])
        .unwrap();
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    a.apply(p.clone(), vec![set_text("edited", "offline edit")])
        .unwrap();
    push(&b, &a);
    push(&a, &b);
    assert_eq!(texts(&a, &p), vec!["original", "offline edit"]);
    assert_eq!(texts(&b, &p), vec!["original", "offline edit"]);
    (a, b, p)
}

#[test]
fn id_only_purge_never_removes_a_restored_page() {
    let (ta, tb) = (
        TempDir::new("ignored_editor"),
        TempDir::new("ignored_purger"),
    );
    let (a, b, p) = restored_page(&ta, &tb);
    let legacy = PurgeRecord {
        page: p.clone(),
        version: None,
        device: None,
    };
    for v in [&a, &b] {
        // Both the modern record path and the v1 ID path ignore the replay:
        // nothing is removed, and no tombstone is stored.
        assert_eq!(v.import_purge_records(vec![legacy.clone()]).unwrap(), 0);
        assert_eq!(v.import_purged(vec![p.clone()]).unwrap(), 0);
        assert_eq!(texts(v, &p), vec!["original", "offline edit"]);
        assert!(!v.load_page(p.clone()).unwrap().in_trash);
        assert!(!v.purged_pages().contains(&p));
        assert!(v.restored_after_purge_pages().unwrap().is_empty());
    }
    // The restoration history is durable: the replays still do nothing after
    // a restart.
    drop(a);
    drop(b);
    let a = ta.open();
    assert_eq!(a.import_purge_records(vec![legacy]).unwrap(), 0);
    assert_eq!(a.import_purged(vec![p.clone()]).unwrap(), 0);
    assert_eq!(texts(&a, &p), vec!["original", "offline edit"]);
    assert!(!a.purged_pages().contains(&p));
}

#[test]
fn covering_repurge_after_restore_still_removes_the_page() {
    let (ta, tb) = (
        TempDir::new("repurge_editor"),
        TempDir::new("repurge_purger"),
    );
    let (a, b, p) = restored_page(&ta, &tb);
    b.delete_page(p.clone()).unwrap();
    b.purge_page(p.clone()).unwrap();
    let repurge = b.purge_records();
    assert!(repurge[0].version.is_some());
    assert_eq!(a.import_purge_records(repurge).unwrap(), 1);
    assert!(matches!(
        a.load_page(p.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(a.purged_pages(), vec![p.clone()]);
    // An ID-only replay of the old tombstone changes nothing more.
    assert_eq!(a.import_purged(vec![p.clone()]).unwrap(), 0);
    assert_eq!(a.purged_pages(), vec![p]);
}

#[test]
fn unedited_legacy_copy_returns_to_marked_trash_in_both_arrival_orders() {
    for tombstone_first in [false, true] {
        let (ta, tb) = (
            TempDir::new("unedited_editor"),
            TempDir::new("unedited_legacy"),
        );
        let (a, b) = (ta.open(), tb.open());
        let p = a.create_page(None, "Unedited".into()).unwrap();
        a.apply(p.clone(), vec![ins(None, nb("body", "untouched"))])
            .unwrap();
        let full = a.export_updates(p.clone(), vec![]).unwrap();
        let legacy = PurgeRecord {
            page: p.clone(),
            version: None,
            device: None,
        };
        if tombstone_first {
            b.import_purge_records(vec![legacy.clone()]).unwrap();
            b.import_updates(p.clone(), full).unwrap();
        } else {
            b.import_updates(p.clone(), full).unwrap();
            b.import_purge_records(vec![legacy.clone()]).unwrap();
        }
        // The copy is kept in the trash, marked, unchanged; the tombstone is
        // cleared. Either order ends in the same state.
        assert_eq!(texts(&b, &p), vec!["untouched"]);
        assert!(b.load_page(p.clone()).unwrap().in_trash);
        assert_eq!(b.restored_after_purge_pages().unwrap(), vec![p.clone()]);
        assert!(b.purged_pages().is_empty());
        // Restarting keeps the marker, and the replayed tombstone changes nothing.
        drop(b);
        let b = tb.open();
        assert!(b.load_page(p.clone()).unwrap().in_trash);
        assert_eq!(b.restored_after_purge_pages().unwrap(), vec![p.clone()]);
        assert_eq!(b.import_purge_records(vec![legacy]).unwrap(), 0);
        assert_eq!(texts(&b, &p), vec!["untouched"]);
        assert!(b.purged_pages().is_empty());
    }
}
