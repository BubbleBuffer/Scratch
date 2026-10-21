//! Compatibility with raw data written by a newer client. These names must
//! remain unknown even when the query-block feature becomes a built-in.
use std::{fs, path::PathBuf};

use loro::{
    Container, ExpandType, ExportMode, LoroDoc, LoroText, LoroValue, StyleConfig, StyleConfigMap,
    TreeID, TreeParentId, ValueOrContainer,
};
use vault_core::{new_id, BlockKind, BlockOp, Vault};

const FUTURE_KIND: &str = "future:query_card";
const FUTURE_ATTR: &str = "future_query";
const BODY: &str = "a😀bé中";

struct TempVault(PathBuf);
impl TempVault {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("query_block_compat_{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}
impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn raw(vault: &Vault, page: &str) -> LoroDoc {
    let doc = LoroDoc::new();
    doc.import(&vault.export_updates(page.into(), vec![]).unwrap())
        .unwrap();
    doc
}

fn node(doc: &LoroDoc, id: &str) -> TreeID {
    let tree = doc.get_tree("blocks");
    tree.roots()
        .into_iter()
        .find(|node| {
            matches!(tree.get_meta(*node).unwrap().get("id"),
            Some(ValueOrContainer::Value(LoroValue::String(s))) if s.as_ref() == id)
        })
        .expect("fixture block")
}

fn text(doc: &LoroDoc, id: &str) -> LoroText {
    match doc
        .get_tree("blocks")
        .get_meta(node(doc, id))
        .unwrap()
        .get("text")
    {
        Some(ValueOrContainer::Container(Container::Text(text))) => text,
        _ => panic!("fixture text container"),
    }
}

fn raw_kind(doc: &LoroDoc, id: &str) -> Vec<u8> {
    match doc
        .get_tree("blocks")
        .get_meta(node(doc, id))
        .unwrap()
        .get("kind")
    {
        Some(ValueOrContainer::Value(LoroValue::String(s))) => s.as_bytes().to_vec(),
        _ => panic!("fixture kind"),
    }
}

fn insert_raw(doc: &LoroDoc, id: &str, kind: &str, body: &str, marked: bool) {
    let tree = doc.get_tree("blocks");
    tree.enable_fractional_index(0);
    let node = tree.create(TreeParentId::Root).unwrap();
    let meta = tree.get_meta(node).unwrap();
    meta.insert("id", id).unwrap();
    meta.insert("kind", kind).unwrap();
    meta.insert("checked", false).unwrap();
    let text = meta.insert_container("text", LoroText::new()).unwrap();
    text.insert(0, body).unwrap();
    if marked {
        text.mark(1..4, FUTURE_ATTR, "opaque-v1").unwrap();
    }
}

fn fixture(vault: &Vault, count: usize) -> String {
    let page = vault.create_page(None, "Compatibility".into()).unwrap();
    let doc = raw(vault, &page);
    doc.set_peer_id(987654).unwrap();
    let mut styles = StyleConfigMap::new();
    styles.insert(
        FUTURE_ATTR.into(),
        StyleConfig {
            expand: ExpandType::After,
        },
    );
    doc.config_text_style(styles);
    insert_raw(&doc, "future", FUTURE_KIND, BODY, true);
    insert_raw(&doc, "paragraph", "paragraph", BODY, true);
    insert_raw(&doc, "other", "paragraph", "before", false);
    for i in 3..count {
        insert_raw(&doc, &format!("padding-{i}"), "paragraph", "padding", false);
    }
    doc.commit();
    vault
        .import_updates(page.clone(), doc.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    page
}

fn attr_ranges(doc: &LoroDoc, id: &str) -> Vec<(usize, usize, String)> {
    let LoroValue::List(items) = text(doc, id).get_richtext_value() else {
        panic!("rich text")
    };
    let mut position = 0;
    let mut ranges = Vec::new();
    for item in items.iter() {
        let LoroValue::Map(item) = item else {
            panic!("rich-text span")
        };
        let Some(LoroValue::String(insert)) = item.get("insert") else {
            panic!("insert span")
        };
        let end = position + insert.chars().count();
        if let Some(LoroValue::Map(attrs)) = item.get("attributes") {
            if let Some(LoroValue::String(value)) = attrs.get(FUTURE_ATTR) {
                ranges.push((position, end, value.to_string()));
            }
        }
        position = end;
    }
    ranges
}

#[test]
fn unknown_kind_and_raw_marks_survive_other_block_edit_sync_and_reopen() {
    let left_dir = TempVault::new();
    let right_dir = TempVault::new();
    let left = left_dir.open();
    let right = right_dir.open();
    let page = fixture(&left, 3);
    let projected = left.load_page(page.clone()).unwrap();
    let future = projected.blocks.iter().find(|b| b.id == "future").unwrap();
    assert_eq!(future.kind, BlockKind::Unknown(FUTURE_KIND.into()));
    assert_eq!(future.text, BODY);
    assert!(
        future.marks.is_empty(),
        "unknown marks are hidden by the old projection"
    );
    let before = raw(&left, &page);
    assert_eq!(
        attr_ranges(&before, "future"),
        vec![(1, 4, "opaque-v1".into())]
    );
    right
        .import_updates(
            page.clone(),
            left.export_updates(page.clone(), vec![]).unwrap(),
        )
        .unwrap();
    right
        .apply(
            page.clone(),
            vec![BlockOp::SetText {
                id: "other".into(),
                text: "after".into(),
                marks: vec![],
            }],
        )
        .unwrap();
    left.import_updates(
        page.clone(),
        right.export_updates(page.clone(), vec![]).unwrap(),
    )
    .unwrap();
    for vault in [&left, &right] {
        let after = raw(vault, &page);
        for id in ["future", "paragraph"] {
            assert_eq!(raw_kind(&after, id), raw_kind(&before, id));
            assert_eq!(
                text(&after, id).to_string().as_bytes(),
                text(&before, id).to_string().as_bytes()
            );
            assert_eq!(
                text(&after, id).get_richtext_value(),
                text(&before, id).get_richtext_value()
            );
        }
        assert_eq!(text(&after, "other").to_string(), "after");
        vault.flush().unwrap();
    }
    drop(left);
    drop(right);
    let reopened = left_dir.open();
    let after = raw(&reopened, &page);
    assert_eq!(raw_kind(&after, "future"), FUTURE_KIND.as_bytes());
    assert_eq!(
        text(&after, "future").get_richtext_value(),
        text(&before, "future").get_richtext_value()
    );
}

#[test]
fn same_paragraph_unicode_edit_shifts_unknown_mark_in_scalar_offsets() {
    let dir = TempVault::new();
    let vault = dir.open();
    let page = fixture(&vault, 3);
    vault
        .apply(
            page.clone(),
            vec![BlockOp::SetText {
                id: "paragraph".into(),
                text: format!("🧪{BODY}"),
                marks: vec![],
            }],
        )
        .unwrap();
    let after = raw(&vault, &page);
    assert_eq!(text(&after, "paragraph").to_string(), format!("🧪{BODY}"));
    assert_eq!(
        attr_ranges(&after, "paragraph"),
        vec![(2, 5, "opaque-v1".into())]
    );
    assert_eq!(
        attr_ranges(&after, "future"),
        vec![(1, 4, "opaque-v1".into())]
    );
    vault.flush().unwrap();
    drop(vault);
    let reopened = dir.open();
    assert_eq!(
        attr_ranges(&raw(&reopened, &page), "paragraph"),
        vec![(2, 5, "opaque-v1".into())]
    );
}

#[test]
fn incompatible_typed_block_cache_falls_back_to_authoritative_loro() {
    let dir = TempVault::new();
    let vault = dir.open();
    let page = fixture(&vault, 300);
    let before = raw(&vault, &page);
    vault.flush().unwrap();
    drop(vault);
    let path = dir.0.join(format!(".vault/docs/{page}.blocks.json"));
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for incompatible_mark in [false, true] {
        let mut cache = original.clone();
        let blocks = cache["blocks"].as_array_mut().unwrap();
        let future = blocks.iter_mut().find(|b| b["id"] == "future").unwrap();
        // Model either newer enum in a derived cache; neither is authoritative.
        if incompatible_mark {
            future["marks"] = serde_json::json!([
                { "start": 1, "end": 4, "kind": "FutureQuery" }
            ]);
        } else {
            future["kind"] = serde_json::json!("FutureQueryCard");
        }
        future["text"] = serde_json::json!("cache must not become authoritative");
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let reopened = dir.open();
        let projected = reopened.load_page(page.clone()).unwrap();
        assert_eq!(projected.blocks.len(), 300);
        let future = projected.blocks.iter().find(|b| b.id == "future").unwrap();
        assert_eq!(future.kind, BlockKind::Unknown(FUTURE_KIND.into()));
        assert_eq!(
            future.text, BODY,
            "fallback failed; unknown mark case: {incompatible_mark}"
        );
        let after = raw(&reopened, &page);
        assert_eq!(raw_kind(&after, "future"), raw_kind(&before, "future"));
        assert_eq!(
            text(&after, "future").get_richtext_value(),
            text(&before, "future").get_richtext_value()
        );
        // End this vault session before replacing its derived cache again.
        drop(reopened);
    }
}
