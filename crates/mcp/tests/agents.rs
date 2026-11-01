//! AI tasks stored in the vault (placeholders, claiming, the agent's prompt,
//! writing the answer back, cancelling) and the merge prompt and answer. The BubblePaw agent that answers
//! them is tested in crates/agent; the command-line runners (Claude Code,
//! Codex) and their fake were removed on 2026-10-22.

use std::fs;
use std::path::PathBuf;

use vault_core::*;
use vault_mcp::{merge, tasks};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_agents_test_{}", new_id())))
    }
    fn root(&self) -> String {
        self.0.join("vault").to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn texts(v: &Vault, page: &str) -> Vec<String> {
    v.load_page(page.into())
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.text.clone())
        .collect()
}

#[test]
fn a_task_gets_a_placeholder_a_prompt_and_its_answer() {
    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let page = v.create_page(None, "Trip plan".into()).unwrap();
    let ids = v
        .insert_markdown(
            page.clone(),
            None,
            None,
            true,
            "Day one\n\n<br>\n\nDay two".into(),
        )
        .unwrap();
    let blank = v.load_page(page.clone()).unwrap().blocks[1].clone();
    assert!(blank.text.is_empty());

    // Created from the empty paragraph: it becomes the placeholder.
    let task = tasks::create(v, "Summarize the plan", &page, Some(&blank.id), "").unwrap();
    assert_eq!(task.status, tasks::QUEUED);
    assert_eq!(task.block.as_deref(), Some(blank.id.as_str()));
    let p = v.load_page(page.clone()).unwrap();
    assert_eq!(p.blocks[1].kind, BlockKind::Callout);
    assert!(p.blocks[1]
        .text
        .starts_with("AI task queued: Summarize the plan"));
    let coll = tasks::collection(v).unwrap().unwrap();
    assert_eq!(v.load_page(coll.clone()).unwrap().title, "AI tasks");
    assert_eq!(tasks::list(v, 10).unwrap().len(), 1);

    let claimed = tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
    assert_eq!(claimed.id, task.id);
    assert_eq!(claimed.status, tasks::RUNNING);
    assert_eq!(claimed.device, v.device_id());
    assert!(texts(v, &page)[1].starts_with("AI is working on this"));
    assert!(tasks::claim_next(v, "x").unwrap().is_none());

    let prompt = tasks::agent_prompt(v, &claimed, tasks::PromptTools::Js).unwrap();
    assert!(
        prompt.contains("Summarize the plan")
            && prompt.contains("Day two")
            && prompt.contains("js tool")
            && !prompt.contains("AI is working")
    );
    tasks::finish(
        v,
        &claimed.id,
        Ok("- Fake answer
- Read the plan"
            .into()),
    )
    .unwrap();

    let after = texts(v, &page);
    assert_eq!(
        after,
        ["Day one", "Fake answer", "Read the plan", "Day two"]
    );
    assert_eq!(v.load_page(page.clone()).unwrap().blocks[0].id, ids[0]);
    let done = tasks::get(v, &claimed.id).unwrap();
    assert_eq!(done.status, tasks::DONE);
    assert!(v.page_markdown(done.id).unwrap().contains("Fake answer"));
}

/// Copies every page of `from` into `to`, as a sync would.
fn send(from: &Vault, to: &Vault) {
    for pv in from.page_versions().unwrap() {
        to.import_updates(
            pv.page.clone(),
            from.export_updates(pv.page, vec![]).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn task_from_the_phone_runs_on_the_desktop() {
    let t = Temp::new();
    let desktop = Vault::open(t.root()).unwrap();
    let phone_root = t.0.join("phone").to_string_lossy().into_owned();
    let phone = Vault::open(phone_root).unwrap();
    let page = phone.create_page(None, "Groceries".into()).unwrap();
    let task = tasks::create(&phone, "Add a list", &page, None, "").unwrap();
    // New rows are pinned to the phone, so model a legacy row (from before
    // pinning) by clearing its node: any desktop may claim it.
    phone
        .remove_property(task.id.clone(), "node".into())
        .unwrap();
    send(&phone, &desktop);
    let got = tasks::claim_next(&desktop, &desktop.device_id())
        .unwrap()
        .unwrap();
    assert_eq!(got.id, task.id);
    tasks::finish(&desktop, &got.id, Ok("- milk\n- eggs".into())).unwrap();
    send(&desktop, &phone);
    assert_eq!(tasks::get(&phone, &task.id).unwrap().status, tasks::DONE);
    assert_eq!(texts(&phone, &page), ["milk", "eggs"]);
}

#[test]
fn new_tasks_are_pinned_to_the_device_that_made_them() {
    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let me = v.device_id();
    let page = v.create_page(None, "Pins".into()).unwrap();
    let by_create = tasks::create(v, "One", &page, None, "").unwrap();
    assert_eq!(by_create.node, me);
    // An explicit empty node resolves to this device too.
    let by_empty = tasks::create_on(v, "Two", &page, None, "", "").unwrap();
    assert_eq!(by_empty.node, me);
    // A named device and the hub keep the node they were given.
    let named = tasks::create_on(v, "Three", &page, None, "", "other-desktop").unwrap();
    assert_eq!(named.node, "other-desktop");
    let hub = tasks::create_on(v, "Four", &page, None, "", "hub").unwrap();
    assert_eq!(hub.node, "hub");

    // This device claims its two rows, then nothing: not the other
    // device's row, nor the hub's.
    let mut mine = vec![
        tasks::claim_next(v, &me).unwrap().unwrap().id,
        tasks::claim_next(v, &me).unwrap().unwrap().id,
    ];
    mine.sort();
    let mut want = vec![by_create.id.clone(), by_empty.id.clone()];
    want.sort();
    assert_eq!(mine, want);
    assert!(tasks::claim_next(v, &me).unwrap().is_none());
    assert_eq!(
        tasks::claim_next(v, "other-desktop").unwrap().unwrap().id,
        named.id
    );
    assert!(tasks::claim_next(v, "hub").unwrap().is_none());
    assert_eq!(tasks::get(v, &hub.id).unwrap().status, tasks::QUEUED);
}

#[test]
fn two_devices_on_one_synced_vault_claim_only_their_own_tasks() {
    let t = Temp::new();
    let desktop = Vault::open(t.root()).unwrap();
    let phone = Vault::open(t.0.join("phone").to_string_lossy().into_owned()).unwrap();
    let page = desktop.create_page(None, "Shared".into()).unwrap();
    let for_desktop = tasks::create(&desktop, "Desk task", &page, None, "").unwrap();
    send(&desktop, &phone);

    // The phone sees the desktop's task and must not claim it.
    assert!(tasks::claim_next(&phone, &phone.device_id())
        .unwrap()
        .is_none());
    let got = tasks::claim_next(&desktop, &desktop.device_id())
        .unwrap()
        .unwrap();
    assert_eq!(got.id, for_desktop.id);

    // And the other way: the desktop must not claim the phone's task.
    let for_phone = tasks::create(&phone, "Phone task", &page, None, "").unwrap();
    send(&phone, &desktop);
    assert!(tasks::claim_next(&desktop, &desktop.device_id())
        .unwrap()
        .is_none());
    send(&desktop, &phone);
    let got = tasks::claim_next(&phone, &phone.device_id())
        .unwrap()
        .unwrap();
    assert_eq!(got.id, for_phone.id);
}

#[test]
fn concurrent_claims_on_one_vault_go_only_to_the_pinned_device() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let me = v.device_id();
    // Another device's real id: the id of a replica of this vault elsewhere.
    let other = Vault::open(t.0.join("other").to_string_lossy().into_owned())
        .unwrap()
        .device_id();
    assert_ne!(other, me);
    let page = v.create_page(None, "Race".into()).unwrap();
    let done = AtomicBool::new(false);
    // Two scoped claimers run while the rows are created, on the same Vault.
    // Each returns once creation is over and nothing more is claimable.
    let (created, theirs, mine) = std::thread::scope(|s| {
        let creator = s.spawn(|| {
            let ids: Vec<String> = (0..30)
                .map(|i| {
                    tasks::create(v, &format!("Task {i}"), &page, None, "")
                        .unwrap()
                        .id
                })
                .collect();
            done.store(true, Ordering::SeqCst);
            ids
        });
        let claimer = |device: &str| {
            let mut got = Vec::new();
            loop {
                let finished = done.load(Ordering::SeqCst);
                match tasks::claim_next(v, device).unwrap() {
                    Some(task) => got.push(task.id),
                    None if finished => return got,
                    None => std::thread::yield_now(),
                }
            }
        };
        let other_claimer = s.spawn(move || claimer(other.as_str()));
        let my_claimer = s.spawn(move || claimer(me.as_str()));
        (
            creator.join().unwrap(),
            other_claimer.join().unwrap(),
            my_claimer.join().unwrap(),
        )
    });
    assert!(theirs.is_empty(), "the other device claimed {theirs:?}");
    let mut mine = mine;
    mine.sort();
    let mut created = created;
    created.sort();
    assert_eq!(mine, created);
}

#[test]
fn other_devices_backlog_does_not_hide_this_devices_task() {
    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let me = v.device_id();
    let page = v.create_page(None, "Backlog".into()).unwrap();
    // More queued rows than one claim reads, all pinned elsewhere.
    for i in 0..201 {
        tasks::create_on(v, &format!("Other {i}"), &page, None, "", "other-desktop").unwrap();
    }
    let mine = tasks::create(v, "Mine", &page, None, "").unwrap();
    let got = tasks::claim_next(v, &me).unwrap().unwrap();
    assert_eq!(got.id, mine.id);
}

#[test]
fn ask_ai_on_this_device_is_pinned_and_cancellable() {
    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let me = v.device_id();
    let page = v.create_page(None, "Notes".into()).unwrap();
    // Pinned to this device: no other desktop claims it.
    let task = tasks::create_on(v, "Explain this", &page, None, "", &me).unwrap();
    assert_eq!(task.node, me);
    assert!(tasks::claim_next(v, "another-desktop").unwrap().is_none());
    let got = tasks::claim_next(v, &me).unwrap().unwrap();
    assert_eq!(got.id, task.id);

    // Cancelled while running: the late result is dropped and the
    // placeholder says it was cancelled.
    assert!(tasks::cancel(v, &got.id).unwrap());
    // Not stopped until the runner says so: still running, cancel requested.
    let asked = tasks::get(v, &got.id).unwrap();
    assert_eq!(asked.status, tasks::RUNNING);
    assert!(asked.cancel_requested_ms.is_some());
    tasks::finish_attempt(v, &got.id, Some(&asked.attempt), Ok("Too late".into())).unwrap();
    let after = texts(v, &page);
    assert!(after.iter().all(|t| !t.contains("Too late")), "{after:?}");
    assert!(
        after.iter().any(|t| t.contains("AI task cancelled")),
        "{after:?}"
    );
    assert_eq!(tasks::get(v, &got.id).unwrap().status, tasks::CANCELLED);
    // A second cancel does nothing.
    assert!(!tasks::cancel(v, &got.id).unwrap());

    // A hub job is cancelled on the hub, not here.
    let hub = tasks::create_on(v, "On the hub", &page, None, "", "hub").unwrap();
    assert!(tasks::cancel(v, &hub.id).is_err());
    assert!(tasks::claim_next(v, "hub").unwrap().is_none());
}

#[test]
fn a_row_naming_a_removed_runner_is_claimed_like_any_other() {
    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let page = v.create_page(None, "Old row".into()).unwrap();
    for runner in ["claude", "codex", "deepseek"] {
        let task = tasks::create(v, "Answer me", &page, None, runner).unwrap();
        let got = tasks::claim_next(v, &v.device_id()).unwrap().unwrap();
        assert_eq!(got.id, task.id);
        assert_eq!(got.runner, runner);
        tasks::agent_prompt(v, &got, tasks::PromptTools::Js).unwrap();
        tasks::finish(v, &got.id, Ok(format!("Answered for {runner}"))).unwrap();
        assert_eq!(tasks::get(v, &got.id).unwrap().status, tasks::DONE);
    }
    let after = texts(v, &page);
    for runner in ["claude", "codex", "deepseek"] {
        assert!(
            after.contains(&format!("Answered for {runner}")),
            "{after:?}"
        );
    }
}

#[test]
fn merge_prompt_and_answer() {
    let t = Temp::new();
    let v = &Vault::open(t.root()).unwrap();
    let page = v.create_page(None, "Shared".into()).unwrap();
    let ids = v
        .insert_markdown(page.clone(), None, None, true, "Meet at 10".into())
        .unwrap();
    let side = |text: &str| Block {
        id: ids[0].clone(),
        kind: BlockKind::Paragraph,
        text: text.into(),
        marks: vec![],
        checked: false,
        children: vec![],
        target: None,
        color: None,
        width: None,
        numbered_start: None,
    };
    let c = Conflict {
        page: page.clone(),
        id: ids[0].clone(),
        format: 2,
        block: ids[0].clone(),
        time_ms: 0,
        scope: ConflictScope::Block {
            block: ids[0].clone(),
        },
        status: ConflictStatus::Pending,
        applied: vec![],
        sides: vec![
            ConflictSide::new(
                "laptop-device".into(),
                0,
                None,
                SideState::Present {
                    content: vec![side("Meet at 10 at the cafe")],
                },
                None,
            ),
            ConflictSide::new(
                "phone-device".into(),
                0,
                None,
                SideState::Present {
                    content: vec![side("Meet at 11")],
                },
                None,
            ),
        ],
    }
    .with_legacy_blocks();
    let prompt = merge::prompt(v, &c);
    assert!(prompt.contains("The edited version wins; merge only text that both sides edited."));
    assert!(prompt.contains("Keep children intact, including unchanged descendants."));
    assert!(!prompt.contains("intent of both edits"));
    assert!(
        prompt.contains("Meet at 10 at the cafe")
            && prompt.contains("Meet at 11")
            && prompt.contains("\"Shared\"")
    );
    let p = merge::parse(v, "**Merged** by the agent");
    assert_eq!(p.text, "Merged by the agent");
    assert_eq!(p.marks.len(), 1);
    assert_eq!(p.marks[0].kind, MarkKind::Bold);
    // Fences and several paragraphs.
    assert_eq!(merge::parse(v, "```\nA *b*\n```").text, "A b");
    assert_eq!(merge::parse(v, "One\n\nTwo").text, "One\nTwo");
    // Nothing was written.
    assert_eq!(texts(v, &page), ["Meet at 10"]);
}
