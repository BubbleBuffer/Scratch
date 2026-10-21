//! Compare first-sync HTTP body bytes for one generated vault, with and
//! without compression. HTTP headers and TCP/TLS overhead are not counted.
//!
//! cargo run -p vault_sync --example wire_compression_bench -- [pages]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tiny_http::{Header, Response, Server};
use vault_core::{new_id, seed, version_includes, Vault};
use vault_sync::{Hub, HubRequest, SyncClient, DEVICE_HEADER, NAME_HEADER};

struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn main() {
    let pages: u32 = std::env::args()
        .nth(1)
        .map(|v| v.parse().expect("page count"))
        .unwrap_or(1_000);
    let work = WorkDir(std::env::temp_dir().join(format!("notes-wire-bench-{}", new_id())));
    let hub_root = work.0.join("hub").to_string_lossy().into_owned();
    seed::generate(hub_root.clone(), pages, 0).expect("generate fixture");
    let source = Arc::new(Vault::open(hub_root).unwrap());
    let versions: HashMap<_, _> = source
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    let hub = Hub::new(source).unwrap();
    println!(
        "fixture: seed::generate({pages}, 0), {} total pages",
        versions.len()
    );

    // A legacy adapter passes only the headers known to protocol v1. Its
    // request and response bodies therefore remain exactly the old encoding.
    let legacy = Arc::new(Server::http("127.0.0.1:0").unwrap());
    let legacy_url = format!("http://{}", legacy.server_addr());
    let (server, old_hub) = (legacy.clone(), hub.clone());
    let legacy_thread = std::thread::spawn(move || {
        while let Ok(mut req) = server.recv() {
            let headers = req
                .headers()
                .iter()
                .filter(|h| {
                    ["Authorization", DEVICE_HEADER, NAME_HEADER]
                        .iter()
                        .any(|name| h.field.equiv(*name))
                })
                .map(|h| (h.field.to_string(), h.value.to_string()))
                .collect();
            let body_len = req.body_length().map(|n| n as u64);
            let path = req.url().to_string();
            let reply = old_hub.handle(HubRequest {
                method: req.method().as_str().into(),
                path,
                query: String::new(),
                headers,
                body: Box::new(req.as_reader()),
                body_len,
                cancel: None,
            });
            assert!(reply.headers.is_empty(), "legacy reply has new headers");
            assert_eq!(reply.body.first(), Some(&vault_sync::wire::PROTOCOL));
            let response = Response::from_data(reply.body)
                .with_status_code(reply.status)
                .with_header(Header::from_bytes("Content-Type", reply.content_type).unwrap());
            req.respond(response).unwrap();
        }
    });
    let current = hub.serve("127.0.0.1:0").unwrap();
    let current_url = format!("http://{}", current.addr());
    let mut totals = Vec::new();
    for (label, url) in [("uncompressed", legacy_url), ("zstd", current_url)] {
        let root = work.0.join(label).to_string_lossy().into_owned();
        let vault = Arc::new(Vault::open(root).unwrap());
        let client = SyncClient::new(vault.clone(), &url, &hub.pair(label).unwrap());
        let report = client.sync_once().unwrap();
        assert_eq!(report.created as usize, versions.len());
        for page in vault.page_versions().unwrap() {
            assert!(version_includes(&page.version, &versions[&page.page]).unwrap());
        }
        let total = report.bytes_up + report.bytes_down;
        totals.push(total);
        println!(
            "{label}: down={} up={} total={} bytes, rounds={}",
            report.bytes_down, report.bytes_up, total, report.rounds
        );
    }
    legacy.unblock();
    legacy_thread.join().unwrap();
    current.stop();
    assert!(totals[1] < totals[0]);
    println!(
        "saved {:.2}%",
        100.0 * (1.0 - totals[1] as f64 / totals[0] as f64)
    );
}
