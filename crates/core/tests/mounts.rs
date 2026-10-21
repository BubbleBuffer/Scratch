//! Folder mounts: a folder's `.md` files as pages, edits written back,
//! outside edits read in, and the user's files never lost.

use std::fs;
use std::path::{Path, PathBuf};

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

fn child(v: &Vault, parent: &str, title: &str) -> Option<String> {
    v.list_children(Some(parent.into()), 0, 100)
        .unwrap()
        .into_iter()
        .find(|p| p.title == title)
        .map(|p| p.id)
}

fn read(p: &Path) -> String {
    fs::read_to_string(p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// Writes `text` to `p`, changing its size so a poll notices it.
fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

#[test]
fn mounted_folder_round_trip() {
    let t = TempDir::new("mount_vault");
    let f = TempDir::new("mount_folder");
    let dir = f.0.clone();
    write(
        &dir.join("Ideas.md"),
        "---\ntags: [garden]\n---\nGrow tomatoes.\n",
    );
    write(&dir.join("Plain.md"), "Just text.\n");
    write(&dir.join("notes.txt"), "not markdown");
    fs::create_dir_all(dir.join("sub")).unwrap();
    write(&dir.join("sub").join("Deep.md"), "ignored\n");

    let v = t.open();
    // Not a relative path, not the vault itself.
    assert!(v.add_mount("relative/dir".into(), None).is_err());
    assert!(v.add_mount(t.0.to_string_lossy().into(), None).is_err());
    let m = v.add_mount(dir.to_string_lossy().into(), None).unwrap();
    assert_eq!(v.list_mounts(), vec![m.clone()]);
    assert!(v.add_mount(dir.to_string_lossy().into(), None).is_err());
    let root = m.page.clone();
    let mut titles: Vec<String> = v
        .list_children(Some(root.clone()), 0, 100)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    titles.sort();
    assert_eq!(titles, ["Ideas", "Plain"]);
    let ideas = child(&v, &root, "Ideas").unwrap();
    let plain = child(&v, &root, "Plain").unwrap();
    assert_eq!(texts(&v, &ideas), ["Grow tomatoes."]);

    // File -> page -> edit -> file: the vault's own frontmatter stays out.
    v.apply(ideas.clone(), vec![para("b", "Water daily.", None)])
        .unwrap();
    v.flush().unwrap();
    let s = read(&dir.join("Ideas.md"));
    assert!(s.contains("Water daily."), "{s}");
    assert!(s.contains("Grow tomatoes."), "{s}");
    assert!(s.contains("tags:"), "{s}");
    assert!(!s.contains("id:") && !s.contains("created:"), "{s}");
    assert_eq!(read(&dir.join("notes.txt")), "not markdown");

    // An outside edit is picked up by a rescan.
    write(&dir.join("Plain.md"), "Just text, edited outside.\n");
    assert!(v.rescan_mounts().unwrap() >= 1);
    assert_eq!(texts(&v, &plain), ["Just text, edited outside."]);
    // Nothing changed: nothing to do.
    assert_eq!(v.rescan_mounts().unwrap(), 0);

    // A new file becomes a page; a deleted one sends its page to the trash.
    write(&dir.join("Fresh.md"), "New outside.\n");
    fs::remove_file(dir.join("Plain.md")).unwrap();
    v.rescan_mounts().unwrap();
    let fresh = child(&v, &root, "Fresh").unwrap();
    assert_eq!(texts(&v, &fresh), ["New outside."]);
    assert!(child(&v, &root, "Plain").is_none());
    assert!(v.list_trash().unwrap().iter().any(|p| p.page.id == plain));

    // Both sides changed before a rescan: the file is not overwritten, the
    // rescan merges, and the merged page is written back.
    write(&dir.join("Fresh.md"), "New outside.\n\nMore outside.\n");
    v.apply(fresh.clone(), vec![para("x", "Added in app.", None)])
        .unwrap();
    v.flush().unwrap();
    assert_eq!(
        read(&dir.join("Fresh.md")),
        "New outside.\n\nMore outside.\n"
    );
    v.rescan_mounts().unwrap();
    v.flush().unwrap();
    let got = texts(&v, &fresh);
    assert!(got.contains(&"More outside.".to_string()), "{got:?}");
    assert!(got.contains(&"Added in app.".to_string()), "{got:?}");
    let s = read(&dir.join("Fresh.md"));
    assert!(
        s.contains("More outside.") && s.contains("Added in app."),
        "{s}"
    );

    // A page renamed in the app renames its file.
    v.rename_page(fresh.clone(), "Renamed".into()).unwrap();
    v.flush().unwrap();
    assert!(!dir.join("Fresh.md").exists());
    assert!(read(&dir.join("Renamed.md")).contains("Added in app."));

    // A new page in the app is a new file; a taken name gets a number.
    write(&dir.join("Taken.md"), "The user's own.\n");
    let p = v.create_page(Some(root.clone()), "Taken".into()).unwrap();
    v.apply(p.clone(), vec![para("t", "From the app.", None)])
        .unwrap();
    v.flush().unwrap();
    assert_eq!(read(&dir.join("Taken.md")), "The user's own.\n");
    assert!(read(&dir.join("Taken (2).md")).contains("From the app."));

    // A page trashed in the app: its file leaves the folder, kept in the
    // vault.
    v.delete_page(ideas.clone()).unwrap();
    v.flush().unwrap();
    assert!(!dir.join("Ideas.md").exists());
    assert!(t
        .0
        .join(".vault")
        .join("mount-trash")
        .join(format!("{ideas}.md"))
        .exists());
    // A file edited outside is never removed: it comes back as a page.
    write(&dir.join("Renamed.md"), "Edited outside, then trashed.\n");
    v.delete_page(fresh.clone()).unwrap();
    v.flush().unwrap();
    assert_eq!(
        read(&dir.join("Renamed.md")),
        "Edited outside, then trashed.\n"
    );
    v.rescan_mounts().unwrap();
    let back = child(&v, &root, "Renamed").unwrap();
    assert_ne!(back, fresh);

    // The mount survives a reopen; a reindex finds the files again
    // without duplicating pages.
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    assert_eq!(v.list_mounts().len(), 1);
    let before = v.list_children(Some(root.clone()), 0, 100).unwrap().len();
    v.reindex().unwrap();
    write(&dir.join("Taken (2).md"), "From the app, then outside.\n");
    v.rescan_mounts().unwrap();
    assert_eq!(
        v.list_children(Some(root.clone()), 0, 100).unwrap().len(),
        before
    );
    assert_eq!(texts(&v, &p), ["From the app, then outside."]);

    // Unmounting keeps the pages and leaves the folder alone.
    v.remove_mount(root.clone()).unwrap();
    assert!(v.list_mounts().is_empty());
    v.apply(p.clone(), vec![para("u", "After unmount.", None)])
        .unwrap();
    v.flush().unwrap();
    assert_eq!(
        read(&dir.join("Taken (2).md")),
        "From the app, then outside.\n"
    );
    assert!(texts(&v, &p).contains(&"After unmount.".to_string()));
    // Nothing below the folder's top level was touched.
    assert_eq!(read(&dir.join("sub").join("Deep.md")), "ignored\n");
}
