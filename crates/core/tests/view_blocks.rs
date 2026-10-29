//! View payloads and the concurrent first-open migration boundary.

use loro::{ExportMode, LoroDoc, LoroText, ToJson, TreeParentId};
use serde_json::json;
use vault_core::*;

struct TempVault(std::path::PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_views_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snapshot(vault: &Vault, page: &str) -> LoroDoc {
    let doc = LoroDoc::new();
    doc.import(&vault.export_updates(page.into(), vec![]).unwrap())
        .unwrap();
    doc
}

fn legacy(vault: &Vault, schema: &[PropDef], views: &[ViewDef]) -> String {
    let page = vault
        .create_page_with_type(
            None,
            "Old collection".into(),
            Some(PAGE_TYPE_DATABASE.into()),
        )
        .unwrap();
    let doc = snapshot(vault, &page);
    doc.set_peer_id(10_000).unwrap();
    for def in schema {
        doc.get_map("schema")
            .insert(&def.key, serde_json::to_string(def).unwrap())
            .unwrap();
        doc.get_list("schema_order").push(def.key.as_str()).unwrap();
    }
    for def in views {
        doc.get_map("views")
            .insert(&def.id, serde_json::to_string(def).unwrap())
            .unwrap();
        doc.get_list("view_order").push(def.id.as_str()).unwrap();
    }
    doc.commit();
    vault
        .import_updates(page.clone(), doc.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    page
}

fn non_title_schema() -> Vec<PropDef> {
    vec![PropDef {
        key: "status".into(),
        name: "Status".into(),
        kind: PropKind::Status,
        options: vec![],
        config: String::new(),
    }]
}

fn exchange(a: &Vault, b: &Vault, page: &str) {
    let a_bytes = a.export_updates(page.into(), vec![]).unwrap();
    let b_bytes = b.export_updates(page.into(), vec![]).unwrap();
    a.import_updates(page.into(), b_bytes).unwrap();
    b.import_updates(page.into(), a_bytes).unwrap();
}

fn insert(id: &str, parent: Option<&str>, kind: BlockKind, text: &str) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(Into::into),
        after: None,
        block: NewBlock {
            id: id.into(),
            kind,
            text: text.into(),
            checked: false,
            marks: vec![],
        },
    }
}

fn block(text: &str) -> Block {
    Block {
        id: "block-id".into(),
        kind: BlockKind::View,
        text: text.into(),
        marks: vec![],
        checked: false,
        children: vec![],
        target: None,
        color: None,
        width: None,
    }
}

fn view() -> ViewDef {
    ViewDef {
        id: "legacy-id".into(),
        name: "Due work".into(),
        kind: ViewKind::Board,
        filter: Some(Filter::Cond(Condition {
            prop: "status".into(),
            op: FilterOp::Is,
            value: PropValue::Status("open".into()),
        })),
        sorts: vec![Sort {
            prop: "$title".into(),
            descending: true,
        }],
        group_by: Some("status".into()),
        date_prop: Some("due".into()),
        visible_props: vec!["status".into(), "due".into()],
        card_cover: Some("$cover".into()),
        config: r#"{"columnWidths":{"status":200}}"#.into(),
    }
}

#[test]
fn view_kind_and_payload_round_trip_with_block_identity() {
    assert_eq!(BlockKind::View.as_str(), "view");
    assert_eq!(BlockKind::parse("view"), BlockKind::View);
    assert_eq!(
        serde_json::to_value(BlockKind::View).unwrap(),
        json!("view")
    );
    let original = view();
    let text = view_block_text(&original);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["source"], "contents");
    assert!(value.get("id").is_none());
    let mut expected = original;
    expected.id = "block-id".into();
    assert_eq!(ViewBlock::def(&block(&text)), expected);

    // An id supplied by an old writer cannot impersonate another block.
    let mut legacy = value;
    legacy["id"] = json!({"invalid": "ignored"});
    assert_eq!(ViewBlock::def(&block(&legacy.to_string())), expected);
}

#[test]
fn missing_and_malformed_view_payloads_have_a_safe_list_default() {
    for text in ["", "{bad json", "null", "[]", r#"{"kind":"future-layout"}"#] {
        let def = ViewBlock::def(&block(text));
        assert_eq!(def.id, "block-id");
        assert_eq!(def.kind, ViewKind::List);
        assert_eq!(def.name, "List");
        assert!(def.filter.is_none());
        assert!(def.sorts.is_empty());
    }
    let partial = ViewBlock::def(&block(r#"{"name":"My cards","kind":"Gallery"}"#));
    assert_eq!(partial.id, "block-id");
    assert_eq!(partial.name, "My cards");
    assert_eq!(partial.kind, ViewKind::Gallery);
    assert!(partial.visible_props.is_empty());
}

#[test]
fn concurrent_same_id_tree_inserts_retain_distinct_nodes_and_their_edits() {
    // These are the operations a lazy migration performs on two replicas:
    // both start from one page and assign the same deterministic logical id.
    let baseline = LoroDoc::new();
    baseline.set_peer_id(1).unwrap();
    baseline.get_map("page").insert("id", "page-id").unwrap();
    baseline.commit();
    let bytes = baseline.export(ExportMode::Snapshot).unwrap();
    let a = LoroDoc::new();
    let b = LoroDoc::new();
    let mut created = Vec::new();
    for (doc, peer) in [(&a, 2), (&b, 3)] {
        doc.import(&bytes).unwrap();
        doc.set_peer_id(peer).unwrap();
        let tree = doc.get_tree("blocks");
        tree.enable_fractional_index(0);
        let node = tree.create_at(TreeParentId::Root, 0).unwrap();
        let meta = tree.get_meta(node).unwrap();
        meta.insert("id", "same-deterministic-view-id").unwrap();
        meta.insert("kind", "view").unwrap();
        let text = meta.insert_container("text", LoroText::new()).unwrap();
        text.insert(0, &view_block_text(&view())).unwrap();
        doc.get_map("page").insert("views_migrated", 1_i64).unwrap();
        doc.commit();
        created.push(node);
    }

    // A user edits a migrated view before the replicas meet. Dropping either
    // duplicate now risks dropping real state; identity alone cannot dedup.
    b.get_tree("blocks")
        .get_meta(created[1])
        .unwrap()
        .insert("target", "another-source-page")
        .unwrap();
    b.commit();
    a.import(&b.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    b.import(&a.export(ExportMode::all_updates()).unwrap())
        .unwrap();

    for doc in [&a, &b] {
        let tree = doc.get_tree("blocks");
        assert_eq!(tree.roots().len(), 2, "logical ids do not merge tree nodes");
        assert!(tree.roots().contains(&created[0]));
        assert!(tree.roots().contains(&created[1]));
        assert!(tree.get_meta(created[0]).unwrap().get("target").is_none());
        assert!(tree.get_meta(created[1]).unwrap().get("target").is_some());
    }
    assert_eq!(a.get_deep_value(), b.get_deep_value());
}

#[test]
fn first_open_appends_saved_views_in_order_and_marker_makes_reopen_a_noop() {
    let temp = TempVault::new();
    let vault = temp.open();
    let first = view();
    let mut second = view();
    second.id = "second-view".into();
    second.name = "Calendar".into();
    second.kind = ViewKind::Calendar;
    let page = legacy(
        &vault,
        &non_title_schema(),
        &[first.clone(), second.clone()],
    );
    vault
        .apply(
            page.clone(),
            vec![insert(
                "prose",
                None,
                BlockKind::Paragraph,
                "Keep this first",
            )],
        )
        .unwrap();
    let child = vault
        .create_page(Some(page.clone()), "Existing child".into())
        .unwrap();
    let opened = vault.load_page(page.clone()).unwrap();
    assert_eq!(opened.page_type, PAGE_TYPE_DATABASE);
    assert_eq!(opened.blocks.len(), 3);
    assert_eq!(opened.blocks[0].id, "prose");
    for (block, mut expected) in opened.blocks[1..]
        .iter()
        .zip([first.clone(), second.clone()])
    {
        assert_eq!(block.kind, BlockKind::View);
        expected.id = block.id.clone();
        assert_eq!(ViewBlock::def(block), expected);
        assert_eq!(block.target, None);
    }
    assert_ne!(opened.blocks[1].id, opened.blocks[2].id);
    assert_eq!(vault.list_views(page.clone()).unwrap(), vec![first, second]);
    assert_eq!(vault.load_page(child).unwrap().parent, Some(page.clone()));
    let after = snapshot(&vault, &page);
    assert_eq!(
        after
            .get_map("page")
            .get("views_migrated")
            .unwrap()
            .get_deep_value(),
        loro::LoroValue::I64(1)
    );
    assert_eq!(vault.load_page(page.clone()).unwrap().blocks, opened.blocks);
    assert_eq!(snapshot(&vault, &page).oplog_vv(), after.oplog_vv());
    vault.flush().unwrap();
    drop(vault);
    let reopened = temp.open();
    assert_eq!(reopened.load_page(page).unwrap().blocks, opened.blocks);
}

#[test]
fn title_only_database_becomes_document_with_one_default_list_and_keeps_children() {
    for schema in [
        vec![],
        vec![PropDef {
            key: "$title".into(),
            name: "Name".into(),
            kind: PropKind::Text,
            options: vec![],
            config: String::new(),
        }],
    ] {
        let temp = TempVault::new();
        let vault = temp.open();
        let mut other = view();
        other.id = "other".into();
        let saved = vec![view(), other];
        let page = legacy(&vault, &schema, &saved);
        let child = vault
            .create_page(Some(page.clone()), "Child".into())
            .unwrap();
        let opened = vault.load_page(page.clone()).unwrap();
        assert_eq!(opened.page_type, PAGE_TYPE_DOCUMENT);
        assert_eq!(opened.blocks.len(), 1);
        let list = ViewBlock::def(&opened.blocks[0]);
        assert_eq!(list.name, "List");
        assert_eq!(list.kind, ViewKind::List);
        assert!(list.filter.is_none());
        assert!(list.sorts.is_empty());
        assert_eq!(vault.list_views(page.clone()).unwrap(), saved);
        assert_eq!(
            snapshot(&vault, &page).get_map("schema").len(),
            schema.len()
        );
        assert_eq!(vault.load_page(child).unwrap().parent, Some(page.clone()));
        let before = snapshot(&vault, &page).oplog_vv();
        assert_eq!(vault.load_page(page.clone()).unwrap().blocks, opened.blocks);
        assert_eq!(snapshot(&vault, &page).oplog_vv(), before);
    }
}

#[test]
fn database_without_saved_views_gets_only_marker_and_document_is_untouched() {
    let temp = TempVault::new();
    let vault = temp.open();
    let page = legacy(&vault, &[], &[]);
    let opened = vault.load_page(page.clone()).unwrap();
    assert_eq!(opened.page_type, PAGE_TYPE_DATABASE);
    assert!(opened.blocks.is_empty());
    let after = snapshot(&vault, &page);
    assert!(after.get_map("page").get("views_migrated").is_some());
    vault.load_page(page.clone()).unwrap();
    assert_eq!(snapshot(&vault, &page).oplog_vv(), after.oplog_vv());

    let document = vault.create_page(None, "Document".into()).unwrap();
    let before = snapshot(&vault, &document).oplog_vv();
    vault.load_page(document.clone()).unwrap();
    assert_eq!(snapshot(&vault, &document).oplog_vv(), before);
    assert!(snapshot(&vault, &document)
        .get_map("page")
        .get("views_migrated")
        .is_none());
}

#[test]
fn concurrent_first_opens_and_concurrent_dedup_converge_to_one_identical_view() {
    let temp_a = TempVault::new();
    let temp_b = TempVault::new();
    let a = temp_a.open();
    let b = temp_b.open();
    let page = legacy(&a, &non_title_schema(), &[view()]);
    b.import_updates(
        page.clone(),
        a.export_updates(page.clone(), vec![]).unwrap(),
    )
    .unwrap();
    let a_blocks = a.load_page(page.clone()).unwrap().blocks;
    let b_blocks = b.load_page(page.clone()).unwrap().blocks;
    assert_eq!(
        a_blocks, b_blocks,
        "replica migrations derive the same logical id"
    );
    exchange(&a, &b, &page);
    let merged = snapshot(&a, &page);
    assert_eq!(merged.get_tree("blocks").roots().len(), 2);
    let first = merged.get_tree("blocks").roots()[0];
    assert_eq!(a.load_page(page.clone()).unwrap().blocks.len(), 1);
    assert_eq!(b.load_page(page.clone()).unwrap().blocks.len(), 1);
    assert_eq!(snapshot(&a, &page).get_tree("blocks").roots(), vec![first]);
    exchange(&a, &b, &page);
    assert_eq!(
        snapshot(&a, &page).get_deep_value(),
        snapshot(&b, &page).get_deep_value()
    );
    let before = snapshot(&a, &page).oplog_vv();
    a.load_page(page.clone()).unwrap();
    assert_eq!(snapshot(&a, &page).oplog_vv(), before);
}

#[test]
fn duplicate_copy_uses_physical_identity_and_keeps_unknown_metadata_in_place() {
    let temp = TempVault::new();
    let vault = temp.open();
    let page = vault.create_page(None, "View repair".into()).unwrap();
    vault
        .apply(
            page.clone(),
            vec![insert(
                "view",
                None,
                BlockKind::View,
                &view_block_text(&view()),
            )],
        )
        .unwrap();
    let doc = snapshot(&vault, &page);
    doc.set_peer_id(20_000).unwrap();
    let tree = doc.get_tree("blocks");
    tree.enable_fractional_index(0);
    let original = tree.roots()[0];
    let duplicate = tree.create_at(TreeParentId::Root, 1).unwrap();
    let meta = tree.get_meta(duplicate).unwrap();
    meta.insert("id", "view").unwrap();
    meta.insert("kind", "view").unwrap();
    meta.insert("target", "different-source").unwrap();
    meta.insert("color", "blue_background").unwrap();
    meta.insert("checked", true).unwrap();
    meta.insert("width", 42_i64).unwrap();
    let extension = meta
        .insert_container("future-state", loro::LoroMap::new())
        .unwrap();
    extension.insert("mode", "preserved").unwrap();
    let text = meta.insert_container("text", LoroText::new()).unwrap();
    text.insert(0, &view_block_text(&view())).unwrap();
    let child = tree.create_at(duplicate, 0).unwrap();
    let child_meta = tree.get_meta(child).unwrap();
    child_meta.insert("id", "descendant").unwrap();
    child_meta.insert("kind", "paragraph").unwrap();
    child_meta.insert("future-flag", 7_i64).unwrap();
    child_meta
        .insert_container("text", LoroText::new())
        .unwrap()
        .insert(0, "Keep this child")
        .unwrap();
    let grandchild = tree.create_at(child, 0).unwrap();
    tree.get_meta(grandchild)
        .unwrap()
        .insert("id", "grandchild")
        .unwrap();
    tree.get_meta(grandchild)
        .unwrap()
        .insert("kind", "paragraph")
        .unwrap();
    let following = tree.create_at(TreeParentId::Root, 2).unwrap();
    tree.get_meta(following)
        .unwrap()
        .insert("id", "following")
        .unwrap();
    tree.get_meta(following)
        .unwrap()
        .insert("kind", "paragraph")
        .unwrap();
    let mut expected_meta = meta.get_deep_value().to_json_value();
    let mut expected_child = child_meta.get_deep_value().to_json_value();
    doc.commit();
    vault
        .import_updates(page.clone(), doc.export(ExportMode::all_updates()).unwrap())
        .unwrap();

    let opened = vault.load_page(page.clone()).unwrap();
    assert_eq!(opened.blocks.len(), 3);
    assert_eq!(opened.blocks[0].id, "view");
    assert_eq!(opened.blocks[2].id, "following");
    assert_ne!(opened.blocks[1].id, "view");
    assert_ne!(opened.blocks[1].children[0].id, "descendant");
    assert_ne!(opened.blocks[1].children[0].children[0].id, "grandchild");
    let repaired = snapshot(&vault, &page);
    let repaired_tree = repaired.get_tree("blocks");
    assert_eq!(repaired_tree.roots()[0], original);
    assert_eq!(repaired_tree.roots()[2], following);
    assert!(repaired_tree.is_node_deleted(&duplicate).unwrap());
    let replacement = repaired_tree.roots()[1];
    expected_meta["id"] = json!(opened.blocks[1].id);
    assert_eq!(
        repaired_tree
            .get_meta(replacement)
            .unwrap()
            .get_deep_value()
            .to_json_value(),
        expected_meta
    );
    let replacement_child = repaired_tree.children(replacement).unwrap()[0];
    expected_child["id"] = json!(opened.blocks[1].children[0].id);
    assert_eq!(
        repaired_tree
            .get_meta(replacement_child)
            .unwrap()
            .get_deep_value()
            .to_json_value(),
        expected_child
    );
}

#[test]
fn duplicate_identical_text_target_and_children_keeps_first_even_when_color_differs() {
    let temp = TempVault::new();
    let vault = temp.open();
    let page = vault.create_page(None, "Duplicates".into()).unwrap();
    vault
        .apply(
            page.clone(),
            vec![insert("view", None, BlockKind::View, "same")],
        )
        .unwrap();
    let doc = snapshot(&vault, &page);
    doc.set_peer_id(30_000).unwrap();
    let tree = doc.get_tree("blocks");
    tree.enable_fractional_index(0);
    let first = tree.roots()[0];
    let later = tree.create_at(TreeParentId::Root, 1).unwrap();
    let meta = tree.get_meta(later).unwrap();
    meta.insert("id", "view").unwrap();
    meta.insert("kind", "view").unwrap();
    meta.insert("color", "red").unwrap();
    meta.insert_container("text", LoroText::new())
        .unwrap()
        .insert(0, "same")
        .unwrap();
    doc.commit();
    vault
        .import_updates(page.clone(), doc.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    assert_eq!(vault.load_page(page.clone()).unwrap().blocks.len(), 1);
    assert_eq!(
        snapshot(&vault, &page).get_tree("blocks").roots(),
        vec![first]
    );
}

#[test]
fn existing_replacement_identity_is_reserved_before_repairing_an_old_source() {
    let temp_a = TempVault::new();
    let temp_b = TempVault::new();
    let a = temp_a.open();
    let b = temp_b.open();
    let page = a.create_page(None, "Occupied replacement".into()).unwrap();
    a.apply(
        page.clone(),
        vec![insert("view", None, BlockKind::View, "original")],
    )
    .unwrap();
    let doc = snapshot(&a, &page);
    doc.set_peer_id(40_000).unwrap();
    let tree = doc.get_tree("blocks");
    tree.enable_fractional_index(0);
    let original = tree.roots()[0];
    let duplicate = tree.create_at(TreeParentId::Root, 1).unwrap();
    let meta = tree.get_meta(duplicate).unwrap();
    meta.insert("id", "view").unwrap();
    meta.insert("kind", "view").unwrap();
    meta.insert_container("text", LoroText::new())
        .unwrap()
        .insert(0, "modified")
        .unwrap();
    doc.commit();
    a.import_updates(page.clone(), doc.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    b.import_updates(
        page.clone(),
        a.export_updates(page.clone(), vec![]).unwrap(),
    )
    .unwrap();

    // Obtain the actual deterministic replacement identity on one replica.
    // The other still has the same old physical source; preexisting imported
    // content can already occupy the identity its repair would normally use.
    let replacement_id = a.load_page(page.clone()).unwrap().blocks[1].id.clone();
    b.apply(
        page.clone(),
        vec![insert(
            &replacement_id,
            None,
            BlockKind::View,
            "already here",
        )],
    )
    .unwrap();
    let occupied = snapshot(&b, &page).get_tree("blocks").roots()[0];
    let opened = b.load_page(page.clone()).unwrap();
    assert_eq!(opened.blocks.len(), 3);
    assert_eq!(opened.blocks[0].id, replacement_id);
    assert_eq!(opened.blocks[0].text, "already here");
    assert_eq!(opened.blocks[1].id, "view");
    assert_eq!(opened.blocks[1].text, "original");
    assert_eq!(opened.blocks[2].text, "modified");
    let ids: std::collections::HashSet<_> = opened.blocks.iter().map(|block| &block.id).collect();
    assert_eq!(
        ids.len(),
        opened.blocks.len(),
        "one open must leave every view identity unique"
    );
    let repaired = snapshot(&b, &page);
    let repaired_tree = repaired.get_tree("blocks");
    assert_eq!(repaired_tree.roots()[0], occupied);
    assert_eq!(repaired_tree.roots()[1], original);
    assert!(repaired_tree.is_node_deleted(&duplicate).unwrap());
    let before = repaired.oplog_vv();
    assert_eq!(b.load_page(page.clone()).unwrap().blocks, opened.blocks);
    assert_eq!(snapshot(&b, &page).oplog_vv(), before);
}

#[test]
fn conflicting_view_copies_keep_all_state_and_children_through_concurrent_repairs() {
    let temp_a = TempVault::new();
    let temp_b = TempVault::new();
    let a = temp_a.open();
    let b = temp_b.open();
    let page = legacy(&a, &non_title_schema(), &[view()]);
    b.import_updates(
        page.clone(),
        a.export_updates(page.clone(), vec![]).unwrap(),
    )
    .unwrap();
    let id = a.load_page(page.clone()).unwrap().blocks[0].id.clone();
    b.load_page(page.clone()).unwrap();
    // Same child logical id on both replicas tests that the copied subtree
    // receives fresh identities and never aliases the retained view's child.
    a.apply(
        page.clone(),
        vec![insert(
            "shared-child",
            Some(&id),
            BlockKind::Todo,
            "Original child",
        )],
    )
    .unwrap();
    let mut modified = view();
    modified.name = "Edited before sync".into();
    b.apply(
        page.clone(),
        vec![
            BlockOp::SetText {
                id: id.clone(),
                text: view_block_text(&modified),
                marks: vec![],
            },
            BlockOp::SetTarget {
                id: id.clone(),
                target: Some("external-source".into()),
            },
            BlockOp::SetColor {
                id: id.clone(),
                color: Some("red_background".into()),
            },
            BlockOp::SetWidth {
                id: id.clone(),
                width: Some(37),
            },
            BlockOp::SetChecked {
                id: id.clone(),
                checked: true,
            },
            insert("shared-child", Some(&id), BlockKind::Todo, "Modified child"),
            BlockOp::SetChecked {
                id: "shared-child".into(),
                checked: true,
            },
            BlockOp::SetText {
                id: "shared-child".into(),
                text: "Modified child".into(),
                marks: vec![Mark {
                    start: 0,
                    end: 8,
                    kind: MarkKind::Bold,
                }],
            },
        ],
    )
    .unwrap();
    exchange(&a, &b, &page);
    let merged = a.load_page(page.clone()).unwrap().blocks;
    let repaired_b = b.load_page(page.clone()).unwrap().blocks;
    assert_eq!(merged, repaired_b);
    assert_eq!(merged.len(), 2);
    assert_ne!(merged[0].id, merged[1].id);
    assert_ne!(merged[0].children[0].id, merged[1].children[0].id);
    let edited = merged
        .iter()
        .find(|block| ViewBlock::def(block).name == modified.name)
        .unwrap();
    assert_eq!(edited.target.as_deref(), Some("external-source"));
    assert_eq!(edited.color.as_deref(), Some("red_background"));
    assert_eq!(edited.width, Some(37));
    assert!(edited.checked);
    assert_eq!(edited.children[0].text, "Modified child");
    assert!(edited.children[0].checked);
    assert_eq!(
        edited.children[0].marks,
        vec![Mark {
            start: 0,
            end: 8,
            kind: MarkKind::Bold
        }]
    );
    // Concurrent repairs create equivalent physical copies. Their stable
    // identities let the next open collapse them instead of multiplying tabs.
    exchange(&a, &b, &page);
    assert_eq!(a.load_page(page.clone()).unwrap().blocks, merged);
    assert_eq!(b.load_page(page.clone()).unwrap().blocks, merged);
    exchange(&a, &b, &page);
    assert_eq!(
        snapshot(&a, &page).get_deep_value(),
        snapshot(&b, &page).get_deep_value()
    );
    let before = snapshot(&a, &page).oplog_vv();
    a.load_page(page.clone()).unwrap();
    assert_eq!(snapshot(&a, &page).oplog_vv(), before);
}

#[test]
fn page_given_properties_now_gets_no_view_block_from_its_saved_view() {
    let temp = TempVault::new();
    let vault = temp.open();
    let page = vault.create_page(None, "Projects".into()).unwrap();
    vault.set_schema(page.clone(), non_title_schema()).unwrap();
    let opened = vault.load_page(page.clone()).unwrap();
    assert_eq!(opened.page_type, PAGE_TYPE_DATABASE);
    assert!(opened.blocks.is_empty());

    let other = vault.create_page(None, "Reading".into()).unwrap();
    vault.make_collection(other.clone()).unwrap();
    assert!(vault.load_page(other).unwrap().blocks.is_empty());
}
