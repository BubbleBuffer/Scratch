//! Deterministic schedule outputs survive retries after ordinary vault sync.
use std::collections::HashMap;
use vault_core::{schedule_occurrence_id, version_includes, Vault};

// Same incremental exchange as day_notes.rs: no transport or new sync path.
fn push(from: &Vault, to: &Vault) {
    to.import_purged(from.purged_pages()).unwrap();
    let have: HashMap<_, _> = to.page_versions().unwrap().into_iter()
        .map(|v| (v.page, v.version)).collect();
    for v in from.page_versions().unwrap() {
        let since = have.get(&v.page).cloned().unwrap_or_default();
        if !version_includes(&since, &v.version).unwrap() {
            to.import_updates(v.page.clone(), from.export_updates(v.page, since).unwrap()).unwrap();
        }
    }
}

#[test]
fn synced_occurrence_retries_find_one_page_on_both_replicas() {
    let dir = std::env::temp_dir().join(format!("notes_schedule_replicas_{}", vault_core::new_id()));
    {
        let a = Vault::open(dir.join("a").to_string_lossy().into()).unwrap();
        let b = Vault::open(dir.join("b").to_string_lossy().into()).unwrap();
        let parent = a.create_page(None, "Journal".into()).unwrap();
        let template = a.create_template(parent.clone(), "Weekly".into()).unwrap();
        push(&a, &b);
        let id = schedule_occurrence_id("schedule", "2026-10-19T06:00:00Z");
        assert!(a.create_page_from_identified(id.clone(), parent.clone(), "Week 43 2026".into(), Some(template.clone())).unwrap().1);
        push(&a, &b);
        let before = b.load_page(id.clone()).unwrap();
        assert!(!b.create_page_from_identified(id.clone(), parent.clone(), "Retry must keep original title".into(), Some(template.clone())).unwrap().1);
        assert_eq!(b.load_page(id.clone()).unwrap(), before);
        push(&b, &a);
        for vault in [&a, &b] {
            assert_eq!(vault.list_children(Some(parent.clone()), 0, 100).unwrap().iter().filter(|p| p.id == id).count(), 1);
            assert!(!vault.create_page_from_identified(id.clone(), parent.clone(), "Again".into(), Some(template.clone())).unwrap().1);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
