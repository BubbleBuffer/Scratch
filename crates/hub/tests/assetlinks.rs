//! `/.well-known/assetlinks.json` (Digital Asset Links for Android App Links):
//! served without auth only when the Android package and its certificate
//! fingerprints are configured.

use std::fs;
use std::path::PathBuf;

use notes_hub::{AgentSection, Config};
use reqwest::{Client, StatusCode};
use vault_core::new_id;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const FP: &str = "91:B6:FA:29:8D:52:AF:E8:23:FE:D0:D7:74:53:56:E7:6A:A6:88:8E:72:CE:9B:16:04:5D:94:E3:92:E5:86:59";

fn config(data: &TempDir, extra: Config) -> Config {
    Config {
        listen: "127.0.0.1:0".into(),
        data_dir: data.0.clone(),
        trust_proxy: false,
        agent: AgentSection {
            enabled: false,
            ..AgentSection::default()
        },
        ..extra
    }
}

#[tokio::test]
async fn assetlinks_is_served_when_the_android_app_is_configured() {
    let data = TempDir(std::env::temp_dir().join(format!("notes_hub_assetlinks_{}", new_id())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut extra = Config::default();
    extra.android_package = "de.bubblebuffer.notes".into();
    extra.android_sha256_certs = vec![FP.to_ascii_lowercase()];
    let mut cfg = config(&data, extra);
    cfg.listen = listener.local_addr().unwrap().to_string();
    let server = notes_hub::start_on(listener, cfg).await.unwrap();
    let base = format!("http://{}", server.addr());

    let r = Client::new()
        .get(format!("{base}/.well-known/assetlinks.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let ctype = r.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ctype.starts_with("application/json"), "{ctype}");
    let statement: serde_json::Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
    assert_eq!(
        statement,
        serde_json::json!([{
            "relation": ["delegate_permission/common.handle_all_urls"],
            "target": {
                "namespace": "android_app",
                "package_name": "de.bubblebuffer.notes",
                "sha256_cert_fingerprints": [FP],
            }
        }])
    );
}

#[tokio::test]
async fn assetlinks_has_no_route_when_unconfigured() {
    let data = TempDir(std::env::temp_dir().join(format!("notes_hub_assetlinks_{}", new_id())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = config(&data, Config::default());
    cfg.listen = listener.local_addr().unwrap().to_string();
    let server = notes_hub::start_on(listener, cfg).await.unwrap();
    let base = format!("http://{}", server.addr());

    // Not a statement: the request falls through to the sync protocol,
    // which refuses it without a token.
    let r = Client::new()
        .get(format!("{base}/.well-known/assetlinks.json"))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), StatusCode::OK);
    let body = r.text().await.unwrap();
    assert!(!body.contains("delegate_permission"), "{body}");
}
