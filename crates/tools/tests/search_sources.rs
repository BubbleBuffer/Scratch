use vault_core::{ExternalSearchRow, Vault};

#[test]
fn page_title_resolution_ignores_external_chats_with_the_same_title() {
    let root = std::env::temp_dir().join(format!("notes_tool_search_source_{}", vault_core::new_id()));
    let vault = Vault::open(root.to_string_lossy().into_owned()).unwrap();
    let page = vault.create_page(None, "Budget".into()).unwrap();
    vault.upsert_search_row("agent".into(), ExternalSearchRow {
        id: "chat:session".into(),
        title: "Budget".into(),
        body: "Discussed the budget".into(),
        page_type: "chat".into(),
        target: None,
        updated_ms: 1,
    }).unwrap();
    assert_eq!(vault_tools::resolve_page(&vault, "Budget").unwrap(), page);
    drop(vault);
    std::fs::remove_dir_all(root).unwrap();
}
