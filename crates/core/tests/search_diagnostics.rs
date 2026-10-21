//! Reproducible stage profile and pre-change SQL body-recall comparison.
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use vault_core::{seed, SearchHit, Vault};

const BASELINE: &str = "SELECT p.id, p.title, snippet(fts, 1, '', '', '…', 12)
    FROM fts JOIN pages p ON p.rowid = fts.rowid
    WHERE fts MATCH ?1 AND p.in_trash = 0 AND p.is_template = 0
    ORDER BY bm25(fts, 10.0, 1.0), p.updated_ms DESC, p.id LIMIT ?2";

fn baseline(conn: &Connection, query: &str) -> Vec<SearchHit> {
    conn.prepare_cached(BASELINE)
        .unwrap()
        .query_map(params![query, 50], |r| {
            Ok(SearchHit {
                id: r.get(0)?,
                title: r.get(1)?,
                snippet: r.get(2)?,
                fuzzy: false,
                corrected: None,
                matched_filters: Vec::new(),
                source: String::new(),
                page_type: String::new(),
                target: None,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn measure(label: &str, mut work: impl FnMut()) -> Duration {
    for _ in 0..3 {
        work();
    }
    let mut times = Vec::new();
    for _ in 0..50 {
        let start = Instant::now();
        work();
        times.push(start.elapsed());
    }
    times.sort_unstable();
    println!(
        "{label}: p50 {:.3} ms, p95 {:.3} ms",
        ms(times[24]),
        ms(times[47])
    );
    times[47]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

// Mirrors the core estimator's query rewrite so timing includes tokenization
// and capped distinct-term seeks, rather than measuring a precomputed query.
fn estimate_query(conn: &Connection, q: &str) -> Option<String> {
    let tokens = conn
        .prepare_cached("SELECT token FROM search_tokenizer WHERE input=?1 ORDER BY position")
        .unwrap()
        .query_map([q], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    if tokens.is_empty() {
        return Some(q.into());
    }
    let mut parts = Vec::new();
    for token in tokens {
        if token.chars().count() <= 3 {
            parts.push(format!("\"{token}\"*"));
            continue;
        }
        let upper = format!("{token}\u{10ffff}");
        let mut lower = token.clone();
        let mut terms = Vec::new();
        let mut finished = false;
        for _ in 0..=16 {
            use rusqlite::OptionalExtension;
            let next: Option<String> = conn
                .prepare_cached(
                    "SELECT term FROM search_terms WHERE term >= ?1 AND term < ?2 LIMIT 1",
                )
                .unwrap()
                .query_row(params![lower, upper], |r| r.get(0))
                .optional()
                .unwrap();
            let Some(term) = next else {
                finished = true;
                break;
            };
            lower = format!("{term}\u{1}");
            terms.push(format!("\"{term}\""));
        }
        if !finished {
            continue;
        }
        parts.push(if !terms.is_empty() {
            format!("({})", terms.join(" OR "))
        } else {
            format!("\"{token}\"*")
        });
    }
    (!parts.is_empty()).then(|| parts.join(" AND "))
}

#[test]
#[ignore = "release-only stage profile on 10,000 pages"]
fn profile_search_stages_and_recall() {
    // Runtime guard, not assert!: clippy rejects assertions on constant cfg values.
    if cfg!(debug_assertions) {
        panic!("stage profile must be measured with cargo test --release");
    }
    let existing = std::env::var_os("NOTES_SEARCH_DIAGNOSTIC_CORPUS");
    let root = existing
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("notes-search-profile-{}", vault_core::new_id()))
        });
    if existing.is_none() {
        seed::generate(root.to_string_lossy().into_owned(), 9_999, 2_000).unwrap();
    }
    let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
    let conn = Connection::open(root.join(".vault/index.sqlite")).unwrap();
    conn.pragma_update(None, "cache_size", -16_000).unwrap();
    conn.pragma_update(None, "temp_store", "MEMORY").unwrap();
    conn.execute_batch("CREATE VIRTUAL TABLE temp.search_terms USING fts5vocab(main,fts,instance);
        CREATE VIRTUAL TABLE temp.search_tokenizer USING fts3tokenize(unicode61,'remove_diacritics=2');").unwrap();
    measure("uncached live page count", || {
        let pages: u32 = conn
            .query_row("SELECT count(*) FROM pages WHERE in_trash = 0", [], |r| {
                r.get(0)
            })
            .unwrap();
        std::hint::black_box(pages);
    });
    let mut count_cache: Option<(i64, u64, u32)> = None;
    let mut slow_estimates = Vec::new();
    for query in [
        "p",
        "pr",
        "pro",
        "proj",
        "project",
        "g",
        "ga",
        "gar",
        "gard",
        "garden",
        "a",
        "t",
        "tomato",
        "weekly review",
        "coffee mor",
        "zür",
        "café",
        "big page",
        "xylophone",
    ] {
        let q = query
            .split_whitespace()
            .map(|t| format!("\"{t}\"*"))
            .collect::<Vec<_>>()
            .join(" ");
        let p95 = measure(
            &format!("{query} capped breadth estimate (including page count)"),
            || {
                let version: i64 = conn
                    .pragma_query_value(None, "data_version", |r| r.get(0))
                    .unwrap();
                let changes = conn.total_changes();
                let pages = match count_cache {
                    Some((cached_version, cached_changes, pages))
                        if cached_version == version && cached_changes == changes =>
                    {
                        pages
                    }
                    _ => {
                        let pages: u32 = conn
                            .query_row("SELECT count(*) FROM pages WHERE in_trash = 0", [], |r| {
                                r.get(0)
                            })
                            .unwrap();
                        count_cache = Some((version, changes, pages));
                        pages
                    }
                };
                let threshold = 200.max(pages / 20);
                let Some(count_query) = estimate_query(&conn, &q) else {
                    std::hint::black_box(true);
                    return;
                };
                let count: u32 = conn
                    .prepare_cached(
                        "SELECT count(*) FROM (SELECT rowid FROM fts WHERE fts MATCH ?1 LIMIT ?2)",
                    )
                    .unwrap()
                    .query_row(params![count_query, threshold + 1], |r| r.get(0))
                    .unwrap();
                std::hint::black_box(count > threshold);
            },
        );
        if p95 >= Duration::from_millis(2) {
            slow_estimates.push(format!("{query}: {p95:?}"));
        }
    }
    for query in ["g", "garden"] {
        let q = format!("\"{query}\"*");
        for (stage, sql) in [
            ("FTS match/count", "SELECT count(*) FROM fts WHERE fts MATCH ?1"),
            ("match + bm25 + LIMIT", "SELECT rowid FROM fts WHERE fts MATCH ?1 ORDER BY bm25(fts, 10.0, 1.0) LIMIT 50"),
            ("match + pages + bm25 + LIMIT", "SELECT p.rowid FROM fts JOIN pages p ON p.rowid = fts.rowid WHERE fts MATCH ?1 AND p.in_trash = 0 AND p.is_template = 0 ORDER BY bm25(fts, 10.0, 1.0), p.updated_ms DESC, p.id LIMIT 50"),
        ] {
            measure(&format!("{query} {stage}"), || {
                let mut st = conn.prepare_cached(sql).unwrap();
                let rows = st.query_map([&q], |r| r.get::<_, i64>(0)).unwrap();
                std::hint::black_box(rows.collect::<rusqlite::Result<Vec<_>>>().unwrap());
            });
        }
        measure(&format!("{query} baseline snippets + DTO"), || {
            std::hint::black_box(baseline(&conn, &q));
        });
        let hits = baseline(&conn, &q);
        measure(&format!("{query} DTO copy only (50 DTOs)"), || {
            std::hint::black_box(hits.clone());
        });
        measure(&format!("{query} JSON only (50 DTOs)"), || {
            std::hint::black_box(serde_json::to_string(&hits).unwrap());
        });
        measure(&format!("{query} vault search"), || {
            std::hint::black_box(vault.search(query.into(), 50).unwrap());
        });
        measure(&format!("{query} full search after optimization"), || {
            std::hint::black_box(vault.search_full(query.into(), 50).unwrap());
        });
    }
    // Six longest gate queries, plus both queries tied at six characters.
    for query in [
        "weekly review",
        "coffee mor",
        "xylophone",
        "big page",
        "project",
        "garden",
        "tomato",
    ] {
        let q = query
            .split_whitespace()
            .map(|t| format!("\"{t}\"*"))
            .collect::<Vec<_>>()
            .join(" ");
        let expected = baseline(&conn, &q);
        let actual = vault.search_full(query.into(), 50).unwrap();
        // Title tiers intentionally change top-50 ordering/membership. The
        // full API must still retain every baseline body match when asked
        // for all pages; compare recall without requiring the old ranking.
        let full = vault
            .search_full(query.into(), vault.page_count().unwrap())
            .unwrap();
        for hit in &expected {
            assert!(
                full.iter().any(|h| h.id == hit.id && !h.fuzzy),
                "full body recall {query}: missing {}",
                hit.id
            );
        }
        let retained = expected
            .iter()
            .filter(|h| actual.iter().any(|a| a.id == h.id))
            .count();
        println!("recall {query:?}: all {} baseline hits retained in full search; {retained} remain in top 50", expected.len());
    }
    drop(conn);
    drop(vault);
    if existing.is_none() {
        std::fs::remove_dir_all(root).unwrap();
    }
    assert!(
        slow_estimates.is_empty(),
        "breadth estimate exceeded2ms: {}",
        slow_estimates.join(", ")
    );
}
