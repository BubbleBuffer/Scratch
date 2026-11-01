//! Block kinds, block colours and inline marks added for the block
//! editor: stored in the document, written to markdown and read back.

use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_core_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn nb(id: &str, kind: BlockKind, text: &str) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind,
        text: text.into(),
        marks: vec![],
        checked: false,
    }
}

fn ins(parent: Option<&str>, after: Option<&str>, block: NewBlock) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(Into::into),
        after: after.map(Into::into),
        block,
    }
}

#[test]
fn conditional_block_edit_preserves_children_added_since_the_snapshot() {
    let dir = TempDir::new("conditional_blocks");
    let v = dir.open();
    let page = v.create_page(None, "Work".into()).unwrap();
    let empty = v.load_page(page.clone()).unwrap().blocks;
    v.apply_if_blocks_match(
        page.clone(),
        &empty,
        vec![ins(None, None, nb("owned", BlockKind::Todo, "Read"))],
    )
    .unwrap();
    let before_user_edit = v.load_page(page.clone()).unwrap().blocks;
    v.apply(
        page.clone(),
        vec![ins(
            Some("owned"),
            None,
            nb("user", BlockKind::Paragraph, "Keep my detail"),
        )],
    )
    .unwrap();
    let with_user_child = v.load_page(page.clone()).unwrap().blocks;
    assert!(v
        .apply_if_blocks_match(
            page.clone(),
            &before_user_edit,
            vec![BlockOp::Delete { id: "owned".into() }],
        )
        .is_err());
    assert_eq!(v.load_page(page).unwrap().blocks, with_user_child);
}

fn md_path(t: &TempDir, id: &str) -> PathBuf {
    fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
        .expect("markdown file")
}

fn mark(start: u32, end: u32, kind: MarkKind) -> Mark {
    Mark { start, end, kind }
}

/// A page using every new kind, a colour and the new marks.
fn build(v: &Vault) -> PageId {
    let p = v.create_page(None, "Rich".into()).unwrap();
    let mut text = nb("p", BlockKind::Paragraph, "under red due Oct 1");
    text.marks = vec![
        mark(0, 5, MarkKind::Underline),
        mark(6, 9, MarkKind::Color("red".into())),
        mark(14, 19, MarkKind::Date("2026-10-01".into())),
    ];
    v.apply(
        p.clone(),
        vec![
            ins(None, None, nb("t", BlockKind::Toggle, "Details")),
            ins(Some("t"), None, nb("t1", BlockKind::Paragraph, "hidden")),
            ins(None, Some("t"), nb("h", BlockKind::ToggleHeading2, "Section")),
            ins(Some("h"), None, nb("h1", BlockKind::Bulleted, "inside")),
            ins(None, Some("h"), nb("c", BlockKind::Callout, "Note this")),
            BlockOp::SetTarget {
                id: "c".into(),
                target: Some("💡".into()),
            },
            BlockOp::SetColor {
                id: "c".into(),
                color: Some("yellow_background".into()),
            },
            ins(None, Some("c"), nb("cl", BlockKind::ColumnList, "")),
            ins(Some("cl"), None, nb("k1", BlockKind::Column, "")),
            ins(Some("k1"), None, nb("k1a", BlockKind::Paragraph, "left")),
            ins(Some("cl"), Some("k1"), nb("k2", BlockKind::Column, "")),
            ins(Some("k2"), None, nb("k2a", BlockKind::Paragraph, "right")),
            ins(None, Some("cl"), nb("toc", BlockKind::TableOfContents, "")),
            ins(None, Some("toc"), text),
            BlockOp::SetColor {
                id: "p".into(),
                color: Some("blue".into()),
            },
        ],
    )
    .unwrap();
    p
}

/// (id, kind, text, target, color, depth) of one block.
type Shape = (String, BlockKind, String, Option<String>, Option<String>, usize);

fn shape(b: &[Block]) -> Vec<Shape> {
    fn walk(
        b: &[Block],
        out: &mut Vec<Shape>,
    ) {
        for x in b {
            out.push((
                x.id.clone(),
                x.kind.clone(),
                x.text.clone(),
                x.target.clone(),
                x.color.clone(),
                x.children.len(),
            ));
            walk(&x.children, out);
        }
    }
    let mut out = Vec::new();
    walk(b, &mut out);
    out
}

#[test]
fn new_kinds_colours_and_marks_are_stored() {
    let t = TempDir::new("kinds");
    let v = t.open();
    let p = build(&v);
    let page = v.load_page(p.clone()).unwrap();
    let kinds: Vec<BlockKind> = page.blocks.iter().map(|b| b.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            BlockKind::Toggle,
            BlockKind::ToggleHeading2,
            BlockKind::Callout,
            BlockKind::ColumnList,
            BlockKind::TableOfContents,
            BlockKind::Paragraph
        ]
    );
    assert_eq!(page.blocks[2].target.as_deref(), Some("💡"));
    assert_eq!(page.blocks[2].color.as_deref(), Some("yellow_background"));
    assert_eq!(page.blocks[3].children.len(), 2);
    assert_eq!(page.blocks[3].children[1].children[0].text, "right");
    assert_eq!(
        page.blocks[5].marks,
        [
            mark(0, 5, MarkKind::Underline),
            mark(6, 9, MarkKind::Color("red".into())),
            mark(14, 19, MarkKind::Date("2026-10-01".into())),
        ]
    );
    // "default" and None clear the colour.
    v.apply(
        p.clone(),
        vec![BlockOp::SetColor {
            id: "p".into(),
            color: Some("default".into()),
        }],
    )
    .unwrap();
    assert_eq!(v.load_page(p.clone()).unwrap().blocks[5].color, None);
    // Unknown block ids are rejected.
    assert!(v
        .apply(
            p,
            vec![BlockOp::SetColor {
                id: "nope".into(),
                color: None
            }]
        )
        .is_err());
}

#[test]
fn markdown_round_trips_new_kinds() {
    let t = TempDir::new("kinds_md");
    let v = t.open();
    let p = build(&v);
    let before = shape(&v.load_page(p.clone()).unwrap().blocks);
    v.flush().unwrap();
    let md = fs::read_to_string(md_path(&t, &p)).unwrap();
    for want in [
        "- Details <!-- toggle -->",
        "## Section <!-- toggle -->",
        "> Note this <!-- callout icon=💡 color=yellow_background -->",
        "<!-- columns -->",
        "  <!-- column -->",
        "<!-- toc -->",
        "<u>under</u> <span data-color=\"red\">red</span> due <time datetime=\"2026-10-01\">Oct 1</time> <!-- color=blue -->",
    ] {
        assert!(md.contains(want), "missing {want:?} in\n{md}");
    }
    // An outside edit elsewhere in the file keeps everything else.
    let edited = md.replace("left", "left side");
    fs::write(md_path(&t, &p), &edited).unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let after = shape(&v.load_page(p.clone()).unwrap().blocks);
    assert_eq!(after.len(), before.len());
    for (a, b) in before.iter().zip(after.iter()) {
        if a.0 == "k1a" {
            assert_eq!(b.2, "left side");
        } else {
            assert_eq!(a, b);
        }
    }
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(
        page.blocks[5].marks,
        [
            mark(0, 5, MarkKind::Underline),
            mark(6, 9, MarkKind::Color("red".into())),
            mark(14, 19, MarkKind::Date("2026-10-01".into())),
        ]
    );

    // Import reads the same forms.
    let dir = t.0.join("import_src");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("Imported.md"),
        "- Open me <!-- toggle color=gray -->\n  - child\n\n> Heads up <!-- callout icon=⚠️ -->\n\n<!-- toc -->\n\nA <!-- comment --> stays <!-- as text -->\n",
    )
    .unwrap();
    v.import_markdown(dir.to_string_lossy().into_owned(), None)
        .unwrap();
    let hit = v
        .list_pages(None, 0, 100)
        .unwrap()
        .into_iter()
        .find(|r| r.title == "Imported")
        .expect("imported page");
    let page = v.load_page(hit.id).unwrap();
    let kinds: Vec<BlockKind> = page.blocks.iter().map(|b| b.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            BlockKind::Toggle,
            BlockKind::Callout,
            BlockKind::TableOfContents,
            BlockKind::Paragraph
        ]
    );
    assert_eq!(page.blocks[0].color.as_deref(), Some("gray"));
    assert_eq!(page.blocks[0].children[0].text, "child");
    assert_eq!(page.blocks[1].target.as_deref(), Some("⚠️"));
    assert_eq!(page.blocks[3].text, "A <!-- comment --> stays <!-- as text -->");
}

#[test]
fn restore_and_duplicate_keep_colours_and_kinds() {
    let t = TempDir::new("kinds_dup");
    let v = t.open();
    let p = build(&v);
    let dup = v.duplicate_page(p.clone()).unwrap();
    let a = shape(&v.load_page(p).unwrap().blocks);
    let b = shape(&v.load_page(dup).unwrap().blocks);
    let strip = |x: &Vec<Shape>| {
        x.iter()
            .map(|r| (r.1.clone(), r.2.clone(), r.3.clone(), r.4.clone(), r.5))
            .collect::<Vec<_>>()
    };
    assert_eq!(strip(&a), strip(&b));
}

#[test]
fn image_width_is_stored_and_round_trips() {
    let t = TempDir::new("kinds_width");
    let v = t.open();
    let p = v.create_page(None, "Pictures".into()).unwrap();
    let att = v
        .add_attachment_bytes("dot.png".into(), vec![137, 80, 78, 71, 13, 10, 26, 10])
        .unwrap();
    v.apply(
        p.clone(),
        vec![
            ins(None, None, nb("i", BlockKind::Image, "A caption")),
            BlockOp::SetTarget {
                id: "i".into(),
                target: Some(att),
            },
            BlockOp::SetWidth {
                id: "i".into(),
                width: Some(60),
            },
        ],
    )
    .unwrap();
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(page.blocks[0].width, Some(60));
    v.flush().unwrap();
    let md = fs::read_to_string(md_path(&t, &p)).unwrap();
    assert!(md.contains("<!-- width=60 -->"), "{md}");
    // An outside edit keeps the width.
    fs::write(md_path(&t, &p), md.replace("A caption", "New caption")).unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(page.blocks[0].text, "New caption");
    assert_eq!(page.blocks[0].width, Some(60));
    // 100 (and None) mean the full width.
    v.apply(
        p.clone(),
        vec![BlockOp::SetWidth {
            id: "i".into(),
            width: Some(100),
        }],
    )
    .unwrap();
    assert_eq!(v.load_page(p).unwrap().blocks[0].width, None);
}
