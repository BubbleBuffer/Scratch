//! Page types: documents by default, databases made explicitly, the old
//! `collection` flag and file front matter, and unknown types kept as they are.

use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_core_page_type_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).expect("open")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The markdown file of a page: `pages/<title>-<id>.md`.
fn md_path(t: &TempDir, id: &str) -> PathBuf {
    fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
        .expect("markdown file")
}

/// The top-level page list entry with this title.
fn titled(v: &Vault, title: &str) -> PageSummary {
    v.list_children(None, 0, 200)
        .unwrap()
        .into_iter()
        .find(|p| p.title == title)
        .unwrap_or_else(|| panic!("no page titled {title}"))
}

#[test]
fn pages_are_documents_until_made_databases() {
    let t = TempDir::new("default");
    let v = t.open();
    let id = v.create_page(None, "Plain".into()).unwrap();
    let page = v.load_page(id.clone()).unwrap();
    assert_eq!(page.page_type, PAGE_TYPE_DOCUMENT);
    assert!(!page.is_database());
    assert_eq!(titled(&v, "Plain").page_type, PAGE_TYPE_DOCUMENT);

    v.make_collection(id.clone()).unwrap();
    // Already a database: nothing changes.
    v.make_collection(id.clone()).unwrap();
    assert_eq!(
        v.load_page(id.clone()).unwrap().page_type,
        PAGE_TYPE_DATABASE
    );
    assert!(titled(&v, "Plain").is_database());

    // The file says `type` for a database, and nothing for a document.
    let note = v.create_page(None, "Note".into()).unwrap();
    v.flush().unwrap();
    let md = fs::read_to_string(md_path(&t, &id)).unwrap();
    assert!(md.contains("type: \"database\"\n"), "{md}");
    assert!(!md.contains("collection"), "{md}");
    let note_md = fs::read_to_string(md_path(&t, &note)).unwrap();
    assert!(!note_md.contains("type"), "{note_md}");
}

#[test]
fn database_type_survives_an_index_from_an_older_schema() {
    let t = TempDir::new("migrate");
    let id = {
        let v = t.open();
        let id = v.create_page(None, "Tasks".into()).unwrap();
        v.make_collection(id.clone()).unwrap();
        v.flush().unwrap();
        id
    };
    // An index written by the previous schema (`is_collection`) is rebuilt
    // on open, and the types come back from the vault's own data.
    let index = t.0.join(".vault").join("index.sqlite");
    rusqlite::Connection::open(&index)
        .unwrap()
        .pragma_update(None, "user_version", 8)
        .unwrap();
    let v = t.open();
    assert_eq!(v.load_page(id).unwrap().page_type, PAGE_TYPE_DATABASE);
    assert!(titled(&v, "Tasks").is_database());
}

#[test]
fn file_front_matter_sets_the_type_and_unknown_types_are_kept() {
    let t = TempDir::new("files");
    let v = t.open();
    // The old front matter, `collection: true`, reads as a database.
    fs::write(
        t.0.join("pages/Legacy.md"),
        "---\ntitle: \"Legacy\"\ncollection: true\n---\n\nA row.\n",
    )
    .unwrap();
    // A type a mod defines is kept as it is, and is not a property.
    fs::write(
        t.0.join("pages/Board.md"),
        "---\ntitle: \"Board\"\ntype: \"kanban\"\n---\n\nCards.\n",
    )
    .unwrap();
    assert_eq!(v.rescan().unwrap(), 2);

    assert!(titled(&v, "Legacy").is_database());
    let board = titled(&v, "Board");
    assert_eq!(board.page_type, "kanban");
    assert!(!board.is_database());
    assert!(board.properties.iter().all(|p| p.key != "type"));

    v.flush().unwrap();
    let md = fs::read_to_string(md_path(&t, &board.id)).unwrap();
    assert!(md.contains("type: \"kanban\"\n"), "{md}");

    // Only a document can become a database; the kanban type is refused.
    assert!(v.make_collection(board.id.clone()).is_err());
    assert_eq!(v.load_page(board.id).unwrap().page_type, "kanban");
}

#[test]
fn an_edited_file_changes_the_type_only_when_it_declares_one() {
    let t = TempDir::new("edits");
    let v = t.open();
    let id = v.create_page(None, "Ledger".into()).unwrap();
    v.make_collection(id.clone()).unwrap();
    v.flush().unwrap();

    // Dropping the type line says nothing about the type: still a database.
    let path = md_path(&t, &id);
    let md = fs::read_to_string(&path).unwrap();
    fs::write(&path, md.replace("type: \"database\"\n", "")).unwrap();
    v.rescan().unwrap();
    assert!(v.load_page(id.clone()).unwrap().is_database());

    // A file that declares `type: "document"` makes the page a document.
    let path = md_path(&t, &id);
    let md = fs::read_to_string(&path).unwrap();
    assert!(!md.contains("type:"), "{md}");
    fs::write(&path, md.replacen("---\n", "---\ntype: \"document\"\n", 1)).unwrap();
    v.rescan().unwrap();
    assert_eq!(v.load_page(id).unwrap().page_type, PAGE_TYPE_DOCUMENT);
    assert!(!titled(&v, "Ledger").is_database());
}

#[test]
fn unknown_type_strings_round_trip_exactly_including_whitespace_and_empty() {
    let (source_dir, destination_dir) = (
        TempDir::new("exact_source"),
        TempDir::new("exact_destination"),
    );
    let source = source_dir.open();
    let types = [" mod:board 雪 ", "", "documentary"];
    for (i, kind) in types.iter().enumerate() {
        fs::write(
            source_dir.0.join(format!("pages/Exact{i}.md")),
            format!(
                "---\ntitle: Exact{i}\ntype: {}\n---\nbody\n",
                serde_json::to_string(kind).unwrap()
            ),
        )
        .unwrap();
    }
    source.rescan().unwrap();
    source.flush().unwrap();
    let destination = destination_dir.open();
    let imported = destination
        .import_markdown(
            source_dir.0.join("pages").to_string_lossy().into_owned(),
            None,
        )
        .unwrap();
    assert_eq!(imported.pages, types.len() as u32);
    for (i, kind) in types.iter().enumerate() {
        for v in [&source, &destination] {
            let row = titled(v, &format!("Exact{i}"));
            assert_eq!(&row.page_type, kind);
            assert_eq!(&v.load_page(row.id).unwrap().page_type, kind);
        }
    }
}

#[test]
fn a_property_named_type_is_distinct_from_the_page_type() {
    let (a, b) = (
        TempDir::new("reserved_source"),
        TempDir::new("reserved_destination"),
    );
    let source = a.open();
    let id = source.create_page(None, "Typed property".into()).unwrap();
    source
        .set_property(id.clone(), "type".into(), PropValue::Text("book".into()))
        .unwrap();
    source.flush().unwrap();
    let md = fs::read_to_string(md_path(&a, &id)).unwrap();
    assert!(md.contains("\"prop:type\":"), "{md}");
    let destination = b.open();
    let report = destination
        .import_markdown(a.0.join("pages").to_string_lossy().into_owned(), None)
        .unwrap();
    let page = destination.load_page(report.top_pages[0].clone()).unwrap();
    assert_eq!(page.page_type, PAGE_TYPE_DOCUMENT);
    assert!(page
        .properties
        .iter()
        .any(|p| p.key == "type" && p.value == PropValue::Text("book".into())));
}
