//! A real `SyncClient` against a hub with device keys: a legacy token is
//! upgraded on the first sync, later syncs run on sessions alone, and a
//! hub restart (sessions dropped) renews the session transparently.

mod support;

use support::TempDir;
use vault_sync::{DeviceSession, Hub, HubRequest, SyncClient};

fn changes_status(hub: &Hub, bearer: &str, device: &str) -> u16 {
    hub.handle(HubRequest {
        method: "GET".into(),
        path: "/changes".into(),
        query: String::new(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {bearer}")),
            ("X-Device".into(), device.into()),
        ],
        body: Box::new(std::io::empty()),
        body_len: Some(0),
        cancel: None,
    })
    .status
}

#[test]
fn a_legacy_token_is_upgraded_and_later_syncs_run_on_sessions() {
    let (th, ta) = (TempDir::new("dkc_hub"), TempDir::new("dkc_a"));
    let hub_vault = th.open();
    let hub = Hub::new(hub_vault.clone()).unwrap();
    let server = hub.serve("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.addr());
    let token = hub.pair_legacy_token("phone").unwrap();
    let va = ta.open();
    let device = va.device_id();
    let page = hub_vault.create_page(None, "From the hub".into()).unwrap();

    let a = SyncClient::new(va.clone(), &url, &token);
    a.sync_once().unwrap();
    assert!(va.list_pages(None, 0, 1000).unwrap().iter().any(|p| p.id == page));

    // Key-bound now: the key file exists, the client remembers the
    // binding, and the bare token is refused.
    assert!(ta.0.join(".vault").join("device-key.json").is_file());
    let client_state = std::fs::read_to_string(ta.0.join(".vault").join("hub-client.json")).unwrap();
    assert!(client_state.contains(&vault_core::sha256_hex(token.as_bytes())));
    assert_eq!(changes_status(&hub, &token, &device), 401);
    let session = DeviceSession::shared(&ta.root(), &device, &url, &token, None).unwrap();
    assert!(session.is_bound());
    let bearer = session.bearer().unwrap();
    assert_ne!(bearer, token);
    assert_eq!(changes_status(&hub, &bearer, &device), 200);

    // A second sync, and a fresh client for the same vault, on sessions alone.
    let p2 = hub_vault.create_page(None, "Second".into()).unwrap();
    a.sync_once().unwrap();
    let a2 = SyncClient::new(va.clone(), &url, &token);
    a2.sync_once().unwrap();
    assert!(va.list_pages(None, 0, 1000).unwrap().iter().any(|p| p.id == p2));
    drop(server);
}

#[test]
fn a_session_is_renewed_after_a_hub_restart() {
    let (th, ta) = (TempDir::new("dkc_restart_hub"), TempDir::new("dkc_restart_a"));
    let hub_vault = th.open();
    let hub = Hub::new(hub_vault.clone()).unwrap();
    let server = hub.serve("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.addr());
    let token = hub.pair_legacy_token("phone").unwrap();
    let va = ta.open();
    let a = SyncClient::new(va.clone(), &url, &token);
    a.sync_once().unwrap();

    // The hub forgets every session, as a restart does; the client's
    // cached session gets a 401 and renews it with a new proof.
    hub.drop_sessions();
    let page = hub_vault.create_page(None, "After restart".into()).unwrap();
    a.sync_once().unwrap();
    assert!(va.list_pages(None, 0, 1000).unwrap().iter().any(|p| p.id == page));
    server.stop();
    drop(hub);
    drop(hub_vault);

    // A new hub process for the same vault (a new address, so a new
    // provider): the client proves its bound key without upgrading.
    let hub2 = Hub::new(th.open()).unwrap();
    let server2 = hub2.serve("127.0.0.1:0").unwrap();
    let url2 = format!("http://{}", server2.addr());
    let b = SyncClient::new(va.clone(), &url2, &token);
    let page2 = hub2.vault().create_page(None, "New process".into()).unwrap();
    b.sync_once().unwrap();
    assert!(va.list_pages(None, 0, 1000).unwrap().iter().any(|p| p.id == page2));
    assert_eq!(changes_status(&hub2, &token, &va.device_id()), 401);
}

#[test]
fn a_redeemed_pairing_code_syncs_on_sessions_from_the_start() {
    let (th, ta) = (TempDir::new("dkc_pair_hub"), TempDir::new("dkc_pair_a"));
    let hub_vault = th.open();
    let hub = Hub::new(hub_vault.clone()).unwrap();
    let server = hub.serve("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.addr());
    let code = hub.pair("tablet").unwrap();
    let va = ta.open();
    let (token, label) = vault_sync::session::redeem_pairing_code(
        &ta.root(),
        &va.device_id(),
        &url,
        &code,
        vault_sync::tls::Trust::new(&[]),
    )
    .unwrap();
    assert_eq!(label, "tablet");
    assert_eq!(changes_status(&hub, &token, &va.device_id()), 401);
    let page = hub_vault.create_page(None, "Paired".into()).unwrap();
    SyncClient::new(va.clone(), &url, &token).sync_once().unwrap();
    assert!(va.list_pages(None, 0, 1000).unwrap().iter().any(|p| p.id == page));
    // The code worked once.
    assert!(vault_sync::session::redeem_pairing_code(
        &ta.root(),
        &va.device_id(),
        &url,
        &code,
        vault_sync::tls::Trust::new(&[]),
    )
    .is_err());
}
