//! The MCP read/edit boundary uses the core view fence format.

use serde_json::json;
use vault_core::{BlockKind, BlockOp, NewBlock, Vault};

#[test]
fn read_view_fence_can_be_used_for_an_equal_edit() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().to_string_lossy().into_owned()).unwrap();
    let page = vault.create_page(None, "Contents".into()).unwrap();
    let source = vault.create_page(None, "Source".into()).unwrap();
    let text = json!({"source": "contents", "name": "Cards", "kind": "Gallery"}).to_string();
    vault
        .apply(
            page.clone(),
            vec![
                BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: NewBlock {
                        id: "view".into(),
                        kind: BlockKind::View,
                        text,
                        marks: vec![],
                        checked: false,
                    },
                },
                BlockOp::SetTarget {
                    id: "view".into(),
                    target: Some(source.clone()),
                },
            ],
        )
        .unwrap();
    let before = vault.load_page(page.clone()).unwrap().blocks[0].clone();
    let read = vault_tools::call(
        &vault,
        "read_page",
        &json!({"page": page, "block_ids": true}),
    )
    .unwrap();
    assert!(read.contains("<!-- block view -->"), "{read}");
    let start = read.find("```view\n").expect("view fence in read_page");
    let end = start + read[start..].find("\n```").unwrap() + 4;
    let fence = &read[start..end];
    assert!(fence.contains(&format!(r#""page":"{source}""#)), "{fence}");
    vault_tools::call(
        &vault,
        "edit_page",
        &json!({
            "page": page, "op": "replace", "block_ids": ["view"], "markdown": fence,
        }),
    )
    .unwrap();
    let mut after = vault.load_page(page).unwrap().blocks[0].clone();
    // The edit API may replace block identity; its contents must be identical.
    after.id = before.id.clone();
    assert_eq!(after, before);
}
