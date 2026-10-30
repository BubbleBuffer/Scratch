//! Merge proposals: validation, dedup, dismissal, the merge and its undo.
use std::{fs, path::PathBuf, thread::sleep, time::Duration};
use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault-triage-{}", new_id())))
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

/// A page with one paragraph, created a moment after the previous one so the
/// creation times differ.
fn page(v: &Vault, parent: Option<&str>, title: &str, text: &str) -> String {
    sleep(Duration::from_millis(5));
    let id = v.create_page(parent.map(String::from), title.into()).unwrap();
    add_paragraph(v, &id, text);
    id
}

fn add_paragraph(v: &Vault, id: &str, text: &str) {
    let last = v.load_page(id.into()).unwrap().blocks.last().map(|b| b.id.clone());
    v.apply(
        id.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: last,
            block: NewBlock {
                id: new_id(),
                kind: BlockKind::Paragraph,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
}

fn texts(v: &Vault, id: &str) -> Vec<String> {
    v.load_page(id.into())
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.text.clone())
        .collect()
}

fn prop(v: &Vault, id: &str, key: &str, text: &str) {
    v.set_property(id.into(), key.into(), PropValue::Text(text.into()))
        .unwrap();
}

#[test]
fn propose_checks_the_pair() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Notes A", "alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");
    let other_root = page(&v, None, "Other project", "");
    let elsewhere = page(&v, Some(&other_root), "Notes C", "gamma");
    let typed = {
        sleep(Duration::from_millis(5));
        let id = v
            .create_page_with_type(Some(root.clone()), "Typed".into(), Some("task".into()))
            .unwrap();
        add_paragraph(&v, &id, "delta");
        id
    };
    let child = page(&v, Some(&a), "Child of A", "epsilon");
    let gone = page(&v, Some(&root), "Gone", "zeta");
    v.delete_page(gone.clone()).unwrap();

    let propose = |x: &str, y: &str| {
        v.triage_propose(x.into(), y.into(), "same thing".into(), String::new())
    };
    assert!(propose(&a, &a).is_err(), "a page with itself");
    assert!(propose(&a, &elsewhere).is_err(), "different projects");
    assert!(propose(&a, &typed).is_err(), "different page types");
    assert!(propose(&a, &child).is_err(), "page and its sub-page");
    assert!(propose(&a, &gone).is_err(), "trashed page");
    assert!(propose(&a, &"0".repeat(26)).is_err(), "unknown page");

    let p = propose(&b, &a).unwrap();
    assert_eq!(p.keep, a, "the older page is kept, whatever the order given");
    assert_eq!(p.dropped, b);
    assert_eq!(p.status, TRIAGE_PENDING);
    assert!(!p.reason.is_empty());
}

#[test]
fn a_pair_has_one_record_and_a_dismissed_pair_stays_dismissed() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Notes A", "alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");

    let first = v
        .triage_propose(a.clone(), b.clone(), "dup".into(), "task1".into())
        .unwrap();
    let again = v
        .triage_propose(b.clone(), a.clone(), "other words".into(), String::new())
        .unwrap();
    assert_eq!(first, again, "a pending pair is not proposed twice");
    assert_eq!(v.triage_proposals(true).unwrap().len(), 1);

    let dismissed = v.triage_dismiss(first.id.clone()).unwrap();
    assert_eq!(dismissed.status, TRIAGE_DISMISSED);
    assert!(v.triage_proposals(true).unwrap().is_empty());

    for (x, y) in [(&a, &b), (&b, &a)] {
        let p = v
            .triage_propose(x.clone(), y.clone(), "again".into(), String::new())
            .unwrap();
        assert_eq!(p.status, TRIAGE_DISMISSED, "a dismissed pair is not proposed again");
        assert_eq!(p.id, first.id);
    }
    assert!(v.triage_proposals(true).unwrap().is_empty());
    assert_eq!(v.triage_proposals(false).unwrap().len(), 1);
    assert!(
        v.triage_accept(first.id).is_err(),
        "a dismissed proposal is not accepted"
    );
}

#[test]
fn accept_merges_content_and_properties_and_trashes_the_newer_page() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Notes A", "alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");
    prop(&v, &a, "shared", "from A");
    prop(&v, &b, "shared", "from B");
    prop(&v, &b, "only_b", "kept");

    let p = v
        .triage_propose(a.clone(), b.clone(), "dup".into(), String::new())
        .unwrap();
    let done = v.triage_accept(p.id.clone()).unwrap();
    assert_eq!(done.status, TRIAGE_ACCEPTED);
    assert!(done.merged_version.is_some());

    let merged = v.load_page(a.clone()).unwrap();
    let kinds: Vec<_> = merged.blocks.iter().map(|b| b.kind.clone()).collect();
    assert!(kinds.contains(&BlockKind::Divider));
    let all = texts(&v, &a);
    assert_eq!(all.first().map(String::as_str), Some("alpha"));
    assert_eq!(all.last().map(String::as_str), Some("beta"));
    assert!(all.iter().any(|t| t.contains("Notes B")), "{all:?}");
    let value = |key: &str| {
        merged
            .properties
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.value.clone())
    };
    assert_eq!(value("shared"), Some(PropValue::Text("from A".into())));
    assert_eq!(value("only_b"), Some(PropValue::Text("kept".into())));
    assert!(v.load_page(b.clone()).unwrap().in_trash);
    assert!(
        v.triage_accept(p.id).is_err(),
        "an accepted proposal is not accepted twice"
    );
}

#[test]
fn undo_restores_the_kept_page_exactly_and_the_dropped_page_from_the_trash() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Notes A", "alpha");
    add_paragraph(&v, &a, "more alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");
    prop(&v, &a, "shared", "from A");
    prop(&v, &b, "only_b", "kept");
    let before = v.load_page(a.clone()).unwrap();

    let p = v
        .triage_propose(a.clone(), b.clone(), "dup".into(), String::new())
        .unwrap();
    v.triage_accept(p.id.clone()).unwrap();
    assert_ne!(v.load_page(a.clone()).unwrap().blocks, before.blocks);

    let undone = v.triage_undo(p.id.clone()).unwrap();
    assert_eq!(undone.status, TRIAGE_DISMISSED, "the pair is not proposed again");
    let after = v.load_page(a.clone()).unwrap();
    assert_eq!(after.blocks, before.blocks, "the exact pre-merge blocks");
    assert_eq!(
        format!("{:?}", after.properties),
        format!("{:?}", before.properties)
    );
    assert_eq!(after.title, before.title);
    assert!(
        !v.load_page(b.clone()).unwrap().in_trash,
        "the dropped page is out of the trash"
    );
    assert!(v.list_trash().unwrap().iter().all(|t| t.page.id != b));
    let again = v
        .triage_propose(a, b, "dup".into(), String::new())
        .unwrap();
    assert_eq!(again.status, TRIAGE_DISMISSED);
    assert!(
        v.triage_undo(p.id).is_err(),
        "a dismissed proposal has nothing to undo"
    );
}

#[test]
fn the_merge_refuses_a_page_with_sub_pages_and_returns_a_version() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Notes A", "alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");
    let _sub = page(&v, Some(&b), "Sub of B", "gamma");
    assert!(
        v.merge_pages(a.clone(), b.clone()).is_err(),
        "sub-pages would be trashed"
    );
    assert!(!v.load_page(b.clone()).unwrap().in_trash);
    assert_eq!(texts(&v, &a), vec!["alpha"]);

    let c = page(&v, Some(&root), "Notes C", "delta");
    let version = v.merge_pages(a.clone(), c.clone()).unwrap();
    assert!(!version.is_empty());
    assert!(texts(&v, &a).contains(&"delta".to_string()));
}

#[test]
fn candidates_list_the_scope_without_trashed_pages() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "root text");
    let a = page(&v, Some(&root), "Notes A", "alpha alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");
    let gone = page(&v, Some(&root), "Gone", "zeta");
    v.delete_page(gone).unwrap();
    let outside = page(&v, None, "Outside", "omega");

    let by_root = v.triage_candidates(Some(root.clone()), vec![]).unwrap();
    let ids: Vec<&str> = by_root.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec![root.as_str(), a.as_str(), b.as_str()]);
    assert_eq!(by_root[1].summary, "alpha alpha");
    let listed = v
        .triage_candidates(None, vec![a.clone(), outside.clone()])
        .unwrap();
    assert_eq!(listed.len(), 2);
}

fn def(key: &str, name: &str, kind: PropKind, config: &str) -> PropDef {
    PropDef {
        key: key.into(),
        name: name.into(),
        kind,
        options: vec![],
        config: config.into(),
    }
}

fn block(v: &Vault, page: &str, kind: BlockKind, text: &str, marks: Vec<Mark>, target: Option<&str>) {
    let id = new_id();
    let last = v.load_page(page.into()).unwrap().blocks.last().map(|b| b.id.clone());
    let mut ops = vec![BlockOp::Insert {
        parent: None,
        after: last,
        block: NewBlock {
            id: id.clone(),
            kind,
            text: text.into(),
            marks,
            checked: false,
        },
    }];
    if let Some(t) = target {
        ops.push(BlockOp::SetTarget {
            id,
            target: Some(t.into()),
        });
    }
    v.apply(page.into(), ops).unwrap();
}

fn links_in(v: &Vault, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            for m in &b.marks {
                if let MarkKind::PageLink(p) = &m.kind {
                    out.push(format!("mark:{p}"));
                }
            }
            if matches!(b.kind, BlockKind::LinkToPage | BlockKind::Embed) {
                out.push(format!("block:{}", b.target.clone().unwrap_or_default()));
            }
            walk(&b.children, out);
        }
    }
    walk(&v.load_page(id.into()).unwrap().blocks, &mut out);
    out
}

#[test]
fn accepting_repoints_every_link_and_undo_restores_the_linking_pages() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");

    // Two rows of the Tasks database: the older is kept, the newer dropped.
    let tasks = v.create_page(Some(root.clone()), "Tasks".into()).unwrap();
    v.set_schema(tasks.clone(), vec![]).unwrap();
    let a = v.create_page(Some(tasks.clone()), "Fix login".into()).unwrap();
    add_paragraph(&v, &a, "alpha");
    sleep(Duration::from_millis(5));
    let b = v.create_page(Some(tasks.clone()), "Login fix".into()).unwrap();
    add_paragraph(&v, &b, "beta");

    // A row of another database relates to the dropped row (and to the kept
    // one: the result must hold the kept row once).
    let plans = v.create_page(Some(root.clone()), "Plans".into()).unwrap();
    let rel = serde_json::json!({ "relation": tasks }).to_string();
    v.set_schema(plans.clone(), vec![def("tasks", "Tasks", PropKind::Relation, &rel)])
        .unwrap();
    let plan_row = v.create_page(Some(plans.clone()), "Release".into()).unwrap();
    v.set_property(
        plan_row.clone(),
        "tasks".into(),
        PropValue::Relation(vec![a.clone(), b.clone()]),
    )
    .unwrap();
    let plan_only_b = v.create_page(Some(plans.clone()), "Hotfix".into()).unwrap();
    v.set_property(
        plan_only_b.clone(),
        "tasks".into(),
        PropValue::Relation(vec![b.clone()]),
    )
    .unwrap();

    // Pages linking to the dropped row: an inline link, a link-to-page block,
    // an embed, a link nested in a toggle, and the kept page itself.
    let inline = page(&v, Some(&root), "Notes", "see the login fix");
    let marks = vec![Mark {
        start: 8,
        end: 17,
        kind: MarkKind::PageLink(b.clone()),
    }];
    let first = v.load_page(inline.clone()).unwrap().blocks[0].id.clone();
    v.apply(
        inline.clone(),
        vec![BlockOp::SetText {
            id: first,
            text: "see the login fix".into(),
            marks,
        }],
    )
    .unwrap();
    let blocklink = page(&v, Some(&root), "Index", "");
    block(&v, &blocklink, BlockKind::LinkToPage, "", vec![], Some(&b));
    let embedder = page(&v, Some(&root), "Board", "");
    block(&v, &embedder, BlockKind::Embed, "", vec![], Some(&b));
    let keeps_link = page(&v, Some(&root), "Unrelated", "x");
    block(&v, &keeps_link, BlockKind::LinkToPage, "", vec![], Some(&a)); // not touched
    block(&v, &a, BlockKind::LinkToPage, "", vec![], Some(&b));

    let snap = |id: &str| {
        let p = v.load_page(id.into()).unwrap();
        (p.blocks, format!("{:?}", p.properties))
    };
    let pages = [&a, &plan_row, &plan_only_b, &inline, &blocklink, &embedder, &keeps_link];
    let before: Vec<_> = pages.iter().map(|p| snap(p)).collect();

    let p = v
        .triage_propose(a.clone(), b.clone(), "dup".into(), String::new())
        .unwrap();
    let done = v.triage_accept(p.id.clone()).unwrap();
    assert_eq!(done.status, TRIAGE_ACCEPTED);

    // Nothing points at the dropped row any more.
    assert_eq!(links_in(&v, &inline), vec![format!("mark:{a}")]);
    assert_eq!(links_in(&v, &blocklink), vec![format!("block:{a}")]);
    assert_eq!(links_in(&v, &embedder), vec![format!("block:{a}")]);
    assert!(links_in(&v, &a).iter().all(|l| !l.contains(&b)), "{:?}", links_in(&v, &a));
    let rel_of = |id: &str| {
        v.load_page(id.into())
            .unwrap()
            .properties
            .into_iter()
            .find(|p| p.key == "tasks")
            .map(|p| p.value)
    };
    assert_eq!(rel_of(&plan_row), Some(PropValue::Relation(vec![a.clone()])));
    assert_eq!(rel_of(&plan_only_b), Some(PropValue::Relation(vec![a.clone()])));
    assert!(v.backlinks(b.clone()).unwrap().is_empty(), "no live page links to it");
    assert_eq!(links_in(&v, &keeps_link), vec![format!("block:{a}")], "other links stay");
    let mut recorded: Vec<_> = done.repointed.iter().map(|r| r.page.clone()).collect();
    recorded.sort();
    let mut want = vec![plan_row.clone(), plan_only_b.clone(), inline.clone(), blocklink.clone(), embedder.clone()];
    want.sort();
    assert_eq!(recorded, want, "the kept page restores from its own version");

    v.triage_undo(p.id.clone()).unwrap();
    for (id, was) in pages.iter().zip(before) {
        assert_eq!(snap(id), was, "page {id} is exactly as before the merge");
    }
    assert!(!v.load_page(b.clone()).unwrap().in_trash);
}

#[test]
fn a_merge_without_links_records_no_repointed_pages() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, None, "Project", "");
    let a = page(&v, Some(&root), "Notes A", "alpha");
    let b = page(&v, Some(&root), "Notes B", "beta");
    let p = v
        .triage_propose(a, b, "dup".into(), String::new())
        .unwrap();
    assert!(v.triage_accept(p.id).unwrap().repointed.is_empty());
}
