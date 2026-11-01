//! `Vault::open_deferred`: the recovery scan and the typo-correction word
//! index run in the background after the open. Results match `open`, with the
//! word index built on demand if a query comes first.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use vault_core::*;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        Self(std::env::temp_dir().join(format!("vault_deferred_{tag}_{}", new_id())))
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn seed(t: &Temp) -> PageId {
    let v = Vault::open(t.root()).unwrap();
    let id = v.create_page(None, "Quarterly planning".into()).unwrap();
    v.create_page(None, "Groceries".into()).unwrap();
    v.flush().unwrap();
    id
}

#[test]
fn fuzzy_search_works_before_the_background_warm_up() {
    let t = Temp::new("fuzzy");
    let id = seed(&t);
    let v = Vault::open_deferred(t.root()).unwrap();
    // Immediately: the word index is not warmed yet, the query builds it.
    let hits = v.search("Quartrly".into(), 10).unwrap();
    assert!(hits.iter().any(|h| h.id == id && h.fuzzy), "{hits:?}");
}

#[test]
fn deferred_open_recovers_a_page_missing_from_the_index() {
    let t = Temp::new("recover");
    let id = seed(&t);
    rusqlite::Connection::open(t.0.join(".vault/index.sqlite"))
        .unwrap()
        .execute("DELETE FROM pages WHERE id = ?1", [&id])
        .unwrap();
    let v = Vault::open_deferred(t.root()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if v.search("Quarterly".into(), 10).unwrap().iter().any(|h| h.id == id) {
            break;
        }
        assert!(Instant::now() < deadline, "the background scan never restored the row");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn closing_right_after_a_deferred_open_does_not_wait_for_the_background_step() {
    let t = Temp::new("close");
    seed(&t);
    let start = Instant::now();
    drop(Vault::open_deferred(t.root()).unwrap());
    // The warm-up sleeps 1.5 s; dropping must not wait for it, and the vault
    // can be reopened at once.
    let v = Vault::open(t.root()).unwrap();
    assert!(start.elapsed() < Duration::from_millis(1400), "{:?}", start.elapsed());
    drop(v);
}
