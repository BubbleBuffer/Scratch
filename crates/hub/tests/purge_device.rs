//! The purging device's id through the notes-hub axum router: `/health`
//! advertises both purge capabilities, the id from a laptop's purge reaches
//! the hub's vault and the phone, and a malformed flag is refused over HTTP.
//! The server is `notes_hub::start_on`, and devices are real `SyncClient`s.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use notes_hub::{start_on, AgentSection, Config, OidcConfig, Server};
use tokio::net::TcpListener;
use vault_core::{new_id, Vault};
use vault_sync::wire::PURGE_PROTOCOL;
use vault_sync::{
    Hub, SyncClient, SyncError, DEVICE_HEADER, PURGE_CAPABILITY, PURGE_CAPABILITY_HEADER,
    PURGE_DEVICE_CAPABILITY, PURGE_DEVICE_CAPABILITY_HEADER,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_purge_device_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.0.to_string_lossy().into_owned()).expect("open"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config(data: &TempDir) -> Config {
    Config {
        mcp_agent_enabled: false,
        mcp_agent_default_host: "hub".into(),
        listen: "127.0.0.1:0".into(),
        data_dir: data.0.clone(),
        public_url: "http://127.0.0.1".into(),
        trust_proxy: false,
        // No issuer: the tokens come from `pair` on the running server's hub.
        oidc: OidcConfig::default(),
        agent: AgentSection {
            enabled: false,
            ..AgentSection::default()
        },
        import_source_limit_bytes: Config::default().import_source_limit_bytes,
        schedules: Default::default(),
        mcp_oauth: Default::default(),
        ..Config::default()
    }
}

async fn start(config: &Config) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = start_on(listener, config.clone()).await.unwrap();
    // The config's public_url is a fake, so proofs name the Host instead.
    server.hub().set_key_policy(vault_sync::KeyPolicy {
        public_url: None,
        require_device_keys: false,
    });
    server
}

/// Runs blocking work off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task")
}

/// One `SyncClient` sync over HTTP, from `vault`.
async fn sync_as(vault: &Arc<Vault>, url: &str, token: &str) -> Result<(), SyncError> {
    let (vault, url, token) = (vault.clone(), url.to_string(), token.to_string());
    blocking(move || SyncClient::new(vault, &url, &token).sync_once().map(|_| ())).await
}

#[tokio::test(flavor = "multi_thread")]
async fn health_advertises_the_purge_capabilities() {
    let data = TempDir::new("health");
    let server = start(&config(&data)).await;
    let url = format!("http://{}", server.addr());

    let health = reqwest::get(format!("{url}/health")).await.unwrap();
    let header = |name: &str| {
        health
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    assert_eq!(header(PURGE_CAPABILITY_HEADER).as_deref(), Some(PURGE_CAPABILITY));
    assert_eq!(
        header(PURGE_DEVICE_CAPABILITY_HEADER).as_deref(),
        Some(PURGE_DEVICE_CAPABILITY)
    );

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_purging_device_id_is_kept_and_forwarded_by_the_hub() {
    let data = TempDir::new("hub");
    let server = start(&config(&data)).await;
    let hub: Arc<Hub> = server.hub().clone();
    let url = format!("http://{}", server.addr());

    let (laptop_token, phone_token) = {
        let hub = hub.clone();
        blocking(move || (hub.pair_legacy_token("laptop").unwrap(), hub.pair_legacy_token("phone").unwrap())).await
    };
    let laptop = TempDir::new("laptop");
    let phone = TempDir::new("phone");
    let (laptop_vault, phone_vault) = (laptop.open(), phone.open());
    let laptop_id = laptop_vault.device_id();

    // A page made on the laptop, synced to the hub and the phone; then the
    // laptop purges it.
    let t = laptop_vault.create_page(None, "T".into()).unwrap();
    sync_as(&laptop_vault, &url, &laptop_token).await.unwrap();
    sync_as(&phone_vault, &url, &phone_token).await.unwrap();
    laptop_vault.delete_page(t.clone()).unwrap();
    laptop_vault.purge_page(t.clone()).unwrap();
    sync_as(&laptop_vault, &url, &laptop_token).await.unwrap();

    let stored = hub.vault().purge_records();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].page, t);
    assert_eq!(stored[0].device.as_deref(), Some(laptop_id.as_str()));

    // The phone receives the purge through the axum router, with the id.
    sync_as(&phone_vault, &url, &phone_token).await.unwrap();
    assert!(phone_vault.purged_pages().contains(&t));
    let forwarded = phone_vault.purge_records();
    let received = forwarded.iter().find(|record| record.page == t).unwrap();
    assert_eq!(received.device.as_deref(), Some(laptop_id.as_str()));

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_device_flag_is_refused_over_http() {
    let data = TempDir::new("flag");
    let server = start(&config(&data)).await;
    let hub = server.hub().clone();
    let url = format!("http://{}", server.addr());

    let token = {
        let hub = hub.clone();
        blocking(move || hub.pair_legacy_token("probe").unwrap()).await
    };
    let probe = TempDir::new("probe");
    let device = probe.open().device_id();
    let resp = reqwest::Client::new()
        .post(format!("{url}/sync"))
        .header("Authorization", format!("Bearer {token}"))
        .header(DEVICE_HEADER, device)
        .header(PURGE_DEVICE_CAPABILITY_HEADER, "2")
        .body(vec![PURGE_PROTOCOL])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let text = resp.text().await.unwrap();
    assert!(text.contains(PURGE_DEVICE_CAPABILITY_HEADER), "{text}");

    server.stop().await;
}
