//! Identified page creation from a template, deterministic schedule and node
//! ids, and node records under the hidden Nodes collection.

use serde_json::{json, Value};
use std::{fs, path::PathBuf};
use vault_core::*;

const UTC: &str = "2026-10-19T08:00:00Z";

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_identified_{}", new_id())))
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

fn insert(v: &Vault, page: &str, after: Option<&str>, id: &str, text: &str) {
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: after.map(str::to_owned),
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Paragraph,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
}

/// A parent with a template holding two paragraphs and one property.
fn setup(v: &Vault) -> (String, String) {
    let parent = v.create_page(None, "Journal".into()).unwrap();
    let template = v.create_template(parent.clone(), "Weekly".into()).unwrap();
    insert(v, &template, None, "p1", "Review goals");
    insert(v, &template, Some("p1"), "p2", "Plan the week");
    v.set_property(
        template.clone(),
        "status".into(),
        PropValue::Text("open".into()),
    )
    .unwrap();
    (parent, template)
}

fn texts(page: &Page) -> Vec<&str> {
    page.blocks.iter().map(|b| b.text.as_str()).collect()
}

/// Removes the stored document and log files of one page, to make its
/// template unreadable while the index still lists it.
fn forget_page_files(t: &TempVault, id: &str) {
    for dir in [
        t.0.join(".vault").join("docs"),
        t.0.join(".vault").join("logs"),
    ] {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name().to_string_lossy().contains(id) {
                fs::remove_file(entry.path()).unwrap();
            }
        }
    }
}

#[test]
fn identified_page_copies_the_template_and_keeps_the_caller_id() {
    let t = TempVault::new();
    let v = t.open();
    let (parent, template) = setup(&v);
    let plain = v.create_page(Some(parent.clone()), "Plain".into()).unwrap();
    let id = schedule_occurrence_id("schedule-1", UTC);

    let (got, created) = v
        .create_page_from_identified(
            id.clone(),
            parent.clone(),
            "Week 42".into(),
            Some(template.clone()),
        )
        .unwrap();
    assert_eq!((got.as_str(), created), (id.as_str(), true));

    let page = v.load_page(id.clone()).unwrap();
    assert_eq!(page.title, "Week 42");
    assert_eq!(page.parent.as_deref(), Some(parent.as_str()));
    assert_eq!(texts(&page), ["Review goals", "Plan the week"]);
    assert_ne!(page.blocks[0].id, "p1", "copied blocks get fresh ids");
    assert!(page
        .properties
        .iter()
        .any(|p| p.key == "status" && matches!(&p.value, PropValue::Text(s) if s == "open")));

    // The template keeps its own title and blocks.
    let tpl = v.load_page(template.clone()).unwrap();
    assert_eq!(tpl.title, "Weekly");
    assert_eq!(texts(&tpl), ["Review goals", "Plan the week"]);

    // A child that is not a template of the parent is refused, and nothing is made.
    let refused = schedule_occurrence_id("schedule-1", "2026-10-26T08:00:00Z");
    assert!(v
        .create_page_from_identified(refused.clone(), parent.clone(), "No".into(), Some(plain))
        .is_err());
    assert!(v.load_page(refused).is_err());

    // Ids become file names, so anything but a page id is refused.
    assert!(v
        .create_page_from_identified("../escape".into(), parent, "No".into(), None)
        .is_err());
}

#[test]
fn identified_ids_must_be_page_ids_within_the_ulid_bounds() {
    let t = TempVault::new();
    let v = t.open();
    let parent = v.create_page(None, "Journal".into()).unwrap();

    // Leading characters above `7` exceed a 128-bit value; wrong lengths and
    // characters outside the alphabet (`I`, `L`, `O`, `U`) are not ids either.
    let bad = [
        format!("8{}", "0".repeat(25)),
        format!("Z{}", "0".repeat(25)),
        format!("0I{}", "0".repeat(24)),
        "0".repeat(25),
        "0".repeat(27),
    ];
    for id in bad {
        assert!(
            v.create_page_from_identified(id.clone(), parent.clone(), "No".into(), None)
                .is_err(),
            "{id} is refused"
        );
    }

    // The largest 128-bit shape is accepted, as is a generated id.
    let top = format!("7{}", "Z".repeat(25));
    assert_eq!(
        v.create_page_from_identified(top.clone(), parent.clone(), "Top".into(), None)
            .unwrap(),
        (top, true)
    );
    let generated = schedule_occurrence_id("schedule-1", UTC);
    assert!(generated.as_bytes()[0] <= b'7');
    assert_eq!(
        v.create_page_from_identified(generated.clone(), parent, "Gen".into(), None)
            .unwrap(),
        (generated, true)
    );
}

#[test]
fn unreadable_template_is_an_error_and_creates_nothing() {
    let t = TempVault::new();
    let id = schedule_occurrence_id("schedule-1", UTC);
    let (parent, template) = {
        let v = t.open();
        let (parent, template) = setup(&v);
        v.flush().unwrap();
        (parent, template)
    };
    forget_page_files(&t, &template);

    let v = t.open();
    assert!(v
        .create_page_from_identified(id.clone(), parent.clone(), "Week 42".into(), Some(template),)
        .is_err());
    assert!(v.load_page(id.clone()).is_err());
    assert!(v
        .list_children(Some(parent.clone()), 0, 100)
        .unwrap()
        .iter()
        .all(|p| p.id != id));

    // Nothing was installed, so the same id is still free.
    let (got, created) = v
        .create_page_from_identified(id.clone(), parent, "Retry".into(), None)
        .unwrap();
    assert_eq!((got.as_str(), created), (id.as_str(), true));
}

#[test]
fn repeated_identified_call_returns_false_and_changes_nothing() {
    let t = TempVault::new();
    let id = schedule_occurrence_id("schedule-1", UTC);
    let (parent, template) = {
        let v = t.open();
        let (parent, template) = setup(&v);
        let (got, created) = v
            .create_page_from_identified(
                id.clone(),
                parent.clone(),
                "Week 42".into(),
                Some(template.clone()),
            )
            .unwrap();
        assert_eq!((got.as_str(), created), (id.as_str(), true));
        v.flush().unwrap();
        (parent, template)
    };

    let v = t.open();
    let before = v.page_versions().unwrap();
    let children = v.list_children(Some(parent.clone()), 0, 100).unwrap().len();
    for (title, with_template) in [("Other title", None), ("Third", Some(template))] {
        let (got, created) = v
            .create_page_from_identified(id.clone(), parent.clone(), title.into(), with_template)
            .unwrap();
        assert_eq!((got.as_str(), created), (id.as_str(), false));
    }
    assert_eq!(v.load_page(id.clone()).unwrap().title, "Week 42");
    assert_eq!(v.page_versions().unwrap(), before);
    assert_eq!(
        v.list_children(Some(parent), 0, 100).unwrap().len(),
        children
    );
}

#[test]
fn trashed_and_purged_ids_are_not_recreated() {
    let t = TempVault::new();
    let v = t.open();
    let (parent, template) = setup(&v);
    let id = schedule_occurrence_id("schedule-1", UTC);
    v.create_page_from_identified(
        id.clone(),
        parent.clone(),
        "Week 42".into(),
        Some(template.clone()),
    )
    .unwrap();

    v.delete_page(id.clone()).unwrap();
    let (got, created) = v
        .create_page_from_identified(
            id.clone(),
            parent.clone(),
            "Again".into(),
            Some(template.clone()),
        )
        .unwrap();
    assert_eq!((got.as_str(), created), (id.as_str(), false));
    assert!(v
        .list_children(Some(parent.clone()), 0, 100)
        .unwrap()
        .iter()
        .all(|p| p.id != id));

    // A purge is permanent: the same id is not reused under another id either.
    v.purge_page(id.clone()).unwrap();
    let (got, created) = v
        .create_page_from_identified(id.clone(), parent.clone(), "After purge".into(), None)
        .unwrap();
    assert_eq!((got.as_str(), created), (id.as_str(), false));
    assert!(v.load_page(id.clone()).is_err());
    assert!(v
        .list_children(Some(parent), 0, 100)
        .unwrap()
        .iter()
        .all(|p| p.id != id && p.title != "After purge"));
}

#[test]
fn independent_handles_create_one_identified_page() {
    let t = TempVault::new();
    let a = t.open();
    let b = t.open();
    let (parent, template) = setup(&a);
    let id = schedule_occurrence_id("schedule-1", UTC);

    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = [&a, &b]
            .into_iter()
            .map(|v| {
                let barrier = &barrier;
                let (parent, template, id) = (parent.clone(), template.clone(), id.clone());
                scope.spawn(move || {
                    barrier.wait();
                    v.create_page_from_identified(id, parent, "Week 42".into(), Some(template))
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|(_, created)| *created).count(), 1);
    assert!(results.iter().all(|(got, _)| *got == id));
    for v in [&a, &b] {
        let rows = v.list_children(Some(parent.clone()), 0, 100).unwrap();
        assert_eq!(rows.iter().filter(|r| r.id == id).count(), 1);
    }
}

#[test]
fn node_records_are_deterministic_under_the_hidden_nodes_page() {
    let t = TempVault::new();
    let v = t.open();
    v.create_page(None, "Groceries".into()).unwrap();
    v.create_page(None, "Hub notes".into()).unwrap();
    let visible = v.page_count().unwrap();
    let device = "device-hub";

    let first = json!({
        "device_id": device, "name": "Hub", "kind": "hub", "available": true,
        "tools": ["read_page", "search_notes"],
    })
    .to_string();
    let id = v.write_node_record(device.into(), first).unwrap();
    assert_eq!(id, node_record_id(device));

    // The Nodes collection is hidden like the calendar page: not listed at the
    // top, not searched, and its own id is not counted. The record is an
    // ordinary page, so it is listed under Nodes, searched and counted.
    let nodes = v.find_by_role("nodes".into()).unwrap().expect("Nodes page");
    assert!(v
        .list_children(None, 0, 100)
        .unwrap()
        .iter()
        .all(|p| p.id != id && p.id != nodes));
    let listed = v.list_children(Some(nodes.clone()), 0, 100).unwrap();
    assert!(
        listed.iter().any(|p| p.id == id),
        "record lists under Nodes"
    );
    assert_eq!(v.page_count().unwrap(), visible + 1);
    v.flush().unwrap();

    let hits = v.search("Hub".into(), 10).unwrap();
    assert!(
        hits.iter().any(|h| h.title == "Hub notes"),
        "control page is found"
    );
    assert!(hits.iter().any(|h| h.id == id), "record is found by search");
    assert!(v
        .search("Nodes".into(), 10)
        .unwrap()
        .iter()
        .all(|h| h.id != nodes));

    // The runner picker queries node records by type.
    let typed = v.search("type:node".into(), 10).unwrap();
    assert!(
        typed.iter().any(|h| h.id == id),
        "type:node finds the record"
    );
    assert!(typed.iter().all(|h| h.id != nodes));

    // A rewrite updates the same page: the title, the fields that changed, and
    // no `tools` key once the record no longer has one.
    let renamed = json!({
        "device_id": device, "name": "Hub renamed", "kind": "hub", "available": false,
    })
    .to_string();
    assert_eq!(
        v.write_node_record(device.into(), renamed.clone()).unwrap(),
        id
    );
    assert_eq!(v.load_page(id.clone()).unwrap().title, "Hub renamed");
    let data: Value = serde_json::from_str(&v.type_data_all(id.clone()).unwrap()).unwrap();
    assert_eq!(data["node"]["available"], json!(false));
    assert!(data["node"].get("tools").is_none());

    // Add-on data on the record that the scheduler does not own is kept.
    v.type_data_set(id.clone(), "probe".into(), vec!["k".into()], "1".into())
        .unwrap();
    assert_eq!(v.write_node_record(device.into(), renamed).unwrap(), id);
    let data: Value = serde_json::from_str(&v.type_data_all(id.clone()).unwrap()).unwrap();
    assert_eq!(data["probe"]["k"], json!(1));
    assert_eq!(data["node"]["name"], json!("Hub renamed"));

    // Records that do not match the contract are refused.
    assert!(v
        .write_node_record(
            "other-device".into(),
            json!({"device_id": device, "name": "x"}).to_string(),
        )
        .is_err());
    assert!(
        v.write_node_record(
            device.into(),
            json!({"device": device, "name": "x"}).to_string()
        )
        .is_err(),
        "the field is device_id, not device"
    );
    assert!(v
        .write_node_record(device.into(), json!({"device_id": device}).to_string())
        .is_err());
    assert!(v.write_node_record(device.into(), "[]".into()).is_err());
}

#[test]
fn deleted_node_record_is_written_again_and_a_purged_one_is_refused() {
    let t = TempVault::new();
    let v = t.open();
    let device = "device-laptop";
    let json = json!({"device_id": device, "name": "Laptop", "kind": "desktop", "available": true})
        .to_string();
    let id = v.write_node_record(device.into(), json.clone()).unwrap();

    v.delete_page(id.clone()).unwrap();
    assert_eq!(
        v.write_node_record(device.into(), json.clone()).unwrap(),
        id
    );
    assert!(!v.load_page(id.clone()).unwrap().in_trash);

    v.delete_page(id.clone()).unwrap();
    v.purge_page(id.clone()).unwrap();
    assert!(v.write_node_record(device.into(), json).is_err());
}

#[test]
fn schedule_and_node_ids_are_stable_and_separate() {
    let a = schedule_occurrence_id("schedule-1", UTC);
    assert_eq!(a.len(), 26);
    assert_eq!(a, schedule_occurrence_id("schedule-1", UTC));
    assert_ne!(
        a,
        schedule_occurrence_id("schedule-1", "2026-10-26T08:00:00Z")
    );
    assert_ne!(a, schedule_occurrence_id("schedule-2", UTC));

    let n = node_record_id("schedule-1");
    assert_eq!(n.len(), 26);
    assert_eq!(n, node_record_id("schedule-1"));
    assert_ne!(n, a, "distinct domains never share an id");
    assert_ne!(node_record_id("device-a"), node_record_id("device-b"));
}
