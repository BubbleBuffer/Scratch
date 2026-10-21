//! Cold-history fixtures and checks. Opt-in, disposable roots only.
//!
//! Run through `measure_cold_history` (the root must not exist or be empty):
//!   COLD_BENCH_ROOT=<dir> COLD_BENCH_PHASE=<phase> cargo test --release -p vault_core \
//!     --lib measure_cold_history -- --ignored --nocapture
//!
//! generate: the seed is seed::generate in the sibling <root>.seed (must not exist;
//!   removed afterwards unless COLD_BENCH_KEEP_SEED is set). With COLD_BENCH_SEED_SOURCE
//!   the seed is an earlier seed::generate output instead: its .vault/docs is only read,
//!   and nothing there is opened for writing, written or deleted. Every seed page's head
//!   (meta, block tree with IDs, marks, targets, colours, widths, props) is replayed into
//!   a new history: a creation commit, build commits, appends, rewrites with fresh
//!   paragraphs, restores, and two recent commits. Appends, rewrites and text restores
//!   replace block text by explicit Loro delete+insert (no text diff). That is artificial
//!   history, and the manifest says so. Marks, structure and props go through
//!   PageDoc::apply. Commits are spread from COLD_BENCH_DAYS (default 180) days before
//!   COLD_BENCH_ANCHOR_S (default now) to 32 days before it. The replayed head must equal
//!   the seed head, or generation fails. Env: COLD_BENCH_PAGES (10000 ordinary pages),
//!   COLD_BENCH_BIG (2000 big-page blocks).
//!
//! prepare: copies COLD_BENCH_SOURCE (the real vault; only read) into the root.
//!   Optionally trashes up to COLD_BENCH_TRASH_SMALL (default 0) small pages in the
//!   copy; trashed pages go cold at any size. Records digests for every page and
//!   candidate samples among the pages predicted to go cold.
//!
//! cold: one cold pass over a generated or prepared copy, with the real clock and the
//!   default policy (30 days, 16 KiB). Samples are chosen only from candidates that
//!   actually moved. Every page is checked against bench-digests.json, and every
//!   candidate's version list and old view against the digests taken before the pass.
//!   Generated fixtures also need most pages cold, 20+ cold files, one dictionary and
//!   the big page cold.
//!
//! Each root is cold-passed once: copy the fixture for every run. The timings are in
//! examples/bench.rs (`--history <copy>`), through the public API only.
use super::*;
use crate::attach::sha256_hex;
use crate::doc::PageDoc;
use crate::ids::new_id;
use crate::seed;
use crate::types::{Block, BlockKind, BlockOp, Mark, NewBlock, PropValue};
use loro::{
    Container, ExportMode, Frontiers, LoroDoc, LoroText, LoroValue, TreeID, TreeParentId,
    ValueOrContainer, VersionVector, ID,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const DAY_S: i64 = 86_400;
/// Replay devices: the creation commit uses DEVICES[0]; each commit switches with 20%.
const DEVICES: [u64; 2] = [0x0bb0_0001, 0x0bb0_0002];
/// Ordinary pages in the samples the cold pass picks.
const SAMPLES: usize = 60;
/// Candidate pages recorded before the cold pass (samples are picked from the moved ones).
const CANDIDATES: usize = 4 * SAMPLES;
/// Eligible encoded history the generator aims for (margin over the cold rule).
const TARGET_ELIGIBLE: usize = 20 * 1024;
/// The cold rule: at least this much encoded history before the cutoff.
const MIN_ELIGIBLE: usize = 16 * 1024;
/// Newest replayed old commit is this many days before the anchor (cold cutoff is 30).
const OLD_END_DAYS: i64 = 32;
const MAX_ATTEMPTS: u32 = 8;
const MAX_VOLUME: usize = 200_000;
const REWRITE_ROUNDS: usize = 24;
const BUILD_BATCH: usize = 8;
const BIG_BATCH: usize = 40;
const IDLE_OFF: &[u8] = br#"{"idle_seconds":3600}"#;
const WORDS: &[&str] = &[
    "garden", "plan", "review", "tomato", "project", "meeting", "idea", "book", "trip", "budget",
    "recipe", "note", "task", "morning", "coffee", "client", "draft", "sketch", "list", "school",
    "family", "home", "repair", "road", "river", "music", "photo", "market", "bread", "winter",
    "summer", "study", "design", "code", "test", "release", "check", "call", "letter", "field",
    "camp", "bike", "paint", "sand", "stone", "light", "water", "quiet", "fresh", "small",
];

/// Digest per page: version vector, frontiers, head and version list.
type Digests = BTreeMap<PageId, String>;

/// A page the cold check looks at, with its old version and the digests taken
/// before the cold pass. `big` marks the 2,000-block page.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Sample {
    page: PageId,
    #[serde(default)]
    big: bool,
    versions: usize,
    old_version: String,
    edit_block: String,
    versions_sha256: String,
    old_view_sha256: String,
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn env_num(name: &str, default: i64) -> i64 {
    match std::env::var(name) {
        Ok(v) => v.parse().unwrap_or_else(|_| panic!("{name} must be a number")),
        Err(_) => default,
    }
}

fn file_len(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// `<path><suffix>` as a sibling directory of `path`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", path.display()))
}

fn bytes_in(dir: &Path, suffix: &str) -> u64 {
    fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(suffix))
        .map(|e| e.metadata().unwrap().len()).sum()
}

fn files_in(dir: &Path, suffix: &str) -> usize {
    fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(suffix))
        .count()
}

/// Ids of the snapshots in a docs directory, sorted.
fn page_ids(docs: &Path) -> Vec<PageId> {
    let mut ids: Vec<PageId> = fs::read_dir(docs).into_iter().flatten().filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".loro").map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids
}

fn assert_empty(path: &Path) {
    assert!(
        !path.exists() || fs::read_dir(path).unwrap().next().is_none(),
        "{} must not exist or must be empty",
        path.display()
    );
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("{}: {e} (run generate or prepare first)", path.display()));
    serde_json::from_slice(&bytes).unwrap()
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn open(root: &str) -> Vault {
    Vault::open(root.to_owned()).unwrap()
}

fn in_trash(v: &Vault, id: &str) -> bool {
    v.with_db(|db| db.in_trash(id)).unwrap().unwrap_or(false)
}

fn digest<T: serde::Serialize>(value: &T) -> String {
    sha256_hex(&serde_json::to_vec(value).unwrap())
}

/// Copies a directory tree; links are skipped and counted.
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

fn canon_vv(vv: &VersionVector) -> Vec<(u64, i32)> {
    let mut out: Vec<(u64, i32)> = vv.iter().map(|(p, c)| (*p, *c)).collect();
    out.sort();
    out
}

fn canon_frontiers(f: &Frontiers) -> Vec<(u64, i32)> {
    let mut out: Vec<(u64, i32)> = f.iter().map(|op| (op.peer, op.counter)).collect();
    out.sort();
    out
}

/// Head of a page document: blocks with IDs, the raw page meta map, and the props
/// (sorted, values in Debug form). Generation compares the replay with the seed.
fn head_digest(pd: &mut PageDoc) -> String {
    let blocks = pd.blocks();
    let meta: BTreeMap<String, String> = match pd.doc.get_map("page").get_value() {
        LoroValue::Map(m) => m.iter().map(|(k, v)| (k.clone(), format!("{v:?}"))).collect(),
        _ => BTreeMap::new(),
    };
    let mut props: Vec<(String, String)> = pd.props().into_iter()
        .map(|p| (p.key, format!("{:?}", p.value)))
        .collect();
    props.sort();
    digest(&(blocks, meta, props))
}

/// Version vector, frontiers and head digest of a page, read from its history.
fn head_state(v: &Vault, page: &PageId) -> (Vec<(u64, i32)>, Vec<(u64, i32)>, String) {
    v.inner.read_history(page, |d| {
        (canon_vv(&d.doc.oplog_vv()), canon_frontiers(&d.doc.oplog_frontiers()), head_digest(d))
    }).unwrap()
}

/// One digest for everything the cold pass must preserve for a page.
fn page_digest(v: &Vault, page: &PageId) -> String {
    let (vv, frontiers, head) = head_state(v, page);
    let versions = v.list_versions(page.clone()).unwrap();
    digest(&(vv, frontiers, head, digest(&versions)))
}

/// Encoded bytes of the history before `cutoff` (the cold rule's measure).
fn eligible_estimate(doc: &PageDoc, cutoff: i64) -> usize {
    match crate::history::cutoff_root(doc, cutoff).unwrap() {
        Some(root) => doc.fork_at(&root).unwrap().doc.export(ExportMode::all_updates()).unwrap().len(),
        None => 0,
    }
}

fn vocab(rng: &mut Rng, min_chars: usize) -> String {
    let mut s = String::new();
    while s.len() < min_chars {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
    }
    s.push('.');
    s
}

/// Splits text into k cumulative prefixes at word boundaries; the last one is the text.
fn split_pieces(text: &str, k: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_inclusive(' ').collect();
    let k = k.min(words.len()).max(1);
    let per = words.len().div_ceil(k);
    (1..=k).map(|g| words[..(g * per).min(words.len())].concat()).collect()
}

/// A seed block in pre-order, with its parent and previous sibling.
struct Flat {
    id: String,
    parent: Option<String>,
    after: Option<String>,
    kind: BlockKind,
    text: String,
    marks: Vec<Mark>,
    checked: bool,
    target: Option<String>,
    color: Option<String>,
    width: Option<u32>,
}

fn flatten(blocks: &[Block], parent: Option<&str>, out: &mut Vec<Flat>) {
    let mut prev: Option<String> = None;
    for b in blocks {
        out.push(Flat {
            id: b.id.clone(),
            parent: parent.map(str::to_owned),
            after: prev.clone(),
            kind: b.kind.clone(),
            text: b.text.clone(),
            marks: b.marks.clone(),
            checked: b.checked,
            target: b.target.clone(),
            color: b.color.clone(),
            width: b.width,
        });
        flatten(&b.children, Some(b.id.as_str()), out);
        prev = Some(b.id.clone());
    }
}

/// One commit of the replay: props set, then block texts replaced (`raw`, an explicit
/// Loro delete+insert with no diff), then block ops applied.
#[derive(Default)]
struct Step {
    props: Vec<(String, PropValue)>,
    raw: Vec<(String, String)>,
    ops: Vec<BlockOp>,
}

/// The commits that rebuild a seed head. The first `old` steps are the old history
/// (build, appends, rewrites, restores); the rest are recent. Every text block is
/// rewritten with about `volume` characters of vocabulary, then restored to the seed
/// text and marks, so the final head is the seed head. Text changes are raw
/// replacements; apply only builds structure and sets marks.
fn plan(blocks: &[Block], props: &[(String, PropValue)], seed: u64, volume: usize, big: bool) -> (Vec<Step>, usize) {
    let mut rng = Rng::new(seed);
    let batch = if big { BIG_BATCH } else { BUILD_BATCH };
    let mut flat = Vec::new();
    flatten(blocks, None, &mut flat);
    let pieces: Vec<Vec<String>> = flat.iter()
        .map(|f| if f.text.is_empty() { Vec::new() } else { split_pieces(&f.text, 1 + rng.below(3) as usize) })
        .collect();
    let texty: Vec<usize> = (0..flat.len())
        .filter(|&i| !pieces[i].is_empty() && flat[i].kind != BlockKind::Embed)
        .collect();

    // Build: every block inserted with its first piece, parents before children.
    let mut steps: Vec<Step> = Vec::new();
    for start in (0..flat.len()).step_by(batch) {
        let mut step = Step::default();
        if start == 0 {
            step.props = props.to_vec();
        }
        for i in start..(start + batch).min(flat.len()) {
            let f = &flat[i];
            let first = pieces[i].first().cloned().unwrap_or_default();
            step.ops.push(BlockOp::Insert {
                parent: f.parent.clone(),
                after: f.after.clone(),
                block: NewBlock {
                    id: f.id.clone(),
                    kind: f.kind.clone(),
                    text: first,
                    marks: Vec::new(),
                    checked: f.checked,
                },
            });
            if let Some(t) = &f.target {
                step.ops.push(BlockOp::SetTarget { id: f.id.clone(), target: Some(t.clone()) });
            }
            if let Some(c) = &f.color {
                step.ops.push(BlockOp::SetColor { id: f.id.clone(), color: Some(c.clone()) });
            }
            if let Some(w) = f.width {
                step.ops.push(BlockOp::SetWidth { id: f.id.clone(), width: Some(w) });
            }
        }
        steps.push(step);
    }
    if steps.is_empty() {
        steps.push(Step { props: props.to_vec(), ..Step::default() });
    }

    // Appends: each later piece grows the text (marks come back with the restore).
    let max_k = pieces.iter().map(|p| p.len()).max().unwrap_or(0);
    for j in 1..max_k {
        let idx: Vec<usize> = (0..flat.len()).filter(|&i| pieces[i].len() > j).collect();
        for chunk in idx.chunks(batch) {
            steps.push(Step {
                raw: chunk.iter().map(|&i| (flat[i].id.clone(), pieces[i][j].clone())).collect(),
                ..Step::default()
            });
        }
    }

    // Rewrites: random text blocks get vocabulary text. Half the rounds add a fresh
    // paragraph after the first top-level block; the next round deletes it.
    let anchor = flat.iter().find(|f| f.parent.is_none()).map(|f| f.id.clone());
    let mut pending: Option<String> = None;
    if !texty.is_empty() {
        for _ in 0..REWRITE_ROUNDS {
            let mut step = Step::default();
            if let Some(p) = pending.take() {
                step.ops.push(BlockOp::Delete { id: p });
            }
            for _ in 0..1 + rng.below(3) {
                let i = texty[rng.below(texty.len() as u64) as usize];
                step.raw.push((flat[i].id.clone(), vocab(&mut rng, volume)));
            }
            if let Some(a) = &anchor {
                if rng.below(100) < 50 {
                    let fresh = new_id();
                    step.ops.push(BlockOp::Insert {
                        parent: None,
                        after: Some(a.clone()),
                        block: NewBlock {
                            id: fresh.clone(),
                            kind: BlockKind::Paragraph,
                            text: vocab(&mut rng, 120),
                            marks: Vec::new(),
                            checked: false,
                        },
                    });
                    pending = Some(fresh);
                }
            }
            steps.push(step);
        }
    }

    // Restores: the seed text and marks of every text block, and no fresh paragraph.
    let mut restore: Vec<Step> = Vec::new();
    for chunk in texty.chunks(batch) {
        restore.push(Step {
            raw: chunk.iter().map(|&i| (flat[i].id.clone(), flat[i].text.clone())).collect(),
            // Apply only sets marks, so a block without marks needs no call.
            ops: chunk.iter().filter(|&&i| !flat[i].marks.is_empty()).map(|&i| BlockOp::SetText {
                id: flat[i].id.clone(),
                text: flat[i].text.clone(),
                marks: flat[i].marks.clone(),
            }).collect(),
            ..Step::default()
        });
    }
    if let Some(p) = pending.take() {
        match restore.first_mut() {
            Some(s) => s.ops.insert(0, BlockOp::Delete { id: p }),
            None => restore.push(Step { ops: vec![BlockOp::Delete { id: p }], ..Step::default() }),
        }
    }
    steps.extend(restore);
    let old = steps.len();

    // Recent: one text block is rewritten and restored, inside the last month.
    if !texty.is_empty() {
        let i = texty[rng.below(texty.len() as u64) as usize];
        let id = flat[i].id.clone();
        steps.push(Step { raw: vec![(id.clone(), vocab(&mut rng, 200))], ..Step::default() });
        let ops = if flat[i].marks.is_empty() {
            Vec::new()
        } else {
            vec![BlockOp::SetText { id: id.clone(), text: flat[i].text.clone(), marks: flat[i].marks.clone() }]
        };
        steps.push(Step { raw: vec![(id, flat[i].text.clone())], ops, ..Step::default() });
    }
    (steps, old)
}

/// Commit times for `n` old steps, strictly increasing between `t0` and `old_end`.
fn old_times(t0: i64, old_end: i64, n: usize) -> Vec<i64> {
    (1..=n as i64).map(|i| t0 + i * (old_end - t0) / (n as i64 + 1)).collect()
}

/// Runs steps as commits at `times`; the device switches with 20% per commit.
fn run_steps(pd: &mut PageDoc, steps: &[Step], times: &[i64], seed: u64, tag: u64, dev: &mut usize) {
    for (i, (s, &t)) in steps.iter().zip(times).enumerate() {
        if Rng::new(seed ^ tag ^ (i as u64).wrapping_mul(0x9E37_79B9)).below(100) < 20 {
            *dev = 1 - *dev;
        }
        pd.set_peer(DEVICES[*dev]).unwrap();
        pd.doc.set_record_timestamp(true);
        pd.doc.set_next_commit_timestamp(t);
        for (k, v) in &s.props {
            pd.set_prop(k, v).unwrap();
        }
        replace_texts(&pd.doc, &s.raw);
        if !s.ops.is_empty() {
            pd.apply(&s.ops).unwrap();
        }
        pd.doc.commit();
    }
}

/// Text containers of a page's live blocks, by block ID. The tree is walked through the
/// public LoroDoc, as PageDoc walks it.
fn block_texts(doc: &LoroDoc) -> BTreeMap<String, LoroText> {
    let tree = doc.get_tree("blocks");
    let mut out = BTreeMap::new();
    let mut stack: Vec<TreeID> = tree.children(TreeParentId::Root).unwrap_or_default();
    while let Some(node) = stack.pop() {
        stack.extend(tree.children(node).unwrap_or_default());
        let Ok(m) = tree.get_meta(node) else { continue };
        let id = match m.get("id") {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => s.to_string(),
            _ => continue,
        };
        if let Some(ValueOrContainer::Container(Container::Text(t))) = m.get("text") {
            out.insert(id, t);
        }
    }
    out
}

/// Replaces block texts by explicit Loro deletion and insertion, with no diff. Only the
/// synthetic history uses it (the manifest discloses this). Marks are left to apply.
fn replace_texts(doc: &LoroDoc, raw: &[(String, String)]) {
    if raw.is_empty() {
        return;
    }
    let texts = block_texts(doc);
    for (id, text) in raw {
        let t = texts.get(id).unwrap_or_else(|| panic!("block {id} has no text"));
        let len = t.len_unicode();
        if len > 0 {
            t.delete(0, len).unwrap();
        }
        if !text.is_empty() {
            t.insert(0, text).unwrap();
        }
    }
}

/// A page document whose creation commit has the seed's meta map, at `t0`. A raw
/// doc is used because `PageDoc::new_page` commits at the wall clock.
fn creation(meta: &[(String, LoroValue)], t0: i64) -> PageDoc {
    let raw = LoroDoc::new();
    raw.set_record_timestamp(true);
    raw.set_peer_id(DEVICES[0]).unwrap();
    raw.set_next_commit_timestamp(t0);
    let map = raw.get_map("page");
    for (k, v) in meta {
        map.insert(k, v.clone()).unwrap();
    }
    raw.commit();
    let bytes = raw.export(ExportMode::Snapshot).unwrap();
    PageDoc::from_bytes(&bytes, &[]).unwrap()
}

struct Replayed {
    bytes: Vec<u8>,
    est: usize,
    attempts: u32,
    edit_block: String,
}

/// Replays one seed head into a history spread over `window_s` seconds before the
/// anchor. The old part is rebuilt with more text until its eligible estimate reaches
/// TARGET_ELIGIBLE (at most MAX_ATTEMPTS). Panics if the final head is not the seed head.
fn replay_page(src: &mut PageDoc, id: &str, seed: u64, big: bool, anchor_s: i64, window_s: i64) -> Replayed {
    let want = head_digest(src);
    let blocks = src.blocks();
    let props: Vec<(String, PropValue)> = src.props().into_iter().map(|p| (p.key, p.value)).collect();
    let meta: Vec<(String, LoroValue)> = match src.doc.get_map("page").get_value() {
        LoroValue::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        other => panic!("page {id}: meta is not a map: {other:?}"),
    };
    let t0 = anchor_s - window_s;
    let old_end = anchor_s - OLD_END_DAYS * DAY_S;
    let recent = [anchor_s - 25 * DAY_S, anchor_s - 20 * DAY_S];
    let mut volume = if big { 1_500 } else { 800 };
    let mut attempts = 0u32;
    let (mut pd, est, steps, old, mut dev) = loop {
        attempts += 1;
        let (steps, old) = plan(&blocks, &props, seed, volume, big);
        let mut pd = creation(&meta, t0);
        let mut dev = 0usize;
        run_steps(&mut pd, &steps[..old], &old_times(t0, old_end, old), seed, 0x5eed, &mut dev);
        let est = pd.doc.export(ExportMode::all_updates()).unwrap().len();
        if est >= TARGET_ELIGIBLE || attempts >= MAX_ATTEMPTS {
            break (pd, est, steps, old, dev);
        }
        volume = (volume * TARGET_ELIGIBLE / est.max(1) * 5 / 4 + 1).min(MAX_VOLUME);
    };
    let recent_steps = &steps[old..];
    run_steps(&mut pd, recent_steps, &recent[..recent_steps.len()], seed, 0xbeef, &mut dev);
    assert_eq!(head_digest(&mut pd), want, "page {id}: replayed head differs from the seed head");
    let edit_block = blocks.first().map(|b| b.id.clone()).unwrap_or_default();
    Replayed { bytes: pd.snapshot().unwrap(), est, attempts, edit_block }
}

struct Outcome {
    id: PageId,
    big: bool,
    est: usize,
    attempts: u32,
    edit_block: String,
}

/// Replays one seed snapshot and writes the new one into `docs`.
fn replay_file(seed_docs: &Path, docs: &Path, id: &str, seed: u64, anchor_s: i64, window_s: i64) -> Outcome {
    let bytes = fs::read(seed_docs.join(format!("{id}.loro"))).unwrap();
    let mut src = PageDoc::from_bytes(&bytes, &[]).unwrap();
    let big = src.meta().title == seed::BIG_PAGE_TITLE;
    let r = replay_page(&mut src, id, seed, big, anchor_s, window_s);
    fs::write(docs.join(format!("{id}.loro")), &r.bytes).unwrap();
    Outcome { id: id.to_owned(), big, est: r.est, attempts: r.attempts, edit_block: r.edit_block }
}

/// Replays all seed pages on up to 8 threads; the outcomes stay in page order.
fn replay_all(seed_docs: &Path, docs: &Path, ids: &[PageId], anchor_s: i64, window_s: i64) -> Vec<Outcome> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let chunk = ids.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..ids.len())
            .step_by(chunk)
            .map(|start| {
                let (ids, seed_docs, docs) = (ids, seed_docs, docs);
                s.spawn(move || {
                    (start..(start + chunk).min(ids.len()))
                        .map(|i| replay_file(seed_docs, docs, &ids[i], i as u64 + 1, anchor_s, window_s))
                        .collect::<Vec<Outcome>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("replay thread")).collect()
    })
}

/// The check record for one page: its version list and a version halfway back.
fn sample_for(v: &Vault, id: &str, big: bool, edit_block: &str) -> Sample {
    let versions = v.list_versions(id.to_owned()).unwrap();
    let old = versions.get(versions.len() / 2).expect("history").id.clone();
    let view = v.load_page_at(id.to_owned(), old.clone()).unwrap();
    Sample {
        page: id.to_owned(),
        big,
        versions: versions.len(),
        old_version: old,
        edit_block: edit_block.to_owned(),
        versions_sha256: digest(&versions),
        old_view_sha256: digest(&view),
    }
}

/// Candidates for the cold check: up to CANDIDATES ordinary pages spread over `normal`,
/// plus the big page.
fn pick_candidates(v: &Vault, normal: &[(PageId, String)], big: Option<(PageId, String)>) -> Vec<Sample> {
    let stride = (normal.len() / CANDIDATES).max(1);
    let mut out: Vec<Sample> = normal.iter().step_by(stride).take(CANDIDATES)
        .map(|(id, edit)| sample_for(v, id, false, edit)).collect();
    if let Some((id, edit)) = big {
        out.push(sample_for(v, &id, true, &edit));
    }
    out
}

fn generate(path: &Path) {
    let pages = env_num("COLD_BENCH_PAGES", 10_000) as u32;
    let big_blocks = env_num("COLD_BENCH_BIG", 2_000) as u32;
    let days = env_num("COLD_BENCH_DAYS", 180);
    let anchor_s = env_num("COLD_BENCH_ANCHOR_S", crate::ids::now_ms() / 1000);
    assert!(days > OLD_END_DAYS + 10, "COLD_BENCH_DAYS must leave room after day {OLD_END_DAYS}");
    assert_empty(path);
    let docs = path.join(".vault/docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(path.join(".vault/cold-policy.json"), IDLE_OFF).unwrap();

    // The seed is an earlier seed::generate output (COLD_BENCH_SEED_SOURCE: only read,
    // never opened for writing, written or deleted), or a fresh one in the sibling
    // <root>.seed, which is ours and is removed after.
    let source = std::env::var("COLD_BENCH_SEED_SOURCE").ok().map(PathBuf::from);
    let seed_dir = sibling(path, ".seed");
    let seed_docs = match &source {
        Some(src) => {
            assert_ne!(src, path, "COLD_BENCH_SEED_SOURCE must not be the root");
            assert!(src.join(".vault/docs").is_dir(), "{} has no .vault/docs", src.display());
            src.join(".vault/docs")
        }
        None => {
            assert!(!seed_dir.exists(), "{} exists; remove it first", seed_dir.display());
            seed::generate(seed_dir.to_string_lossy().into_owned(), pages, big_blocks).expect("seed::generate");
            seed_dir.join(".vault/docs")
        }
    };
    let ids = page_ids(&seed_docs);
    assert_eq!(ids.len(), pages as usize + 1, "seed should write {pages} pages plus the big page");

    let t = Instant::now();
    let outcomes = replay_all(&seed_docs, &docs, &ids, anchor_s, days * DAY_S);
    let replay_ms = t.elapsed().as_secs_f64() * 1000.0;
    if source.is_none() && std::env::var("COLD_BENCH_KEEP_SEED").is_err() {
        fs::remove_dir_all(&seed_dir).unwrap();
    }

    // Opening a vault with snapshots and no index rebuilds the index.
    let t = Instant::now();
    let v = open(&path.to_string_lossy());
    let index_ms = ms(t);
    assert_eq!(v.page_count().unwrap() as usize, ids.len());
    let t = Instant::now();
    let mut digests = Digests::new();
    for o in &outcomes {
        digests.insert(o.id.clone(), page_digest(&v, &o.id));
    }
    let digest_ms = ms(t);

    let mut est: Vec<usize> = outcomes.iter().filter(|o| !o.big).map(|o| o.est).collect();
    assert!(!est.is_empty(), "no ordinary pages");
    est.sort();
    let (min, p50, max) = (est[0], est[est.len() / 2], est[est.len() - 1]);
    let below = est.iter().filter(|&&e| e < MIN_ELIGIBLE).count();
    let retried = outcomes.iter().filter(|o| o.attempts > 1).count();
    let big_est = outcomes.iter().find(|o| o.big).map(|o| o.est).unwrap_or(0);

    let normal: Vec<(PageId, String)> = outcomes.iter()
        .filter(|o| !o.big && o.est >= MIN_ELIGIBLE)
        .map(|o| (o.id.clone(), o.edit_block.clone()))
        .collect();
    let big = outcomes.iter().find(|o| o.big).map(|o| (o.id.clone(), o.edit_block.clone()));
    let candidates = pick_candidates(&v, &normal, big);
    drop(v);

    let seed_label = match &source {
        Some(src) => format!("COLD_BENCH_SEED_SOURCE {} (read only; its page and block IDs are kept)", src.display()),
        None => "seed::generate (random IDs: page and block IDs differ per run)".to_owned(),
    };
    let manifest = serde_json::json!({
        "kind": "generated",
        "generator": {
            "pages": pages, "big_page_blocks": big_blocks, "days": days, "anchor_s": anchor_s,
            "old_end_days": OLD_END_DAYS, "target_eligible_bytes": TARGET_ELIGIBLE,
            "min_eligible_bytes": MIN_ELIGIBLE, "rewrite_rounds": REWRITE_ROUNDS,
            "build_batch": BUILD_BATCH, "big_batch": BIG_BATCH, "candidates": CANDIDATES,
            "devices": DEVICES.len(), "device_switch_percent": 20,
            "seed": seed_label,
            "artificial_raw_rewrites": "appends, rewrites and text restores replace block LoroText by explicit delete+insert (no text diff); marks, structure and props go through PageDoc::apply. Artificial benchmark history only.",
        },
        "pages": outcomes.len(),
        "eligible_estimate_pages": est.len() - below,
        "samples": [],
    });
    write_json(&path.join("bench-manifest.json"), &manifest);
    write_json(&path.join("bench-digests.json"), &digests);
    write_json(&path.join("bench-candidates.json"), &candidates);
    println!(
        "COLD_BENCH generate pages={} big_blocks={big_blocks} days={days} replay_ms={replay_ms:.0} \
         index_ms={index_ms:.0} digest_ms={digest_ms:.0} est_min={min} est_p50={p50} est_max={max} \
         est_big={big_est} below_16k={below} retried={retried} candidates={} hot_bytes={}",
        outcomes.len(), candidates.len(), bytes_in(&docs, ".loro")
    );
}

/// Copies the real vault into the root and records what the cold pass should do.
fn prepare(path: &Path) {
    let source = PathBuf::from(std::env::var("COLD_BENCH_SOURCE").expect("COLD_BENCH_SOURCE: the real vault to copy"));
    assert!(source.join(".vault/docs").is_dir(), "{} is not a vault", source.display());
    assert_eq!(files_in(&source.join(".vault/cold"), ".cold"), 0, "the source already has cold files; refusing to copy it");
    assert_empty(path);
    let links = copy_dir(&source, path).expect("copy the real vault");
    fs::write(path.join(".vault/cold-policy.json"), IDLE_OFF).unwrap();

    let docs = path.join(".vault/docs");
    let cutoff = crate::ids::now_ms() / 1000 - ColdPolicy::default().age_seconds;
    let v = open(&path.to_string_lossy());
    let ids = page_ids(&docs);
    let trash_small = env_num("COLD_BENCH_TRASH_SMALL", 0) as usize;
    let mut trashed_small: Vec<PageId> = Vec::new();
    for id in &ids {
        if trashed_small.len() == trash_small {
            break;
        }
        let doc = v.inner.read_doc(id).unwrap();
        let est = eligible_estimate(&doc, cutoff);
        if !in_trash(&v, id) && est > 0 && est < MIN_ELIGIBLE {
            v.delete_page(id.clone()).unwrap();
            trashed_small.push(id.clone());
        }
    }
    if !trashed_small.is_empty() {
        v.flush().unwrap();
    }

    // Digests are taken after trashing, so the cold pass must preserve them.
    let mut digests = Digests::new();
    let mut cands: Vec<(PageId, String, u64)> = Vec::new();
    for id in &ids {
        digests.insert(id.clone(), page_digest(&v, id));
        let mut doc = v.inner.read_doc(id).unwrap();
        let edit = doc.blocks().first().map(|b| b.id.clone()).unwrap_or_default();
        if !edit.is_empty() && (in_trash(&v, id) || eligible_estimate(&doc, cutoff) >= MIN_ELIGIBLE) {
            cands.push((id.clone(), edit, file_len(&docs.join(format!("{id}.loro")))));
        }
    }
    assert!(!cands.is_empty(), "no page is predicted to become cold");
    let big = cands.iter().max_by_key(|c| c.2).map(|c| (c.0.clone(), c.1.clone()));
    let big_id = big.as_ref().map(|b| b.0.clone());
    let normal: Vec<(PageId, String)> = cands.iter()
        .filter(|c| Some(&c.0) != big_id.as_ref())
        .map(|c| (c.0.clone(), c.1.clone()))
        .collect();
    let candidates = pick_candidates(&v, &normal, big);
    drop(v);

    let manifest = serde_json::json!({
        "kind": "prepared",
        "source": source.to_string_lossy(),
        "pages": ids.len(),
        "predicted_cold": cands.len(),
        "trashed_small": trashed_small,
        "samples": [],
    });
    write_json(&path.join("bench-manifest.json"), &manifest);
    write_json(&path.join("bench-digests.json"), &digests);
    write_json(&path.join("bench-candidates.json"), &candidates);
    println!(
        "COLD_BENCH prepare pages={} predicted_cold={} trashed_small={} candidates={} links_skipped={links}",
        ids.len(), cands.len(), trashed_small.len(), candidates.len()
    );
}

/// One cold pass over every page with the real clock, then the checks. Samples are
/// taken from the candidates that moved.
fn cold(path: &Path) {
    let root = path.to_string_lossy().into_owned();
    let docs = path.join(".vault/docs");
    let cold_dir = path.join(".vault/cold");
    let dicts = cold_dir.join("dictionaries");
    let before: Digests = read_json(&path.join("bench-digests.json"));
    let candidates: Vec<Sample> = read_json(&path.join("bench-candidates.json"));
    let manifest_path = path.join("bench-manifest.json");
    let mut manifest: serde_json::Value = read_json(&manifest_path);
    let generated = manifest["kind"] == "generated";
    fs::write(path.join(".vault/cold-policy.json"), IDLE_OFF).unwrap();
    let pages = page_ids(&docs);
    assert_eq!(pages.len(), before.len(), "page count differs from bench-digests.json");
    let hot_before: BTreeMap<PageId, u64> = pages.iter()
        .map(|id| (id.clone(), file_len(&docs.join(format!("{id}.loro"))))).collect();

    let v = open(&root);
    v.flush().unwrap();
    let at_ms = crate::ids::now_ms();
    let policy = ColdPolicy::default();
    let cutoff = at_ms / 1000 - policy.age_seconds;
    let mut moved: BTreeSet<PageId> = BTreeSet::new();
    let (mut pass_ms, mut retreated) = (0.0f64, 0usize);
    let mut ops_behind: Vec<u64> = Vec::new();
    let mut secs_behind: Vec<i64> = Vec::new();
    let mut cutoff_gaps: Vec<i64> = Vec::new();
    let (mut plain_cold_bytes, mut encoded_cold_bytes, mut dictionary_cold_files) = (0u64, 0u64, 0usize);
    let no_dictionary = path.join("dictionary-comparison-without-vault");
    assert!(!no_dictionary.exists());
    for id in &pages {
        let doc = v.inner.read_doc(id).unwrap();
        // A trashed page takes its full history; otherwise the cutoff root (as cold_page does).
        let requested = if in_trash(&v, id) {
            Some(doc.doc.oplog_frontiers())
        } else {
            crate::history::cutoff_root(&doc, cutoff).unwrap()
        };
        let t = Instant::now();
        let did = v.with_db(|db| v.inner.cold_page(db, id, &policy, at_ms)).unwrap();
        pass_ms += ms(t);
        if !did {
            continue;
        }
        moved.insert(id.clone());
        let requested = requested.expect("a moved page has a requested root");
        let part = cold::read(path, id).unwrap();
        encoded_cold_bytes += part.updates.len() as u64;
        plain_cold_bytes += cold::prepare(&no_dictionary, &part).unwrap().len() as u64;
        dictionary_cold_files += (fs::read(cold::path(path, id)).unwrap()[5] == 1) as usize;
        let act: BTreeMap<u64, i32> = canon_vv(&part.root.vv).into_iter().collect();
        let req_vv = doc.doc.frontiers_to_vv(&requested).expect("requested root is in the page");
        // Changes in the requested root that the cold root does not cover.
        let behind: u64 = canon_vv(&req_vv).into_iter()
            .map(|(p, c)| (c - act.get(&p).copied().unwrap_or(0)).max(0) as u64)
            .sum();
        ops_behind.push(behind);
        let latest = |f: &Frontiers| f.iter()
            .map(|op| doc.doc.get_change(ID::new(op.peer, op.counter)).unwrap().timestamp)
            .max().unwrap_or(0);
        secs_behind.push(latest(&requested) - latest(&part.root.frontiers));
        cutoff_gaps.extend(part.root.frontiers.iter().map(|op|
            cutoff - doc.doc.get_change(ID::new(op.peer, op.counter)).unwrap().timestamp));
        retreated += (canon_frontiers(&requested) != canon_frontiers(&part.root.frontiers)) as usize;
        // The vault trains its dictionary after a pass that moved pages. The harness
        // runs pages one by one, so it asks after each moved page until one exists.
        if files_in(&dicts, ".dict") == 0 {
            cold::maybe_train_dictionary(path).unwrap();
        }
    }
    drop(v);

    println!("COLD_BENCH compression encoded_prefix_bytes={encoded_cold_bytes} \
        cold_without_dictionary_bytes={plain_cold_bytes} cold_with_dictionary_bytes={} \
        dictionary_cold_files={dictionary_cold_files}", bytes_in(&cold_dir, ".cold"));

    let moved_cands: Vec<&Sample> = candidates.iter().filter(|c| moved.contains(&c.page)).collect();
    let normal_moved: Vec<&Sample> = moved_cands.iter().copied().filter(|c| !c.big).collect();
    let stride = (normal_moved.len() / SAMPLES).max(1);
    let mut samples: Vec<Sample> = normal_moved.iter().step_by(stride).take(SAMPLES).map(|c| (*c).clone()).collect();
    if let Some(b) = moved_cands.iter().find(|c| c.big) {
        samples.push((*b).clone());
    }
    println!(
        "COLD_BENCH pass pages={} moved={} candidates={} moved_candidates={} \
         pass_total_ms={pass_ms:.0} per_page_ms={:.3} retreated={retreated} ops_behind_sum={} ops_behind_max={} \
         secs_behind_min={} secs_behind_max={} cutoff_gap_s_min={} cutoff_gap_s_max={} \
         cold_files={} dictionary_files={} dictionary_bytes={} hot_bytes={} cold_bytes={}",
        pages.len(), moved.len(), candidates.len(), moved_cands.len(), pass_ms / pages.len().max(1) as f64,
        ops_behind.iter().sum::<u64>(), ops_behind.iter().max().copied().unwrap_or(0),
        secs_behind.iter().min().copied().unwrap_or(0), secs_behind.iter().max().copied().unwrap_or(0),
        cutoff_gaps.iter().min().copied().unwrap_or(0), cutoff_gaps.iter().max().copied().unwrap_or(0),
        files_in(&cold_dir, ".cold"), files_in(&dicts, ".dict"), bytes_in(&dicts, ".dict"),
        bytes_in(&docs, ".loro"), bytes_in(&cold_dir, ".cold")
    );
    if let Some(b) = candidates.iter().find(|c| c.big) {
        println!(
            "COLD_BENCH big page={} moved={} hot_before={} hot_after={} cold_file={}",
            b.page, moved.contains(&b.page),
            hot_before.get(&b.page).copied().unwrap_or(0),
            file_len(&docs.join(format!("{}.loro", b.page))),
            file_len(&cold_dir.join(format!("{}.cold", b.page))),
        );
    }

    // Every page must match its digest; every candidate's version list and old view too.
    let v = open(&root);
    let t = Instant::now();
    let mismatched: Vec<&PageId> = pages.iter()
        .filter(|id| page_digest(&v, id) != before[id.as_str()])
        .collect();
    let (mut versions_bad, mut old_bad) = (0usize, 0usize);
    for c in &candidates {
        let versions = v.list_versions(c.page.clone()).unwrap();
        versions_bad += (digest(&versions) != c.versions_sha256) as usize;
        let view = v.load_page_at(c.page.clone(), c.old_version.clone()).unwrap();
        old_bad += (digest(&view) != c.old_view_sha256) as usize;
    }
    let check_ms = ms(t);
    drop(v);

    manifest["samples"] = serde_json::to_value(&samples).unwrap();
    manifest["cold"] = serde_json::json!({
        "pages": pages.len(), "moved": moved.len(), "candidates": candidates.len(),
        "moved_candidates": moved_cands.len(), "samples": samples.len(),
        "mismatched": mismatched.len(), "versions_mismatch": versions_bad, "old_view_mismatch": old_bad,
    });
    write_json(&manifest_path, &manifest);
    println!(
        "COLD_BENCH check pages={} mismatched={} first_mismatch={:?} candidates={} versions_mismatch={versions_bad} \
         old_view_mismatch={old_bad} samples={} check_ms={check_ms:.0}",
        pages.len(), mismatched.len(), mismatched.first(), candidates.len(), samples.len()
    );
    assert!(mismatched.is_empty(), "cold pass changed page state");
    assert_eq!(versions_bad, 0, "cold pass changed a version list");
    assert_eq!(old_bad, 0, "cold pass changed an old version");
    assert!(!samples.is_empty(), "no candidate page became cold");
    if generated {
        assert!(moved.len() * 2 > pages.len(), "most pages should have eligible cold history");
        assert!(files_in(&cold_dir, ".cold") >= 20, "fewer than 20 cold files");
        assert_eq!(files_in(&dicts, ".dict"), 1, "expected one trained dictionary");
        assert!(samples.iter().any(|s| s.big), "the big page did not become cold");
    }
}

#[test]
#[ignore = "opt-in: needs a disposable COLD_BENCH_ROOT"]
fn measure_cold_history() {
    let root = std::env::var("COLD_BENCH_ROOT").expect("disposable COLD_BENCH_ROOT");
    let phase = std::env::var("COLD_BENCH_PHASE").unwrap_or_else(|_| "generate".into());
    let path = PathBuf::from(&root);
    match phase.as_str() {
        "generate" => generate(&path),
        "prepare" => prepare(&path),
        "cold" => cold(&path),
        other => panic!("unknown COLD_BENCH_PHASE {other}"),
    }
}
