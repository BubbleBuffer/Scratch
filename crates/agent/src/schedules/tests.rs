use super::*;

struct Fixture {
    runner: Arc<Scheduler>,
    vault: Arc<Vault>,
    page: String,
    parent: String,
    raw: Value,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new(missed: &str, available: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
        let parent = vault.create_page(None, "Journal".into()).unwrap();
        let template = vault
            .create_template(parent.clone(), "Weekly note".into())
            .unwrap();
        vault
            .apply(
                template.clone(),
                vec![vault_core::BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: vault_core::NewBlock {
                        id: vault_core::new_id(),
                        kind: vault_core::BlockKind::Paragraph,
                        text: "Template body {{date}} stays literal".into(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        let runner = Scheduler::open(
            vault.clone(),
            dir.path(),
            "desktop",
            "Test desktop",
            available,
        )
        .unwrap();
        runner.set_test_time(Some("2026-10-01T00:00:00Z")).unwrap();
        let raw = json!({"version":1,
            "trigger":{"rrule":"FREQ=WEEKLY;BYDAY=MO", "at":"2026-10-05T08:00", "tz":"Europe/Berlin", "missed":missed},
            "runner":runner.node_id(), "tools":["create_page"],
            "steps":[{"id":"note", "do":"note.from_template", "template":template,
                "parent":parent,"title":"Week {{week}} {{year}}"}],
            "future":{"keep":[1,2,3]},
        });
        let page = vault
            .create_page_with_type(None, "Weekly schedule".into(), Some("schedule".into()))
            .unwrap();
        let f = Self {
            runner,
            vault,
            page,
            parent,
            raw,
            _dir: dir,
        };
        f.store();
        f
    }

    fn store(&self) {
        for (key, value) in self.raw.as_object().unwrap() {
            self.vault
                .type_data_set(
                    self.page.clone(),
                    "schedule".into(),
                    vec![key.clone()],
                    value.to_string(),
                )
                .unwrap();
        }
    }

    fn accept(&self) {
        self.runner
            .accept(&self.page, &definition_hash(&self.raw).unwrap())
            .unwrap();
    }

    fn tick(&self, at: &str) {
        self.runner.set_test_time(Some(at)).unwrap();
        self.runner.tick().unwrap();
    }

    fn notes(&self) -> Vec<vault_core::PageSummary> {
        self.vault
            .list_children(Some(self.parent.clone()), 0, 100)
            .unwrap()
    }

    fn status(&self) -> Value {
        serde_json::from_str(
            &self
                .vault
                .type_data(self.page.clone(), "schedule_status".into())
                .unwrap(),
        )
        .unwrap()
    }

    fn device_record(&self) -> Value {
        serde_json::from_str(
            &self
                .vault
                .type_data(self.runner.node_id().into(), "node".into())
                .unwrap(),
        )
        .unwrap()
    }
}

#[test]
fn state_and_preview_supply_the_same_engine_definition_hash() {
    let f = Fixture::new("latest", true);
    let state = f.runner.state(&f.page).unwrap();
    let preview = f.runner.preview(&f.raw.to_string()).unwrap();
    assert!(preview.valid);
    assert_eq!(preview.current_hash, state.current_hash);
    let hash = state.current_hash.unwrap();
    assert_eq!(hash.len(), 64);
    assert!(f.runner.accept(&f.page, &hash).unwrap().hash_matches);
    assert!(f.runner.preview("not JSON").unwrap().current_hash.is_none());
}

#[test]
fn latest_runs_only_newest_and_logs_missed_occurrences() {
    let f = Fixture::new("latest", true);
    f.accept();
    f.tick("2026-10-20T00:00:00Z");
    let state = f.runner.state(&f.page).unwrap();
    assert_eq!(
        state
            .last_runs
            .iter()
            .map(|r| r.result.as_str())
            .collect::<Vec<_>>(),
        ["skipped", "skipped", "created"]
    );
    assert_eq!(state.last_runs[2].occurrence, "2026-10-19T06:00:00Z");
    assert_eq!(f.notes().len(), 1);
    let note = &f.notes()[0];
    assert_eq!(
        note.id,
        vault_core::schedule_occurrence_id(&f.page, "2026-10-19T06:00:00Z")
    );
    assert_eq!(note.title, "Week 43 2026");
    assert_eq!(
        f.vault.load_page(note.id.clone()).unwrap().blocks[0].text,
        "Template body {{date}} stays literal"
    );
    assert_eq!(f.status()["last_run"]["result"], "created");
    assert_eq!(f.status()["runner_state"], "ok");
    let raw: Value = serde_json::from_str(
        &f.vault
            .type_data(f.page.clone(), "schedule".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(raw, f.raw);
    f.tick("2026-10-20T00:00:00Z");
    assert_eq!(f.runner.state(&f.page).unwrap().last_runs.len(), 3);
    assert_eq!(f.notes().len(), 1);
}

#[test]
fn stored_schedule_naming_old_tools_still_validates_and_runs() {
    // A schedule saved before the tool merge names old tools. Validation
    // only needs create_page in the list, so it stays valid and runs.
    let mut f = Fixture::new("latest", true);
    f.raw["tools"] = json!(["create_page", "append_blocks", "agenda"]);
    f.store();
    let preview = f.runner.preview(&f.raw.to_string()).unwrap();
    assert!(preview.valid, "{preview:?}");
    f.accept();
    f.tick("2026-10-20T00:00:00Z");
    let state = f.runner.state(&f.page).unwrap();
    assert_eq!(state.last_runs.last().unwrap().result, "created");
    assert_eq!(f.notes().len(), 1);
    assert_eq!(f.status()["last_run"]["result"], "created");
}

#[test]
fn skip_catches_only_an_occurrence_within_tick_interval() {
    let f = Fixture::new("skip", true);
    f.accept();
    f.tick("2026-10-20T00:00:00Z");
    assert!(f.notes().is_empty());
    assert_eq!(
        f.runner
            .state(&f.page)
            .unwrap()
            .last_runs
            .iter()
            .filter(|r| r.result == "skipped")
            .count(),
        3
    );
    f.tick("2026-10-26T07:00:25Z");
    assert_eq!(f.notes().len(), 1);
}

#[test]
fn acceptance_never_backfills_and_is_persisted_across_restart() {
    let f = Fixture::new("latest", true);
    f.tick("2026-10-20T00:00:00Z");
    assert!(f.notes().is_empty());
    assert_eq!(f.status()["runner_state"], "needs_acceptance");
    f.accept();
    f.tick("2026-10-20T00:00:01Z");
    assert!(f.runner.state(&f.page).unwrap().last_runs.is_empty());
    f.tick("2026-10-26T07:00:00Z");
    assert_eq!(f.notes().len(), 1);
    let reopened = Scheduler::open(
        f.vault.clone(),
        f._dir.path(),
        "desktop",
        "Test desktop",
        true,
    )
    .unwrap();
    reopened
        .set_test_time(Some("2026-10-26T07:00:01Z"))
        .unwrap();
    reopened.tick().unwrap();
    let state = reopened.state(&f.page).unwrap();
    assert!(state.accepted && state.hash_matches);
    assert_eq!(state.last_runs.len(), 1);
    assert_eq!(f.notes().len(), 1);
}

#[test]
fn definition_edit_requires_acceptance_but_trigger_edit_does_not() {
    let mut f = Fixture::new("latest", true);
    f.accept();
    assert!(f.runner.accept(&f.page, "stale hash").is_err());
    f.raw["trigger"]["at"] = json!("2026-10-05T09:00");
    f.store();
    f.tick("2026-10-05T07:00:00Z");
    assert_eq!(f.notes().len(), 1);
    assert!(f.runner.state(&f.page).unwrap().hash_matches);
    f.raw["steps"][0]["title"] = json!("Edited {{date}}");
    f.store();
    f.tick("2026-10-12T07:00:00Z");
    assert_eq!(f.notes().len(), 1);
    assert_eq!(f.status()["runner_state"], "needs_acceptance");
    assert!(!f.runner.state(&f.page).unwrap().hash_matches);
    assert!(f.runner.run_now(&f.page).is_err());
}

#[test]
fn other_runner_is_never_executed_or_written_back() {
    let mut f = Fixture::new("latest", true);
    f.accept();
    f.raw["runner"] = json!("another node");
    f.store();
    let old = f.status();
    f.tick("2026-10-20T00:00:00Z");
    assert!(f.notes().is_empty());
    assert_eq!(f.status(), old);
    assert!(f
        .runner
        .accept(&f.page, &definition_hash(&f.raw).unwrap())
        .is_err());
    assert!(f.runner.run_now(&f.page).is_err());
}

#[test]
fn off_and_paused_states_block_execution_and_update_record() {
    let f = Fixture::new("latest", false);
    f.accept();
    f.tick("2026-10-20T00:00:00Z");
    assert!(f.notes().is_empty());
    assert_eq!(f.status()["runner_state"], "runner_off");
    assert!(matches!(
        f.vault.load_page(f.runner.node_id().into()),
        Err(vault_core::VaultError::NotFound(_))
    ));
    assert_eq!(f.vault.find_by_role("nodes".into()).unwrap(), None);
    f.runner.set_available(true).unwrap();
    let record: Value = serde_json::from_str(
        &f.vault
            .type_data(f.runner.node_id().into(), "node".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["available"], true);
    f.runner.pause(&f.page, true).unwrap();
    f.tick("2026-10-20T00:00:00Z");
    assert!(f.notes().is_empty());
    assert_eq!(f.status()["runner_state"], "paused");
    f.runner.pause(&f.page, false).unwrap();
    f.tick("2026-10-20T00:00:00Z");
    assert_eq!(f.notes().len(), 1);
    f.runner.set_available(false).unwrap();
    assert_eq!(f.status()["runner_state"], "runner_off");
    let record: Value = serde_json::from_str(
        &f.vault
            .type_data(f.runner.node_id().into(), "node".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["available"], false);
    assert!(record.get("tools").is_none());
}

#[test]
fn reopening_a_desktop_off_marks_its_existing_record_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    let runner = Scheduler::open(vault.clone(), dir.path(), "desktop", "Test", true).unwrap();
    let node = runner.node_id().to_owned();
    let before = vault.page_count().unwrap();
    drop(runner);

    let runner = Scheduler::open(vault.clone(), dir.path(), "desktop", "Test", false).unwrap();
    let record: Value =
        serde_json::from_str(&vault.type_data(node.clone(), "node".into()).unwrap()).unwrap();
    assert_eq!(record["available"], false);
    assert_eq!(runner.node_id(), node);
    assert_eq!(vault.page_count().unwrap(), before);
}

#[test]
fn presence_record_is_not_a_registration_until_the_runner_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into()).unwrap());
    vault.publish_device_activity().unwrap();
    let node = vault_core::node_record_id(&vault.device_id());
    assert!(!is_registered(&vault, &node).unwrap());

    let runner = Scheduler::open(vault.clone(), dir.path(), "desktop", "Test", false).unwrap();
    let record: Value =
        serde_json::from_str(&vault.type_data(node.clone(), "node".into()).unwrap()).unwrap();
    assert_eq!(record["kind"], "desktop");
    assert!(record.get("schedule_format").is_none());
    assert!(!is_registered(&vault, &node).unwrap());

    runner.set_available(true).unwrap();
    assert!(is_registered(&vault, &node).unwrap());
}

#[test]
fn run_now_retries_same_minute_without_duplicate_and_preserves_unknowns() {
    let f = Fixture::new("latest", true);
    f.accept();
    f.runner
        .set_test_time(Some("2026-10-05T06:00:20Z"))
        .unwrap();
    let one = f.runner.run_now(&f.page).unwrap();
    let two = f.runner.run_now(&f.page).unwrap();
    assert_eq!(one.result, "created");
    assert_eq!(two.result, "existed");
    assert_eq!(one.page, two.page);
    assert_eq!(one.occurrence, "2026-10-05T06:00:00Z");
    assert_eq!(f.notes().len(), 1);
    assert_eq!(f.status()["last_run"]["result"], "existed");
}

#[test]
fn failed_template_is_recorded_without_creating_a_page() {
    let mut f = Fixture::new("latest", true);
    f.raw["steps"][0]["template"] = json!("missing");
    f.store();
    f.accept();
    f.tick("2026-10-05T06:00:00Z");
    assert_eq!(f.status()["last_run"]["result"], "failed");
    assert!(!f.status()["last_run"]["error"].as_str().unwrap().is_empty());
    assert!(f.notes().is_empty());
}

#[test]
fn refusal_is_written_once_and_preview_reports_unsupported_versions() {
    let mut f = Fixture::new("latest", true);
    f.raw["version"] = json!(2);
    f.store();
    f.tick("2026-10-05T06:00:00Z");
    assert_eq!(f.status()["runner_state"], "refused");
    assert!(f
        .runner
        .accept(&f.page, &definition_hash(&f.raw).unwrap())
        .is_err());
    let before = f.vault.page_versions().unwrap();
    f.tick("2026-10-05T06:00:01Z");
    assert_eq!(f.vault.page_versions().unwrap(), before);
    let preview = f.runner.preview(&f.raw.to_string()).unwrap();
    assert!(!preview.valid);
    assert!(preview.reason.is_some());
    assert!(!f.runner.preview("{").unwrap().valid);
}

#[test]
fn log_keeps_last_fifty_with_watermark() {
    let f = Fixture::new("skip", true);
    f.accept();
    f.tick("2026-10-01T01:00:00Z");
    f.tick("2028-10-01T00:00:00Z");
    let state = f.runner.state(&f.page).unwrap();
    assert_eq!(state.last_runs.len(), 50);
    assert!(state.last_runs.iter().all(|r| r.result == "skipped"));
    f.tick("2028-10-01T00:00:00Z");
    assert_eq!(f.runner.state(&f.page).unwrap().last_runs.len(), 50);
    assert!(f.notes().is_empty());
}

#[test]
fn device_record_refreshes_once_per_activity_interval() {
    let f = Fixture::new("latest", true);
    f.tick("2028-10-01T00:00:00Z");
    assert_eq!(f.device_record()["last_seen"], "2028-10-01T00:00:00Z");
    f.tick("2028-10-01T00:00:59Z");
    assert_eq!(f.device_record()["last_seen"], "2028-10-01T00:00:00Z");
    f.tick("2028-10-01T00:01:00Z");
    assert_eq!(f.device_record()["last_seen"], "2028-10-01T00:01:00Z");
}

#[test]
fn rename_survives_later_scheduler_writes() {
    let f = Fixture::new("latest", true);
    f.accept();
    f.vault
        .rename_device(f.vault.device_id(), "Studio mac".into())
        .unwrap();
    f.vault
        .type_data_set(
            f.runner.node_id().into(),
            "node".into(),
            vec!["extra".into()],
            json!({"keep": true}).to_string(),
        )
        .unwrap();
    Scheduler::open(
        f.vault.clone(),
        f._dir.path(),
        "desktop",
        "Test desktop",
        true,
    )
    .unwrap();
    f.tick("2028-10-01T00:00:00Z");
    let record = f.device_record();
    assert_eq!(record["name"], "Studio mac");
    assert_eq!(record["extra"]["keep"], true);
    assert_eq!(record["device_id"], f.vault.device_id());
    assert_eq!(record["kind"], "desktop");
    assert_eq!(record["available"], true);
    assert_eq!(record["schedule_format"], 1);
    assert_eq!(record["steps"], json!(["note.from_template"]));
    assert_eq!(record["last_seen"], "2028-10-01T00:00:00Z");
    assert_eq!(
        f.vault.load_page(f.runner.node_id().into()).unwrap().title,
        "Studio mac"
    );
}

#[test]
fn title_links_resolve_without_rewriting_schedule_and_preview_renders() {
    let mut f = Fixture::new("latest", true);
    f.raw["steps"][0]["template"] = json!("[[Weekly note]]");
    f.raw["steps"][0]["parent"] = json!("[[Journal]]");
    f.store();
    f.accept();
    let preview = f.runner.preview(&f.raw.to_string()).unwrap();
    assert!(preview.valid);
    assert_eq!(preview.occurrences.len(), 3);
    assert_eq!(preview.occurrences[0].title, "Week 41 2026");
    f.tick("2026-10-05T06:00:00Z");
    assert_eq!(f.notes().len(), 1);
}

#[test]
fn iso_week_year_is_correct_on_every_day_of_the_year_boundary() {
    let f = Fixture::new("latest", true);
    let schedule = Schedule::parse(f.raw.clone()).unwrap();
    for date in [
        "2026-12-28",
        "2026-12-29",
        "2026-12-30",
        "2026-12-31",
        "2027-01-01",
        "2027-01-02",
        "2027-01-03",
    ] {
        assert_eq!(
            schedule
                .render_title(format!("{date}T12:00:00Z").parse().unwrap())
                .unwrap(),
            "Week 53 2026",
            "{date}"
        );
    }
    assert_eq!(
        schedule
            .render_title("2027-01-04T12:00:00Z".parse().unwrap())
            .unwrap(),
        "Week 1 2027"
    );
}

#[test]
fn corrupt_state_fails_closed_instead_of_discarding_acceptance() {
    let f = Fixture::new("latest", true);
    f.accept();
    let path = f._dir.path().join("agent/schedules.json");
    std::fs::write(&path, b"broken state").unwrap();
    assert!(Scheduler::open(f.vault.clone(), f._dir.path(), "desktop", "Test", true).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"broken state");
}

#[test]
fn external_search_rows_are_not_executable_schedule_pages() {
    let f = Fixture::new("latest", true);
    f.vault
        .upsert_search_row(
            "external".into(),
            vault_core::ExternalSearchRow {
                id: "external-schedule".into(),
                title: "Not a vault page".into(),
                body: String::new(),
                page_type: "schedule".into(),
                target: None,
                updated_ms: 0,
            },
        )
        .unwrap();
    f.accept();
    f.tick("2026-10-05T06:00:00Z");
    assert_eq!(f.notes().len(), 1);
}

#[test]
fn skipped_log_eviction_retains_the_last_actual_run_summary() {
    let f = Fixture::new("skip", true);
    f.accept();
    f.tick("2026-10-05T06:00:00Z");
    let last = f.status()["last_run"].clone();
    assert_eq!(last["result"], "created");
    f.tick("2028-10-01T00:00:00Z");
    let state = f.runner.state(&f.page).unwrap();
    assert_eq!(state.last_runs.len(), 50);
    assert!(state.last_runs.iter().all(|run| run.result == "skipped"));
    assert_eq!(f.status()["last_run"], last);
}

#[test]
fn retrying_acceptance_keeps_the_original_catch_up_boundary() {
    let f = Fixture::new("latest", true);
    f.accept();
    let accepted_at = f.runner.state(&f.page).unwrap().accepted_at;
    f.runner
        .set_test_time(Some("2026-10-20T00:00:00Z"))
        .unwrap();
    f.accept();
    assert_eq!(f.runner.state(&f.page).unwrap().accepted_at, accepted_at);
    f.tick("2026-10-20T00:00:00Z");
    assert_eq!(f.notes().len(), 1);
    assert_eq!(f.runner.state(&f.page).unwrap().last_runs.len(), 3);
}

#[test]
fn a_note_with_schedule_data_must_have_the_live_schedule_type() {
    let f = Fixture::new("latest", true);
    f.accept();
    f.vault
        .set_page_type(f.page.clone(), "document".into())
        .unwrap();
    f.tick("2026-10-20T00:00:00Z");
    assert!(f.runner.run_now(&f.page).is_err());
    assert!(f
        .runner
        .accept(&f.page, &definition_hash(&f.raw).unwrap())
        .is_err());
    assert!(f.runner.pause(&f.page, true).is_err());
    assert!(f.notes().is_empty());
}
