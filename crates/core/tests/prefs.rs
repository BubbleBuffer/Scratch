//! Page display options (`PagePrefs`): stored in the page document, kept
//! across reopening, copied by duplicate and carried by sync.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_prefs_{tag}_{}", new_id()));
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

fn prefs() -> PagePrefs {
    PagePrefs {
        small_text: true,
        full_width: true,
        locked: true,
        cover_position: Some(250),
    }
}

#[test]
fn prefs_default_set_and_survive_reopen() {
    let t = TempDir::new("reopen");
    let id;
    {
        let v = t.open();
        id = v.create_page(None, "Styled".into()).unwrap();
        assert_eq!(v.load_page(id.clone()).unwrap().prefs, PagePrefs::default());
        v.set_page_prefs(id.clone(), prefs()).unwrap();
        assert_eq!(v.load_page(id.clone()).unwrap().prefs, prefs());
        // The change is reported on the feed.
        let b = v.poll_changes(0);
        assert!(b.changes.contains(&Change::PageChanged(id.clone())));
        v.flush().unwrap();
    }
    let v = t.open();
    assert_eq!(v.load_page(id.clone()).unwrap().prefs, prefs());

    // Back to the defaults.
    v.set_page_prefs(id.clone(), PagePrefs::default()).unwrap();
    assert_eq!(v.load_page(id.clone()).unwrap().prefs, PagePrefs::default());

    // Out of range and unknown pages are refused.
    let bad = PagePrefs {
        cover_position: Some(1001),
        ..PagePrefs::default()
    };
    assert!(matches!(
        v.set_page_prefs(id, bad),
        Err(VaultError::Invalid(_))
    ));
    assert!(matches!(
        v.set_page_prefs(new_id(), prefs()),
        Err(VaultError::NotFound(_))
    ));
}

#[test]
fn duplicate_copies_prefs() {
    let t = TempDir::new("dup");
    let v = t.open();
    let id = v.create_page(None, "Wide".into()).unwrap();
    v.set_page_prefs(id.clone(), prefs()).unwrap();
    let copy = v.duplicate_page(id).unwrap();
    assert_eq!(v.load_page(copy).unwrap().prefs, prefs());
}

#[test]
fn prefs_sync_to_another_vault() {
    let ta = TempDir::new("sync_a");
    let tb = TempDir::new("sync_b");
    let a = ta.open();
    let b = tb.open();
    let id = a.create_page(None, "Shared".into()).unwrap();
    a.set_page_prefs(id.clone(), prefs()).unwrap();

    let have: HashMap<String, Vec<u8>> = b
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    let mut updates = Vec::new();
    for pv in a.page_versions().unwrap() {
        let theirs = have.get(&pv.page).cloned().unwrap_or_default();
        let bytes = a.export_updates(pv.page.clone(), theirs).unwrap();
        updates.push(PageUpdate {
            page: pv.page,
            bytes,
        });
    }
    b.import_updates_batch(updates).unwrap();
    assert_eq!(b.load_page(id).unwrap().prefs, prefs());
}
