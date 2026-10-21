use vault_core::{new_id, BlockKind, BlockOp, NewBlock, Vault};

#[test]
fn broad_corrected_expansion_narrows_and_fills_by_recency() {
    let root = std::env::temp_dir().join(format!("notes-corrected-narrowing-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        // 300 pages match the corrected word, above the 200-page broad floor.
        for i in 0..300 {
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
                            text: "garden plan".into(),
                            marks: vec![],
                            checked: false,
                        },
                    }],
                )
                .unwrap();
        }
        vault.flush().unwrap();

        // "gardne" is absent from the vocabulary and has one candidate, "garden",
        // at a constant cost of one edit. The fill path applies.
        let adaptive = vault.search_adaptive("gardne".into(), 50).unwrap();
        assert!(adaptive.narrowed);
        assert_eq!(adaptive.hits.len(), 50);
        assert!(adaptive.hits.iter().all(|h| h.fuzzy));
        assert!(adaptive
            .hits
            .iter()
            .all(|h| h.corrected.as_deref() == Some("garden")));
        assert!(adaptive.hits.iter().all(|h| h.snippet.is_empty()));

        // The full ranked search keeps its bm25 path and snippets.
        let full = vault.search_full("gardne".into(), 50).unwrap();
        assert_eq!(full.len(), 50);
        assert!(full.iter().all(|h| h.fuzzy));
        assert!(full.iter().any(|h| !h.snippet.is_empty()));
    }
    std::fs::remove_dir_all(root).unwrap();
}
