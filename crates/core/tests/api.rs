use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> TempVault {
        let p = std::env::temp_dir().join(format!("vault_core_test_{}", new_id()));
        TempVault(p)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).expect("open")
    }
}

impl Drop for TempVault {
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

/// The tree as "id(children...)" for compact assertions.
fn shape(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|b| {
            if b.children.is_empty() {
                b.id.clone()
            } else {
                format!("{}({})", b.id, shape(&b.children))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    for b in blocks {
        if b.id == id {
            return Some(b);
        }
        if let Some(x) = find(&b.children, id) {
            return Some(x);
        }
    }
    None
}

fn md_file(t: &TempVault, id: &str) -> String {
    let dir = t.0.join("pages");
    let f = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
        .expect("markdown file");
    fs::read_to_string(f).unwrap()
}

#[test]
fn open_creates_layout_and_reopens() {
    let t = TempVault::new();
    {
        let v = t.open();
        assert_eq!(v.page_count().unwrap(), 0);
        assert!(v.list_pages(None, 0, 10).unwrap().is_empty());
    }
    assert!(t.0.join("pages").is_dir());
    assert!(t.0.join(".vault/docs").is_dir());
    assert!(t.0.join(".vault/index.sqlite").is_file());
    let v = t.open();
    assert_eq!(v.page_count().unwrap(), 0);
}

#[test]
fn create_rename_delete_pages() {
    let t = TempVault::new();
    let v = t.open();
    let root = v.create_page(None, "Garden".into()).unwrap();
    let child = v
        .create_page(Some(root.clone()), "Tomatoes".into())
        .unwrap();
    let grandchild = v.create_page(Some(child.clone()), "Seeds".into()).unwrap();
    let other = v.create_page(None, "Other".into()).unwrap();
    assert_eq!(v.page_count().unwrap(), 4);
    assert!(matches!(
        v.create_page(Some("nope".into()), "x".into()),
        Err(VaultError::NotFound(_))
    ));

    let all = v.list_pages(None, 0, 100).unwrap();
    assert_eq!(all.len(), 4);
    // Newest first.
    assert!(all.windows(2).all(|w| w[0].updated_ms >= w[1].updated_ms));
    let r = all.iter().find(|p| p.id == root).unwrap();
    assert_eq!(
        (r.title.as_str(), r.child_count, r.parent.clone()),
        ("Garden", 1, None)
    );
    let kids = v.list_pages(Some(root.clone()), 0, 100).unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].id, child);
    assert_eq!(kids[0].parent.as_deref(), Some(root.as_str()));
    assert_eq!(v.list_pages(None, 1, 2).unwrap().len(), 2);

    v.rename_page(child.clone(), "Cherry tomatoes".into())
        .unwrap();
    assert_eq!(v.load_page(child.clone()).unwrap().title, "Cherry tomatoes");
    assert_eq!(v.search("cherry".into(), 10).unwrap()[0].id, child);
    v.flush().unwrap();
    assert!(md_file(&t, &child).contains("title: \"Cherry tomatoes\""));
    let names: Vec<String> = fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&format!("cherry-tomatoes-{child}.md")),
        "{names:?}"
    );
    assert!(
        !names.contains(&format!("tomatoes-{child}.md")),
        "old file removed: {names:?}"
    );

    // delete_page moves the page and its subtree to the trash.
    v.delete_page(root.clone()).unwrap();
    assert_eq!(v.page_count().unwrap(), 1);
    for id in [&root, &child, &grandchild] {
        assert!(v.load_page(id.clone()).unwrap().in_trash);
    }
    // Deleting a page already in the trash does nothing.
    v.delete_page(child.clone()).unwrap();
    v.delete_page(root.clone()).unwrap();
    let trash = v.list_trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].page.id, root);
    // Purging deletes it for good.
    v.purge_page(root.clone()).unwrap();
    for id in [&root, &child, &grandchild] {
        assert!(matches!(
            v.load_page(id.clone()),
            Err(VaultError::NotFound(_))
        ));
        assert!(!t.0.join(format!(".vault/docs/{id}.loro")).exists());
    }
    assert!(matches!(
        v.delete_page(root.clone()),
        Err(VaultError::NotFound(_))
    ));
    v.flush().unwrap();
    let names: Vec<String> = fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![format!("other-{other}.md")]);
    assert!(v.search("cherry".into(), 10).unwrap().is_empty());
}

#[test]
fn every_block_op_with_nesting_and_moves() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Ops".into()).unwrap();
    v.apply(
        p.clone(),
        vec![
            ins(None, None, nb("a", BlockKind::Paragraph, "A")),
            ins(None, Some("a"), nb("b", BlockKind::Bulleted, "B")),
            ins(None, Some("b"), nb("c", BlockKind::Todo, "C")),
            ins(Some("b"), None, nb("b1", BlockKind::Bulleted, "B1")),
            ins(Some("b"), Some("b1"), nb("b2", BlockKind::Bulleted, "B2")),
            ins(Some("b1"), None, nb("b1x", BlockKind::Numbered, "B1x")),
            ins(None, None, nb("first", BlockKind::Heading1, "Title")),
        ],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "first a b(b1(b1x) b2) c"
    );

    // Move: into a parent as first child, after a sibling, out to the top level.
    v.apply(
        p.clone(),
        vec![BlockOp::Move {
            id: "c".into(),
            parent: Some("b".into()),
            after: None,
        }],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "first a b(c b1(b1x) b2)"
    );
    v.apply(
        p.clone(),
        vec![BlockOp::Move {
            id: "c".into(),
            parent: Some("b".into()),
            after: Some("b2".into()),
        }],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "first a b(b1(b1x) b2 c)"
    );
    v.apply(
        p.clone(),
        vec![BlockOp::Move {
            id: "b1".into(),
            parent: None,
            after: Some("first".into()),
        }],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "first b1(b1x) a b(b2 c)"
    );
    v.apply(
        p.clone(),
        vec![BlockOp::Move {
            id: "first".into(),
            parent: None,
            after: Some("b".into()),
        }],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "b1(b1x) a b(b2 c) first"
    );
    // Moving to where it already is is a no-op.
    v.apply(
        p.clone(),
        vec![BlockOp::Move {
            id: "a".into(),
            parent: None,
            after: Some("b1".into()),
        }],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "b1(b1x) a b(b2 c) first"
    );

    v.apply(
        p.clone(),
        vec![
            BlockOp::SetKind {
                id: "a".into(),
                kind: BlockKind::Quote,
            },
            BlockOp::SetChecked {
                id: "c".into(),
                checked: true,
            },
            BlockOp::SetText {
                id: "b2".into(),
                text: "B2 edited".into(),
                marks: vec![],
            },
        ],
    )
    .unwrap();
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(find(&page.blocks, "a").unwrap().kind, BlockKind::Quote);
    assert!(find(&page.blocks, "c").unwrap().checked);
    assert_eq!(find(&page.blocks, "b2").unwrap().text, "B2 edited");
    assert_eq!(find(&page.blocks, "b1x").unwrap().kind, BlockKind::Numbered);

    // Delete removes children too.
    v.apply(p.clone(), vec![BlockOp::Delete { id: "b".into() }])
        .unwrap();
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(shape(&page.blocks), "b1(b1x) a first");
    assert!(find(&page.blocks, "c").is_none());

    // Errors change nothing, even after valid ops in the same batch.
    let before = v.load_page(p.clone()).unwrap();
    let bad: Vec<Vec<BlockOp>> = vec![
        vec![
            BlockOp::SetText {
                id: "a".into(),
                text: "changed".into(),
                marks: vec![],
            },
            BlockOp::Delete { id: "c".into() },
        ],
        vec![ins(None, None, nb("a", BlockKind::Paragraph, "dup"))],
        vec![BlockOp::Move {
            id: "b1".into(),
            parent: Some("b1x".into()),
            after: None,
        }],
        vec![BlockOp::Move {
            id: "a".into(),
            parent: None,
            after: Some("b1x".into()),
        }],
        vec![
            ins(Some("b1"), None, nb("n", BlockKind::Paragraph, "x")),
            BlockOp::Delete { id: "b1".into() },
            BlockOp::SetText {
                id: "n".into(),
                text: "y".into(),
                marks: vec![],
            },
        ],
        vec![BlockOp::SetText {
            id: "a".into(),
            text: "ab".into(),
            marks: vec![Mark {
                start: 1,
                end: 3,
                kind: MarkKind::Bold,
            }],
        }],
    ];
    for ops in bad {
        assert!(
            v.apply(p.clone(), ops.clone()).is_err(),
            "{ops:?} should fail"
        );
        assert_eq!(v.load_page(p.clone()).unwrap(), before);
    }
    // Deleting a block then using its id in the same batch fails; inserting
    // under a block inserted earlier in the same batch works.
    v.apply(
        p.clone(),
        vec![
            ins(None, None, nb("z", BlockKind::Bulleted, "Z")),
            ins(Some("z"), None, nb("z1", BlockKind::Bulleted, "Z1")),
        ],
    )
    .unwrap();
    assert_eq!(
        shape(&v.load_page(p.clone()).unwrap().blocks),
        "z(z1) b1(b1x) a first"
    );
    assert!(matches!(
        v.apply("missing".into(), vec![BlockOp::Delete { id: "a".into() }]),
        Err(VaultError::NotFound(_))
    ));
}

#[test]
fn marks_with_non_ascii_text() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Marks".into()).unwrap();
    // "Grüße 😀 aus Zürich": ü, ß and 😀 are one scalar each (😀 is 2 UTF-16
    // units and 4 bytes).
    let text = "Grüße 😀 aus Zürich";
    let marks = vec![
        Mark {
            start: 0,
            end: 5,
            kind: MarkKind::Bold,
        },
        Mark {
            start: 6,
            end: 7,
            kind: MarkKind::Italic,
        },
        Mark {
            start: 12,
            end: 18,
            kind: MarkKind::Link("https://zürich.ch".into()),
        },
        Mark {
            start: 2,
            end: 4,
            kind: MarkKind::Code,
        },
    ];
    v.apply(
        p.clone(),
        vec![ins(
            None,
            None,
            NewBlock {
                id: "x".into(),
                kind: BlockKind::Paragraph,
                text: text.into(),
                marks: marks.clone(),
                checked: false,
            },
        )],
    )
    .unwrap();
    let b = v.load_page(p.clone()).unwrap().blocks.remove(0);
    assert_eq!(b.text, text);
    let mut want = marks.clone();
    want.sort_by_key(|m| m.start);
    assert_eq!(b.marks, want);
    let chars: Vec<char> = b.text.chars().collect();
    assert_eq!(chars[6], '😀');
    assert_eq!(chars[12..18].iter().collect::<String>(), "Zürich");

    // Edit in the middle (diffed into the Loro text) and shift marks.
    let text2 = "Liebe Grüße 🎉😀 aus Zürich!";
    let marks2 = vec![
        Mark {
            start: 6,
            end: 11,
            kind: MarkKind::Bold,
        },
        Mark {
            start: 12,
            end: 14,
            kind: MarkKind::Italic,
        },
        Mark {
            start: 19,
            end: 25,
            kind: MarkKind::Link("https://zürich.ch".into()),
        },
    ];
    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "x".into(),
            text: text2.into(),
            marks: marks2.clone(),
        }],
    )
    .unwrap();
    let b = v.load_page(p.clone()).unwrap().blocks.remove(0);
    assert_eq!(b.text, text2);
    assert_eq!(b.marks, marks2);

    // Overlapping same-kind marks are merged; removing all marks works.
    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "x".into(),
            text: text2.into(),
            marks: vec![
                Mark {
                    start: 0,
                    end: 3,
                    kind: MarkKind::Strike,
                },
                Mark {
                    start: 2,
                    end: 5,
                    kind: MarkKind::Strike,
                },
            ],
        }],
    )
    .unwrap();
    assert_eq!(
        v.load_page(p.clone()).unwrap().blocks[0].marks,
        vec![Mark {
            start: 0,
            end: 5,
            kind: MarkKind::Strike
        }]
    );
    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "x".into(),
            text: "ÄÖÜ".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    let b = v.load_page(p.clone()).unwrap().blocks.remove(0);
    assert_eq!((b.text.as_str(), b.marks.len()), ("ÄÖÜ", 0));

    // Survives a reopen.
    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "x".into(),
            text: "😀😀 ü".into(),
            marks: vec![Mark {
                start: 1,
                end: 4,
                kind: MarkKind::Bold,
            }],
        }],
    )
    .unwrap();
    drop(v);
    let v = t.open();
    let b = v.load_page(p.clone()).unwrap().blocks.remove(0);
    assert_eq!(b.text, "😀😀 ü");
    assert_eq!(
        b.marks,
        vec![Mark {
            start: 1,
            end: 4,
            kind: MarkKind::Bold
        }]
    );
    assert!(md_file(&t, &p).contains("😀**😀 ü**"));
}

#[test]
fn persistence_across_reopen_and_from_update_log() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Persist".into()).unwrap();
    v.apply(
        p.clone(),
        vec![
            ins(None, None, nb("a", BlockKind::Todo, "keep me")),
            BlockOp::SetChecked {
                id: "a".into(),
                checked: true,
            },
        ],
    )
    .unwrap();
    // A second instance sees the change before the writer ran: it replays
    // the update log on top of the snapshot.
    {
        let v2 = t.open();
        let page = v2.load_page(p.clone()).unwrap();
        assert_eq!(page.blocks[0].text, "keep me");
        assert!(page.blocks[0].checked);
    }
    for i in 0..50 {
        v.apply(
            p.clone(),
            vec![BlockOp::SetText {
                id: "a".into(),
                text: format!("keep me {i}"),
                marks: vec![],
            }],
        )
        .unwrap();
    }
    let expected = v.load_page(p.clone()).unwrap();
    drop(v);
    let v = t.open();
    assert_eq!(v.load_page(p.clone()).unwrap(), expected);
    assert_eq!(v.page_count().unwrap(), 1);
    assert_eq!(v.list_pages(None, 0, 10).unwrap()[0].title, "Persist");
    // Everything was compacted into the snapshot on close.
    for entry in fs::read_dir(t.0.join(".vault/logs")).unwrap() {
        assert_eq!(entry.unwrap().metadata().unwrap().len(), 0);
    }
}

#[test]
fn markdown_file_content() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Märkdown: \"test\"".into()).unwrap();
    let b = |id: &str, kind, text: &str| nb(id, kind, text);
    v.apply(
        p.clone(),
        vec![
            ins(None, None, b("h", BlockKind::Heading1, "Heading")),
            ins(
                None,
                Some("h"),
                NewBlock {
                    id: "p".into(),
                    kind: BlockKind::Paragraph,
                    text: "Some bold and a link".into(),
                    marks: vec![
                        Mark {
                            start: 5,
                            end: 9,
                            kind: MarkKind::Bold,
                        },
                        Mark {
                            start: 16,
                            end: 20,
                            kind: MarkKind::Link("https://a.b".into()),
                        },
                    ],
                    checked: false,
                },
            ),
            ins(None, Some("p"), b("l1", BlockKind::Bulleted, "one")),
            ins(Some("l1"), None, b("l1a", BlockKind::Bulleted, "nested")),
            ins(None, Some("l1"), b("l2", BlockKind::Bulleted, "two")),
            ins(
                None,
                Some("l2"),
                NewBlock {
                    id: "t".into(),
                    kind: BlockKind::Todo,
                    text: "done".into(),
                    marks: vec![],
                    checked: true,
                },
            ),
            ins(None, Some("t"), b("n1", BlockKind::Numbered, "first")),
            ins(None, Some("n1"), b("n2", BlockKind::Numbered, "second")),
            ins(None, Some("n2"), b("q", BlockKind::Quote, "wise words")),
            ins(
                None,
                Some("q"),
                b("c", BlockKind::Code, "let x = 1;\nlet y = *x;"),
            ),
            ins(None, Some("c"), b("d", BlockKind::Divider, "")),
            ins(None, Some("d"), b("h2", BlockKind::Heading2, "Two")),
            ins(None, Some("h2"), b("h3", BlockKind::Heading3, "Three")),
        ],
    )
    .unwrap();
    v.flush().unwrap();
    let md = md_file(&t, &p);
    let expected_body = "
# Heading

Some **bold** and a [link](https://a.b)

- one
  - nested
- two
- [x] done
1. first
2. second

> wise words

```
let x = 1;
let y = *x;
```

---

## Two

### Three
";
    assert!(
        md.starts_with(&format!(
            "---\nid: {p}\ntitle: \"Märkdown: \\\"test\\\"\"\nparent: null\ncreated: "
        )),
        "{md}"
    );
    let body = md.splitn(3, "---\n").nth(2).unwrap();
    assert_eq!(body, expected_body, "{md}");
    let name = fs::read_dir(t.0.join("pages"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    assert_eq!(name.to_string_lossy(), format!("märkdown-test-{p}.md"));
}

#[test]
fn search_prefix_matching() {
    let t = TempVault::new();
    let v = t.open();
    let a = v.create_page(None, "Gardening tips".into()).unwrap();
    let b = v.create_page(None, "Kitchen".into()).unwrap();
    let c = v.create_page(None, "Über Zürich".into()).unwrap();
    v.apply(
        b.clone(),
        vec![ins(
            None,
            None,
            nb(
                "x",
                BlockKind::Paragraph,
                "Roasted tomatoes with zucchini and garlic",
            ),
        )],
    )
    .unwrap();
    v.flush().unwrap();
    let ids = |q: &str| -> Vec<String> {
        let mut v: Vec<String> = v
            .search(q.into(), 10)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect();
        v.sort();
        v
    };
    let sorted = |mut x: Vec<String>| {
        x.sort();
        x
    };
    // Exact and prefix hits (never fuzzy) come first; fuzzy hits follow.
    let split = |q: &str| -> (Vec<String>, Vec<String>) {
        let hits = v.search(q.into(), 10).unwrap();
        let first_fuzzy = hits.iter().position(|h| h.fuzzy).unwrap_or(hits.len());
        assert!(
            hits[first_fuzzy..].iter().all(|h| h.fuzzy),
            "{q:?}: an exact hit ranks after a fuzzy one"
        );
        let exact = hits[..first_fuzzy].iter().map(|h| h.id.clone()).collect();
        let fuzzy = hits[first_fuzzy..].iter().map(|h| h.id.clone()).collect();
        (exact, fuzzy)
    };
    let (exact, fuzzy) = split("gar");
    assert_eq!(sorted(exact), sorted(vec![a.clone(), b.clone()]));
    assert!(fuzzy.is_empty(), "{fuzzy:?}");
    // An existing prefix is never corrected, even with just one result.
    let (exact, fuzzy) = split("gard");
    assert_eq!(exact, vec![a.clone()]);
    assert!(fuzzy.is_empty());
    assert_eq!(v.search("gard".into(), 10).unwrap()[0].id, a);
    assert_eq!(ids("Garde"), vec![a.clone()]);
    assert_eq!(ids("tom"), vec![b.clone()]);
    assert_eq!(ids("zucc TOM"), vec![b.clone()]);
    assert_eq!(ids("zucc tips"), Vec::<String>::new());
    assert_eq!(ids("zür"), vec![c.clone()]);
    assert_eq!(ids("zur"), vec![c.clone()]);
    assert_eq!(ids("uber"), vec![c.clone()]);
    assert!(ids("").is_empty());
    assert!(ids("  \"*()^ ").is_empty());
    assert!(ids("nothinghere").is_empty());
    let hit = &v.search("zucchini".into(), 10).unwrap()[0];
    assert_eq!(hit.title, "Kitchen");
    assert!(hit.snippet.contains("zucchini"), "{}", hit.snippet);
    // Title matches rank above body matches.
    assert_eq!(v.search("gar".into(), 10).unwrap()[0].id, a);
    assert_eq!(v.search("g".into(), 1).unwrap().len(), 1);
}

#[test]
fn seed_and_reindex_rebuild_the_same_results() {
    let t = TempVault::new();
    seed::generate(t.root(), 300, 400).unwrap();
    assert!(
        seed::generate(t.root(), 10, 10).is_err(),
        "seeding a non-empty vault fails"
    );
    let v = t.open();
    assert_eq!(v.page_count().unwrap(), 301);
    assert_eq!(
        v.ensure_space_colors().unwrap(),
        0,
        "seed colors need no writer backfill"
    );
    let top = v.list_pages(None, 0, 5).unwrap();
    assert_eq!(top[0].title, seed::BIG_PAGE_TITLE);
    let big = v.load_page(top[0].id.clone()).unwrap();
    fn count(b: &[Block]) -> usize {
        b.iter().map(|x| 1 + count(&x.children)).sum()
    }
    assert_eq!(count(&big.blocks), 400);
    assert!(
        big.blocks.iter().any(|b| !b.children.is_empty()),
        "big page has nested blocks"
    );
    for kind in [
        BlockKind::Heading1,
        BlockKind::Heading2,
        BlockKind::Heading3,
        BlockKind::Bulleted,
        BlockKind::Numbered,
        BlockKind::Todo,
        BlockKind::Quote,
        BlockKind::Code,
        BlockKind::Divider,
        BlockKind::Paragraph,
    ] {
        assert!(big.blocks.iter().any(|b| b.kind == kind), "{kind:?}");
    }
    assert!(v
        .list_pages(None, 0, 400)
        .unwrap()
        .iter()
        .any(|p| p.parent.is_some() && p.child_count == 0));

    let queries = [
        "pro",
        "garden plan",
        "caf",
        "zür",
        "b",
        "weekly review",
        "tomato",
    ];
    let snapshot = |v: &Vault| {
        let lists = (
            v.list_pages(None, 0, 1000).unwrap(),
            v.list_pages(Some(top[1].id.clone()), 0, 100).unwrap(),
        );
        let searches: Vec<Vec<SearchHit>> = queries
            .iter()
            .map(|q| v.search((*q).into(), 50).unwrap())
            .collect();
        (lists, searches)
    };
    let before = snapshot(&v);
    assert!(
        before.1.iter().all(|s| !s.is_empty()),
        "every query finds something"
    );
    assert_eq!(v.reindex().unwrap(), 301);
    assert_eq!(snapshot(&v), before);
    drop(v);
    // A lost index is rebuilt on open.
    for f in ["index.sqlite", "index.sqlite-wal", "index.sqlite-shm"] {
        let _ = fs::remove_file(t.0.join(".vault").join(f));
    }
    let v = t.open();
    assert_eq!(v.page_count().unwrap(), 301);
    assert_eq!(snapshot(&v), before);
}

#[test]
fn api_types_serialize_for_bindings() {
    let op = BlockOp::SetText {
        id: "b".into(),
        text: "hi".into(),
        marks: vec![Mark {
            start: 0,
            end: 2,
            kind: MarkKind::Link("u".into()),
        }],
    };
    let json = serde_json::to_string(&op).unwrap();
    assert_eq!(
        json,
        r#"{"op":"SetText","id":"b","text":"hi","marks":[{"start":0,"end":2,"kind":{"Link":"u"}}]}"#
    );
    assert_eq!(serde_json::from_str::<BlockOp>(&json).unwrap(), op);
    assert_eq!(serde_json::to_string(&MarkKind::Bold).unwrap(), r#""Bold""#);
    let e: String = VaultError::NotFound("page x".into()).into();
    assert_eq!(e, "not found: page x");
}

#[test]
fn big_page_block_cache_matches_the_document() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "Big".into()).unwrap();
    let small = v.create_page(None, "Small".into()).unwrap();
    let mut ops = Vec::new();
    let mut last: Option<String> = None;
    for i in 0..300 {
        let id = format!("b{i}");
        let text = format!("Block {i} — ümlaut 😀");
        let marks = vec![Mark {
            start: 0,
            end: 5,
            kind: MarkKind::Bold,
        }];
        ops.push(ins(
            None,
            last.as_deref(),
            NewBlock {
                id: id.clone(),
                kind: BlockKind::Bulleted,
                text,
                marks,
                checked: false,
            },
        ));
        if i % 10 == 0 {
            ops.push(ins(
                Some(&id),
                None,
                nb(&format!("c{i}"), BlockKind::Todo, "child"),
            ));
        }
        last = Some(id);
    }
    v.apply(p.clone(), ops).unwrap();
    v.apply(
        small.clone(),
        vec![ins(None, None, nb("x", BlockKind::Paragraph, "x"))],
    )
    .unwrap();
    let expected = v.load_page(p.clone()).unwrap();
    drop(v);
    assert!(t.0.join(format!(".vault/docs/{p}.blocks.json")).is_file());
    assert!(!t
        .0
        .join(format!(".vault/docs/{small}.blocks.json"))
        .exists());

    // Cold load from the cache, then from the warmed document: identical.
    let v = t.open();
    assert_eq!(v.load_page(p.clone()).unwrap(), expected);
    assert_eq!(v.load_page(p.clone()).unwrap(), expected);
    // Editing right after a cached load works.
    v.apply(
        p.clone(),
        vec![BlockOp::SetText {
            id: "b5".into(),
            text: "changed".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    let edited = v.load_page(p.clone()).unwrap();
    assert_eq!(find(&edited.blocks, "b5").unwrap().text, "changed");

    // A cache older than the document (edit not flushed yet) is ignored.
    let v2 = t.open();
    assert_eq!(v2.load_page(p.clone()).unwrap(), edited);
    drop(v2);
    drop(v);
    let v = t.open();
    assert_eq!(v.load_page(p.clone()).unwrap(), edited);
}
