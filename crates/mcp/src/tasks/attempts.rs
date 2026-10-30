//! A task's attempts: the compact record each run keeps, for review and
//! later evaluation (see the delegation spec, "History").
//!
//! The row's `attempt` property names the current attempt; `attempts` holds
//! the records as JSON, oldest first. A retry is a new attempt whose
//! `parent` is the one before it. Only one attempt is current: a result that
//! arrives from any other attempt goes into that attempt's record and
//! changes nothing else (see [`super::finish_attempt`]).
//!
//! Unknown values stay unknown: `usage`, `model` and `provider` are `None`
//! until the runner learns them. Transcripts stay in BubblePaw's store; the
//! record keeps the brief the worker was given (cut to [`BRIEF_MAX`]) and a
//! short result summary.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vault_core::{PropValue, Vault};

use super::{collection, get, migrate_schema, text, Task};
use crate::jobs::now_ms;
use crate::{err, Result};

/// The row property naming the current attempt.
pub const CURRENT: &str = "attempt";
/// The row property holding the attempts, as JSON.
pub const HISTORY: &str = "attempts";
/// The row property set (ms since the epoch) by a cancel command and
/// cleared by the next attempt. While the status is still `running`, the
/// runner has not acknowledged it.
pub const CANCEL_REQUESTED: &str = "cancel_requested";
/// The row property set (ms since the epoch) when the user asks the runner to
/// send the worker the note's current version; cleared once sent.
pub const UPDATE_REQUESTED: &str = "update_requested";

/// Attempts kept in a row; the oldest go first.
const MAX_ATTEMPTS: usize = 20;
/// Events kept in an attempt.
const MAX_EVENTS: usize = 60;
/// The longest brief kept, in characters.
pub const BRIEF_MAX: usize = 4000;
/// The longest result summary kept, in characters.
pub const RESULT_MAX: usize = 2000;
const EVENT_TEXT_MAX: usize = 300;

/// Event kinds, in the order they normally happen.
pub mod kind {
    pub const REQUESTED: &str = "requested";
    pub const ACCEPTED: &str = "accepted";
    pub const STARTED: &str = "started";
    pub const CHECKPOINT: &str = "checkpoint";
    pub const STEERING_SENT: &str = "steering_sent";
    pub const STEERING_ACKNOWLEDGED: &str = "steering_acknowledged";
    pub const UPDATE_REQUESTED: &str = "update_requested";
    pub const UPDATE_SENT: &str = "update_sent";
    pub const CANCEL_REQUESTED: &str = "cancel_requested";
    pub const CANCEL_ACKNOWLEDGED: &str = "cancel_acknowledged";
    pub const RESULT: &str = "result";
    /// A result that came after the attempt was no longer current, or after
    /// a cancel: kept here, never applied to the task.
    pub const LATE_RESULT: &str = "late_result";
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    /// ms since the epoch.
    pub at: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Attempt {
    pub id: String,
    /// The attempt this one retried.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Who asked for the run: the job's owner, else `user`.
    pub actor: String,
    /// The runner node (a device id or `hub`) and the device that ran it.
    pub node: String,
    pub device: String,
    pub session: String,
    /// The page revision the brief was made from (see [`super::source_revision`]).
    pub source_rev: String,
    /// The prompt the worker was given, cut to [`BRIEF_MAX`] characters.
    pub brief: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// The run's token usage as reported; `None` when unknown.
    pub usage: Option<Value>,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    /// `done`, `failed`, `cancelled` or `interrupted` once it ended.
    pub outcome: Option<String>,
    /// A summary of the result, cut to [`RESULT_MAX`] characters. Kept for
    /// a late result as well.
    pub result: Option<String>,
    /// To-do blocks whose text was sent to the worker as steering.
    pub steered: Vec<String>,
    /// To-do blocks that were under the task when the run began: the user's,
    /// never sent as steering.
    pub baseline: Vec<String>,
    pub events: Vec<Event>,
}

/// What a runner knows once the run starts.
#[derive(Debug, Clone, Default)]
pub struct Started {
    pub session: String,
    pub brief: String,
    pub source_rev: String,
    /// The model id as the engine names it, like `provider/model`.
    pub model: Option<String>,
}

pub(crate) fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s.to_string()
    }
}

fn new_event(kind: &str, text: &str) -> Event {
    Event {
        at: now_ms(),
        kind: kind.into(),
        text: cut(text, EVENT_TEXT_MAX),
    }
}

fn push_event(a: &mut Attempt, e: Event) {
    if a.events.len() >= MAX_EVENTS {
        // Checkpoints are the noisiest; they go first, then the oldest
        // event after the first one.
        let at = a
            .events
            .iter()
            .position(|x| x.kind == kind::CHECKPOINT)
            .unwrap_or(1);
        a.events.remove(at.min(a.events.len() - 1));
    }
    a.events.push(e);
}

/// The row's current attempt id, if it has one.
pub fn current_id(v: &Vault, task_id: &str) -> Result<Option<String>> {
    let row = v.load_page(task_id.to_string())?;
    Ok(Some(text(&row.properties, CURRENT)).filter(|s| !s.is_empty()))
}

/// The row's attempts, oldest first. Damaged JSON is an error: reading it
/// as empty would erase the history on the next save.
pub fn list(v: &Vault, task_id: &str) -> Result<Vec<Attempt>> {
    let row = v.load_page(task_id.to_string())?;
    let raw = text(&row.properties, HISTORY);
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str(&raw) {
        Ok(list) => Ok(list),
        Err(_) => err("the attempt history is damaged"),
    }
}

fn save(v: &Vault, task_id: &str, mut all: Vec<Attempt>) -> Result<()> {
    if all.len() > MAX_ATTEMPTS {
        all.drain(..all.len() - MAX_ATTEMPTS);
    }
    // A database from before these properties gets them now: set_property
    // refuses a key its collection's schema lacks.
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    v.set_property(
        task_id.into(),
        HISTORY.into(),
        PropValue::Text(serde_json::to_string(&all)?),
    )?;
    Ok(())
}

/// Changes attempt `attempt` (the current one when None) with `f`. A missing
/// attempt is an error and nothing is written.
pub fn update(
    v: &Vault,
    task_id: &str,
    attempt: Option<&str>,
    f: impl FnOnce(&mut Attempt),
) -> Result<()> {
    let id = match attempt {
        Some(a) => a.to_string(),
        None => match current_id(v, task_id)? {
            Some(a) => a,
            None => return err("the task has no attempt"),
        },
    };
    let mut all = list(v, task_id)?;
    let Some(a) = all.iter_mut().find(|a| a.id == id) else {
        return err("no such attempt");
    };
    f(a);
    save(v, task_id, all)
}

/// Starts a new attempt for `t`, which becomes the current one. The previous
/// current attempt is its parent. A cancel request of the earlier attempt is
/// cleared. Returns the new attempt's id.
pub fn begin(v: &Vault, t: &Task, actor: &str) -> Result<String> {
    let mut all = list(v, &t.id)?;
    let parent = current_id(v, &t.id)?;
    let id = vault_core::new_id();
    let now = now_ms();
    all.push(Attempt {
        id: id.clone(),
        parent,
        actor: if actor.trim().is_empty() {
            "user".into()
        } else {
            actor.into()
        },
        node: t.node.clone(),
        device: t.device.clone(),
        started_ms: now,
        events: vec![
            Event {
                at: t.created_ms.max(0),
                kind: kind::REQUESTED.into(),
                text: String::new(),
            },
            Event {
                at: now,
                kind: kind::ACCEPTED.into(),
                text: String::new(),
            },
        ],
        ..Attempt::default()
    });
    save(v, &t.id, all)?;
    v.set_property(t.id.clone(), CURRENT.into(), PropValue::Text(id.clone()))?;
    v.remove_property(t.id.clone(), CANCEL_REQUESTED.into())?;
    Ok(id)
}

/// The current attempt when it is still going, else a new one: for a runner
/// that was handed a row without claiming it.
pub fn ensure(v: &Vault, t: &Task, actor: &str) -> Result<String> {
    if let Some(id) = current_id(v, &t.id)? {
        let open = list(v, &t.id)?
            .iter()
            .any(|a| a.id == id && a.ended_ms.is_none() && a.session.is_empty());
        if open {
            return Ok(id);
        }
    }
    begin(v, t, actor)
}

/// Records what the runner knows once the run starts.
pub fn started(v: &Vault, task_id: &str, attempt: &str, s: Started) -> Result<()> {
    update(v, task_id, Some(attempt), |a| {
        a.session = s.session;
        a.brief = cut(&s.brief, BRIEF_MAX);
        a.source_rev = s.source_rev;
        if let Some(model) = s.model.filter(|m| !m.trim().is_empty()) {
            a.provider = model.split_once('/').map(|(p, _)| p.to_string());
            a.model = Some(model);
        }
        push_event(a, new_event(kind::STARTED, ""));
    })
}

/// Appends an event to attempt `attempt` (the current one when None).
pub fn event(
    v: &Vault,
    task_id: &str,
    attempt: Option<&str>,
    event_kind: &str,
    event_text: &str,
) -> Result<()> {
    update(v, task_id, attempt, |a| {
        push_event(a, new_event(event_kind, event_text))
    })
}

/// Records the run's usage as reported.
pub fn set_usage(v: &Vault, task_id: &str, attempt: &str, usage: Value) -> Result<()> {
    update(v, task_id, Some(attempt), |a| a.usage = Some(usage))
}

/// Closes attempt `attempt` with `outcome` and a result summary. An attempt
/// that already ended keeps its outcome; a later result is a late result.
pub(crate) fn close(
    v: &Vault,
    task_id: &str,
    attempt: Option<&str>,
    outcome: &str,
    result: Option<&str>,
    late: bool,
) -> Result<()> {
    update(v, task_id, attempt, |a| {
        let summary = result.map(|r| cut(r, RESULT_MAX)).filter(|r| !r.is_empty());
        if late || a.ended_ms.is_some() {
            if summary.is_some() {
                a.result = summary;
            }
            push_event(
                a,
                new_event(
                    kind::LATE_RESULT,
                    &format!("{outcome} after the attempt ended"),
                ),
            );
            return;
        }
        a.ended_ms = Some(now_ms());
        a.outcome = Some(outcome.into());
        a.result = summary;
        push_event(a, new_event(kind::RESULT, outcome));
    })
}

/// Marks the block as sent to the worker as steering.
pub fn mark_steered(v: &Vault, task_id: &str, attempt: &str, block: &str) -> Result<()> {
    update(v, task_id, Some(attempt), |a| {
        if !a.steered.iter().any(|b| b == block) {
            a.steered.push(block.to_string());
        }
        push_event(a, new_event(kind::STEERING_SENT, ""));
    })
}

/// The cancel request time (ms), if the task has an unacknowledged cancel.
pub fn cancel_requested_at(v: &Vault, task_id: &str) -> Result<Option<i64>> {
    let row = v.load_page(task_id.to_string())?;
    Ok(text(&row.properties, CANCEL_REQUESTED).parse().ok())
}

/// Records a cancel command on a running task: the status stays `running`
/// until the runner acknowledges. Idempotent.
pub fn request_cancel(v: &Vault, t: &Task) -> Result<()> {
    if cancel_requested_at(v, &t.id)?.is_some() {
        return Ok(());
    }
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    v.set_property(
        t.id.clone(),
        CANCEL_REQUESTED.into(),
        PropValue::Text(now_ms().to_string()),
    )?;
    // The history is best effort: the cancel itself is already recorded.
    let _ = event(v, &t.id, None, kind::CANCEL_REQUESTED, "");
    Ok(())
}

/// Whether the task's cancel was asked for and not yet acknowledged, or it
/// is already cancelled.
pub fn is_cancelling(v: &Vault, task_id: &str) -> bool {
    get(v, task_id).is_ok_and(|t| t.status == super::CANCELLED)
        || cancel_requested_at(v, task_id).is_ok_and(|c| c.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_stay_capped_and_keep_the_first() {
        let mut a = Attempt::default();
        push_event(&mut a, new_event(kind::REQUESTED, ""));
        for i in 0..(MAX_EVENTS * 2) {
            push_event(&mut a, new_event(kind::CHECKPOINT, &i.to_string()));
        }
        assert_eq!(a.events.len(), MAX_EVENTS);
        assert_eq!(a.events[0].kind, kind::REQUESTED);
        assert_eq!(
            a.events.last().unwrap().text,
            (MAX_EVENTS * 2 - 1).to_string()
        );
    }

    #[test]
    fn cut_keeps_short_text_and_marks_long() {
        assert_eq!(cut(" hi ", 10), "hi");
        assert_eq!(cut("abcdef", 3), "abc…");
    }
}
