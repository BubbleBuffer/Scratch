//! Worker progress in the AI tasks database (`vault_mcp::tasks::report_progress`):
//! the schema upgrade of older databases, normalization, that only a running
//! task takes progress, and that the last report survives every ending.

use std::fs;
use std::path::PathBuf;

use vault_core::*;
use vault_mcp::jobs::{self, End};
use vault_mcp::tasks;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_progress_test_{}", new_id())))
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

/// A text property of a row, read from the page as the vault stores it.
fn prop(v: &Vault, id: &str, key: &str) -> String {
    let p = v.load_page(id.to_string()).unwrap();
    p.properties
        .iter()
        .find(|x| x.key == key)
        .and_then(|x| match &x.value {
            PropValue::Text(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// A task about a new page, claimed by this device (so it is running).
fn running_task(v: &Vault, prompt: &str) -> PageId {
    let page = v.create_page(None, "Notes".into()).unwrap();
    tasks::create(v, prompt, &page, None, "").unwrap();
    let device = v.device_id();
    tasks::claim_next(v, &device).unwrap().unwrap().id
}

#[test]
fn an_older_tasks_database_gains_the_progress_properties() {
    let t = Temp::new();
    let v = t.open();
    let id = running_task(&v, "Tidy");
    // The schema older builds wrote: no progress properties.
    let coll = tasks::collection(&v).unwrap().unwrap();
    let old: Vec<PropDef> = tasks::schema()
        .into_iter()
        .filter(|d| !["progress", "progress_at"].contains(&d.key.as_str()))
        .collect();
    v.set_schema(coll.clone(), old).unwrap();

    // The task was already running when the new build reports progress.
    tasks::report_progress(&v, &id, "Found 3 notes").unwrap();
    let schema = v.get_schema(coll).unwrap();
    for key in ["progress", "progress_at"] {
        assert!(schema.iter().any(|d| d.key == key), "{key}");
    }
    assert_eq!(prop(&v, &id, "progress"), "Found 3 notes");
    assert!(prop(&v, &id, "progress_at").parse::<i64>().is_ok());
    assert_eq!(tasks::get(&v, &id).unwrap().status, "running");
}

#[test]
fn progress_is_trimmed_capped_and_empty_is_rejected() {
    let t = Temp::new();
    let v = t.open();
    let id = running_task(&v, "Tidy");

    assert!(tasks::report_progress(&v, &id, " \n\t ").is_err());
    assert_eq!(prop(&v, &id, "progress"), "");

    tasks::report_progress(&v, &id, "  Reading the week  ").unwrap();
    assert_eq!(prop(&v, &id, "progress"), "Reading the week");

    // Capped at 500 characters, not bytes: each é is two bytes.
    let long = format!("  {}  ", "é".repeat(700));
    tasks::report_progress(&v, &id, &long).unwrap();
    let stored = prop(&v, &id, "progress");
    assert_eq!(stored.chars().count(), tasks::PROGRESS_MAX);
    assert!(stored.chars().all(|c| c == 'é'));

    // The task itself is unchanged: only the progress properties moved.
    let task = tasks::get(&v, &id).unwrap();
    assert_eq!(
        (task.status.as_str(), task.prompt.as_str()),
        ("running", "Tidy")
    );
}

#[test]
fn only_a_running_task_takes_progress() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Notes".into()).unwrap();
    let queued = tasks::create(&v, "Queued", &page, None, "").unwrap();
    assert!(tasks::report_progress(&v, &queued.id, "Too early").is_err());
    assert_eq!(prop(&v, &queued.id, "progress"), "");
    assert_eq!(prop(&v, &queued.id, "progress_at"), "");

    let device = v.device_id();
    let claimed = tasks::claim_next(&v, &device).unwrap().unwrap();
    tasks::report_progress(&v, &claimed.id, "Now running").unwrap();

    tasks::finish(&v, &claimed.id, Ok("Done".into())).unwrap();
    assert!(tasks::report_progress(&v, &claimed.id, "Too late").is_err());
    assert_eq!(prop(&v, &claimed.id, "progress"), "Now running");

    // A row that is not a task at all takes nothing.
    assert!(tasks::report_progress(&v, &page, "Not a task").is_err());
    assert_eq!(prop(&v, &page, "progress"), "");
}

#[test]
fn no_progress_is_written_after_cancel() {
    let t = Temp::new();
    let v = t.open();
    let id = running_task(&v, "Tidy");
    tasks::report_progress(&v, &id, "Working").unwrap();
    let (at, reported) = (prop(&v, &id, "progress_at"), prop(&v, &id, "progress"));

    assert!(tasks::cancel(&v, &id).unwrap());
    assert!(tasks::report_progress(&v, &id, "Still going").is_err());
    assert_eq!(tasks::get(&v, &id).unwrap().status, "cancelled");
    assert_eq!(prop(&v, &id, "progress"), reported);
    assert_eq!(prop(&v, &id, "progress_at"), at);
}

#[test]
fn latest_progress_replaces_the_previous() {
    let t = Temp::new();
    let v = t.open();
    let id = running_task(&v, "Tidy");

    tasks::report_progress(&v, &id, "First step").unwrap();
    let before = jobs::now_ms();
    tasks::report_progress(&v, &id, "Second step").unwrap();
    let after = jobs::now_ms();

    assert_eq!(prop(&v, &id, "progress"), "Second step");
    let at: i64 = prop(&v, &id, "progress_at").parse().unwrap();
    assert!(
        before <= at && at <= after,
        "{at} not in {before}..={after}"
    );
}

#[test]
fn the_last_progress_survives_every_ending() {
    let t = Temp::new();
    let v = t.open();
    let device = v.device_id();

    // Done.
    let done = running_task(&v, "Finish");
    tasks::report_progress(&v, &done, "Step 2 of 2").unwrap();
    tasks::finish(&v, &done, Ok("The result".into())).unwrap();
    assert_eq!(tasks::get(&v, &done).unwrap().status, "done");
    assert_eq!(prop(&v, &done, "progress"), "Step 2 of 2");

    // Failed.
    let failed = running_task(&v, "Fail");
    tasks::report_progress(&v, &failed, "Step 1 of 2").unwrap();
    tasks::finish(&v, &failed, Err("the model timed out".into())).unwrap();
    assert_eq!(tasks::get(&v, &failed).unwrap().status, "failed");
    assert_eq!(prop(&v, &failed, "progress"), "Step 1 of 2");

    // Cancelled.
    let cancelled = running_task(&v, "Cancel");
    tasks::report_progress(&v, &cancelled, "Half way").unwrap();
    assert!(tasks::cancel(&v, &cancelled).unwrap());
    assert_eq!(prop(&v, &cancelled, "progress"), "Half way");

    // Interrupted by a desktop restart: recovered as failed.
    let recovered = running_task(&v, "Restart");
    tasks::report_progress(&v, &recovered, "Before the restart").unwrap();
    assert_eq!(tasks::recover(&v, &device).unwrap(), 1);
    assert_eq!(tasks::get(&v, &recovered).unwrap().status, "failed");
    assert_eq!(prop(&v, &recovered, "progress"), "Before the restart");

    // Interrupted by a hub restart: a job.
    let job = jobs::create(&v, "Long job", "agent:a", "read", "hub").unwrap();
    jobs::mark_running(&v, &job.id, "s").unwrap();
    tasks::report_progress(&v, &job.id, "Reading page 3").unwrap();
    jobs::finish(&v, &job.id, End::Interrupted).unwrap();
    assert_eq!(
        jobs::get(&v, &job.id).unwrap().unwrap().status,
        "interrupted"
    );
    assert_eq!(prop(&v, &job.id, "progress"), "Reading page 3");
}
