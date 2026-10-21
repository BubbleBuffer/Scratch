//! Page roles through markdown: the feedback page and its child report
//! survive export and a fresh import, a known `role:` becomes the page role,
//! an unknown one stays an ordinary property, and a property named `role`
//! is escaped as `prop:role`.

use std::{fs, path::PathBuf};

use vault_core::*;

/// A real PNG, from the app's icon set.
const PNG: &[u8] = include_bytes!("../../../app/assets/icons/app_icon_64.png");

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        Temp(std::env::temp_dir().join(format!("notes_feedback_{tag}_{}", new_id())))
    }
    /// A vault in a subfolder of the temp root.
    fn vault(&self, name: &str) -> Vault {
        Vault::open(self.0.join(name).to_string_lossy().into_owned()).unwrap()
    }
    /// A folder under the temp root, created.
    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Drop for Temp {
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

/// Inserts a top-level block after `after` (or first when `None`).
fn ins(after: Option<&str>, block: NewBlock) -> BlockOp {
    BlockOp::Insert {
        parent: None,
        after: after.map(Into::into),
        block,
    }
}

/// The id of a top-level page with this title.
fn titled(v: &Vault, title: &str) -> PageId {
    v.list_pages(None, 0, 100)
        .unwrap()
        .into_iter()
        .find(|p| p.title == title)
        .unwrap_or_else(|| panic!("no page titled {title}"))
        .id
}

/// Whether a frontmatter line is the escaped property `prop:role`. The key
/// is quoted, since it contains a colon: `"prop:role": "custom"`.
fn is_role_prop_line(line: &str) -> bool {
    let key = line.strip_prefix('"').unwrap_or(line);
    key.starts_with("prop:role\":") || key.starts_with("prop:role:")
}

/// Whether the page has an ordinary property `key` with this text value.
fn has_text_prop(v: &Vault, page: &PageId, key: &str, text: &str) -> bool {
    v.load_page(page.clone())
        .unwrap()
        .properties
        .iter()
        .any(|p| p.key == key && p.value == PropValue::Text(text.to_string()))
}

#[test]
fn feedback_page_and_child_report_round_trip() {
    let t = Temp::new("roundtrip");
    let src = t.vault("source");
    let fb = src.feedback_page().unwrap();
    let title = src.load_page(fb.clone()).unwrap().title;
    let report = src.create_page(Some(fb.clone()), "Report".into()).unwrap();
    let att = src
        .add_attachment_bytes("app_icon_64.png".into(), PNG.to_vec())
        .unwrap();
    src.apply(
        report.clone(),
        vec![
            ins(None, nb("p", BlockKind::Paragraph, "Paragraph text")),
            ins(Some("p"), nb("i", BlockKind::Image, "Pixel caption")),
            BlockOp::SetTarget {
                id: "i".into(),
                target: Some(att.clone()),
            },
            ins(Some("i"), nb("c", BlockKind::Code, "let x = 1;")),
            BlockOp::SetTarget {
                id: "c".into(),
                target: Some("text".into()),
            },
        ],
    )
    .unwrap();

    let fb_md = src.export_markdown(fb.clone()).unwrap();
    let report_md = src.export_markdown(report.clone()).unwrap();
    assert!(fb_md.contains("\nrole: feedback\n"), "{fb_md}");
    assert!(
        report_md.contains(&format!("\nparent: {fb}\n")),
        "{report_md}"
    );
    assert!(
        report_md.contains("```text\nlet x = 1;\n```"),
        "{report_md}"
    );
    assert!(report_md.contains(&att), "{report_md}");

    // The files as an export lays them out: the feedback page beside its
    // folder, the report in it, and the image blob the report links to.
    let md = t.dir("md");
    fs::write(md.join(format!("{title}.md")), &fb_md).unwrap();
    let children = t.dir(&format!("md/{title}"));
    fs::write(children.join("Report.md"), &report_md).unwrap();
    fs::write(children.join(&att), PNG).unwrap();

    let dst = t.vault("fresh");
    dst.import_markdown(md.to_string_lossy().into_owned(), None)
        .unwrap();

    // The feedback page: role from the file, hidden from the top level.
    let fb2 = dst
        .find_by_role("feedback".into())
        .unwrap()
        .expect("feedback role after import");
    assert_eq!(dst.load_page(fb2.clone()).unwrap().title, title);
    let top = dst.list_pages(None, 0, 100).unwrap();
    assert!(!top.iter().any(|p| p.id == fb2), "feedback page is visible");

    // The report: a child of the feedback page, hidden with it.
    let rep2 = dst
        .list_pages(Some(fb2.clone()), 0, 100)
        .unwrap()
        .into_iter()
        .find(|p| p.title == "Report")
        .expect("report is a child of the feedback page")
        .id;
    assert_eq!(
        dst.load_page(rep2.clone()).unwrap().parent,
        Some(fb2.clone()),
        "report lost its parent"
    );
    assert!(!top.iter().any(|p| p.id == rep2), "report is visible");

    let page = dst.load_page(rep2.clone()).unwrap();
    let para = page
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Paragraph)
        .unwrap();
    assert_eq!(para.text, "Paragraph text");
    let image = page
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Image)
        .unwrap();
    assert_eq!(image.text, "Pixel caption");
    let stored = image.target.clone().expect("image attachment");
    assert_eq!(stored, att, "attachment id changed on import");
    let bytes = fs::read(dst.attachment_path(stored).unwrap()).unwrap();
    assert_eq!(bytes, PNG, "attachment bytes changed on import");
    let code = page
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Code)
        .unwrap();
    assert_eq!(code.text, "let x = 1;");
    assert_eq!(code.target.as_deref(), Some("text"));

    // Exported again, the role is still written.
    let again = dst.export_markdown(fb2).unwrap();
    assert!(again.contains("\nrole: feedback\n"), "{again}");

    // Hidden: nothing from the feedback subtree counts as a visible page or
    // turns up in search, even after a flush.
    dst.flush().unwrap();
    assert_eq!(dst.page_count().unwrap(), 0, "feedback pages are counted");
    assert!(
        dst.search_full("Paragraph".into(), 10).unwrap().is_empty(),
        "report text is searchable"
    );
    assert!(
        dst.search("rprt".into(), 10).unwrap().is_empty(),
        "report title is searchable"
    );
}

#[test]
fn known_roles_import_and_unknown_role_stays_a_property() {
    let t = Temp::new("roles");
    let md = t.dir("md");
    fs::write(md.join("Daily.md"), "---\nrole: journal\n---\nDay\n").unwrap();
    fs::write(md.join("Cal.md"), "---\nrole: calendars\n---\nCal\n").unwrap();
    fs::write(md.join("Nodes.md"), "---\nrole: nodes\n---\nNodes\n").unwrap();
    fs::write(md.join("Custom.md"), "---\nrole: custom\n---\nCustom\n").unwrap();
    fs::write(
        md.join("Escaped.md"),
        "---\nprop:role: escaped\n---\nEscaped\n",
    )
    .unwrap();

    let v = t.vault("roles");
    v.import_markdown(md.to_string_lossy().into_owned(), None)
        .unwrap();

    for (role, title) in [
        ("journal", "Daily"),
        ("calendars", "Cal"),
        ("nodes", "Nodes"),
    ] {
        let id = v
            .find_by_role(role.into())
            .unwrap()
            .unwrap_or_else(|| panic!("no page with role {role}"));
        assert_eq!(v.load_page(id).unwrap().title, title);
    }

    // An unknown value is a property named `role`, not a page role.
    let custom = titled(&v, "Custom");
    assert_eq!(v.find_by_role("custom".into()).unwrap(), None);
    assert!(has_text_prop(&v, &custom, "role", "custom"));
    let exported = v.export_markdown(custom).unwrap();
    assert!(exported.lines().any(is_role_prop_line), "{exported}");
    assert!(
        !exported.lines().any(|l| l.starts_with("role:")),
        "{exported}"
    );

    // `prop:role` is the same ordinary property, and is not a page role.
    let escaped = titled(&v, "Escaped");
    assert!(has_text_prop(&v, &escaped, "role", "escaped"));
    assert_eq!(v.find_by_role("escaped".into()).unwrap(), None);

    // Export the known roles and the escaped properties, then import them
    // into another fresh vault: roles come back as roles, not properties.
    let round = t.dir("md2");
    for (role, title) in [
        ("journal", "Daily"),
        ("calendars", "Cal"),
        ("nodes", "Nodes"),
    ] {
        let id = v
            .find_by_role(role.into())
            .unwrap()
            .unwrap_or_else(|| panic!("no page with role {role}"));
        let text = v.export_markdown(id).unwrap();
        assert!(text.contains(&format!("\nrole: {role}\n")), "{text}");
        fs::write(round.join(format!("{title}.md")), text).unwrap();
    }
    for title in ["Custom", "Escaped"] {
        let text = v.export_markdown(titled(&v, title)).unwrap();
        fs::write(round.join(format!("{title}.md")), text).unwrap();
    }

    let w = t.vault("roles2");
    w.import_markdown(round.to_string_lossy().into_owned(), None)
        .unwrap();
    for (role, title) in [
        ("journal", "Daily"),
        ("calendars", "Cal"),
        ("nodes", "Nodes"),
    ] {
        let id = w
            .find_by_role(role.into())
            .unwrap()
            .unwrap_or_else(|| panic!("no page with role {role} after reimport"));
        assert_eq!(w.load_page(id).unwrap().title, title);
    }
    assert_eq!(w.find_by_role("custom".into()).unwrap(), None);
    assert!(has_text_prop(&w, &titled(&w, "Custom"), "role", "custom"));
    assert!(has_text_prop(&w, &titled(&w, "Escaped"), "role", "escaped"));
}

#[test]
fn rescan_applies_a_role_edited_into_the_file() {
    let t = Temp::new("rescan");
    let md = t.dir("md");
    fs::write(md.join("Custom.md"), "---\nrole: custom\n---\nCustom\n").unwrap();

    let v = t.vault("rescan");
    v.import_markdown(md.to_string_lossy().into_owned(), None)
        .unwrap();
    v.flush().unwrap();
    let custom = titled(&v, "Custom");
    assert!(has_text_prop(&v, &custom, "role", "custom"));

    // The page's file, as the app wrote it: the property is `prop:role`.
    let pages = t.0.join("rescan").join("pages");
    let file = fs::read_dir(&pages)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{custom}.md")))
        .expect("markdown file");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.lines().any(is_role_prop_line), "{text}");
    let edited: String = text
        .lines()
        .map(|l| {
            if is_role_prop_line(l) {
                "role: journal".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&file, edited).unwrap();

    assert_eq!(v.rescan().unwrap(), 1);
    assert_eq!(
        v.find_by_role("journal".into()).unwrap(),
        Some(custom.clone())
    );
    let page = v.load_page(custom).unwrap();
    assert!(!page.properties.iter().any(|p| p.key == "role"));
}
