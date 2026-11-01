//! Markdown fidelity on import and export: GFM tables kept verbatim, code
//! fence info strings, `.md` links with their full target as written, and
//! missing links counted with their anchors.
use std::{fs, path::PathBuf};
use vault_core::*;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        Temp(std::env::temp_dir().join(format!("notes_mdfid_{tag}_{}", new_id())))
    }
    fn vault(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
    /// A new folder with the given files; returns its path.
    fn source(&self, files: &[(&str, &str)]) -> String {
        let dir = self.0.join(format!("src_{}", new_id()));
        fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            fs::write(dir.join(name), body).unwrap();
        }
        dir.to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn import(v: &Vault, dir: &str) -> ImportReport {
    v.import_markdown(dir.to_string(), None).unwrap()
}

fn page_titled(v: &Vault, title: &str) -> Page {
    let hit = v
        .list_pages(None, 0, 100)
        .unwrap()
        .into_iter()
        .find(|r| r.title == title)
        .unwrap_or_else(|| panic!("no page titled {title}"));
    v.load_page(hit.id).unwrap()
}

/// An exported page's body, without its frontmatter.
fn body(exported: &str) -> &str {
    exported.split_once("\n---\n").map_or(exported, |(_, b)| b)
}

const TABLES: &str = r#"Intro before table
| Left | Center | Right |
|:-----|:------:|------:|
| `a|b` | x \| y | z |
| 1 | 2 | 3 |

Second table:

| k | v |
| - | - |
| only | row |
"#;

const TABLE_ONE: &str =
    "| Left | Center | Right |\n|:-----|:------:|------:|\n| `a|b` | x \\| y | z |\n| 1 | 2 | 3 |";
const TABLE_TWO: &str = "| k | v |\n| - | - |\n| only | row |";

#[test]
fn two_tables_import_verbatim_and_stay_distinct() {
    let t = Temp::new("tables");
    let v = t.vault();
    let dir = t.source(&[("Tables.md", TABLES)]);
    assert_eq!(import(&v, &dir).tables, 2);
    let page = page_titled(&v, "Tables");
    let kinds: Vec<BlockKind> = page.blocks.iter().map(|b| b.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            BlockKind::Paragraph,
            BlockKind::Table,
            BlockKind::Paragraph,
            BlockKind::Table
        ]
    );
    assert_eq!(page.blocks[1].text, TABLE_ONE);
    assert_eq!(page.blocks[3].text, TABLE_TWO);
    assert!(page.blocks[1].marks.is_empty());
}

#[test]
fn table_text_survives_export_and_reimport() {
    let t = Temp::new("roundtrip");
    let v = t.vault();
    let dir = t.source(&[("Tables.md", TABLES)]);
    import(&v, &dir);
    let page = page_titled(&v, "Tables");
    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(
        body(&exported).contains(&format!("{TABLE_ONE}\n\n")),
        "{exported}"
    );
    assert!(body(&exported).contains(TABLE_TWO), "{exported}");
    // The exported body, imported again as a new page.
    let dir2 = t.source(&[("Again.md", body(&exported))]);
    import(&v, &dir2);
    let again = page_titled(&v, "Again");
    let texts: Vec<&str> = again
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Table)
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(texts, [TABLE_ONE, TABLE_TWO]);
}

#[test]
fn fence_info_string_is_kept_and_exported() {
    let t = Temp::new("fences");
    let v = t.vault();
    let dir = t.source(&[("Code.md", "```rust\nlet x = 1;\n```\n\n```\nplain\n```\n")]);
    import(&v, &dir);
    let page = page_titled(&v, "Code");
    assert_eq!(page.blocks[0].kind, BlockKind::Code);
    assert_eq!(page.blocks[0].target.as_deref(), Some("rust"));
    assert_eq!(page.blocks[0].text, "let x = 1;");
    assert_eq!(page.blocks[1].kind, BlockKind::Code);
    assert_eq!(page.blocks[1].target, None);
    assert_eq!(page.blocks[1].text, "plain");
    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(
        body(&exported).contains("```rust\nlet x = 1;\n```\n"),
        "{exported}"
    );
    assert!(body(&exported).contains("```\nplain\n```\n"), "{exported}");
}

#[test]
fn md_links_keep_raw_encoded_targets() {
    let t = Temp::new("encoded");
    let v = t.vault();
    let dir = t.source(&[(
        "Links.md",
        "[enc](Notes/My%20Page.md#Intro) and [plain](Other%C3%A9.md)\n",
    )]);
    let report = import(&v, &dir);
    let targets: Vec<&str> = report
        .unresolved_targets
        .iter()
        .map(|u| u.target.as_str())
        .collect();
    assert_eq!(targets, ["Notes/My%20Page.md#Intro", "Other%C3%A9.md"]);
    assert_eq!(report.unresolved_links, 2);
    let page = page_titled(&v, "Links");
    assert_eq!(page.blocks[0].text, "enc and plain");
    assert!(page.blocks[0].marks.contains(&Mark {
        start: 0,
        end: 3,
        kind: MarkKind::Link("Notes/My%20Page.md#Intro".into()),
    }));
    assert!(page.blocks[0].marks.contains(&Mark {
        start: 8,
        end: 13,
        kind: MarkKind::Link("Other%C3%A9.md".into()),
    }));
    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(
        body(&exported).contains("[enc](Notes/My%20Page.md#Intro) and [plain](Other%C3%A9.md)"),
        "{exported}"
    );
}

#[test]
fn md_targets_reach_the_resolver_and_misses_are_counted() {
    let t = Temp::new("counts");
    let v = t.vault();
    let dir = t.source(&[
        ("Target.md", "# Target\n"),
        (
            "Source.md",
            "[ok](Target.md#Sec) and [upper](Target.MD) and [miss](Nope.md#X)\n\n[[Missing#Anchor]] and [[Target#Top|alias]]\n",
        ),
    ]);
    let report = import(&v, &dir);
    assert_eq!(report.resolved_links, 3);
    assert_eq!(report.unresolved_links, 2);
    let targets: Vec<&str> = report
        .unresolved_targets
        .iter()
        .map(|u| u.target.as_str())
        .collect();
    assert_eq!(targets, ["Nope.md#X", "Missing#Anchor"]);

    let target_id = page_titled(&v, "Target").id;
    let source = page_titled(&v, "Source");
    let marks: Vec<&Mark> = source.blocks.iter().flat_map(|b| b.marks.iter()).collect();
    let page_ids: Vec<&PageId> = marks
        .iter()
        .filter_map(|m| match &m.kind {
            MarkKind::PageLink(id) => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(page_ids, vec![&target_id; 3]);
    let raw: Vec<&str> = marks
        .iter()
        .filter_map(|m| match &m.kind {
            MarkKind::Link(url) => Some(url.as_str()),
            _ => None,
        })
        .collect();
    // The wikilink keeps its target as written, anchor included.
    assert_eq!(
        raw,
        ["Target.md#Sec", "Target.MD", "Nope.md#X", "Target#Top"]
    );

    let exported = v.export_markdown(source.id.clone()).unwrap();
    let out = body(&exported);
    assert!(
        out.contains("[ok](Target.md#Sec) and [upper](Target.MD) and [miss](Nope.md#X)"),
        "{exported}"
    );
    assert!(
        out.contains(r"\[\[Missing#Anchor\]\] and [[Target#Top|alias]]"),
        "{exported}"
    );
}

#[test]
fn crlf_table_keeps_its_line_endings_in_text_and_export() {
    let t = Temp::new("crlf");
    let v = t.vault();
    let dir = t.source(&[(
        "Crlf.md",
        "Before\r\n\r\n| a | b |\r\n|:-|-:|\r\n| 1 | 2 |\r\n\r\nAfter\r\n",
    )]);
    import(&v, &dir);
    let page = page_titled(&v, "Crlf");
    const CRLF_TABLE: &str = "| a | b |\r\n|:-|-:|\r\n| 1 | 2 |";
    let table = page
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Table)
        .expect("table");
    assert_eq!(table.text, CRLF_TABLE);
    // The table's inner rows keep `\r\n`; the last line is written with `\n`.
    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(
        body(&exported).contains(&format!("{CRLF_TABLE}\n\n")),
        "{exported}"
    );
    // Imported again, the table is the same.
    let dir2 = t.source(&[("Again.md", body(&exported))]);
    import(&v, &dir2);
    let again = page_titled(&v, "Again");
    let texts: Vec<&str> = again
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Table)
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(texts, [CRLF_TABLE]);
}

fn md_path(t: &Temp, id: &str) -> PathBuf {
    fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
        .expect("markdown file")
}

#[test]
fn edits_elsewhere_keep_links_code_and_tables() {
    let t = Temp::new("flush");
    let v = t.vault();
    let dir = t.source(&[
        ("Target.md", "# Target\n"),
        (
            "Mixed.md",
            "See [ok](Target.md#Sec) and [[Target#Top|alias]].\n\nNote to edit.\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        ),
    ]);
    import(&v, &dir);
    let target_id = page_titled(&v, "Target").id;
    let page = page_titled(&v, "Mixed");
    let note = page
        .blocks
        .iter()
        .find(|b| b.text == "Note to edit.")
        .expect("note")
        .id
        .clone();
    v.apply(
        page.id.clone(),
        vec![BlockOp::SetText {
            id: note,
            text: "Edited note.".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    v.flush().unwrap();
    let md = fs::read_to_string(md_path(&t, &page.id)).unwrap();
    assert!(md.contains("[ok](Target.md#Sec)"), "{md}");
    assert!(md.contains("[[Target#Top|alias]]"), "{md}");
    assert!(md.contains("```rust\nlet x = 1;\n```\n"), "{md}");
    assert!(md.contains("| a | b |\n|---|---|\n| 1 | 2 |\n"), "{md}");
    // The frontmatter maps each original target to its page.
    assert!(md.contains("Target.md#Sec"), "{md}");
    assert!(md.contains(&target_id), "{md}");

    // An outside edit to another paragraph, then a rescan.
    fs::write(
        md_path(&t, &page.id),
        md.replace("Edited note.", "Outside edit."),
    )
    .unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let page = page_titled(&v, "Mixed");
    let first = &page.blocks[0];
    assert_eq!(first.text, "See ok and alias.");
    for want in [
        Mark {
            start: 4,
            end: 6,
            kind: MarkKind::Link("Target.md#Sec".into()),
        },
        Mark {
            start: 4,
            end: 6,
            kind: MarkKind::PageLink(target_id.clone()),
        },
        Mark {
            start: 11,
            end: 16,
            kind: MarkKind::Link("Target#Top".into()),
        },
        Mark {
            start: 11,
            end: 16,
            kind: MarkKind::PageLink(target_id.clone()),
        },
    ] {
        assert!(first.marks.contains(&want), "{want:?} in {:?}", first.marks);
    }
    assert!(page.blocks.iter().any(|b| b.text == "Outside edit."));
    let code = page
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Code)
        .expect("code");
    assert_eq!(code.target.as_deref(), Some("rust"));
    assert_eq!(code.text, "let x = 1;");
    let table = page
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Table)
        .expect("table");
    assert_eq!(table.text, "| a | b |\n|---|---|\n| 1 | 2 |");
}

#[test]
fn table_indentation_and_pipeless_body_rows_are_kept_verbatim() {
    let t = Temp::new("indented");
    let v = t.vault();
    let table = "  | a | b |  \n  | :--- | ---: |\n  | 1 | 2 |\n  single cell";
    let dir = t.source(&[("Indented.md", table)]);
    import(&v, &dir);
    let page = page_titled(&v, "Indented");
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].kind, BlockKind::Table);
    assert_eq!(page.blocks[0].text, table);
    assert_eq!(
        body(&v.export_markdown(page.id).unwrap()).trim_matches('\n'),
        table
    );
}

#[test]
fn adjacent_wikilinks_keep_distinct_targets_through_rename_and_rescan() {
    let t = Temp::new("adjacent");
    let v = t.vault();
    let dir = t.source(&[
        (
            "Target.md",
            "---\ntitle: Different title\n---\nTarget body.\n",
        ),
        (
            "Source.md",
            "[[Target|x]][[Target#Y|y]] [md](Target.md#Z)\n\nOther paragraph.\n",
        ),
    ]);
    import(&v, &dir);
    let page = page_titled(&v, "Source");
    let target = page_titled(&v, "Different title");
    v.rename_page(target.id.clone(), "Renamed target".into())
        .unwrap();
    v.flush().unwrap();
    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(
        body(&exported).contains("[[Target|x]][[Target#Y|y]] [md](Target.md#Z)"),
        "{exported}"
    );
    fs::write(
        md_path(&t, &page.id),
        exported.replace("Other paragraph.", "Outside paragraph."),
    )
    .unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let after = v.load_page(page.id).unwrap();
    for raw in ["Target", "Target#Y", "Target.md#Z"] {
        assert!(
            after.blocks[0]
                .marks
                .iter()
                .any(|m| m.kind == MarkKind::Link(raw.into())),
            "{after:?}"
        );
    }
    assert!(after.blocks[0]
        .marks
        .iter()
        .any(|m| m.start == 0 && m.end >= 2 && m.kind == MarkKind::PageLink(target.id.clone())));
}

#[test]
fn encoded_markdown_extensions_resolve_and_missing_targets_are_reported() {
    let t = Temp::new("encoded_extension");
    let v = t.vault();
    let dir = t.source(&[
        ("Target.md", "Target body.\n"),
        (
            "Source.md",
            "[encoded](%54arget%2Em%64#%61nchor) [missing](Missing%2Emd#x)\n",
        ),
    ]);
    let report = import(&v, &dir);
    assert_eq!((report.resolved_links, report.unresolved_links), (1, 1));
    assert_eq!(report.unresolved_targets[0].target, "Missing%2Emd#x");
    let source = page_titled(&v, "Source");
    let target = page_titled(&v, "Target");
    assert!(source.blocks[0]
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::PageLink(target.id.clone())));
    assert!(v
        .export_markdown(source.id)
        .unwrap()
        .contains("[encoded](%54arget%2Em%64#%61nchor)"));
}

/// The char range of `label` inside the first `whole` in `text`.
fn label_in(text: &str, whole: &str, label: &str) -> (u32, u32) {
    let at = text.find(whole).expect(whole) + whole.find(label).expect(label);
    let lo = text[..at].chars().count() as u32;
    (lo, lo + label.chars().count() as u32)
}

fn table_of(page: &Page) -> &Block {
    page.blocks
        .iter()
        .find(|b| b.kind == BlockKind::Table)
        .expect("table")
}

const LINK_TABLE: &str = "| Link | Target |\n|---|---|\n| [good](../Target.md#Sec) | [[Target#Top\\|alias]] |\n| [miss](Nope.md#X) | [[Missing#Anchor]] |\n| [enc](%54arget%2Em%64#%61nchor) | [[Target]] |";

#[test]
fn table_links_resolve_in_place_and_keep_the_source() {
    let t = Temp::new("table_links");
    let v = t.vault();
    let dir = t.source(&[("Target.md", "# Target\n")]);
    // The table sits in a subfolder, so `../` is a real relative path.
    let sub = PathBuf::from(&dir).join("Sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("Source.md"), format!("Intro\n\n{LINK_TABLE}\n")).unwrap();
    let report = import(&v, &dir);
    assert_eq!((report.resolved_links, report.unresolved_links), (4, 2));
    let targets: Vec<&str> = report
        .unresolved_targets
        .iter()
        .map(|u| u.target.as_str())
        .collect();
    assert_eq!(targets, ["Nope.md#X", "Missing#Anchor"]);

    let target_id = page_titled(&v, "Target").id;
    let page = page_titled(&v, "Source");
    let table = table_of(&page);
    // The table text is the source, unchanged.
    assert_eq!(table.text, LINK_TABLE);
    let good = label_in(&table.text, "[good](", "good");
    let alias = label_in(&table.text, "[[Target#Top\\|alias]]", "alias");
    let miss = label_in(&table.text, "[miss](", "miss");
    let enc = label_in(&table.text, "[enc](", "enc");
    let target = label_in(&table.text, "[[Target]]", "Target");
    let mark = |(start, end): (u32, u32), kind: MarkKind| Mark { start, end, kind };
    let want = [
        mark(good, MarkKind::Link("../Target.md#Sec".into())),
        mark(good, MarkKind::PageLink(target_id.clone())),
        mark(alias, MarkKind::Link("Target#Top".into())),
        mark(alias, MarkKind::PageLink(target_id.clone())),
        // A missing note keeps its target, without a page.
        mark(miss, MarkKind::Link("Nope.md#X".into())),
        mark(enc, MarkKind::Link("%54arget%2Em%64#%61nchor".into())),
        mark(enc, MarkKind::PageLink(target_id.clone())),
        mark(target, MarkKind::Link("Target".into())),
        mark(target, MarkKind::PageLink(target_id.clone())),
    ];
    for m in &want {
        assert!(table.marks.contains(m), "{m:?} in {:?}", table.marks);
    }
    // Nothing else: the missing wikilink stays literal.
    assert_eq!(table.marks.len(), want.len(), "{:?}", table.marks);

    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(body(&exported).contains(LINK_TABLE), "{exported}");

    // Imported again unchanged: same blocks and marks, nothing rewritten.
    let again = import(&v, &dir);
    assert_eq!((again.created, again.updated), (0, 0));
    let page2 = page_titled(&v, "Source");
    assert_eq!(page2.id, page.id);
    assert_eq!(table_of(&page2).id, table.id);
    assert_eq!(table_of(&page2).marks, table.marks);

    // An outside edit to the paragraph; the table's links survive a rescan.
    v.flush().unwrap();
    let md = fs::read_to_string(md_path(&t, &page.id)).unwrap();
    fs::write(md_path(&t, &page.id), md.replace("Intro\n", "Outside\n")).unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let after = page_titled(&v, "Source");
    let table_after = table_of(&after);
    assert_eq!(table_after.text, LINK_TABLE);
    for m in &want {
        assert!(
            table_after.marks.contains(m),
            "{m:?} in {:?}",
            table_after.marks
        );
    }
    assert_eq!(table_after.marks.len(), want.len());
}

const LITERAL_TABLE: &str = "| code | escaped | real |\n|---|---|---|\n| `[[Target]]` `[t](Target.md)` | \\[[Target]] \\[t](Target.md) | [[Target|real]] |";

#[test]
fn table_code_and_escaped_brackets_are_not_links() {
    let t = Temp::new("table_literal");
    let v = t.vault();
    let dir = t.source(&[("Target.md", "# Target\n"), ("Literal.md", LITERAL_TABLE)]);
    let report = import(&v, &dir);
    // Only the real wikilink is read; the code and escaped cells are not.
    assert_eq!((report.resolved_links, report.unresolved_links), (1, 0));
    let target_id = page_titled(&v, "Target").id;
    let page = page_titled(&v, "Literal");
    let table = table_of(&page);
    assert_eq!(table.text, LITERAL_TABLE);
    let real = label_in(&table.text, "[[Target|real]]", "real");
    assert_eq!(table.marks.len(), 2, "{:?}", table.marks);
    assert!(table.marks.contains(&Mark {
        start: real.0,
        end: real.1,
        kind: MarkKind::Link("Target".into()),
    }));
    assert!(table.marks.contains(&Mark {
        start: real.0,
        end: real.1,
        kind: MarkKind::PageLink(target_id),
    }));
    let exported = v.export_markdown(page.id.clone()).unwrap();
    assert!(body(&exported).contains(LITERAL_TABLE), "{exported}");
}

fn query_card(text: &str) -> Block {
    Block {
        id: "query-card".into(),
        kind: BlockKind::Query,
        text: text.into(),
        marks: vec![],
        checked: false,
        children: vec![],
        target: None,
        color: None,
        width: None,
        numbered_start: None,
    }
}

#[test]
fn query_fences_round_trip_literal_text_and_trailing_newlines() {
    let t = Temp::new("query_fences");
    let v = t.vault();
    for spec in [
        "",
        "tag:work in:[[page-id]]",
        "**literal** [x](url)\n```\n~~~~\ntrailing\n",
    ] {
        let markdown = v.blocks_markdown(vec![query_card(spec)]);
        if !spec.contains('`') {
            assert!(markdown.contains("```query\n"), "{markdown}");
        }
        let blocks = v.parse_markdown(markdown);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, BlockKind::Query);
        assert_eq!(blocks[0].text, spec);
        assert!(blocks[0].marks.is_empty());
        assert!(blocks[0].children.is_empty());
        assert_eq!(blocks[0].target, None);
    }
    // Other language info strings keep their existing code-block meaning.
    let code = v.parse_markdown("```query extra\ntag:work\n```\n".into());
    assert_eq!(code[0].kind, BlockKind::Code);
    assert_eq!(code[0].target.as_deref(), Some("query extra"));
    let leaf = v.parse_markdown("```query\ntag:work\n```\n\n  following paragraph\n".into());
    assert_eq!(leaf.len(), 2);
    assert!(leaf[0].children.is_empty());
    assert_eq!(leaf[1].text, "following paragraph");
}

#[test]
fn query_chip_round_trips_query_entities_and_nested_marks() {
    let t = Temp::new("query_chip");
    let v = t.vault();
    let spec = "title:\"[open\" tag:雪 <5 >2 & &quot;\n\r\t in:[[page-id]]";
    let label = "🔎 4 open #work tasks";
    let mut block = query_card("");
    block.kind = BlockKind::Paragraph;
    block.text = label.into();
    block.marks = vec![
        Mark {
            start: 2,
            end: 20,
            kind: MarkKind::Query(spec.into()),
        },
        Mark {
            start: 2,
            end: 20,
            kind: MarkKind::Color("red".into()),
        },
        Mark {
            start: 4,
            end: 8,
            kind: MarkKind::Bold,
        },
        Mark {
            start: 2,
            end: 20,
            kind: MarkKind::Link("https://example.test".into()),
        },
    ];
    let markdown = v.blocks_markdown(vec![block.clone()]);
    assert!(markdown.contains("<span data-query=\""), "{markdown}");
    assert!(markdown.contains("&quot;"), "{markdown}");
    assert!(markdown.contains("&amp;quot;"), "{markdown}");
    assert!(markdown.contains("&#10;&#13;&#9;"), "{markdown}");
    let parsed = v.parse_markdown(markdown);
    assert_eq!(parsed[0].text, label);
    assert_eq!(
        parsed[0].marks.len(),
        block.marks.len(),
        "{:?}",
        parsed[0].marks
    );
    for mark in &block.marks {
        assert!(parsed[0].marks.contains(mark), "{mark:?}");
    }
}

#[test]
fn query_card_and_chip_survive_file_export_import() {
    let t = Temp::new("query_file");
    let v = t.vault();
    let spec = "tag:work in:[[page-id]]";
    let source = t.source(&[(
        "Queries.md",
        &format!(
            "```query\n{spec}\n```\n\n<span data-query=\"{spec}\">4 open #work tasks</span>\n"
        ),
    )]);
    let report = import(&v, &source);
    assert_eq!(report.created, 1);
    let page = page_titled(&v, "Queries");
    assert_eq!(page.blocks[0].kind, BlockKind::Query);
    assert_eq!(page.blocks[0].text, spec);
    assert_eq!(page.blocks[1].marks[0].kind, MarkKind::Query(spec.into()));
    let exported = v.export_markdown(page.id).unwrap();
    let again_source = t.source(&[("Again.md", body(&exported))]);
    import(&v, &again_source);
    let again = page_titled(&v, "Again");
    for (actual, expected) in again.blocks.iter().zip(&page.blocks) {
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.marks, expected.marks);
    }
    assert_eq!(again.blocks.len(), page.blocks.len());
}

/// One paragraph, `before <code> after`, whose code span is `span` and
/// whose markdown is `rendered` (the span as written). The text is read back
/// with the same text and marks.
fn code_span_round_trips(span: &str, rendered: &str) {
    let t = Temp::new("inline_code");
    let v = t.vault();
    let text = format!("before {span} after");
    let start = "before ".chars().count() as u32;
    let block = Block {
        id: new_id(),
        kind: BlockKind::Paragraph,
        text: text.clone(),
        marks: vec![Mark {
            start,
            end: start + span.chars().count() as u32,
            kind: MarkKind::Code,
        }],
        checked: false,
        children: Vec::new(),
        target: None,
        color: None,
        width: None,
        numbered_start: None,
    };
    let markdown = v.blocks_markdown(vec![block.clone()]);
    assert_eq!(markdown, format!("before {rendered} after"));
    let again = v.parse_markdown(markdown).remove(0);
    assert_eq!(again.kind, BlockKind::Paragraph);
    assert_eq!(again.text, text);
    assert_eq!(again.marks, block.marks);
}

#[test]
fn inline_code_with_a_backtick_inside_round_trips() {
    code_span_round_trips("a`b", "``a`b``");
}

#[test]
fn inline_code_of_only_backticks_round_trips() {
    code_span_round_trips("```", "```` ``` ````");
}

#[test]
fn inline_code_starting_and_ending_with_backticks_round_trips() {
    code_span_round_trips("`x`", "`` `x` ``");
}

#[test]
fn plain_inline_code_keeps_single_backticks() {
    code_span_round_trips("code", "`code`");
}

#[test]
fn inline_code_preserves_content_spaces_around_backticks() {
    code_span_round_trips(" `x` ", "``  `x`  ``");
}

/// Independently of the writer, CommonMark drops one space
/// from each side of a code span that has both, so `` `` `x` `` `` is `` `x` ``.
#[test]
fn inline_code_padding_is_stripped_when_parsed() {
    let t = Temp::new("inline_code_padding");
    let v = t.vault();
    let block = v.parse_markdown("before `` `x` `` after".into()).remove(0);
    assert_eq!(block.text, "before `x` after");
    let code = block
        .marks
        .iter()
        .find(|m| m.kind == MarkKind::Code)
        .expect("a code mark");
    let code_text: String = block
        .text
        .chars()
        .skip(code.start as usize)
        .take((code.end - code.start) as usize)
        .collect();
    assert_eq!(code_text, "`x`");
}

#[test]
fn inline_code_padding_strips_exactly_one_space_and_preserves_exceptions() {
    let t = Temp::new("inline_code_padding_edges");
    let v = t.vault();
    for (source, expected) in [
        ("`  x  `", " x "),
        ("` x`", " x"),
        ("`x `", "x "),
        ("` `", " "),
        ("`   `", "   "),
        ("` \t `", "\t"),
    ] {
        let blocks = v.parse_markdown(format!("before {source} after"));
        assert_eq!(blocks.len(), 1, "{source:?}");
        let block = &blocks[0];
        assert_eq!(block.text, format!("before {expected} after"), "{source:?}");
        assert_eq!(block.marks, vec![Mark {
            start: 7,
            end: 7 + expected.chars().count() as u32,
            kind: MarkKind::Code,
        }], "{source:?}");
    }
}
