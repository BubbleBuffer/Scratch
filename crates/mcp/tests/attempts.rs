//! Attempts of an AI task (`vault_mcp::tasks::attempts`): one record per
//! run, a cancel that is a request until the runner acknowledges it, retry
//! guards, and late results that never replace a newer attempt, a
//! cancellation or a later decision.

use std::fs;
use std::path::PathBuf;

use vault_core::*;
use vault_mcp::tasks;
use vault_mcp::tasks::attempts::{self, kind};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        Temp(std::env::temp_dir().join(format!("vault_attempts_test_{}", new_id())))
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

fn running_task(v: &Vault, prompt: &str) -> (PageId, PageId) {
    let page = v.create_page(None, "Notes".into()).unwrap();
    tasks::create(v, prompt, &page, None, "").unwrap();
    let device = v.device_id();
    (page, tasks::claim_next(v, &device).unwrap().unwrap().id)
}

fn kinds(v: &Vault, id: &str, attempt: &str) -> Vec<String> {
    attempts::list(v, id)
        .unwrap()
        .into_iter()
        .find(|a| a.id == attempt)
        .unwrap()
        .events
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

#[test]
fn a_claim_starts_an_attempt_and_a_retry_chains_a_new_one() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Tidy");
    let first = tasks::get(&v, &id).unwrap().attempt;
    assert!(!first.is_empty());
    let all = attempts::list(&v, &id).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].parent, None);
    assert_eq!(all[0].node, v.device_id());
    // Nothing is made up: what the runner has not reported stays unknown.
    assert_eq!(all[0].usage, None);
    assert_eq!(all[0].model, None);
    assert_eq!(
        kinds(&v, &id, &first),
        vec![kind::REQUESTED, kind::ACCEPTED]
    );

    tasks::finish_attempt(&v, &id, Some(&first), Err("boom".into())).unwrap();
    tasks::retry(&v, &id).unwrap();
    let device = v.device_id();
    tasks::claim_next(&v, &device).unwrap().unwrap();
    let second = tasks::get(&v, &id).unwrap().attempt;
    assert_ne!(first, second);
    let all = attempts::list(&v, &id).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].parent.as_deref(), Some(first.as_str()));
    assert_eq!(all[0].outcome.as_deref(), Some("failed"));
    assert_eq!(all[0].result.as_deref(), Some("boom"));
}

#[test]
fn started_keeps_the_brief_the_revision_and_the_model() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Tidy");
    let task = tasks::get(&v, &id).unwrap();
    attempts::started(
        &v,
        &id,
        &task.attempt,
        attempts::Started {
            session: "s1".into(),
            brief: "the brief".into(),
            source_rev: tasks::source_revision(&v, &task),
            model: Some("opencode/deepseek-v4.1-flash".into()),
        },
    )
    .unwrap();
    let a = &attempts::list(&v, &id).unwrap()[0];
    assert_eq!(a.session, "s1");
    assert_eq!(a.brief, "the brief");
    assert_eq!(a.provider.as_deref(), Some("opencode"));
    assert_eq!(a.model.as_deref(), Some("opencode/deepseek-v4.1-flash"));
    assert_eq!(a.events.last().unwrap().kind, kind::STARTED);
}

#[test]
fn a_cancel_is_a_request_until_the_runner_acknowledges() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Tidy");
    let attempt = tasks::get(&v, &id).unwrap().attempt;

    assert!(tasks::cancel(&v, &id).unwrap());
    let asked = tasks::get(&v, &id).unwrap();
    assert_eq!(
        asked.status,
        tasks::RUNNING,
        "never stopped before it stops"
    );
    assert!(asked.cancel_requested_ms.is_some());
    // Asking again changes nothing.
    assert!(tasks::cancel(&v, &id).unwrap());
    assert_eq!(
        kinds(&v, &id, &attempt)
            .iter()
            .filter(|k| *k == kind::CANCEL_REQUESTED)
            .count(),
        1
    );
    // A retry waits for the acknowledgement.
    assert!(tasks::retry(&v, &id).is_err());

    // The run finished with a result at the same moment: the cancel wins and
    // the result is kept in history.
    tasks::finish_attempt(&v, &id, Some(&attempt), Ok("The answer".into())).unwrap();
    let done = tasks::get(&v, &id).unwrap();
    assert_eq!(done.status, tasks::CANCELLED);
    let a = &attempts::list(&v, &id).unwrap()[0];
    assert_eq!(a.outcome.as_deref(), Some("cancelled"));
    assert_eq!(a.result.as_deref(), Some("The answer"));
    let k = kinds(&v, &id, &attempt);
    assert!(k.contains(&kind::CANCEL_ACKNOWLEDGED.to_string()), "{k:?}");
    assert!(k.contains(&kind::LATE_RESULT.to_string()), "{k:?}");
    let body = v.load_page(id.clone()).unwrap();
    assert!(
        v.blocks_markdown(body.blocks).trim().is_empty(),
        "the task's body stays without the late result"
    );
}

#[test]
fn a_queued_task_is_cancelled_at_once() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Notes".into()).unwrap();
    let task = tasks::create(&v, "Wait", &page, None, "").unwrap();
    assert!(tasks::cancel(&v, &task.id).unwrap());
    assert_eq!(tasks::get(&v, &task.id).unwrap().status, tasks::CANCELLED);
}

#[test]
fn progress_is_rejected_while_a_cancel_is_pending() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Tidy");
    tasks::report_progress(&v, &id, "Working").unwrap();
    tasks::cancel(&v, &id).unwrap();
    assert!(tasks::report_progress(&v, &id, "More").is_err());
    let attempt = tasks::get(&v, &id).unwrap().attempt;
    assert!(kinds(&v, &id, &attempt).contains(&kind::CHECKPOINT.to_string()));
}

#[test]
fn a_late_result_never_replaces_a_newer_attempt() {
    let t = Temp::new();
    let v = t.open();
    let (page, id) = running_task(&v, "Tidy");
    let old = tasks::get(&v, &id).unwrap().attempt;

    // Cancel, acknowledge, retry: a second attempt runs.
    tasks::cancel(&v, &id).unwrap();
    tasks::finish_attempt(&v, &id, Some(&old), Err("cancelled".into())).unwrap();
    tasks::retry(&v, &id).unwrap();
    let device = v.device_id();
    tasks::claim_next(&v, &device).unwrap().unwrap();
    let new = tasks::get(&v, &id).unwrap().attempt;
    assert_ne!(old, new);

    // The old run delivers a result late.
    tasks::finish_attempt(&v, &id, Some(&old), Ok("Old answer".into())).unwrap();
    let row = tasks::get(&v, &id).unwrap();
    assert_eq!(row.status, tasks::RUNNING);
    assert_eq!(row.attempt, new);
    let first = attempts::list(&v, &id)
        .unwrap()
        .into_iter()
        .find(|a| a.id == old)
        .unwrap();
    assert_eq!(first.result.as_deref(), Some("Old answer"));
    assert!(first.events.iter().any(|e| e.kind == kind::LATE_RESULT));
    let p = v.load_page(page).unwrap();
    assert!(!v.blocks_markdown(p.blocks).contains("Old answer"));

    // The newer attempt still ends normally.
    tasks::finish_attempt(&v, &id, Some(&new), Ok("New answer".into())).unwrap();
    assert_eq!(tasks::get(&v, &id).unwrap().status, tasks::DONE);
}

#[test]
fn a_result_after_the_task_ended_is_kept_but_changes_nothing() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Tidy");
    let attempt = tasks::get(&v, &id).unwrap().attempt;
    tasks::finish_attempt(&v, &id, Some(&attempt), Err("boom".into())).unwrap();
    tasks::finish_attempt(&v, &id, Some(&attempt), Ok("Again".into())).unwrap();
    let row = tasks::get(&v, &id).unwrap();
    assert_eq!(row.status, tasks::FAILED);
    assert_eq!(row.error, "boom");
}

#[test]
fn retry_refuses_a_running_task_and_clears_the_old_progress() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Tidy");
    assert!(tasks::retry(&v, &id).is_err());
    tasks::report_progress(&v, &id, "Half way").unwrap();
    let attempt = tasks::get(&v, &id).unwrap().attempt;
    tasks::finish_attempt(&v, &id, Some(&attempt), Err("boom".into())).unwrap();
    tasks::retry(&v, &id).unwrap();
    let row = tasks::get(&v, &id).unwrap();
    assert_eq!(row.status, tasks::QUEUED);
    assert_eq!(row.progress, "");
    // A queued task is already waiting.
    tasks::retry(&v, &id).unwrap();
}

#[test]
fn the_source_revision_follows_the_note_and_ignores_the_run() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Notes".into()).unwrap();
    let p = v.load_page(page.clone()).unwrap();
    let last = p.blocks.last().map(|b| b.id.clone());
    v.insert_markdown(page.clone(), None, last, false, "Write the plan".into())
        .unwrap();
    tasks::create(&v, "Do it", &page, None, "").unwrap();
    let device = v.device_id();
    let task = tasks::claim_next(&v, &device).unwrap().unwrap();
    let rev = tasks::source_revision(&v, &task);
    assert_eq!(rev.len(), 16);
    assert_eq!(rev, tasks::source_revision(&v, &task));

    // The run's own text (progress, the placeholder) is not an edit.
    tasks::report_progress(&v, &task.id, "Working").unwrap();
    assert_eq!(
        rev,
        tasks::source_revision(&v, &tasks::get(&v, &task.id).unwrap())
    );

    // An edit of the note is.
    let end = v
        .load_page(page.clone())
        .unwrap()
        .blocks
        .last()
        .map(|b| b.id.clone());
    v.insert_markdown(page, None, end, false, "A new paragraph".into())
        .unwrap();
    assert_ne!(
        rev,
        tasks::source_revision(&v, &tasks::get(&v, &task.id).unwrap())
    );
}

#[test]
fn an_old_ended_job_loses_its_session_but_keeps_its_row_and_attempts() {
    use vault_mcp::jobs::{self, End};
    let t = Temp::new();
    let v = t.open();
    let job = jobs::create(&v, "Old job", "agent:a", "read", "hub").unwrap();
    jobs::mark_running(&v, &job.id, "session-1").unwrap();
    jobs::finish(&v, &job.id, End::Completed("The result".into())).unwrap();
    let now = jobs::now_ms();
    let day = 86_400_000;

    // Too young: nothing expires.
    assert!(jobs::expired_sessions(&v, "hub", 30 * day, now).unwrap().is_empty());
    // Old enough: the session is due, the row is still there.
    let due = jobs::expired_sessions(&v, "hub", 30 * day, now + 31 * day).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].session, "session-1");

    jobs::session_expired(&v, &job.id).unwrap();
    assert!(jobs::expired_sessions(&v, "hub", 30 * day, now + 31 * day)
        .unwrap()
        .is_empty());
    let kept = jobs::get(&v, &job.id).unwrap().unwrap();
    assert_eq!(kept.status, "completed");
    assert!(kept.session.is_empty());
    assert!(!attempts::list(&v, &job.id).unwrap().is_empty());
}

#[test]
fn a_reviewed_task_ends_in_review_and_accept_makes_it_done() {
    let t = Temp::new();
    let v = t.open();
    let page = v.create_page(None, "Notes".into()).unwrap();
    let me = v.device_id();
    let task = vault_mcp::runners::assign(&v, "Plan it", &page, None, &me, None);
    // This device is not a published PC runner here: use a plain row.
    let id = match task {
        Ok(t) => t.id,
        Err(_) => {
            let t = tasks::create(&v, "Plan it", &page, None, "agent").unwrap();
            tasks::set_review(&v, &t.id).unwrap();
            t.id
        }
    };
    tasks::claim_next(&v, &v.device_id()).unwrap().unwrap();
    assert!(!tasks::accept(&v, &id).unwrap());
    tasks::finish(&v, &id, Ok("The plan".into())).unwrap();
    assert_eq!(tasks::get(&v, &id).unwrap().status, tasks::REVIEW);
    assert!(tasks::accept(&v, &id).unwrap());
    assert_eq!(tasks::get(&v, &id).unwrap().status, tasks::DONE);
    assert!(!tasks::accept(&v, &id).unwrap());

    // A task without the policy still ends in done.
    let plain = tasks::create(&v, "Plain", &page, None, "agent").unwrap();
    tasks::claim_next(&v, &v.device_id()).unwrap().unwrap();
    tasks::finish(&v, &plain.id, Ok("Done".into())).unwrap();
    assert_eq!(tasks::get(&v, &plain.id).unwrap().status, tasks::DONE);
}

#[test]
fn a_silent_runner_is_stalled_after_ten_minutes_and_only_then_can_be_failed() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Long");
    let task = tasks::get(&v, &id).unwrap();
    // claim_next wrote a heartbeat.
    let beat = task.heartbeat_ms.expect("a heartbeat at claim");
    assert_eq!(tasks::stalled_since(&task, beat + 1000), None);
    assert_eq!(
        tasks::stalled_since(&task, beat + tasks::STALL_MS + 1),
        Some(beat)
    );
    // Not stalled now: refused, nothing changes.
    assert!(!tasks::mark_failed(&v, &id).unwrap());
    assert_eq!(tasks::get(&v, &id).unwrap().status, "running");

    // An old heartbeat: stalled, and the user may mark it failed.
    v.set_property(
        id.clone(),
        "heartbeat".into(),
        PropValue::Text((jobs_now() - tasks::STALL_MS - 1000).to_string()),
    )
    .unwrap();
    assert!(tasks::mark_failed(&v, &id).unwrap());
    let failed = tasks::get(&v, &id).unwrap();
    assert_eq!(failed.status, "failed");
    assert!(failed.error.contains("stopped responding"));
}

fn jobs_now() -> i64 {
    vault_mcp::jobs::now_ms()
}

#[test]
fn an_old_ended_task_loses_its_session_but_keeps_its_row() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Old");
    v.set_property(id.clone(), "session".into(), PropValue::Text("s-1".into()))
        .unwrap();
    tasks::finish(&v, &id, Ok("Result".into())).unwrap();
    let me = v.device_id();
    let day = 86_400_000;
    let now = jobs_now();
    assert!(tasks::expired_sessions(&v, &me, 30 * day, now).unwrap().is_empty());
    let due = tasks::expired_sessions(&v, &me, 30 * day, now + 31 * day).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].1, "s-1");
    tasks::session_expired(&v, &id).unwrap();
    assert!(tasks::expired_sessions(&v, &me, 30 * day, now + 31 * day)
        .unwrap()
        .is_empty());
    assert_eq!(tasks::get(&v, &id).unwrap().status, "done");
    assert!(!attempts::list(&v, &id).unwrap().is_empty());
}

fn board(v: &Vault, title: &str, columns: &[(&str, &str)], card: &str, in_col: &str) -> PageId {
    let b = v.create_page(None, title.into()).unwrap();
    v.set_page_type(b.clone(), "kanban".into()).unwrap();
    let cols: Vec<_> = columns
        .iter()
        .map(|(id, t)| serde_json::json!({"id": id, "title": t}))
        .collect();
    v.type_data_set(b.clone(), "kanban".into(), vec!["columns".into()], serde_json::Value::Array(cols).to_string())
        .unwrap();
    for (id, _) in columns {
        let list = if *id == in_col { serde_json::json!([card]) } else { serde_json::json!([]) };
        v.type_data_set(b.clone(), "kanban".into(), vec!["cards".into(), (*id).into()], list.to_string())
            .unwrap();
    }
    b
}

fn column_of(v: &Vault, b: &str, card: &str) -> String {
    let d: serde_json::Value =
        serde_json::from_str(&v.type_data(b.to_string(), "kanban".into()).unwrap()).unwrap();
    d["cards"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, l)| l.as_array().unwrap().iter().any(|x| x == card))
        .map(|(k, _)| k.clone())
        .unwrap_or_default()
}

#[test]
fn a_reviewed_run_moves_its_card_to_review_and_accept_to_the_last_column() {
    let t = Temp::new();
    let v = t.open();
    let card = v.create_page(None, "Card".into()).unwrap();
    let with = board(
        &v,
        "With review",
        &[("a", "To do"), ("b", "review "), ("c", "Done")],
        &card,
        "a",
    );
    let without = board(&v, "No review", &[("x", "Todo"), ("y", "Done")], &card, "x");
    let t1 = tasks::create(&v, "Plan it", &card, None, "agent").unwrap();
    tasks::set_review(&v, &t1.id).unwrap();
    tasks::claim_next(&v, &v.device_id()).unwrap().unwrap();
    tasks::finish(&v, &t1.id, Ok("The plan".into())).unwrap();
    assert_eq!(column_of(&v, &with, &card), "b");
    assert_eq!(column_of(&v, &without, &card), "x", "no Review column: left alone");

    // Moving again changes nothing.
    assert_eq!(tasks::board::move_card(&v, &card, tasks::board::Target::Review).unwrap(), 0);

    assert!(tasks::accept(&v, &t1.id).unwrap());
    assert_eq!(column_of(&v, &with, &card), "c");
    assert_eq!(column_of(&v, &without, &card), "y");
    assert_eq!(tasks::board::move_card(&v, &card, tasks::board::Target::Last).unwrap(), 0);
}

#[test]
fn a_question_is_open_until_the_reply_and_a_run_that_ends_waiting_is_not_failed() {
    let t = Temp::new();
    let v = t.open();
    let (_, id) = running_task(&v, "Decide");
    assert!(tasks::ask(&v, &id, "  ").is_err());
    tasks::ask(&v, &id, "Which one?").unwrap();
    assert_eq!(tasks::get(&v, &id).unwrap().question, "Which one?");
    assert_eq!(tasks::get(&v, &id).unwrap().status, tasks::RUNNING);
    assert_eq!(tasks::take_reply(&v, &id).unwrap(), None);
    assert!(!tasks::reply(&v, &id, "  ").unwrap());
    assert!(tasks::reply(&v, &id, "The first").unwrap());
    assert!(!tasks::reply(&v, &id, "again").unwrap(), "no open question");
    assert!(tasks::get(&v, &id).unwrap().question.is_empty());
    assert_eq!(tasks::take_reply(&v, &id).unwrap().as_deref(), Some("The first"));
    assert_eq!(tasks::take_reply(&v, &id).unwrap(), None);

    // The run ends while it waits: the attempt says so and the row is not failed.
    tasks::ask(&v, &id, "Still there?").unwrap();
    tasks::finish_attempt(
        &v,
        &id,
        None,
        Err(format!("{}: no reply within the run limit", tasks::WAITING_FOR_INPUT)),
    )
    .unwrap();
    let row = tasks::get(&v, &id).unwrap();
    assert_eq!(row.status, tasks::INTERRUPTED);
    assert_eq!(row.question, "Still there?");
    let a = attempts::list(&v, &id).unwrap();
    assert_eq!(a.last().unwrap().outcome.as_deref(), Some(tasks::WAITING_FOR_INPUT));
    assert!(!tasks::reply(&v, &id, "late").unwrap(), "nobody is waiting");
}
