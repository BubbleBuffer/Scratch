//! Sync timings: a full sync of a seeded vault into an empty one, through
//! the core hooks directly and through a hub over HTTP on localhost, then
//! delta syncs.
//!
//! cargo run -p vault_sync --release --example sync_bench -- <work dir> [pages] [big page blocks]
//!
//! Seeds `<work dir>/hub` if it is empty (10,000 pages and a 2,000-block
//! page by default) and reuses it on later runs. The receiving vaults are
//! created fresh under `<work dir>` and removed at the end.
//!
//! With `--features storage-metrics`, each phase also prints one
//! `phase-json {...}` line after all timings: `core_export`, `core_import`
//! (import and flush) and `http_first_sync`. Each report covers only its
//! phase. The HTTP phase's report includes the hub's spans, which run on
//! other threads, so its `client.http.send` span contains the server time.
//! Do not sum overlapping spans.
//! `--only core-export` runs just the core export (no import, no hub).
//! `--only http-first-sync` runs just the HTTP first sync, with no core
//! export first, and skips the deltas. The default runs everything.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use vault_core::{seed, version_includes, BlockOp, PageUpdate, Vault};
use vault_sync::{Hub, SyncClient};

type Phases = Vec<(&'static str, serde_json::Value)>;

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

fn dir_size(p: &std::path::Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let m = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            n += if m.is_dir() {
                dir_size(&e.path())
            } else {
                m.len()
            };
        }
    }
    n
}

/// `--only <phase>`: removes it from `args` and returns the phase.
#[cfg(feature = "storage-metrics")]
fn take_only(args: &mut Vec<String>) -> Option<String> {
    let i = args.iter().position(|a| a == "--only")?;
    args.remove(i);
    let phase = args.get(i).cloned().expect("--only needs a phase");
    args.remove(i);
    assert!(
        matches!(phase.as_str(), "core-export" | "http-first-sync"),
        "--only takes core-export or http-first-sync"
    );
    Some(phase)
}

#[cfg(not(feature = "storage-metrics"))]
fn take_only(args: &mut Vec<String>) -> Option<String> {
    assert!(
        !args.iter().any(|a| a == "--only"),
        "--only needs --features storage-metrics"
    );
    None
}

#[cfg(feature = "storage-metrics")]
fn reset_metrics() {
    vault_core::storage_metrics::reset();
}

#[cfg(not(feature = "storage-metrics"))]
fn reset_metrics() {}

#[cfg(feature = "storage-metrics")]
fn take_metrics(phases: &mut Phases, phase: &'static str) {
    phases.push((phase, vault_core::storage_metrics::report()));
}

#[cfg(not(feature = "storage-metrics"))]
fn take_metrics(_: &mut Phases, _: &'static str) {}

/// One machine-readable line per phase; empty without the feature.
fn print_phases(phases: &Phases) {
    for (phase, report) in phases {
        println!(
            "phase-json {}",
            serde_json::json!({ "phase": phase, "metrics": report })
        );
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let only = take_only(&mut args);
    let mut args = args.into_iter();
    let work = std::path::PathBuf::from(
        args.next()
            .expect("usage: sync_bench <work dir> [pages] [big page blocks]"),
    );
    let pages: u32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(10_000);
    let big: u32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(2_000);
    let core_export = only.as_deref() != Some("http-first-sync");
    let core_import = only.is_none();
    let http = only.as_deref() != Some("core-export");
    let deltas = only.is_none();
    let mut phases: Phases = Vec::new();

    let hub_dir = work.join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let hub_root = hub_dir.to_string_lossy().into_owned();
    if Vault::open(hub_root.clone()).unwrap().page_count().unwrap() == 0 {
        let t = Instant::now();
        seed::generate(hub_root.clone(), pages, big).unwrap();
        println!(
            "seed {pages} pages + big page of {big} blocks: {:.1} s",
            secs(t)
        );
    }
    let hub_vault = Arc::new(Vault::open(hub_root).unwrap());
    let n = hub_vault.page_versions().unwrap().len();
    println!(
        "hub vault: {n} pages, {:.1} MB on disk",
        dir_size(&hub_dir) as f64 / 1e6
    );

    // 1. Core hooks only: export every page, import into an empty vault.
    let core_dir = work.join(format!("core-{}", vault_core::new_id()));
    if core_export {
        let dst =
            core_import.then(|| Vault::open(core_dir.to_string_lossy().into_owned()).unwrap());
        reset_metrics();
        let t = Instant::now();
        let versions = hub_vault.page_versions().unwrap();
        let mut bytes = 0usize;
        let updates: Vec<PageUpdate> = versions
            .into_iter()
            .map(|v| {
                let b = hub_vault.export_updates(v.page.clone(), vec![]).unwrap();
                bytes += b.len();
                PageUpdate {
                    page: v.page,
                    bytes: b,
                }
            })
            .collect();
        let t_export = secs(t);
        take_metrics(&mut phases, "core_export");
        match dst {
            Some(dst) => {
                reset_metrics();
                let t2 = Instant::now();
                for chunk in updates.chunks(500) {
                    dst.import_updates_batch(chunk.to_vec()).unwrap();
                }
                let t_import = secs(t2);
                let t3 = Instant::now();
                dst.flush().unwrap();
                println!(
                    "core full sync of {n} pages: export {t_export:.2} s ({:.1} MB), import {t_import:.2} s, flush {:.2} s",
                    bytes as f64 / 1e6,
                    secs(t3)
                );
                take_metrics(&mut phases, "core_import");
                assert_eq!(dst.page_count().unwrap(), hub_vault.page_count().unwrap());
            }
            None => println!(
                "core export of {n} pages: {t_export:.2} s ({:.1} MB)",
                bytes as f64 / 1e6
            ),
        }
    }
    let _ = std::fs::remove_dir_all(&core_dir);

    // 2. Through the hub over HTTP.
    if http {
        let hub = Hub::new(hub_vault.clone()).unwrap();
        let server = hub.serve("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.addr());
        let client_dir = work.join(format!("client-{}", vault_core::new_id()));
        let client_vault =
            Arc::new(Vault::open(client_dir.to_string_lossy().into_owned()).unwrap());
        let client = SyncClient::new(client_vault.clone(), &url, &hub.pair("bench").unwrap());
        reset_metrics();
        let t = Instant::now();
        let r = client.sync_once().unwrap();
        let t_sync = secs(t);
        take_metrics(&mut phases, "http_first_sync");
        let t2 = Instant::now();
        client_vault.flush().unwrap();
        println!(
            "hub full sync of {n} pages into an empty vault: {t_sync:.2} s ({} rounds, {:.1} MB down, {:.2} MB up), then flush {:.2} s",
            r.rounds,
            r.bytes_down as f64 / 1e6,
            r.bytes_up as f64 / 1e6,
            secs(t2)
        );
        assert_eq!(r.created as usize, n);
        // Every version matches.
        let hv: HashMap<String, Vec<u8>> = hub_vault
            .page_versions()
            .unwrap()
            .into_iter()
            .map(|v| (v.page, v.version))
            .collect();
        for v in client_vault.page_versions().unwrap() {
            assert!(version_includes(&v.version, &hv[&v.page]).unwrap());
        }
        println!(
            "client vault after sync: {:.1} MB on disk",
            dir_size(&client_dir) as f64 / 1e6
        );

        if deltas {
            let mut times = Vec::new();
            for _ in 0..20 {
                let t = Instant::now();
                let r = client.sync_once().unwrap();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                assert!(!r.full && r.created == 0 && r.changed == 0);
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "delta sync, nothing changed: p50 {:.1} ms, max {:.1} ms (n=20)",
                times[10], times[19]
            );

            let page = hub_vault.list_pages(None, 0, 1).unwrap()[0].id.clone();
            let block = hub_vault.load_page(page.clone()).unwrap().blocks[0]
                .id
                .clone();
            let mut pull = Vec::new();
            let mut push = Vec::new();
            for i in 0..20 {
                hub_vault
                    .apply(
                        page.clone(),
                        vec![BlockOp::SetText {
                            id: block.clone(),
                            text: format!("hub edit {i}"),
                            marks: vec![],
                        }],
                    )
                    .unwrap();
                let t = Instant::now();
                let r = client.sync_once().unwrap();
                pull.push(t.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(r.changed, 1);
                client_vault
                    .apply(
                        page.clone(),
                        vec![BlockOp::SetText {
                            id: block.clone(),
                            text: format!("client edit {i}"),
                            marks: vec![],
                        }],
                    )
                    .unwrap();
                let t = Instant::now();
                let r = client.sync_once().unwrap();
                push.push(t.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(r.pushed, 1);
            }
            pull.sort_by(|a, b| a.partial_cmp(b).unwrap());
            push.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "delta sync, one page edited on the hub: p50 {:.1} ms, max {:.1} ms (n=20)",
                pull[10], pull[19]
            );
            println!(
                "delta sync, one page edited on the client: p50 {:.1} ms, max {:.1} ms (n=20)",
                push[10], push[19]
            );
            // Leave the hub vault as seeded (text restored below is the last
            // edit; the bench only needs the page count stable).
        }
        drop(client);
        drop(client_vault);
        server.stop();
        let _ = std::fs::remove_dir_all(&client_dir);
    }
    print_phases(&phases);
}
