//! External search identity: the search projection migrates into an index
//! created without it, broad external titles still fill results, malformed
//! or reserved sources and ids are refused, and the page-only search helper
//! never returns external rows, even when they outnumber the limit.
use std::collections::BTreeSet;
use std::path::PathBuf;

use vault_core::*;

const CHAT: &str = "chat";

struct TempVault(PathBuf);

impl TempVault {
    fn new(tag: &str) -> Self {
        Self(std::env::temp_dir().join(format!("vault_external_identity_{tag}_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }

    fn index(&self) -> PathBuf {
        self.0.join(".vault").join("index.sqlite")
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn page(v: &Vault, title: &str) -> PageId {
    v.create_page(None, title.into()).unwrap()
}

fn row(id: &str, title: &str, page_type: &str, updated_ms: i64) -> ExternalSearchRow {
    ExternalSearchRow {
        id: id.into(),
        title: title.into(),
        body: String::new(),
        page_type: page_type.into(),
        target: None,
        updated_ms,
    }
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

#[test]
fn migration_adds_the_search_projection_and_keeps_external_rows() {
    let t = TempVault::new("migration");
    let v = t.open();
    let doc = page(&v, "Budget plan");
    v.upsert_search_row(CHAT.into(), row("c1", "Budget chat", CHAT, 5))
        .unwrap();
    v.flush().unwrap();
    let before = keys(&v.search("budget".into(), 50).unwrap());
    assert_eq!(before, [key("vault", &doc), key(CHAT, "c1")].into_iter().collect());
    drop(v);

    // An index from before the projection: no table and no triggers.
    {
        let conn = rusqlite::Connection::open(t.index()).unwrap();
        conn.execute_batch(
            "DROP TRIGGER search_rows_insert;
             DROP TRIGGER search_rows_update;
             DROP TRIGGER search_rows_delete;
             DROP TRIGGER search_rows_external_insert;
             DROP TRIGGER search_rows_external_update;
             DROP TRIGGER search_rows_external_delete;
             DROP TABLE search_rows;",
        )
        .unwrap();
    }

    let v = t.open();
    assert_eq!(keys(&v.search("budget".into(), 50).unwrap()), before);

    // The migrated index is maintained again by page and external writes.
    let late = page(&v, "Budget late");
    v.upsert_search_row(CHAT.into(), row("c2", "Budget second", CHAT, 6))
        .unwrap();
    let after = keys(&v.search("budget".into(), 50).unwrap());
    assert!(after.contains(&key("vault", &late)), "{after:?}");
    assert!(after.contains(&key(CHAT, "c2")), "{after:?}");
    drop(v);

    // Reopening a migrated index changes nothing.
    let v = t.open();
    assert_eq!(keys(&v.search("budget".into(), 50).unwrap()), after);
}

#[test]
fn broad_external_titles_fill_the_limit_and_pages_still_match() {
    let t = TempVault::new("broad");
    let v = t.open();
    // Live external rows alone exceed the breadth threshold of a small corpus.
    for i in 0..250 {
        v.upsert_search_row(
            CHAT.into(),
            row(&format!("c{i}"), &format!("Budget log {i}"), CHAT, i),
        )
        .unwrap();
    }
    let plan = page(&v, "Budget plan");

    let broad = v.search("budget".into(), 10).unwrap();
    assert_eq!(broad.len(), 10, "the limit is filled by external titles");
    assert!(broad.iter().all(|h| h.title.to_lowercase().contains("budget")));

    let narrow = keys(&v.search("budget plan".into(), 10).unwrap());
    assert!(narrow.contains(&key("vault", &plan)), "{narrow:?}");
}

#[test]
fn reserved_and_malformed_identities_are_rejected_without_writes() {
    let t = TempVault::new("identity");
    let v = t.open();
    let sep = '\u{1f}';
    let sources = ["vault".to_string(), String::new(), format!("a{sep}b")];
    for source in sources {
        let upsert = v.upsert_search_row(source.clone(), row("c1", "Budget", CHAT, 1));
        assert!(matches!(upsert, Err(VaultError::Invalid(_))), "{source:?}: {upsert:?}");
        let delete = v.delete_search_row(source.clone(), "c1".into());
        assert!(matches!(delete, Err(VaultError::Invalid(_))), "{source:?}: {delete:?}");
        let clear = v.clear_search_source(source.clone());
        assert!(matches!(clear, Err(VaultError::Invalid(_))), "{source:?}: {clear:?}");
    }
    for id in [String::new(), format!("c{sep}1")] {
        let upsert = v.upsert_search_row(CHAT.into(), row(&id, "Budget", CHAT, 1));
        assert!(matches!(upsert, Err(VaultError::Invalid(_))), "{id:?}: {upsert:?}");
        let delete = v.delete_search_row(CHAT.into(), id.clone());
        assert!(matches!(delete, Err(VaultError::Invalid(_))), "{id:?}: {delete:?}");
    }

    page(&v, "Budget plan");
    let hits = v.search("budget".into(), 10).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.source == "vault"), "{hits:?}");
}

#[test]
fn page_only_search_never_returns_external_rows_even_when_they_outnumber_the_limit() {
    let t = TempVault::new("pages_only");
    let v = t.open();
    let plan = page(&v, "Budget plan");
    // An unknown page type, kept as it is, like a type a mod defines.
    std::fs::create_dir_all(t.0.join("pages")).unwrap();
    std::fs::write(
        t.0.join("pages/Board.md"),
        "---\ntitle: \"Budget board\"\ntype: \"kanban\"\n---\n\nCards.\n",
    )
    .unwrap();
    v.rescan().unwrap();
    let board = keys(&v.search("board".into(), 10).unwrap())
        .into_iter()
        .find(|(source, _)| source == "vault")
        .expect("the board page")
        .1;

    // Far more chats than the limit, and an external document.
    for i in 0..120 {
        v.upsert_search_row(
            CHAT.into(),
            row(&format!("c{i}"), &format!("Budget chat {i}"), CHAT, i),
        )
        .unwrap();
    }
    v.upsert_search_row("other".into(), row("d1", "Budget note", "document", 1))
        .unwrap();

    let pages = v.search_pages("budget".into(), 5).unwrap();
    assert_eq!(
        keys(&pages),
        [key("vault", &plan), key("vault", &board)].into_iter().collect(),
        "{pages:?}"
    );
    let board_hit = pages.iter().find(|h| h.id == board).unwrap();
    assert_eq!(board_hit.page_type, "kanban");

    // The mixed search still includes external rows.
    assert!(v
        .search("budget".into(), 5)
        .unwrap()
        .iter()
        .any(|h| h.source == CHAT));

    // Filters pass through the same scope: no chat or external document
    // can be a page result, and a page type still selects its pages.
    assert!(v.search_pages("type:chat budget".into(), 5).unwrap().is_empty());
    assert!(v.search_pages("type:document budget".into(), 5).unwrap().iter().all(|h| h.source == "vault"));
    let kanban = v.search_pages("type:kanban budget".into(), 5).unwrap();
    assert_eq!(keys(&kanban), [key("vault", &board)].into_iter().collect());
}

#[test]
fn schema_rebuild_preserves_external_rows_and_restores_projection_maintenance() {
    let t = TempVault::new("schema_rebuild");
    let v = t.open();
    let doc = page(&v, "Budget plan");
    v.upsert_search_row(CHAT.into(), row("c1", "Budget chat", CHAT, 5)).unwrap();
    v.flush().unwrap();
    drop(v);
    let conn = rusqlite::Connection::open(t.index()).unwrap();
    conn.pragma_update(None, "user_version", 0).unwrap();
    drop(conn);

    let v = t.open();
    assert!(keys(&v.search("budget".into(), 50).unwrap()).contains(&key(CHAT, "c1")));
    v.rename_page(doc.clone(), "Renewed plan".into()).unwrap();
    let added = page(&v, "Renewed second plan");
    let hits = v.search("type:document".into(), 50).unwrap();
    assert!(hits.iter().any(|h| h.id == doc && h.title == "Renewed plan"), "{hits:?}");
    assert!(hits.iter().any(|h| h.id == added), "new pages enter the rebuilt projection: {hits:?}");
    v.delete_search_row(CHAT.into(), "c1".into()).unwrap();
    assert!(!keys(&v.search("budget".into(), 50).unwrap()).contains(&key(CHAT, "c1")));
}

#[test]
fn external_title_filters_fold_unicode_like_vault_titles() {
    let t = TempVault::new("unicode");
    let v = t.open();
    v.upsert_search_row(CHAT.into(), row("c1", "Überprüfung", CHAT, 5)).unwrap();
    let hits = v.search("title:über".into(), 50).unwrap();
    assert_eq!(keys(&hits), [key(CHAT, "c1")].into_iter().collect());
}
