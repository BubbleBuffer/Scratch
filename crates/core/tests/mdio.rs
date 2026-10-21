//! Markdown in and out of pages (`mdio.rs`): used by the MCP server and the
//! agent runner.

use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> TempVault {
        TempVault(std::env::temp_dir().join(format!("vault_mdio_test_{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn kinds(p: &Page) -> Vec<BlockKind> {
    p.blocks.iter().map(|b| b.kind.clone()).collect()
}

#[test]
fn markdown_round_trip_and_links() {
    let t = TempVault::new();
    let v = t.open();
    let other = v.create_page(None, "Other page".into()).unwrap();
    let p = v.create_page(None, "Notes".into()).unwrap();
    let ids = v
        .insert_markdown(
            p.clone(),
            None,
            None,
            true,
            "# Plan\n\nSee [[Other page]] and **bold**.\n\n- [ ] one\n- [x] two\n\n- a\n  - nested"
                .into(),
        )
        .unwrap();
    assert_eq!(ids.len(), 5);
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(
        kinds(&page),
        [
            BlockKind::Heading1,
            BlockKind::Paragraph,
            BlockKind::Todo,
            BlockKind::Todo,
            BlockKind::Bulleted,
        ]
    );
    // The wikilink resolved to the page.
    assert!(page.blocks[1]
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::PageLink(other.clone())));
    assert!(page.blocks[3].checked);
    assert_eq!(page.blocks[4].children.len(), 1);
    let md = v.page_markdown(p.clone()).unwrap();
    assert!(md.starts_with("# Plan"), "{md}");
    assert!(md.contains("[[Other page]]"), "{md}");
    assert!(md.contains("- [x] two"), "{md}");
    assert!(md.contains("  - nested"), "{md}");
    v.flush().unwrap();
    assert_eq!(v.backlinks(other).unwrap().len(), 1);
}

#[test]
fn insert_positions() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "P".into()).unwrap();
    let a = v
        .insert_markdown(p.clone(), None, None, true, "A\n\nC".into())
        .unwrap();
    v.insert_markdown(p.clone(), None, Some(a[0].clone()), false, "B".into())
        .unwrap();
    v.insert_markdown(p.clone(), None, None, false, "Start".into())
        .unwrap();
    v.insert_markdown(p.clone(), None, None, true, "End\n\nEnd 2".into())
        .unwrap();
    let texts: Vec<String> = v
        .load_page(p)
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert_eq!(texts, ["Start", "A", "B", "C", "End", "End 2"]);
    assert!(v
        .insert_markdown(new_id(), None, None, true, "x".into())
        .is_err());
}

#[test]
fn replace_whole_page_keeps_unchanged_ids() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "P".into()).unwrap();
    let ids = v
        .insert_markdown(p.clone(), None, None, true, "One\n\nTwo\n\nThree".into())
        .unwrap();
    v.replace_markdown(
        p.clone(),
        vec![],
        "One\n\nTwo changed\n\nThree\n\nFour".into(),
    )
    .unwrap();
    let page = v.load_page(p.clone()).unwrap();
    let texts: Vec<&str> = page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, ["One", "Two changed", "Three", "Four"]);
    assert_eq!(page.blocks[0].id, ids[0]);
    assert_eq!(page.blocks[2].id, ids[2]);
}

#[test]
fn replace_listed_blocks() {
    let t = TempVault::new();
    let v = t.open();
    let p = v.create_page(None, "P".into()).unwrap();
    let ids = v
        .insert_markdown(
            p.clone(),
            None,
            None,
            true,
            "One\n\nTwo\n\nThree\n\nFour".into(),
        )
        .unwrap();
    let new = v
        .replace_markdown(
            p.clone(),
            vec![ids[1].clone(), ids[2].clone()],
            "## New\n\n- x".into(),
        )
        .unwrap();
    assert_eq!(new.len(), 2);
    let page = v.load_page(p.clone()).unwrap();
    let texts: Vec<&str> = page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, ["One", "New", "x", "Four"]);
    assert_eq!(page.blocks[1].kind, BlockKind::Heading2);
    // Empty markdown deletes; unknown ids fail without changing anything.
    v.replace_markdown(p.clone(), vec![ids[3].clone()], String::new())
        .unwrap();
    assert!(v
        .replace_markdown(p.clone(), vec!["nope".into()], "x".into())
        .is_err());
    assert_eq!(v.load_page(p).unwrap().blocks.len(), 3);
}

#[test]
fn owner_flag() {
    let t = TempVault::new();
    let a = t.open();
    assert!(a.is_owner());
    let b = t.open();
    assert!(!b.is_owner());
    drop(b);
    drop(a);
    assert!(t.open().is_owner());
}
