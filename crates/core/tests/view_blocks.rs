//! View payloads and the concurrent first-open migration boundary.

use loro::{ExportMode, LoroDoc, LoroText, TreeParentId};
use serde_json::json;
use vault_core::*;

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
    assert_eq!(serde_json::to_value(BlockKind::View).unwrap(), json!("view"));
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
    a.import(&b.export(ExportMode::all_updates()).unwrap()).unwrap();
    b.import(&a.export(ExportMode::all_updates()).unwrap()).unwrap();

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
