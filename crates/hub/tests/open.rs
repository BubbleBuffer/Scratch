//! The `/open/<page id>[?block=<block id>]` bounce route: no auth, a page
//! that sends the browser to `notes://page/...`, and 400 for anything that is
//! not a vault id.

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

const PAGE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const BLOCK: &str = "01BX5ZZKBKACTAV9WEVGEMMVRY";

#[tokio::test]
async fn open_route_bounces_vault_ids_and_refuses_the_rest() {
    let data = TempDir(std::env::temp_dir().join(format!("notes_hub_open_{}", new_id())));
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
    let server = notes_hub::start_on(listener, config).await.unwrap();
    let base = format!("http://{}", server.addr());
    let client = Client::new();

    // A page: 200 HTML with no auth, the notes:// target in the script,
    // the meta refresh and the visible link.
    let r = client
        .get(format!("{base}/open/{PAGE}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let ctype = r.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ctype.starts_with("text/html"), "{ctype}");
    let body = r.text().await.unwrap();
    assert!(body.contains("<title>Opening in Scratch</title>"), "{body}");
    assert!(
        body.contains(&format!("location.replace(\"notes://page/{PAGE}\")")),
        "{body}"
    );
    assert!(
        body.contains(&format!("content=\"0; url=notes://page/{PAGE}\"")),
        "{body}"
    );
    assert!(
        body.contains(&format!("href=\"notes://page/{PAGE}\">Open in Scratch</a>")),
        "{body}"
    );
    assert!(body.contains("<h1>Open this note</h1>"), "{body}");
    assert!(!body.contains("close this tab"), "{body}");

    // A block: the target carries it.
    let r = client
        .get(format!("{base}/open/{PAGE}?block={BLOCK}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let body = r.text().await.unwrap();
    assert!(
        body.contains(&format!(
            "location.replace(\"notes://page/{PAGE}?block={BLOCK}\")"
        )),
        "{body}"
    );

    // Anything that is not a vault id is refused, page or block.
    let bad_paths = [
        "/open/01ARZ3NDEKTSV4RRFFQ69G5FA".to_string(),
        "/open/01ARZ3NDEKTSV4RRFFQ69G5FAVX".to_string(),
        "/open/01arz3ndektsv4rrffq69g5fav".to_string(),
        "/open/81ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        "/open/01ARZ3NDEKTSV4RRFFQ69G5FAI".to_string(),
        "/open/%3Cscript%3E".to_string(),
        format!("/open/{PAGE}?block=bad"),
        format!("/open/{PAGE}?block="),
        format!("/open/{PAGE}?block=01ARZ3NDEKTSV4RRFFQ69G5FAI"),
        format!("/open/{PAGE}?block=%22%3E%3Cscript%3E"),
    ];
    for path in bad_paths {
        let r = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{path}");
    }

    server.stop().await;
}
