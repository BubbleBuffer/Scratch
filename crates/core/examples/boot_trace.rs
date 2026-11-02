//! Boot timing of Vault::open on a generated vault (boot-timing branch).
//! `cargo run --release -p vault_core --example boot_trace -- <dir> <pages>`
//! Seeds `<dir>` when empty, then opens it three times (the first open is
//! the coldest the OS cache allows), prints the per-step trace
//! (`.vault/boot-timing.log`) and finally an open after deleting the index
//! (what a schema bump costs the first launch after an update).
use std::path::Path;
use std::time::Instant;

fn open_and_print(root: &str, label: &str) {
    let t = Instant::now();
    let v = vault_core::Vault::open(root.to_string()).expect("open");
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("=== {label}: Vault::open wall {ms:.1} ms, pages={}", v.page_count().unwrap());
    let log = std::fs::read_to_string(Path::new(root).join(".vault").join("boot-timing.log")).unwrap_or_default();
    print!("{log}");
    drop(v);
}

fn main() {
    let mut a = std::env::args().skip(1);
    let root = a.next().expect("dir");
    let pages: u32 = a.next().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let empty = !Path::new(&root).join("pages").exists();
    if empty {
        let t = Instant::now();
        vault_core::seed::generate(root.clone(), pages, 2000).expect("seed");
        println!("seeded {pages} pages in {:.1}s", t.elapsed().as_secs_f64());
    }
    open_and_print(&root, "open 1");
    open_and_print(&root, "open 2 (warm)");
    open_and_print(&root, "open 3 (warm)");
    for f in ["index.sqlite", "index.sqlite-wal", "index.sqlite-shm"] {
        let _ = std::fs::remove_file(Path::new(&root).join(".vault").join(f));
    }
    open_and_print(&root, "open after index deleted (rebuild)");
}
