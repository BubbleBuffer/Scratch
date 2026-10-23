//! Title hits keep their body snippet when the body also matches. Title ids
//! are excluded from body searches, so their snippets come from one separate
//! query. Title-first order and typo correction must stay as they were.
use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> TempVault {
        TempVault(std::env::temp_dir().join(format!("vault_title_snip_{}", new_id())))
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

fn body(v: &Vault, id: &str, text: &str) {
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
    body(v, &id, text);
    id
}

fn corpus(v: &Vault) {
    // Title and body both match: the title hit keeps its body snippet.
    page(v, "Tomato sauce", "simmer the tomatoes slowly");
    // Title-only match: the snippet is the body's opening text, as before.
    page(v, "Tomato planning", "calendar for the spring");
    // Body-only match: it follows the title tier with its own snippet.
    page(v, "Garden plan", "spring beds and tomatoes");
    v.flush().unwrap();
}

#[test]
fn title_hits_keep_body_snippets_in_title_first_order() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);

    for hits in [
        v.search("tomato".into(), 10).unwrap(),
        v.search_full("tomato".into(), 10).unwrap(),
        v.search_pages("tomato".into(), 10).unwrap(),
    ] {
        // Both title hits come first; the body-only page follows them.
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles.len(), 3, "{titles:?}");
        assert_eq!(titles[2], "Garden plan");
        assert!(titles[..2].contains(&"Tomato sauce"));
        assert!(titles[..2].contains(&"Tomato planning"));

        let sauce = hits.iter().find(|h| h.title == "Tomato sauce").unwrap();
        assert!(!sauce.fuzzy);
        assert!(
            sauce.snippet.to_lowercase().contains("tomato"),
            "title hit lost its body snippet: {:?}",
            sauce.snippet
        );

        let planning = hits.iter().find(|h| h.title == "Tomato planning").unwrap();
        assert_eq!(planning.snippet, "calendar for the spring");

        let garden = hits.iter().find(|h| h.title == "Garden plan").unwrap();
        assert!(garden.snippet.to_lowercase().contains("tomatoes"));
    }
}

#[test]
fn page_search_keeps_the_snippet_when_chat_titles_crowd_the_limit() {
    let t = TempVault::new();
    let v = t.open();
    let budget = v.create_page(None, "Budget plan".into()).unwrap();
    body(&v, &budget, "Some text about budget.");
    // Exact-title chats outrank the prefix page in the all-source search.
    for i in 0..120 {
        v.upsert_search_row(
            "agent".into(),
            ExternalSearchRow {
                id: format!("chat-{i}"),
                title: "Budget".into(),
                body: String::new(),
                page_type: "chat".into(),
                target: None,
                updated_ms: 0,
            },
        )
        .unwrap();
    }
    v.flush().unwrap();

    let hits = v.search_pages("budget".into(), 100).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
        [budget.as_str()]
    );
    assert_eq!(hits[0].source, "vault");
    assert_eq!(hits[0].snippet, "Some text about budget.");
}

#[test]
fn typo_hits_still_follow_exact_hits() {
    let t = TempVault::new();
    let v = t.open();
    corpus(&v);

    // "tomatoe" is not in the vocabulary, so correction runs after exact hits.
    let hits = v.search_full("tomatoe".into(), 10).unwrap();
    let first_fuzzy = hits.iter().position(|h| h.fuzzy).unwrap_or(hits.len());
    assert!(hits[..first_fuzzy].iter().all(|h| !h.fuzzy));
    assert!(hits[first_fuzzy..].iter().all(|h| h.fuzzy));
}
