//! Provider-independent, node-local schedule execution over a synced vault.
pub mod format;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use jiff::Timestamp;
use notes_agent_wire::{SchedulePreview, ScheduleRun, ScheduleState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vault_core::{MarkKind, Vault};

use format::{definition_hash, Schedule};

pub const TICK_SECONDS: u64 = 30;
const RECORD_REFRESH_SECONDS: i64 = 6 * 60 * 60;

#[derive(Clone, Default, Serialize, Deserialize)]
struct SavedSchedule {
    accepted_hash: Option<String>,
    accepted_at: Option<String>,
    paused: bool,
    /// High-water mark includes skipped occurrences, even after log eviction.
    last_occurrence: Option<String>,
    runs: Vec<ScheduleRun>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Saved {
    schedules: BTreeMap<String, SavedSchedule>,
}

struct Local {
    saved: Saved,
    available: bool,
    record_at: Timestamp,
}

pub struct Scheduler {
    vault: Arc<Vault>,
    search_spec: vault_core::SearchSpec,
    file: PathBuf,
    node_id: String,
    kind: String,
    name: String,
    local: Mutex<Local>,
    clock: Mutex<Option<Timestamp>>,
}

/// The host owns this guard; dropping it wakes and joins its timer thread.
pub struct RunningScheduler {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl RunningScheduler {
    pub fn start(scheduler: Arc<Scheduler>) -> Result<Self> {
        // Complete startup before handing the runner to its host. In particular,
        // a test clock cannot race a startup tick with a captured system time.
        if let Err(error) = scheduler.tick() {
            eprintln!("schedule startup tick failed: {error:#}");
        }
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = stop.clone();
        let thread = std::thread::Builder::new()
            .name("schedules".into())
            .spawn(move || loop {
                let guard = signal.0.lock().unwrap();
                let (guard, _) = signal
                    .1
                    .wait_timeout_while(guard, Duration::from_secs(TICK_SECONDS), |stopped| {
                        !*stopped
                    })
                    .unwrap();
                if *guard {
                    break;
                }
                drop(guard);
                if let Err(error) = scheduler.tick() {
                    eprintln!("schedule tick failed: {error:#}");
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for RunningScheduler {
    fn drop(&mut self) {
        *self.stop.0.lock().unwrap() = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Scheduler {
    /// `data_dir` is the host data directory; state lives beside agent.sqlite.
    pub fn open(
        vault: Arc<Vault>,
        data_dir: &Path,
        kind: &str,
        name: &str,
        available: bool,
    ) -> Result<Arc<Self>> {
        ensure!(matches!(kind, "hub" | "desktop"), "unsupported runner kind");
        let file = data_dir.join("agent/schedules.json");
        std::fs::create_dir_all(file.parent().unwrap())?;
        let saved = if file.exists() {
            serde_json::from_slice(&std::fs::read(&file)?)
                .with_context(|| format!("read {}", file.display()))?
        } else {
            Saved::default()
        };
        let now = Timestamp::now();
        let runner = Arc::new(Self {
            search_spec: vault.parse_search("type:schedule".into())?.spec,
            node_id: vault_core::node_record_id(&vault.device_id()),
            vault,
            file,
            kind: kind.into(),
            name: name.into(),
            local: Mutex::new(Local {
                saved,
                available,
                record_at: now,
            }),
            clock: Mutex::new(None),
        });
        runner.write_record(available, now)?;
        Ok(runner)
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    fn now(&self) -> Timestamp {
        self.clock.lock().unwrap().unwrap_or_else(Timestamp::now)
    }

    /// Deterministic host integration tests, without changing the vault clock.
    #[doc(hidden)]
    pub fn set_test_time(&self, instant: Option<&str>) -> Result<()> {
        *self.clock.lock().unwrap() = instant.map(str::parse).transpose()?;
        Ok(())
    }

    fn save(&self, saved: &Saved) -> Result<()> {
        let temporary = self.file.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(saved)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &self.file)
            .with_context(|| format!("save {}", self.file.display()))?;
        Ok(())
    }

    fn write_record(&self, available: bool, now: Timestamp) -> Result<()> {
        // An off desktop may update its existing record, but must not publish
        // itself (or create the hidden Nodes collection) until opted in.
        if self.kind == "desktop" && !available {
            match self.vault.load_page(self.node_id.clone()) {
                Err(vault_core::VaultError::NotFound(_)) => return Ok(()),
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
        }
        self.vault.write_node_record(
            self.vault.device_id(),
            json!({
                "device_id": self.vault.device_id(), "name": self.name,
                "kind": self.kind, "available": available,
                "app_version": env!("CARGO_PKG_VERSION"), "schedule_format": 1,
                "steps": ["note.from_template"], "last_seen": now.to_string(),
            })
            .to_string(),
        )?;
        Ok(())
    }

    fn raw(&self, page: &str) -> Result<Value> {
        let value = self.vault.type_data(page.into(), "schedule".into())?;
        Ok(serde_json::from_str(&value)?)
    }

    fn view(&self, page: &str, raw: &Value, local: &Local, now: Timestamp) -> ScheduleState {
        let parsed = Schedule::parse(raw.clone());
        let saved = local.saved.schedules.get(page).cloned().unwrap_or_default();
        let current_hash = definition_hash(raw).ok();
        let hash_matches = saved
            .accepted_hash
            .as_ref()
            .is_some_and(|h| Some(h) == current_hash.as_ref());
        let mut refusal_reason = parsed.as_ref().err().map(|e| e.to_string());
        let wrong_runner = raw.get("runner").and_then(Value::as_str) != Some(&self.node_id);
        let runner_state = if wrong_runner {
            refusal_reason = Some("this node is not the schedule runner".into());
            "refused"
        } else if !local.available {
            "runner_off"
        } else if parsed.is_err() {
            "refused"
        } else if saved.paused {
            "paused"
        } else if !hash_matches {
            "needs_acceptance"
        } else {
            "ok"
        };
        let next = parsed
            .as_ref()
            .ok()
            .and_then(|s| s.next(now, 1).ok())
            .and_then(|v| v.first().map(ToString::to_string));
        ScheduleState {
            page_id: page.into(),
            node_id: self.node_id.clone(),
            current_hash,
            accepted: saved.accepted_hash.is_some(),
            hash_matches,
            paused: saved.paused,
            accepted_at: saved.accepted_at,
            last_runs: saved.runs,
            next_run: next,
            refusal_reason,
            runner_state: runner_state.into(),
        }
    }

    pub fn state(&self, page: &str) -> Result<ScheduleState> {
        let local = self.local.lock().unwrap();
        self.require_schedule(page)?;
        Ok(self.view(page, &self.raw(page)?, &local, self.now()))
    }

    pub fn preview(&self, json_text: &str) -> Result<SchedulePreview> {
        Ok(format::preview(json_text, self.now()))
    }

    pub fn accept(&self, page: &str, hash: &str) -> Result<ScheduleState> {
        let mut local = self.local.lock().unwrap();
        self.require_schedule(page)?;
        let raw = self.raw(page)?;
        let schedule = Schedule::parse(raw.clone())?;
        ensure!(
            schedule.runner == self.node_id,
            "this node is not the schedule runner"
        );
        ensure!(
            schedule.hash() == hash,
            "schedule changed; acceptance hash does not match"
        );
        let mut saved = local.saved.clone();
        let entry = saved.schedules.entry(page.into()).or_default();
        if entry.accepted_hash.as_deref() != Some(hash) || entry.accepted_at.is_none() {
            // Retrying the same acceptance must not move its catch-up boundary.
            entry.accepted_hash = Some(hash.into());
            entry.accepted_at = Some(self.now().to_string());
            self.save(&saved)?;
            local.saved = saved;
        }
        let state = self.view(page, &raw, &local, self.now());
        self.write_status(page, &state)?;
        Ok(state)
    }

    pub fn pause(&self, page: &str, paused: bool) -> Result<ScheduleState> {
        let mut local = self.local.lock().unwrap();
        self.require_schedule(page)?;
        let raw = self.raw(page)?;
        ensure!(
            raw["runner"].as_str() == Some(&self.node_id),
            "this node is not the schedule runner"
        );
        let mut saved = local.saved.clone();
        saved.schedules.entry(page.into()).or_default().paused = paused;
        self.save(&saved)?;
        local.saved = saved;
        let state = self.view(page, &raw, &local, self.now());
        self.write_status(page, &state)?;
        Ok(state)
    }

    /// Desktop setting only. The hub uses its config at startup.
    pub fn set_available(&self, available: bool) -> Result<()> {
        ensure!(
            self.kind == "desktop",
            "runner_available is desktop-only; configure hub schedules.runner"
        );
        let mut local = self.local.lock().unwrap();
        if local.available != available {
            self.write_record(available, self.now())?;
            local.available = available;
            local.record_at = self.now();
        }
        self.refresh_statuses(&local, self.now())
    }

    fn schedule_pages(&self) -> Result<Vec<String>> {
        let mut pages = vec![];
        let mut offset = 0;
        loop {
            let results = self
                .vault
                .search_spec(self.search_spec.clone(), 100, offset)?;
            let count = results.hits.len();
            pages.extend(
                results
                    .hits
                    .into_iter()
                    .filter(|hit| hit.source == "vault")
                    .map(|hit| hit.id),
            );
            if count < 100 {
                break;
            }
            offset += count as u32;
        }
        Ok(pages)
    }

    fn require_schedule(&self, page: &str) -> Result<()> {
        ensure!(
            self.schedule_pages()?.iter().any(|id| id == page),
            "page is not a live schedule"
        );
        Ok(())
    }

    fn refresh_statuses(&self, local: &Local, now: Timestamp) -> Result<()> {
        for page in self.schedule_pages()? {
            let raw = self.raw(&page)?;
            if raw["runner"].as_str() == Some(&self.node_id) {
                self.write_status(&page, &self.view(&page, &raw, local, now))?;
            }
        }
        Ok(())
    }

    fn write_status(&self, page: &str, state: &ScheduleState) -> Result<()> {
        let last = state.last_runs.iter().rev().find(|r| r.result != "skipped");
        let mut wanted = json!({"runner_state": state.runner_state});
        if let Some(last) = last {
            wanted["last_run"] = serde_json::to_value(last)?;
        }
        if let Some(next) = &state.next_run {
            wanted["next_run"] = json!(next);
        }
        let reason = state
            .refusal_reason
            .clone()
            .or_else(|| match state.runner_state.as_str() {
                "needs_acceptance" => Some(
                    if state.accepted {
                        "needs acceptance again"
                    } else {
                        "needs acceptance"
                    }
                    .into(),
                ),
                "runner_off" => Some("runner turned off".into()),
                _ => None,
            });
        if let Some(reason) = reason {
            wanted["reason"] = json!(reason);
        }
        let old: Value = serde_json::from_str(
            &self
                .vault
                .type_data(page.into(), "schedule_status".into())?,
        )?;
        for key in ["runner_state", "last_run", "next_run", "reason"] {
            if let Some(value) = wanted.get(key) {
                if old.get(key) != Some(value) {
                    self.vault.type_data_set(
                        page.into(),
                        "schedule_status".into(),
                        vec![key.into()],
                        value.to_string(),
                    )?;
                }
            } else if key != "last_run" && old.get(key).is_some() {
                // A long series of skips can evict the last actual run from
                // the bounded local log. Its synced summary remains valid.
                self.vault.type_data_delete(
                    page.into(),
                    "schedule_status".into(),
                    vec![key.into()],
                )?;
            }
        }
        Ok(())
    }

    fn resolve(&self, reference: &str) -> Result<String> {
        if !reference.starts_with("[[") {
            return Ok(reference.into());
        }
        ensure!(reference.ends_with("]]"), "invalid page link");
        self.vault
            .parse_markdown(reference.into())
            .into_iter()
            .flat_map(|b| b.marks)
            .find_map(|mark| match mark.kind {
                MarkKind::PageLink(id) => Some(id),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("cannot resolve page link {reference}"))
    }

    fn execute(
        &self,
        page: &str,
        schedule: &Schedule,
        occurrence: Timestamp,
        now: Timestamp,
    ) -> ScheduleRun {
        let result = (|| -> Result<(String, bool)> {
            // The step runs only with a tool its schedule's tools allow.
            anyhow::ensure!(
                schedule.tools.allows(format::NOTE_STEP_TOOL),
                "the schedule's tools do not allow {}",
                format::NOTE_STEP_TOOL
            );
            let id = vault_core::schedule_occurrence_id(page, &occurrence.to_string());
            self.vault
                .create_page_from_identified(
                    id,
                    self.resolve(&schedule.step.parent)?,
                    schedule.render_title(occurrence)?,
                    Some(self.resolve(&schedule.step.template)?),
                )
                .map_err(Into::into)
        })();
        match result {
            Ok((id, created)) => ScheduleRun {
                occurrence: occurrence.to_string(),
                at: now.to_string(),
                result: if created { "created" } else { "existed" }.into(),
                page: Some(id),
                error: None,
            },
            Err(e) => ScheduleRun {
                occurrence: occurrence.to_string(),
                at: now.to_string(),
                result: "failed".into(),
                page: None,
                error: Some(format!("{e:#}")),
            },
        }
    }

    pub fn run_now(&self, page: &str) -> Result<ScheduleRun> {
        let mut local = self.local.lock().unwrap();
        self.require_schedule(page)?;
        let raw = self.raw(page)?;
        let now = self.now();
        ensure!(
            raw["runner"].as_str() == Some(&self.node_id),
            "this node is not the schedule runner"
        );
        let state = self.view(page, &raw, &local, now);
        self.write_status(page, &state)?;
        ensure!(
            state.runner_state == "ok",
            "schedule cannot run: {}",
            state.runner_state
        );
        let schedule = Schedule::parse(raw.clone())?;
        let occurrence = Timestamp::from_second(now.as_second().div_euclid(60) * 60)?;
        let run = self.execute(page, &schedule, occurrence, now);
        let mut saved = local.saved.clone();
        let entry = saved.schedules.entry(page.into()).or_default();
        record(entry, run.clone());
        self.save(&saved)?;
        local.saved = saved;
        self.write_status(page, &self.view(page, &raw, &local, now))?;
        Ok(run)
    }

    pub fn tick(&self) -> Result<()> {
        let mut local = self.local.lock().unwrap();
        self.tick_locked(&mut local, self.now())
    }

    pub fn tick_at(&self, now: Timestamp) -> Result<()> {
        let mut local = self.local.lock().unwrap();
        self.tick_locked(&mut local, now)
    }

    fn tick_locked(&self, local: &mut Local, now: Timestamp) -> Result<()> {
        if now.as_second() - local.record_at.as_second() >= RECORD_REFRESH_SECONDS {
            self.write_record(local.available, now)?;
            local.record_at = now;
        }
        let mut errors = vec![];
        for page in self.schedule_pages()? {
            if let Err(error) = self.tick_page(&page, local, now) {
                errors.push(format!("{page}: {error:#}"));
            }
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        Ok(())
    }

    fn tick_page(&self, page: &str, local: &mut Local, now: Timestamp) -> Result<()> {
        let raw = self.raw(page)?;
        if raw["runner"].as_str() != Some(&self.node_id) {
            return Ok(());
        }
        let state = self.view(page, &raw, local, now);
        if state.runner_state != "ok" {
            return self.write_status(page, &state);
        }
        let schedule = Schedule::parse(raw.clone())?;
        let mut saved = local.saved.clone();
        let entry = saved.schedules.entry(page.into()).or_default();
        let accepted: Timestamp = entry
            .accepted_at
            .as_ref()
            .context("missing acceptance time")?
            .parse()?;
        let after = entry
            .last_occurrence
            .as_deref()
            .map(str::parse::<Timestamp>)
            .transpose()?
            .map_or(accepted, |last| last.max(accepted));
        let due = schedule.occurrences(after, now)?;
        let newest = due.last().copied();
        for occurrence in &due {
            let on_time =
                now.as_millisecond() - occurrence.as_millisecond() <= (TICK_SECONDS * 1000) as i64;
            let run = if (schedule.missed == "latest" && Some(*occurrence) == newest)
                || (schedule.missed == "skip" && on_time)
            {
                self.execute(page, &schedule, *occurrence, now)
            } else {
                ScheduleRun {
                    occurrence: occurrence.to_string(),
                    at: now.to_string(),
                    result: "skipped".into(),
                    page: None,
                    error: None,
                }
            };
            record(entry, run);
        }
        if !due.is_empty() {
            self.save(&saved)?;
            local.saved = saved;
        }
        self.write_status(page, &self.view(page, &raw, local, now))
    }
}

fn record(saved: &mut SavedSchedule, run: ScheduleRun) {
    // Manual runs never move the watermark backwards.
    if saved.last_occurrence.as_ref().is_none_or(|last| {
        let last: Option<Timestamp> = last.parse().ok();
        last.is_none_or(|last| {
            run.occurrence
                .parse::<Timestamp>()
                .is_ok_and(|at| at > last)
        })
    }) {
        saved.last_occurrence = Some(run.occurrence.clone());
    }
    saved.runs.push(run);
    if saved.runs.len() > 50 {
        saved.runs.drain(..saved.runs.len() - 50);
    }
}

#[cfg(test)]
mod tests;
