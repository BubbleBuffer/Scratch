use loro::{ExportMode, LoroDoc, ToJson};
use std::{fs, path::PathBuf};
use vault_core::*;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("notes_mdnumbered_{}", new_id())))
    }

    fn vault(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn numbered_list_non_one_start_round_trips_byte_identically() {
    let temp = Temp::new();
    let vault = temp.vault();
    let markdown = "3. a\n4. b\n";

    let blocks = vault.parse_markdown(markdown.into());
    assert_eq!(blocks.len(), 2);
    assert!(blocks.iter().all(|block| block.kind == BlockKind::Numbered));
    assert_eq!(blocks[0].numbered_start, Some(3));
    assert_eq!(blocks[1].numbered_start, None);
    // The blocks API omits surrounding newlines; a Markdown file ends in one.
    assert_eq!(format!("{}\n", vault.blocks_markdown(blocks)), markdown);
}

#[test]
fn numbered_list_one_start_round_trips_byte_identically() {
    let temp = Temp::new();
    let vault = temp.vault();
    let markdown = "1. a\n2. b\n";

    let blocks = vault.parse_markdown(markdown.into());
    assert!(blocks.iter().all(|block| block.numbered_start.is_none()));
    let serialized = serde_json::to_value(&blocks).unwrap();
    assert!(serialized.as_array().unwrap().iter().all(|block| block.get("numbered_start").is_none()));
    let rendered = vault.blocks_markdown(blocks);
    assert_eq!(format!("{rendered}\n"), markdown);
}

#[test]
fn legacy_serialized_document_without_numbered_start_still_deserializes() {
    let block: Block = serde_json::from_str(
        r#"{"id":"legacy","kind":"Numbered","text":"a","marks":[],"checked":false,"children":[]}"#,
    )
    .unwrap();
    let temp = Temp::new();
    let vault = temp.vault();

    assert_eq!(block.kind, BlockKind::Numbered);
    assert_eq!(block.numbered_start, None);
    assert_eq!(vault.blocks_markdown(vec![block]), "1. a");
}

fn raw_page(vault: &Vault, page: &str) -> LoroDoc {
    let raw = LoroDoc::new();
    raw.import(&vault.export_updates(page.into(), vec![]).unwrap())
        .unwrap();
    raw
}

#[test]
fn loro_numbered_start_raw_map_survives_read_and_text_edit() {
    let temp = Temp::new();
    let vault = temp.vault();
    let page = vault.create_page(None, "Numbered".into()).unwrap();
    let ids = vault
        .insert_markdown(page.clone(), None, None, true, "1. a\n2. b\n".into())
        .unwrap();
    let raw = raw_page(&vault, &page);
    let tree = raw.get_tree("blocks");
    let first = tree.roots()[0];
    tree.get_meta(first)
        .unwrap()
        .insert("numbered_start", 3_i64)
        .unwrap();
    raw.commit();
    vault
        .import_updates(page.clone(), raw.export(ExportMode::Snapshot).unwrap())
        .unwrap();

    assert_eq!(vault.load_page(page.clone()).unwrap().blocks[0].numbered_start, Some(3));
    assert_eq!(vault.page_markdown(page.clone()).unwrap(), "3. a\n4. b");
    vault
        .apply(
            page.clone(),
            vec![BlockOp::SetText {
                id: ids[0].clone(),
                text: "edited".into(),
                marks: vec![],
            }],
        )
        .unwrap();
    let exported = raw_page(&vault, &page);
    let metadata = exported.get_tree("blocks").get_meta(first).unwrap();
    assert_eq!(metadata.get_value().to_json_value()["numbered_start"], 3);
    assert_eq!(vault.page_markdown(page).unwrap(), "3. edited\n4. b");
}

#[test]
fn numbered_list_nested_and_separate_runs_keep_their_own_starts() {
    let temp = Temp::new();
    let vault = temp.vault();
    let markdown = "9. parent\n   4. child\n   5. next child\n10. sibling\n\nparagraph\n\n6. reset\n7. next\n";
    let blocks = vault.parse_markdown(markdown.into());

    assert_eq!(blocks[0].numbered_start, Some(9));
    assert_eq!(blocks[0].children[0].numbered_start, Some(4));
    assert_eq!(blocks[0].children[1].numbered_start, None);
    assert_eq!(blocks[1].numbered_start, None);
    assert_eq!(blocks[3].numbered_start, Some(6));
    assert_eq!(blocks[4].numbered_start, None);
    assert_eq!(format!("{}\n", vault.blocks_markdown(blocks)), markdown);
}

#[test]
fn numbered_list_only_first_marker_sets_start_including_zero() {
    let temp = Temp::new();
    let vault = temp.vault();
    for (markdown, expected) in [
        ("3. a\n99. b\n", "3. a\n4. b"),
        ("1. a\n99. b\n", "1. a\n2. b"),
        ("0. a\n1. b\n", "0. a\n1. b"),
    ] {
        let blocks = vault.parse_markdown(markdown.into());
        assert_eq!(blocks[1].numbered_start, None);
        assert_eq!(vault.blocks_markdown(blocks), expected);
    }
}

#[test]
fn loro_numbered_start_is_written_only_on_first_non_one_item_and_reopens() {
    let temp = Temp::new();
    let page = {
        let vault = temp.vault();
        let page = vault.create_page(None, "Numbered".into()).unwrap();
        vault
            .insert_markdown(page.clone(), None, None, true, "3. a\n4. b\n".into())
            .unwrap();
        let raw = raw_page(&vault, &page);
        let tree = raw.get_tree("blocks");
        let roots = tree.roots();
        assert_eq!(tree.get_meta(roots[0]).unwrap().get_value().to_json_value()["numbered_start"], 3);
        assert!(tree.get_meta(roots[1]).unwrap().get("numbered_start").is_none());
        vault.flush().unwrap();
        page
    };
    let reopened = temp.vault();
    let blocks = reopened.load_page(page.clone()).unwrap().blocks;
    assert_eq!(blocks[0].numbered_start, Some(3));
    assert_eq!(blocks[1].numbered_start, None);
    assert_eq!(reopened.page_markdown(page).unwrap(), "3. a\n4. b");
}

#[test]
fn loro_legacy_numbered_blocks_without_start_read_as_one_and_keep_unknown_keys() {
    let temp = Temp::new();
    let page = {
        let vault = temp.vault();
        let page = vault.create_page(None, "Legacy".into()).unwrap();
        vault
            .apply(page.clone(), vec![BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: "legacy".into(), kind: BlockKind::Numbered, text: "a".into(),
                    marks: vec![], checked: false,
                },
            }])
            .unwrap();
        let raw = raw_page(&vault, &page);
        let tree = raw.get_tree("blocks");
        let meta = tree.get_meta(tree.roots()[0]).unwrap();
        assert!(meta.get("numbered_start").is_none());
        meta.insert("future_list_option", "kept").unwrap();
        raw.commit();
        vault.import_updates(page.clone(), raw.export(ExportMode::Snapshot).unwrap()).unwrap();
        vault.apply(page.clone(), vec![BlockOp::SetText {
            id: "legacy".into(), text: "edited".into(), marks: vec![],
        }]).unwrap();
        vault.flush().unwrap();
        page
    };
    let vault = temp.vault();
    assert_eq!(vault.load_page(page.clone()).unwrap().blocks[0].numbered_start, None);
    assert_eq!(vault.page_markdown(page.clone()).unwrap(), "1. edited");
    let raw = raw_page(&vault, &page);
    let tree = raw.get_tree("blocks");
    let meta = tree.get_meta(tree.roots()[0]).unwrap();
    assert!(meta.get("numbered_start").is_none());
    assert_eq!(meta.get_value().to_json_value()["future_list_option"], "kept");
}

#[test]
fn numbered_start_changes_and_clears_through_markdown_diff_without_replacing_ids() {
    let temp = Temp::new();
    let vault = temp.vault();
    let page = vault.create_page(None, "Edits".into()).unwrap();
    let ids = vault.insert_markdown(page.clone(), None, None, true, "3. a\n4. b\n".into()).unwrap();
    for markdown in ["7. a\n8. b", "1. a\n2. b"] {
        let changed = vault.replace_markdown(page.clone(), vec![], markdown.into()).unwrap();
        assert_eq!(changed, ids);
        assert_eq!(vault.page_markdown(page.clone()).unwrap(), markdown);
    }
    let raw = raw_page(&vault, &page);
    let tree = raw.get_tree("blocks");
    assert!(tree.get_meta(tree.roots()[0]).unwrap().get("numbered_start").is_none());
}

#[test]
fn numbered_start_survives_import_duplicate_and_template_copy() {
    let temp = Temp::new();
    let vault = temp.vault();
    let source = temp.0.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("List.md"), "3. a\n4. b\n").unwrap();
    vault.import_markdown(source.to_string_lossy().into_owned(), None).unwrap();
    let page = vault.list_pages(None, 0, 100).unwrap().into_iter().find(|page| page.title == "List").unwrap().id;
    assert_eq!(vault.page_markdown(page.clone()).unwrap(), "3. a\n4. b");
    let duplicate = vault.duplicate_page(page.clone()).unwrap();
    assert_eq!(vault.page_markdown(duplicate).unwrap(), "3. a\n4. b");

    let template = vault.create_page(Some(page.clone()), "Template".into()).unwrap();
    vault.insert_markdown(template.clone(), None, None, true, "6. c\n7. d\n".into()).unwrap();
    vault.set_template(template.clone(), true).unwrap();
    let target = vault.create_page_from(page, "From template".into(), Some(template)).unwrap();
    assert_eq!(vault.page_markdown(target).unwrap(), "6. c\n7. d");
}
