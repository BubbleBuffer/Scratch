//! Job rows in the AI tasks database (`vault_mcp::jobs`): storage, the
//! schema migration of older databases, and that desktops never claim jobs.

use std::fs;
use std::path::PathBuf;

use vault_core::*;
use vault_mcp::jobs::{self, End};
use vault_mcp::tasks;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_jobs_test_{}", new_id())))
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

#[test]
fn jobs_are_rows_that_desktops_never_claim() {
    let t = Temp::new();
    let v = t.open();
    let job = jobs::create(&v, "Summarise my week", "agent:a", "write", "hub").unwrap();
    assert_eq!(job.status, "queued");
    assert_eq!(job.owner, "agent:a");
    assert_eq!(job.tools, "write");
    assert!(job.result.is_none());

    // A desktop skips the job and claims the ordinary task behind it.
    assert!(tasks::claim_next(&v, "desktop").unwrap().is_none());
    let page = v.create_page(None, "Notes".into()).unwrap();
    let task = tasks::create(&v, "Tidy", &page, None, "").unwrap();
    let claimed = tasks::claim_next(&v, "desktop").unwrap().unwrap();
    assert_eq!(claimed.id, task.id);
    // Ordinary tasks are no jobs.
    assert!(jobs::get(&v, &task.id).unwrap().is_none());
    assert_eq!(jobs::all(&v).unwrap().len(), 1);

    jobs::mark_running(&v, &job.id, "s1").unwrap();
    let running = jobs::get(&v, &job.id).unwrap().unwrap();
    assert_eq!((running.status.as_str(), running.session.as_str()), ("running", "s1"));
    jobs::finish(&v, &job.id, End::Completed("All summarised.".into())).unwrap();
    let done = jobs::get(&v, &job.id).unwrap().unwrap();
    assert_eq!(done.status, "completed");
    assert_eq!(done.result.as_deref(), Some("All summarised."));
    assert!(done.finished_ms.is_some());
    assert!(done.ended());
}

#[test]
fn recover_interrupts_and_sweep_trashes() {
    let t = Temp::new();
    let v = t.open();
    let a = jobs::create(&v, "A", "agent:a", "write", "hub").unwrap();
    let b = jobs::create(&v, "B", "agent:a", "write", "hub").unwrap();
    jobs::mark_running(&v, &a.id, "s").unwrap();
    assert_eq!(jobs::recover(&v, "hub").unwrap(), 1);
    let a = jobs::get(&v, &a.id).unwrap().unwrap();
    assert_eq!(a.status, "interrupted");
    assert!(a.error.contains("restarted"));
    assert_eq!(jobs::get(&v, &b.id).unwrap().unwrap().status, "queued");

    // Only ended jobs older than the keep time go.
    let now = jobs::now_ms();
    assert_eq!(jobs::sweep(&v, "hub", 60_000, now).unwrap(), 0);
    assert_eq!(jobs::sweep(&v, "hub", 60_000, now + 120_000).unwrap(), 1);
    assert!(jobs::get(&v, &a.id).unwrap().is_none());
    assert!(v.load_page(a.id.clone()).unwrap().in_trash);
    jobs::finish(&v, &b.id, End::Cancelled).unwrap();
    assert_eq!(jobs::get(&v, &b.id).unwrap().unwrap().status, "cancelled");
}

#[test]
fn an_older_tasks_database_gains_the_job_properties() {
    let t = Temp::new();
    let v = t.open();
    // The schema older builds wrote: no job properties, four statuses.
    let old: Vec<PropDef> = tasks::schema()
        .into_iter()
        .filter(|d| !["node", "owner", "tools", "session", "finished"].contains(&d.key.as_str()))
        .map(|mut d| {
            d.options.retain(|o| !["cancelled", "interrupted"].contains(&o.id.as_str()));
            d
        })
        .collect();
    let id = v.create_page(None, tasks::TITLE.into()).unwrap();
    v.make_collection(id.clone()).unwrap();
    v.set_schema(id.clone(), old).unwrap();
    v.set_property(id.clone(), tasks::MARKER.into(), PropValue::Checkbox(true))
        .unwrap();

    let job = jobs::create(&v, "Go", "agent:a", "read", "hub").unwrap();
    assert_eq!(job.node, "hub");
    assert_eq!(job.tools, "read");
    let schema = v.get_schema(id.clone()).unwrap();
    for key in ["node", "owner", "tools", "session", "finished"] {
        assert!(schema.iter().any(|d| d.key == key), "{key}");
    }
    let status = schema.iter().find(|d| d.key == "status").unwrap();
    assert!(status.options.iter().any(|o| o.id == "interrupted"));
    jobs::finish(&v, &job.id, End::Interrupted).unwrap();
    assert_eq!(jobs::get(&v, &job.id).unwrap().unwrap().status, "interrupted");
}
