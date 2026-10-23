use std::path::PathBuf;
use vault_core::*;

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn vault() -> (Dir, Vault) {
    let path = std::env::temp_dir().join(format!("conflict_detection_{}", new_id()));
    let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
    (Dir(path), vault)
}
fn copy(from: &Vault, to: &Vault, page: &str) {
    to.import_updates(
        page.into(),
        from.export_updates(page.into(), vec![]).unwrap(),
    )
    .unwrap();
}
fn insert(id: &str, parent: Option<&str>, after: Option<&str>, text: &str) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(Into::into),
        after: after.map(Into::into),
        block: NewBlock {
            id: id.into(),
            kind: BlockKind::Paragraph,
            text: text.into(),
            marks: vec![],
            checked: false,
        },
    }
}
fn edit(id: &str, text: &str) -> BlockOp {
    BlockOp::SetText {
        id: id.into(),
        text: text.into(),
        marks: vec![],
    }
}

#[test]
fn nested_subtree_delete_edit_restores_the_entire_editing_version_at_detection() {
    let (_a_dir, a) = vault();
    let (_b_dir, b) = vault();
    let page = a.create_page(None, "Section".into()).unwrap();
    a.apply(
        page.clone(),
        vec![
            insert("root", None, None, "Section"),
            insert("nested", Some("root"), None, "Nested"),
            insert("edited", Some("nested"), None, "Original"),
            insert("unchanged", Some("root"), Some("nested"), "Keep me"),
        ],
    )
    .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![edit("edited", "Offline edit")])
        .unwrap();
    b.apply(page.clone(), vec![BlockOp::Delete { id: "root".into() }])
        .unwrap();
    copy(&a, &b, &page);
    assert_eq!(
        b.load_page(page.clone()).unwrap().blocks,
        a.load_page(page.clone()).unwrap().blocks
    );
    let records = b.list_conflicts(Some(page)).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].scope,
        ConflictScope::Subtree {
            root: "root".into()
        }
    );
    assert_eq!(records[0].status, ConflictStatus::Pending);
    assert!(
        matches!(&records[0].applied[0].by, ResolutionBy::Auto { rule } if rule == "edit-beats-delete")
    );
    assert!(records[0]
        .sides
        .iter()
        .any(|side| matches!(side.state, SideState::Deleted)));
}

#[test]
fn trash_edit_restores_the_whole_page_and_records_both_lifecycle_sides() {
    let (_a_dir, a) = vault();
    let (_b_dir, b) = vault();
    let page = a.create_page(None, "Page".into()).unwrap();
    a.apply(
        page.clone(),
        vec![
            insert("edited", None, None, "Original"),
            insert("unchanged", None, Some("edited"), "Keep me"),
        ],
    )
    .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![edit("edited", "Offline edit")])
        .unwrap();
    b.delete_page(page.clone()).unwrap();
    copy(&a, &b, &page);
    assert_eq!(
        b.load_page(page.clone()).unwrap().blocks,
        a.load_page(page.clone()).unwrap().blocks
    );
    let records = b.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].scope, ConflictScope::Page);
    assert!(records[0]
        .sides
        .iter()
        .any(|side| matches!(side.state, SideState::Trashed { .. })));
    let raw = loro::LoroDoc::new();
    raw.import(&b.export_updates(page, vec![]).unwrap())
        .unwrap();
    assert!(raw.get_map("page").get("trashed").is_none());
}

#[test]
fn causal_purge_return_records_page_scope_without_changing_restoration() {
    let (_a_dir, a) = vault();
    let (_b_dir, b) = vault();
    let page = a.create_page(None, "Purged page".into()).unwrap();
    a.apply(page.clone(), vec![insert("edited", None, None, "Original")])
        .unwrap();
    copy(&a, &b, &page);
    b.delete_page(page.clone()).unwrap();
    b.purge_page(page.clone()).unwrap();
    a.apply(page.clone(), vec![edit("edited", "Offline edit")])
        .unwrap();
    copy(&a, &b, &page);
    assert_eq!(
        b.load_page(page.clone()).unwrap().blocks[0].text,
        "Offline edit"
    );
    let records = b.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].scope, ConflictScope::Page);
    assert!(records[0]
        .sides
        .iter()
        .any(|side| matches!(side.state, SideState::Purged { .. })));
    assert!(matches!(
        records[0].applied[0].before_state,
        Some(BeforeState::Purged { .. })
    ));
    assert!(records[0].applied[0].before.is_empty());
    let raw = loro::LoroDoc::new();
    raw.import(&b.export_updates(page, vec![]).unwrap())
        .unwrap();
    assert!(raw.get_map("page").get("trashed").is_none());
    assert!(raw.get_map("page").get("purge_restore_warning").is_none());
}

#[test]
fn legacy_purge_return_records_page_scope_and_keeps_the_trash_warning() {
    let (_a_dir, a) = vault();
    let (_b_dir, b) = vault();
    let page = a.create_page(None, "Legacy page".into()).unwrap();
    a.apply(page.clone(), vec![insert("edited", None, None, "Original")])
        .unwrap();
    b.import_purged(vec![page.clone()]).unwrap();
    a.apply(page.clone(), vec![edit("edited", "Offline edit")])
        .unwrap();
    copy(&a, &b, &page);
    let records = b.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].scope, ConflictScope::Page);
    assert!(records[0].applied.is_empty());
    let raw = loro::LoroDoc::new();
    raw.import(&b.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    assert!(raw.get_map("page").get("trashed").is_some());
    assert!(raw.get_map("page").get("purge_restore_warning").is_some());
    assert_eq!(b.restored_after_purge_pages().unwrap(), vec![page]);
}

#[test]
fn simultaneous_detection_on_both_devices_converges_to_one_editing_subtree() {
    let (_a_dir, a) = vault();
    let (_b_dir, b) = vault();
    let (_c_dir, c) = vault();
    let page = a.create_page(None, "Concurrent repair".into()).unwrap();
    a.apply(
        page.clone(),
        vec![
            insert("root", None, None, "Root"),
            insert("edited", Some("root"), None, "Original"),
            insert("unchanged", Some("root"), Some("edited"), "Keep original"),
        ],
    )
    .unwrap();
    copy(&a, &b, &page);
    copy(&a, &c, &page);
    a.apply(page.clone(), vec![edit("edited", "Editing side")])
        .unwrap();
    let expected = a.load_page(page.clone()).unwrap().blocks;
    b.apply(
        page.clone(),
        vec![
            edit("unchanged", "Discard deletion-side change"),
            BlockOp::Delete { id: "root".into() },
        ],
    )
    .unwrap();
    let left = a.export_updates(page.clone(), vec![]).unwrap();
    let right = b.export_updates(page.clone(), vec![]).unwrap();
    a.import_updates(page.clone(), right).unwrap();
    b.import_updates(page.clone(), left).unwrap();
    for _ in 0..2 {
        copy(&a, &b, &page);
        copy(&b, &a, &page);
        copy(&a, &c, &page);
    }
    for vault in [&a, &b, &c] {
        assert_eq!(vault.load_page(page.clone()).unwrap().blocks, expected);
        assert_eq!(vault.list_conflicts(Some(page.clone())).unwrap().len(), 1);
    }
}
