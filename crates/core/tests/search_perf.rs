//! Run separately from correctness checks:
//! CARGO_INCREMENTAL=0 cargo test --release -p vault_core --test search_perf -- --ignored --nocapture

use std::path::PathBuf;
use std::time::{Duration, Instant};

use vault_core::{seed, PropValue, Vault};

const BUDGET: Duration = Duration::from_millis(20);
const FUZZY_BUDGET: Duration = Duration::from_millis(30);
const TYPO_QUERIES: &[&str] = &[
    "tomatoe",
    "gardne",
    "weekyl review",
    "cofee",
    "cofee mor",
    "projcet",
    "recipies",
    "gadr",
    "meeitng",
    "buget",
    "desgin",
    "journla",
    "databsae",
];
const QUERIES: &[&str] = &[
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
];

struct Corpus(PathBuf);

impl Corpus {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("notes-search-perf-{}", vault_core::new_id())))
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn percentile(samples: &mut [Duration], percentile: usize) -> Duration {
    samples.sort_unstable();
    samples[(samples.len() * percentile).div_ceil(100) - 1]
}

fn report(label: &str, samples: &mut [Duration]) -> Duration {
    let p50 = percentile(samples, 50);
    let p95 = percentile(samples, 95);
    println!(
        "{label}: p50 {:.3} ms, p95 {:.3} ms (n={}, target p95 < 20 ms)",
        p50.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        samples.len()
    );
    p95
}

/// `SEARCH_PERF_REPORT_ONLY=1` keeps printing every timing case but skips the
/// timing asserts. Correctness asserts always run.
fn report_only() -> bool {
    std::env::var("SEARCH_PERF_REPORT_ONLY").as_deref() == Ok("1")
}

#[test]
#[ignore = "release-only 10,000-page search performance gate"]
fn warm_search_p95_under_20_ms() {
    // Runtime guard, not assert!: clippy rejects assertions on constant cfg values.
    if cfg!(debug_assertions) {
        panic!("search performance must be measured with cargo test --release");
    }
    let corpus = Corpus::new();
    // Reuse the benchmark's deterministic, small-vocabulary body/title corpus.
    // generate adds one big page, so 9,999 ordinary pages gives exactly 10,000.
    seed::generate(corpus.0.to_string_lossy().into_owned(), 9_999, 2_000).unwrap();
    let vault = Vault::open(corpus.0.to_string_lossy().into_owned()).unwrap();
    assert_eq!(vault.page_count().unwrap(), 10_000);
    let stats = vault.search_index_stats();
    println!(
        "word index, 10,000 pages: {} words, {} deletion keys, {} estimated bytes, {:.3} ms build",
        stats.words, stats.deletion_keys, stats.estimated_bytes, stats.build_ms
    );

    for _ in 0..3 {
        for query in QUERIES {
            let hits = vault.search((*query).into(), 50).unwrap();
            assert!(hits.len() <= 50);
            if *query != "xylophone" {
                assert!(!hits.is_empty(), "corpus must match {query:?}");
            }
        }
    }

    let mut all = Vec::with_capacity(QUERIES.len() * 50);
    let mut per_query = vec![Vec::with_capacity(50); QUERIES.len()];
    // Interleave queries to avoid measuring each one's own warmed result path.
    for _ in 0..50 {
        for (index, query) in QUERIES.iter().enumerate() {
            let start = Instant::now();
            let hits = vault.search((*query).into(), 50).unwrap();
            let elapsed = start.elapsed();
            assert!(hits.len() <= 50);
            per_query[index].push(elapsed);
            all.push(elapsed);
        }
    }

    let overall = report("warm search, 10,000 pages, limit 50", &mut all);
    let mut slow = Vec::new();
    for (query, samples) in QUERIES.iter().zip(&mut per_query) {
        let p95 = report(&format!("query {query:?}"), samples);
        if p95 >= BUDGET {
            slow.push(format!("{query:?}: {:.3} ms", p95.as_secs_f64() * 1000.0));
        }
    }
    assert!(
        report_only() || (overall < BUDGET && slow.is_empty()),
        "search exceeded 20 ms: overall p95 {:.3} ms; slow queries: {}",
        overall.as_secs_f64() * 1000.0,
        slow.join(", ")
    );

    // Separate allowance for queries that actually enter the fuzzy path.
    // Keep the exact-query corpus, workload, assertions and 20 ms gate above.
    for _ in 0..3 {
        for query in TYPO_QUERIES {
            let hits = vault.search((*query).into(), 50).unwrap();
            assert!(hits.iter().any(|hit| hit.fuzzy), "fuzzy path: {query:?}");
            assert!(hits.len() <= 50);
        }
    }
    let mut all = Vec::with_capacity(TYPO_QUERIES.len() * 50);
    let mut per_query = vec![Vec::with_capacity(50); TYPO_QUERIES.len()];
    for _ in 0..50 {
        for (index, query) in TYPO_QUERIES.iter().enumerate() {
            let start = Instant::now();
            let hits = vault.search((*query).into(), 50).unwrap();
            let elapsed = start.elapsed();
            assert!(hits.len() <= 50);
            assert!(hits.iter().any(|hit| hit.fuzzy), "fuzzy path: {query:?}");
            per_query[index].push(elapsed);
            all.push(elapsed);
        }
    }
    let fuzzy_report = |label: &str, samples: &mut [Duration]| {
        let p50 = percentile(samples, 50);
        let p95 = percentile(samples, 95);
        println!(
            "{label}: p50 {:.3} ms, p95 {:.3} ms (n={}, fuzzy ceiling p95 < 30 ms)",
            p50.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            samples.len()
        );
        p95
    };
    let overall = fuzzy_report("fuzzy search, 10,000 pages, limit 50", &mut all);
    let mut slow = Vec::new();
    for (query, samples) in TYPO_QUERIES.iter().zip(&mut per_query) {
        let p95 = fuzzy_report(&format!("typo query {query:?}"), samples);
        if p95 >= FUZZY_BUDGET {
            slow.push(format!("{query:?}: {:.3} ms", p95.as_secs_f64() * 1000.0));
        }
    }
    assert!(
        report_only() || (overall < FUZZY_BUDGET && slow.is_empty()),
        "fuzzy search exceeded 30 ms: overall p95 {:.3} ms; slow queries: {}",
        overall.as_secs_f64() * 1000.0,
        slow.join(", ")
    );

    filtered_search_cases(&vault);
}

/// Add metadata and a genuinely deep subtree to the same 10k-page corpus,
/// after the original unfiltered gates have finished. All setup is untimed.
fn filtered_search_cases(vault: &Vault) {
    let pages = vault.list_pages(None, 0, 10_000).unwrap();
    assert_eq!(pages.len(), 10_000);
    for (index, page) in pages.iter().enumerate() {
        // Broad candidates cross the ~5k execution crossover. The selective
        // bucket stays small enough to exercise candidate-restricted FTS.
        let mut tags = vec![
            "perf/all".to_string(),
            format!("perf/bucket/{:02}", index % 100),
        ];
        if index < 8_000 {
            tags.push("perf/broad".into());
        }
        if index % 100 == 0 {
            tags.push("perf/selective".into());
        }
        if index >= 8_000 {
            tags.push("perf/excluded".into());
        }
        vault
            .set_property(page.id.clone(), "tags".into(), PropValue::MultiSelect(tags))
            .unwrap();
    }
    // Move existing pages rather than add pages: the seed remains exactly 10k.
    let roots = vault.list_children(None, 0, 100).unwrap();
    assert!(roots.len() >= 32, "deep subtree requires 32 seed roots");
    let chain: Vec<_> = roots[..32].iter().map(|p| p.id.clone()).collect();
    for pair in chain.windows(2) {
        vault
            .move_page(pair[1].clone(), Some(pair[0].clone()), None)
            .unwrap();
    }
    vault.flush().unwrap();
    assert_eq!(vault.page_count().unwrap(), 10_000);

    // Prime only metadata, then compile 100 different never-searched tag
    // filters. Unlike the warm cases below, these timings include the initial
    // 10k-page tag lookup and candidate-set construction, so they can justify
    // whether the SEP representation needs a multi-select value table.
    vault.parse_search("tag:perf/all".into()).unwrap();
    let mut uncached_tags = Vec::with_capacity(100);
    for bucket in 0..100 {
        let start = Instant::now();
        let parsed = vault
            .parse_search(format!("tag:perf/bucket/{bucket:02}"))
            .unwrap();
        let result = vault.search_spec(parsed.spec, 50, 0).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(result.total_bounded, 100);
        assert_eq!(result.hits.len(), 50);
        uncached_tags.push(elapsed);
    }
    let p95 = report(
        "uncached tag candidate construction, 100 distinct filters, 10,000 pages",
        &mut uncached_tags,
    );
    assert!(
        report_only() || p95 < BUDGET,
        "uncached tag search exceeded 20 ms: {:.3} ms",
        p95.as_secs_f64() * 1000.0
    );

    // Use a stable id in the deep operator. Existing random descendants also
    // participate; traversal must include every level of the constructed chain.
    let deep = format!("in:[[{}]]", chain[0]);
    let cases = [
        ("filter only", "tag:perf/selective".to_string()),
        (
            "selective filter plus text",
            "project tag:perf/selective".into(),
        ),
        ("broad filter plus broad text", "p tag:perf/broad".into()),
        ("negatives only", "-tag:perf/excluded".into()),
        ("deep in", deep),
        ("tag on 10k", "tag:perf/all".into()),
    ];
    for _ in 0..3 {
        for (_, query) in &cases {
            let parsed = vault.parse_search(query.clone()).unwrap();
            let result = vault.search_spec(parsed.spec, 50, 0).unwrap();
            assert!(!result.hits.is_empty(), "seed must match {query:?}");
            assert!(result.hits.len() <= 50);
        }
    }
    let mut samples = vec![Vec::with_capacity(50); cases.len()];
    for _ in 0..50 {
        for (index, (_, query)) in cases.iter().enumerate() {
            // Include parsing and resolution: this is the per-keystroke API
            // cost, not just a precompiled SQL execution measurement.
            let start = Instant::now();
            let parsed = vault.parse_search(query.clone()).unwrap();
            let result = vault.search_spec(parsed.spec, 50, 0).unwrap();
            let elapsed = start.elapsed();
            assert!(result.hits.len() <= 50);
            assert!(!result.hits.is_empty(), "seed must match {query:?}");
            samples[index].push(elapsed);
        }
    }
    let mut slow = Vec::new();
    for ((label, query), samples) in cases.iter().zip(&mut samples) {
        let p95 = report(
            &format!("filtered {label}, {query:?}, 10,000 pages"),
            samples,
        );
        if p95 >= BUDGET {
            slow.push(format!("{label}: {:.3} ms", p95.as_secs_f64() * 1000.0));
        }
    }
    assert!(
        report_only() || slow.is_empty(),
        "filtered search exceeded 20 ms: {}",
        slow.join(", ")
    );

    // Each successive typed prefix keeps precisely the same filter. Interleave
    // them to exercise candidate caching across distinct query text.
    let prefixes = ["p", "pr", "pro", "proj", "project"];
    let mut prefix_samples = vec![Vec::with_capacity(50); prefixes.len()];
    for _ in 0..3 {
        for prefix in prefixes {
            assert!(!vault
                .search(format!("{prefix} tag:perf/broad"), 50)
                .unwrap()
                .is_empty());
        }
    }
    for _ in 0..50 {
        for (index, prefix) in prefixes.iter().enumerate() {
            let start = Instant::now();
            let hits = vault
                .search(format!("{prefix} tag:perf/broad"), 50)
                .unwrap();
            let elapsed = start.elapsed();
            assert!(!hits.is_empty());
            assert!(hits.len() <= 50);
            prefix_samples[index].push(elapsed);
        }
    }
    let mut slow = Vec::new();
    for (prefix, samples) in prefixes.iter().zip(&mut prefix_samples) {
        let p95 = report(
            &format!("typed prefix {prefix:?}, constant tag:perf/broad, 10,000 pages"),
            samples,
        );
        if p95 >= BUDGET {
            slow.push(format!("{prefix:?}: {:.3} ms", p95.as_secs_f64() * 1000.0));
        }
    }
    assert!(
        report_only() || slow.is_empty(),
        "filtered typed prefixes exceeded 20 ms: {}",
        slow.join(", ")
    );

    let typo = "gardne tag:perf/broad";
    for _ in 0..3 {
        let hits = vault.search(typo.into(), 50).unwrap();
        assert!(
            hits.iter().any(|h| h.fuzzy),
            "filtered typo must enter fuzzy path"
        );
    }
    let mut typo_samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let start = Instant::now();
        let hits = vault.search(typo.into(), 50).unwrap();
        let elapsed = start.elapsed();
        assert!(hits.len() <= 50);
        assert!(hits.iter().any(|h| h.fuzzy));
        // A cached set must apply even on title and body correction paths.
        assert!(hits
            .iter()
            .all(|h| h.matched_filters.iter().any(|f| f.key == "tag")));
        typo_samples.push(elapsed);
    }
    let p50 = percentile(&mut typo_samples, 50);
    let p95 = percentile(&mut typo_samples, 95);
    println!(
        "filtered typo {typo:?}, 10,000 pages: p50 {:.3} ms, p95 {:.3} ms (n={}, fuzzy ceiling p95 < 30 ms)",
        p50.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        typo_samples.len()
    );
    assert!(
        report_only() || p95 < FUZZY_BUDGET,
        "filtered typo exceeded 30 ms: {:.3} ms",
        p95.as_secs_f64() * 1000.0
    );
}
