//! Typo-tolerant search: title subsequences, body word correction, the
//! exact-first rule, short words, and refresh after writes.
use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> TempVault {
        TempVault(std::env::temp_dir().join(format!("vault_typo_{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn body(page: &Vault, id: &str, text: &str) {
    page.apply(
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
    body(v, &id, text);
    id
}

/// A ten-plus page vault with the typo targets and filler pages.
fn corpus(v: &Vault) -> Vec<(&'static str, PageId)> {
    let mut ids = vec![
        (
            "Garden plan",
            page(v, "Garden plan", "spring beds and tomatoes"),
        ),
        (
            "Tomato sauce",
            page(v, "Tomato sauce", "simmer for an hour"),
        ),
        (
            "Weekly review",
            page(v, "Weekly review", "inbox zero and goals"),
        ),
        (
            "Coffee morning",
            page(v, "Coffee morning", "beans from the roastery"),
        ),
        (
            "Project ideas",
            page(v, "Project ideas", "side projects list"),
        ),
        (
            "Dinner ideas",
            page(v, "Dinner ideas", "recipes for weeknight dinner"),
        ),
        (
            "Reading list",
            page(v, "Reading list", "books to finish this month"),
        ),
        (
            "Gardening tips",
            page(v, "Gardening tips", "mulch and water"),
        ),
    ];
    for n in 0..24 {
        let title = format!("Note {n:02}");
        ids.push((
            "filler",
            page(v, &title, &format!("filler text number {n}")),
        ));
    }
    v.flush().unwrap();
    ids
}

fn top(v: &Vault, query: &str, n: usize) -> Vec<SearchHit> {
    let mut hits = v.search(query.into(), 10).unwrap();
    hits.truncate(n);
    hits
}

#[test]
fn ten_plus_typos_find_the_intended_page_in_top_three() {
    let t = TempVault::new();
    let v = t.open();
    let ids = corpus(&v);
    let id_of = |title: &str| ids.iter().find(|(t, _)| *t == title).unwrap().1.clone();
    let cases = [
        // "tomatoe" is a prefix of "tomatoes", so it exists and is not
        // corrected; "tomatoo" is missing from the vocabulary.
        ("tomatoo", "Tomato sauce"),
        ("gardne", "Garden plan"),
        ("weekyl review", "Weekly review"),
        ("cofee", "Coffee morning"),
        ("projcet", "Project ideas"),
        ("recipies", "Dinner ideas"),
        ("garedn", "Garden plan"),
        ("reading lsit", "Reading list"),
        ("bookz", "Reading list"),
        ("garden gadr", "Garden plan"),
        ("grdn pln", "Garden plan"),
    ];
    assert!(cases.len() >= 10);
    for (query, title) in cases {
        let hits = top(&v, query, 3);
        let want = id_of(title);
        assert!(
            hits.iter().any(|h| h.id == want),
            "{query:?} should find {title:?} in the top three, got {:?}",
            hits.iter().map(|h| h.title.as_str()).collect::<Vec<_>>()
        );
        let hit = hits.iter().find(|h| h.id == want).unwrap();
        assert!(hit.fuzzy, "{query:?} hit should be fuzzy");
        assert!(
            hit.corrected.is_some(),
            "{query:?} hit should carry a correction"
        );
    }
}

#[test]
fn prefix_typo_gadr_matches_garden_titles() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    let hits = top(&v, "gadr", 3);
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert!(
        titles.contains(&"Garden plan") && titles.contains(&"Gardening tips"),
        "gadr should find the garden titles: {titles:?}"
    );
    assert!(hits.iter().all(|h| h.fuzzy));
}

#[test]
fn grdn_pln_finds_garden_plan_first() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    let hits = v.search("grdn pln".into(), 10).unwrap();
    assert_eq!(hits[0].title, "Garden plan");
    assert!(hits[0].fuzzy);
    assert_eq!(hits[0].corrected.as_deref(), Some("garden plan"));
    // The agent path follows the same rule.
    let full = v.search_full("grdn pln".into(), 10).unwrap();
    assert_eq!(full[0].title, "Garden plan");
    assert!(full[0].fuzzy);
}

#[test]
fn exact_and_prefix_hits_stay_above_every_corrected_hit() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // "tomato" is exact in "Tomato sauce" and in the body of "Garden plan";
    // "Gardening tips" is not reachable by exact prefix for "tomato".
    let hits = v.search("tomato".into(), 10).unwrap();
    assert_eq!(hits[0].title, "Tomato sauce");
    assert!(!hits[0].fuzzy);
    let first_fuzzy = hits.iter().position(|h| h.fuzzy).unwrap_or(hits.len());
    assert!(hits[..first_fuzzy].iter().all(|h| !h.fuzzy));
    assert!(hits[first_fuzzy..].iter().all(|h| h.fuzzy));
    let full = v.search_full("tomato".into(), 10).unwrap();
    let first_fuzzy = full.iter().position(|h| h.fuzzy).unwrap_or(full.len());
    assert!(full[..first_fuzzy].iter().all(|h| !h.fuzzy));
}

#[test]
fn correctly_typed_populated_queries_run_no_correction() {
    let t = TempVault::new();
    let v = t.open();
    for n in 0..12 {
        v.create_page(None, format!("Garden journal {n}")).unwrap();
    }
    v.flush().unwrap();
    // "garden" exists in the vocabulary, so no word is missing: no fuzzy hits.
    let hits = v.search("garden".into(), 50).unwrap();
    assert_eq!(hits.len(), 12);
    assert!(hits.iter().all(|h| !h.fuzzy && h.corrected.is_none()));
}

#[test]
fn short_words_are_never_corrected_but_may_skip_letters_in_titles() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // "tez" is one substitution from "tea" in the body below: a three-letter
    // word takes no substitutions, so nothing matches.
    page(&v, "Snacks", "tea cup biscuits");
    page(&v, "Tea time", "after lunch");
    v.flush().unwrap();
    assert!(v.search("tez".into(), 10).unwrap().is_empty());
    // Three letters may skip title letters: t, m, a in "Tomato sauce".
    let hits = v.search("tma".into(), 10).unwrap();
    assert!(hits.iter().any(|h| h.title == "Tomato sauce" && h.fuzzy));
}

#[test]
fn body_only_correction_uses_the_indexed_word() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // "gardne" is missing from the vocabulary: no term starts with it.
    let a = page(&v, "Alpha", "garden party on sunday");
    v.flush().unwrap();
    for hits in [
        v.search("gardne".into(), 10).unwrap(),
        v.search_full("gardne".into(), 10).unwrap(),
    ] {
        // The body word is the indexed word itself, never a truncated prefix.
        let alpha = hits.iter().find(|h| h.id == a).unwrap();
        assert!(alpha.fuzzy);
        assert_eq!(alpha.corrected.as_deref(), Some("garden"));
    }
}

#[test]
fn existing_prefix_word_is_not_expanded_by_a_misspelled_neighbour() {
    let t = TempVault::new();
    let v = t.open();
    let shed = page(&v, "Shed", "garden tools");
    // "gardner" is one edit from "garden", but "garden" exists in the
    // vocabulary, so it must keep its typed term and never expand to it.
    let bench = page(&v, "Bench", "gardner tools");
    v.flush().unwrap();
    for hits in [
        v.search("garden tolls".into(), 10).unwrap(),
        v.search_full("garden tolls".into(), 10).unwrap(),
    ] {
        assert!(hits.iter().all(|h| h.id != bench), "{hits:?}");
        let hit = hits.iter().find(|h| h.id == shed).unwrap();
        assert!(hit.fuzzy);
    }
}

#[test]
fn missing_word_is_corrected_beside_an_existing_word() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // "garden" exists; "gadr" is missing and must still be corrected.
    let patio = page(&v, "Patio", "garden lights");
    v.flush().unwrap();
    for hits in [
        v.search("garden gadr".into(), 10).unwrap(),
        v.search_full("garden gadr".into(), 10).unwrap(),
    ] {
        let hit = hits.iter().find(|h| h.id == patio).unwrap();
        assert!(hit.fuzzy);
        assert_eq!(hit.corrected.as_deref(), Some("garden garden"));
    }
}

#[test]
fn selective_queries_with_existing_words_run_no_fuzzy_work() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    page(&v, "Lantern notes", "a single unique page");
    v.flush().unwrap();
    // Few results, every word present: no title scoring, no body correction.
    for query in ["garden plan", "lantern", "unique page"] {
        for hits in [
            v.search(query.into(), 10).unwrap(),
            v.search_full(query.into(), 10).unwrap(),
        ] {
            assert!(!hits.is_empty(), "{query:?}");
            assert!(
                hits.iter().all(|h| !h.fuzzy && h.corrected.is_none()),
                "{query:?}"
            );
        }
    }
}

#[test]
fn body_candidates_at_different_edit_counts_rank_by_edits() {
    let t = TempVault::new();
    let v = t.open();
    // "recipies" is one edit from "recipes" and two from "recipe". Both pages
    // hold a single candidate, so each hit's correction is its own word.
    let near = page(&v, "Soup one", "recipes for soup");
    let far = page(&v, "Soup two", "recipe soup");
    v.flush().unwrap();
    let hits = v.search("recipies soup".into(), 10).unwrap();
    let near_hit = hits.iter().find(|h| h.id == near).unwrap();
    let far_hit = hits.iter().find(|h| h.id == far).unwrap();
    assert!(near_hit.fuzzy && far_hit.fuzzy);
    assert_eq!(near_hit.corrected.as_deref(), Some("recipes soup"));
    assert_eq!(far_hit.corrected.as_deref(), Some("recipe soup"));
    let position = |id: &PageId| hits.iter().position(|h| &h.id == id).unwrap();
    assert!(position(&near) < position(&far), "fewer edits rank first");
}

#[test]
fn title_substring_and_word_start_hits_are_not_fuzzy() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // "ekly rev" is a contiguous substring of "Weekly review"; "week rev" is
    // a word-start match. Neither is a correction.
    for query in ["ekly rev", "week rev"] {
        let hits = v.search(query.into(), 10).unwrap();
        let hit = hits
            .iter()
            .find(|h| h.title == "Weekly review")
            .unwrap_or_else(|| panic!("{query:?} should find Weekly review"));
        assert!(!hit.fuzzy, "{query:?}");
        assert!(hit.corrected.is_none(), "{query:?}");
    }
}

#[test]
fn refreshes_after_create_rename_trash_and_restore() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // Immediately after create, with no flush, the title is found.
    let quarterly = v.create_page(None, "Quarterly budget".into()).unwrap();
    assert_eq!(v.search("quartely".into(), 10).unwrap()[0].id, quarterly);
    // Rename: the new title matches, the old one no longer does.
    v.rename_page(quarterly.clone(), "Annual plan".into())
        .unwrap();
    let hits = v.search("anual pln".into(), 10).unwrap();
    assert_eq!(hits[0].id, quarterly);
    assert!(v.search("quartely".into(), 10).unwrap().is_empty());
    // Trash removes it from corrections; restore brings it back.
    v.delete_page(quarterly.clone()).unwrap();
    assert!(v.search("anual pln".into(), 10).unwrap().is_empty());
    v.restore_page(quarterly.clone()).unwrap();
    assert_eq!(v.search("anual pln".into(), 10).unwrap()[0].id, quarterly);
}

#[test]
fn body_vocabulary_refreshes_after_flush() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // A body word written after open is correctable once flushed.
    let scratch = page(&v, "Scratch", "marzipan cake for the fete");
    v.flush().unwrap();
    let hits = v.search("marzipn".into(), 10).unwrap();
    assert_eq!(hits[0].id, scratch);
    assert_eq!(hits[0].corrected.as_deref(), Some("marzipan"));
    // A word that never existed stays uncorrected.
    assert!(v.search("zxqvbnm".into(), 10).unwrap().is_empty());
}

#[test]
fn stats_reflect_the_built_cache() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);
    // The cache is built at open and refreshed by a fuzzy query after writes.
    assert!(!v.search("tomatoo".into(), 10).unwrap().is_empty());
    let before = v.search_index_stats();
    assert!(before.words > 0);
    assert!(before.deletion_keys > before.words);
    assert!(before.estimated_bytes > 0);
    assert!(before.build_ms >= 0.0);
    // A query that triggers a rebuild after a new word appears.
    page(&v, "Vocabulary", "quixotic lantern");
    v.flush().unwrap();
    assert_eq!(
        v.search("quixotc".into(), 10).unwrap()[0].title,
        "Vocabulary"
    );
    let after = v.search_index_stats();
    assert!(after.words > before.words);
    assert!(after.deletion_keys > before.deletion_keys);
}
