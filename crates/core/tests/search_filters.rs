//! Search operators are hard constraints across exact, title and typo paths.
use std::collections::BTreeSet;
use std::path::PathBuf;

use vault_core::*;

const OCT_10: i64 = 1_791_590_400_000; // 2026-10-10, midnight UTC.
const DAY: i64 = 86_400_000;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_search_filters_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }

    // Fixed page timestamps make date matching and ordering independent of
    // scheduling, and exercise the same derived index that search reads.
    fn timestamps(&self, id: &str, created: i64, updated: i64) {
        rusqlite::Connection::open(self.0.join(".vault/index.sqlite"))
            .unwrap()
            .execute(
                "UPDATE pages SET created_ms = ?1, updated_ms = ?2 WHERE id = ?3",
                rusqlite::params![created, updated, id],
            )
            .unwrap();
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn paragraph(v: &Vault, id: &str, text: &str) {
    v.apply(
        id.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
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

fn page(v: &Vault, title: &str, text: &str) -> PageId {
    let id = v.create_page(None, title.into()).unwrap();
    if !text.is_empty() {
        paragraph(v, &id, text);
    }
    id
}

fn prop(v: &Vault, id: &str, key: &str, value: PropValue) {
    v.set_property(id.into(), key.into(), value).unwrap();
}

fn tags(v: &Vault, id: &str, values: &[&str]) {
    prop(
        v,
        id,
        "tags",
        PropValue::MultiSelect(values.iter().map(|s| (*s).into()).collect()),
    );
}

fn date(v: &Vault, id: &str, key: &str, when: i64) {
    prop(
        v,
        id,
        key,
        PropValue::Date {
            start_ms: when,
            end_ms: None,
            has_time: true,
        },
    );
}

fn search(v: &Vault, query: &str, limit: u32, offset: u32) -> SearchSpecResults {
    let parsed = v.parse_search(query.into()).unwrap();
    v.search_spec(parsed.spec, limit, offset).unwrap()
}

fn ids(v: &Vault, query: &str) -> BTreeSet<PageId> {
    search(v, query, 100, 0)
        .hits
        .into_iter()
        .map(|h| h.id)
        .collect()
}

fn assert_ids(v: &Vault, query: &str, expected: &[&PageId]) {
    let expected: BTreeSet<_> = expected.iter().map(|id| (*id).clone()).collect();
    assert_eq!(ids(v, query), expected, "query {query:?}");
}

#[test]
fn tag_alias_hierarchy_comma_repetition_and_negation() {
    let t = TempVault::new();
    let v = t.open();
    let work = page(&v, "Work", "");
    let child = page(&v, "Meeting", "");
    let home = page(&v, "Home", "");
    let lookalike = page(&v, "Workplace", "");
    let plain = page(&v, "Plain", "");
    let literal = page(&v, "Literal tag", "");
    tags(&v, &work, &["work", "urgent"]);
    tags(&v, &child, &["Work/Meetings"]);
    tags(&v, &home, &["home", "urgent"]);
    tags(&v, &lookalike, &["workplace"]);
    tags(&v, &literal, &["work,home", "project notes", "école"]);
    v.flush().unwrap();
    assert_ids(&v, "tag:WORK", &[&work, &child]);
    assert_ids(&v, "#work", &[&work, &child]);
    assert_ids(&v, "tag:work,home", &[&work, &child, &home]);
    assert_ids(&v, "tag:work tag:urgent", &[&work]);
    assert_ids(&v, "tag:work -tag:urgent", &[&child]);
    assert_ids(&v, "-tag:work,home", &[&lookalike, &plain, &literal]);
    assert_ids(&v, "tag:\"work,home\"", &[&literal]);
    assert_ids(&v, "tag:\"project notes\"", &[&literal]);
    assert_ids(&v, "tag:ÉCOLE", &[&literal]);
    let result = search(&v, "tag:work", 10, 0);
    assert!(result.hits.iter().all(|h| h
        .matched_filters
        .iter()
        .any(|f| f.key == "tag" && !f.value.is_empty())));
}

#[test]
fn property_types_quoted_values_and_normalized_keys() {
    let t = TempVault::new();
    let v = t.open();
    let a = page(&v, "A", "");
    let b = page(&v, "B", "");
    let empty = page(&v, "Empty", "");
    for (id, points, done, owner, status, labels) in [
        (
            &a,
            5.0,
            true,
            "Alice Smith",
            "In progress",
            vec!["red", "blue", "école"],
        ),
        (&b, 9.0, false, "Bob Jones", "Done", vec!["blue"]),
    ] {
        prop(&v, id, "points", PropValue::Number(points));
        prop(&v, id, "done", PropValue::Checkbox(done));
        prop(&v, id, "owner", PropValue::Text(owner.into()));
        prop(&v, id, "status", PropValue::Select(status.into()));
        prop(
            &v,
            id,
            "labels",
            PropValue::MultiSelect(labels.into_iter().map(Into::into).collect()),
        );
    }
    date(&v, &a, "Due date", OCT_10 + DAY / 2);
    date(&v, &b, "Due date", OCT_10 + 2 * DAY + DAY / 2);
    prop(&v, &a, "in", PropValue::Text("personal inbox".into()));
    prop(&v, &a, "school", PropValue::Select("école".into()));
    v.flush().unwrap();
    for query in [
        "points:5",
        "points:1..5",
        "points:<=5",
        "points:<9",
        "done:yes",
        "done:true",
        "owner:SMITH",
        "owner:\"Alice Smith\"",
        "status:\"IN PROGRESS\"",
        "labels:red",
        "labels:red labels:blue",
        "labels:ÉCOLE",
        "school:ÉCOLE",
        "duedate:2026-10-10",
        "due_date:2026-10-10",
        "DUE-DATE:2026-10-10",
        "\"due date\":2026-10-10",
        "\"in\":inbox",
    ] {
        assert_ids(&v, query, &[&a]);
    }
    for query in [
        "points:>5",
        "points:>=9",
        "status:done",
        "labels:blue -labels:red",
    ] {
        assert_ids(&v, query, &[&b]);
    }
    // Structured query semantics treat a missing checkbox as unchecked.
    for query in ["done:no", "done:false"] {
        assert_ids(&v, query, &[&b, &empty]);
    }
    assert_ids(&v, "labels:blue", &[&a, &b]);
    assert_ids(&v, "labels:red,blue", &[&a, &b]);
    assert_ids(&v, "points:none", &[&empty]);
    assert_ids(&v, "owner:none", &[&empty]);
    for key in ["points", "owner"] {
        let result = search(&v, &format!("{key}:none"), 10, 0);
        assert!(
            result.hits[0]
                .matched_filters
                .iter()
                .any(|f| f.key == key && f.value == "none"),
            "empty matched property must be visible: {key}"
        );
    }
    assert_ids(&v, "has:owner", &[&a, &b]);
    assert_ids(&v, "-has:owner", &[&empty]);
    for query in [
        "status:Inprogress",
        "status:In_progress",
        "status:In-progress",
        "labels:red,green",
    ] {
        let parsed = v.parse_search(query.into()).unwrap();
        assert!(
            !parsed.spec.warnings.is_empty(),
            "unreadable option must warn for {query:?}"
        );
        assert!(parsed.spec.operators.is_empty());
        let result = v.search_spec(parsed.spec, 10, 0).unwrap();
        assert_eq!(
            result
                .hits
                .iter()
                .map(|h| h.id.clone())
                .collect::<BTreeSet<_>>(),
            [a.clone(), b.clone(), empty.clone()].into_iter().collect()
        );
    }
}

#[test]
fn collection_property_display_names_and_option_names_resolve() {
    let t = TempVault::new();
    let v = t.open();
    let collection = page(&v, "Tasks", "");
    v.set_schema(
        collection.clone(),
        vec![PropDef {
            key: "opaque-status-key".into(),
            name: "Task state".into(),
            kind: PropKind::Select,
            options: vec![SelectOption {
                id: "opaque-todo-id".into(),
                name: "To do".into(),
                color: String::new(),
            }],
            config: String::new(),
        }],
    )
    .unwrap();
    let row = v
        .create_page(Some(collection.clone()), "A task".into())
        .unwrap();
    prop(
        &v,
        &row,
        "opaque-status-key",
        PropValue::Select("opaque-todo-id".into()),
    );
    v.flush().unwrap();
    assert_ids(&v, "taskstate:\"TO DO\"", &[&row]);
    assert_ids(&v, "\"Task state\":\"To do\"", &[&row]);
}

#[test]
fn computed_timestamp_properties_filter_has_sort_and_round_trip_without_stored_values() {
    let t = TempVault::new();
    let v = t.open();
    let collection = page(&v, "Timestamp collection", "");
    v.set_schema(
        collection.clone(),
        vec![
            PropDef {
                key: "created-at-key".into(),
                name: "Created at".into(),
                kind: PropKind::CreatedTime,
                options: vec![],
                config: String::new(),
            },
            PropDef {
                key: "edited-at-key".into(),
                name: "Edited at".into(),
                kind: PropKind::UpdatedTime,
                options: vec![],
                config: String::new(),
            },
        ],
    )
    .unwrap();
    let a = v
        .create_page(Some(collection.clone()), "Row A".into())
        .unwrap();
    let b = v.create_page(Some(collection), "Row B".into()).unwrap();
    v.flush().unwrap();
    // These computed schema properties have no props rows. Opposite timestamp
    // orders detect accidental property-table sorts or updated fallbacks.
    t.timestamps(&a, OCT_10 - 2 * DAY + DAY / 2, OCT_10 + 2 * DAY + DAY / 2);
    t.timestamps(&b, OCT_10 - DAY + DAY / 2, OCT_10 + DAY / 2);
    for (query, expected) in [
        ("created-at-key:2026-10-08 title:Row", vec![a.clone()]),
        ("\"Edited at\":2026-10-10 title:Row", vec![b.clone()]),
        (
            "has:\"Created at\" has:\"Edited at\" title:Row sort:\"Created at\"",
            vec![b.clone(), a.clone()],
        ),
        (
            "has:\"Edited at\" title:Row sort:\"Edited at\"",
            vec![a.clone(), b.clone()],
        ),
        ("-has:\"Created at\" title:Row", vec![]),
        ("-has:\"Edited at\" title:Row", vec![]),
    ] {
        let parsed = v.parse_search(query.into()).unwrap();
        assert!(parsed.spec.warnings.is_empty(), "{query:?}");
        let result = v.search_spec(parsed.spec.clone(), 10, 0).unwrap();
        assert_eq!(
            result.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            expected,
            "{query:?}"
        );
        for hit in &result.hits {
            if query.contains("created-at-key:") || query.contains("has:\"Created at\"") {
                let day = if hit.id == a {
                    "2026-10-08"
                } else {
                    "2026-10-09"
                };
                assert!(
                    hit.matched_filters
                        .iter()
                        .any(|f| f.key == "created-at-key" && f.value.starts_with(day)),
                    "created value for {query:?}: {hit:?}"
                );
            }
            if query.contains("\"Edited at\":") || query.contains("has:\"Edited at\"") {
                let day = if hit.id == a {
                    "2026-10-12"
                } else {
                    "2026-10-10"
                };
                assert!(
                    hit.matched_filters
                        .iter()
                        .any(|f| f.key == "edited-at-key" && f.value.starts_with(day)),
                    "updated value for {query:?}: {hit:?}"
                );
            }
        }
        let reparsed = v.parse_search(parsed.spec.to_query_string()).unwrap();
        assert!(reparsed.spec.warnings.is_empty(), "roundtrip {query:?}");
        assert_eq!(
            v.search_spec(reparsed.spec, 10, 0)
                .unwrap()
                .hits
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>(),
            expected,
            "roundtrip {query:?}"
        );
    }
}

#[test]
fn created_updated_and_date_match_any_date_on_the_page() {
    let t = TempVault::new();
    let v = t.open();
    let created = page(&v, "Created", "");
    let updated = page(&v, "Updated", "");
    let property = page(&v, "Property", "");
    let outside = page(&v, "Outside", "");
    date(&v, &property, "event", OCT_10 + DAY / 2);
    v.flush().unwrap();
    for id in [&created, &updated, &property, &outside] {
        t.timestamps(id, OCT_10 - 40 * DAY, OCT_10 - 20 * DAY);
    }
    t.timestamps(&created, OCT_10 + DAY / 2, OCT_10 + 2 * DAY);
    t.timestamps(&updated, OCT_10 - 40 * DAY, OCT_10 + DAY / 2);
    assert_ids(&v, "created:2026-10-10", &[&created]);
    assert_ids(&v, "updated:2026-10-10", &[&updated]);
    assert_ids(&v, "date:2026-10-10", &[&created, &updated, &property]);
    assert_ids(&v, "created:2026-10", &[&created]);
    assert_ids(&v, "created:>2026-09", &[&created]);
    assert_ids(&v, "created:2026-09..2026-10", &[&created]);
    assert_ids(&v, "date:2026", &[&created, &updated, &property, &outside]);
    assert_ids(&v, "-date:2026-10-10", &[&outside]);
}

#[test]
fn subtree_filters_are_pinned_and_invalidate_after_tree_changes() {
    let t = TempVault::new();
    let v = t.open();
    let root = page(&v, "Projects", "");
    let child = v.create_page(Some(root.clone()), "Child".into()).unwrap();
    let grandchild = v
        .create_page(Some(child.clone()), "Grandchild".into())
        .unwrap();
    let elsewhere = page(&v, "Elsewhere", "");
    v.flush().unwrap();
    let parsed = v.parse_search("in:[[Projects]]".into()).unwrap();
    assert_ids(&v, "in:\"Projects\"", &[&root, &child, &grandchild]);
    assert_ids(
        &v,
        "in:[[Projects]],[[Elsewhere]]",
        &[&root, &child, &grandchild, &elsewhere],
    );
    assert_ids(&v, "in:[[Projects]] in:[[Child]]", &[&child, &grandchild]);
    assert_ids(&v, "in:[[Projects]] -in:[[Child]]", &[&root]);
    v.rename_page(root.clone(), "Renamed projects".into())
        .unwrap();
    let pinned = v.search_spec(parsed.spec.clone(), 100, 0).unwrap();
    assert_eq!(
        pinned
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<BTreeSet<_>>(),
        [&root, &child, &grandchild].into_iter().cloned().collect()
    );
    let printed = parsed.spec.to_query_string();
    let reparsed = v.parse_search(printed).unwrap();
    assert_eq!(
        v.search_spec(reparsed.spec, 100, 0)
            .unwrap()
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<BTreeSet<_>>(),
        pinned.hits.iter().map(|h| h.id.clone()).collect()
    );
    v.move_page(child.clone(), Some(elsewhere), None).unwrap();
    assert_eq!(
        v.search_spec(parsed.spec.clone(), 100, 0)
            .unwrap()
            .hits
            .iter()
            .map(|h| &h.id)
            .collect::<Vec<_>>(),
        vec![&root]
    );
    v.move_page(child.clone(), Some(root.clone()), None)
        .unwrap();
    assert_eq!(
        v.search_spec(parsed.spec.clone(), 100, 0)
            .unwrap()
            .hits
            .len(),
        3
    );
    let fresh = v
        .create_page(Some(root.clone()), "New child".into())
        .unwrap();
    assert_eq!(
        v.search_spec(parsed.spec.clone(), 100, 0)
            .unwrap()
            .hits
            .len(),
        4
    );
    v.delete_page(child.clone()).unwrap();
    assert_eq!(
        v.search_spec(parsed.spec.clone(), 100, 0)
            .unwrap()
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<BTreeSet<_>>(),
        [root.clone(), fresh].into_iter().collect()
    );
    v.restore_page(child).unwrap();
    assert_eq!(v.search_spec(parsed.spec, 100, 0).unwrap().hits.len(), 4);
}

#[test]
fn ambiguous_titles_resolve_one_subtree_and_keep_its_id_after_rename() {
    let t = TempVault::new();
    let v = t.open();
    let a = page(&v, "Projects", "");
    let b = page(&v, "Projects", "");
    let aa = v.create_page(Some(a.clone()), "A child".into()).unwrap();
    let bb = v.create_page(Some(b.clone()), "B child".into()).unwrap();
    let parsed = v.parse_search("in:\"Projects\"".into()).unwrap();
    let got = v.search_spec(parsed.spec.clone(), 100, 0).unwrap();
    let got_ids: BTreeSet<_> = got.hits.iter().map(|h| h.id.clone()).collect();
    let a_tree = [a.clone(), aa].into_iter().collect();
    let b_tree = [b.clone(), bb].into_iter().collect();
    assert!(
        got_ids == a_tree || got_ids == b_tree,
        "one resolved subtree: {got_ids:?}"
    );
    v.rename_page(a, "First renamed".into()).unwrap();
    v.rename_page(b, "Second renamed".into()).unwrap();
    assert_eq!(
        v.search_spec(parsed.spec, 100, 0)
            .unwrap()
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<BTreeSet<_>>(),
        got_ids
    );
}

#[test]
fn title_filter_does_not_match_body_text() {
    let t = TempVault::new();
    let v = t.open();
    let title = page(&v, "Garden plan", "tomato harvest");
    let body = page(&v, "Other", "garden plan tomato harvest");
    v.flush().unwrap();
    assert_ids(&v, "title:garden", &[&title]);
    assert_ids(&v, "tomato title:garden", &[&title]);
    assert_ids(&v, "tomato -title:garden", &[&body]);
}

#[test]
fn links_from_and_content_flags_use_indexed_content() {
    let t = TempVault::new();
    let v = t.open();
    let target = page(&v, "Target", "");
    let source = page(&v, "Source", "");
    let file = page(&v, "File", "");
    let plain = page(&v, "Plain", "");
    v.apply(
        source.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: new_id(),
                kind: BlockKind::Paragraph,
                text: "Target".into(),
                marks: vec![Mark {
                    start: 0,
                    end: 6,
                    kind: MarkKind::PageLink(target.clone()),
                }],
                checked: false,
            },
        }],
    )
    .unwrap();
    let attachment = v
        .add_attachment_bytes("example.txt".into(), b"attachment".to_vec())
        .unwrap();
    let block_id = new_id();
    v.apply(
        file.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: block_id.clone(),
                    kind: BlockKind::File,
                    text: "example.txt".into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: block_id,
                target: Some(attachment),
            },
        ],
    )
    .unwrap();
    v.flush().unwrap();
    assert_ids(&v, "links:[[Target]]", &[&source]);
    assert_ids(&v, "from:[[Source]]", &[&target]);
    assert_ids(&v, "has:link", &[&source]);
    assert_ids(&v, "has:attachment", &[&file]);
    assert_ids(&v, "-has:link -has:attachment", &[&target, &plain]);
}

#[test]
fn phrase_and_excluded_words_are_constraints_beside_filters() {
    let t = TempVault::new();
    let v = t.open();
    let exact = page(&v, "One", "weekly review garden notes");
    let separated = page(&v, "Two", "weekly garden review notes");
    let excluded = page(&v, "Three", "weekly review garden secret");
    let outside = page(&v, "Weekly review", "garden notes");
    for id in [&exact, &separated, &excluded] {
        tags(&v, id, &["work"]);
    }
    tags(&v, &outside, &["home"]);
    v.flush().unwrap();
    assert_ids(&v, "\"weekly review\" tag:work -secret", &[&exact]);
    assert_ids(&v, "weekly review tag:work -secret", &[&exact, &separated]);
    assert_ids(&v, "tag:work -\"weekly review\"", &[&separated]);
}

#[test]
fn corrections_and_title_subsequences_never_loosen_filters() {
    let t = TempVault::new();
    let v = t.open();
    let allowed_title = page(&v, "Garden plan", "seedlings");
    let allowed_body = page(&v, "A", "garden plan");
    let disallowed = page(&v, "Garden planning", "garden plan garden garden");
    tags(&v, &allowed_title, &["work"]);
    tags(&v, &allowed_body, &["work"]);
    tags(&v, &disallowed, &["home"]);
    v.flush().unwrap();
    for query in [
        "gardne tag:work",
        "grdn pln tag:work",
        "gadr tag:work",
        "garden -tag:home",
        "gardne -tag:home",
    ] {
        let result = search(&v, query, 20, 0);
        assert!(!result.hits.is_empty(), "{query:?}");
        assert!(
            result
                .hits
                .iter()
                .all(|h| h.id == allowed_title || h.id == allowed_body),
            "{query:?}: {:?}",
            result.hits
        );
        if query.contains("gardne") || query.contains("grdn") || query.contains("gadr") {
            assert!(result.hits.iter().any(|h| h.fuzzy));
        }
        assert_eq!(
            v.search(query.into(), 20)
                .unwrap()
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>(),
            result.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>()
        );
        let full = v.search_full(query.into(), 20).unwrap();
        assert!(!full.is_empty(), "full path {query:?}");
        assert!(
            full.iter()
                .all(|h| h.id == allowed_title || h.id == allowed_body),
            "full path {query:?}: {full:?}"
        );
    }
    assert!(
        search(&v, "gardne tag:wrko", 20, 0).hits.is_empty(),
        "property values do not receive text correction"
    );
}

#[test]
fn invalid_known_operators_warn_and_are_ignored_unknown_keys_are_text() {
    let t = TempVault::new();
    let v = t.open();
    let garden = page(&v, "Garden", "mystery banana");
    let other = page(&v, "Other", "");
    v.flush().unwrap();
    for query in [
        "created:banana",
        "updated:banana",
        "date:banana",
        "-created:banana",
    ] {
        let parsed = v.parse_search(query.into()).unwrap();
        let result = v.search_spec(parsed.spec, 10, 0).unwrap();
        assert!(!result.warnings.is_empty(), "{query:?}");
        assert_eq!(
            result
                .hits
                .iter()
                .map(|h| h.id.clone())
                .collect::<BTreeSet<_>>(),
            [garden.clone(), other.clone()].into_iter().collect()
        );
    }
    assert_ids(&v, "garden created:banana", &[&garden]);
    tags(&v, &garden, &["work"]);
    tags(&v, &other, &["home"]);
    assert_ids(&v, "tag:work created:banana", &[&garden]);
    assert_ids(&v, "mystery:banana", &[&garden]);
    assert!(search(&v, "unknown:unfindable", 10, 0).hits.is_empty());
}

#[test]
fn filters_remain_current_after_property_changes_and_hide_trash_templates() {
    let t = TempVault::new();
    let v = t.open();
    let live = page(&v, "Live", "");
    let trashed = page(&v, "Trashed", "");
    let template_parent = page(&v, "Template parent", "");
    let template = v
        .create_page(Some(template_parent), "Template".into())
        .unwrap();
    for id in [&live, &trashed, &template] {
        tags(&v, id, &["work"]);
    }
    v.set_template(template, true).unwrap();
    v.delete_page(trashed.clone()).unwrap();
    v.flush().unwrap();
    assert_ids(&v, "tag:work", &[&live]);
    tags(&v, &live, &["home"]);
    assert!(ids(&v, "tag:work").is_empty());
    assert_ids(&v, "tag:home", &[&live]);
    v.restore_page(trashed.clone()).unwrap();
    assert_ids(&v, "tag:work", &[&trashed]);
}

#[test]
fn filter_only_sort_offset_and_bounded_totals() {
    let t = TempVault::new();
    let v = t.open();
    let mut expected = Vec::new();
    for n in 0..225 {
        let id = page(&v, &format!("Item {n:03}"), "");
        tags(&v, &id, &["work"]);
        prop(&v, &id, "points", PropValue::Number(n as f64));
        expected.push(id);
    }
    v.flush().unwrap();
    let first = search(&v, "tag:work sort:title", 10, 0);
    assert_eq!(first.total_bounded, 201);
    assert_eq!(
        first.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        expected[..10]
    );
    let second = search(&v, "tag:work sort:title", 10, 10);
    assert_eq!(second.total_bounded, first.total_bounded);
    assert_eq!(
        second.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        expected[10..20]
    );
    assert!(search(&v, "tag:work sort:title", 10, 225).hits.is_empty());
    assert!(search(&v, "tag:work", 0, 0).hits.is_empty());
    let numeric = search(&v, "tag:work sort:points", 5, 15);
    assert_eq!(
        numeric
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<Vec<_>>(),
        expected[15..20]
    );
    let small = search(&v, "points:0..3 sort:title", 2, 1);
    assert_eq!(small.total_bounded, 4);
    assert_eq!(
        small.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        expected[1..3]
    );
}

#[test]
fn negatives_only_default_and_explicit_timestamp_order() {
    let t = TempVault::new();
    let v = t.open();
    let a = page(&v, "A", "");
    let b = page(&v, "B", "");
    let excluded = page(&v, "Excluded", "secret");
    tags(&v, &excluded, &["work"]);
    v.flush().unwrap();
    t.timestamps(&a, OCT_10 - 2 * DAY, OCT_10);
    t.timestamps(&b, OCT_10 - DAY, OCT_10 - DAY);
    t.timestamps(&excluded, OCT_10, OCT_10 + DAY);
    let ordered = |query| {
        search(&v, query, 10, 0)
            .hits
            .into_iter()
            .map(|h| h.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ordered("-tag:work"), [a.clone(), b.clone()]);
    assert_eq!(ordered("-secret"), [a.clone(), b.clone()]);
    assert_eq!(ordered("-tag:work sort:updated"), [a.clone(), b.clone()]);
    assert_eq!(ordered("-tag:work sort:created"), [b, a]);
}

#[test]
fn one_date_filter_orders_past_newest_and_future_soonest() {
    let t = TempVault::new();
    let v = t.open();
    let early = page(&v, "Early", "");
    let later = page(&v, "Later", "");
    date(&v, &early, "due", OCT_10 + DAY / 2);
    date(&v, &later, "due", OCT_10 + 2 * DAY + DAY / 2);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let soon = page(&v, "Soon", "");
    let far = page(&v, "Far", "");
    date(&v, &soon, "due", now + DAY);
    date(&v, &far, "due", now + 3 * DAY);
    v.flush().unwrap();
    // Choose an unequivocally past year, without depending on this month's day.
    date(&v, &early, "due", OCT_10 - 365 * DAY + DAY / 2);
    date(&v, &later, "due", OCT_10 - 365 * DAY + 2 * DAY + DAY / 2);
    let past = search(&v, "due:2025", 10, 0);
    assert_eq!(
        past.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        [later, early]
    );
    let future = search(&v, "due:+7d", 10, 0);
    assert_eq!(
        future.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        [soon, far]
    );
    assert!(future.hits.iter().all(|h| h
        .matched_filters
        .iter()
        .any(|f| f.key == "due" && !f.value.is_empty())));
}

#[test]
fn completion_offers_keys_values_dates_and_page_ids_with_byte_spans() {
    let t = TempVault::new();
    let v = t.open();
    let projects = page(&v, "Projects Zürich", "");
    let item = page(&v, "Item", "");
    tags(&v, &item, &["work", "work/meetings", "home"]);
    prop(
        &v,
        &item,
        "Due date",
        PropValue::Date {
            start_ms: OCT_10,
            end_ms: None,
            has_time: false,
        },
    );
    prop(&v, &item, "state", PropValue::Select("In progress".into()));
    v.flush().unwrap();
    let keys = v.complete_search("upd".into(), 3).unwrap();
    assert!(keys.iter().any(|c| c.value.starts_with("updated:")));
    let property_keys = v.complete_search("due".into(), 3).unwrap();
    assert!(property_keys
        .iter()
        .any(|c| c.label.to_lowercase().contains("due")));
    assert!(
        v.complete_search("café".into(), 4).is_err(),
        "cursor inside é is invalid"
    );
    assert!(
        v.complete_search("café".into(), 99).is_err(),
        "cursor beyond query is invalid"
    );
    for (query, wanted) in [
        ("tag:wo", "work"),
        ("#wo", "work"),
        ("state:In", "In progress"),
        ("date:to", "today"),
    ] {
        let completions = v.complete_search(query.into(), query.len() as u32).unwrap();
        assert!(
            completions
                .iter()
                .any(|c| c.value.contains(wanted) || c.label.contains(wanted)),
            "{query:?}: {completions:?}"
        );
    }
    for key in ["in", "links", "from"] {
        let query = format!("café {key}:ProjectsSuffix trailing");
        let cursor = (query.find("ProjectsSuffix").unwrap() + "Proj".len()) as u32;
        let completions = v.complete_search(query.clone(), cursor).unwrap();
        let completion = completions
            .iter()
            .find(|c| c.resolved_id.as_deref() == Some(projects.as_str()))
            .unwrap_or_else(|| panic!("{query:?}: {completions:?}"));
        assert!(query.is_char_boundary(completion.start as usize));
        assert!(query.is_char_boundary(completion.end as usize));
        assert_eq!(
            &query[completion.start as usize..completion.end as usize],
            "ProjectsSuffix"
        );
        let applied = format!(
            "{}{}{}",
            &query[..completion.start as usize],
            completion.value,
            &query[completion.end as usize..]
        );
        assert!(applied.starts_with("café ") && applied.ends_with(" trailing"));
        assert!(v.parse_search(applied).unwrap().spec.warnings.is_empty());
    }
    let query = "tag:home,woSuffix";
    let cursor = (query.find("woSuffix").unwrap() + 2) as u32;
    let suggestions = v.complete_search(query.into(), cursor).unwrap();
    let work = suggestions.iter().find(|c| c.label == "work").unwrap();
    assert_eq!(&query[work.start as usize..work.end as usize], "woSuffix");
    assert_eq!(
        format!("{}{}", &query[..work.start as usize], work.value),
        "tag:home,work"
    );
    let query = "tag:woSuffix,home";
    let cursor = (query.find("woSuffix").unwrap() + 2) as u32;
    let suggestions = v.complete_search(query.into(), cursor).unwrap();
    let work = suggestions.iter().find(|c| c.label == "work").unwrap();
    assert_eq!(&query[work.start as usize..work.end as usize], "woSuffix");
    assert_eq!(
        format!(
            "{}{}{}",
            &query[..work.start as usize],
            work.value,
            &query[work.end as usize..]
        ),
        "tag:work,home"
    );
    let query = "#woSuffix";
    let suggestions = v.complete_search(query.into(), 3).unwrap();
    let work = suggestions.iter().find(|c| c.label == "work").unwrap();
    assert_eq!(&query[work.start as usize..work.end as usize], "woSuffix");
    assert_eq!(
        format!("{}{}", &query[..work.start as usize], work.value),
        "#work"
    );
}

#[test]
fn shorthand_completion_preserves_negation_and_other_comma_alternatives() {
    let t = TempVault::new();
    let v = t.open();
    let item = page(&v, "Item", "");
    tags(&v, &item, &["work", "home", "urgent"]);
    v.flush().unwrap();
    for sign in ["", "-"] {
        for (values, completed) in [
            ("woSuffix,home", "work,home"),
            ("home,woSuffix", "home,work"),
            ("home,woSuffix,urgent", "home,work,urgent"),
        ] {
            let query = format!("café {sign}#{values} trailing");
            let cursor = (query.find("woSuffix").unwrap() + 2) as u32;
            let suggestions = v.complete_search(query.clone(), cursor).unwrap();
            let work = suggestions
                .iter()
                .find(|c| c.label == "work")
                .unwrap_or_else(|| panic!("{query:?}: {suggestions:?}"));
            assert_eq!(&query[work.start as usize..work.end as usize], "woSuffix");
            let applied = format!(
                "{}{}{}",
                &query[..work.start as usize],
                work.value,
                &query[work.end as usize..]
            );
            assert_eq!(applied, format!("café {sign}#{completed} trailing"));
            let parsed = v.parse_search(applied).unwrap();
            assert!(parsed.spec.warnings.is_empty());
            assert_eq!(parsed.spec.operators.len(), 1);
            assert_eq!(parsed.spec.operators[0].negated, sign == "-");
        }
    }
}

#[test]
fn completion_builtin_values_win_over_shadow_properties_unless_key_is_quoted() {
    let t = TempVault::new();
    let v = t.open();
    let item = page(&v, "Item", "");
    tags(&v, &item, &["todo"]);
    date(&v, &item, "tag", OCT_10);
    prop(
        &v,
        &item,
        "title",
        PropValue::Select("Shadow option".into()),
    );
    v.flush().unwrap();
    let suggestions = v.complete_search("tag:to".into(), 6).unwrap();
    assert!(suggestions.iter().any(|c| c.label == "todo"));
    assert!(!suggestions
        .iter()
        .any(|c| matches!(c.label.as_str(), "today" | "tomorrow")));
    let query = "\"tag\":to";
    let suggestions = v.complete_search(query.into(), query.len() as u32).unwrap();
    let today = suggestions.iter().find(|c| c.label == "today").unwrap();
    let applied = format!(
        "{}{}{}",
        &query[..today.start as usize],
        today.value,
        &query[today.end as usize..]
    );
    let parsed = v.parse_search(applied).unwrap();
    assert!(parsed.spec.warnings.is_empty());
    assert!(parsed.spec.operators[0].property);
    assert_eq!(parsed.spec.operators[0].key, "tag");
    assert!(!v
        .complete_search("title:Sh".into(), 8)
        .unwrap()
        .iter()
        .any(|c| c.label == "Shadow option"));
    let query = "\"title\":Sh";
    assert!(v
        .complete_search(query.into(), query.len() as u32)
        .unwrap()
        .iter()
        .any(|c| c.label == "Shadow option"));
}

#[test]
fn completion_and_parser_both_prefer_stable_property_keys_over_display_names() {
    let t = TempVault::new();
    let v = t.open();
    let collection = page(&v, "Collection", "");
    let definition = |key: &str, name: &str, option_id: &str, label: &str| PropDef {
        key: key.into(),
        name: name.into(),
        kind: PropKind::Select,
        options: vec![SelectOption {
            id: option_id.into(),
            name: label.into(),
            color: String::new(),
        }],
        config: String::new(),
    };
    v.set_schema(
        collection.clone(),
        vec![
            definition("a-key", "Status", "wrong-id", "Wrong option"),
            definition("status", "Other", "right-id", "Right option"),
        ],
    )
    .unwrap();
    let item = v.create_page(Some(collection), "Item".into()).unwrap();
    prop(&v, &item, "a-key", PropValue::Select("wrong-id".into()));
    prop(&v, &item, "status", PropValue::Select("right-id".into()));
    v.flush().unwrap();
    let suggestions = v.complete_search("status:".into(), 7).unwrap();
    assert!(suggestions.iter().any(|c| c.label == "Right option"));
    assert!(!suggestions.iter().any(|c| c.label == "Wrong option"));
    let query = "status:R";
    let suggestions = v.complete_search(query.into(), query.len() as u32).unwrap();
    let right = suggestions
        .iter()
        .find(|c| c.label == "Right option")
        .unwrap();
    let applied = format!(
        "{}{}{}",
        &query[..right.start as usize],
        right.value,
        &query[right.end as usize..]
    );
    let parsed = v.parse_search(applied).unwrap();
    assert!(parsed.spec.warnings.is_empty());
    assert_eq!(parsed.spec.operators[0].key, "status");
    assert_eq!(
        v.search_spec(parsed.spec, 10, 0)
            .unwrap()
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<Vec<_>>(),
        [item]
    );
    assert!(v.complete_search("status:W".into(), 8).unwrap().is_empty());
}

#[test]
fn completion_decodes_escaped_quoted_keys_and_closed_or_partial_unicode_values() {
    let t = TempVault::new();
    let v = t.open();
    let collection = page(&v, "Collection", "");
    let name = "École \"state\"\\desk";
    let label = "Work \"review\" café\\morning";
    v.set_schema(
        collection.clone(),
        vec![PropDef {
            key: "quoted-state".into(),
            name: name.into(),
            kind: PropKind::Select,
            options: vec![SelectOption {
                id: "quoted-option".into(),
                name: label.into(),
                color: String::new(),
            }],
            config: String::new(),
        }],
    )
    .unwrap();
    let row = v.create_page(Some(collection), "Row".into()).unwrap();
    prop(
        &v,
        &row,
        "quoted-state",
        PropValue::Select("quoted-option".into()),
    );
    let free = page(&v, "Free tags", "");
    tags(&v, &free, &["work"]);
    v.flush().unwrap();
    let escaped = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let query = format!("\"{}\":\"{}\"", escaped(name), escaped(label));
    for cursor in [query.len(), query.find("café").unwrap() + "café".len()] {
        let suggestions = v.complete_search(query.clone(), cursor as u32).unwrap();
        let completion = suggestions
            .iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("cursor {cursor}: {suggestions:?}"));
        assert_eq!(completion.start as usize, query.find(':').unwrap() + 1);
        assert_eq!(completion.end as usize, query.len());
        assert!(query.is_char_boundary(completion.start as usize));
        assert!(query.is_char_boundary(completion.end as usize));
        let applied = format!(
            "{}{}{}",
            &query[..completion.start as usize],
            completion.value,
            &query[completion.end as usize..]
        );
        let parsed = v.parse_search(applied).unwrap();
        assert!(parsed.spec.warnings.is_empty());
        assert_eq!(parsed.spec.operators.len(), 1);
        assert_eq!(parsed.spec.operators[0].key, "quoted-state");
        assert!(parsed.spec.operators[0].property);
        assert_eq!(
            v.search_spec(parsed.spec, 10, 0)
                .unwrap()
                .hits
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>(),
            [row.clone()]
        );
    }
    let query = "tag:\"work\"";
    let suggestions = v.complete_search(query.into(), query.len() as u32).unwrap();
    let work = suggestions.iter().find(|c| c.label == "work").unwrap();
    assert_eq!(&query[work.start as usize..work.end as usize], "\"work\"");
    let applied = format!("{}{}", &query[..work.start as usize], work.value);
    assert_eq!(applied, "tag:work");
    assert!(v.parse_search(applied).unwrap().spec.warnings.is_empty());
}

#[test]
fn stored_any_date_spec_matches_properties_added_after_parsing() {
    let t = TempVault::new();
    let v = t.open();
    let id = page(&v, "New date", "");
    v.flush().unwrap();
    t.timestamps(&id, OCT_10 - 40 * DAY, OCT_10 - 20 * DAY);
    let spec = v.parse_search("date:2026-10-10".into()).unwrap().spec;
    assert!(v.search_spec(spec.clone(), 10, 0).unwrap().hits.is_empty());
    date(&v, &id, "new-event-date", OCT_10 + DAY / 2);
    // Property edits update the page timestamp as well; keep it outside the
    // range so only the newly added date property can satisfy the stored spec.
    v.flush().unwrap();
    t.timestamps(&id, OCT_10 - 40 * DAY, OCT_10 - 20 * DAY);
    let result = v.search_spec(spec.clone(), 10, 0).unwrap();
    assert_eq!(
        result.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        [id.clone()]
    );
    assert!(result.hits[0]
        .matched_filters
        .iter()
        .any(|f| f.key == "new-event-date" && f.value.starts_with("2026-10-10")));
    v.remove_property(id.clone(), "new-event-date".into())
        .unwrap();
    v.flush().unwrap();
    t.timestamps(&id, OCT_10 - 40 * DAY, OCT_10 - 20 * DAY);
    assert!(v.search_spec(spec, 10, 0).unwrap().hits.is_empty());
}

#[test]
fn body_correction_preserves_existing_prefixes_and_hard_filter() {
    let t = TempVault::new();
    let v = t.open();
    let wanted = page(&v, "A", "garden tools");
    let extended_word = page(&v, "B", "gardener tools");
    let wrong_filter = page(&v, "C", "garden tools");
    tags(&v, &wanted, &["work"]);
    tags(&v, &extended_word, &["work"]);
    tags(&v, &wrong_filter, &["home"]);
    v.flush().unwrap();
    for query in [
        "garden tools tag:work",
        "garden tolls tag:work",
        "garden tolls tag:work sort:title",
    ] {
        let result = search(&v, query, 10, 0);
        assert_eq!(
            result
                .hits
                .iter()
                .map(|h| h.id.clone())
                .collect::<BTreeSet<_>>(),
            [wanted.clone(), extended_word.clone()]
                .into_iter()
                .collect(),
            "{query:?}"
        );
        assert_eq!(result.total_bounded, 2);
        if query.contains("tolls") {
            assert!(result.hits.iter().all(|h| h.fuzzy));
            assert!(result.hits.iter().all(|h| matches!(
                h.corrected.as_deref(),
                Some("garden tools" | "gardener tools")
            )));
        }
    }
    assert_ids(&v, "\"garden tools\" tag:work", &[&wanted]);
}

#[test]
fn collection_tags_match_option_labels_and_report_actual_hierarchical_value() {
    let t = TempVault::new();
    let v = t.open();
    let collection = page(&v, "Tagged collection", "");
    v.set_schema(
        collection.clone(),
        vec![PropDef {
            key: "opaque-tag-key".into(),
            name: "Tags".into(),
            kind: PropKind::MultiSelect,
            options: vec![
                SelectOption {
                    id: "opaque-work-id".into(),
                    name: "Work/Meetings".into(),
                    color: String::new(),
                },
                SelectOption {
                    id: "opaque-school-id".into(),
                    name: "École".into(),
                    color: String::new(),
                },
            ],
            config: String::new(),
        }],
    )
    .unwrap();
    let row = v
        .create_page(Some(collection), "Tagged row".into())
        .unwrap();
    prop(
        &v,
        &row,
        "opaque-tag-key",
        PropValue::MultiSelect(vec!["opaque-work-id".into(), "opaque-school-id".into()]),
    );
    v.flush().unwrap();
    assert_ids(&v, "tag:work", &[&row]);
    assert_ids(&v, "#ÉCOLE", &[&row]);
    let result = search(&v, "tag:work", 10, 0);
    assert!(result.hits[0]
        .matched_filters
        .iter()
        .any(|f| f.key == "tag" && f.value == "Work/Meetings"));
}

#[test]
fn tag_index_backfills_existing_properties_and_tracks_full_lifecycle() {
    let t = TempVault::new();
    let v = t.open();
    let free = page(&v, "Free tags", "");
    tags(&v, &free, &["Work", "école", "work,home"]);
    let collection = page(&v, "Collection tags", "");
    v.set_schema(
        collection.clone(),
        vec![PropDef {
            key: "opaque-tags".into(),
            name: "Tags".into(),
            kind: PropKind::MultiSelect,
            options: vec![
                SelectOption {
                    id: "opaque-work-id".into(),
                    name: "Work/Meetings".into(),
                    color: String::new(),
                },
                SelectOption {
                    id: "opaque-home-id".into(),
                    name: "Home".into(),
                    color: String::new(),
                },
            ],
            config: String::new(),
        }],
    )
    .unwrap();
    let row = v
        .create_page(Some(collection.clone()), "Collection row".into())
        .unwrap();
    prop(
        &v,
        &row,
        "opaque-tags",
        PropValue::MultiSelect(vec!["opaque-work-id".into(), "opaque-home-id".into()]),
    );
    v.flush().unwrap();
    assert_ids(&v, "tag:work", &[&free, &row]);
    drop(v);

    // Simulate an older, otherwise complete derived index. The original props
    // and persisted page documents remain intact; only the new acceleration
    // table and its maintenance triggers are absent when this vault reopens.
    {
        let db = rusqlite::Connection::open(t.0.join(".vault/index.sqlite")).unwrap();
        db.execute_batch(
            "DROP TRIGGER IF EXISTS search_multi_insert;
            DROP TRIGGER IF EXISTS search_multi_update;
            DROP TRIGGER IF EXISTS search_multi_delete;
            DROP TRIGGER IF EXISTS search_multi_page_delete;
            DROP TABLE search_multi;",
        )
        .unwrap();
        let props: u32 = db
            .query_row("SELECT count(*) FROM props", [], |r| r.get(0))
            .unwrap();
        assert_eq!(props, 2, "legacy properties must survive migration setup");
    }
    let v = t.open();
    assert_ids(&v, "tag:work", &[&free, &row]);
    assert_ids(&v, "tag:ÉCOLE", &[&free]);
    assert_ids(&v, "tag:\"work,home\"", &[&free]);
    assert_ids(&v, "tag:home", &[&row]);

    // Replacements and removals must discard stale value rows even before a
    // background content flush, for both raw tags and schema option IDs.
    tags(&v, &free, &["home"]);
    assert_ids(&v, "tag:work", &[&row]);
    assert!(ids(&v, "tag:école").is_empty());
    assert!(ids(&v, "tag:\"work,home\"").is_empty());
    prop(
        &v,
        &row,
        "opaque-tags",
        PropValue::MultiSelect(vec!["opaque-home-id".into()]),
    );
    assert!(ids(&v, "tag:work").is_empty());
    assert_ids(&v, "tag:home", &[&free, &row]);
    v.remove_property(free.clone(), "tags".into()).unwrap();
    assert_ids(&v, "tag:home", &[&row]);
    v.remove_property(row.clone(), "opaque-tags".into())
        .unwrap();
    assert!(ids(&v, "tag:home").is_empty());

    tags(&v, &free, &["work"]);
    prop(
        &v,
        &row,
        "opaque-tags",
        PropValue::MultiSelect(vec!["opaque-home-id".into()]),
    );
    v.delete_page(collection.clone()).unwrap();
    assert!(ids(&v, "tag:home").is_empty());
    assert_ids(&v, "tag:work", &[&free]);
    v.restore_page(collection).unwrap();
    assert_ids(&v, "tag:home", &[&row]);
    v.flush().unwrap();
    assert_eq!(v.reindex().unwrap(), 3);
    assert_ids(&v, "tag:work", &[&free]);
    assert_ids(&v, "tag:home", &[&row]);
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    assert_ids(&v, "tag:work", &[&free]);
    assert_ids(&v, "tag:home", &[&row]);
    assert!(ids(&v, "tag:école").is_empty());
}

#[test]
fn filtered_breadth_and_text_pagination_use_only_matching_candidates() {
    let t = TempVault::new();
    let v = t.open();
    let mut selected = BTreeSet::new();
    for n in 0..300 {
        let id = page(&v, &format!("Body {n:03}"), "garden plan");
        if n < 12 {
            tags(&v, &id, &["work"]);
            selected.insert(id);
        }
    }
    v.flush().unwrap();
    for query in ["garden tag:work", "gardne tag:work"] {
        let whole = search(&v, query, 100, 0);
        assert!(
            !whole.narrowed,
            "12 filtered matches should not be broad: {query:?}"
        );
        assert_eq!(whole.total_bounded, 12);
        assert_eq!(
            whole
                .hits
                .iter()
                .map(|h| h.id.clone())
                .collect::<BTreeSet<_>>(),
            selected
        );
        let slice = search(&v, query, 5, 4);
        assert_eq!(slice.total_bounded, whole.total_bounded);
        assert_eq!(
            slice.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            whole.hits[4..9]
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>()
        );
        assert!(search(&v, query, 5, 12).hits.is_empty());
    }
}

#[test]
fn parsed_byte_spans_and_stored_spec_preserve_quoted_terms_and_filters() {
    let t = TempVault::new();
    let v = t.open();
    let wanted = page(&v, "Wanted", "café weekly review notes");
    let excluded = page(&v, "Excluded", "café weekly review secret");
    let wrong_tag = page(&v, "Wrong tag", "café weekly review notes");
    for id in [&wanted, &excluded, &wrong_tag] {
        tags(&v, id, &["work"]);
        prop(&v, id, "owner", PropValue::Text("Alice Smith".into()));
    }
    tags(&v, &wanted, &["work", "urgent"]);
    tags(&v, &excluded, &["work", "urgent"]);
    v.flush().unwrap();
    let query = "café \"weekly review\" -secret tag:work tag:urgent owner:\"Alice Smith\"";
    let parsed = v.parse_search(query.into()).unwrap();
    let spans: Vec<_> = parsed
        .tokens
        .iter()
        .map(|token| {
            let (start, end) = (token.start as usize, token.end as usize);
            assert!(start < end && end <= query.len());
            assert!(query.is_char_boundary(start) && query.is_char_boundary(end));
            &query[start..end]
        })
        .collect();
    assert_eq!(
        spans,
        [
            "café",
            "\"weekly review\"",
            "-secret",
            "tag:work",
            "tag:urgent",
            "owner:\"Alice Smith\""
        ]
    );
    let serialized = serde_json::to_string(&parsed.spec).unwrap();
    let stored: SearchSpec = serde_json::from_str(&serialized).unwrap();
    let restored = v.search_spec(stored.clone(), 10, 0).unwrap();
    assert_eq!(
        restored
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<Vec<_>>(),
        [wanted.clone()]
    );
    let reparsed = v.parse_search(stored.to_query_string()).unwrap();
    assert_eq!(
        v.search_spec(reparsed.spec, 10, 0)
            .unwrap()
            .hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<Vec<_>>(),
        [wanted]
    );
}
