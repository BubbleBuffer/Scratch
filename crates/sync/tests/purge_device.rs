//! The purging device's id (`X-Purge-Device: 1`) through a real hub over
//! HTTP: it reaches the hub, survives a restart, and is forwarded to the
//! other devices. Also the peers that do not send it, and the malformed
//! flag, which the hub refuses.

mod support;

use std::sync::atomic::Ordering;

use support::*;
use vault_core::*;
use vault_sync::wire::{SyncRequestV2, SyncResponseV2, PROTOCOL, PURGE_PROTOCOL};
use vault_sync::{
    Hub, HubRequest, HubResponse, PURGE_CAPABILITY, PURGE_CAPABILITY_HEADER,
    PURGE_DEVICE_CAPABILITY, PURGE_DEVICE_CAPABILITY_HEADER,
};

fn nb(id: &str, text: &str) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind: BlockKind::Paragraph,
        text: text.into(),
        marks: vec![],
        checked: false,
    }
}

fn ins(after: Option<&str>, block: NewBlock) -> BlockOp {
    BlockOp::Insert {
        parent: None,
        after: after.map(Into::into),
        block,
    }
}

fn set_text(id: &str, text: &str) -> BlockOp {
    BlockOp::SetText {
        id: id.into(),
        text: text.into(),
        marks: vec![],
    }
}

/// A page "T" with blocks "1" and "2", made on the hub and synced to the
/// given devices.
fn shared_page(net: &Net, devices: &[&Device]) -> PageId {
    let h = &net.hub_vault;
    let t = h.create_page(None, "T".into()).unwrap();
    h.apply(
        t.clone(),
        vec![ins(None, nb("1", "one")), ins(Some("1"), nb("2", "two"))],
    )
    .unwrap();
    for d in devices {
        d.sync();
    }
    t
}

/// The phone's offline edit: a rename and a changed first block.
fn phone_edit(a: &Device, t: &PageId) {
    a.v().rename_page(t.clone(), "T (phone)".into()).unwrap();
    a.v()
        .apply(t.clone(), vec![set_text("1", "one (phone)")])
        .unwrap();
}

/// The laptop's offline purge: it deletes block 2, trashes the page and
/// purges it. The phone never saw the deletion.
fn laptop_purge(b: &Device, t: &PageId) {
    b.v()
        .apply(t.clone(), vec![BlockOp::Delete { id: "2".into() }])
        .unwrap();
    b.v().delete_page(t.clone()).unwrap();
    b.v().purge_page(t.clone()).unwrap();
}

/// The `/sync` exchanges, in order. The health probe and the other routes
/// are left out: only `/sync` carries v2 purge records.
fn sync_exchanges(ex: &[Exchange]) -> Vec<&Exchange> {
    ex.iter().filter(|e| e.path == "/sync").collect()
}

/// The v2 requests the client sent on `/sync`, read with the given layout.
fn sent_v2(ex: &[Exchange], device: bool) -> Vec<SyncRequestV2> {
    sync_exchanges(ex)
        .into_iter()
        .map(|e| {
            SyncRequestV2::decode_with_device(&request_message(e), device)
                .expect("a v2 request in this layout")
        })
        .collect()
}

/// The v2 answers to `/sync`, read with the given layout.
fn answers_v2(ex: &[Exchange], device: bool) -> Vec<SyncResponseV2> {
    sync_exchanges(ex)
        .into_iter()
        .map(|e| {
            SyncResponseV2::decode_with_device(&answer_message(e), device)
                .expect("a v2 answer in this layout")
        })
        .collect()
}

#[test]
fn the_purging_device_crosses_the_wire_and_survives_a_hub_restart() {
    let net = Net::new();
    let a = net.device("phone");
    let b = net.device("laptop");
    let t = shared_page(&net, &[&a, &b]);
    let laptop = b.v().device_id();
    laptop_purge(&b, &t);

    let from = net.count();
    b.sync();
    let sent = net.since(from);
    for e in sync_exchanges(&sent) {
        assert_eq!(
            header(&e.req_headers, PURGE_DEVICE_CAPABILITY_HEADER),
            Some("1"),
            "the client sends the flag"
        );
        assert_eq!(
            header(&e.hub_headers, PURGE_DEVICE_CAPABILITY_HEADER),
            Some("1"),
            "the hub receives the flag"
        );
    }
    let pushed: Vec<PurgeRecord> = sent_v2(&sent, true)
        .into_iter()
        .flat_map(|r| r.purges)
        .collect();
    assert!(!pushed.is_empty(), "the purge reached the hub");
    assert!(pushed
        .iter()
        .all(|p| p.page == t && p.device.as_deref() == Some(laptop.as_str())));

    let stored = net.hub_vault.purge_records();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].device.as_deref(), Some(laptop.as_str()));

    // The phone pulls the purge: the answer carries the device id too.
    let from = net.count();
    a.sync();
    let pulled: Vec<PurgeRecord> = answers_v2(&net.since(from), true)
        .into_iter()
        .flat_map(|r| r.purges)
        .collect();
    assert!(pulled
        .iter()
        .any(|p| p.page == t && p.device.as_deref() == Some(laptop.as_str())));

    // A restart keeps the id on the hub, and a device that has never seen
    // the purge gets it in its first answer.
    let net = Net::on(net.shutdown_hub());
    let stored = net.hub_vault.purge_records();
    assert!(stored
        .iter()
        .any(|p| p.device.as_deref() == Some(laptop.as_str())));
    let tablet = net.device("tablet");
    tablet.sync();
    let first = answers_v2(&net.since(0), true);
    assert!(first
        .iter()
        .flat_map(|r| &r.purges)
        .any(|p| p.page == t && p.device.as_deref() == Some(laptop.as_str())));
}

/// Turns the device advertisement on or off for the client syncs after this
/// call. Without it the client sends no flag and no device ids.
fn advertise_device(net: &Net, on: bool) {
    net.proxy.shim.hide_device.store(!on, Ordering::SeqCst);
}

/// Every `/sync` request in `ex` has the layout `device` names, byte for
/// byte: it decodes, and encodes back to the bytes that crossed the proxy.
fn assert_request_layout(ex: &[Exchange], device: bool) {
    let syncs = sync_exchanges(ex);
    assert!(!syncs.is_empty(), "the client synced");
    for e in syncs {
        let msg = request_message(e);
        let v2 = SyncRequestV2::decode_with_device(&msg, device).expect("a v2 request");
        assert_eq!(v2.encode_with_device(device), msg, "the request's layout");
    }
}

/// As [`assert_request_layout`], for the answers on `/sync`.
fn assert_answer_layout(ex: &[Exchange], device: bool) {
    let syncs = sync_exchanges(ex);
    assert!(!syncs.is_empty(), "the client synced");
    for e in syncs {
        let msg = answer_message(e);
        let v2 = SyncResponseV2::decode_with_device(&msg, device).expect("a v2 answer");
        assert_eq!(v2.encode_with_device(device), msg, "the answer's layout");
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Leg 1: a capability-off producer sends its purge fieldless. Leg 2: the
/// hub's answer to a capability-on reader is extended, and the record has
/// no device. Leg 3: both survive a hub restart.
#[test]
fn a_fieldless_purge_from_a_capability_off_producer_reads_back_with_no_id() {
    let net = Net::new();
    let a = net.device("phone");
    let b = net.device("laptop");
    let t = shared_page(&net, &[&a, &b]);
    laptop_purge(&b, &t);

    // Leg 1: the producer's health probe sees no device capability.
    advertise_device(&net, false);
    let from = net.count();
    b.sync();
    let producer = net.since(from);
    for e in sync_exchanges(&producer) {
        assert_eq!(header(&e.req_headers, PURGE_DEVICE_CAPABILITY_HEADER), None);
        assert_eq!(header(&e.hub_headers, PURGE_DEVICE_CAPABILITY_HEADER), None);
    }
    assert_request_layout(&producer, false);
    let pushed = request_purges(&producer, false);
    assert!(!pushed.is_empty(), "the purge reached the hub");
    assert!(pushed.iter().all(|p| p.page == t && p.device.is_none()));
    let stored = net.hub_vault.purge_records();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].device, None);

    // Leg 2: the reader's health probe sees the capability, so its answer
    // is extended. The record still has no device, and the phone applies it.
    advertise_device(&net, true);
    let from = net.count();
    a.sync();
    let reader = net.since(from);
    for e in sync_exchanges(&reader) {
        assert_eq!(
            header(&e.req_headers, PURGE_DEVICE_CAPABILITY_HEADER),
            Some("1")
        );
        assert_eq!(
            header(&e.hub_headers, PURGE_DEVICE_CAPABILITY_HEADER),
            Some("1")
        );
    }
    assert_answer_layout(&reader, true);
    let pulled = answers_v2(&reader, true);
    assert!(pulled
        .iter()
        .flat_map(|r| &r.purges)
        .any(|p| p.page == t && p.device.is_none()));
    assert!(
        a.v().purged_pages().contains(&t),
        "the phone applied the purge"
    );

    // Leg 3: after a restart the record is still fieldless on the hub, and a
    // new capability-on device gets it extended with no id.
    let net = Net::on(net.shutdown_hub());
    let stored = net.hub_vault.purge_records();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].device, None);
    let tablet = net.device("tablet");
    tablet.sync();
    let first = net.since(0);
    assert_answer_layout(&first, true);
    assert!(answers_v2(&first, true)
        .iter()
        .flat_map(|r| &r.purges)
        .any(|p| p.page == t && p.device.is_none()));
}

/// The purge records in the `/sync` requests on `ex`, read with `device`'s
/// layout.
fn request_purges(ex: &[Exchange], device: bool) -> Vec<PurgeRecord> {
    sent_v2(ex, device)
        .into_iter()
        .flat_map(|r| r.purges)
        .collect()
}

/// The devices named on the purging side of the conflicts `v` holds for `t`.
fn purge_sides(v: &Vault, t: &PageId) -> Vec<String> {
    v.list_conflicts(Some(t.clone()))
        .unwrap()
        .into_iter()
        .flat_map(|c| c.sides)
        .filter(|s| matches!(s.state, SideState::Purged { .. }))
        .map(|s| s.device)
        .collect()
}

/// Leg 1: a capability-on producer sends its id extended. Leg 2: a
/// capability-off reader gets the answer fieldless. Its offline edit
/// recovery records the purging side as "unknown", since the wire carried
/// no id.
#[test]
fn a_known_device_from_a_capability_on_producer_reads_back_as_fieldless_wire() {
    let net = Net::new();
    let a = net.device("phone");
    let b = net.device("laptop");
    let t = shared_page(&net, &[&a, &b]);
    let laptop = b.v().device_id();
    // The phone's offline edit, made before the purge reaches it.
    phone_edit(&a, &t);
    laptop_purge(&b, &t);

    // Leg 1: the producer's health probe sees the capability.
    advertise_device(&net, true);
    let from = net.count();
    b.sync();
    let producer = net.since(from);
    for e in sync_exchanges(&producer) {
        assert_eq!(
            header(&e.req_headers, PURGE_DEVICE_CAPABILITY_HEADER),
            Some("1")
        );
        assert_eq!(
            header(&e.hub_headers, PURGE_DEVICE_CAPABILITY_HEADER),
            Some("1")
        );
    }
    assert_request_layout(&producer, true);
    let pushed = request_purges(&producer, true);
    assert!(!pushed.is_empty(), "the purge reached the hub");
    assert!(pushed
        .iter()
        .all(|p| p.page == t && p.device.as_deref() == Some(laptop.as_str())));
    assert!(
        sync_exchanges(&producer)
            .iter()
            .any(|e| contains(&request_message(e), laptop.as_bytes())),
        "the id is in the request bytes"
    );
    let stored = net.hub_vault.purge_records();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].device.as_deref(), Some(laptop.as_str()));

    // Leg 2: the reader's health probe sees no capability, so the answer is
    // fieldless and the phone applies the purge without an id.
    advertise_device(&net, false);
    let from = net.count();
    a.sync();
    let reader = net.since(from);
    for e in sync_exchanges(&reader) {
        assert_eq!(header(&e.req_headers, PURGE_DEVICE_CAPABILITY_HEADER), None);
        assert_eq!(header(&e.hub_headers, PURGE_DEVICE_CAPABILITY_HEADER), None);
    }
    assert_request_layout(&reader, false);
    assert_answer_layout(&reader, false);
    let pulled = answers_v2(&reader, false);
    assert!(pulled.iter().flat_map(|r| &r.purges).any(|p| p.page == t));
    assert!(
        !a.v().purged_pages().contains(&t),
        "the offline edit superseded the purge"
    );
    let restored = a.v().load_page(t.clone()).unwrap();
    assert!(!restored.in_trash);
    assert_eq!(restored.title, "T (phone)");
    assert_eq!(
        restored
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        vec!["one (phone)", "two"]
    );

    let sides = purge_sides(a.v(), &t);
    assert!(!sides.is_empty(), "the phone recorded the purge");
    assert!(sides.iter().all(|s| s == "unknown"), "{sides:?}");
}

#[test]
fn a_capability_on_purge_names_the_purging_device_in_the_conflict() {
    let net = Net::new();
    let a = net.device("phone");
    let b = net.device("laptop");
    let t = shared_page(&net, &[&a, &b]);
    phone_edit(&a, &t);
    let laptop = b.v().device_id();
    laptop_purge(&b, &t);
    b.sync();
    a.sync();
    let sides = purge_sides(a.v(), &t);
    assert!(!sides.is_empty(), "the phone recorded the purge");
    assert!(sides.iter().all(|s| s == &laptop), "{sides:?} vs {laptop}");
}

/// A `/sync` POST to `hub` with the given device-flag headers.
fn sync_with_flags(hub: &Hub, token: &str, flags: &[&str], body: &[u8]) -> HubResponse {
    let mut headers: Vec<(String, String)> = vec![
        ("Authorization".into(), format!("Bearer {token}")),
        ("X-Device".into(), "flags-device".into()),
    ];
    headers.extend(
        flags
            .iter()
            .map(|f| (PURGE_DEVICE_CAPABILITY_HEADER.to_string(), f.to_string())),
    );
    hub.handle(HubRequest {
        method: "POST".into(),
        path: "/sync".into(),
        query: String::new(),
        headers,
        body: Box::new(body),
        body_len: Some(body.len() as u64),
        cancel: None,
    })
}

#[test]
fn health_advertises_both_purge_capabilities() {
    let dir = TempDir::new("health");
    let hub = Hub::new(dir.open()).unwrap();
    let r = hub.handle(HubRequest {
        method: "GET".into(),
        path: "/health".into(),
        query: String::new(),
        headers: Vec::new(),
        body: Box::new(&b""[..]),
        body_len: None,
        cancel: None,
    });
    assert_eq!(r.status, 200);
    assert_eq!(
        header(&r.headers, PURGE_CAPABILITY_HEADER),
        Some(PURGE_CAPABILITY)
    );
    assert_eq!(
        header(&r.headers, PURGE_DEVICE_CAPABILITY_HEADER),
        Some(PURGE_DEVICE_CAPABILITY)
    );
}

#[test]
fn a_malformed_device_flag_is_refused_before_the_body_is_read() {
    let dir = TempDir::new("flags");
    let hub = Hub::new(dir.open()).unwrap();
    let token = hub.pair("flags").unwrap();
    let cases: Vec<Vec<&str>> = vec![vec!["2"], vec!["yes"], vec![""], vec!["1", "1"]];
    for flags in &cases {
        let r = sync_with_flags(&hub, &token, flags, &[PURGE_PROTOCOL]);
        assert_eq!(r.status, 400, "flags {flags:?}");
        let text = String::from_utf8_lossy(&r.body);
        assert!(text.contains(PURGE_DEVICE_CAPABILITY_HEADER), "{text}");
    }
}

#[test]
fn the_device_flag_needs_a_protocol_two_body() {
    let dir = TempDir::new("v1flag");
    let hub = Hub::new(dir.open()).unwrap();
    let token = hub.pair("flags").unwrap();
    let r = sync_with_flags(&hub, &token, &["1"], &[PROTOCOL]);
    assert_eq!(r.status, 400);
    let text = String::from_utf8_lossy(&r.body);
    assert!(text.contains(PURGE_DEVICE_CAPABILITY_HEADER), "{text}");
}
