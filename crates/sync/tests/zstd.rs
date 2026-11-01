//! The zstd-v1 transport: compressed `/sync` and `/push` bodies between a
//! real client and hub, the plain fallback with old peers, and the hub's
//! refusals of unnegotiated, unknown or malformed compressed requests.
//!
//! Most tests run through a recording proxy in front of the hub, which logs
//! the bytes on the wire (so byte counts and encodings are checked as sent)
//! and can hide the capability from requests or the confirmation from
//! answers, to act as an older peer. The refusal tests call `Hub::handle`.

mod support;

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use support::*;
use vault_core::*;
use vault_sync::wire::{
    PageBytes, PushRequest, PushResponse, SyncRequest, SyncResponse, SyncResponseV2, PROTOCOL,
    PURGE_PROTOCOL,
};
use vault_sync::{
    Hub, HubRequest, HubResponse, SyncError, CAPABILITY_HEADER, PURGE_CAPABILITY,
    PURGE_CAPABILITY_HEADER, ZSTD_CAPABILITY,
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

/// `pages` pages of `blocks` paragraphs each: enough to be worth compressing.
fn fill(v: &Vault, tag: &str, pages: usize, blocks: usize) {
    for p in 0..pages {
        let page = v.create_page(None, format!("{tag} {p}")).unwrap();
        let ids: Vec<String> = (0..blocks).map(|b| format!("{tag}{p}-{b}")).collect();
        let ops: Vec<BlockOp> = (0..blocks)
            .map(|b| BlockOp::Insert {
                parent: None,
                after: (b > 0).then(|| ids[b - 1].clone()),
                block: nb(
                    &ids[b],
                    &format!("{tag}: paragraph {b} of page {p}, lorem ipsum dolor sit amet."),
                ),
            })
            .collect();
        v.apply(page, ops).unwrap();
    }
}

/// Every page (id -> page) of a vault.
fn snapshot(v: &Vault) -> BTreeMap<String, Page> {
    v.page_versions()
        .unwrap()
        .into_iter()
        .map(|p| (p.page.clone(), v.load_page(p.page).unwrap()))
        .collect()
}

// ---- Through the proxy: real client and hub ----

#[test]
fn pull_and_push_are_compressed_on_the_wire() {
    let net = Net::new();
    fill(&net.hub_vault, "hub", 30, 20);
    let a = net.device("phone");
    let report = a.client.sync_once().unwrap();
    let ex = net.since(0);

    // The health probe comes first and advertises the purge protocol.
    assert_eq!(ex[0].path, "/health");
    assert_eq!(
        header(&ex[0].resp_headers, PURGE_CAPABILITY_HEADER),
        Some(PURGE_CAPABILITY)
    );
    // The first sync request is plain but advertises zstd, and is v2 since
    // the hub speaks it; the answer is compressed and confirms.
    let first = syncs(&ex)[0];
    assert_eq!(first.path, "/sync");
    assert_eq!(
        header(&first.req_headers, CAPABILITY_HEADER),
        Some(ZSTD_CAPABILITY)
    );
    assert_eq!(header(&first.req_headers, "Content-Encoding"), None);
    assert_eq!(first.req_body[0], PURGE_PROTOCOL);
    assert_eq!(
        header(&first.resp_headers, "Content-Encoding"),
        Some("zstd")
    );
    assert_eq!(
        header(&first.resp_headers, CAPABILITY_HEADER),
        Some(ZSTD_CAPABILITY)
    );
    let plain = decode(&first.resp_body);
    assert_eq!(plain[0], PURGE_PROTOCOL);
    assert!(first.resp_body.len() * 2 < plain.len(), "compressed well");
    SyncResponseV2::decode(&plain).expect("binary v2 inside the frame");

    // The report counts the bodies as they crossed the wire, the health
    // probe's included.
    assert_eq!((report.bytes_up, report.bytes_down), wire_totals(&ex));
    assert_eq!(snapshot(a.client.vault()), snapshot(&net.hub_vault));

    // A device with pages of its own: the hub asks for them in the first
    // answer, so the push follows in the same sync and is compressed.
    let b = net.device("laptop");
    fill(b.client.vault(), "laptop", 20, 20);
    let before = net.count();
    let report = b.client.sync_once().unwrap();
    let ex = net.since(before);
    let pushes: Vec<&Exchange> = ex.iter().filter(|e| e.path == "/push").collect();
    assert!(!pushes.is_empty(), "the hub asked for the pages");
    assert_eq!(report.pushed as usize, 20);
    for p in &pushes {
        assert_eq!(header(&p.req_headers, "Content-Encoding"), Some("zstd"));
        assert_eq!(decode(&p.req_body)[0], PURGE_PROTOCOL);
        assert!(p.req_body.len() < decode(&p.req_body).len());
    }
    assert_eq!((report.bytes_up, report.bytes_down), wire_totals(&ex));
    assert_eq!(snapshot(b.client.vault()), snapshot(&net.hub_vault));
}

#[test]
fn an_old_hub_gets_and_sends_plain_bodies() {
    let net = Net::new();
    // An old hub: no zstd and no purge protocol in its answers.
    net.proxy.shim.hide_capability.store(true, Ordering::SeqCst);
    net.proxy.shim.hide_purge.store(true, Ordering::SeqCst);
    fill(&net.hub_vault, "hub", 30, 20);
    let a = net.device("phone");
    let report = a.client.sync_once().unwrap();
    let ex = net.since(0);
    assert_eq!(snapshot(a.client.vault()), snapshot(&net.hub_vault));
    assert_eq!((report.bytes_up, report.bytes_down), wire_totals(&ex));
    for e in &ex {
        assert_eq!(header(&e.resp_headers, PURGE_CAPABILITY_HEADER), None);
        assert_eq!(header(&e.resp_headers, "Content-Encoding"), None);
        assert_eq!(header(&e.resp_headers, CAPABILITY_HEADER), None);
    }
    for e in syncs(&ex) {
        assert_eq!(e.req_body[0], PROTOCOL, "binary v1 on the wire");
        assert_eq!(e.resp_body[0], PROTOCOL, "binary v1 on the wire");
    }

    // Pushes stay plain too: the sync never got a confirmation.
    let b = net.device("laptop");
    fill(b.client.vault(), "laptop", 20, 20);
    let before = net.count();
    let report = b.client.sync_once().unwrap();
    let ex = net.since(before);
    assert!(ex.iter().any(|e| e.path == "/push"));
    assert!(ex
        .iter()
        .all(|e| header(&e.req_headers, "Content-Encoding").is_none()));
    assert!(syncs(&ex).iter().all(|e| e.req_body[0] == PROTOCOL));
    assert_eq!((report.bytes_up, report.bytes_down), wire_totals(&ex));
    assert_eq!(snapshot(b.client.vault()), snapshot(&net.hub_vault));
}

#[test]
fn an_old_purge_hub_gets_v1_bodies_and_keeps_zstd() {
    let net = Net::new();
    // Only the purge protocol is hidden: zstd is still negotiated.
    net.proxy.shim.hide_purge.store(true, Ordering::SeqCst);
    fill(&net.hub_vault, "hub", 30, 20);
    let a = net.device("phone");
    let report = a.client.sync_once().unwrap();
    let ex = net.since(0);
    assert_eq!(header(&ex[0].resp_headers, PURGE_CAPABILITY_HEADER), None);
    let sync = syncs(&ex);
    assert_eq!(sync[0].req_body[0], PROTOCOL);
    assert_eq!(
        header(&sync[0].resp_headers, "Content-Encoding"),
        Some("zstd")
    );
    assert_eq!(decode(&sync[0].resp_body)[0], PROTOCOL);
    assert_eq!((report.bytes_up, report.bytes_down), wire_totals(&ex));
    assert_eq!(snapshot(a.client.vault()), snapshot(&net.hub_vault));
}

#[test]
fn v1_fallback_warns_once_per_client_connection() {
    struct Warnings(Arc<Mutex<Vec<String>>>);
    struct Message(String);
    impl tracing::field::Visit for Message {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }
    impl tracing::Subscriber for Warnings {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.target() == "vault_sync::client" && *metadata.level() == tracing::Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut message = Message(String::new());
            event.record(&mut message);
            self.0.lock().unwrap().push(message.0);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    let net = Net::new();
    let a = net.device("phone");
    let messages = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(Warnings(messages.clone()), || {
        a.client.sync_once().unwrap(); // A v2 hub emits no warning.
        assert!(messages.lock().unwrap().is_empty());
        net.proxy.shim.hide_purge.store(true, Ordering::SeqCst);
        a.client.sync_once().unwrap();
        a.client.sync_once().unwrap();
        assert_eq!(messages.lock().unwrap().len(), 1);
        net.proxy.shim.hide_purge.store(false, Ordering::SeqCst);
        a.client.sync_once().unwrap();
        net.proxy.shim.hide_purge.store(true, Ordering::SeqCst);
        a.client.sync_once().unwrap();
        assert_eq!(messages.lock().unwrap().len(), 1);
        let b = net.device("laptop");
        b.client.sync_once().unwrap();
        b.client.sync_once().unwrap();
    });
    let messages = messages.lock().unwrap();
    assert_eq!(messages.len(), 2);
    for message in messages.iter() {
        assert!(message.contains("Purge recovery is off against this hub"));
        assert!(message.contains("edits to permanently deleted pages may be dropped"));
    }
}

#[test]
fn an_answer_without_confirmation_is_refused() {
    let net = Net::new();
    fill(&net.hub_vault, "hub", 30, 20);
    net.proxy
        .shim
        .hide_confirmation
        .store(true, Ordering::SeqCst);
    let a = net.device("phone");
    let err = a.client.sync_once().unwrap_err();
    assert!(matches!(err, SyncError::Protocol(_)), "{err}");
    // Nothing was decoded or imported from the refused answer.
    assert!(a.client.vault().page_versions().unwrap().is_empty());
}

#[test]
fn every_sync_renegotiates() {
    let net = Net::new();
    let a = net.device("phone");
    // The hub is old during the first sync...
    net.proxy.shim.hide_capability.store(true, Ordering::SeqCst);
    fill(a.client.vault(), "one", 10, 20);
    let before = net.count();
    a.client.sync_once().unwrap();
    let first = net.since(before);
    assert!(first.iter().any(|e| e.path == "/push"));
    assert!(first
        .iter()
        .all(|e| header(&e.resp_headers, "Content-Encoding").is_none()));

    // ...and new the next time: its old answers are not remembered.
    net.proxy
        .shim
        .hide_capability
        .store(false, Ordering::SeqCst);
    fill(a.client.vault(), "two", 10, 20);
    let before = net.count();
    a.client.sync_once().unwrap();
    let all = net.since(before);
    let second = syncs(&all);
    // The first request of this sync is plain, and the hub confirms it.
    assert_eq!(header(&second[0].req_headers, "Content-Encoding"), None);
    assert_eq!(
        header(&second[0].resp_headers, CAPABILITY_HEADER),
        Some(ZSTD_CAPABILITY)
    );
    let pushes: Vec<&Exchange> = second
        .iter()
        .copied()
        .filter(|e| e.path == "/push")
        .collect();
    assert!(!pushes.is_empty());
    assert!(pushes
        .iter()
        .all(|p| header(&p.req_headers, "Content-Encoding") == Some("zstd")));
    assert_eq!(snapshot(a.client.vault()), snapshot(&net.hub_vault));
}

// ---- An old client, spoken by hand ----

#[test]
fn a_new_client_against_a_v1_hub_keeps_delete_wins() {
    let net = Net::new();
    net.proxy.shim.hide_purge.store(true, Ordering::SeqCst);
    let p = net
        .hub_vault
        .create_page(None, "Before purge".into())
        .unwrap();
    let a = net.device("phone");
    a.client.sync_once().unwrap();
    net.hub_vault.delete_page(p.clone()).unwrap();
    net.hub_vault.purge_page(p.clone()).unwrap();
    a.client
        .vault()
        .rename_page(p.clone(), "Offline edit".into())
        .unwrap();
    let report = a.client.sync_once().unwrap();
    assert_eq!(report.purged, 1);
    for v in [&*net.hub_vault, &**a.client.vault()] {
        assert!(v.load_page(p.clone()).is_err());
        assert!(v.purged_pages().contains(&p));
    }
    assert!(syncs(&net.since(0)).iter().all(|e| {
        let bytes = if header(&e.req_headers, "Content-Encoding") == Some("zstd") {
            decode(&e.req_body)
        } else {
            e.req_body.clone()
        };
        bytes[0] == PROTOCOL
    }));
}

/// One plain `POST` as an old client makes it: binary v1, no capability.
/// The answer must come back plain too.
fn post_plain(url: &str, token: &str, device: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let mut resp = ureq::post(&format!("{url}{path}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Device", device)
        .header("Content-Type", "application/octet-stream")
        .send(body)
        .expect("the hub answers");
    assert_eq!(resp.status().as_u16(), 200);
    assert!(resp.headers().get("Content-Encoding").is_none());
    assert!(resp.headers().get(CAPABILITY_HEADER).is_none());
    resp.body_mut().read_to_vec().unwrap()
}

/// A sync as the old client runs it: full rounds of `/sync`, applying the
/// hub's updates to `v` and pushing the pages the hub asks for, until
/// neither side has anything left.
fn old_client_sync(v: &Vault, url: &str, token: &str) {
    let device = v.device_id();
    for _ in 0..10 {
        let versions: PageBytes = v
            .page_versions()
            .unwrap()
            .into_iter()
            .map(|p| (p.page, p.version))
            .collect();
        let req = SyncRequest {
            full: true,
            versions,
            budget: 8 << 20,
            ..SyncRequest::default()
        };
        let body = post_plain(url, token, &device, "/sync", &req.encode());
        let resp = SyncResponse::decode(&body).unwrap();
        assert!(!resp.reset);
        v.import_purged(resp.purged).unwrap();
        let idle = resp.updates.is_empty() && resp.need.is_empty() && !resp.more;
        if !resp.updates.is_empty() {
            let updates = resp
                .updates
                .into_iter()
                .map(|(page, bytes)| PageUpdate { page, bytes })
                .collect();
            v.import_updates_batch(updates).unwrap();
        }
        if !resp.need.is_empty() {
            let updates: PageBytes = resp
                .need
                .iter()
                .map(|(page, hv)| {
                    (
                        page.clone(),
                        v.export_updates(page.clone(), hv.clone()).unwrap(),
                    )
                })
                .collect();
            let push = PushRequest { updates }.encode();
            PushResponse::decode(&post_plain(url, token, &device, "/push", &push)).unwrap();
        }
        if idle {
            return;
        }
    }
    panic!("the old client did not converge");
}

#[test]
fn an_old_client_pulls_and_pushes_plain_binary_v1() {
    let net = Net::new();
    fill(&net.hub_vault, "hub", 30, 20);
    let dir = TempDir::new("old");
    let v = dir.open();
    fill(&v, "old", 5, 20);
    let token = net.hub.pair_legacy_token("old").unwrap();
    old_client_sync(&v, &net.proxy.url, &token);

    // Both sides hold everything: the hub's 30 pages and the client's 5.
    assert_eq!(v.page_versions().unwrap().len(), 35);
    assert_eq!(snapshot(&v), snapshot(&net.hub_vault));
    let ex = net.since(0);
    assert!(ex.iter().any(|e| e.path == "/sync"));
    assert!(
        ex.iter().any(|e| e.path == "/push"),
        "the client's pages were pushed"
    );
    for e in &ex {
        assert_eq!(header(&e.req_headers, CAPABILITY_HEADER), None);
        assert_eq!(header(&e.req_headers, "Content-Encoding"), None);
        assert_eq!(header(&e.resp_headers, CAPABILITY_HEADER), None);
        assert_eq!(header(&e.resp_headers, "Content-Encoding"), None);
    }
    for e in syncs(&ex) {
        assert_eq!(e.req_body[0], PROTOCOL);
        assert_eq!(e.resp_body[0], PROTOCOL);
    }
}

// ---- Hub requests, transport-free ----

/// A hub with `pages` pages and a token for a device named `probe`.
fn lone_hub(pages: usize) -> (TempDir, Arc<Hub>, String) {
    let dir = TempDir::new("lone");
    let v = dir.open();
    if pages > 0 {
        fill(&v, "lone", pages, 20);
    }
    let hub = Hub::new(v).unwrap();
    let token = hub.pair_legacy_token("probe").unwrap();
    (dir, hub, token)
}

/// One `POST` to `path` with the given headers; `auth` is the bearer token
/// (None: no Authorization header).
fn post(
    hub: &Hub,
    auth: Option<&str>,
    path: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> HubResponse {
    let mut headers = vec![("X-Device".to_string(), "probe-device".to_string())];
    if let Some(t) = auth {
        headers.push(("Authorization".to_string(), format!("Bearer {t}")));
    }
    headers.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    hub.handle(HubRequest {
        method: "POST".into(),
        path: path.into(),
        query: String::new(),
        headers,
        body: Box::new(body),
        body_len: Some(body.len() as u64),
        cancel: None,
    })
}

fn full_request() -> Vec<u8> {
    SyncRequest {
        full: true,
        ..SyncRequest::default()
    }
    .encode()
}

fn compressed(raw: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(raw, 3).unwrap()
}

const CAP: (&str, &str) = (CAPABILITY_HEADER, ZSTD_CAPABILITY);
const ZSTD_BODY: (&str, &str) = ("Content-Encoding", "zstd");

#[test]
fn plain_request_gets_a_plain_answer() {
    let (_d, hub, token) = lone_hub(30);
    let r = post(&hub, Some(&token), "/sync", &[], &full_request());
    assert_eq!(r.status, 200);
    assert!(r.headers.is_empty(), "no confirmation, no encoding");
    assert_eq!(r.body[0], PROTOCOL);
    let resp = SyncResponse::decode(&r.body).unwrap();
    assert!(!resp.updates.is_empty());
}

#[test]
fn advertised_request_gets_a_confirmed_compressed_answer() {
    let (_d, hub, token) = lone_hub(30);
    let plain = post(&hub, Some(&token), "/sync", &[], &full_request());
    let r = post(&hub, Some(&token), "/sync", &[CAP], &full_request());
    assert_eq!(r.status, 200);
    assert!(r
        .headers
        .contains(&(CAPABILITY_HEADER.into(), ZSTD_CAPABILITY.into())));
    assert!(r
        .headers
        .contains(&("Content-Encoding".into(), "zstd".into())));
    assert!(r.body.len() < plain.body.len());
    assert_eq!(decode(&r.body), plain.body);
}

#[test]
fn tiny_answers_are_confirmed_but_plain() {
    let (_d, hub, token) = lone_hub(0);
    let r = post(&hub, Some(&token), "/sync", &[CAP], &full_request());
    assert_eq!(r.status, 200);
    let cap = vec![(CAPABILITY_HEADER.to_string(), ZSTD_CAPABILITY.to_string())];
    assert_eq!(r.headers, cap);
    assert!(r.body.len() < 1024);
}

#[test]
fn unknown_capability_version_means_none() {
    let (_d, hub, token) = lone_hub(30);
    let r = post(
        &hub,
        Some(&token),
        "/sync",
        &[(CAPABILITY_HEADER, "zstd-v2")],
        &full_request(),
    );
    assert_eq!(r.status, 200);
    assert!(r.headers.is_empty());
    assert_eq!(r.body[0], PROTOCOL);
}

#[test]
fn compressed_request_without_the_capability_is_refused() {
    let (_d, hub, token) = lone_hub(30);
    // A valid compressed body is refused, not decoded.
    let body = compressed(&full_request());
    let r = post(&hub, Some(&token), "/sync", &[ZSTD_BODY], &body);
    assert_eq!(r.status, 415);
    // So is garbage: the negotiation is checked before any decoding.
    let r = post(&hub, Some(&token), "/push", &[ZSTD_BODY], b"not zstd");
    assert_eq!(r.status, 415);
}

#[test]
fn negotiated_compressed_request_is_decoded() {
    let (_d, hub, token) = lone_hub(30);
    // Empty version vectors: `need` skips them, so the request is valid v1
    // without depending on Loro's encoding.
    let versions = (0..200)
        .map(|i| (format!("page-{i}"), Vec::new()))
        .collect();
    let req = SyncRequest {
        full: true,
        versions,
        ..SyncRequest::default()
    };
    let raw = req.encode();
    let body = compressed(&raw);
    assert!(body.len() < raw.len());
    let r = post(&hub, Some(&token), "/sync", &[CAP, ZSTD_BODY], &body);
    assert_eq!(r.status, 200);
    SyncResponse::decode(&decode(&r.body)).unwrap();
}

#[test]
fn malformed_compressed_request_is_400() {
    let (_d, hub, token) = lone_hub(30);
    let r = post(
        &hub,
        Some(&token),
        "/sync",
        &[CAP, ZSTD_BODY],
        b"not zstd at all",
    );
    assert_eq!(r.status, 400);
    // Cut mid-frame: the decoder reports it, and the hub answers 400.
    let body = compressed(&big_request());
    let r = post(
        &hub,
        Some(&token),
        "/sync",
        &[CAP, ZSTD_BODY],
        &body[..body.len() / 2],
    );
    assert_eq!(r.status, 400, "a truncated frame is a decoder error");
}

/// A valid protocol-v1 `SyncRequest` of about 1.5 MB: a long list of pages
/// the hub does not have, with no versions to check.
fn big_request() -> Vec<u8> {
    let versions = (0..100_000)
        .map(|i| (format!("page-{i:06}"), Vec::new()))
        .collect();
    SyncRequest {
        full: true,
        versions,
        ..SyncRequest::default()
    }
    .encode()
}

#[test]
fn oversized_decoder_window_is_refused() {
    let (_d, hub, token) = lone_hub(30);
    let raw = big_request();
    // The payload is valid v1, so a refusal can only come from the decoder.
    SyncRequest::decode(&raw).expect("valid protocol v1");
    // The same payload with an ordinary window is accepted...
    let ordinary = compressed(&raw);
    let r = post(&hub, Some(&token), "/sync", &[CAP, ZSTD_BODY], &ordinary);
    assert_eq!(r.status, 200);
    // ...and with a 128 MB window it is refused by the decoder.
    let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    enc.window_log(27).unwrap();
    enc.write_all(&raw).unwrap();
    let big_window = enc.finish().unwrap();
    let r = post(&hub, Some(&token), "/sync", &[CAP, ZSTD_BODY], &big_window);
    assert_eq!(r.status, 400);
    assert!(
        String::from_utf8_lossy(&r.body).contains("compressed body"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
}

#[test]
fn other_encodings_are_refused() {
    let (_d, hub, token) = lone_hub(30);
    let r = post(
        &hub,
        Some(&token),
        "/sync",
        &[CAP, ("Content-Encoding", "gzip")],
        &full_request(),
    );
    assert_eq!(r.status, 415);
}

#[test]
fn authentication_comes_before_negotiation() {
    let (_d, hub, _token) = lone_hub(30);
    // No token and a compressed body without the capability: 401, not 415.
    let body = compressed(&full_request());
    let r = post(&hub, None, "/sync", &[ZSTD_BODY], &body);
    assert_eq!(r.status, 401);
}

#[test]
fn attachments_are_never_compressed() {
    let (_d, hub, token) = lone_hub(0);
    let id = "ab".repeat(32);
    let headers = vec![
        ("X-Device".to_string(), "probe-device".to_string()),
        ("Authorization".to_string(), format!("Bearer {token}")),
        (CAPABILITY_HEADER.to_string(), ZSTD_CAPABILITY.to_string()),
        ("Content-Encoding".to_string(), "zstd".to_string()),
        ("X-Name".to_string(), "a.txt".to_string()),
    ];
    let r = hub.handle(HubRequest {
        method: "PUT".into(),
        path: format!("/blobs/{id}"),
        query: String::new(),
        headers,
        body: Box::new(&b"not zstd"[..]),
        body_len: Some(8),
        cancel: None,
    });
    assert_eq!(r.status, 415);
    assert!(hub.vault().attachment_path(id).is_err(), "nothing stored");
}
