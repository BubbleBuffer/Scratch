//! Steering a running task: to-dos the user adds under it, the worker's list
//! taking them over, and a requested update of the note.

use std::fs;
use std::path::PathBuf;

use vault_core::*;
use vault_mcp::tasks::{self, attempts, SubtaskItem};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_steering_test_{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.join("vault").to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A running task about a page with one paragraph, with its subtasks begun.
fn start(v: &Vault) -> (PageId, tasks::Task) {
    let page = v.create_page(None, "Notes".into()).unwrap();
    let last = v.load_page(page.clone()).unwrap().blocks.last().map(|b| b.id.clone());
    v.insert_markdown(page.clone(), None, last, false, "Write the plan".into())
        .unwrap();
    tasks::create(v, "Do it", &page, None, "").unwrap();
    let task = tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
    tasks::begin_subtasks(v, &task.id, "s1").unwrap();
    tasks::record_baseline(v, &task.id).unwrap();
    (page, task)
}

fn children(v: &Vault, page: &str, parent: &str) -> Vec<(String, String, bool)> {
    fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
        blocks.iter().find_map(|b| {
            if b.id == id {
                Some(b)
            } else {
                find(&b.children, id)
            }
        })
    }
    let p = v.load_page(page.to_string()).unwrap();
    find(&p.blocks, parent)
        .unwrap()
        .children
        .iter()
        .map(|c| (c.id.clone(), c.text.clone(), c.checked))
        .collect()
}

fn item(text: &str, status: &str) -> SubtaskItem {
    SubtaskItem {
        text: text.into(),
        status: status.into(),
    }
}

#[test]
fn a_step_added_under_the_task_is_pending_until_it_is_sent() {
    let t = Temp::new();
    let v = t.open();
    let (page, task) = start(&v);
    assert!(tasks::pending_steps(&v, &task.id).unwrap().is_empty());

    tasks::add_step(&v, &task.id, "  Also check the dates ").unwrap();
    let steps = tasks::pending_steps(&v, &task.id).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].text, "Also check the dates");
    let placeholder = task.block.clone().unwrap();
    assert_eq!(children(&v, &page, &placeholder).len(), 1);

    let attempt = tasks::get(&v, &task.id).unwrap().attempt;
    attempts::mark_steered(&v, &task.id, &attempt, &steps[0].block).unwrap();
    assert!(tasks::pending_steps(&v, &task.id).unwrap().is_empty());
}

#[test]
fn what_was_under_the_task_before_the_run_is_not_steering() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Notes".into()).unwrap();
    tasks::create(&v, "Do it", &page, None, "").unwrap();
    let task = tasks::claim_next(&v, &v.device_id()).unwrap().unwrap();
    let placeholder = task.block.clone().unwrap();
    // A to-do the user had already put under the placeholder.
    v.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: Some(placeholder),
            after: None,
            block: NewBlock {
                id: new_id(),
                kind: BlockKind::Todo,
                text: "Mine".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    tasks::begin_subtasks(&v, &task.id, "s1").unwrap();
    tasks::record_baseline(&v, &task.id).unwrap();
    assert!(tasks::pending_steps(&v, &task.id).unwrap().is_empty());
    tasks::add_step(&v, &task.id, "New one").unwrap();
    let steps = tasks::pending_steps(&v, &task.id).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].text, "New one");
}

#[test]
fn the_workers_list_takes_over_a_step_instead_of_adding_a_copy() {
    let t = Temp::new();
    let v = t.open();
    let (page, task) = start(&v);
    let placeholder = task.block.clone().unwrap();
    tasks::mirror_subtasks(&v, &task.id, "s1", &[item("Read", "done")]).unwrap();

    tasks::add_step(&v, &task.id, "Check the dates").unwrap();
    let step = tasks::pending_steps(&v, &task.id).unwrap().remove(0);
    let attempt = tasks::get(&v, &task.id).unwrap().attempt;
    attempts::mark_steered(&v, &task.id, &attempt, &step.block).unwrap();

    // The worker's list now holds the step as well.
    tasks::mirror_subtasks(
        &v,
        &task.id,
        "s1",
        &[item("Read", "done"), item("Check the dates", "pending")],
    )
    .unwrap();
    let kids = children(&v, &page, &placeholder);
    assert_eq!(kids.len(), 2, "{kids:?}");
    assert!(kids.iter().any(|(id, _, _)| *id == step.block));

    // Done on the worker's list ticks the user's to-do.
    tasks::mirror_subtasks(
        &v,
        &task.id,
        "s1",
        &[item("Read", "done"), item("Check the dates", "done")],
    )
    .unwrap();
    let kids = children(&v, &page, &placeholder);
    assert_eq!(kids.len(), 2, "{kids:?}");
    assert!(kids.iter().all(|(_, _, checked)| *checked), "{kids:?}");
}

#[test]
fn a_step_is_refused_while_a_cancel_is_pending_or_when_nothing_runs() {
    let t = Temp::new();
    let v = t.open();
    let (_, task) = start(&v);
    assert!(tasks::add_step(&v, &task.id, "   ").is_err());
    tasks::add_step(&v, &task.id, "One").unwrap();
    tasks::cancel(&v, &task.id).unwrap();
    assert!(tasks::pending_steps(&v, &task.id).unwrap().is_empty());
    assert!(tasks::add_step(&v, &task.id, "Two").is_err());
}

#[test]
fn an_update_is_requested_sent_and_clears_the_stale_mark() {
    let t = Temp::new();
    let v = t.open();
    let (page, task) = start(&v);
    let attempt = tasks::get(&v, &task.id).unwrap().attempt;
    let rev = tasks::source_revision(&v, &task);
    attempts::started(
        &v,
        &task.id,
        &attempt,
        attempts::Started {
            session: "s1".into(),
            brief: "brief".into(),
            source_rev: rev.clone(),
            model: None,
        },
    )
    .unwrap();
    let row = || tasks::get(&v, &task.id).unwrap();
    assert!(!tasks::is_stale(&v, &row()));
    assert_eq!(tasks::update_message(&v, &task.id).unwrap(), None);

    // The user edits the note: the worker runs on an earlier revision.
    let last = v.load_page(page.clone()).unwrap().blocks.last().map(|b| b.id.clone());
    v.insert_markdown(page, None, last, false, "A new paragraph".into())
        .unwrap();
    assert!(tasks::is_stale(&v, &row()));
    // Editing alone sends nothing.
    assert_eq!(tasks::update_message(&v, &task.id).unwrap(), None);

    assert!(tasks::request_update(&v, &task.id).unwrap());
    let (message, new_rev) = tasks::update_message(&v, &task.id).unwrap().unwrap();
    assert!(message.contains("A new paragraph"), "{message}");
    assert_ne!(new_rev, rev);

    tasks::update_sent(&v, &task.id, Some(&attempt), &new_rev).unwrap();
    assert!(!tasks::is_stale(&v, &row()));
    assert_eq!(tasks::update_message(&v, &task.id).unwrap(), None);
    let a = &attempts::list(&v, &task.id).unwrap()[0];
    assert!(a.events.iter().any(|e| e.kind == attempts::kind::UPDATE_SENT));
    // The brief stays the one the worker was given.
    assert_eq!(a.brief, "brief");
}

#[test]
fn an_update_needs_a_running_task() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Notes".into()).unwrap();
    let task = tasks::create(&v, "Do it", &page, None, "").unwrap();
    assert!(!tasks::request_update(&v, &task.id).unwrap());
}
