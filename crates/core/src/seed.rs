//! Test vaults for the bake-off and benchmarks.

use std::fs;

use crate::attach::sha256_hex;
use crate::doc::{plain_text, PageDoc};
use crate::error::{invalid, Result};
use crate::ids::{new_id, now_ms};
use crate::index::{ContentRow, MetaRow};
use crate::markdown::{self, RenderCtx};
use crate::order;
use crate::types::{
    BlockKind, BlockOp, Condition, Filter, FilterOp, Mark, MarkKind, NewBlock, PageId, PropDef,
    PropKind, PropValue, SelectOption, Sort, ViewDef, ViewKind,
};
use crate::vault::{write_atomic, Vault};

/// Title of the seeded page with many blocks.
pub const BIG_PAGE_TITLE: &str = "Big page";

const WORDS: &[&str] = &[
    "project",
    "meeting",
    "notes",
    "garden",
    "recipe",
    "travel",
    "budget",
    "design",
    "review",
    "plan",
    "idea",
    "journal",
    "reading",
    "book",
    "movie",
    "music",
    "health",
    "workout",
    "habit",
    "goal",
    "weekly",
    "monthly",
    "quarter",
    "roadmap",
    "launch",
    "release",
    "bug",
    "feature",
    "research",
    "draft",
    "letter",
    "invoice",
    "contract",
    "client",
    "team",
    "family",
    "birthday",
    "holiday",
    "kitchen",
    "bicycle",
    "mountain",
    "river",
    "ocean",
    "forest",
    "city",
    "village",
    "train",
    "flight",
    "hotel",
    "museum",
    "coffee",
    "tea",
    "bread",
    "cheese",
    "tomato",
    "garlic",
    "onion",
    "pepper",
    "lemon",
    "apple",
    "server",
    "database",
    "network",
    "backup",
    "sync",
    "editor",
    "keyboard",
    "screen",
    "window",
    "phone",
    "morning",
    "evening",
    "weekend",
    "summer",
    "winter",
    "spring",
    "autumn",
    "rain",
    "snow",
    "sunshine",
    "quick",
    "slow",
    "bright",
    "quiet",
    "simple",
    "careful",
    "honest",
    "curious",
    "gentle",
    "steady",
    "write",
    "build",
    "test",
    "measure",
    "compare",
    "decide",
    "share",
    "collect",
    "sketch",
    "polish",
    "paper",
    "pencil",
    "canvas",
    "camera",
    "photo",
    "album",
    "letterbox",
    "garage",
    "attic",
    "balcony",
    "physics",
    "history",
    "language",
    "grammar",
    "algebra",
    "chemistry",
    "biology",
    "poetry",
    "novel",
    "essay",
    "café",
    "Müller",
    "naïve",
    "über",
    "façade",
    "Zürich",
    "résumé",
    "jalapeño",
    "Straße",
    "smörgåsbord",
];

const TITLE_FORMS: &[&str] = &[
    "{a} {b}",
    "Notes on {a}",
    "{A} plan",
    "Weekly {a} review",
    "{A} and {b}",
    "Ideas for the {a}",
    "{A} checklist",
    "Draft: {a} {b}",
    "{A} log",
    "How to {a} a {b}",
];

/// Small deterministic generator (xorshift64*), so seeds are reproducible
/// without a random-number dependency.
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
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn word(&mut self) -> &'static str {
        WORDS[self.below(WORDS.len() as u64) as usize]
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn title(rng: &mut Rng) -> String {
    let form = TITLE_FORMS[rng.below(TITLE_FORMS.len() as u64) as usize];
    let a = rng.word();
    let b = rng.word();
    form.replace("{A}", &capitalize(a))
        .replace("{a}", a)
        .replace("{b}", b)
}

/// A sentence-like paragraph, sometimes with one bold or italic word.
fn paragraph(rng: &mut Rng, min_words: u64, max_words: u64) -> (String, Vec<Mark>) {
    let n = min_words + rng.below(max_words - min_words + 1);
    let mut text = String::new();
    let mut marks = Vec::new();
    let marked = if rng.chance(30) {
        Some(rng.below(n))
    } else {
        None
    };
    for i in 0..n {
        if i > 0 {
            text.push(' ');
        }
        let w = rng.word();
        let w = if i == 0 { capitalize(w) } else { w.to_string() };
        let start = text.chars().count() as u32;
        text.push_str(&w);
        if marked == Some(i) {
            let kind = if rng.chance(50) {
                MarkKind::Bold
            } else {
                MarkKind::Italic
            };
            marks.push(Mark {
                start,
                end: start + w.chars().count() as u32,
                kind,
            });
        }
    }
    text.push('.');
    (text, marks)
}

fn insert(
    parent: Option<&str>,
    after: Option<&str>,
    id: &str,
    kind: BlockKind,
    text: String,
    marks: Vec<Mark>,
    checked: bool,
) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(str::to_string),
        after: after.map(str::to_string),
        block: NewBlock {
            id: id.to_string(),
            kind,
            text,
            marks,
            checked,
        },
    }
}

fn small_page_ops(rng: &mut Rng) -> Vec<BlockOp> {
    let mut ops = Vec::new();
    let mut last: Option<String> = None;
    let n = 2 + rng.below(4);
    for _ in 0..n {
        let id = new_id();
        let (text, marks) = paragraph(rng, 12, 40);
        let kind = if rng.chance(15) {
            BlockKind::Bulleted
        } else {
            BlockKind::Paragraph
        };
        ops.push(insert(None, last.as_deref(), &id, kind, text, marks, false));
        last = Some(id);
    }
    ops
}

/// Ops for the big page: `count` blocks of every kind, with nested list
/// items, marks (including links) and non-ASCII text.
pub(crate) fn big_page_ops(count: u32, seed: u64) -> Vec<BlockOp> {
    let mut rng = Rng::new(seed);
    let mut ops = Vec::with_capacity(count as usize);
    let mut last_top: Option<String> = None;
    // (parent id, last child id) of the list item that may take children.
    let mut nest: Option<(String, Option<String>)> = None;
    let mut made = 0u32;
    while made < count {
        let id = new_id();
        let section = made % 40;
        let (kind, text, marks, checked) = match section {
            0 => {
                let k = [
                    BlockKind::Heading1,
                    BlockKind::Heading2,
                    BlockKind::Heading3,
                ][(made / 40 % 3) as usize].clone();
                (
                    k,
                    format!("Section {} — {}", made / 40 + 1, capitalize(rng.word())),
                    vec![],
                    false,
                )
            }
            1..=6 => {
                let (t, m) = paragraph(&mut rng, 20, 60);
                (BlockKind::Paragraph, t, m, false)
            }
            7..=14 => {
                let (t, m) = paragraph(&mut rng, 4, 12);
                (BlockKind::Bulleted, t, m, false)
            }
            15..=19 => {
                let (t, m) = paragraph(&mut rng, 4, 10);
                (BlockKind::Numbered, t, m, false)
            }
            20..=25 => {
                let (t, m) = paragraph(&mut rng, 3, 8);
                let c = rng.chance(40);
                (BlockKind::Todo, t, m, c)
            }
            26 => {
                let (t, _) = paragraph(&mut rng, 10, 25);
                (BlockKind::Quote, t, vec![], false)
            }
            27 => (
                BlockKind::Code,
                format!(
                    "fn {}() {{\n    println!(\"{}\");\n}}",
                    rng.word()
                        .to_lowercase()
                        .replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
                    rng.word()
                ),
                vec![],
                false,
            ),
            28 => (BlockKind::Divider, String::new(), vec![], false),
            _ => {
                let (t, mut m) = paragraph(&mut rng, 10, 30);
                // A link over the first word and a strike or code mark.
                let first = t.chars().take_while(|c| *c != ' ').count() as u32;
                m.retain(|x| x.start > first);
                m.push(Mark {
                    start: 0,
                    end: first,
                    kind: MarkKind::Link(format!("https://example.com/{made}")),
                });
                if rng.chance(50) {
                    m.push(Mark {
                        start: first + 1,
                        end: (first + 5).min(t.chars().count() as u32),
                        kind: MarkKind::Strike,
                    });
                }
                (BlockKind::Paragraph, t, m, false)
            }
        };
        // Some bulleted items become children of the previous top-level one.
        let is_list = matches!(kind, BlockKind::Bulleted);
        if is_list && nest.is_some() && rng.chance(40) {
            let (p, after) = nest.take().unwrap();
            ops.push(insert(
                Some(&p),
                after.as_deref(),
                &id,
                kind,
                text,
                marks,
                checked,
            ));
            nest = Some((p, Some(id)));
        } else {
            ops.push(insert(
                None,
                last_top.as_deref(),
                &id,
                kind,
                text,
                marks,
                checked,
            ));
            nest = if is_list {
                Some((id.clone(), None))
            } else {
                None
            };
            last_top = Some(id);
        }
        made += 1;
    }
    ops
}

type Built = (MetaRow, ContentRow);

#[allow(clippy::too_many_arguments)]
fn build_page(
    vault: &Vault,
    id: &str,
    title: &str,
    parent: Option<&str>,
    created: i64,
    updated: i64,
    order_key: &str,
    color: Option<&str>,
    ops: &[BlockOp],
) -> Result<Built> {
    let mut doc = PageDoc::new_page(vault.inner().peer, id, title, parent, created)?;
    doc.set_meta_str("order", Some(order_key))?;
    doc.set_meta_str("color", color)?;
    doc.apply(ops)?;
    doc.set_updated(updated)?;
    doc.doc.commit();
    let inner = vault.inner();
    write_atomic(&inner.doc_path(id), &doc.snapshot()?)?;
    let meta = doc.meta();
    let blocks = doc.blocks();
    if blocks.len() > 100 {
        inner.store_block_cache(id, &doc.version(), &blocks)?;
    }
    let name = markdown::file_name(title, id);
    let text = markdown::render(&meta, &[], &blocks, &RenderCtx::none());
    fs::write(inner.pages_dir().join(&name), text.as_bytes())?;
    let row = crate::vault::meta_row(&doc);
    let content = ContentRow {
        body: plain_text(&blocks, &|_: &str| None),
        md_path: Some(name),
        md_hash: Some(sha256_hex(text.as_bytes())),
        md_version: Some(doc.version()),
        links: Vec::new(),
        refs: Vec::new(),
        vv: Some(doc.doc.oplog_vv().encode()),
        todos: crate::agenda::block_agenda(&blocks),
    };
    Ok((row, content))
}

/// Creates a test vault at `root`: `pages` pages with varied titles, some
/// nested under others, and a few paragraphs each, plus one page titled
/// "Big page" with `big_page_blocks` blocks of every kind. The big page is the
/// most recently updated. Fails if the vault already has pages.
pub fn generate(root: String, pages: u32, big_page_blocks: u32) -> Result<()> {
    let vault = Vault::open(root)?;
    if vault.page_count()? > 0 {
        return Err(invalid("the vault already has pages"));
    }
    let now = now_ms();
    let n = pages as usize;
    let ids: Vec<String> = (0..n).map(|_| new_id()).collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunk = n.div_ceil(threads).max(1);
    let year_ms: u64 = 365 * 24 * 3600 * 1000;

    let keys = order::spread(n + 1);
    // Assign root colors before snapshots and indexing. A post-build backfill
    // queues thousands of writer flushes while foreground metadata writes
    // contend for SQLite's write lock. Preserve the sidebar-order color cycle.
    let mut roots = 0usize;
    let colors: Vec<_> = (0..n)
        .map(|i| {
            let mut rng = Rng::new(i as u64 + 1);
            if i >= n / 20 && rng.chance(60) {
                None
            } else {
                let color = crate::agenda::SPACE_COLORS[roots % 6];
                roots += 1;
                Some(color)
            }
        })
        .collect();
    let mut rows: Vec<Built> = Vec::with_capacity(n + 1);
    std::thread::scope(|s| -> Result<()> {
        let handles: Vec<_> = (0..n)
            .step_by(chunk)
            .map(|start| {
                let ids = &ids;
                let vault = &vault;
                let keys = &keys;
                let colors = &colors;
                s.spawn(move || -> Result<Vec<Built>> {
                    let mut out = Vec::new();
                    for i in start..(start + chunk).min(n) {
                        let mut rng = Rng::new(i as u64 + 1);
                        let parent = if i >= n / 20 && rng.chance(60) {
                            Some(ids[rng.below(i as u64) as usize].as_str())
                        } else {
                            None
                        };
                        let updated = now - 60_000 - rng.below(year_ms) as i64;
                        let created = updated - rng.below(year_ms / 12) as i64;
                        let title = title(&mut rng);
                        let ops = small_page_ops(&mut rng);
                        out.push(build_page(
                            vault, &ids[i], &title, parent, created, updated, &keys[i], colors[i],
                            &ops,
                        )?);
                    }
                    Ok(out)
                })
            })
            .collect();
        for h in handles {
            rows.extend(h.join().map_err(|_| invalid("seed thread panicked"))??);
        }
        Ok(())
    })?;

    let big_id = new_id();
    let ops = big_page_ops(big_page_blocks, 42);
    rows.push(build_page(
        &vault,
        &big_id,
        BIG_PAGE_TITLE,
        None,
        now - 86_400_000,
        now,
        &keys[n],
        Some(crate::agenda::SPACE_COLORS[roots % 6]),
        &ops,
    )?);

    vault.with_db(|db| {
        let tx = db.write_tx()?;
        for (m, c) in &rows {
            db.upsert(m, c)?;
        }
        tx.commit()?;
        Ok(())
    })?;
    Ok(())
}

/// Title of the collection made by [`generate_tasks`].
pub const TASKS_TITLE: &str = "Tasks";

const DAY_MS: i64 = 86_400_000;
const TAGS: [&str; 6] = ["home", "work", "garden", "travel", "health", "money"];

fn option(id: &str, name: &str, color: &str) -> SelectOption {
    SelectOption {
        id: id.into(),
        name: name.into(),
        color: color.into(),
    }
}

/// The schema of the sample "Tasks" collection.
fn tasks_schema() -> Vec<PropDef> {
    let def = |key: &str, name: &str, kind: PropKind, options: Vec<SelectOption>| PropDef {
        key: key.into(),
        name: name.into(),
        kind,
        options,
        config: String::new(),
    };
    vec![
        def(
            "status",
            "Status",
            PropKind::Status,
            vec![
                option("todo", "To do", "gray"),
                option("doing", "In progress", "blue"),
                option("done", "Done", "green"),
            ],
        ),
        def("due", "Due", PropKind::Date, Vec::new()),
        def(
            "tags",
            "Tags",
            PropKind::MultiSelect,
            TAGS.iter()
                .zip(["red", "blue", "green", "yellow", "pink", "purple"])
                .map(|(t, c)| option(t, &capitalize(t), c))
                .collect(),
        ),
        def("estimate", "Estimate", PropKind::Number, Vec::new()),
        def("urgent", "Urgent", PropKind::Checkbox, Vec::new()),
        def("created", "Created", PropKind::CreatedTime, Vec::new()),
    ]
}

fn tasks_views() -> Vec<ViewDef> {
    let view = |id: &str, name: &str, kind: ViewKind| ViewDef {
        id: id.into(),
        name: name.into(),
        kind,
        filter: None,
        sorts: Vec::new(),
        group_by: None,
        date_prop: None,
        visible_props: vec![
            "status".into(),
            "due".into(),
            "tags".into(),
            "estimate".into(),
        ],
        card_cover: None,
        config: String::new(),
    };
    let open = Filter::And(vec![
        Filter::Cond(Condition {
            prop: "status".into(),
            op: FilterOp::IsNot,
            value: PropValue::Status("done".into()),
        }),
        Filter::Cond(Condition {
            prop: "due".into(),
            op: FilterOp::IsNotEmpty,
            value: PropValue::Empty,
        }),
    ]);
    vec![
        view("all", "All tasks", ViewKind::Table),
        ViewDef {
            filter: Some(open),
            sorts: vec![Sort {
                prop: "due".into(),
                descending: false,
            }],
            ..view("open", "Open by due date", ViewKind::Table)
        },
        ViewDef {
            group_by: Some("status".into()),
            ..view("board", "Board", ViewKind::Board)
        },
        ViewDef {
            date_prop: Some("due".into()),
            ..view("calendar", "Calendar", ViewKind::Calendar)
        },
        ViewDef {
            date_prop: Some("due".into()),
            ..view("timeline", "Timeline", ViewKind::Timeline)
        },
    ]
}

/// Properties of one sample task. Due dates are whole days spread over 180
/// days around `now`; one in ten is empty, one in ten a range of up to a
/// week.
fn task_props(rng: &mut Rng, now: i64) -> Vec<(String, PropValue)> {
    let mut out = Vec::new();
    let status = ["todo", "doing", "done"][rng.below(3) as usize];
    out.push(("status".to_string(), PropValue::Status(status.into())));
    if !rng.chance(10) {
        let today = now.div_euclid(DAY_MS) * DAY_MS;
        let start = today + (rng.below(180) as i64 - 90) * DAY_MS;
        let end = rng
            .chance(10)
            .then(|| start + (1 + rng.below(7) as i64) * DAY_MS);
        out.push((
            "due".to_string(),
            PropValue::Date {
                start_ms: start,
                end_ms: end,
                has_time: false,
            },
        ));
    }
    let mut t: Vec<String> = Vec::new();
    for _ in 0..rng.below(4) {
        let x = TAGS[rng.below(TAGS.len() as u64) as usize].to_string();
        if !t.contains(&x) {
            t.push(x);
        }
    }
    if !t.is_empty() {
        out.push(("tags".to_string(), PropValue::MultiSelect(t)));
    }
    if !rng.chance(20) {
        out.push((
            "estimate".to_string(),
            PropValue::Number([1.0, 2.0, 3.0, 5.0, 8.0, 13.0][rng.below(6) as usize]),
        ));
    }
    if rng.chance(15) {
        out.push(("urgent".to_string(), PropValue::Checkbox(true)));
    }
    out
}

/// Adds one sample collection titled "Tasks" (at the top level) to the vault
/// at `root`, with `rows` rows, a schema (status, due date, tags, estimate,
/// urgent, created time) and five views (all, open by due date, board,
/// calendar, timeline). Deterministic apart from ids and times. Returns the
/// collection's id. Fails if a top-level page titled "Tasks" exists. Do not
/// call it while the vault is open elsewhere.
pub fn generate_tasks(root: String, rows: u32) -> Result<PageId> {
    let vault = Vault::open(root)?;
    let exists = vault
        .list_children(None, 0, u32::MAX)?
        .iter()
        .any(|p| p.title == TASKS_TITLE);
    if exists {
        return Err(invalid("the vault already has a Tasks page"));
    }
    let now = now_ms();
    let schema = tasks_schema();
    let cid = new_id();
    let last = vault.with_db(|db| db.last_child_key(None))?;
    let mut coll = PageDoc::new_page(vault.inner().peer, &cid, TASKS_TITLE, None, now)?;
    let spaces: u32 = vault.with_db(|db| {
        Ok(db.conn.query_row(
            "SELECT count(*) FROM pages WHERE parent IS NULL AND in_trash = 0",
            [],
            |r| r.get(0),
        )?)
    })?;
    coll.set_meta_str(
        "color",
        Some(crate::agenda::SPACE_COLORS[spaces as usize % 6]),
    )?;
    coll.set_meta_str("order", Some(&order::between(last.as_deref(), None)))?;
    coll.set_meta_str("icon", Some("\u{2705}"))?;
    coll.set_page_type(crate::types::PAGE_TYPE_DATABASE)?;
    coll.set_schema(&schema)?;
    coll.set_views(&tasks_views())?;
    coll.doc.commit();

    let n = rows as usize;
    let keys = order::spread(n);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunk = n.div_ceil(threads).max(1);
    let mut built: Vec<Built> = Vec::with_capacity(n + 1);
    std::thread::scope(|s| -> Result<()> {
        let handles: Vec<_> = (0..n)
            .step_by(chunk)
            .map(|start| {
                let (keys, vault, schema, cid) = (&keys, &vault, &schema, &cid);
                s.spawn(move || -> Result<Vec<Built>> {
                    let mut out = Vec::new();
                    for (i, key) in keys.iter().enumerate().skip(start).take(chunk) {
                        let mut rng = Rng::new(1_000_003 + i as u64);
                        let title = format!("{} {}", capitalize(rng.word()), rng.word());
                        let created = now - 60_000 - rng.below(90 * DAY_MS as u64) as i64;
                        let id = new_id();
                        let mut doc =
                            PageDoc::new_page(vault.inner().peer, &id, &title, Some(cid), created)?;
                        doc.set_meta_str("order", Some(key))?;
                        for (k, v) in task_props(&mut rng, now) {
                            doc.set_prop(&k, &v)?;
                        }
                        if rng.chance(30) {
                            let (text, marks) = paragraph(&mut rng, 4, 12);
                            doc.apply(&[BlockOp::Insert {
                                parent: None,
                                after: None,
                                block: NewBlock {
                                    id: new_id(),
                                    kind: BlockKind::Paragraph,
                                    text,
                                    marks,
                                    checked: false,
                                },
                            }])?;
                        }
                        doc.set_updated(created)?;
                        doc.doc.commit();
                        out.push(write_page(vault, &mut doc, Some(schema))?);
                    }
                    Ok(out)
                })
            })
            .collect();
        for h in handles {
            built.extend(h.join().map_err(|_| invalid("seed thread panicked"))??);
        }
        Ok(())
    })?;
    let coll_row = write_page(&vault, &mut coll, None)?;
    vault.with_db(|db| {
        let tx = db.write_tx()?;
        db.upsert(&coll_row.0, &coll_row.1)?;
        for (m, c) in &built {
            db.upsert(m, c)?;
        }
        tx.commit()?;
        Ok(())
    })?;
    Ok(cid)
}

/// Writes a built page's snapshot and markdown file; returns its index rows.
fn write_page(vault: &Vault, doc: &mut PageDoc, schema: Option<&[PropDef]>) -> Result<Built> {
    let inner = vault.inner();
    let meta = doc.meta();
    write_atomic(&inner.doc_path(&meta.id), &doc.snapshot()?)?;
    let blocks = doc.blocks();
    let row = crate::vault::meta_row(doc);
    let no_titles = |_: &str| None;
    let ctx = RenderCtx {
        titles: &no_titles,
        schema,
    };
    let name = markdown::file_name(&meta.title, &meta.id);
    let text = markdown::render(&meta, &row.props, &blocks, &ctx);
    fs::write(inner.pages_dir().join(&name), text.as_bytes())?;
    let content = ContentRow {
        body: plain_text(&blocks, &no_titles),
        md_path: Some(name),
        md_hash: Some(sha256_hex(text.as_bytes())),
        md_version: Some(doc.version()),
        links: Vec::new(),
        refs: Vec::new(),
        vv: Some(doc.doc.oplog_vv().encode()),
        todos: crate::agenda::block_agenda(&blocks),
    };
    Ok((row, content))
}
