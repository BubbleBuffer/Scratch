//! Preservation of block kinds that this version cannot render natively.
use std::{fs, path::PathBuf};
use vault_core::*;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("notes_unknown_{}", new_id())))
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

fn insert(v: &Vault, page: &str, kind: BlockKind, text: &str, target: Option<&str>) {
    v.apply(
        page.into(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: "future".into(),
                    kind,
                    text: text.into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: "future".into(),
                target: target.map(Into::into),
            },
        ],
    )
    .unwrap();
}

#[test]
fn unknown_kind_survives_reopen_sync_and_search() {
    let (ta, tb) = (Temp::new(), Temp::new());
    let raw = "mod:future kind 雪 -->";
    let target = "{\"path\":\"target with spaces -->\"}";
    let page = {
        let a = ta.open();
        let page = a.create_page(None, "Future".into()).unwrap();
        insert(
            &a,
            &page,
            BlockKind::Unknown(raw.into()),
            "quasar fallback",
            Some(target),
        );
        a.flush().unwrap();
        page
    };
    let a = ta.open();
    let b = tb.open();
    b.import_updates_batch(vec![PageUpdate {
        page: page.clone(),
        bytes: a.export_updates(page.clone(), vec![]).unwrap(),
    }])
    .unwrap();
    let at_b = b.load_page(page.clone()).unwrap();
    assert_eq!(at_b.blocks[0].kind, BlockKind::Unknown(raw.into()));
    assert_eq!(at_b.blocks[0].target.as_deref(), Some(target));
    assert_eq!(at_b.blocks[0].text, "quasar fallback");
    b.apply(
        page.clone(),
        vec![BlockOp::SetText {
            id: "future".into(),
            text: "quasar edited".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    let have = a
        .page_versions()
        .unwrap()
        .into_iter()
        .find(|v| v.page == page)
        .unwrap()
        .version;
    a.import_updates_batch(vec![PageUpdate {
        page: page.clone(),
        bytes: b.export_updates(page.clone(), have).unwrap(),
    }])
    .unwrap();
    a.flush().unwrap();
    b.flush().unwrap();
    drop(a);
    drop(b);
    for v in [ta.open(), tb.open()] {
        let loaded = v.load_page(page.clone()).unwrap();
        let block = &loaded.blocks[0];
        assert_eq!(block.kind, BlockKind::Unknown(raw.into()));
        assert_eq!(block.target.as_deref(), Some(target));
        assert_eq!(block.text, "quasar edited");
        assert!(v
            .search("quasar".into(), 10)
            .unwrap()
            .iter()
            .any(|hit| hit.id == page));
    }
}

#[test]
fn generic_markdown_preserves_unknown_and_chat_ref_with_readable_fallback() {
    let temp = Temp::new();
    let v = temp.open();
    for kind in [
        BlockKind::Unknown("mod:snow 雪 --> kind".into()),
        BlockKind::ChatRef,
    ] {
        for text in [
            "",
            "# Heading-looking fallback",
            "line one\n\n- line two\n",
            "  spaced  ",
            "a **literal** <tag> \\ ",
            "summary 雪",
        ] {
            for target in [
                None,
                Some(""),
                Some("{\"summary\":\"spaces --> 雪\",\"lines\":\"a\\nb\"}"),
            ] {
                let source = Block {
                    id: "ignored".into(),
                    kind: kind.clone(),
                    text: text.into(),
                    marks: vec![],
                    checked: true,
                    target: target.map(Into::into),
                    color: Some("red".into()),
                    width: Some(50),
                    children: vec![],
                };
                let md = v.blocks_markdown(vec![source.clone()]);
                assert!(md.contains("<!-- block="), "{md}");
                let parsed = v.parse_markdown(md);
                assert_eq!(parsed.len(), 1);
                let b = &parsed[0];
                assert_eq!(b.kind, source.kind);
                assert_eq!(b.target, source.target);
                assert_eq!(b.text, source.text);
                assert_eq!(b.checked, source.checked);
                assert_eq!(b.color, source.color);
                assert_eq!(b.width, source.width);
            }
        }
    }
}

#[test]
fn generic_markdown_keeps_children_marks_and_adjacent_blocks() {
    let temp = Temp::new();
    let v = temp.open();
    let p = v.create_page(None, "Mixed".into()).unwrap();
    v.insert_markdown(p.clone(), None, None, true, "before".into())
        .unwrap();
    insert(
        &v,
        &p,
        BlockKind::Unknown("mod:diagram".into()),
        "bold\n\nafter blank",
        Some("target"),
    );
    v.apply(
        p.clone(),
        vec![
            BlockOp::SetText {
                id: "future".into(),
                text: "bold\n\nafter blank".into(),
                marks: vec![Mark {
                    start: 0,
                    end: 4,
                    kind: MarkKind::Bold,
                }],
            },
            BlockOp::Insert {
                parent: Some("future".into()),
                after: None,
                block: NewBlock {
                    id: "kid".into(),
                    kind: BlockKind::Unknown("mod:child".into()),
                    text: "child".into(),
                    marks: vec![],
                    checked: false,
                },
            },
        ],
    )
    .unwrap();
    v.insert_markdown(p.clone(), None, None, true, "after".into())
        .unwrap();
    let original = v.load_page(p.clone()).unwrap();
    let markdown = v.page_markdown(p).unwrap();
    let parsed = v.parse_markdown(markdown);
    assert_eq!(parsed.len(), original.blocks.len());
    for (a, b) in original.blocks.iter().zip(&parsed) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.text, b.text);
        assert_eq!(a.target, b.target);
        assert_eq!(a.marks, b.marks);
        assert_eq!(a.children.len(), b.children.len());
        for (ac, bc) in a.children.iter().zip(&b.children) {
            assert_eq!(ac.kind, bc.kind);
            assert_eq!(ac.text, bc.text);
        }
    }
}

#[test]
fn generic_blocks_survive_markdown_file_export_and_import() {
    let (source_dir, destination_dir) = (Temp::new(), Temp::new());
    let source = source_dir.open();
    for (title, kind) in [
        ("Future", BlockKind::Unknown("mod:future".into())),
        ("Chat", BlockKind::ChatRef),
    ] {
        let page = source.create_page(None, title.into()).unwrap();
        insert(
            &source,
            &page,
            kind,
            "Readable fallback\n\nSecond line",
            Some("payload --> 雪"),
        );
    }
    source.flush().unwrap();
    let destination = destination_dir.open();
    let report = destination
        .import_markdown(
            source_dir.0.join("pages").to_string_lossy().into_owned(),
            None,
        )
        .unwrap();
    assert_eq!(report.pages, 2);
    for page in report.top_pages {
        let loaded = destination.load_page(page).unwrap();
        let expected = if loaded.title == "Chat" {
            BlockKind::ChatRef
        } else {
            BlockKind::Unknown("mod:future".into())
        };
        assert_eq!(loaded.blocks[0].kind, expected);
        assert_eq!(loaded.blocks[0].target.as_deref(), Some("payload --> 雪"));
        assert_eq!(loaded.blocks[0].text, "Readable fallback\n\nSecond line");
    }
}
