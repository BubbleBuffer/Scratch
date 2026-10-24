//! Device presence uses ordinary vault pages and the real HTTP sync client.
mod support;

use serde_json::json;
use support::TempDir;
use vault_core::{node_record_id, Vault};
use vault_sync::{Hub, SyncClient};

fn stale_record(v: &Vault, name: &str) {
    v.write_node_record(
        v.device_id(),
        json!({
            "device_id": v.device_id(), "name": name, "kind": "desktop",
            "last_seen": "2020-01-01T00:00:00Z",
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn two_devices_see_names_and_successful_sync_times_in_the_same_exchange() {
    let (ht, at, bt) = (
        TempDir::new("devices_hub"),
        TempDir::new("devices_a"),
        TempDir::new("devices_b"),
    );
    let (hv, a, b) = (ht.open(), at.open(), bt.open());
    stale_record(&a, "Studio");
    stale_record(&b, "Laptop");
    let hub = Hub::new(hv.clone()).unwrap();
    let server = hub.serve("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.addr());
    let ac = SyncClient::new(a.clone(), &url, &hub.pair("Studio").unwrap());
    let bc = SyncClient::new(b.clone(), &url, &hub.pair("Laptop").unwrap());
    ac.sync_once().unwrap();
    // The timestamp written after the successful exchange reaches the hub
    // before sync_once returns, without a separate heartbeat protocol.
    let a_seen = a.device_records().unwrap()[0].last_seen_ms.unwrap();
    assert!(a_seen > 1_577_836_800_000);
    assert_eq!(hv.device_records().unwrap()[0].last_seen_ms, Some(a_seen));
    bc.sync_once().unwrap();
    ac.sync_once().unwrap();
    let mut expected = a.device_records().unwrap();
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(expected.len(), 2);
    for v in [&hv, &a, &b] {
        let mut records = v.device_records().unwrap();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(records, expected);
        assert_eq!(
            records.iter().find(|r| r.id == a.device_id()).unwrap().name,
            "Studio"
        );
        assert_eq!(
            records.iter().find(|r| r.id == b.device_id()).unwrap().name,
            "Laptop"
        );
        assert!(records
            .iter()
            .all(|r| r.last_seen_ms.unwrap() > 1_577_836_800_000));
    }
    let before = a
        .type_data(node_record_id(&a.device_id()), "node".into())
        .unwrap();
    ac.sync_once().unwrap();
    assert_eq!(
        a.type_data(node_record_id(&a.device_id()), "node".into())
            .unwrap(),
        before,
        "immediate sync does not churn the presence record"
    );
}

#[test]
fn failed_sync_does_not_refresh_presence() {
    let (ht, at) = (
        TempDir::new("devices_failure_hub"),
        TempDir::new("devices_failure_a"),
    );
    let (hv, a) = (ht.open(), at.open());
    stale_record(&a, "Studio");
    let before = a.device_records().unwrap();
    let hub = Hub::new(hv).unwrap();
    let server = hub.serve("127.0.0.1:0").unwrap();
    let client = SyncClient::new(
        a.clone(),
        &format!("http://{}", server.addr()),
        "invalid-token",
    );
    assert!(client.sync_once().is_err());
    assert_eq!(a.device_records().unwrap(), before);
}
