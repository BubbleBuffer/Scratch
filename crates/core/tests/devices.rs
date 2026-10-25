//! Device records: heartbeat throttling across reopen, renames, merges that
//! keep fields the scheduler omits, and two vaults exchanging their records.
use serde_json::{json, Value};
use std::collections::HashMap;
use vault_core::{
    new_id, node_record_id, version_includes, DeviceProject, Vault, DEVICE_ACTIVITY_INTERVAL_MS,
    RUNNER_ONLINE_WINDOW_MS,
};

const MINUTE: i64 = DEVICE_ACTIVITY_INTERVAL_MS;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_devices_{}", new_id())))
    }
    fn vault(&self, name: &str) -> Vault {
        Vault::open(self.0.join(name).to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Same incremental exchange as schedule_replicas.rs: no transport or new sync path.
fn push(from: &Vault, to: &Vault) {
    to.import_purged(from.purged_pages()).unwrap();
    let have: HashMap<_, _> = to
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    for v in from.page_versions().unwrap() {
        let since = have.get(&v.page).cloned().unwrap_or_default();
        if !version_includes(&since, &v.version).unwrap() {
            to.import_updates(v.page.clone(), from.export_updates(v.page, since).unwrap())
                .unwrap();
        }
    }
}

// The node record's type data, the fields the scheduler writes.
fn node_data(v: &Vault, device: &str) -> Value {
    let id = node_record_id(device);
    serde_json::from_str::<Value>(&v.type_data_all(id).unwrap()).unwrap()["node"].clone()
}

#[test]
fn constants_match_the_runner_contract() {
    assert_eq!(RUNNER_ONLINE_WINDOW_MS, 300_000);
    assert_eq!(DEVICE_ACTIVITY_INTERVAL_MS, 60_000);
}

#[test]
fn heartbeat_is_throttled_and_stays_throttled_after_reopen() {
    let dir = TempDir::new();
    let v = dir.vault("a");
    assert!(
        v.device_records().unwrap().is_empty(),
        "reading creates no record"
    );
    let device = v.device_id();
    let t0 = 1_000_123;

    assert!(
        v.publish_device_activity_at(t0).unwrap(),
        "first heartbeat writes"
    );
    v.merge_node_record(
        device.clone(),
        json!({
            "device_id": device, "available": false,
            "last_seen": "1970-01-01T00:17:10.123Z"
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        v.device_records().unwrap()[0].last_seen_ms,
        Some(t0),
        "scheduler and app publication share the same persisted throttle"
    );
    assert!(!v.publish_device_activity_at(t0 + MINUTE - 1).unwrap());
    drop(v);

    let v = dir.vault("a");
    assert!(
        !v.publish_device_activity_at(t0 + 30_000).unwrap(),
        "throttle survives reopen"
    );
    assert!(
        v.publish_device_activity_at(t0 + MINUTE).unwrap(),
        "one interval later writes"
    );
    let records = v.device_records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, device);
    assert_eq!(records[0].kind, "desktop");
    assert!(
        !records[0].name.trim().is_empty(),
        "defaults to the host name"
    );
    assert_eq!(records[0].last_seen_ms, Some(t0 + MINUTE));
}

#[test]
fn clock_heartbeat_is_recent_and_throttled_within_the_interval() {
    let dir = TempDir::new();
    let v = dir.vault("a");
    assert!(v.publish_device_activity().unwrap());
    assert!(!v.publish_device_activity().unwrap());
    let seen = v.device_records().unwrap()[0]
        .last_seen_ms
        .expect("last_seen");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert!(
        now - seen < RUNNER_ONLINE_WINDOW_MS,
        "the heartbeat is online"
    );
}

#[test]
fn rename_is_trimmed_validated_and_kept_by_later_heartbeats() {
    let dir = TempDir::new();
    let v = dir.vault("a");
    let device = v.device_id();
    assert!(
        v.rename_device(device.clone(), "Hub".into()).is_err(),
        "no record yet"
    );
    v.publish_device_activity_at(60_000).unwrap();

    assert!(v
        .rename_device("not-a-device".into(), "Ghost".into())
        .is_err());
    assert!(v.rename_device("".into(), "Ghost".into()).is_err());
    assert!(v.rename_device(device.clone(), "   ".into()).is_err());
    assert_eq!(
        v.device_records().unwrap().len(),
        1,
        "unknown ids create nothing"
    );

    v.rename_device(device.clone(), "  Hub  ".into()).unwrap();
    let nodes = v.find_by_role("nodes".into()).unwrap().expect("Nodes page");
    let titles: Vec<String> = v
        .list_children(Some(nodes), 0, 10)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert_eq!(titles, vec!["Hub"], "the title follows the name");

    assert!(v.publish_device_activity_at(120_000).unwrap());
    let records = v.device_records().unwrap();
    let rec = &records[0];
    assert_eq!(rec.name, "Hub");
    assert_eq!(rec.kind, "desktop");
    assert_eq!(rec.last_seen_ms, Some(120_000));
}

#[test]
fn merge_keeps_omitted_fields_and_the_renamed_name() {
    let dir = TempDir::new();
    let v = dir.vault("a");
    let device = v.device_id();
    let first = json!({
        "device_id": device, "name": "Scheduler", "kind": "desktop",
        "available": true, "steps": ["note.from_template"],
    });
    let id = v
        .merge_node_record(device.clone(), first.to_string())
        .unwrap();
    assert_eq!(id, node_record_id(&device));
    assert_eq!(v.device_records().unwrap()[0].name, "Scheduler");

    v.rename_device(device.clone(), "Hub".into()).unwrap();
    let later = json!({ "device_id": device, "name": "Other", "available": false });
    assert_eq!(
        v.merge_node_record(device.clone(), later.to_string())
            .unwrap(),
        id
    );

    let data = node_data(&v, &device);
    assert_eq!(
        data["name"],
        json!("Hub"),
        "a supplied name does not rename"
    );
    assert_eq!(
        data["available"],
        json!(false),
        "supplied fields are merged in"
    );
    assert_eq!(
        data["steps"],
        json!(["note.from_template"]),
        "omitted fields are kept"
    );
    assert_eq!(v.device_records().unwrap()[0].name, "Hub");
}

#[test]
fn merge_refuses_other_devices_and_mismatched_ids() {
    let dir = TempDir::new();
    let v = dir.vault("a");
    let device = v.device_id();
    assert!(v
        .merge_node_record(
            "other-device".into(),
            json!({"device_id": "other-device", "name": "X"}).to_string()
        )
        .is_err());
    assert!(v
        .merge_node_record(
            device.clone(),
            json!({"device_id": "other-device", "name": "X"}).to_string()
        )
        .is_err());
    assert!(v
        .merge_node_record(
            device.clone(),
            json!({"device_id": device, "name": " "}).to_string()
        )
        .is_err());
    assert!(
        v.device_records().unwrap().is_empty(),
        "refused merges write nothing"
    );
}

#[test]
fn two_vaults_see_each_others_records_after_an_exchange() {
    let dir = TempDir::new();
    let a = dir.vault("a");
    let b = dir.vault("b");
    a.publish_device_activity_at(120_000).unwrap();
    a.rename_device(a.device_id(), "Desk".into()).unwrap();
    b.publish_device_activity_at(180_000).unwrap();
    b.rename_device(b.device_id(), "Laptop".into()).unwrap();

    push(&a, &b);
    push(&b, &a);
    let expected = vec![
        (a.device_id(), "Desk".to_owned(), Some(120_000)),
        (b.device_id(), "Laptop".to_owned(), Some(180_000)),
    ];
    for v in [&a, &b] {
        let seen: Vec<_> = v
            .device_records()
            .unwrap()
            .into_iter()
            .map(|d| (d.id, d.name, d.last_seen_ms))
            .collect();
        assert_eq!(seen, expected);
    }
}

#[test]
fn remote_rename_and_local_heartbeat_converge_without_losing_either() {
    let dir = TempDir::new();
    let a = dir.vault("a");
    let b = dir.vault("b");
    let device = a.device_id();
    a.publish_device_activity_at(60_000).unwrap();
    push(&a, &b);

    // B renames A's record. A heartbeats before it has seen the rename.
    b.rename_device(device.clone(), "Desk".into()).unwrap();
    assert!(a.publish_device_activity_at(180_000).unwrap());

    push(&b, &a);
    push(&a, &b);
    for v in [&a, &b] {
        let rec = v.device_records().unwrap().remove(0);
        assert_eq!(rec.name, "Desk", "the rename survives the heartbeat");
        assert_eq!(
            rec.last_seen_ms,
            Some(180_000),
            "the heartbeat survives the rename"
        );
    }
}

fn app(id: &str, name: &str) -> DeviceProject {
    DeviceProject {
        id: id.into(),
        name: name.into(),
    }
}

#[test]
fn projects_publish_only_id_and_name_and_survive_heartbeats_and_merges() {
    let dir = TempDir::new();
    let v = dir.vault("a");
    let device = v.device_id();
    // A projects-only publish creates the record without a heartbeat.
    assert!(v
        .publish_device_projects(vec![app("app", "  App  "), app("notes", " ")])
        .unwrap());
    assert_eq!(v.device_records().unwrap()[0].last_seen_ms, None);
    assert_eq!(
        node_data(&v, &device)["projects"],
        json!([{ "id": "app", "name": "App" }, { "id": "notes", "name": "notes" }]),
        "only id and name are published, and a blank name is the id"
    );
    let listed = vec![app("app", "App"), app("notes", "notes")];
    assert_eq!(v.device_records().unwrap()[0].projects, listed);
    assert!(!v
        .publish_device_projects(vec![app("app", "App"), app("notes", "notes")])
        .unwrap());

    // Heartbeats, scheduler merges and a full record write keep the list.
    assert!(v.publish_device_activity_at(MINUTE).unwrap());
    v.merge_node_record(
        device.clone(),
        json!({ "device_id": device, "available": true }).to_string(),
    )
    .unwrap();
    v.write_node_record(
        device.clone(),
        json!({ "device_id": device, "name": "Desk", "kind": "desktop" }).to_string(),
    )
    .unwrap();
    assert_eq!(v.device_records().unwrap()[0].projects, listed);

    // Only publish_device_projects writes the list, and it refuses bad lists.
    assert!(v
        .write_node_record(
            device.clone(),
            json!({ "device_id": device, "name": "Desk", "projects": [] }).to_string(),
        )
        .is_err());
    assert!(v
        .merge_node_record(device.clone(), json!({ "projects": [] }).to_string())
        .is_err());
    assert!(v.publish_device_projects(vec![app(" ", "Blank")]).is_err());
    assert!(v
        .publish_device_projects(vec![app("app", "A"), app("app", "B")])
        .is_err());
    assert_eq!(v.device_records().unwrap()[0].projects, listed);

    // Another vault that receives the record sees its projects too.
    let b = dir.vault("b");
    push(&v, &b);
    let seen = b
        .device_records()
        .unwrap()
        .into_iter()
        .find(|r| r.id == device)
        .unwrap();
    assert_eq!(seen.projects, listed);
}
