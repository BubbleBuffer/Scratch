use vault_core::{new_id, BlockKind, BlockOp, NewBlock, Vault};

#[test]
fn exact_title_with_punctuation_precedes_the_shorter_word_start_hit() {
    let root = std::env::temp_dir().join(format!("notes-exact-title-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let exact = vault.create_page(None, "Draft: garden".into()).unwrap();
        vault.create_page(None, "Draft garden".into()).unwrap();
        vault.flush().unwrap();
        for hits in [
            vault.search("Draft: garden".into(), 10).unwrap(),
            vault.search_full("Draft: garden".into(), 10).unwrap(),
        ] {
            assert_eq!(hits[0].id, exact);
            assert!(!hits[0].fuzzy);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrected_prefix_expansion_keeps_the_eight_vocabulary_word_cap() {
    let root = std::env::temp_dir().join(format!("notes-prefix-cap-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let add = |text: &str| {
            let id = vault.create_page(None, "Body".into()).unwrap();
            vault
                .apply(
                    id.clone(),
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
            id
        };
        for _ in 0..12 {
            add("garden");
        }
        for suffix in ["aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh", "ii"] {
            add(&format!("garden{suffix}"));
        }
        let outside = add("gardenzz");
        vault.flush().unwrap();
        for hits in [
            vault.search("gadr".into(), 50).unwrap(),
            vault.search_full("gadr".into(), 50).unwrap(),
        ] {
            // One frequent word plus seven equally frequent longer words.
            // Expanding the chosen `garden` again with `*` would leak the
            // other three vocabulary words into this result set.
            assert_eq!(hits.len(), 19);
            assert!(hits.iter().all(|h| h.fuzzy));
            assert!(hits.iter().all(|h| h.id != outside));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn body_hits_use_their_matched_correction_and_edit_penalty() {
    let root = std::env::temp_dir().join(format!("notes-search-ranking-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let add = |title: &str, text: String| {
            let id = vault.create_page(None, title.into()).unwrap();
            vault
                .apply(
                    id.clone(),
                    vec![BlockOp::Insert {
                        parent: None,
                        after: None,
                        block: NewBlock {
                            id: new_id(),
                            kind: BlockKind::Paragraph,
                            text,
                            marks: vec![],
                            checked: false,
                        },
                    }],
                )
                .unwrap();
            id
        };
        // The second page has stronger term frequency, but needs two edits.
        let one = add("A", "abcefgh".into());
        let two = add("B", "abxyefgh ".repeat(30));
        vault.flush().unwrap();
        for hits in [
            vault.search("abcdefgh".into(), 10).unwrap(),
            vault.search_full("abcdefgh".into(), 10).unwrap(),
        ] {
            let first = hits.iter().find(|h| h.id == one).unwrap();
            let second = hits.iter().find(|h| h.id == two).unwrap();
            assert_eq!(first.corrected.as_deref(), Some("abcefgh"));
            assert_eq!(second.corrected.as_deref(), Some("abxyefgh"));
            assert!(first.fuzzy && second.fuzzy);
            assert!(
                hits.iter().position(|h| h.id == one).unwrap()
                    < hits.iter().position(|h| h.id == two).unwrap()
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn indexed_words_do_not_trigger_title_subsequence_correction() {
    let root = std::env::temp_dir().join(format!("notes-title-ranking-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let title = vault.create_page(None, "Garden plan".into()).unwrap();
        for i in 0..12 {
            let id = vault.create_page(None, format!("Body {i}")).unwrap();
            vault
                .apply(
                    id,
                    vec![BlockOp::Insert {
                        parent: None,
                        after: None,
                        block: NewBlock {
                            id: new_id(),
                            kind: BlockKind::Paragraph,
                            text: "grdn pln".into(),
                            marks: vec![],
                            checked: false,
                        },
                    }],
                )
                .unwrap();
        }
        vault.flush().unwrap();
        for hits in [
            vault.search("grdn pln".into(), 50).unwrap(),
            vault.search_full("grdn pln".into(), 50).unwrap(),
        ] {
            assert_eq!(hits.len(), 12);
            assert!(hits.iter().all(|h| !h.fuzzy && h.id != title));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn substring_titles_do_not_suppress_body_correction() {
    let root = std::env::temp_dir().join(format!("notes-substring-fallback-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        for i in 0..12 {
            vault.create_page(None, format!("XtomatoeZ {i}")).unwrap();
        }
        let body = vault.create_page(None, "A".into()).unwrap();
        vault
            .apply(
                body.clone(),
                vec![BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: NewBlock {
                        id: new_id(),
                        kind: BlockKind::Paragraph,
                        text: "tomato harvest".into(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        vault.flush().unwrap();
        for hits in [
            vault.search("tomatoe".into(), 50).unwrap(),
            vault.search_full("tomatoe".into(), 50).unwrap(),
        ] {
            assert_eq!(hits.len(), 13);
            assert!(hits[..12].iter().all(|h| !h.fuzzy));
            assert_eq!(hits[12].id, body);
            assert!(hits[12].fuzzy);
            assert_eq!(hits[12].corrected.as_deref(), Some("tomato"));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
