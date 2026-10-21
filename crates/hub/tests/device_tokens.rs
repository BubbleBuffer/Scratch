//! The pairing tokens of a running notes-hub, in one process. The server is
//! `notes_hub::start_on`: the real axum router, with the sync protocol served
//! by the hub. The CLI side is `notes_hub::open_hub` on the same data dir,
//! the call `notes-hub --pair/--unpair/--devices` make, so it is a separate
//! `Hub` with its own `Tokens`. Clients are real `SyncClient`s over HTTP. Every
//! blocking call (file locks, sync) runs on `spawn_blocking`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use notes_hub::{open_hub, start_on, AgentSection, Config, OidcConfig, Server};
use serde_json::Value;
use tokio::net::TcpListener;
use vault_core::{new_id, sha256_hex, Vault};
use vault_sync::{Hub, SyncClient, SyncError};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_devtok_{tag}_{}", new_id()));
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

/// `<data_dir>/vault/.vault/hub/tokens.json`, the server's vault.
fn tokens_file(data: &TempDir) -> PathBuf {
    data.0
        .join("vault")
        .join(".vault")
        .join("hub")
        .join("tokens.json")
}

/// The entries on disk, as (label, bound device) pairs.
fn on_disk(data: &TempDir) -> Vec<(String, Option<String>)> {
    let v: Vec<Value> = serde_json::from_slice(&fs::read(tokens_file(data)).unwrap()).unwrap();
    v.iter()
        .map(|e| {
            (
                e["label"].as_str().unwrap().to_string(),
                e["device"].as_str().map(str::to_string),
            )
        })
        .collect()
}

fn labels(data: &TempDir) -> Vec<String> {
    on_disk(data).into_iter().map(|(l, _)| l).collect()
}

/// The file no longer holds `token`'s hash or a `doomed` label.
fn assert_absent(data: &TempDir, token: &str, label: &str) {
    let text = fs::read_to_string(tokens_file(data)).unwrap();
    assert!(
        !text.contains(&sha256_hex(token.as_bytes())),
        "hash on disk"
    );
    assert!(!labels(data).iter().any(|l| l == label), "{label} on disk");
}

fn config(data: &TempDir) -> Config {
    Config {
        mcp_agent_enabled: false,
        mcp_agent_default_host: "hub".into(),
        listen: "127.0.0.1:0".into(),
        data_dir: data.0.clone(),
        public_url: "http://127.0.0.1".into(),
        trust_proxy: false,
        import_source_limit_bytes: Config::default().import_source_limit_bytes,
        schedules: Default::default(),
        mcp_oauth: Default::default(),
        // No issuer: device login is off, and the tokens come from the CLI
        // and from `login_device` in-process.
        oidc: OidcConfig::default(),
        agent: AgentSection {
            enabled: false,
            ..AgentSection::default()
        },
        ..Config::default()
    }
}

async fn start(config: &Config) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    start_on(listener, config.clone()).await.unwrap()
}

/// Runs blocking work off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task")
}

/// A call on the running server's own `Hub`.
async fn on_hub<T: Send + 'static>(
    hub: &Arc<Hub>,
    f: impl FnOnce(&Hub) -> T + Send + 'static,
) -> T {
    let hub = hub.clone();
    blocking(move || f(&hub)).await
}

/// One `SyncClient` sync over HTTP, from a vault of its own.
async fn sync_as(vault: &Arc<Vault>, url: &str, token: &str) -> Result<(), SyncError> {
    let (vault, url, token) = (vault.clone(), url.to_string(), token.to_string());
    blocking(move || SyncClient::new(vault, &url, &token).sync_once().map(|_| ())).await
}

/// The CLI, as `notes-hub --unpair <label>` runs it.
async fn cli_unpair(config: &Config, label: &str) -> usize {
    let (config, label) = (config.clone(), label.to_string());
    blocking(move || open_hub(&config).unwrap().unpair(&label).unwrap()).await
}

/// The CLI, as `notes-hub --pair <label>` runs it.
async fn cli_pair(config: &Config, label: &str) -> String {
    let (config, label) = (config.clone(), label.to_string());
    blocking(move || open_hub(&config).unwrap().pair(&label).unwrap()).await
}

/// The CLI, as `notes-hub --devices` runs it.
async fn cli_devices(config: &Config) -> Vec<String> {
    let config = config.clone();
    blocking(move || {
        open_hub(&config)
            .unwrap()
            .devices()
            .into_iter()
            .map(|d| d.label)
            .collect()
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_revoke_reaches_the_running_hub_and_stays_revoked() {
    let data = TempDir::new("hub");
    let config = config(&data);
    let server = start(&config).await;
    let url = format!("http://{}", server.addr());
    let served = server.hub().clone();

    let phone_token = on_hub(&served, |h| h.pair("phone")).await.unwrap();
    let laptop_token = on_hub(&served, |h| h.pair("laptop")).await.unwrap();
    let phone_vault = TempDir::new("phone");
    let laptop_vault = TempDir::new("laptop");
    let (phone, laptop) = (phone_vault.open(), laptop_vault.open());

    // The phone's first use binds its token to its device, over HTTP.
    sync_as(&phone, &url, &phone_token).await.unwrap();

    // The CLI revokes the phone; the running hub is not restarted.
    assert_eq!(cli_unpair(&config, "phone").await, 1);
    assert!(matches!(
        sync_as(&phone, &url, &phone_token).await,
        Err(SyncError::Status(401, _))
    ));

    // The laptop's first use binds its token: the server saves the file. The
    // phone's entry must not come back, and its hash must not be on disk.
    sync_as(&laptop, &url, &laptop_token).await.unwrap();
    let entries = on_disk(&data);
    assert_eq!(labels(&data), vec!["laptop"]);
    assert_eq!(entries[0].1.as_deref(), Some(laptop.device_id().as_str()));
    assert_absent(&data, &phone_token, "phone");

    // A login is a save too; the phone stays out.
    let (_, label) = on_hub(&served, |h| h.login_device("tablet-device", "tablet"))
        .await
        .unwrap();
    assert_eq!(label, "tablet");
    assert_eq!(labels(&data), vec!["laptop", "tablet"]);
    assert_absent(&data, &phone_token, "phone");

    // A token the CLI adds works on the running hub without a restart.
    let cli_token = cli_pair(&config, "cli-added").await;
    let cli_vault = TempDir::new("cli");
    sync_as(&cli_vault.open(), &url, &cli_token).await.unwrap();
    assert!(on_disk(&data)
        .iter()
        .any(|(l, d)| l == "cli-added" && d.is_some()));
    assert!(matches!(
        sync_as(&phone, &url, &phone_token).await,
        Err(SyncError::Status(401, _))
    ));

    // The CLI's list and the running hub's list agree with the file.
    assert_eq!(
        cli_devices(&config).await,
        vec!["laptop", "tablet", "cli-added"]
    );
    let served_labels: Vec<String> = on_hub(&served, |h| {
        h.devices().into_iter().map(|d| d.label).collect()
    })
    .await;
    assert_eq!(served_labels, vec!["laptop", "tablet", "cli-added"]);
    server.stop().await;
}

/// A running server whose cache still holds `doomed` after the CLI removed
/// it. Nothing has checked or listed on the server since the CLI's removal,
/// so its cache is stale until its next mutation. `keeper` is a different
/// token, which the scenarios act on.
struct Stale {
    data: TempDir,
    server: Server,
    url: String,
    doomed: String,
    keeper: String,
}

async fn stale(tag: &str) -> Stale {
    let data = TempDir::new(tag);
    let config = config(&data);
    let server = start(&config).await;
    let url = format!("http://{}", server.addr());
    let served = server.hub().clone();
    let doomed = on_hub(&served, |h| h.pair("doomed")).await.unwrap();
    let keeper = on_hub(&served, |h| h.pair("keeper")).await.unwrap();
    assert_eq!(cli_unpair(&config, "doomed").await, 1);
    assert_eq!(labels(&data), vec!["keeper"]);
    Stale {
        data,
        server,
        url,
        doomed,
        keeper,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_server_pair_does_not_bring_back_a_removed_token() {
    let s = stale("stale-pair").await;
    on_hub(s.server.hub(), |h| h.pair("new")).await.unwrap();
    assert_eq!(labels(&s.data), vec!["keeper", "new"]);
    assert_absent(&s.data, &s.doomed, "doomed");
    assert!(on_disk(&s.data).iter().all(|(_, d)| d.is_none()));
    s.server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_server_login_does_not_bring_back_a_removed_token() {
    let s = stale("stale-login").await;
    let (_, label) = on_hub(s.server.hub(), |h| h.login_device("dev-l", "login"))
        .await
        .unwrap();
    assert_eq!(label, "login");
    assert_eq!(labels(&s.data), vec!["keeper", "login"]);
    assert_absent(&s.data, &s.doomed, "doomed");
    s.server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_server_unpair_does_not_bring_back_a_removed_token() {
    let s = stale("stale-unpair").await;
    assert_eq!(
        on_hub(s.server.hub(), |h| h.unpair("keeper"))
            .await
            .unwrap(),
        1
    );
    assert!(labels(&s.data).is_empty());
    assert_absent(&s.data, &s.doomed, "doomed");
    s.server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_server_revoke_does_not_bring_back_a_removed_token() {
    let s = stale("stale-revoke").await;
    let keeper = s.keeper.clone();
    assert!(on_hub(s.server.hub(), move |h| h.sign_out(&keeper))
        .await
        .unwrap());
    assert!(labels(&s.data).is_empty());
    assert_absent(&s.data, &s.doomed, "doomed");
    s.server.stop().await;
}

/// The first-use bind is a request, and a request's check refreshes the
/// cache before it binds, so this scenario does not isolate the bind's own
/// reload from the check's. It shows the bind's save keeps the removed token
/// absent. The bind's reload is covered by the file-level tests, which
/// bind through `check` with a stale cache in the same way.
#[tokio::test(flavor = "multi_thread")]
async fn stale_server_first_use_bind_does_not_bring_back_a_removed_token() {
    let s = stale("stale-bind").await;
    let device = TempDir::new("keeper-device");
    let vault = device.open();
    sync_as(&vault, &s.url, &s.keeper).await.unwrap();
    assert_eq!(labels(&s.data), vec!["keeper"]);
    assert_eq!(
        on_disk(&s.data)[0].1.as_deref(),
        Some(vault.device_id().as_str())
    );
    assert_absent(&s.data, &s.doomed, "doomed");
    s.server.stop().await;
}
