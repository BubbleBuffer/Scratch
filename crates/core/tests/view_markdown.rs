//! Complete page transport and derived search behavior for contents views.

use std::{fs, path::PathBuf};
use vault_core::*;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("view_markdown_{}", new_id())))
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

fn settings() -> ViewDef {
    ViewDef {
        id: "ignored-settings-id".into(),
        name: "Children".into(),
        kind: ViewKind::Table,
        filter: Some(Filter::And(vec![])),
        sorts: vec![Sort {
            prop: "$title".into(),
            descending: true,
        }],
        group_by: None,
        date_prop: None,
        visible_props: vec!["$title".into()],
        card_cover: None,
        config: r#"{"custom":"viewconfigneedle"}"#.into(),
    }
}

fn insert(v: &Vault, page: &str, id: &str, after: Option<&str>, target: Option<&str>) {
    v.apply(
        page.into(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: after.map(str::to_owned),
                block: NewBlock {
                    id: id.into(),
                    kind: BlockKind::View,
                    text: view_block_text(&settings()),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: id.into(),
                target: target.map(str::to_owned),
            },
            BlockOp::SetColor {
                id: id.into(),
                color: Some("blue_background".into()),
            },
        ],
    )
    .unwrap();
}

fn without_ids(blocks: &mut [Block]) {
    for block in blocks {
        block.id.clear();
        without_ids(&mut block.children);
    }
}

#[test]
fn full_page_export_reimport_preserves_view_blocks() {
    let (source_dir, export_dir, destination_dir) = (Temp::new(), Temp::new(), Temp::new());
    let source = source_dir.open();
    let page = source.create_page(None, "Contents".into()).unwrap();
    insert(&source, &page, "self-view", None, None);
    insert(
        &source,
        &page,
        "linked-view",
        Some("self-view"),
        Some("external-source"),
    );
    source
        .apply(
            page.clone(),
            vec![BlockOp::Insert {
                parent: Some("self-view".into()),
                after: None,
                block: NewBlock {
                    id: "child".into(),
                    kind: BlockKind::Paragraph,
                    text: "View note".into(),
                    marks: vec![],
                    checked: false,
                },
            }],
        )
        .unwrap();
    let mut original = source.load_page(page.clone()).unwrap().blocks;
    let exported = source.export_markdown(page).unwrap();
    assert!(exported.contains("```view"));
    assert!(exported.contains(r#""page":"external-source""#));
    fs::create_dir_all(&export_dir.0).unwrap();
    fs::write(export_dir.0.join("Contents.md"), &exported).unwrap();
    let destination = destination_dir.open();
    let report = destination
        .import_markdown(export_dir.0.to_string_lossy().into_owned(), None)
        .unwrap();
    assert_eq!(report.pages, 1);
    let imported = destination.load_page(report.top_pages[0].clone()).unwrap();
    assert_eq!(imported.title, "Contents");
    assert!(imported.blocks.iter().all(|b| !b.id.is_empty()));
    let mut restored = imported.blocks;
    // Markdown import intentionally assigns fresh block identities to all kinds.
    // Every persisted block field other than those new identities must match.
    without_ids(&mut original);
    without_ids(&mut restored);
    assert_eq!(restored, original);
}

#[test]
fn importing_source_and_host_remaps_the_view_source_page() {
    let (source_dir, export_dir, destination_dir) = (Temp::new(), Temp::new(), Temp::new());
    let source = source_dir.open();
    let contents = source.create_page(None, "Source".into()).unwrap();
    let host = source.create_page(None, "Host".into()).unwrap();
    insert(&source, &host, "view", None, Some(&contents));
    fs::create_dir_all(&export_dir.0).unwrap();
    for (id, name) in [(&contents, "Source.md"), (&host, "Host.md")] {
        fs::write(
            export_dir.0.join(name),
            source.export_markdown(id.clone()).unwrap(),
        )
        .unwrap();
    }
    let destination = destination_dir.open();
    destination
        .import_markdown(export_dir.0.to_string_lossy().into_owned(), None)
        .unwrap();
    let pages = destination.list_pages(None, 0, 100).unwrap();
    let target = &pages.iter().find(|p| p.title == "Source").unwrap().id;
    let imported_host = &pages.iter().find(|p| p.title == "Host").unwrap().id;
    assert_ne!(target, &contents);
    assert_eq!(
        destination.load_page(imported_host.clone()).unwrap().blocks[0]
            .target
            .as_ref(),
        Some(target)
    );
}

#[test]
fn view_settings_are_not_searchable_after_write_or_index_upgrade() {
    let dir = Temp::new();
    let page;
    {
        let v = dir.open();
        page = v.create_page(None, "Contents".into()).unwrap();
        insert(&v, &page, "view", None, None);
        v.flush().unwrap();
        assert!(v.search("viewconfigneedle".into(), 10).unwrap().is_empty());
        assert!(v
            .search("Contents".into(), 10)
            .unwrap()
            .iter()
            .any(|h| h.id == page));
    }
    // Reproduce a prior index that stored the settings as body text.
    let db = rusqlite::Connection::open(dir.0.join(".vault/index.sqlite")).unwrap();
    db.execute("UPDATE fts SET body = 'viewconfigneedle' WHERE rowid = (SELECT rowid FROM pages WHERE id = ?1)", [&page]).unwrap();
    db.pragma_update(None, "user_version", 13).unwrap();
    drop(db);
    let v = dir.open();
    v.flush().unwrap();
    assert!(v.search("viewconfigneedle".into(), 10).unwrap().is_empty());
    assert!(v
        .search("Contents".into(), 10)
        .unwrap()
        .iter()
        .any(|h| h.id == page));
}
