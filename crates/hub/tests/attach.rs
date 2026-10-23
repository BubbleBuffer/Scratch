//! attach_file on the hub: over /mcp with an agent token, `data` larger than
//! the ordinary request limit is accepted, `path` is refused (the hub never
//! reads its own disk for a caller), and an attached blob syncs to devices
//! and reads back in read_page there. A device's own attachment syncs to a
//! second device through the hub as well.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, Server};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use vault_core::{new_id, Vault};
use vault_sync::SyncClient;

const PNG: &[u8] =
    b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_attach_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.0.to_string_lossy().into_owned()).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    server: Server,
    client: Client,
    url: String,
    base: String,
    write: String,
    read: String,
    _data: TempDir,
}

impl Env {
    async fn new() -> Env {
        let data = TempDir::new("hub");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            listen: listener.local_addr().unwrap().to_string(),
            data_dir: data.0.clone(),
            trust_proxy: false,
            agent: AgentSection {
                enabled: false,
                ..AgentSection::default()
            },
            ..Config::default()
        };
        let store = AgentTokens::open(config.vault_dir()).unwrap();
        let write = store.add("writer", AgentScope::Write).unwrap();
        let read = store.add("reader", AgentScope::Read).unwrap();
        let server = notes_hub::start_on(listener, config).await.unwrap();
        let base = format!("http://{}", server.addr());
        Env {
            url: format!("{base}/mcp"),
            base,
            server,
            client: Client::new(),
            write,
            read,
            _data: data,
        }
    }

    async fn post(&self, token: &str, name: &str, arguments: Value) -> (StatusCode, Value) {
        let r = self
            .client
            .post(&self.url)
            .bearer_auth(token)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":name,"arguments":arguments}}))
            .send()
            .await
            .unwrap();
        let status = r.status();
        let body = r.text().await.unwrap();
        (
            status,
            serde_json::from_str(&body).unwrap_or(Value::String(body)),
        )
    }

    async fn ok(&self, name: &str, arguments: Value) -> String {
        let (status, reply) = self.post(&self.write, name, arguments).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn fails(&self, name: &str, arguments: Value) -> String {
        let (status, reply) = self.post(&self.write, name, arguments).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The value after `key: ` on its own line of a tool result.
fn line<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")))
        .unwrap_or_else(|| panic!("no {key} in {text}"))
}

fn attachment_id(text: &str) -> String {
    line(text, "attachment")
        .rsplit('/')
        .next()
        .unwrap()
        .to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

async fn sync_and_fetch(vault: &Arc<Vault>, url: &str, token: &str) {
    let (vault, url, token) = (vault.clone(), url.to_string(), token.to_string());
    blocking(move || {
        let c = SyncClient::new(vault, &url, &token);
        c.sync_once().unwrap();
        c.fetch_missing_attachments().unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn path_is_refused_on_the_hub() {
    let env = Env::new().await;
    env.ok("create_page", json!({"title":"Pics"})).await;
    let file = TempDir::new("file");
    let p = file.0.join("cat.png");
    fs::write(&p, PNG).unwrap();
    let e = env
        .fails(
            "attach_file",
            json!({"page":"Pics","path":p.to_string_lossy()}),
        )
        .await;
    assert!(e.ends_with(vault_tools::attach::PATH_NOT_HERE), "{e}");
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn read_tokens_may_not_attach() {
    let env = Env::new().await;
    env.ok("create_page", json!({"title":"Pics"})).await;
    let (status, reply) = env
        .post(
            &env.read,
            "attach_file",
            json!({"page":"Pics","data":b64(PNG)}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(
        reply.get("error").is_some() || reply["result"]["isError"] == true,
        "{reply}"
    );
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn data_above_the_ordinary_request_limit_is_accepted_and_the_cap_holds() {
    let env = Env::new().await;
    env.ok("create_page", json!({"title":"Files"})).await;
    // 9 MiB of data is about 12 MiB of base64: over the 8 MiB request limit.
    let big = vec![7u8; 9 << 20];
    let text = env
        .ok(
            "attach_file",
            json!({"page":"Files","data":b64(&big),"name":"big.bin"}),
        )
        .await;
    assert!(text.contains("as a file block"), "{text}");
    // Over 10 MB decoded: refused with the cap, not a bare 413.
    let over = vec![7u8; (10 << 20) + 1];
    let e = env
        .fails("attach_file", json!({"page":"Files","data":b64(&over)}))
        .await;
    assert!(e.contains("10 MB"), "{e}");
    // Far over: the body is refused outright.
    let huge = "A".repeat(40 << 20);
    let (status, _) = env
        .post(&env.write, "read_page", json!({"page":"Files","data":huge}))
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_sync_to_devices_and_read_back() {
    let env = Env::new().await;
    let hub = env.server.hub().clone();
    let phone_token = blocking({
        let hub = hub.clone();
        move || hub.pair("phone").unwrap()
    })
    .await;
    let laptop_token = blocking(move || hub.pair("laptop").unwrap()).await;

    // An agent attaches an image on the hub.
    env.ok("create_page", json!({"title":"Trip"})).await;
    let text = env
        .ok(
            "attach_file",
            json!({"page":"Trip","data":b64(PNG),"name":"beach.png","caption":"The beach"}),
        )
        .await;
    let hub_blob = attachment_id(&text);

    // The phone syncs: the block reads back and the blob is fetched.
    let phone_dir = TempDir::new("phone");
    let phone = phone_dir.open();
    sync_and_fetch(&phone, &env.base, &phone_token).await;
    let page = vault_tools::call(&phone, "read_page", &json!({"page":"Trip"})).unwrap();
    assert!(page.contains("The beach"), "{page}");
    let path = phone.attachment_path(hub_blob.clone()).unwrap();
    assert_eq!(fs::read(path).unwrap(), PNG);

    // The phone attaches a file itself; the laptop gets it through the hub.
    let doc = b"%PDF-1.4 not really".to_vec();
    let text = vault_tools::call(
        &phone,
        "attach_file",
        &json!({"page":"Trip","data":b64(&doc),"name":"tickets.pdf"}),
    )
    .unwrap();
    assert!(text.contains("as a file block"), "{text}");
    let phone_blob = attachment_id(&text);
    sync_and_fetch(&phone, &env.base, &phone_token).await;

    let laptop_dir = TempDir::new("laptop");
    let laptop = laptop_dir.open();
    sync_and_fetch(&laptop, &env.base, &laptop_token).await;
    let page = vault_tools::call(&laptop, "read_page", &json!({"page":"Trip"})).unwrap();
    assert!(page.contains("tickets.pdf"), "{page}");
    assert!(page.contains("The beach"), "{page}");
    assert_eq!(
        fs::read(laptop.attachment_path(phone_blob).unwrap()).unwrap(),
        doc
    );
    assert_eq!(
        fs::read(laptop.attachment_path(hub_blob).unwrap()).unwrap(),
        PNG
    );
    env.server.stop().await;
}
