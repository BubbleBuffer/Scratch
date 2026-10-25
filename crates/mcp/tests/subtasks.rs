//! A run's todo list mirrored into the task's page: creation (the source
//! todo), the additive migration, reconciliation, user blocks, deletion
//! suppression, retries and stale sessions, and the placeholder's result.

use std::{fs, path::PathBuf};

use vault_core::*;
use vault_mcp::{runners, tasks};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_subtasks_{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn insert(
    v: &Vault,
    page: &str,
    parent: Option<&str>,
    after: Option<&str>,
    kind: BlockKind,
    text: &str,
    checked: bool,
) -> String {
    let id = new_id();
    v.apply(
        page.to_string(),
        vec![BlockOp::Insert {
            parent: parent.map(String::from),
            after: after.map(String::from),
            block: NewBlock {
                id: id.clone(),
                kind,
                text: text.into(),
                marks: vec![],
                checked,
            },
        }],
    )
    .unwrap();
    id
}

fn find_in<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|b| {
        if b.id == id {
            Some(b)
        } else {
            find_in(&b.children, id)
        }
    })
}

fn block(v: &Vault, page: &str, id: &str) -> Block {
    let p = v.load_page(page.to_string()).unwrap();
    find_in(&p.blocks, id).unwrap().clone()
}

fn kids(v: &Vault, page: &str, id: &str) -> Vec<Block> {
    block(v, page, id).children
}

fn shape(blocks: &[Block]) -> Vec<(String, bool)> {
    blocks.iter().map(|b| (b.text.clone(), b.checked)).collect()
}

fn ids(blocks: &[Block]) -> Vec<String> {
    blocks.iter().map(|b| b.id.clone()).collect()
}

fn want(items: &[(&str, bool)]) -> Vec<(String, bool)> {
    items.iter().map(|(t, c)| (t.to_string(), *c)).collect()
}

fn list(items: &[(&str, &str)]) -> Vec<tasks::SubtaskItem> {
    items
        .iter()
        .map(|(text, status)| tasks::SubtaskItem {
            text: text.to_string(),
            status: status.to_string(),
        })
        .collect()
}

/// A block and its children as one string, so a whole subtree compares equal
/// only when nothing in it changed.
fn tree(b: &Block) -> String {
    let parts: Vec<String> = b.children.iter().map(tree).collect();
    format!("{}|{}|{}[{}]", b.id, b.text, b.checked, parts.join(","))
}

fn snapshot(v: &Vault, page: &str) -> String {
    let p = v.load_page(page.to_string()).unwrap();
    p.blocks.iter().map(tree).collect::<Vec<_>>().join(";")
}

fn claim(v: &Vault, task: &str) {
    let claimed = tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
    assert_eq!(claimed.id, task);
    assert_eq!(claimed.status, tasks::RUNNING);
}

/// A page: an intro, the todo "Pack", the outro. A running task is asked
/// after the todo, so its placeholder comes right after it. Returns the page,
/// the todo and the task.
fn start(v: &Vault) -> (String, String, String) {
    let page = v.create_page(None, "Trip".into()).unwrap();
    let intro = insert(v, &page, None, None, BlockKind::Paragraph, "Intro", false);
    let todo = insert(v, &page, None, Some(&intro), BlockKind::Todo, "Pack", false);
    insert(
        v,
        &page,
        None,
        Some(&todo),
        BlockKind::Paragraph,
        "Outro",
        false,
    );
    let task = tasks::create(v, "Plan the trip", &page, Some(&todo), "").unwrap();
    claim(v, &task.id);
    (page, todo, task.id)
}

/// A page with an intro, and a running task asked at its end (no source
/// todo, so the subtasks nest under the placeholder).
fn start_without_source(v: &Vault) -> (String, String) {
    let page = v.create_page(None, "Trip".into()).unwrap();
    insert(v, &page, None, None, BlockKind::Paragraph, "Intro", false);
    let task = tasks::create(v, "Plan the trip", &page, None, "").unwrap();
    assert_eq!(task.source_block, None);
    claim(v, &task.id);
    (page, task.id)
}

#[test]
fn an_older_tasks_database_gains_the_subtask_properties_and_keeps_its_own() {
    let t = Temp::new();
    let v = t.open();
    let old: Vec<PropDef> = tasks::schema()
        .into_iter()
        .filter(|d| d.key.as_str() != "source_block" && d.key.as_str() != "subtask_blocks")
        .collect();
    let coll = v.create_page(None, tasks::TITLE.into()).unwrap();
    v.make_collection(coll.clone()).unwrap();
    v.set_schema(coll.clone(), old).unwrap();
    v.set_property(
        coll.clone(),
        tasks::MARKER.into(),
        PropValue::Checkbox(true),
    )
    .unwrap();
    let row = v.create_page(Some(coll.clone()), "Old row".into()).unwrap();
    v.set_property(row.clone(), "prompt".into(), PropValue::Text("Keep".into()))
        .unwrap();
    let before = v.load_page(row.clone()).unwrap();

    tasks::migrate_schema(&v, &coll).unwrap();
    let migrated = v.get_schema(coll.clone()).unwrap();
    for key in ["source_block", "subtask_blocks"] {
        assert!(
            migrated
                .iter()
                .any(|d| d.key == key && d.kind == PropKind::Text),
            "{key}"
        );
    }
    assert_eq!(v.load_page(row).unwrap(), before);

    // Running the migration again changes nothing.
    tasks::migrate_schema(&v, &coll).unwrap();
    assert_eq!(v.get_schema(coll.clone()).unwrap(), migrated);

    // A task asked after a todo on the migrated database gets its source.
    let page = v.create_page(None, "Plan".into()).unwrap();
    let todo = insert(&v, &page, None, None, BlockKind::Todo, "Pack", false);
    let task = tasks::create(&v, "Plan it", &page, Some(&todo), "").unwrap();
    assert_eq!(task.source_block.as_deref(), Some(todo.as_str()));
}

#[test]
fn a_task_asked_after_a_todo_takes_it_as_its_source() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Plan".into()).unwrap();
    let todo = insert(&v, &page, None, None, BlockKind::Todo, "Pack", false);
    let note = insert(
        &v,
        &page,
        None,
        Some(&todo),
        BlockKind::Paragraph,
        "Note",
        false,
    );

    let asked = tasks::create(&v, "Plan it", &page, Some(&todo), "").unwrap();
    assert_eq!(asked.source_block.as_deref(), Some(todo.as_str()));
    // The placeholder goes right after the todo, as it did before.
    let p = v.load_page(page.clone()).unwrap();
    assert_eq!(
        ids(&p.blocks),
        vec![todo.clone(), asked.block.clone().unwrap(), note.clone()]
    );
    assert_eq!(p.blocks[1].kind, BlockKind::Callout);

    // Asked after a paragraph, there is no source todo.
    let plain = tasks::create(&v, "Plan it", &page, Some(&note), "").unwrap();
    assert_eq!(plain.source_block, None);

    // The app's assignment path infers the same source.
    let assigned = runners::assign(&v, "Plan it", &page, Some(&todo), "hub", None).unwrap();
    assert_eq!(assigned.source_block.as_deref(), Some(todo.as_str()));
}

#[test]
fn the_list_nests_under_the_source_todo_and_the_parent_stays_as_it_was() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    let before = block(&v, &page, &todo);

    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("Tent", "pending"), ("Stove", "done")]),
    )
    .unwrap();

    assert_eq!(
        shape(&kids(&v, &page, &todo)),
        want(&[("Tent", false), ("Stove", true)])
    );
    let after = block(&v, &page, &todo);
    assert_eq!(after.text, before.text);
    assert_eq!(after.checked, before.checked);
    assert_eq!(after.kind, before.kind);
    // The page's own blocks and the placeholder are where they were.
    let p = v.load_page(page.clone()).unwrap();
    let placeholder = tasks::get(&v, &task).unwrap().block.unwrap();
    assert_eq!(p.blocks.len(), 4);
    assert_eq!(p.blocks[2].id, placeholder);
    assert_eq!(p.blocks[3].text, "Outro");
    assert_eq!(tasks::get(&v, &task).unwrap().status, tasks::RUNNING);
}

#[test]
fn without_a_source_todo_the_list_nests_under_the_placeholder() {
    let t = Temp::new();
    let v = t.open();
    let (page, task) = start_without_source(&v);
    let placeholder = tasks::get(&v, &task).unwrap().block.unwrap();

    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("Tent", "pending"), ("Stove", "done")]),
    )
    .unwrap();

    assert_eq!(
        shape(&kids(&v, &page, &placeholder)),
        want(&[("Tent", false), ("Stove", true)])
    );
    // The placeholder is still the callout it was.
    assert_eq!(block(&v, &page, &placeholder).kind, BlockKind::Callout);
}

#[test]
fn the_list_edits_reorders_and_removes_only_the_runs_own_todos() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending"), ("C", "pending")]),
    )
    .unwrap();
    let first = ids(&kids(&v, &page, &todo));
    let (a, b, c) = (first[0].clone(), first[1].clone(), first[2].clone());

    // C and A keep their todos, B's todo becomes X (the leftover pair), and
    // the list order is kept.
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("C", "pending"), ("A", "pending"), ("X", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(
        shape(&now),
        want(&[("C", false), ("A", false), ("X", false)])
    );
    assert_eq!(ids(&now), vec![c.clone(), a.clone(), b.clone()]);

    // Items that left the list lose their todos.
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("C", "pending")])).unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(shape(&now), want(&[("C", false)]));
    assert_eq!(ids(&now), vec![c]);
}

#[test]
fn a_todo_with_the_users_children_is_left_whole_and_never_moved_or_deleted() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let first = ids(&kids(&v, &page, &todo));
    let (a, b) = (first[0].clone(), first[1].clone());

    // The user writes under A, and puts a block of their own between A and B.
    let note = insert(
        &v,
        &page,
        Some(&a),
        None,
        BlockKind::Paragraph,
        "My note",
        false,
    );
    let loose = insert(
        &v,
        &page,
        Some(&todo),
        Some(&a),
        BlockKind::Paragraph,
        "Loose",
        false,
    );
    let subtree = tree(&block(&v, &page, &a));
    let loose_before = tree(&block(&v, &page, &loose));

    // A leaves the list. Its todo has the user's child, so the whole subtree
    // stays exactly as it was, in place, and is no longer the attempt's.
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("B", "pending")])).unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(ids(&now), vec![a.clone(), loose.clone(), b.clone()]);
    assert_eq!(tree(&block(&v, &page, &a)), subtree);
    assert_eq!(ids(&block(&v, &page, &a).children), vec![note.clone()]);
    assert_eq!(tree(&block(&v, &page, &loose)), loose_before);

    // Only B is the attempt's now. A new item goes at the end, and the
    // retained subtree and the user's sibling still do not move.
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("B", "pending"), ("D", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(
        shape(&now),
        want(&[("A", false), ("Loose", false), ("B", false), ("D", false)])
    );
    assert_eq!(tree(&block(&v, &page, &a)), subtree);
    assert_eq!(tree(&block(&v, &page, &loose)), loose_before);
}

#[test]
fn a_retry_leaves_a_todo_with_the_users_children_where_it_is() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let first = ids(&kids(&v, &page, &todo));
    let (a, b) = (first[0].clone(), first[1].clone());
    insert(
        &v,
        &page,
        Some(&a),
        None,
        BlockKind::Paragraph,
        "My note",
        false,
    );
    let subtree = tree(&block(&v, &page, &a));

    // The retry cleans the unchecked B. A has the user's child, so it stays
    // exactly as it is and is not deleted or moved.
    tasks::finish(&v, &task, Err("boom".into())).unwrap();
    tasks::retry(&v, &task).unwrap();
    claim(&v, &task);
    tasks::begin_subtasks(&v, &task, "s2").unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(ids(&now), vec![a.clone()]);
    assert_eq!(tree(&block(&v, &page, &a)), subtree);
    assert!(find_in(&v.load_page(page.clone()).unwrap().blocks, &b).is_none());
}

#[test]
fn a_retry_starts_a_fresh_list_so_an_item_the_user_deleted_may_come_back() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let a = ids(&kids(&v, &page, &todo))[0].clone();
    v.apply(page.clone(), vec![BlockOp::Delete { id: a.clone() }])
        .unwrap();

    // Within the attempt, A stays gone while it is on the list.
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    assert_eq!(shape(&kids(&v, &page, &todo)), want(&[("B", false)]));

    // A retry starts a fresh list, so A is mirrored again.
    tasks::finish(&v, &task, Err("boom".into())).unwrap();
    tasks::retry(&v, &task).unwrap();
    claim(&v, &task);
    tasks::begin_subtasks(&v, &task, "s2").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s2",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(shape(&now), want(&[("A", false), ("B", false)]));
    assert!(ids(&now).iter().all(|id| *id != a));
}

#[test]
fn item_text_is_kept_exactly_and_in_progress_is_not_checked() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[
            ("  Tent  ", "in_progress"),
            ("Stove", "done"),
            ("   ", "pending"),
        ]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(shape(&now), want(&[("  Tent  ", false), ("Stove", true)]));

    // The same exact text matches the same todo, now done.
    let first = ids(&now);
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("  Tent  ", "done")])).unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(shape(&now), want(&[("  Tent  ", true)]));
    assert_eq!(ids(&now), vec![first[0].clone()]);
}

#[test]
fn damaged_tracking_is_an_error_and_leaves_the_page_as_it_was() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).unwrap();
    v.set_property(
        task.clone(),
        "subtask_blocks".into(),
        PropValue::Text("{not json".into()),
    )
    .unwrap();
    let before = snapshot(&v, &page);

    assert!(tasks::begin_subtasks(&v, &task, "s2").is_err());
    assert!(tasks::begin_subtasks(&v, &task, "s1").is_err());
    assert!(tasks::mirror_subtasks(&v, &task, "s1", &list(&[("B", "pending")])).is_err());
    assert_eq!(snapshot(&v, &page), before);
    assert_eq!(shape(&kids(&v, &page, &todo)), want(&[("A", false)]));
}

#[test]
fn a_task_whose_row_names_another_session_refuses_begin_and_mirror() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    v.set_property(
        task.clone(),
        "session".into(),
        PropValue::Text("owner".into()),
    )
    .unwrap();

    assert!(tasks::begin_subtasks(&v, &task, "s1").is_err());
    assert!(tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).is_err());
    assert!(kids(&v, &page, &todo).is_empty());

    // The row's own session may begin and mirror; any other is still refused.
    tasks::begin_subtasks(&v, &task, "owner").unwrap();
    tasks::mirror_subtasks(&v, &task, "owner", &list(&[("A", "pending")])).unwrap();
    assert_eq!(shape(&kids(&v, &page, &todo)), want(&[("A", false)]));
    assert!(tasks::mirror_subtasks(&v, &task, "s1", &list(&[("B", "pending")])).is_err());
}

#[test]
fn a_mirror_the_user_deleted_stays_gone_while_its_item_is_on_the_list() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending"), ("C", "pending")]),
    )
    .unwrap();
    let first = ids(&kids(&v, &page, &todo));
    let (a, b, c) = (first[0].clone(), first[1].clone(), first[2].clone());
    v.apply(page.clone(), vec![BlockOp::Delete { id: b.clone() }])
        .unwrap();

    // Re-sent, with B's status changed, and reordered: B is not back.
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "done"), ("C", "pending")]),
    )
    .unwrap();
    assert_eq!(ids(&kids(&v, &page, &todo)), vec![a.clone(), c.clone()]);
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("C", "pending"), ("B", "pending"), ("A", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(ids(&now), vec![c.clone(), a.clone()]);
    assert_eq!(shape(&now), want(&[("C", false), ("A", false)]));

    // B leaves the list: its suppression ends with it. Listed again, it comes
    // back as a new todo.
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).unwrap();
    assert_eq!(ids(&kids(&v, &page, &todo)), vec![a.clone()]);
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(shape(&now), want(&[("A", false), ("B", false)]));
    assert_ne!(now[1].id, b);
}

#[test]
fn a_mirror_the_user_moved_away_is_left_where_it_is() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let first = ids(&kids(&v, &page, &todo));
    let (a, b) = (first[0].clone(), first[1].clone());
    v.apply(
        page.clone(),
        vec![BlockOp::Move {
            id: a.clone(),
            parent: None,
            after: Some(todo.clone()),
        }],
    )
    .unwrap();

    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(ids(&now), vec![b]);
    assert_eq!(shape(&now), want(&[("B", false)]));
    // A stays where the user put it, and is not pulled back.
    let p = v.load_page(page.clone()).unwrap();
    assert!(p.blocks.iter().any(|x| x.id == a));
}

#[test]
fn duplicate_items_match_in_order_and_the_first_mirror_stays() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("A", "pending")]),
    )
    .unwrap();
    let first = ids(&kids(&v, &page, &todo));
    assert_eq!(first.len(), 2);

    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).unwrap();
    assert_eq!(ids(&kids(&v, &page, &todo)), vec![first[0].clone()]);
}

#[test]
fn a_retry_replaces_an_unchecked_attempt_with_a_fresh_one_at_the_end() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let old = ids(&kids(&v, &page, &todo));

    tasks::finish(&v, &task, Err("boom".into())).unwrap();
    tasks::retry(&v, &task).unwrap();
    claim(&v, &task);

    tasks::begin_subtasks(&v, &task, "s2").unwrap();
    assert!(kids(&v, &page, &todo).is_empty());
    tasks::mirror_subtasks(
        &v,
        &task,
        "s2",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(shape(&now), want(&[("A", false), ("B", false)]));
    assert!(ids(&now).iter().all(|id| !old.contains(id)));
}

#[test]
fn a_retry_leaves_an_attempt_with_a_checked_todo_as_the_users() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "done"), ("B", "pending")])).unwrap();
    let old = ids(&kids(&v, &page, &todo));

    tasks::finish(&v, &task, Err("boom".into())).unwrap();
    tasks::retry(&v, &task).unwrap();
    claim(&v, &task);

    tasks::begin_subtasks(&v, &task, "s2").unwrap();
    let kept = kids(&v, &page, &todo);
    assert_eq!(ids(&kept), old);
    assert_eq!(shape(&kept), want(&[("A", true), ("B", false)]));

    // The new attempt's todos go at the end.
    tasks::mirror_subtasks(&v, &task, "s2", &list(&[("A", "done"), ("B", "pending")])).unwrap();
    let now = kids(&v, &page, &todo);
    assert_eq!(now.len(), 4);
    assert_eq!(ids(&now)[..2], old[..]);
    assert_eq!(shape(&now[2..]), want(&[("A", true), ("B", false)]));
}

#[test]
fn only_the_session_that_began_the_attempt_may_write_to_it() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);

    // Nothing begun yet.
    assert!(tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).is_err());

    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).unwrap();
    let before = ids(&kids(&v, &page, &todo));
    assert!(tasks::mirror_subtasks(&v, &task, "s0", &list(&[("X", "pending")])).is_err());
    assert_eq!(ids(&kids(&v, &page, &todo)), before);

    // A later session takes over; the earlier one can no longer write, and
    // the later one's todos are untouched by its calls.
    tasks::begin_subtasks(&v, &task, "s2").unwrap();
    tasks::mirror_subtasks(&v, &task, "s2", &list(&[("B", "pending")])).unwrap();
    let mine = ids(&kids(&v, &page, &todo));
    assert!(tasks::mirror_subtasks(&v, &task, "s1", &list(&[("Y", "pending")])).is_err());
    assert_eq!(ids(&kids(&v, &page, &todo)), mine);
    assert_eq!(shape(&kids(&v, &page, &todo)), want(&[("B", false)]));
}

#[test]
fn beginning_the_same_session_twice_changes_nothing() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    let before = ids(&kids(&v, &page, &todo));

    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    assert_eq!(ids(&kids(&v, &page, &todo)), before);
    tasks::mirror_subtasks(
        &v,
        &task,
        "s1",
        &list(&[("A", "pending"), ("B", "pending")]),
    )
    .unwrap();
    assert_eq!(ids(&kids(&v, &page, &todo)), before);
}

#[test]
fn nothing_is_written_once_the_task_has_ended() {
    let t = Temp::new();
    let v = t.open();
    let (page, todo, task) = start(&v);
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "pending")])).unwrap();
    tasks::finish(&v, &task, Ok("Result".into())).unwrap();
    let before = ids(&kids(&v, &page, &todo));

    assert!(tasks::mirror_subtasks(&v, &task, "s1", &list(&[("B", "pending")])).is_err());
    assert!(tasks::begin_subtasks(&v, &task, "s2").is_err());
    assert_eq!(ids(&kids(&v, &page, &todo)), before);
}

#[test]
fn a_placeholder_with_subtasks_stays_as_a_done_label_with_the_result_after_it() {
    let t = Temp::new();
    let v = t.open();
    let (page, task) = start_without_source(&v);
    let placeholder = tasks::get(&v, &task).unwrap().block.unwrap();
    tasks::begin_subtasks(&v, &task, "s1").unwrap();
    tasks::mirror_subtasks(&v, &task, "s1", &list(&[("A", "done"), ("B", "pending")])).unwrap();

    tasks::finish(&v, &task, Ok("Here is the plan.".into())).unwrap();

    let p = v.load_page(page.clone()).unwrap();
    let at = p.blocks.iter().position(|b| b.id == placeholder).unwrap();
    let callout = &p.blocks[at];
    assert_eq!(callout.kind, BlockKind::Callout);
    assert_eq!(callout.text, "AI task done: Plan the trip");
    assert_eq!(shape(&callout.children), want(&[("A", true), ("B", false)]));
    assert_eq!(p.blocks[at + 1].text, "Here is the plan.");
}

#[test]
fn a_placeholder_without_subtasks_is_replaced_by_the_result() {
    let t = Temp::new();
    let v = t.open();
    let (page, task) = start_without_source(&v);
    let placeholder = tasks::get(&v, &task).unwrap().block.unwrap();

    tasks::finish(&v, &task, Ok("Done.".into())).unwrap();

    let p = v.load_page(page.clone()).unwrap();
    assert!(find_in(&p.blocks, &placeholder).is_none());
    assert_eq!(p.blocks.len(), 2);
    assert_eq!(p.blocks[1].text, "Done.");
}
