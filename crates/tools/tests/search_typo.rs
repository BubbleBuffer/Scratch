use serde_json::json;
use vault_core::{new_id, BlockKind, BlockOp, NewBlock, Vault};

#[test]
fn search_notes_corrects_body_typos_and_keeps_exact_hits_first() {
    let root = std::env::temp_dir().join(format!("notes-tools-typo-{}", new_id()));
    {
        let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let corrected = vault.create_page(None, "Food diary".into()).unwrap();
        vault
            .apply(
                corrected.clone(),
                vec![BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: NewBlock {
                        id: "food".into(),
                        kind: BlockKind::Paragraph,
                        text: "tomato harvest".into(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        vault.flush().unwrap();
        let output =
            vault_tools::call(&vault, "search_notes", &json!({"query":"tomatoe"})).unwrap();
        assert!(output.contains(&corrected), "{output}");
        // This contiguous title hit doesn't make `tomatoe` a vocabulary prefix.
        let exact = vault.create_page(None, "XtomatoeZ".into()).unwrap();
        let output =
            vault_tools::call(&vault, "search_notes", &json!({"query":"tomatoe"})).unwrap();
        assert!(
            output.find(&exact).unwrap() < output.find(&corrected).unwrap(),
            "{output}"
        );
        let output = vault_tools::call(&vault, "search_notes", &json!({"query":"tam"})).unwrap();
        assert!(
            !output.contains(&corrected),
            "short words must not be corrected: {output}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
