use super::*;

#[test]
fn an_explicit_source_is_validated_and_saved_before_queueing() {
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Temp(std::env::temp_dir().join(format!("task_source_{}", vault_core::new_id())));
    let v = Vault::open(dir.0.to_string_lossy().into()).unwrap();
    let page = v.create_page(None, "Plan".into()).unwrap();
    let source = vault_core::new_id();
    v.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: source.clone(),
                kind: BlockKind::Todo,
                text: "Assigned work".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    let before = v.load_page(page.clone()).unwrap();
    assert!(create_on_with(
        &v,
        "Do the work",
        &page,
        None,
        "agent",
        "",
        &[("source_block", "missing")],
    )
    .is_err());
    assert_eq!(v.load_page(page.clone()).unwrap(), before);
    assert!(collection(&v).unwrap().is_none());

    // No placement block is supplied: the explicit source still wins.
    let task = create_on_with(
        &v,
        "Do the work",
        &page,
        None,
        "agent",
        "",
        &[("source_block", &source)],
    )
    .unwrap();
    assert_eq!(task.source_block.as_deref(), Some(source.as_str()));
    assert_eq!(task.status, QUEUED);
    assert_eq!(
        claim_next(&v, &v.device_id())
            .unwrap()
            .unwrap()
            .source_block,
        Some(source)
    );
}
