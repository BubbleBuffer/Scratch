//! `/agent-tokens`: listing, creating and revoking agent tokens over HTTP
//! with a device token, against the real router (`notes_hub::start_on`).

use std::fs;
use std::path::PathBuf;

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, Server};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use vault_core::new_id;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    server: Server,
    store: AgentTokens,
    client: Client,
    base: String,
    /// A device token, bound to `DEVICE` on first use.
    device_token: String,
    data: TempDir,
}

const DEVICE: &str = "phone-device-1";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

impl Env {
    async fn new() -> Env {
        let data = TempDir(std::env::temp_dir().join(format!("notes_hub_tokapi_{}", new_id())));
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
        let server = notes_hub::start_on(listener, config).await.unwrap();
        let hub = server.hub().clone();
        let device_token = tokio::task::spawn_blocking(move || hub.pair_legacy_token("phone"))
            .await
            .unwrap()
            .unwrap();
        let base = format!("http://{}", server.addr());
        Env {
            server,
            store,
            client: Client::new(),
            base,
            device_token,
            data,
        }
    }

    fn as_device(&self, rb: RequestBuilder) -> RequestBuilder {
        rb.bearer_auth(&self.device_token).header("X-Device", DEVICE)
    }

    fn get(&self) -> RequestBuilder {
        self.client.get(format!("{}/agent-tokens", self.base))
    }

    fn post(&self, body: Value) -> RequestBuilder {
        self.client
            .post(format!("{}/agent-tokens", self.base))
            .json(&body)
    }

    fn delete(&self, name: &str) -> RequestBuilder {
        self.client
            .delete(format!("{}/agent-tokens/{name}", self.base))
    }

    async fn list(&self) -> Value {
        let r = self.as_device(self.get()).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["cache-control"], "no-store");
        r.json().await.unwrap()
    }

    async fn mcp_status(&self, token: &str) -> StatusCode {
        self.client
            .post(format!("{}/mcp", self.base))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
            .send()
            .await
            .unwrap()
            .status()
    }

    fn file(&self) -> PathBuf {
        self.data
            .0
            .join("vault")
            .join(".vault")
            .join("hub")
            .join("agent-tokens.json")
    }
}

#[tokio::test]
async fn a_device_lists_creates_and_revokes() {
    let env = Env::new().await;
    let empty = env.list().await;
    assert_eq!(empty["tokens"], json!([]));
    assert_eq!(empty["expiry_choices_days"], json!([null, 30, 90, 365]));
    assert_eq!(empty["default_expiry_days"], json!(90));

    // Create: the secret comes back once, and only its hash is stored.
    let r = env
        .as_device(env.post(json!({"name": "laptop-claude", "scope": "write", "expires_days": 30})))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let made: Value = r.json().await.unwrap();
    let secret = made["token"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 64);
    assert_eq!(made["name"], "laptop-claude");
    assert_eq!(made["scope"], "write");
    let created = made["created"].as_i64().unwrap();
    assert_eq!(made["expires"].as_i64().unwrap() - created, 30 * DAY_MS);
    assert!(!fs::read_to_string(env.file()).unwrap().contains(&secret));

    // The new token works on /mcp and shows up in the list, without a secret.
    assert_eq!(env.mcp_status(&secret).await, StatusCode::OK);
    let listed = env.list().await;
    let tokens = listed["tokens"].as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    let t = &tokens[0];
    assert_eq!(t["name"], "laptop-claude");
    assert_eq!(t["created"], created);
    assert_eq!(t["expired"], false);
    assert!(t["last_used"].as_i64().unwrap() >= created, "{t}");
    assert!(t.get("token").is_none() && t.get("sha256").is_none());
    assert!(!listed.to_string().contains(&secret));

    // Default lifetime and never.
    let r = env
        .as_device(env.post(json!({"name": "default-life", "scope": "read"})))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        v["expires"].as_i64().unwrap() - v["created"].as_i64().unwrap(),
        90 * DAY_MS
    );
    let r = env
        .as_device(env.post(json!({"name": "forever", "scope": "read", "expires_days": null})))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["expires"], Value::Null);

    // Revoke: /mcp refuses it at once, and a second revoke is a 404.
    let r = env.as_device(env.delete("laptop-claude")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(env.mcp_status(&secret).await, StatusCode::UNAUTHORIZED);
    let r = env.as_device(env.delete("laptop-claude")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let names: Vec<String> = env.list().await["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["default-life", "forever"]);
    env.server.stop().await;
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let env = Env::new().await;
    for (body, error) in [
        (json!({"name": "Bad Name", "scope": "read"}), "invalid_name"),
        (json!({"name": "ok", "scope": "admin"}), "invalid_scope"),
        (json!({"name": "ok", "scope": "read", "expires_days": 7}), "invalid_expiry"),
        (json!({"name": "ok", "scope": "read", "extra": 1}), "invalid_body"),
        (json!({"scope": "read"}), "invalid_body"),
    ] {
        let r = env.as_device(env.post(body.clone())).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{body}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"], error, "{body}");
    }
    let ok = json!({"name": "dup", "scope": "read"});
    let r = env.as_device(env.post(ok.clone())).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let r = env.as_device(env.post(ok)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(env.list().await["tokens"].as_array().unwrap().len(), 1);
    env.server.stop().await;
}

#[tokio::test]
async fn only_device_tokens_may_manage_tokens() {
    let env = Env::new().await;
    let agent_write = env.store.add("existing-writer", AgentScope::Write).unwrap();
    let agent_read = env.store.add("existing-reader", AgentScope::Read).unwrap();
    // Shaped like an OAuth JWT; these routes never consult OAuth.
    let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln";
    let body = json!({"name": "sneaky", "scope": "write"});

    let attempts = |rb: RequestBuilder| async move { rb.send().await.unwrap().status() };
    for bearer in [agent_write.as_str(), agent_read.as_str(), jwt, "not-a-token"] {
        for device in [DEVICE, "agent"] {
            let with = |rb: RequestBuilder| rb.bearer_auth(bearer).header("X-Device", device);
            assert_eq!(attempts(with(env.get())).await, StatusCode::UNAUTHORIZED);
            assert_eq!(attempts(with(env.post(body.clone()))).await, StatusCode::UNAUTHORIZED);
            assert_eq!(
                attempts(with(env.delete("existing-writer"))).await,
                StatusCode::UNAUTHORIZED
            );
        }
    }
    // No bearer at all.
    assert_eq!(attempts(env.get()).await, StatusCode::UNAUTHORIZED);
    // A device token without its device id is refused like on /sync.
    let r = env
        .get()
        .bearer_auth(&env.device_token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    // Bound to DEVICE on first use, so another device id is refused.
    env.list().await;
    let r = env
        .get()
        .bearer_auth(&env.device_token)
        .header("X-Device", "someone-else")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error(), "{}", r.status());
    assert_ne!(r.status(), StatusCode::OK);

    // Nothing changed.
    let names: Vec<String> = env
        .store
        .list()
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, vec!["existing-writer", "existing-reader"]);
    env.server.stop().await;
}

#[tokio::test]
async fn expired_tokens_are_refused_on_mcp() {
    let env = Env::new().await;
    let r = env
        .as_device(env.post(json!({"name": "short", "scope": "read", "expires_days": 30})))
        .send()
        .await
        .unwrap();
    let made: Value = r.json().await.unwrap();
    let secret = made["token"].as_str().unwrap().to_string();
    assert_eq!(env.mcp_status(&secret).await, StatusCode::OK);

    // Time passes: move the expiry into the past on disk.
    let expires = made["expires"].as_i64().unwrap();
    let text = fs::read_to_string(env.file())
        .unwrap()
        .replace(&format!("\"expires\": {expires}"), "\"expires\": 1000");
    assert!(text.contains("\"expires\": 1000"));
    fs::write(env.file(), text).unwrap();

    assert_eq!(env.mcp_status(&secret).await, StatusCode::UNAUTHORIZED);
    let listed = env.list().await;
    assert_eq!(listed["tokens"][0]["name"], "short");
    assert_eq!(listed["tokens"][0]["expired"], true);
    env.server.stop().await;
}
