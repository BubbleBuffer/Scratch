//! Templates for the new children of any page.

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

fn para(id: &str, text: &str, after: Option<&str>) -> BlockOp {
    BlockOp::Insert {
        parent: None,
        after: after.map(Into::into),
        block: NewBlock {
            id: id.into(),
            kind: BlockKind::Paragraph,
            text: text.into(),
            marks: vec![],
            checked: false,
        },
    }
}

fn texts(v: &Vault, id: &str) -> Vec<String> {
    v.load_page(id.into())
        .unwrap()
        .blocks
        .into_iter()
        .map(|b| b.text)
        .collect()
}

fn ids(s: &[PageSummary]) -> Vec<String> {
    s.iter().map(|p| p.id.clone()).collect()
}

#[test]
fn templates_of_a_plain_page_apply_to_new_children() {
    let t = TempDir::new("templates");
    let v = t.open();
    let meetings = v.create_page(None, "Meetings".into()).unwrap();
    let notes = v
        .create_page(Some(meetings.clone()), "Kickoff".into())
        .unwrap();
    let standup = v
        .create_template(meetings.clone(), "Standup".into())
        .unwrap();
    v.apply(
        standup.clone(),
        vec![para("a", "Yesterday", None), para("b", "Today", Some("a"))],
    )
    .unwrap();
    v.set_icon(standup.clone(), Some("☕".into())).unwrap();
    let retro = v.create_template(meetings.clone(), "Retro".into()).unwrap();
    v.apply(retro.clone(), vec![para("c", "What went well", None)])
        .unwrap();

    // Templates are listed for the parent and left out everywhere else.
    assert_eq!(
        ids(&v.list_templates(meetings.clone()).unwrap()),
        [standup.clone(), retro.clone()]
    );
    let kids = v.list_children(Some(meetings.clone()), 0, 50).unwrap();
    assert_eq!(ids(&kids), std::slice::from_ref(&notes));
    let parent = v.list_children(None, 0, 50).unwrap();
    assert_eq!(parent[0].child_count, 1);
    assert!(v
        .list_pages(None, 0, 50)
        .unwrap()
        .iter()
        .all(|p| p.id != standup && p.id != retro));
    v.flush().unwrap();
    assert!(v.search("yesterday".into(), 10).unwrap().is_empty());
    assert!(v
        .query(None, None, vec![], 0, 50)
        .unwrap()
        .rows
        .iter()
        .all(|p| p.id != standup));

    // No default yet: a new child starts empty.
    let plain = v
        .create_page(Some(meetings.clone()), "Plain".into())
        .unwrap();
    assert!(texts(&v, &plain).is_empty());
    // A chosen template, then the default.
    let r = v
        .create_page_from(meetings.clone(), "Retro 1".into(), Some(retro.clone()))
        .unwrap();
    assert_eq!(texts(&v, &r), ["What went well"]);
    v.set_default_template(meetings.clone(), Some(standup.clone()))
        .unwrap();
    let s = v
        .create_page(Some(meetings.clone()), "Monday".into())
        .unwrap();
    let p = v.load_page(s.clone()).unwrap();
    assert_eq!(p.title, "Monday");
    assert_eq!(p.icon.as_deref(), Some("☕"));
    assert_eq!(texts(&v, &s), ["Yesterday", "Today"]);
    let e = v
        .create_page_from(meetings.clone(), "Empty".into(), None)
        .unwrap();
    assert!(texts(&v, &e).is_empty());

    // The picker: a template applied to a new page with an empty paragraph
    // replaces it; applied to a page with text, it follows the text.
    v.apply(plain.clone(), vec![para("x", "", None)]).unwrap();
    v.apply_template(plain.clone(), retro.clone()).unwrap();
    assert_eq!(texts(&v, &plain), ["What went well"]);
    v.apply_template(e.clone(), standup.clone()).unwrap();
    v.apply_template(e.clone(), retro.clone()).unwrap();
    assert_eq!(texts(&v, &e), ["Yesterday", "Today", "What went well"]);
    // Only the parent's templates apply, and only templates.
    let other = v.create_page(None, "Other".into()).unwrap();
    let o = v.create_page(Some(other), "Child".into()).unwrap();
    assert!(v.apply_template(o, retro.clone()).is_err());
    assert!(v
        .create_page_from(meetings.clone(), "x".into(), Some(notes.clone()))
        .is_err());

    // Unmarking the default makes it a plain page and clears the default.
    v.set_template(standup.clone(), false).unwrap();
    assert_eq!(v.default_template(meetings.clone()).unwrap(), None);
    assert_eq!(
        ids(&v.list_templates(meetings.clone()).unwrap()),
        std::slice::from_ref(&retro)
    );
    assert!(v
        .list_children(Some(meetings.clone()), 0, 50)
        .unwrap()
        .iter()
        .any(|p| p.id == standup));
    // A plain child can become a template.
    v.set_template(notes.clone(), true).unwrap();
    assert_eq!(
        ids(&v.list_templates(meetings.clone()).unwrap()),
        [notes.clone(), retro.clone()]
    );

    // Templates survive a reopen (the index is rebuilt from the documents).
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    assert_eq!(
        ids(&v.list_templates(meetings.clone()).unwrap()),
        [notes.clone(), retro.clone()]
    );
    // A duplicated parent keeps its templates.
    let copy = v.duplicate_page(meetings.clone()).unwrap();
    assert_eq!(v.list_templates(copy).unwrap().len(), 2);
}

#[test]
fn database_default_template_is_a_template() {
    let t = TempDir::new("templates_db");
    let v = t.open();
    let c = v.create_page(None, "Tasks".into()).unwrap();
    v.make_collection(c.clone()).unwrap();
    let tpl = v.create_page(Some(c.clone()), "Bug".into()).unwrap();
    v.apply(tpl.clone(), vec![para("a", "Steps to reproduce", None)])
        .unwrap();
    v.set_default_template(c.clone(), Some(tpl.clone()))
        .unwrap();
    assert_eq!(
        ids(&v.list_templates(c.clone()).unwrap()),
        std::slice::from_ref(&tpl)
    );
    let row = v.create_page(Some(c.clone()), "Crash".into()).unwrap();
    assert_eq!(texts(&v, &row), ["Steps to reproduce"]);
    // Clearing the default keeps the template out of the rows.
    v.set_default_template(c.clone(), None).unwrap();
    let r = v.query(Some(c.clone()), None, vec![], 0, 10).unwrap();
    assert_eq!(ids(&r.rows), [row]);
    assert_eq!(ids(&v.list_templates(c).unwrap()), [tpl]);
}
