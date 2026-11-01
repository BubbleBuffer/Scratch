//! Consume durable todo events within the lifetime of a delegated run.

use std::sync::Arc;

use bubblepaw_sdk::events::{EventKind, HarnessEvent};
use bubblepaw_sdk::harness::{Harness};
use bubblepaw_sdk::store::{EVENTS_AFTER_LIMIT};
use tokio::sync::broadcast::error::RecvError;
use vault_core::Vault;
use vault_mcp::tasks::{self, SubtaskItem};

pub(super) struct TodoMirror {
    vault: Arc<Vault>,
    task: String,
    session: String,
    pub run_id: String,
    after: u64,
    enabled: bool,
}

impl TodoMirror {
    pub fn new(vault: Arc<Vault>, task: &str, session: &str, harness: &Harness) -> Self {
        let enabled = match tasks::begin_subtasks(&vault, task, session) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("task {task}: could not prepare subtasks: {error}");
                false
            }
        };
        Self {
            vault,
            task: task.into(),
            session: session.into(),
            run_id: String::new(),
            after: harness.last_sequence(),
            enabled,
        }
    }

    /// Notifications only wake the reader. Reading the durable log in order
    /// also recovers every replacement after a broadcast receiver lags.
    pub async fn on_event(
        &mut self,
        event: Result<HarnessEvent, RecvError>,
        harness: &Harness,
    ) -> bool {
        match event {
            Ok(event)
                if event.session_id == self.session
                    && event.run_id == self.run_id
                    && matches!(&event.kind, EventKind::HostStateChanged { key, .. } if key == "todo") =>
            {
                self.drain(harness).await;
            }
            Err(RecvError::Lagged(_)) => self.drain(harness).await,
            Err(RecvError::Closed) => return false,
            _ => {}
        }
        true
    }

    pub async fn drain(&mut self, harness: &Harness) {
        if !self.enabled {
            return;
        }
        loop {
            let events =
                match harness.events_after(self.after, Some(&self.session), EVENTS_AFTER_LIMIT) {
                    Ok(events) => events,
                    Err(error) => {
                        eprintln!("task {}: could not read subtask events: {error}", self.task);
                        return;
                    }
                };
            let full = events.len() == EVENTS_AFTER_LIMIT;
            for event in events {
                self.after = self.after.max(event.sequence);
                if event.run_id != self.run_id {
                    continue;
                }
                let EventKind::HostStateChanged { key, value } = event.kind else {
                    continue;
                };
                if key != "todo" {
                    continue;
                }
                let items: Vec<SubtaskItem> = match serde_json::from_value(value) {
                    Ok(items) => items,
                    Err(error) => {
                        eprintln!("task {}: invalid subtask event: {error}", self.task);
                        continue;
                    }
                };
                let (vault, task, session) =
                    (self.vault.clone(), self.task.clone(), self.session.clone());
                let result = tokio::task::spawn_blocking(move || {
                    tasks::mirror_subtasks(&vault, &task, &session, &items)
                })
                .await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        eprintln!("task {}: could not mirror subtasks: {error}", self.task);
                    }
                    Err(error) => {
                        eprintln!("task {}: subtask writer stopped: {error}", self.task);
                    }
                }
            }
            if !full {
                return;
            }
        }
    }
}
