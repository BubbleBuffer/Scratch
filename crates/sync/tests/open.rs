//! The `/open/<page id>[?block=<block id>]` route of the desktop's embedded
//! hub: no token, a bounce page to `notes://page/...`, and 400 for anything
//! that is not a vault id. The standalone hub's copy is tested in
//! `crates/hub/tests/open.rs`.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use vault_core::{new_id, Vault};
use vault_sync::{Hub, HubRequest, HubResponse};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_sync_open_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const PAGE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const BLOCK: &str = "01BX5ZZKBKACTAV9WEVGEMMVRY";

fn hub_for(dir: &TempDir) -> Arc<Hub> {
    let vault = Arc::new(Vault::open(dir.0.to_string_lossy().into_owned()).expect("open"));
    Hub::new(vault).unwrap()
}

/// `GET <path>` with no headers, so no token.
fn get(hub: &Hub, target: &str) -> HubResponse {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    hub.handle(HubRequest {
        method: "GET".into(),
        path: path.into(),
        query: query.into(),
        headers: Vec::new(),
        body: Box::new(std::io::empty()),
        body_len: None,
        cancel: None,
    })
}

const BAD: &[&str] = &[
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FA",
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FAVX",
    "/open/01arz3ndektsv4rrffq69g5fav",
    "/open/81ARZ3NDEKTSV4RRFFQ69G5FAV",
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FAI",
    "/open/%3Cscript%3E",
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FAV?block=bad",
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FAV?block=",
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FAV?block=01ARZ3NDEKTSV4RRFFQ69G5FAI",
    "/open/01ARZ3NDEKTSV4RRFFQ69G5FAV?block=%22%3E%3Cscript%3E",
];

#[test]
fn open_route_bounces_vault_ids_without_a_token() {
    let dir = TempDir::new();
    let hub = hub_for(&dir);

    // A page: 200 HTML, the notes:// target in the script, the meta refresh
    // and the visible link. No Authorization header is sent.
    let r = get(&hub, &format!("/open/{PAGE}"));
    assert_eq!(r.status, 200);
    assert_eq!(r.content_type, "text/html; charset=utf-8");
    assert!(r
        .headers
        .contains(&("Cache-Control".into(), "no-store".into())));
    let body = String::from_utf8(r.body).unwrap();
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

    // A block: the target carries it.
    let r = get(&hub, &format!("/open/{PAGE}?block={BLOCK}"));
    assert_eq!(r.status, 200);
    let body = String::from_utf8(r.body).unwrap();
    assert!(
        body.contains(&format!(
            "location.replace(\"notes://page/{PAGE}?block={BLOCK}\")"
        )),
        "{body}"
    );

    // Anything that is not a vault id is refused, page or block.
    for path in BAD {
        let r = get(&hub, path);
        assert_eq!(r.status, 400, "{path}");
        assert_eq!(r.content_type, "text/plain; charset=utf-8", "{path}");
    }

    // The route is one segment: a longer path is not the bounce page.
    assert_ne!(get(&hub, &format!("/open/{PAGE}/x")).status, 200);
}

#[test]
fn open_route_is_served_over_http() {
    let dir = TempDir::new();
    let hub = hub_for(&dir);
    let server = hub.serve("127.0.0.1:0").unwrap();
    let addr = server.addr();
    let raw = |target: &str| -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        write!(
            s,
            "GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };

    let ok = raw(&format!("/open/{PAGE}?block={BLOCK}"));
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(
        ok.to_ascii_lowercase()
            .contains("content-type: text/html; charset=utf-8"),
        "{ok}"
    );
    assert!(
        ok.contains(&format!("notes://page/{PAGE}?block={BLOCK}")),
        "{ok}"
    );

    let bad = raw(&format!("/open/{PAGE}?block=x"));
    assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");

    server.stop();
}
