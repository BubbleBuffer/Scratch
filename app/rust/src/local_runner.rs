//! The schedule runner on this computer. `NotesVault` owns it: when the vault is
//! opened with a data folder on desktop, its timer runs whether or not an agent is
//! open or signed in. Other targets have no runner, and every control answers
//! "not available on this platform".
//!
//! The data folder holds `agent/schedules.json` (the runner's state). It is never
//! derived from the vault folder, since vaults can live anywhere.

use std::sync::Arc;

use vault_core::Vault;

use crate::api::agent::{AgentScheduleRun, AgentScheduleState};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::path::Path;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::sync::Mutex;

#[cfg(any(target_os = "windows", target_os = "linux"))]
use vault_agent::schedules::{RunningScheduler, Scheduler};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use crate::convert::err;

/// The local runner of one open vault.
#[allow(dead_code)]
pub(crate) enum LocalRunner {
    /// Opened without a data folder: this vault has no local runner.
    Unconfigured,
    /// The runner did not start (for example, an unreadable `schedules.json`).
    /// The vault and any agent stay usable; local controls report this message.
    Failed(String),
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    Ready(Ready),
}

/// A started-or-stopped desktop runner. Its timer, when running, is the only one
/// for this vault: `timer` is `Some` exactly while the timer thread runs.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) struct Ready {
    vault: Arc<Vault>,
    scheduler: Arc<Scheduler>,
    /// Guards starts and stops. Dropping the `RunningScheduler` joins its thread,
    /// so an in-flight tick finishes first.
    timer: Mutex<Option<RunningScheduler>>,
    /// Timer starts so far; the tests use it to show there is one timer.
    #[cfg(test)]
    starts: std::sync::atomic::AtomicUsize,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
const UNCONFIGURED: &str =
    "the local schedule runner is not configured: this vault was opened without a data folder";
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
const UNAVAILABLE: &str = "not available on this platform";

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn e(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
impl LocalRunner {
    /// Opens the runner over `vault`. With a data folder, the runner is off but its
    /// existing node is marked off, and a startup failure is kept for the controls.
    pub(crate) fn open(vault: Arc<Vault>, data_dir: Option<String>) -> Self {
        let Some(data_dir) = data_dir else {
            return Self::Unconfigured;
        };
        if data_dir.trim().is_empty() {
            return Self::Failed("the local schedule runner needs a non-empty data folder".into());
        }
        if let Err(error) = vault_agent::projects::publish_at(&vault, Path::new(&data_dir)) {
            eprintln!("runner projects could not be published: {error:#}");
        }
        match Scheduler::open(
            vault.clone(),
            Path::new(&data_dir),
            "desktop",
            "Desktop",
            false,
        ) {
            Ok(scheduler) => {
                if let Err(error) = scheduler.tick() {
                    eprintln!("schedule startup tick failed: {error:#}");
                }
                Self::Ready(Ready {
                    vault,
                    scheduler,
                    timer: Mutex::new(None),
                    #[cfg(test)]
                    starts: std::sync::atomic::AtomicUsize::new(0),
                })
            }
            Err(error) => Self::Failed(format!("schedule runner unavailable: {error:#}")),
        }
    }

    fn ready(&self) -> Result<&Ready, String> {
        match self {
            Self::Ready(ready) => Ok(ready),
            Self::Failed(error) => Err(error.clone()),
            Self::Unconfigured => Err(UNCONFIGURED.into()),
        }
    }

    /// The record of this device once it is a schedule registration. A device
    /// presence record, published on open, is not one. A registration stays
    /// identifiable even if local state cannot load.
    pub(crate) fn node_id(&self, vault: &Vault) -> Option<String> {
        let id = vault_core::node_record_id(&vault.device_id());
        vault_agent::schedules::is_registered(vault, &id)
            .ok()?
            .then_some(id)
    }

    /// Turns the runner on or off. On: starts the timer if it is not running, and
    /// its first tick runs at once. Off: stops the timer, then publishes off.
    pub(crate) fn set_available(&self, available: bool) -> Result<(), String> {
        let ready = self.ready()?;
        let mut timer = ready.timer.lock().unwrap_or_else(|p| p.into_inner());
        if !available {
            drop(timer.take());
            return ready.scheduler.set_available(false).map_err(e);
        }
        ready.scheduler.set_available(true).map_err(e)?;
        if timer.is_none() {
            let started = match RunningScheduler::start(ready.scheduler.clone()) {
                Ok(started) => started,
                Err(error) => {
                    let _ = ready.scheduler.set_available(false);
                    return Err(e(error));
                }
            };
            #[cfg(test)]
            ready
                .starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *timer = Some(started);
        }
        Ok(())
    }

    /// The schedule's scheduler, when `page_id` runs on this node. The vault
    /// enforces this for every local call, state included: the engine reports
    /// another runner's schedule as refused instead of failing.
    fn owned(&self, page_id: &str) -> Result<&Scheduler, String> {
        let ready = self.ready()?;
        let raw = ready
            .vault
            .type_data(page_id.into(), "schedule".into())
            .map_err(err)?;
        let raw: serde_json::Value = serde_json::from_str(&raw).map_err(err)?;
        if raw["runner"].as_str() != Some(ready.scheduler.node_id()) {
            return Err("this node is not the schedule runner".into());
        }
        Ok(ready.scheduler.as_ref())
    }

    pub(crate) fn state(&self, page_id: &str) -> Result<AgentScheduleState, String> {
        self.owned(page_id)?
            .state(page_id)
            .map(Into::into)
            .map_err(e)
    }

    pub(crate) fn accept(&self, page_id: &str, hash: &str) -> Result<AgentScheduleState, String> {
        self.owned(page_id)?
            .accept(page_id, hash)
            .map(Into::into)
            .map_err(e)
    }

    pub(crate) fn pause(&self, page_id: &str, paused: bool) -> Result<AgentScheduleState, String> {
        self.owned(page_id)?
            .pause(page_id, paused)
            .map(Into::into)
            .map_err(e)
    }

    pub(crate) fn run_now(&self, page_id: &str) -> Result<AgentScheduleRun, String> {
        self.owned(page_id)?
            .run_now(page_id)
            .map(Into::into)
            .map_err(e)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl LocalRunner {
    pub(crate) fn open(vault: Arc<Vault>, data_dir: Option<String>) -> Self {
        let _ = (vault, data_dir);
        Self::Unconfigured
    }

    pub(crate) fn node_id(&self, _vault: &Vault) -> Option<String> {
        None
    }

    pub(crate) fn set_available(&self, available: bool) -> Result<(), String> {
        let _ = available;
        Err(UNAVAILABLE.into())
    }

    pub(crate) fn state(&self, page_id: &str) -> Result<AgentScheduleState, String> {
        let _ = page_id;
        Err(UNAVAILABLE.into())
    }

    pub(crate) fn accept(&self, page_id: &str, hash: &str) -> Result<AgentScheduleState, String> {
        let _ = (page_id, hash);
        Err(UNAVAILABLE.into())
    }

    pub(crate) fn pause(&self, page_id: &str, paused: bool) -> Result<AgentScheduleState, String> {
        let _ = (page_id, paused);
        Err(UNAVAILABLE.into())
    }

    pub(crate) fn run_now(&self, page_id: &str) -> Result<AgentScheduleRun, String> {
        let _ = page_id;
        Err(UNAVAILABLE.into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::api::vault::NotesVault;
    use crate::schedule_format::{format, portable};

    /// A temp folder removed when the test ends.
    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("notes_bridge_runner_{}", vault_core::new_id())))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn ready(vault: &NotesVault) -> &super::Ready {
        match &vault.runner {
            super::LocalRunner::Ready(ready) => ready,
            _ => panic!("the vault has no started local runner"),
        }
    }

    /// The `node` fields of the record `node` on `vault`.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn node_data(vault: &NotesVault, node: &str) -> serde_json::Value {
        serde_json::from_str(&vault.type_data(node.into(), "node".into()).unwrap()).unwrap()
    }

    fn at(text: &str) -> jiff::Timestamp {
        text.parse().unwrap()
    }

    /// A one-off schedule on `runner`, due 2026-10-05 08:00 Berlin (06:00Z).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn store_schedule(vault: &NotesVault, runner: &str, parent: &str, template: &str) -> String {
        let page = vault
            .create_page(None, "Trip".into(), Some("schedule".into()))
            .unwrap();
        let raw = json!({
            "version": 1,
            "trigger": {"at": "2026-10-05T08:00", "tz": "Europe/Berlin"},
            "runner": runner,
            "tools": ["create_page"],
            "steps": [{
                "id": "note", "do": "note.from_template", "template": template,
                "parent": parent, "title": "Trip day"
            }],
        });
        for (key, value) in raw.as_object().unwrap() {
            vault
                .type_data_set(
                    page.clone(),
                    "schedule".into(),
                    vec![key.clone()],
                    value.to_string(),
                )
                .unwrap();
        }
        page
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn a_due_schedule_runs_from_the_vault_after_the_agent_is_closed() {
        let root = TempRoot::new();
        let data = root.0.join("state");
        let vault = NotesVault::open(
            root.0.join("vault").to_string_lossy().into_owned(),
            Some(data.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert_eq!(vault.local_node_id(), None);
        let node = ready(&vault).scheduler.node_id().to_owned();
        let scheduler = ready(&vault).scheduler.clone();
        scheduler
            .set_test_time(Some("2026-10-01T00:00:00Z"))
            .unwrap();

        let parent = vault.create_page(None, "Journal".into(), None).unwrap();
        let template = vault
            .create_template(parent.clone(), "Trip template".into())
            .unwrap();
        let page = store_schedule(&vault, &node, &parent, &template);

        // The runner starts off, so the schedule is not yet accepted-and-running.
        let before = vault.local_schedule_state(page.clone()).unwrap();
        assert!(!before.accepted);
        assert_eq!(before.runner_state, "runner_off");
        let accepted = vault
            .local_schedule_accept(page.clone(), before.current_hash.unwrap())
            .unwrap();
        assert!(accepted.accepted && accepted.hash_matches);
        assert_eq!(accepted.runner_state, "runner_off");
        assert!(accepted.last_runs.is_empty());

        let agent = crate::api::agent::NotesAgent::open(
            &vault,
            data.to_string_lossy().into_owned(),
            crate::api::agent::AgentSettings {
                default_model: String::new(),
                bun_path: None,
                claude_code_dir: None,
                permission_mode: crate::api::agent::AgentPermissionMode::Bypass,
                desktop_tools: false,
                desktop_root: String::new(),
                test_tools: false,
            },
        )
        .unwrap();
        drop(agent);

        // Half an hour after the occurrence, the app turns the runner on: the
        // immediate tick creates the note, and the status records it.
        scheduler
            .set_test_time(Some("2026-10-05T06:30:00Z"))
            .unwrap();
        vault.local_runner_set_available(true).unwrap();
        assert_eq!(vault.local_node_id().as_deref(), Some(node.as_str()));
        let expected = vault_core::schedule_occurrence_id(&page, "2026-10-05T06:00:00Z");
        // The template is also a child of the parent, so look for the note by id.
        let children = vault.list_children(Some(parent), 0, 10).unwrap();
        let note = children
            .iter()
            .find(|row| row.id == expected)
            .expect("the note was created");
        assert_eq!(note.title, "Trip day");

        let state = vault.local_schedule_state(page.clone()).unwrap();
        assert_eq!(state.runner_state, "ok");
        assert_eq!(state.last_runs.len(), 1);
        assert_eq!(state.last_runs[0].result, "created");
        assert_eq!(state.last_runs[0].page.as_deref(), Some(expected.as_str()));
        let status: serde_json::Value =
            serde_json::from_str(&vault.type_data(page, "schedule_status".into()).unwrap())
                .unwrap();
        assert_eq!(status["runner_state"], "ok");
        assert_eq!(status["last_run"]["result"], "created");

        vault.local_runner_set_available(false).unwrap();
        drop(vault);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn repeated_availability_keeps_one_timer_and_closing_the_vault_stops_it() {
        let root = TempRoot::new();
        let data = root.0.join("state");
        let data_str = data.to_string_lossy().into_owned();
        let vault = NotesVault::open(
            root.0.join("vault").to_string_lossy().into_owned(),
            Some(data_str.clone()),
        )
        .unwrap();
        let scheduler_weak = Arc::downgrade(&ready(&vault).scheduler);

        vault.local_runner_set_available(true).unwrap();
        vault.local_runner_set_available(true).unwrap();
        vault.local_runner_set_available(true).unwrap();
        assert_eq!(
            ready(&vault)
                .starts
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        vault.local_runner_set_available(false).unwrap();
        vault.local_runner_set_available(false).unwrap();
        assert!(ready(&vault).timer.lock().unwrap().is_none());
        vault.local_runner_set_available(true).unwrap();
        assert_eq!(
            ready(&vault)
                .starts
                .load(std::sync::atomic::Ordering::SeqCst),
            2
        );

        // An agent open on the same folder does not keep the timer alive.
        let agent = crate::api::agent::NotesAgent::open(
            &vault,
            data_str,
            crate::api::agent::AgentSettings {
                default_model: String::new(),
                bun_path: None,
                claude_code_dir: None,
                permission_mode: crate::api::agent::AgentPermissionMode::Bypass,
                desktop_tools: false,
                desktop_root: String::new(),
                test_tools: false,
            },
        )
        .unwrap();
        drop(vault);
        assert!(
            scheduler_weak.upgrade().is_none(),
            "the timer still holds the scheduler"
        );
        assert_eq!(agent.host_info().unwrap().kind, "desktop");
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn a_corrupt_schedule_state_leaves_the_vault_and_agent_usable() {
        let root = TempRoot::new();
        let data = root.0.join("state");
        std::fs::create_dir_all(data.join("agent")).unwrap();
        std::fs::write(data.join("agent/schedules.json"), "broken state").unwrap();
        let data_str = data.to_string_lossy().into_owned();
        let vault = NotesVault::open(
            root.0.join("vault").to_string_lossy().into_owned(),
            Some(data_str.clone()),
        )
        .unwrap();
        let page = vault.create_page(None, "Still works".into(), None).unwrap();
        assert!(vault.load_page(page).is_ok());

        assert_eq!(vault.local_node_id(), None);
        let device = vault.inner.device_id();
        let node = vault
            .inner
            .write_node_record(
                device.clone(),
                json!({
                    "device_id": device, "name": "Desktop", "kind": "desktop",
                    "available": false, "app_version": "test", "schedule_format": 1,
                    "steps": ["note.from_template"], "last_seen": "2026-10-01T00:00:00Z"
                })
                .to_string(),
            )
            .unwrap();
        assert_eq!(vault.local_node_id(), Some(node));
        let error = vault.local_runner_set_available(true).unwrap_err();
        assert!(
            error.contains("schedule runner unavailable") && error.contains("schedules.json"),
            "{error}"
        );
        let error = vault.local_schedule_state("any".into()).unwrap_err();
        assert!(error.contains("schedules.json"), "{error}");

        let agent = crate::api::agent::NotesAgent::open(
            &vault,
            data_str,
            crate::api::agent::AgentSettings {
                default_model: String::new(),
                bun_path: None,
                claude_code_dir: None,
                permission_mode: crate::api::agent::AgentPermissionMode::Bypass,
                desktop_tools: false,
                desktop_root: String::new(),
                test_tools: false,
            },
        )
        .unwrap();
        assert_eq!(agent.host_info().unwrap().kind, "desktop");
        assert!(agent.sessions().unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(data.join("agent/schedules.json")).unwrap(),
            "broken state"
        );
        drop(agent);
        drop(vault);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn runner_off_on_a_fresh_vault_adds_no_schedule_registration() {
        let root = TempRoot::new();
        let vault = NotesVault::open(
            root.0.join("vault").to_string_lossy().into_owned(),
            Some(root.0.join("state").to_string_lossy().into_owned()),
        )
        .unwrap();
        // App open publishes this device's presence record, and no other page.
        let node = vault_core::node_record_id(&vault.device_id());
        assert_eq!(vault.page_count().unwrap(), 1);
        assert_eq!(vault.local_node_id(), None);

        vault.local_runner_set_available(false).unwrap();
        // A refresh seven hours later must not register an off desktop.
        ready(&vault)
            .scheduler
            .tick_at(
                jiff::Timestamp::now()
                    .checked_add(jiff::SignedDuration::from_secs(7 * 60 * 60))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(vault.page_count().unwrap(), 1);
        assert_eq!(vault.local_node_id(), None);
        let record = node_data(&vault, &node);
        assert_eq!(record["kind"], "desktop");
        assert!(record.get("available").is_none());
        assert!(record.get("schedule_format").is_none());
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn device_presence_becomes_a_schedule_registration_only_when_enabled() {
        let root = TempRoot::new();
        let vault_path = root.0.join("vault").to_string_lossy().into_owned();
        let state = root.0.join("state").to_string_lossy().into_owned();
        let vault = NotesVault::open(vault_path.clone(), Some(state.clone())).unwrap();
        let node = vault_core::node_record_id(&vault.device_id());
        assert_eq!(vault.local_node_id(), None);
        vault
            .inner
            .rename_device(vault.device_id(), "Studio".into())
            .unwrap();

        vault.local_runner_set_available(true).unwrap();
        assert_eq!(vault.local_node_id().as_deref(), Some(node.as_str()));
        let on = node_data(&vault, &node);
        assert_eq!(on["name"], "Studio");
        assert_eq!(on["kind"], "desktop");
        assert_eq!(on["available"], true);
        assert_eq!(on["schedule_format"], 1);

        // Turning it off keeps the registration, updated and still named.
        vault.local_runner_set_available(false).unwrap();
        assert_eq!(vault.local_node_id().as_deref(), Some(node.as_str()));
        let off = node_data(&vault, &node);
        assert_eq!(off["name"], "Studio");
        assert_eq!(off["available"], false);
        drop(vault);

        // Reopening keeps the registration too, and still does not turn it on.
        let vault = NotesVault::open(vault_path, Some(state)).unwrap();
        assert_eq!(vault.local_node_id().as_deref(), Some(node.as_str()));
        let reopened = node_data(&vault, &node);
        assert_eq!(reopened["name"], "Studio");
        assert_eq!(reopened["available"], false);
    }

    #[test]
    fn a_vault_without_a_data_folder_has_no_local_runner() {
        let root = TempRoot::new();
        let vault =
            NotesVault::open(root.0.join("vault").to_string_lossy().into_owned(), None).unwrap();
        assert_eq!(vault.local_node_id(), None);
        assert!(vault.local_runner_set_available(true).is_err());
        assert!(vault.local_schedule_pause("any".into(), true).is_err());
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn local_controls_reject_another_nodes_schedule() {
        let root = TempRoot::new();
        let vault = NotesVault::open(
            root.0.join("vault").to_string_lossy().into_owned(),
            Some(root.0.join("state").to_string_lossy().into_owned()),
        )
        .unwrap();
        let parent = vault.create_page(None, "Journal".into(), None).unwrap();
        let template = vault
            .create_template(parent.clone(), "Template".into())
            .unwrap();
        let page = store_schedule(&vault, "another-node", &parent, &template);
        assert!(vault
            .local_schedule_state(page.clone())
            .unwrap_err()
            .contains("not the schedule runner"));
        assert!(vault
            .local_schedule_accept(page.clone(), "unused".into())
            .is_err());
        assert!(vault.local_schedule_pause(page.clone(), true).is_err());
        assert!(vault.local_schedule_run_now(page).is_err());
    }

    #[test]
    fn preview_works_without_a_data_folder_and_matches_the_portable_copy() {
        let root = TempRoot::new();
        let vault =
            NotesVault::open(root.0.join("vault").to_string_lossy().into_owned(), None).unwrap();
        let text = json!({
            "version": 1,
            "trigger": {"at": "2099-01-05T08:00", "tz": "Europe/Berlin", "rrule": "FREQ=DAILY;COUNT=5"},
            "runner": "some-node",
            "tools": ["create_page"],
            "steps": [{
                "id": "note", "do": "note.from_template", "template": "t",
                "parent": "p", "title": "Day {{date}}"
            }],
        })
        .to_string();

        let preview = vault.schedule_preview(text.clone());
        assert!(preview.valid, "{:?}", preview.reason);
        assert_eq!(preview.reason, None);
        assert_eq!(preview.occurrences.len(), 3);
        assert_eq!(preview.occurrences[0].occurrence, "2099-01-05T07:00:00Z");
        assert!(preview.current_hash.is_some());

        // The bridge's copy and vault_agent's copy of format.rs agree on the same input.
        let now = at("2026-10-01T00:00:00Z");
        assert_eq!(portable::preview(&text, now), format::preview(&text, now));
        let invalid = portable::preview("not json", now);
        assert!(!invalid.valid);
        assert!(invalid.reason.unwrap().starts_with("invalid JSON"));
    }
}
