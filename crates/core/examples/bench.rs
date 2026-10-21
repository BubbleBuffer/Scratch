//! Timings for the targets in wiki/architecture/core-state.md.
//!
//! cargo run -p vault_core --release --example bench -- <vault dir> [pages] [big page blocks] [--pages-only|--agenda-only]
//!
//! Seeds the vault first if it is empty (10,000 pages and a 2,000-block page
//! by default), then adds a 10,000-row "Tasks" collection
//! (`seed::generate_tasks`) and a "Link hub" page with 500 backlinks if they
//! are missing. Reruns add nothing: the bench only edits one block of the big
//! page and restores it. `--pages-only` skips the collection and backlink
//! parts (and their seeding), to compare with the bench before collections.
//! `--agenda-only` seeds only Tasks when needed and measures organizer queries.
//!
//! cargo run -p vault_core --release --example bench -- --history <fixture copy>
//!
//! Cold/warm history timings on a fixture from the cold-history harness
//! (`measure_cold_history` in crates/core/tests/support/cold_history_bench.rs).
//! Public API only, so the same command runs on master (no cold history) and on
//! the branch. Needs a fresh copy per run: the run writes block caches and a
//! `.vault/bench-used` marker into it, and refuses a copy that already has the
//! marker. It also copies the fixture to `<fixture copy>.edit` for the edits.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vault_core::{
    seed, BlockKind, BlockOp, Condition, Filter, FilterOp, Mark, MarkKind, NewBlock, PropValue,
    Sort, Vault,
};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct Stats(Vec<f64>);

impl Stats {
    fn of(mut v: Vec<f64>) -> Stats {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Stats(v)
    }
    fn pct(&self, p: f64) -> f64 {
        let i = ((self.0.len() as f64 - 1.0) * p).round() as usize;
        self.0[i]
    }
    fn line(&self) -> String {
        format!(
            "p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.3} ms (n={})",
            self.pct(0.5),
            self.pct(0.95),
            self.pct(0.99),
            self.0[self.0.len() - 1],
            self.0.len()
        )
    }
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let r = f();
    (r, ms(t.elapsed()))
}

fn main() {
    let history_only = std::env::args().any(|a| a == "--history");
    let storage_only = std::env::args().any(|a| a == "--storage-only");
    let pages_only = std::env::args().any(|a| a == "--pages-only");
    let agenda_only = std::env::args().any(|a| a == "--agenda-only");
    let mut args = std::env::args().skip(1).filter(|a| !a.starts_with("--"));
    let root = args
        .next()
        .expect("usage: bench <vault dir> [pages] [big page blocks]");
    let pages: u32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(10_000);
    let big: u32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(2_000);

    if history_only {
        history_timings(&root);
        return;
    }

    if storage_only {
        storage_timings(&root, pages, big);
        return;
    }

    let count = {
        let v = Vault::open(root.clone()).expect("open");
        v.page_count().unwrap()
    };
    if count == 0 && !agenda_only {
        let (r, t) = time(|| seed::generate(root.clone(), pages, big));
        r.expect("seed");
        println!("seed {pages} pages + big page of {big} blocks: {:.0} ms", t);
    }
    let has_tasks = {
        let v = Vault::open(root.clone()).expect("open");
        v.list_children(None, 0, u32::MAX)
            .unwrap()
            .iter()
            .any(|p| p.title == seed::TASKS_TITLE)
    };
    if !has_tasks && !pages_only {
        let (r, t) = time(|| seed::generate_tasks(root.clone(), pages));
        r.expect("seed tasks");
        println!("seed Tasks collection of {pages} rows: {:.0} ms", t);
    }

    if agenda_only {
        let v = Vault::open(root).unwrap();
        println!("pages in vault: {}", v.page_count().unwrap());
        organizer(&v);
        return;
    }

    // open: first open in this process, then the median of 5 reopens.
    let (v, first_open) = time(|| Vault::open(root.clone()).unwrap());
    drop(v);
    let mut opens = Vec::new();
    for _ in 0..5 {
        let (v, t) = time(|| Vault::open(root.clone()).unwrap());
        opens.push(t);
        drop(v);
    }
    let opens = Stats::of(opens);
    let v = Vault::open(root.clone()).unwrap();
    let n = v.page_count().unwrap();
    println!("pages in vault: {n}");
    println!(
        "open: first {:.2} ms, median of 5 {:.2} ms   [target < 200 ms]",
        first_open,
        opens.pct(0.5)
    );

    // list_pages, 100 rows at several offsets.
    let mut list = Vec::new();
    for i in 0..300u32 {
        let offset = [0, n / 2, n.saturating_sub(100)][(i % 3) as usize];
        let (rows, t) = time(|| v.list_pages(None, offset, 100).unwrap());
        assert!(!rows.is_empty());
        list.push(t);
    }
    println!(
        "list_pages 100 rows (offsets 0, mid, end): {}   [target < 5 ms]",
        Stats::of(list).line()
    );

    // search: prefixes as typed, words, two words, no hits.
    let queries = [
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
    let mut search = Vec::new();
    let mut worst = ("", 0.0f64);
    let mut per_query: Vec<Vec<f64>> = vec![Vec::new(); queries.len()];
    let mut hits = vec![0usize; queries.len()];
    for _ in 0..10 {
        for (i, q) in queries.iter().enumerate() {
            let (r, t) = time(|| v.search(q.to_string(), 50).unwrap());
            hits[i] = r.len();
            if t > worst.1 {
                worst = (q, t);
            }
            per_query[i].push(t);
            search.push(t);
        }
    }
    let medians: Vec<String> = queries
        .iter()
        .zip(per_query)
        .zip(&hits)
        .map(|((q, t), h)| format!("{q:?} {:.1} ms/{h}", Stats::of(t).pct(0.5)))
        .collect();
    println!("search medians (query ms/hits): {}", medians.join(", "));
    println!(
        "search (limit 50, {} queries x10): {}; slowest {:?}   [target < 20 ms]",
        queries.len(),
        Stats::of(search).line(),
        worst.0
    );

    // load_page on the big page: cold (document read from disk, in a freshly
    // opened vault) and warm (cached document).
    let big_id = v
        .search("big page".into(), 5)
        .unwrap()
        .into_iter()
        .find(|h| h.title == seed::BIG_PAGE_TITLE)
        .expect("big page")
        .id;
    drop(v);
    let mut colds = Vec::new();
    let mut v = None;
    for _ in 0..5 {
        drop(v.take());
        let fresh = Vault::open(root.clone()).unwrap();
        colds.push(time(|| fresh.load_page(big_id.clone()).unwrap()).1);
        v = Some(fresh);
    }
    let v = v.unwrap();
    let first_cold = colds[0];
    let colds = Stats::of(colds);
    let page = v.load_page(big_id.clone()).unwrap();
    fn count_blocks(b: &[vault_core::Block]) -> usize {
        b.iter().map(|x| 1 + count_blocks(&x.children)).sum()
    }
    let blocks = count_blocks(&page.blocks);
    let mut warm = Vec::new();
    for _ in 0..30 {
        warm.push(time(|| v.load_page(big_id.clone()).unwrap()).1);
    }
    println!(
        "load_page big page ({blocks} blocks): cold first {:.2} ms, cold median of 5 {:.2} ms (max {:.2}); warm {}   [target < 30 ms]",
        first_cold,
        colds.pct(0.5),
        colds.pct(1.0),
        Stats::of(warm).line()
    );

    // apply: typing into one block of the big page, one SetText per key.
    let target = page
        .blocks
        .iter()
        .find(|b| b.kind == vault_core::BlockKind::Paragraph)
        .expect("paragraph");
    let mut text = target.text.clone();
    let marks = target.marks.clone();
    let mut apply = Vec::new();
    for i in 0..1000 {
        let ch = if i % 200 == 199 {
            '\u{fc}'
        } else {
            (b'a' + (i % 26) as u8) as char
        };
        text.push(ch);
        let ops = vec![BlockOp::SetText {
            id: target.id.clone(),
            text: text.clone(),
            marks: marks.clone(),
        }];
        apply.push(time(|| v.apply(big_id.clone(), ops).unwrap()).1);
    }
    println!(
        "apply one SetText (1000 keystrokes, big page): {}   [target p95 < 2 ms]",
        Stats::of(apply).line()
    );
    // Restore the block so reruns start from the same text.
    v.apply(
        big_id.clone(),
        vec![BlockOp::SetText {
            id: target.id.clone(),
            text: target.text.clone(),
            marks,
        }],
    )
    .unwrap();

    let (_, flush) = time(|| v.flush().unwrap());
    println!("flush after the typing burst: {:.2} ms", flush);
    if !pages_only {
        collections(&v);
        backlinks(&v);
    }

    let (_, close) = time(|| drop(v));
    println!("close: {:.2} ms", close);
}

/// One sample page of the fixture manifest (`bench-manifest.json`). `big` marks the
/// 2,000-block page, which is reported on its own line.
#[derive(serde::Deserialize)]
struct Sample {
    page: String,
    #[serde(default)]
    big: bool,
    old_version: String,
    edit_block: String,
    versions_sha256: String,
    old_view_sha256: String,
}

/// Timings of the ordinary sample pages and of the big page, kept apart.
#[derive(Default)]
struct Split {
    normal: Vec<f64>,
    big: Vec<f64>,
}

impl Split {
    fn push(&mut self, big: bool, t: f64) {
        if big {
            self.big.push(t);
        } else {
            self.normal.push(t);
        }
    }
    fn stats(v: &[f64]) -> String {
        if v.is_empty() {
            "none".into()
        } else {
            Stats::of(v.to_vec()).line()
        }
    }
    fn report(&self, name: &str) {
        println!("{name}: {}", Self::stats(&self.normal));
        if !self.big.is_empty() {
            println!("{name}, big page: {}", Self::stats(&self.big));
        }
    }
}

fn digest<T: serde::Serialize>(value: &T) -> String {
    vault_core::sha256_hex(&serde_json::to_vec(value).unwrap())
}

/// Bytes and files of one directory's entries with a suffix.
fn dir_stats(dir: &Path, suffix: &str) -> (u64, usize) {
    fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(suffix))
        .fold((0, 0), |(b, n), e| (b + e.metadata().unwrap().len(), n + 1))
}

/// Copies a directory tree. Links and other non-file entries are skipped and
/// counted; the fixture has none.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut skipped = 0;
    for e in fs::read_dir(from)? {
        let e = e?;
        let ft = e.file_type()?;
        let dst = to.join(e.file_name());
        if ft.is_symlink() {
            skipped += 1;
        } else if ft.is_dir() {
            skipped += copy_dir(&e.path(), &dst)?;
        } else if ft.is_file() {
            fs::copy(e.path(), dst)?;
        } else {
            skipped += 1;
        }
    }
    Ok(skipped)
}

/// Public-API history timings on a fixture copy (see the header). Each sample page is
/// measured in a freshly opened vault, so its first call is the cold one. Edits go to
/// `<dir>.edit`, a copy made before anything is opened. The check asserts that every
/// version list and old version matches the digests the cold phase recorded.
fn history_timings(dir: &str) {
    const REPEATS: usize = 5;
    let base = Path::new(dir);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(base.join("bench-manifest.json"))
            .expect("bench-manifest.json: generate the fixture first"),
    )
    .unwrap();
    let samples: Vec<Sample> = serde_json::from_value(manifest["samples"].clone()).unwrap();
    assert!(!samples.is_empty(), "manifest has no samples: run the cold phase on this fixture first");
    let used = base.join(".vault/bench-used");
    assert!(
        !used.exists(),
        "{dir} was already measured; copy the fixture again for each run"
    );

    // The writer's idle cold pass must not run during the timings.
    fs::write(base.join(".vault/cold-policy.json"), r#"{"idle_seconds":3600}"#).unwrap();
    let edit = PathBuf::from(format!("{dir}.edit"));
    assert!(!edit.exists(), "{} already exists; use a fresh disposable copy", edit.display());
    let skipped = copy_dir(base, &edit).unwrap();
    fs::write(&used, b"").unwrap();

    let (hot_bytes, hot_files) = dir_stats(&base.join(".vault/docs"), ".loro");
    let (cold_bytes, cold_files) = dir_stats(&base.join(".vault/cold"), ".cold");
    let (dict_bytes, dict_files) = dir_stats(&base.join(".vault/cold/dictionaries"), ".dict");
    println!(
        "history dir={dir} samples={} big_samples={} hot_files={hot_files} hot_bytes={hot_bytes} \
         cold_files={cold_files} cold_bytes={cold_bytes} dict_files={dict_files} \
         dict_bytes={dict_bytes} copy_skipped={skipped}",
        samples.len(),
        samples.iter().filter(|s| s.big).count()
    );

    // Cold page open: first load in a freshly opened vault.
    let mut cold_open = Split::default();
    for s in &samples {
        let v = Vault::open(dir.to_owned()).unwrap();
        cold_open.push(s.big, time(|| v.load_page(s.page.clone()).unwrap()).1);
    }
    cold_open.report("page open cold (fresh vault)");

    // Warm page open: repeat loads of a page already in memory.
    let v = Vault::open(dir.to_owned()).unwrap();
    let mut warm = Split::default();
    for s in &samples {
        v.load_page(s.page.clone()).unwrap();
        v.flush().unwrap();
        for _ in 0..REPEATS {
            warm.push(s.big, time(|| v.load_page(s.page.clone()).unwrap()).1);
        }
    }
    drop(v);
    warm.report("page open warm (repeat load)");

    // History list: first call in a fresh vault, then repeats.
    let (mut list_first, mut list_rep, mut list_bad) = (Split::default(), Split::default(), 0usize);
    for s in &samples {
        let v = Vault::open(dir.to_owned()).unwrap();
        let (versions, t) = time(|| v.list_versions(s.page.clone()).unwrap());
        list_first.push(s.big, t);
        list_bad += (digest(&versions) != s.versions_sha256) as usize;
        for _ in 0..REPEATS {
            list_rep.push(s.big, time(|| v.list_versions(s.page.clone()).unwrap()).1);
        }
    }
    list_first.report("history list first (fresh vault)");
    list_rep.report("history list repeat");

    // Old version: the oldest version of each sample page.
    let (mut old_first, mut old_rep, mut old_bad) = (Split::default(), Split::default(), 0usize);
    for s in &samples {
        let v = Vault::open(dir.to_owned()).unwrap();
        let (view, t) = time(|| v.load_page_at(s.page.clone(), s.old_version.clone()).unwrap());
        old_first.push(s.big, t);
        old_bad += (digest(&view) != s.old_view_sha256) as usize;
        for _ in 0..REPEATS {
            old_rep.push(s.big, time(|| v.load_page_at(s.page.clone(), s.old_version.clone()).unwrap()).1);
        }
    }
    old_first.report("old version first (fresh vault)");
    old_rep.report("old version repeat");

    let export_vault = Vault::open(dir.to_owned()).unwrap();
    let t = Instant::now();
    let mut export_bytes = 0usize;
    let pages = export_vault.page_versions().unwrap();
    for page in &pages {
        export_bytes += export_vault.export_updates(page.page.clone(), Vec::new()).unwrap().len();
    }
    println!("full export pages={} bytes={} elapsed_ms={:.3}", pages.len(), export_bytes, ms(t.elapsed()));
    drop(export_vault);

    // Edits on the disposable copy: the first insert after a fresh open (cold), then a
    // second insert in the same vault (warm). Each sample edits its own page.
    let (mut edit_cold, mut edit_warm) = (Split::default(), Split::default());
    for s in &samples {
        let v = Vault::open(edit.to_string_lossy().into_owned()).unwrap();
        let insert = |text: &str| {
            vec![BlockOp::Insert {
                parent: None,
                after: Some(s.edit_block.clone()),
                block: NewBlock {
                    id: vault_core::new_id(),
                    kind: BlockKind::Paragraph,
                    text: text.into(),
                    marks: Vec::new(),
                    checked: false,
                },
            }]
        };
        edit_cold.push(s.big, time(|| v.apply(s.page.clone(), insert("bench edit 1")).unwrap()).1);
        edit_warm.push(s.big, time(|| v.apply(s.page.clone(), insert("bench edit 2")).unwrap()).1);
        v.flush().unwrap();
    }
    edit_cold.report("edit first (fresh vault, cold page)");
    edit_warm.report("edit second (same vault)");

    println!(
        "check versions_mismatch={list_bad} old_view_mismatch={old_bad} of {}",
        samples.len()
    );
    assert_eq!(list_bad, 0, "version lists differ from the cold phase's digests");
    assert_eq!(old_bad, 0, "old versions differ from the cold phase's digests");
}

/// Read-only page/history timings on an existing fixture; seed only if empty.
fn storage_timings(root: &str, pages: u32, big: u32) {
    let v = Vault::open(root.to_owned()).unwrap();
    if v.page_count().unwrap() == 0 {
        drop(v);
        seed::generate(root.to_owned(), pages, big).unwrap();
    } else {
        drop(v);
    }
    let page = std::env::var("COLD_BENCH_PAGE").unwrap_or_else(|_| {
        let entry = std::fs::read_dir(std::path::Path::new(root).join(".vault/docs"))
            .unwrap().filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".loro"))
            .max_by_key(|e| e.metadata().unwrap().len()).expect("page");
        entry.file_name().to_string_lossy().trim_end_matches(".loro").to_owned()
    });
    let mut cold = Vec::new();
    for _ in 0..5 {
        let v = Vault::open(root.to_owned()).unwrap();
        cold.push(time(|| v.load_page(page.clone()).unwrap()).1);
    }
    let first = cold[0];
    let v = Vault::open(root.to_owned()).unwrap();
    v.load_page(page.clone()).unwrap();
    let mut warm = Vec::new();
    for _ in 0..30 { warm.push(time(|| v.load_page(page.clone()).unwrap()).1); }
    let versions = v.list_versions(page.clone()).unwrap();
    let old = versions.last().unwrap().id.clone();
    drop(v);
    let v = Vault::open(root.to_owned()).unwrap();
    let first_history = time(|| v.load_page_at(page.clone(), old.clone()).unwrap()).1;
    let mut history = Vec::new();
    for _ in 0..10 { history.push(time(|| v.load_page_at(page.clone(), old.clone()).unwrap()).1); }
    println!("storage page={page} versions={} page_open_first_ms={first:.3} cold={} warm={} history_first_ms={first_history:.3} repeat={}",
        versions.len(), Stats::of(cold).line(), Stats::of(warm).line(), Stats::of(history).line());
}

fn cond(prop: &str, op: FilterOp, value: PropValue) -> Filter {
    Filter::Cond(Condition {
        prop: prop.into(),
        op,
        value,
    })
}

fn sort(prop: &str, descending: bool) -> Sort {
    Sort {
        prop: prop.into(),
        descending,
    }
}

fn repeat<T>(n: usize, mut f: impl FnMut() -> T) -> (T, Stats) {
    let mut out = None;
    let mut t = Vec::new();
    for _ in 0..n {
        let (r, d) = time(&mut f);
        out = Some(r);
        t.push(d);
    }
    (out.unwrap(), Stats::of(t))
}

fn organizer(v: &Vault) {
    const DAY: i64 = 86_400_000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let today = now.div_euclid(DAY) * DAY;
    let (items, st) = repeat(30, || {
        v.agenda(today, today + 7 * DAY, today, today + 7 * DAY)
            .unwrap()
    });
    println!(
        "agenda week, {} entries: {}   [target < 20 ms]",
        items.len(),
        st.line()
    );
    let (items, st) = repeat(30, || v.overdue(today, today, 50).unwrap());
    println!(
        "overdue (limit 50), {} entries: {}   [target < 20 ms]",
        items.len(),
        st.line()
    );
    let (count, st) = repeat(30, || v.overdue_count(today, today).unwrap());
    println!(
        "overdue_count, {count} entries: {}   [target < 20 ms]",
        st.line()
    );
}

/// Queries, board groups and calendar ranges over the Tasks collection.
fn collections(v: &Vault) {
    let tasks = v
        .list_children(None, 0, u32::MAX)
        .unwrap()
        .into_iter()
        .find(|p| p.title == seed::TASKS_TITLE)
        .expect("Tasks collection")
        .id;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    const DAY: i64 = 86_400_000;
    let open = Filter::And(vec![
        cond("status", FilterOp::IsNot, PropValue::Status("done".into())),
        cond("due", FilterOp::IsNotEmpty, PropValue::Empty),
    ]);
    let rich = Filter::Or(vec![
        Filter::And(vec![
            cond(
                "tags",
                FilterOp::Contains,
                PropValue::MultiSelect(vec!["work".into()]),
            ),
            cond("estimate", FilterOp::GreaterOrEqual, PropValue::Number(3.0)),
        ]),
        cond("urgent", FilterOp::Checked, PropValue::Empty),
        cond(
            "$title",
            FilterOp::StartsWith,
            PropValue::Text("Gar".into()),
        ),
    ]);
    let cases: Vec<(&str, Option<Filter>, Vec<Sort>, u32)> = vec![
        ("no filter, sibling order", None, vec![], 0),
        (
            "open view: status != done and due set, by due",
            Some(open.clone()),
            vec![sort("due", false)],
            0,
        ),
        (
            "open view, page at offset 3,000",
            Some(open),
            vec![sort("due", false)],
            3_000,
        ),
        (
            "tags/estimate/urgent/title or-tree, by estimate desc then title",
            Some(rich),
            vec![sort("estimate", true), sort("$title", false)],
            0,
        ),
        ("no filter, by title", None, vec![sort("$title", false)], 0),
    ];
    for (name, filter, sorts, offset) in cases {
        let (r, st) = repeat(30, || {
            v.query(
                Some(tasks.clone()),
                filter.clone(),
                sorts.clone(),
                offset,
                50,
            )
            .unwrap()
        });
        println!(
            "query 50 rows of {} ({name}): {}   [target < 20 ms]",
            r.total,
            st.line()
        );
    }
    for by in ["status", "tags", "urgent"] {
        let (g, st) = repeat(30, || {
            v.query_groups(Some(tasks.clone()), None, by.into())
                .unwrap()
        });
        let counts: Vec<String> = g
            .iter()
            .map(|g| format!("{}:{}", g.label, g.count))
            .collect();
        println!("query_groups by {by} [{}]: {}", counts.join(" "), st.line());
    }
    let (g, st) = repeat(30, || {
        v.query_groups(
            Some(tasks.clone()),
            Some(cond("due", FilterOp::IsNotEmpty, PropValue::Empty)),
            "status".into(),
        )
        .unwrap()
    });
    println!(
        "query_groups by status, filtered ({} columns): {}",
        g.len(),
        st.line()
    );
    let today = now.div_euclid(DAY) * DAY;
    organizer(v);
    for (name, from, to) in [
        (
            "calendar month (42-day grid)",
            today - 21 * DAY,
            today + 21 * DAY,
        ),
        ("timeline 180 days", today - 90 * DAY, today + 90 * DAY),
    ] {
        let (r, st) = repeat(30, || {
            v.query_range(Some(tasks.clone()), "due".into(), from, to, None)
                .unwrap()
        });
        println!("query_range {name}, {} entries: {}", r.len(), st.line());
    }
}

/// Backlinks of a page that 500 other pages mention.
fn backlinks(v: &Vault) {
    const HUB: &str = "Link hub";
    let hub = match v
        .list_children(None, 0, u32::MAX)
        .unwrap()
        .into_iter()
        .find(|p| p.title == HUB)
    {
        Some(p) => p.id,
        None => {
            let hub = v.create_page(None, HUB.into()).unwrap();
            let pages = v.list_pages(None, 0, 1_000).unwrap();
            for p in pages
                .iter()
                .filter(|p| p.id != hub && p.title != seed::BIG_PAGE_TITLE)
                .step_by(2)
                .take(500)
            {
                v.apply(
                    p.id.clone(),
                    vec![BlockOp::Insert {
                        parent: None,
                        after: None,
                        block: NewBlock {
                            id: vault_core::new_id(),
                            kind: BlockKind::Paragraph,
                            text: format!("see {HUB}"),
                            marks: vec![Mark {
                                start: 4,
                                end: 4 + HUB.len() as u32,
                                kind: MarkKind::PageLink(hub.clone()),
                            }],
                            checked: false,
                        },
                    }],
                )
                .unwrap();
            }
            v.flush().unwrap();
            hub
        }
    };
    let (r, st) = repeat(30, || v.backlinks(hub.clone()).unwrap());
    println!("backlinks, {} linking pages: {}", r.len(), st.line());
    let one = v.list_pages(None, 5_000, 1).unwrap().remove(0).id;
    let (r, st) = repeat(30, || v.backlinks(one.clone()).unwrap());
    println!("backlinks, page with {} links: {}", r.len(), st.line());
}
