use std::{fs, path::PathBuf};

use serde_json::json;
use vault_core::{
    new_id, node_record_id, BlockKind, BlockOp, NewBlock, Vault, RUNNER_ONLINE_WINDOW_MS,
};
use vault_mcp::{runners, tasks};

const SEEN: i64 = 1_700_000_000_000;
const SEEN_TEXT: &str = "2023-11-14T22:13:20Z";

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_runners_{}", new_id())))
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

fn record(v: &Vault, id: &str, name: &str, kind: &str, seen: Option<&str>) {
    v.write_node_record(
        id.into(),
        json!({
            "device_id": id, "name": name, "kind": kind, "last_seen": seen,
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn hub_is_always_first_and_device_ids_and_self_identity_are_preserved() {
    let t = Temp::new();
    let v = t.open();
    let empty = runners::list_at(&v, false, true, SEEN).unwrap();
    assert_eq!(empty.runners.len(), 1);
    assert_eq!(empty.runners[0].id, "hub");
    assert!(!empty.runners[0].online);
    assert_eq!(empty.runners[0].last_seen_ms, None);

    let me = v.device_id();
    record(&v, &me, "Work PC", "desktop", Some(SEEN_TEXT));
    record(&v, "other-pc", "Laptop", "desktop", Some(SEEN_TEXT));
    record(&v, "hub-device", "Scheduler hub", "hub", Some(SEEN_TEXT));
    let list = runners::list_at(&v, true, true, SEEN).unwrap();
    assert_eq!(
        list.runners
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["hub", "other-pc", me.as_str()]
    );
    assert_eq!(list.runners[0].kind, runners::RunnerKind::Hub);
    assert_eq!(list.runners[1].kind, runners::RunnerKind::Pc);
    assert_eq!(list.runners.iter().filter(|r| r.is_this_device).count(), 1);
    assert!(list.runners[2].is_this_device);
    assert_eq!(list.default_runner_id, me);
    assert!(list
        .runners
        .iter()
        .all(|r| r.online && r.last_seen_ms == Some(SEEN)));
    assert!(list
        .runners
        .iter()
        .all(|r| r.id != node_record_id(&v.device_id())));
}

#[test]
fn online_window_is_exclusive_at_five_minutes_for_hub_and_pcs() {
    let t = Temp::new();
    let v = t.open();
    record(&v, &v.device_id(), "PC", "desktop", Some(SEEN_TEXT));
    record(&v, "hub-device", "Hub", "hub", Some(SEEN_TEXT));
    record(&v, "unknown-time", "Old PC", "desktop", None);
    assert_eq!(RUNNER_ONLINE_WINDOW_MS, 300_000);
    for (age, expected) in [
        (0, true),
        (299_999, true),
        (300_000, false),
        (300_001, false),
    ] {
        let list = runners::list_at(&v, true, true, SEEN + age).unwrap();
        for r in list.runners {
            assert_eq!(
                r.online,
                r.last_seen_ms.is_some() && expected,
                "{} at age {age}",
                r.id
            );
        }
    }
    assert!(runners::is_online(Some(SEEN + 1), SEEN));
    assert!(!runners::is_online(None, SEEN));
}

#[test]
fn defaults_follow_synced_choice_platform_and_usable_host_fallback() {
    let t = Temp::new();
    let v = t.open();
    record(&v, &v.device_id(), "PC", "desktop", Some(SEEN_TEXT));
    let chosen = |hub, local| {
        runners::list_at(&v, hub, local, SEEN)
            .unwrap()
            .default_runner_id
    };
    assert_eq!(chosen(true, true), v.device_id());
    assert_eq!(chosen(false, false), "hub");
    v.set_agent_default_runner(Some("hub".into())).unwrap();
    assert_eq!(chosen(true, true), "hub");
    assert_eq!(chosen(false, true), v.device_id());
    v.set_agent_default_runner(Some("this_device".into()))
        .unwrap();
    assert_eq!(chosen(true, true), v.device_id());
    assert_eq!(chosen(true, false), "hub");
    v.set_agent_default_runner(None).unwrap();
    assert_eq!(chosen(true, true), v.device_id());
}

#[test]
fn assignment_creates_a_real_task_pinned_to_the_chosen_offline_pc() {
    let t = Temp::new();
    let v = t.open();
    record(&v, "chosen-pc", "Laptop", "desktop", Some(SEEN_TEXT));
    let page = v.create_page(None, "Plan".into()).unwrap();
    let block = new_id();
    v.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: block.clone(),
                kind: BlockKind::Paragraph,
                text: String::new(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    let task = runners::assign(&v, "Summarize this", &page, Some(&block), "chosen-pc").unwrap();
    assert_eq!(task.node, "chosen-pc");
    assert_eq!(task.status, tasks::QUEUED);
    assert_eq!(task.runner, "agent");
    assert_eq!(task.page.as_deref(), Some(page.as_str()));
    assert_eq!(task.block.as_deref(), Some(block.as_str()));
    assert_eq!(tasks::get(&v, &task.id).unwrap(), task);
    assert!(tasks::claim_next(&v, &v.device_id()).unwrap().is_none());
    assert_eq!(
        tasks::claim_next(&v, "chosen-pc").unwrap().unwrap().id,
        task.id
    );
}

#[test]
fn assignment_accepts_hub_without_a_record_and_desktops_cannot_claim_it() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Plan".into()).unwrap();
    let task = runners::assign(&v, "Plan it", &page, None, "hub").unwrap();
    assert_eq!(task.node, "hub");
    assert!(tasks::claim_next(&v, &v.device_id()).unwrap().is_none());
}

#[test]
fn invalid_destinations_and_blocks_do_not_create_rows_or_placeholders() {
    let t = Temp::new();
    let v = t.open();
    record(&v, &v.device_id(), "PC", "desktop", Some(SEEN_TEXT));
    let page = v.create_page(None, "Plan".into()).unwrap();
    let before = v.load_page(page.clone()).unwrap();
    for id in ["", "missing", &node_record_id(&v.device_id())] {
        assert!(runners::assign(&v, "Plan it", &page, None, id).is_err());
    }
    assert!(runners::assign(&v, "Plan it", &page, Some("missing-block"), &v.device_id()).is_err());
    assert_eq!(v.load_page(page).unwrap(), before);
    assert_eq!(tasks::collection(&v).unwrap(), None);
}
