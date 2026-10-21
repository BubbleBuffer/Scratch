use vault_core::*;

#[test]
fn exact_external_embed_metadata_preserves_target_and_refuses_ambiguous_ids() {
    let root = std::env::temp_dir().join(format!("embed_metadata_{}", new_id()));
    {
        let v = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let row = ExternalSearchRow {
            id: "chat:session".into(), title: "Trip discussion".into(),
            body: "Outline summary".into(), page_type: "chat".into(),
            target: Some("opaque navigation target".into()), updated_ms: 10,
        };
        v.upsert_search_row("agent".into(), row.clone()).unwrap();
        let hit = v.find_indexed_note(row.id.clone()).unwrap().unwrap();
        assert_eq!(hit.source, "agent");
        assert_eq!(hit.title, row.title);
        assert_eq!(hit.page_type, "chat");
        assert_eq!(hit.target, row.target);
        assert_eq!(hit.snippet, row.body);
        assert!(v.find_indexed_note("missing".into()).unwrap().is_none());
        let host = v.create_page(None, "Host".into()).unwrap();
        v.apply(host.clone(), vec![
            BlockOp::Insert { parent: None, after: None, block: NewBlock {
                id: "embed".into(), kind: BlockKind::Embed, text: String::new(),
                marks: vec![], checked: false,
            }},
            BlockOp::SetTarget { id: "embed".into(), target: Some(row.id.clone()) },
        ]).unwrap();
        v.flush().unwrap();
        assert!(v.page_markdown(host.clone()).unwrap().starts_with("> Trip discussion <!-- block="));
        assert!(v.search("discussion".into(), 20).unwrap().iter().any(|h| h.id == host));
        v.upsert_search_row("other".into(), row).unwrap();
        assert!(v.find_indexed_note("chat:session".into()).unwrap().is_none());
        v.flush().unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}
