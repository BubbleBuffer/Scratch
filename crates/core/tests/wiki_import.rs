use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use vault_core::*;

#[test]
fn import_reports_unlinked_files_in_markdownless_folders_on_repeat() {
    let src = TempDir::new();
    src.write(
        "benchmarks/index.md",
        "# Benchmarks\n\n[runs](runs/) [ab](ab/)\n",
    );
    src.write("benchmarks/runs/result.json", "{\"elapsed\": 1}\n");
    src.write("benchmarks/ab/comparison.json", "{\"elapsed\": 2}\n");
    let dest = TempDir::new();
    let vault = dest.open();
    let (first, unlinked) = vault
        .import_markdown_reporting_unlinked(src.root(), None, vec![])
        .unwrap();
    assert_eq!(first.attachments, 0);
    assert_eq!(
        unlinked,
        [
            "benchmarks/ab/comparison.json",
            "benchmarks/runs/result.json"
        ]
    );
    let before = vault.page_versions().unwrap();
    let (second, again) = vault
        .import_markdown_reporting_unlinked(src.root(), None, vec![])
        .unwrap();
    assert_eq!(again, unlinked);
    assert_eq!(second.attachments, 0);
    assert_eq!(second.created + second.updated, 0);
    assert_eq!(second.unchanged, first.created);
    assert_eq!(vault.page_versions().unwrap(), before);
}

#[test]
fn import_reports_unlinked_files_after_legacy_import() {
    let src = TempDir::new();
    src.write("index.md", "# Benchmarks\n\n[runs](runs/)\n");
    src.write("runs/result.json", "{\"elapsed\": 1}\n");
    let dest = TempDir::new();
    let vault = dest.open();
    let legacy = vault.import_markdown(src.root(), None).unwrap();
    let before = vault.page_versions().unwrap();
    let (report, unlinked) = vault
        .import_markdown_reporting_unlinked(src.root(), None, vec![])
        .unwrap();
    assert_eq!(unlinked, ["runs/result.json"]);
    assert_eq!(report.attachments, 0);
    assert_eq!(report.created + report.updated, 0);
    assert_eq!(report.unchanged, legacy.created);
    assert_eq!(vault.page_versions().unwrap(), before);
}

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("vault_wiki_import_{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn write(&self, path: &str, body: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).unwrap()
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn imported(v: &Vault, parent: Option<String>) -> BTreeMap<String, Page> {
    let mut pages = BTreeMap::new();
    let mut pending: Vec<_> = v
        .list_children(parent, 0, 1000)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    while let Some(id) = pending.pop() {
        pending.extend(
            v.list_children(Some(id.clone()), 0, 1000)
                .unwrap()
                .into_iter()
                .map(|p| p.id),
        );
        let page = v.load_page(id).unwrap();
        if let Some(Property {
            value: PropValue::Text(path),
            ..
        }) = page
            .properties
            .iter()
            .find(|p| p.key == "$import_source_path")
        {
            pages.insert(path.clone(), page);
        }
    }
    pages
}

fn marks(blocks: &[Block]) -> Vec<&MarkKind> {
    let mut out = Vec::new();
    for block in blocks {
        out.extend(block.marks.iter().map(|mark| &mark.kind));
        out.extend(marks(&block.children));
    }
    out
}

#[test]
fn relative_links_decode_targets_keep_anchors_and_reject_ambiguous_fallbacks() {
    let source = TempDir::new();
    source.write("Home.md", "Root home.\n");
    source.write("other/Home.md", "Other home.\n");
    source.write("Café [plan].md", "Decoded file.\n");
    source.write("architecture/Home.md", "Local home.\n");
    source.write("architecture/sub/Sync.md", "[root](../../Home.md#intro) [local](../Home.md#local) [encoded](../../Caf%C3%A9%20%5Bplan%5D.md#part) [missing](../absent.md#anchor) [ambiguous](Home.md)\n\n[[Home|wiki root]] [[Absent]]\n");
    let temp = TempDir::new();
    let v = temp.open();
    let report = v.import_markdown(source.root(), None).unwrap();
    let pages = imported(&v, None);
    let links = marks(&pages["architecture/sub/Sync.md"].blocks);
    assert!(links.contains(&&MarkKind::PageLink(pages["Home.md"].id.clone())));
    assert!(links.contains(&&MarkKind::PageLink(
        pages["architecture/Home.md"].id.clone()
    )));
    assert!(links.contains(&&MarkKind::PageLink(pages["Café [plan].md"].id.clone())));
    for target in [
        "../../Home.md#intro",
        "../Home.md#local",
        "../../Caf%C3%A9%20%5Bplan%5D.md#part",
        "../absent.md#anchor",
        "Home.md",
    ] {
        assert!(
            links.contains(&&MarkKind::Link(target.to_string())),
            "target missing: {target}"
        );
    }
    assert_eq!(report.resolved_links, 4);
    assert_eq!(report.unresolved_links, 3);
    for target in ["../absent.md#anchor", "Home.md", "Absent"] {
        assert!(report
            .unresolved_targets
            .iter()
            .any(|item| item.source_file == "architecture/sub/Sync.md" && item.target == target));
    }
}

#[test]
fn folder_bodies_are_consumed_once_with_adjacent_index_readme_precedence() {
    let source = TempDir::new();
    source.write("Home.md", "Home.\n");
    source.write("adjacent.md", "Adjacent wins.\n");
    source.write("adjacent/index.md", "Index remains a child.\n");
    source.write("indexed/index.md", "[Home](../Home.md#intro)\n");
    source.write("indexed/README.md", "README remains a child.\n");
    source.write("readme/README.md", "Readme body.\n");
    let temp = TempDir::new();
    let v = temp.open();
    let report = v.import_markdown(source.root(), None).unwrap();
    let pages = imported(&v, None);
    assert_eq!(report.created, 6);
    assert_eq!(pages["adjacent/"].blocks[0].text, "Adjacent wins.");
    assert_eq!(pages["readme/"].blocks[0].text, "Readme body.");
    assert!(!pages.contains_key("indexed/index.md"));
    assert!(!pages.contains_key("readme/README.md"));
    assert!(!pages.contains_key("adjacent.md"));
    assert!(pages.contains_key("adjacent/index.md"));
    assert!(pages.contains_key("indexed/README.md"));
    assert!(marks(&pages["indexed/"].blocks)
        .contains(&&MarkKind::PageLink(pages["Home.md"].id.clone())));
}

#[test]
fn reimports_preserve_ids_versions_timestamps_and_handmade_pages() {
    let source = TempDir::new();
    source.write("Home.md", "---\ntitle: Home\nstatus: draft\n---\n[Sync](arch/Sync.md#start)\n\n![diagram](arch/diagram.png)\n\n| Task | State |\n| --- | --- |\n| Sync | Ready |\n");
    source.write("arch/index.md", "Architecture body.\n");
    source.write("arch/Sync.md", "Original sync.\n");
    source.write("arch/Old.md", "Retain when removed.\n");
    source.write("arch/diagram.png", "fake png");
    let temp = TempDir::new();
    let v = temp.open();
    let parent = v.create_page(None, "Project".into()).unwrap();
    let first = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    assert_eq!(first.pages, 4);
    assert_eq!((first.tables, first.images), (1, 1));
    let handmade = v
        .create_page(Some(parent.clone()), "Personal note".into())
        .unwrap();
    let original = imported(&v, Some(parent.clone()));
    let handmade_page = v.load_page(handmade.clone()).unwrap();
    v.flush().unwrap();
    let versions = v.page_versions().unwrap();
    let sequence = v.poll_changes(0).seq;
    let second = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    assert_eq!(
        (
            second.pages,
            second.created,
            second.updated,
            second.unchanged
        ),
        (0, 0, 0, 4)
    );
    assert_eq!(second.attachments, 1);
    assert_eq!(second.top_pages, first.top_pages);
    assert_eq!(original, imported(&v, Some(parent.clone())));
    assert_eq!(v.page_versions().unwrap(), versions);
    assert!(v.poll_changes(sequence).changes.is_empty());
    v.flush().unwrap();
    assert_eq!(v.page_versions().unwrap(), versions);
    drop(v);
    let v = temp.open();
    let reopened = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    assert_eq!(
        (reopened.created, reopened.updated, reopened.unchanged),
        (0, 0, 4)
    );
    assert_eq!(v.page_versions().unwrap(), versions);

    source.write(
        "arch/Sync.md",
        "---\ntitle: Revised sync\n---\nChanged sync.\n",
    );
    fs::remove_file(source.0.join("arch/Old.md")).unwrap();
    let third = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    assert_eq!(
        (
            third.created,
            third.updated,
            third.unchanged,
            third.orphaned
        ),
        (0, 1, 2, 1)
    );
    assert_eq!(third.orphaned_sources, ["arch/Old.md"]);
    let after = imported(&v, Some(parent));
    assert_eq!(after["arch/Sync.md"].id, original["arch/Sync.md"].id);
    assert_eq!(
        after["arch/Sync.md"].blocks[0].id,
        original["arch/Sync.md"].blocks[0].id
    );
    assert_eq!(after["arch/Sync.md"].title, "Revised sync");
    assert_eq!(after["arch/Old.md"], original["arch/Old.md"]);
    assert_eq!(v.load_page(handmade).unwrap(), handmade_page);
}

#[test]
fn import_scopes_do_not_crossmatch_and_missing_frontmatter_properties_are_removed() {
    let source = TempDir::new();
    source.write("Page.md", "---\nstatus: draft\n---\nBody.\n");
    let temp = TempDir::new();
    let v = temp.open();
    let root = v.import_markdown(source.root(), None).unwrap().top_pages[0].clone();
    let nested = v
        .import_markdown(source.root(), Some(root.clone()))
        .unwrap()
        .top_pages[0]
        .clone();
    assert_ne!(root, nested);
    let again = v.import_markdown(source.root(), None).unwrap();
    assert_eq!(again.top_pages, [root.clone()]);
    assert_eq!(again.unchanged, 1);
    source.write("Page.md", "Body.\n");
    let updated = v.import_markdown(source.root(), None).unwrap();
    assert_eq!(updated.updated, 1);
    assert!(!v
        .load_page(root)
        .unwrap()
        .properties
        .iter()
        .any(|p| p.key == "status"));
    assert!(v
        .load_page(nested)
        .unwrap()
        .properties
        .iter()
        .any(|p| p.key == "status"));
}

#[test]
fn reimport_restores_source_parent_and_sibling_order_without_replacing_pages() {
    let source = TempDir::new();
    source.write("arch/A.md", "Alpha.\n");
    source.write("arch/B.md", "Beta.\n");
    source.write("other/C.md", "Gamma.\n");
    let temp = TempDir::new();
    let v = temp.open();
    let parent = v.create_page(None, "Project".into()).unwrap();
    v.import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    let original = imported(&v, Some(parent.clone()));
    let arch = original["arch/"].id.clone();
    let a = original["arch/A.md"].id.clone();
    let b = original["arch/B.md"].id.clone();
    let other = original["other/"].id.clone();
    let handmade = v
        .create_page(Some(arch.clone()), "Personal".into())
        .unwrap();
    let handmade_before = v.load_page(handmade.clone()).unwrap();
    v.move_page(b.clone(), Some(arch.clone()), None).unwrap();
    v.move_page(a.clone(), Some(other), None).unwrap();
    let restored = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    assert_eq!(restored.created, 0);
    assert!(restored.updated >= 1);
    assert_eq!(v.load_page(a.clone()).unwrap().parent, Some(arch.clone()));
    let siblings: Vec<_> = v
        .list_children(Some(arch), 0, 100)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .filter(|id| id == &a || id == &b)
        .collect();
    assert_eq!(siblings, [a, b]);
    for (path, page) in imported(&v, Some(parent.clone())) {
        assert_eq!(page.id, original[&path].id);
    }
    assert_eq!(v.load_page(handmade).unwrap(), handmade_before);
    v.flush().unwrap();
    let versions = v.page_versions().unwrap();
    let seq = v.poll_changes(0).seq;
    let repeat = v.import_markdown(source.root(), Some(parent)).unwrap();
    assert_eq!(
        (repeat.created, repeat.updated, repeat.unchanged),
        (0, 0, 5)
    );
    assert_eq!(v.page_versions().unwrap(), versions);
    assert!(v.poll_changes(seq).changes.is_empty());
}

#[test]
fn secrets_and_exclusions_are_reported_and_cannot_be_attachment_fallbacks() {
    let source = TempDir::new();
    source.write("Home.md", "![blocked](private/diagram.png)\n\n![allowed](public/diagram.png)\n\n[credentials](credentials.json)\n\n![dot](.hidden/diagram.png)\n");
    source.write("public/diagram.png", "public");
    source.write("private/diagram.png", "private");
    source.write(".hidden/diagram.png", "hidden");
    source.write("draft.md", "Skip this.\n");
    source.write("nested/skip-one.md", "Skip glob.\n");
    for secret in [
        ".env",
        "credentials.json",
        "id_rsa",
        "key.pem",
        "key.key",
        "key.p12",
        "key.keystore",
        "key.jks",
    ] {
        source.write(secret, "test secret");
    }
    let temp = TempDir::new();
    let v = temp.open();
    let report = v
        .import_markdown_with_excludes(
            source.root(),
            None,
            vec!["private/".into(), "draft.md".into(), "**/skip-*.md".into()],
        )
        .unwrap();
    assert_eq!(report.attachments, 1);
    assert_eq!(report.pages, 1);
    assert_eq!(
        report.images, 1,
        "only successfully parsed image blocks are counted"
    );
    for secret in [
        ".env",
        "credentials.json",
        "id_rsa",
        "key.pem",
        "key.key",
        "key.p12",
        "key.keystore",
        "key.jks",
    ] {
        assert!(
            report
                .skipped_files
                .iter()
                .any(|s| s.source_file == secret && s.reason.contains("secret")),
            "missing skip: {secret}"
        );
    }
    for excluded in ["private/", "draft.md", "nested/skip-one.md"] {
        assert!(report
            .skipped_files
            .iter()
            .any(|s| s.source_file == excluded && s.reason.contains("excluded")));
    }
    assert!(report
        .skipped_files
        .iter()
        .any(|s| s.source_file == ".hidden/"));
    let home = v.load_page(report.top_pages[0].clone()).unwrap();
    let targets: Vec<_> = home
        .blocks
        .iter()
        .filter_map(|b| b.target.as_ref())
        .collect();
    assert_eq!(targets.len(), 1);
    assert_eq!(
        fs::read(v.attachment_path(targets[0].clone()).unwrap()).unwrap(),
        b"public"
    );
}

#[test]
fn excluded_markdown_targets_do_not_fall_back_to_another_page_with_that_name() {
    let source = TempDir::new();
    source.write(
        "Home.md",
        "[blocked](private/Plan.md) [allowed](public/Plan.md)\n",
    );
    source.write("private/Plan.md", "Excluded plan.\n");
    source.write("public/Plan.md", "Allowed plan.\n");
    source.write("arch/Home.md", "[relative](private/Plan.md)\n");
    source.write("arch/private/Plan.md", "Allowed local plan.\n");
    let temp = TempDir::new();
    let v = temp.open();
    let report = v
        .import_markdown_with_excludes(source.root(), None, vec!["private/".into()])
        .unwrap();
    let pages = imported(&v, None);
    assert_eq!((report.resolved_links, report.unresolved_links), (2, 1));
    let home_marks = marks(&pages["Home.md"].blocks);
    assert_eq!(
        home_marks
            .iter()
            .filter(|mark| matches!(mark, MarkKind::PageLink(_)))
            .count(),
        1
    );
    assert!(home_marks.contains(&&MarkKind::Link("private/Plan.md".into())));
    assert!(home_marks.contains(&&MarkKind::PageLink(pages["public/Plan.md"].id.clone())));
    assert!(
        marks(&pages["arch/Home.md"].blocks).contains(&&MarkKind::PageLink(
            pages["arch/private/Plan.md"].id.clone()
        ))
    );
    assert!(report
        .unresolved_targets
        .iter()
        .any(|target| target.source_file == "Home.md" && target.target == "private/Plan.md"));
}

#[test]
fn folder_body_basename_fallback_requires_a_unique_page() {
    let source = TempDir::new();
    source.write("Home.md", "[body](index.md)\n");
    source.write("arch/index.md", "Architecture body.\n");
    let temp = TempDir::new();
    let v = temp.open();
    let first = v.import_markdown(source.root(), None).unwrap();
    assert_eq!((first.resolved_links, first.unresolved_links), (1, 0));
    let pages = imported(&v, None);
    assert!(
        marks(&pages["Home.md"].blocks).contains(&&MarkKind::PageLink(pages["arch/"].id.clone()))
    );
    source.write("other/index.md", "Other body.\n");
    let second = v.import_markdown(source.root(), None).unwrap();
    assert_eq!((second.resolved_links, second.unresolved_links), (0, 1));
    assert_eq!(second.unresolved_targets[0].target, "index.md");
}

#[test]
fn reimport_finds_its_scoped_page_after_a_move_outside_the_parent() {
    let source = TempDir::new();
    source.write("Page.md", "Source body.\n");
    let temp = TempDir::new();
    let v = temp.open();
    let parent = v.create_page(None, "Import parent".into()).unwrap();
    let outside = v.create_page(None, "Other parent".into()).unwrap();
    let page = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap()
        .top_pages[0]
        .clone();
    v.move_page(page.clone(), Some(outside.clone()), None)
        .unwrap();
    let untouched = v.load_page(outside).unwrap();
    let report = v
        .import_markdown(source.root(), Some(parent.clone()))
        .unwrap();
    assert_eq!((report.created, report.updated, report.orphaned), (0, 1, 0));
    assert_eq!(report.top_pages, [page.clone()]);
    assert_eq!(v.load_page(page).unwrap().parent, Some(parent.clone()));
    assert_eq!(v.load_page(untouched.id.clone()).unwrap(), untouched);
    let versions = v.page_versions().unwrap();
    assert_eq!(
        v.import_markdown(source.root(), Some(parent))
            .unwrap()
            .unchanged,
        1
    );
    assert_eq!(v.page_versions().unwrap(), versions);
}
