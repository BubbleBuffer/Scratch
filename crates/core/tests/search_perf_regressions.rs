//! Result, order, snippet and total oracles for the search paths that the
//! performance work touches: batched title-hit snippets and small-candidate
//! filter-only sorting. Every expectation is computed independently: snippets
//! come from the per-row `snippet(fts, ...)` query, and order comes from the
//! unchanged `ORDER BY` applied to a membership list taken from the fixture.
//! An optimization that changes any of these fails here.
use std::fs;
use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};
use vault_core::*;

const BASE: i64 = 1_791_590_400_000; // 2026-10-10, midnight UTC.
const DAY: i64 = 86_400_000;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_search_perf_regress_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }

    /// A second connection to the derived index, for legacy oracles.
    fn index(&self) -> Connection {
        Connection::open(self.0.join(".vault/index.sqlite")).unwrap()
    }

    // Fixed timestamps (written after the vault's own property writes) make
    // the ordering oracle independent of scheduling.
    fn set_times(&self, id: &str, created: i64, updated: i64) {
        self.index()
            .execute(
                "UPDATE pages SET created_ms = ?1, updated_ms = ?2 WHERE id = ?3",
                params![created, updated, id],
            )
            .unwrap();
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn paragraph(v: &Vault, id: &str, text: &str) {
    v.apply(
        id.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: new_id(),
                kind: BlockKind::Paragraph,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
}

fn page(v: &Vault, title: &str, text: &str) -> PageId {
    let id = v.create_page(None, title.into()).unwrap();
    if !text.is_empty() {
        paragraph(v, &id, text);
    }
    id
}

fn tags(v: &Vault, id: &str, values: &[&str]) {
    v.set_property(
        id.into(),
        "tags".into(),
        PropValue::MultiSelect(values.iter().map(|s| (*s).into()).collect()),
    )
    .unwrap();
}

/// The prefix query the title tier builds for typed words: `"word"*` each.
fn typed_match(input: &str) -> String {
    input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The original per-row snippet over the body column, which the batched
/// title query must reproduce exactly for each hit.
fn legacy_snippet(index: &Connection, match_query: &str, fts_rowid: i64) -> Option<String> {
    index
        .query_row(
            "SELECT snippet(fts, 1, '', '', '…', 12) FROM fts WHERE fts MATCH ?1 AND fts.rowid = ?2",
            params![match_query, fts_rowid],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
}

fn page_fts_rowid(index: &Connection, id: &str) -> i64 {
    index
        .query_row("SELECT rowid FROM pages WHERE id = ?1", [id], |r| r.get(0))
        .unwrap()
}

/// External rows share the FTS table under negative rowids.
fn external_fts_rowid(index: &Connection, source: &str, id: &str) -> i64 {
    -index
        .query_row(
            "SELECT rowid FROM search_external WHERE source = ?1 AND id = ?2",
            params![source, id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
}

/// `SELECT id ... ORDER BY <order> LIMIT OFFSET` over a fixed membership list.
fn ordered_ids(
    index: &Connection,
    members: &[String],
    order: &str,
    limit: u32,
    offset: u32,
) -> Vec<String> {
    let json = format!(
        "[{}]",
        members
            .iter()
            .map(|id| format!("\"{id}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let sql = format!(
        "SELECT p.id FROM search_rows p
         WHERE p.in_trash = 0 AND p.id IN (SELECT value FROM json_each(?1))
         ORDER BY {order} LIMIT ?2 OFFSET ?3"
    );
    let mut st = index.prepare(&sql).unwrap();
    st.query_map(params![json, limit, offset], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Title-hit snippets
// ---------------------------------------------------------------------------

fn snippet_corpus(v: &Vault) {
    // Title tier. The uncorrected title tier needs each query word to start a
    // title word (fuzzy.rs `word_starts`), so every title-tier fixture carries
    // a literal "pan..." word. Repeated "sauce" and "pantry" occurrences, with
    // the pantry mention far from the first sauce, so the snippet window is a
    // real choice.
    page(
        v,
        "Tomato sauce pantry",
        "Simmer the tomatoes, then tomato sauce again. Tomato sauce needs basil, \
         sauce and pantry staples. Keep the pantry sauce jar near the pantry door; \
         sauce pantry notes repeat sauce.",
    );
    page(
        v,
        "Tomato sauce pantry ideas",
        "Weekly list: flour, eggs, milk, rice, beans, lentils, oats, honey, salt, \
         pepper, cumin, thyme, then at the very end pantry shelves hold tomato sauce jars.",
    );
    // Body tier: no title match, body match only.
    page(v, "Garden plan", "tomato sauce in the pantry");
    // Matches "tomato" in the title but not "sauce", so it is not a hit.
    page(v, "Tomato planning", "calendar for the spring");
    v.flush().unwrap();
}

#[test]
fn title_snippets_equal_legacy_snippets_for_a_long_multi_term_prefix() {
    let t = TempVault::new();
    let v = t.open();
    snippet_corpus(&v);
    let index = t.index();
    let query = "tomato sauce pan";
    let match_query = typed_match(query);

    for hits in [
        v.search(query.into(), 10).unwrap(),
        v.search_full(query.into(), 10).unwrap(),
        v.search_pages(query.into(), 10).unwrap(),
        v.search_adaptive(query.into(), 10).unwrap().hits,
    ] {
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles.len(), 3, "{titles:?}");
        // Title tier first, body-only tier last.
        assert_eq!(titles[2], "Garden plan");
        assert!(titles[..2].contains(&"Tomato sauce pantry"), "{titles:?}");
        assert!(
            titles[..2].contains(&"Tomato sauce pantry ideas"),
            "{titles:?}"
        );
        assert!(hits.iter().all(|h| !h.fuzzy));

        for hit in &hits {
            let expected = legacy_snippet(&index, &match_query, page_fts_rowid(&index, &hit.id))
                .expect("every hit matches the FTS query");
            assert_eq!(hit.snippet, expected, "snippet drifted for {:?}", hit.title);
            assert!(!hit.snippet.is_empty());
        }
    }
}

#[test]
fn title_snippet_survives_a_limit_that_cuts_the_title_tier() {
    let t = TempVault::new();
    let v = t.open();
    snippet_corpus(&v);
    let index = t.index();
    let match_query = typed_match("tomato sauce pan");

    let full = v.search("tomato sauce pan".into(), 10).unwrap();
    let first = v.search("tomato sauce pan".into(), 1).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].id, full[0].id);
    assert_eq!(
        first[0].snippet,
        legacy_snippet(&index, &match_query, page_fts_rowid(&index, &first[0].id)).unwrap()
    );
}

#[test]
fn title_only_hit_previews_the_body_opening_like_the_legacy_query() {
    let t = TempVault::new();
    let v = t.open();
    let spring = page(&v, "Tomato planning", "calendar for the spring");
    // Longer than the 12-token window, so the preview is cut.
    let long = page(
        &v,
        "Tomato harvest log",
        "one two three four five six seven eight nine ten eleven twelve thirteen fourteen",
    );
    v.flush().unwrap();
    let index = t.index();

    let hits = v.search("tomato plan".into(), 10).unwrap();
    let planning = hits.iter().find(|h| h.id == spring).unwrap();
    assert_eq!(planning.snippet, "calendar for the spring");
    assert_eq!(
        planning.snippet,
        legacy_snippet(
            &index,
            &typed_match("tomato plan"),
            page_fts_rowid(&index, &spring)
        )
        .unwrap()
    );

    let hits = v.search("tomato harv".into(), 10).unwrap();
    let harvest = hits.iter().find(|h| h.id == long).unwrap();
    assert_eq!(
        harvest.snippet,
        legacy_snippet(
            &index,
            &typed_match("tomato harv"),
            page_fts_rowid(&index, &long)
        )
        .unwrap()
    );
}

#[test]
fn unicode_title_snippets_keep_original_text_and_legacy_offsets() {
    let t = TempVault::new();
    let v = t.open();
    let id = page(
        &v,
        "Crème brûlée menu",
        "Our crème brûlée uses café cream and vanilla pods; brûlée is served warm.",
    );
    v.flush().unwrap();
    let index = t.index();

    // Typed without diacritics; the index folds them.
    let hits = v.search("creme brul".into(), 10).unwrap();
    let hit = hits.iter().find(|h| h.id == id).expect("title hit");
    assert_eq!(
        hit.snippet,
        legacy_snippet(
            &index,
            &typed_match("creme brul"),
            page_fts_rowid(&index, &id)
        )
        .unwrap()
    );
    assert!(hit.snippet.contains("brûlée"), "{:?}", hit.snippet);
}

#[test]
fn external_title_snippets_use_negative_rowid_legacy_snippets() {
    let t = TempVault::new();
    let v = t.open();
    snippet_corpus(&v);
    v.upsert_search_row(
        "agent".into(),
        ExternalSearchRow {
            id: "chat-7".into(),
            // Title tier needs a literal "pan..." title word, as above.
            title: "Tomato sauce pantry chat".into(),
            body: "We compared tomato sauce and the pantry shelf, then pantry sauce again".into(),
            page_type: "chat".into(),
            target: None,
            updated_ms: BASE,
        },
    )
    .unwrap();
    v.flush().unwrap();
    let index = t.index();
    let query = "tomato sauce pan";
    let match_query = typed_match(query);

    let hits = v.search(query.into(), 20).unwrap();
    let chat = hits
        .iter()
        .find(|h| h.source == "agent" && h.id == "chat-7")
        .expect("external title hit");
    assert_eq!(chat.page_type, "chat");
    // Title tier: the external title hit precedes the body-only page.
    let chat_at = hits.iter().position(|h| h.id == "chat-7").unwrap();
    let garden_at = hits.iter().position(|h| h.title == "Garden plan").unwrap();
    assert!(chat_at < garden_at, "external title hit fell to body tier");
    assert_eq!(
        chat.snippet,
        legacy_snippet(
            &index,
            &match_query,
            external_fts_rowid(&index, "agent", "chat-7")
        )
        .expect("external row matches")
    );

    // Page-only search keeps the vault snippet and drops the external row.
    let pages = v.search_pages(query.into(), 20).unwrap();
    assert!(pages.iter().all(|h| h.source == "vault"));
    assert!(pages.iter().all(|h| h.id != "chat-7"));
    for hit in &pages {
        assert_eq!(
            hit.snippet,
            legacy_snippet(&index, &match_query, page_fts_rowid(&index, &hit.id)).unwrap()
        );
    }
}

// ---------------------------------------------------------------------------
// Small-candidate filter-only sorting
// ---------------------------------------------------------------------------

const DEFAULT_ORDER: &str = "p.updated_ms DESC, p.id";
const CREATED_ORDER: &str = "p.created_ms DESC, p.updated_ms DESC, p.id";
const TITLE_ORDER: &str = "p.title_fold ASC, p.updated_ms DESC, p.id";

/// Forty pages with tags chosen so that every filter below has a different
/// membership, `updated_ms` ties in groups of four, and titles out of
/// creation order. Returns the page ids by fixture index.
fn filter_corpus(v: &Vault, t: &TempVault) -> Vec<PageId> {
    let mut ids = Vec::new();
    for i in 0..40usize {
        let title = format!("Note {:02}", (i * 7) % 40);
        let id = page(v, &title, "");
        let mut values = Vec::new();
        if i % 3 == 0 {
            values.push("work");
        }
        if i % 6 == 0 {
            values.push("work/meetings");
        }
        if i % 5 == 0 {
            values.push("home");
        }
        if !values.is_empty() {
            tags(v, &id, &values);
        }
        ids.push(id);
    }
    v.flush().unwrap();
    for (i, id) in ids.iter().enumerate() {
        let updated = BASE + (i as i64 / 4) * DAY;
        let created = BASE - i as i64 * DAY / 2;
        t.set_times(id, created, updated);
    }
    ids
}

#[test]
fn small_candidate_filter_order_matches_direct_sql_for_every_sort_and_window() {
    let t = TempVault::new();
    let v = t.open();
    let ids = filter_corpus(&v, &t);
    let index = t.index();

    // Membership comes from the fixture, not from the search code under test.
    let with = |pred: fn(usize) -> bool| -> Vec<String> {
        (0..ids.len())
            .filter(|&i| pred(i))
            .map(|i| ids[i].clone())
            .collect()
    };
    let work = with(|i| i % 3 == 0);
    let meetings = with(|i| i % 6 == 0);
    let home = with(|i| i % 5 == 0);
    let home_not_work = with(|i| i % 5 == 0 && i % 3 != 0);
    let work_and_home = with(|i| i % 3 == 0 && i % 5 == 0);

    let cases: Vec<(&str, Vec<String>, &str)> = vec![
        ("tag:work", work.clone(), DEFAULT_ORDER),
        ("tag:work sort:updated", work.clone(), DEFAULT_ORDER),
        ("tag:work sort:created", work.clone(), CREATED_ORDER),
        ("tag:work sort:title", work.clone(), TITLE_ORDER),
        ("tag:Work", work.clone(), DEFAULT_ORDER),
        ("tag:work/meetings", meetings, DEFAULT_ORDER),
        ("tag:home -tag:work", home_not_work, DEFAULT_ORDER),
        ("tag:work tag:home", work_and_home, DEFAULT_ORDER),
        ("tag:home sort:title", home, TITLE_ORDER),
    ];
    // Windows cross the page boundaries of tie groups (four pages share a
    // timestamp) and run past the end of the membership list.
    let windows = [(10, 0), (10, 10), (7, 3), (50, 0), (5, 30), (10, 100)];

    for (query, members, order) in &cases {
        for (limit, offset) in windows {
            let parsed = v.parse_search(query.to_string()).unwrap();
            let result = v.search_spec(parsed.spec, limit, offset).unwrap();
            let got: Vec<String> = result.hits.iter().map(|h| h.id.clone()).collect();
            let expected = ordered_ids(&index, members, order, limit, offset);
            assert_eq!(got, expected, "{query} limit {limit} offset {offset}");
            assert_eq!(
                result.total_bounded as usize,
                members.len().min(201),
                "{query} total"
            );
            assert!(!result.narrowed, "{query} narrowed");
            assert!(
                result.hits.iter().all(|h| h.snippet.is_empty()),
                "{query} filter-only hits carry no snippet"
            );
        }

        // Public surface without offset: the adaptive entry point must agree.
        if !query.contains("sort:") {
            let public: Vec<String> = v
                .search(query.to_string(), 10)
                .unwrap()
                .into_iter()
                .map(|h| h.id)
                .collect();
            assert_eq!(
                public,
                ordered_ids(&index, members, order, 10, 0),
                "{query} search()"
            );
        }
    }
}

#[test]
fn small_candidate_hits_report_the_matched_tag_values() {
    let t = TempVault::new();
    let v = t.open();
    let ids = filter_corpus(&v, &t);

    // Index 0 carries "work", "work/meetings" and "home".
    let hits = v.search("tag:work".into(), 50).unwrap();
    let first = hits.iter().find(|h| h.id == ids[0]).expect("tagged page");
    let mut values: Vec<&str> = first
        .matched_filters
        .iter()
        .filter(|m| m.key == "tag")
        .map(|m| m.value.as_str())
        .collect();
    values.sort_unstable();
    assert_eq!(values, ["work", "work/meetings"]);

    // "home" was not asked for, so it must not be reported.
    assert!(first.matched_filters.iter().all(|m| m.value != "home"));

    // Index 3 carries only "work": one matched value.
    let only_work = hits.iter().find(|h| h.id == ids[3]).expect("work page");
    assert_eq!(
        only_work
            .matched_filters
            .iter()
            .map(|m| (m.key.as_str(), m.value.as_str()))
            .collect::<Vec<_>>(),
        [("tag", "work")]
    );
}

// ---------------------------------------------------------------------------
// Schema option renames
// ---------------------------------------------------------------------------

/// Hit ids for a query, in engine order.
fn hit_ids(v: &Vault, query: &str) -> Vec<String> {
    let parsed = v.parse_search(query.into()).unwrap();
    v.search_spec(parsed.spec, 50, 0)
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.id)
        .collect()
}

/// The `tag` labels a hit reports as matched, sorted.
fn tag_annotations(v: &Vault, query: &str, row: &str) -> Vec<String> {
    let parsed = v.parse_search(query.into()).unwrap();
    let hits = v.search_spec(parsed.spec, 50, 0).unwrap().hits;
    let hit = hits.iter().find(|h| h.id == row).expect("row is a hit");
    let mut labels: Vec<String> = hit
        .matched_filters
        .iter()
        .filter(|m| m.key == "tag")
        .map(|m| m.value.clone())
        .collect();
    labels.sort();
    labels
}

/// A collection schema whose one tag option has the given name. The option
/// ids are opaque, so a raw stored value can never equal a query name.
fn tag_schema(name: &str) -> Vec<PropDef> {
    vec![PropDef {
        key: "opaque-tag-key".into(),
        name: "Tags".into(),
        kind: PropKind::MultiSelect,
        options: vec![
            SelectOption {
                id: "opaque-focus-id".into(),
                name: name.into(),
                color: String::new(),
            },
            SelectOption {
                id: "opaque-school-id".into(),
                name: "École".into(),
                color: String::new(),
            },
        ],
        config: String::new(),
    }]
}

#[test]
fn schema_option_renames_reach_tag_filters_and_annotations_after_cached_searches() {
    let t = TempVault::new();
    let v = t.open();
    let collection = page(&v, "Renamed tags", "");
    v.set_schema(collection.clone(), tag_schema("Work/Meetings"))
        .unwrap();
    let row = v
        .create_page(Some(collection.clone()), "Renamed row".into())
        .unwrap();
    v.set_property(
        row.clone(),
        "opaque-tag-key".into(),
        PropValue::MultiSelect(vec!["opaque-focus-id".into(), "opaque-school-id".into()]),
    )
    .unwrap();
    v.flush().unwrap();

    // Stage 0: warm the candidate cache under the original name.
    for _ in 0..2 {
        assert_eq!(hit_ids(&v, "tag:work/meetings"), vec![row.clone()]);
    }
    assert_eq!(
        tag_annotations(&v, "tag:work/meetings", &row),
        ["Work/Meetings"]
    );

    // Stage 1: a normal schema edit renames the option. The warm stale query
    // must stop matching, and the new name must match and be reported.
    v.set_schema(collection.clone(), tag_schema("Focus/Deep"))
        .unwrap();
    v.flush().unwrap();
    assert_eq!(hit_ids(&v, "tag:work/meetings"), Vec::<String>::new());
    for _ in 0..2 {
        assert_eq!(hit_ids(&v, "tag:focus/deep"), vec![row.clone()]);
    }
    assert_eq!(hit_ids(&v, "tag:focus"), vec![row.clone()]);
    assert_eq!(tag_annotations(&v, "tag:focus/deep", &row), ["Focus/Deep"]);

    // Stage 2: a separate connection (the writer-style path) rewrites the
    // schema row. The warm "focus/deep" entry was cached before this write
    // and must be invalidated by the data_version change.
    let external = rusqlite::Connection::open(t.0.join(".vault/index.sqlite")).unwrap();
    external
        .execute(
            "UPDATE pages SET schema = replace(schema, 'Focus/Deep', 'Quiet') WHERE id = ?1",
            [&collection],
        )
        .unwrap();
    // UPDATE reports the row even when replace() changes nothing, so check
    // the stored JSON itself.
    let stored: String = external
        .query_row(
            "SELECT schema FROM pages WHERE id = ?1",
            [&collection],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        stored.contains("Quiet") && !stored.contains("Focus/Deep"),
        "{stored}"
    );
    drop(external);
    assert_eq!(hit_ids(&v, "tag:focus/deep"), Vec::<String>::new());
    assert_eq!(hit_ids(&v, "tag:focus"), Vec::<String>::new());
    for _ in 0..2 {
        assert_eq!(hit_ids(&v, "tag:quiet"), vec![row.clone()]);
    }
    // Only the renamed option is reported; the school label is not.
    assert_eq!(tag_annotations(&v, "tag:quiet", &row), ["Quiet"]);
}

#[test]
fn cached_typo_scores_follow_foreground_renames_and_writer_commits() {
    let t = TempVault::new();
    let v = t.open();
    let id = page(&v, "Garden plan", "");
    tags(&v, &id, &["work"]);
    v.flush().unwrap();
    for _ in 0..2 {
        let hits = v.search("gardne tag:work".into(), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);
        assert!(hits[0].fuzzy);
    }
    v.rename_page(id.clone(), "Budget plan".into()).unwrap();
    assert!(v.search("gardne tag:work".into(), 10).unwrap().is_empty());
    v.rename_page(id.clone(), "Garden plan".into()).unwrap();
    v.flush().unwrap();
    assert_eq!(v.search("gardne tag:work".into(), 10).unwrap()[0].id, id);
    let writer = t.index();
    writer
        .execute(
            "UPDATE pages SET title = 'Budget plan', title_fold = 'budget plan' WHERE id = ?1",
            [&id],
        )
        .unwrap();
    // The FTS row must change with the title, as it does on a real write.
    writer.execute("UPDATE fts SET title = 'Budget plan' WHERE rowid = (SELECT rowid FROM pages WHERE id = ?1)", [&id]).unwrap();
    drop(writer);
    assert!(v.search("gardne tag:work".into(), 10).unwrap().is_empty());
}
