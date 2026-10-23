//! External search sources: rows held by a source (agent chats are the first)
//! are searched, ranked, corrected and filtered with pages, but are never
//! pages. They survive reindexing and reopening, and each source owns its ids.
use std::collections::BTreeSet;
use std::path::PathBuf;

use vault_core::*;

const CHAT: &str = "chat";
const NOTES: &str = "notes";
const TARGET: &str = r#"{"sessionId":"s1","firstMessageId":"m1","lastMessageId":"m9","level":"chapter","chapterTitle":"Budget","sectionTitle":null}"#;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_external_search_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

fn row(id: &str, title: &str, body: &str, page_type: &str, updated_ms: i64) -> ExternalSearchRow {
    ExternalSearchRow {
        id: id.into(),
        title: title.into(),
        body: body.into(),
        page_type: page_type.into(),
        target: None,
        updated_ms,
    }
}

fn search(v: &Vault, query: &str) -> Vec<SearchHit> {
    let parsed = v.parse_search(query.into()).unwrap();
    v.search_spec(parsed.spec, 100, 0).unwrap().hits
}

/// Hits as `(source, id)` pairs, which is what identifies a row.
fn keys(hits: &[SearchHit]) -> BTreeSet<(String, String)> {
    hits.iter()
        .map(|h| (h.source.clone(), h.id.clone()))
        .collect()
}

fn key(source: &str, id: &str) -> (String, String) {
    (source.into(), id.into())
}

fn found(v: &Vault, query: &str) -> BTreeSet<(String, String)> {
    keys(&search(v, query))
}

#[test]
fn external_rows_carry_source_type_and_opaque_target() {
    let t = TempVault::new();
    let v = t.open();
    let doc = page(&v, "Budget", "quarterly numbers");
    let mut chat = row(
        "c1",
        "Chats / Budget / Chapter 1",
        "summary of the budget talk",
        CHAT,
        1,
    );
    chat.target = Some(TARGET.into());
    v.upsert_search_row(CHAT.into(), chat).unwrap();

    let hits = search(&v, "budget");
    let ext = hits
        .iter()
        .find(|h| h.source == CHAT)
        .expect("chat row found");
    assert_eq!(ext.id, "c1", "the id is the source's local id");
    assert_eq!(ext.title, "Chats / Budget / Chapter 1");
    assert_eq!(ext.page_type, CHAT);
    assert_eq!(
        ext.target.as_deref(),
        Some(TARGET),
        "target comes back unchanged"
    );

    let vault_hit = hits.iter().find(|h| h.id == doc).expect("page found");
    assert_eq!(vault_hit.source, "vault");
    assert_eq!(vault_hit.page_type, "document");
    assert_eq!(vault_hit.target, None);
}

#[test]
fn external_rows_survive_reindex_and_reopen() {
    let t = TempVault::new();
    {
        let v = t.open();
        page(&v, "Budget", "");
        v.upsert_search_row(CHAT.into(), row("c1", "Budget chat", "", CHAT, 1))
            .unwrap();
        v.reindex().unwrap();
        assert!(found(&v, "budget").contains(&key(CHAT, "c1")));
    }
    let v = t.open();
    assert!(
        found(&v, "budget").contains(&key(CHAT, "c1")),
        "after reopen"
    );
    v.reindex().unwrap();
    assert!(
        found(&v, "budget").contains(&key(CHAT, "c1")),
        "after a second reindex"
    );
}

#[test]
fn external_title_ranks_with_page_titles_ahead_of_body_matches() {
    let t = TempVault::new();
    let v = t.open();
    page(&v, "Field notes", "a zebra crossed the road");
    v.upsert_search_row(CHAT.into(), row("c1", "Zebra", "unrelated text", CHAT, 1))
        .unwrap();
    // Body text reaches the index when the writer flushes.
    v.flush().unwrap();

    let hits = search(&v, "zebra");
    assert_eq!(
        hits.len(),
        2,
        "both the body match and the title match: {hits:?}"
    );
    assert_eq!(
        key(&hits[0].source, &hits[0].id),
        key(CHAT, "c1"),
        "title match first"
    );
    assert_eq!(hits[1].source, "vault");
}

#[test]
fn external_text_is_corrected_like_page_text() {
    let t = TempVault::new();
    let v = t.open();
    page(&v, "Garden plan", "");
    v.upsert_search_row(
        CHAT.into(),
        row(
            "c1",
            "Quarterly review",
            "migration checklist for the warehouse",
            CHAT,
            1,
        ),
    )
    .unwrap();

    let title_hits = search(&v, "quartrly reviw");
    let title = title_hits
        .iter()
        .find(|h| h.source == CHAT)
        .expect("title typo finds the external row");
    assert!(title.fuzzy, "title typo hit is fuzzy");
    assert!(title.corrected.is_some(), "title typo has a correction");

    let body_hits = search(&v, "migraton checklst");
    let body = body_hits
        .iter()
        .find(|h| h.source == CHAT)
        .expect("body typo finds the external row");
    assert!(body.fuzzy, "body typo hit is fuzzy");
}

#[test]
fn external_rows_update_and_delete_by_source_and_local_id() {
    let t = TempVault::new();
    let v = t.open();
    v.upsert_search_row(CHAT.into(), row("c1", "Old title", "alpha words", CHAT, 1))
        .unwrap();
    v.upsert_search_row(CHAT.into(), row("c1", "New title", "beta words", CHAT, 2))
        .unwrap();

    assert!(found(&v, "old").is_empty(), "replaced title is gone");
    assert!(found(&v, "alpha").is_empty(), "replaced body is gone");
    assert_eq!(found(&v, "new"), [key(CHAT, "c1")].into_iter().collect());
    assert_eq!(found(&v, "beta"), [key(CHAT, "c1")].into_iter().collect());

    v.delete_search_row(CHAT.into(), "c1".into()).unwrap();
    assert!(found(&v, "new").is_empty(), "deleted row is gone");
    v.delete_search_row(CHAT.into(), "c1".into()).unwrap();
    v.delete_search_row(CHAT.into(), "never-existed".into())
        .unwrap();
}

#[test]
fn clearing_a_source_removes_only_its_rows() {
    let t = TempVault::new();
    let v = t.open();
    page(&v, "Budget", "");
    v.upsert_search_row(CHAT.into(), row("c1", "Budget chat", "", CHAT, 1))
        .unwrap();
    v.upsert_search_row(NOTES.into(), row("n1", "Budget notes", "", "note", 1))
        .unwrap();

    v.clear_search_source(CHAT.into()).unwrap();
    let got = found(&v, "budget");
    assert!(!got.contains(&key(CHAT, "c1")), "cleared source is gone");
    assert!(got.contains(&key(NOTES, "n1")), "other source stays");
    assert!(
        got.contains(&key("vault", &found_page_id(&v))),
        "pages stay"
    );
    v.reindex().unwrap();
    assert!(
        found(&v, "budget").contains(&key(NOTES, "n1")),
        "reindex keeps other sources"
    );
}

fn found_page_id(v: &Vault) -> String {
    search(v, "budget")
        .into_iter()
        .find(|h| h.source == "vault")
        .expect("the vault page")
        .id
}

#[test]
fn external_rows_are_not_pages() {
    let t = TempVault::new();
    let v = t.open();
    let doc = page(&v, "Budget", "");
    v.upsert_search_row(CHAT.into(), row("c1", "Budget chat", "", CHAT, 1))
        .unwrap();
    let pages_before = v.page_count().unwrap();

    let listed: Vec<String> = v
        .list_pages(None, 0, 100)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(listed, vec![doc.clone()], "listings show only pages");
    assert!(v.load_page("c1".into()).is_err(), "not loadable as a page");
    assert!(v.rename_page("c1".into(), "Renamed".into()).is_err());
    assert!(v.delete_page("c1".into()).is_err());
    assert_eq!(
        v.page_count().unwrap(),
        pages_before,
        "page count unchanged"
    );
    assert!(
        found(&v, "budget").contains(&key(CHAT, "c1")),
        "still searchable"
    );
}

#[test]
fn an_external_id_equal_to_a_page_id_stays_a_separate_identity() {
    let t = TempVault::new();
    let v = t.open();
    let doc = page(&v, "Shared", "");
    v.upsert_search_row(CHAT.into(), row(&doc, "Shared chat", "", CHAT, 1))
        .unwrap();

    let got = found(&v, "shared");
    assert!(got.contains(&key("vault", &doc)), "the page: {got:?}");
    assert!(got.contains(&key(CHAT, &doc)), "the chat row: {got:?}");

    v.delete_search_row(CHAT.into(), doc.clone()).unwrap();
    assert!(v.load_page(doc.clone()).is_ok(), "the page is untouched");
    assert_eq!(
        found(&v, "shared"),
        [key("vault", &doc)].into_iter().collect()
    );
}

#[test]
fn type_filter_selects_pages_and_external_rows_by_type() {
    let t = TempVault::new();
    let v = t.open();
    let note = page(&v, "Budget note", "");
    let table = page(&v, "Budget table", "");
    v.make_collection(table.clone()).unwrap();
    v.upsert_search_row(CHAT.into(), row("c1", "Budget chat one", "", CHAT, 1))
        .unwrap();
    v.upsert_search_row(CHAT.into(), row("c2", "Budget chat two", "", CHAT, 2))
        .unwrap();
    v.upsert_search_row(NOTES.into(), row("n1", "Budget memo", "", "memo", 3))
        .unwrap();

    let chats = key(CHAT, "c1");
    let chat_two = key(CHAT, "c2");
    assert_eq!(
        found(&v, "type:chat"),
        [chats.clone(), chat_two.clone()].into_iter().collect()
    );
    assert_eq!(
        found(&v, "type:document"),
        [key("vault", &note)].into_iter().collect(),
        "a page without a stored type is a document"
    );
    assert_eq!(
        found(&v, "type:database"),
        [key("vault", &table)].into_iter().collect()
    );
    assert_eq!(
        found(&v, "type:CHAT"),
        [chats.clone(), chat_two.clone()].into_iter().collect(),
        "types are case-insensitive"
    );
    assert_eq!(
        found(&v, "type:chat,database").len(),
        3,
        "a comma means any of"
    );
    assert!(
        found(&v, "type:chat type:database").is_empty(),
        "repeated keys mean all of"
    );
    let not_chat = found(&v, "budget -type:chat");
    assert!(
        !not_chat.iter().any(|(s, _)| s == CHAT),
        "negation: {not_chat:?}"
    );
    assert!(not_chat.contains(&key(NOTES, "n1")));
    assert_eq!(
        found(&v, "type:chat chat two"),
        [chat_two].into_iter().collect(),
        "text and a type filter together"
    );
    assert_eq!(
        found(&v, "type:memo").len(),
        1,
        "any type string is filterable"
    );
}

#[test]
fn hard_filters_other_than_type_exclude_external_rows() {
    let t = TempVault::new();
    let v = t.open();
    let tagged = page(&v, "Budget tagged", "");
    v.set_property(
        tagged.clone(),
        "tags".into(),
        PropValue::MultiSelect(vec!["work".into()]),
    )
    .unwrap();
    page(&v, "Projects", "");
    v.upsert_search_row(CHAT.into(), row("c1", "Budget chat", "", CHAT, 1))
        .unwrap();

    assert_eq!(
        found(&v, "budget tag:work"),
        [key("vault", &tagged)].into_iter().collect(),
        "a tag filter never matches an external row"
    );
    assert!(found(&v, "budget tag:work type:chat").is_empty());
    assert!(
        found(&v, "in:\"Projects\" budget").is_empty(),
        "subtree filters never match an external row"
    );
    assert!(found(&v, "budget -tag:work").contains(&key(CHAT, "c1")));
}

#[test]
fn in_title_resolves_to_the_stored_page_id() {
    let t = TempVault::new();
    let v = t.open();
    let projects = page(&v, "Projects", "");
    let child = v
        .create_page(Some(projects.clone()), "Child note".into())
        .unwrap();
    // An external row with the same title must not take the page's place.
    v.upsert_search_row(
        CHAT.into(),
        row("projects", "Projects", "child note elsewhere", CHAT, 1),
    )
    .unwrap();

    let parsed = v.parse_search("in:\"Projects\"".into()).unwrap();
    let resolved = parsed.spec.operators[0].resolved_id.clone();
    assert_eq!(resolved, Some(projects.clone()), "resolves to the page id");

    let stored = parsed.spec;
    let before = keys(&v.search_spec(stored.clone(), 100, 0).unwrap().hits);
    let expected: BTreeSet<_> = [key("vault", &projects), key("vault", &child)]
        .into_iter()
        .collect();
    assert_eq!(before, expected, "the page subtree, no external row");

    v.rename_page(projects.clone(), "Renamed projects".into())
        .unwrap();
    let after = keys(&v.search_spec(stored, 100, 0).unwrap().hits);
    assert_eq!(after, expected, "the stored id survives the rename");
}
