//! Query cards store specs, while inline query marks decorate searchable labels.
use std::{fs, path::PathBuf};

use loro::{Container, LoroDoc, LoroText, LoroValue, ValueOrContainer};
use vault_core::*;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("notes_query_block_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn insert(parent: Option<&str>, id: &str, kind: BlockKind, text: &str) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(Into::into),
        after: None,
        block: NewBlock {
            id: id.into(),
            kind,
            text: text.into(),
            marks: vec![],
            checked: false,
        },
    }
}

fn marked_insert(id: &str, kind: BlockKind, text: &str, marks: Vec<Mark>) -> BlockOp {
    let mut op = insert(None, id, kind, text);
    let BlockOp::Insert { block, .. } = &mut op else {
        unreachable!()
    };
    block.marks = marks;
    op
}

fn mark(text: &str, kind: MarkKind) -> Mark {
    Mark {
        start: 0,
        end: text.chars().count() as u32,
        kind,
    }
}

fn raw_text(doc: &LoroDoc, id: &str) -> LoroText {
    let tree = doc.get_tree("blocks");
    for node in tree.roots() {
        let meta = tree.get_meta(node).unwrap();
        if let Some(ValueOrContainer::Value(LoroValue::String(value))) = meta.get("id") {
            if value.as_str() == id {
                if let Some(ValueOrContainer::Container(Container::Text(text))) = meta.get("text") {
                    return text;
                }
            }
        }
    }
    panic!("missing text for {id}")
}

fn has_hit(vault: &Vault, query: &str, page: &str) -> bool {
    vault
        .search(query.into(), 20)
        .unwrap()
        .iter()
        .any(|hit| hit.id == page)
}

fn assert_plain_text(text: &LoroText) {
    let LoroValue::List(spans) = text.get_richtext_value() else {
        panic!("rich text list")
    };
    for span in spans.iter() {
        let LoroValue::Map(span) = span else {
            panic!("span map")
        };
        if let Some(attrs) = span.get("attributes") {
            let LoroValue::Map(attrs) = attrs else {
                panic!("attribute map")
            };
            assert!(attrs.is_empty(), "unexpected attributes: {attrs:?}");
        }
    }
}

#[test]
fn query_specs_stay_plain_strings_after_insert_edit_conversion_and_reopen() {
    assert_eq!(BlockKind::Query.as_str(), "query");
    assert_eq!(BlockKind::parse("query"), BlockKind::Query);
    assert_eq!(
        serde_json::to_string(&BlockKind::Query).unwrap(),
        "\"query\""
    );
    assert_eq!(
        serde_json::from_str::<BlockKind>("\"query\"").unwrap(),
        BlockKind::Query
    );
    let temp = Temp::new();
    let page = {
        let vault = temp.open();
        let page = vault.create_page(None, "Cards".into()).unwrap();
        let referenced_page = vault.create_page(None, "Work projects".into()).unwrap();
        let spec = vault
            .parse_search("tag:work in:[[Work projects]] sort:updated".into())
            .unwrap()
            .spec
            .to_query_string();
        assert!(
            spec.contains(&format!("in:[[{referenced_page}]]")),
            "{spec}"
        );
        vault
            .apply(
                page.clone(),
                vec![
                    marked_insert(
                        "card",
                        BlockKind::Query,
                        &spec,
                        vec![mark(&spec, MarkKind::Bold)],
                    ),
                    marked_insert(
                        "converted",
                        BlockKind::Paragraph,
                        "type:document",
                        vec![mark("type:document", MarkKind::Italic)],
                    ),
                    BlockOp::SetKind {
                        id: "converted".into(),
                        kind: BlockKind::Query,
                    },
                ],
            )
            .unwrap();
        let card = vault
            .load_page(page.clone())
            .unwrap()
            .blocks
            .into_iter()
            .find(|b| b.id == "card")
            .unwrap();
        assert_eq!(card.text, spec);
        assert!(card.marks.is_empty());
        let edited = "tag:home\n\"literal 🐈\"";
        vault
            .apply(
                page.clone(),
                vec![BlockOp::SetText {
                    id: "card".into(),
                    text: edited.into(),
                    marks: vec![mark(edited, MarkKind::Underline)],
                }],
            )
            .unwrap();
        // Changing back must not restore rich-text attributes hidden by Query.
        vault
            .apply(
                page.clone(),
                vec![BlockOp::SetKind {
                    id: "converted".into(),
                    kind: BlockKind::Paragraph,
                }],
            )
            .unwrap();
        let doc = LoroDoc::new();
        doc.import(&vault.export_updates(page.clone(), vec![]).unwrap())
            .unwrap();
        assert_eq!(raw_text(&doc, "card").to_string(), edited);
        assert_plain_text(&raw_text(&doc, "card"));
        assert_plain_text(&raw_text(&doc, "converted"));
        vault.flush().unwrap();
        page
    };
    let vault = temp.open();
    let blocks = vault.load_page(page).unwrap().blocks;
    let card = blocks.iter().find(|b| b.id == "card").unwrap();
    assert_eq!(card.kind, BlockKind::Query);
    assert_eq!(card.text, "tag:home\n\"literal 🐈\"");
    assert!(card.children.is_empty());
    assert!(blocks.iter().all(|b| b.marks.is_empty()));
}

#[test]
fn inline_query_uses_raw_query_attribute_and_unicode_mark_ranges() {
    let temp = Temp::new();
    let vault = temp.open();
    let page = vault.create_page(None, "Chips".into()).unwrap();
    let label = "🐈 open work";
    let spec = "tag:work in:[[stable-page-id]]";
    let query_mark = mark(label, MarkKind::Query(spec.into()));
    vault
        .apply(
            page.clone(),
            vec![marked_insert(
                "chip",
                BlockKind::Paragraph,
                label,
                vec![
                    query_mark.clone(),
                    mark(label, MarkKind::PageLink("stable-page-id".into())),
                ],
            )],
        )
        .unwrap();
    let block = &vault.load_page(page.clone()).unwrap().blocks[0];
    assert_eq!(
        block.marks,
        vec![
            mark(label, MarkKind::PageLink("stable-page-id".into())),
            query_mark.clone()
        ]
    );
    let raw = LoroDoc::new();
    raw.import(&vault.export_updates(page.clone(), vec![]).unwrap())
        .unwrap();
    let value = raw_text(&raw, "chip").get_richtext_value();
    let LoroValue::List(spans) = value else {
        panic!("rich text list")
    };
    let LoroValue::Map(span) = &spans[0] else {
        panic!("span map")
    };
    let Some(LoroValue::Map(attrs)) = span.get("attributes") else {
        panic!("attributes")
    };
    assert_eq!(attrs.get("query"), Some(&LoroValue::from(spec)));
    assert!(!attrs.contains_key("Query"));
    // Replacing only the query value preserves the ordinary label and range.
    let replacement = mark(label, MarkKind::Query("type:document".into()));
    vault
        .apply(
            page.clone(),
            vec![BlockOp::SetText {
                id: "chip".into(),
                text: label.into(),
                marks: vec![replacement.clone()],
            }],
        )
        .unwrap();
    vault.flush().unwrap();
    drop(vault);
    let vault = temp.open();
    let block = &vault.load_page(page.clone()).unwrap().blocks[0];
    assert_eq!(block.text, label);
    assert_eq!(block.marks, vec![replacement]);
    vault
        .apply(
            page.clone(),
            vec![BlockOp::SetText {
                id: "chip".into(),
                text: label.into(),
                marks: vec![],
            }],
        )
        .unwrap();
    assert!(vault.load_page(page).unwrap().blocks[0].marks.is_empty());
}

#[test]
fn search_indexes_query_labels_but_never_query_specs_after_sync_and_reindex() {
    let source = Temp::new();
    let target = Temp::new();
    let vault = source.open();
    let receiver = target.open();
    let page = vault.create_page(None, "Host".into()).unwrap();
    vault
        .apply(
            page.clone(),
            vec![
                insert(None, "card", BlockKind::Query, "cardsecretneedle"),
                marked_insert(
                    "chip",
                    BlockKind::Paragraph,
                    "visiblelabelneedle",
                    vec![mark(
                        "visiblelabelneedle",
                        MarkKind::Query("chipsecretneedle".into()),
                    )],
                ),
            ],
        )
        .unwrap();
    vault.flush().unwrap();
    receiver
        .import_updates(
            page.clone(),
            vault.export_updates(page.clone(), vec![]).unwrap(),
        )
        .unwrap();
    receiver.flush().unwrap();
    for current in [&vault, &receiver] {
        assert!(has_hit(current, "visiblelabelneedle", &page));
        assert!(!has_hit(current, "cardsecretneedle", &page));
        assert!(!has_hit(current, "chipsecretneedle", &page));
        current.reindex().unwrap();
        assert!(has_hit(current, "visiblelabelneedle", &page));
        assert!(!has_hit(current, "cardsecretneedle", &page));
        assert!(!has_hit(current, "chipsecretneedle", &page));
        let blocks = current.load_page(page.clone()).unwrap().blocks;
        assert!(blocks
            .iter()
            .any(|b| b.kind == BlockKind::Query && b.text == "cardsecretneedle"));
        assert!(blocks.iter().any(|b| b.marks
            == vec![mark(
                "visiblelabelneedle",
                MarkKind::Query("chipsecretneedle".into())
            )]));
    }
}

#[test]
fn opening_a_previous_index_rebuilds_stale_query_spec_search_text() {
    let temp = Temp::new();
    let page = {
        let vault = temp.open();
        let page = vault.create_page(None, "Host".into()).unwrap();
        vault
            .apply(
                page.clone(),
                vec![
                    insert(None, "card", BlockKind::Query, "legacyqueryspecneedle"),
                    marked_insert(
                        "chip",
                        BlockKind::Paragraph,
                        "visiblelabelneedle",
                        vec![mark(
                            "visiblelabelneedle",
                            MarkKind::Query("chipsecretneedle".into()),
                        )],
                    ),
                ],
            )
            .unwrap();
        vault.flush().unwrap();
        page
    };
    let index_path = temp.0.join(".vault/index.sqlite");
    {
        let index = rusqlite::Connection::open(&index_path).unwrap();
        // A previous client preserved the raw `query` block kind but indexed
        // its text as an ordinary unknown block. The documents are unchanged.
        index
            .execute(
                "UPDATE fts SET body = ?1 WHERE rowid = (SELECT rowid FROM pages WHERE id = ?2)",
                rusqlite::params!["legacyqueryspecneedle\nvisiblelabelneedle", page],
            )
            .unwrap();
        index.pragma_update(None, "user_version", 10).unwrap();
        let stale_matches: i64 = index
            .query_row(
                "SELECT count(*) FROM fts WHERE fts MATCH 'legacyqueryspecneedle'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale_matches, 1, "fixture must contain the old self-match");
    }

    // Opening the upgraded core repairs the existing row without an edit,
    // explicit reindex call, or missing-row recovery.
    let vault = temp.open();
    assert!(!has_hit(&vault, "legacyqueryspecneedle", &page));
    assert!(has_hit(&vault, "visiblelabelneedle", &page));
    assert!(!has_hit(&vault, "chipsecretneedle", &page));
    let blocks = vault.load_page(page).unwrap().blocks;
    assert!(blocks
        .iter()
        .any(|b| b.kind == BlockKind::Query && b.text == "legacyqueryspecneedle"));
    assert!(blocks.iter().any(|b| b.marks
        == vec![mark(
            "visiblelabelneedle",
            MarkKind::Query("chipsecretneedle".into())
        )]));
    let rebuilt_version: i64 = rusqlite::Connection::open(index_path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(rebuilt_version, 14);
}

#[test]
fn query_children_are_rejected_without_partial_batch_changes() {
    let temp = Temp::new();
    let vault = temp.open();
    let page = vault.create_page(None, "Host".into()).unwrap();
    vault
        .apply(
            page.clone(),
            vec![
                insert(None, "card", BlockKind::Query, "tag:work"),
                insert(None, "parent", BlockKind::Paragraph, "Parent"),
                insert(Some("parent"), "child", BlockKind::Paragraph, "Preserve me"),
            ],
        )
        .unwrap();
    let before = vault.load_page(page.clone()).unwrap().blocks;
    for invalid in [
        insert(Some("card"), "new-child", BlockKind::Paragraph, "Rejected"),
        BlockOp::Move {
            id: "child".into(),
            parent: Some("card".into()),
            after: None,
        },
        BlockOp::SetKind {
            id: "parent".into(),
            kind: BlockKind::Query,
        },
    ] {
        assert!(vault
            .apply(
                page.clone(),
                vec![
                    BlockOp::SetText {
                        id: "card".into(),
                        text: "changed".into(),
                        marks: vec![]
                    },
                    invalid,
                ]
            )
            .is_err());
        assert_eq!(vault.load_page(page.clone()).unwrap().blocks, before);
    }
    // The parent may have been inserted and changed kind earlier in this batch.
    assert!(vault
        .apply(
            page.clone(),
            vec![
                insert(None, "new-parent", BlockKind::Paragraph, "tag:home"),
                BlockOp::SetKind {
                    id: "new-parent".into(),
                    kind: BlockKind::Query
                },
                insert(
                    Some("new-parent"),
                    "new-child",
                    BlockKind::Paragraph,
                    "Rejected"
                ),
            ]
        )
        .is_err());
    assert_eq!(vault.load_page(page).unwrap().blocks, before);
}

#[test]
fn query_conversion_checks_children_at_the_end_of_the_batch() {
    let temp = Temp::new();
    let vault = temp.open();
    let page = vault.create_page(None, "Host".into()).unwrap();
    vault
        .apply(
            page.clone(),
            vec![
                insert(None, "delete-parent", BlockKind::Paragraph, "tag:work"),
                insert(
                    Some("delete-parent"),
                    "deleted",
                    BlockKind::Paragraph,
                    "Delete me",
                ),
                insert(None, "move-parent", BlockKind::Paragraph, "tag:home"),
                insert(
                    Some("move-parent"),
                    "moved",
                    BlockKind::Paragraph,
                    "Keep me",
                ),
            ],
        )
        .unwrap();
    vault
        .apply(
            page.clone(),
            vec![
                BlockOp::SetKind {
                    id: "delete-parent".into(),
                    kind: BlockKind::Query,
                },
                BlockOp::Delete {
                    id: "deleted".into(),
                },
                BlockOp::SetKind {
                    id: "move-parent".into(),
                    kind: BlockKind::Query,
                },
                BlockOp::Move {
                    id: "moved".into(),
                    parent: None,
                    after: None,
                },
            ],
        )
        .unwrap();
    let blocks = vault.load_page(page).unwrap().blocks;
    assert_eq!(blocks.len(), 3);
    assert!(blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Query)
        .all(|b| b.children.is_empty()));
    assert!(blocks
        .iter()
        .any(|b| b.id == "moved" && b.text == "Keep me"));
}
