//! Journal day-note identity, the fixed Tasks/Log shape, task links and
//! append-only quick logs.

use chrono::{Local, TimeZone};
use loro::{ExportMode, LoroDoc};
use std::collections::HashMap;
use std::{fs, path::PathBuf};
use vault_core::*;

const DAY: i64 = 1_791_244_800_000; // Tuesday 6 October 2026, UTC day stamp.
const DAY_MS: i64 = 86_400_000;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_day_notes_{}", new_id())))
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

fn block(id: &str, kind: BlockKind, text: &str) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind,
        text: text.into(),
        marks: vec![],
        checked: false,
    }
}

fn insert(v: &Vault, page: &str, parent: Option<&str>, after: Option<&str>, block: NewBlock) {
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: parent.map(str::to_owned),
            after: after.map(str::to_owned),
            block,
        }],
    )
    .unwrap();
}

fn dated(v: &Vault, page: &str, day: i64, timed: bool) {
    v.set_property(
        page.into(),
        "date".into(),
        PropValue::Date {
            start_ms: day,
            end_ms: None,
            has_time: timed,
        },
    )
    .unwrap();
}

/// The root order of a day note: a Tasks section, then Log. Says nothing
/// about content, so it also holds once logs or links are added.
fn assert_sections(blocks: &[Block]) {
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        (blocks[0].kind.clone(), blocks[0].text.as_str()),
        (BlockKind::Toggle, "Tasks")
    );
    assert_eq!(
        (blocks[1].kind.clone(), blocks[1].text.as_str()),
        (BlockKind::Toggle, "Log")
    );
    assert!(blocks[0].children.is_empty());
}

/// A brand-new day note: the sections in order, with both empty.
fn assert_fixed_sections(blocks: &[Block]) {
    assert_sections(blocks);
    assert!(blocks[1].children.is_empty());
}

/// A page with a date in its tasks type data.
fn dated_task(v: &Vault, title: &str, iso_day: &str) -> PageId {
    let page = v.create_page(None, title.into()).unwrap();
    v.type_data_set(
        page.clone(),
        "tasks".into(),
        vec!["date".into()],
        format!("\"{iso_day}\""),
    )
    .unwrap();
    page
}

/// An undated task: only tasks.status in its type data.
fn status_task(v: &Vault, title: &str) -> PageId {
    let page = v.create_page(None, title.into()).unwrap();
    v.type_data_set(
        page.clone(),
        "tasks".into(),
        vec!["status".into()],
        "\"todo\"".into(),
    )
    .unwrap();
    page
}

/// Inserts a LinkToPage block to `target`, the way the editor does: insert,
/// then set the target in the same batch.
fn link_block(
    v: &Vault,
    page: &str,
    parent: Option<&str>,
    after: Option<&str>,
    id: &str,
    target: &str,
) {
    v.apply(
        page.into(),
        vec![
            BlockOp::Insert {
                parent: parent.map(str::to_owned),
                after: after.map(str::to_owned),
                block: block(id, BlockKind::LinkToPage, ""),
            },
            BlockOp::SetTarget {
                id: id.into(),
                target: Some(target.into()),
            },
        ],
    )
    .unwrap();
}

fn push(from: &Vault, to: &Vault) {
    to.import_purged(from.purged_pages()).unwrap();
    let have: HashMap<_, _> = to
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    for v in from.page_versions().unwrap() {
        let since = have.get(&v.page).cloned().unwrap_or_default();
        if !version_includes(&since, &v.version).unwrap() {
            to.import_updates(v.page.clone(), from.export_updates(v.page, since).unwrap())
                .unwrap();
        }
    }
}

fn unrelated_snapshot(id: &str) -> Vec<u8> {
    let doc = LoroDoc::new();
    let page = doc.get_map("page");
    page.insert("id", id).unwrap();
    page.insert("title", "Unrelated content").unwrap();
    page.insert("created", 1_i64).unwrap();
    page.insert("updated", 1_i64).unwrap();
    doc.commit();
    doc.export(ExportMode::Snapshot).unwrap()
}

#[test]
fn independent_devices_create_the_same_journal_and_day_but_distinct_seed_blocks() {
    let (ta, tb) = (TempVault::new(), TempVault::new());
    let (a, b) = (ta.open(), tb.open());
    let pa = a
        .daily_note(DAY, "First title".into(), true)
        .unwrap()
        .unwrap();
    let pb = b
        .daily_note(DAY, "Second title".into(), true)
        .unwrap()
        .unwrap();
    let ja = a.find_by_role("journal".into()).unwrap().unwrap();
    let jb = b.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(ja, "000000000000000000000000JR");
    assert_eq!(ja, jb);
    assert_eq!(pa, pb);
    assert_eq!(pa, "307ZX565BYK7TKY0P3XHQ07H11");
    assert_eq!(pa.len(), 26);
    assert_ne!(pa, ja);
    let ra = a.load_page(pa.clone()).unwrap().blocks;
    let rb = b.load_page(pb.clone()).unwrap().blocks;
    assert!(ra.iter().all(|x| rb.iter().all(|y| x.id != y.id)));
    for (v, roots, side) in [(&a, &ra, "a"), (&b, &rb, "b")] {
        insert(
            v,
            &pa,
            Some(&roots[0].id),
            None,
            block(&new_id(), BlockKind::Todo, &format!("task on {side}")),
        );
        insert(
            v,
            &pa,
            Some(&roots[1].id),
            None,
            block(&new_id(), BlockKind::Paragraph, &format!("log on {side}")),
        );
    }
    push(&a, &b);
    push(&b, &a);
    for v in [&a, &b] {
        assert_eq!(v.page_count().unwrap(), 2);
        assert_eq!(v.list_children(Some(ja.clone()), 0, 100).unwrap().len(), 1);
        assert_eq!(
            v.daily_note(DAY, "Ignored".into(), true).unwrap(),
            Some(pa.clone())
        );
        let merged = v.load_page(pa.clone()).unwrap();
        assert_eq!(merged.blocks.len(), 2);
        let mut contents: Vec<_> = merged
            .blocks
            .iter()
            .flat_map(|r| &r.children)
            .map(|c| c.text.as_str())
            .collect();
        contents.sort();
        assert_eq!(contents, ["log on a", "log on b", "task on a", "task on b"]);
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
    assert_eq!(a.load_page(pa.clone()).unwrap(), b.load_page(pa).unwrap());
}

#[test]
fn legacy_journal_and_date_lookup_win_and_future_day_ids_use_that_journal() {
    let (ta, tb, tc) = (TempVault::new(), TempVault::new(), TempVault::new());
    let (a, b, c) = (ta.open(), tb.open(), tc.open());
    let journal = a.create_page(None, "My legacy Journal".into()).unwrap();
    a.set_schema(
        journal.clone(),
        vec![PropDef {
            key: "legacy-date".into(),
            name: "When".into(),
            kind: PropKind::Date,
            options: vec![],
            config: String::new(),
        }],
    )
    .unwrap();
    a.set_page_role(journal.clone(), Some("journal".into()))
        .unwrap();
    let existing = a
        .create_page(Some(journal.clone()), "Legacy day title".into())
        .unwrap();
    a.set_property(
        existing.clone(),
        "legacy-date".into(),
        PropValue::Date {
            start_ms: DAY,
            end_ms: None,
            has_time: false,
        },
    )
    .unwrap();
    insert(
        &a,
        &existing,
        None,
        None,
        block("old", BlockKind::Paragraph, "Keep legacy text"),
    );
    let before = a.load_page(existing.clone()).unwrap();
    push(&a, &b);
    for v in [&a, &b] {
        assert_eq!(
            v.daily_note(DAY, "Do not rename".into(), true).unwrap(),
            Some(existing.clone())
        );
        assert_eq!(v.load_page(existing.clone()).unwrap(), before);
        assert_eq!(
            v.find_by_role("journal".into()).unwrap(),
            Some(journal.clone())
        );
    }
    let next_a = a
        .daily_note(DAY + DAY_MS, "Next a".into(), true)
        .unwrap()
        .unwrap();
    let next_b = b
        .daily_note(DAY + DAY_MS, "Next b".into(), true)
        .unwrap()
        .unwrap();
    assert_eq!(next_a, next_b);
    assert_ne!(next_a, existing);
    let new_journal_day = c
        .daily_note(DAY + DAY_MS, "Next c".into(), true)
        .unwrap()
        .unwrap();
    assert_ne!(
        next_a, new_journal_day,
        "actual Journal key scopes day identity"
    );
    assert_ne!(
        next_a,
        a.daily_note(DAY + 2 * DAY_MS, "Following".into(), true)
            .unwrap()
            .unwrap()
    );
}

#[test]
fn trashed_preferred_day_and_journal_restore_without_resetting_content_or_dates() {
    let t = TempVault::new();
    let v = t.open();
    let day = v
        .daily_note(DAY, "Custom title".into(), true)
        .unwrap()
        .unwrap();
    v.append_log(DAY, "Keep this log".into()).unwrap();
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    v.rename_page(journal.clone(), "Custom Journal".into())
        .unwrap();
    let before = v.load_page(day.clone()).unwrap();
    v.delete_page(day.clone()).unwrap();
    assert_eq!(
        v.daily_note(DAY, "Replacement title".into(), true).unwrap(),
        Some(day.clone())
    );
    let restored = v.load_page(day.clone()).unwrap();
    assert!(!restored.in_trash);
    assert_eq!(restored.title, before.title);
    assert_eq!(restored.properties, before.properties);
    assert_eq!(restored.blocks, before.blocks);
    v.delete_page(journal.clone()).unwrap();
    assert!(v.find_by_role("journal".into()).unwrap().is_none());
    assert_eq!(
        v.daily_note(DAY, "Still ignored".into(), true).unwrap(),
        Some(day.clone())
    );
    assert_eq!(
        v.find_by_role("journal".into()).unwrap(),
        Some(journal.clone())
    );
    assert_eq!(v.load_page(journal).unwrap().title, "Custom Journal");
    let restored = v.load_page(day).unwrap();
    assert_eq!(restored.title, before.title);
    assert_eq!(restored.properties, before.properties);
    assert_eq!(restored.blocks, before.blocks);
}

#[test]
fn purged_preferred_day_and_journal_use_random_replacements() {
    let t = TempVault::new();
    let v = t.open();
    let day = v.daily_note(DAY, "Original".into(), true).unwrap().unwrap();
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    v.delete_page(day.clone()).unwrap();
    v.purge_page(day.clone()).unwrap();
    let replacement = v
        .daily_note(DAY, "Replacement".into(), true)
        .unwrap()
        .unwrap();
    assert_ne!(replacement, day);
    assert_eq!(replacement.len(), 26);
    assert!(v.purged_pages().contains(&day));
    assert!(v.load_page(day).is_err());
    assert_eq!(
        v.daily_note(DAY, String::new(), true).unwrap(),
        Some(replacement.clone())
    );
    v.delete_page(journal.clone()).unwrap();
    v.purge_page(journal.clone()).unwrap();
    let next = v
        .daily_note(DAY, "After Journal purge".into(), true)
        .unwrap()
        .unwrap();
    let next_journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_ne!(next_journal, journal);
    assert_eq!(next_journal.len(), 26);
    assert_ne!(next, replacement);
    assert!(v.purged_pages().contains(&journal));
    assert!(v.purged_pages().contains(&replacement));
    assert_eq!(v.daily_note(DAY, String::new(), true).unwrap(), Some(next));
}

#[test]
fn changed_day_identity_is_rejected_without_resetting_a_live_or_trashed_page() {
    for trash in [false, true] {
        for move_parent in [false, true] {
            let t = TempVault::new();
            let v = t.open();
            let day = v
                .daily_note(DAY, "Keep my changes".into(), true)
                .unwrap()
                .unwrap();
            insert(
                &v,
                &day,
                None,
                None,
                block("keep", BlockKind::Paragraph, "Keep this content"),
            );
            if move_parent {
                v.move_page(day.clone(), None, None).unwrap();
            } else {
                dated(&v, &day, DAY + DAY_MS, false);
            }
            if trash {
                v.delete_page(day.clone()).unwrap();
            }
            let before = v.load_page(day.clone()).unwrap();
            let before_count = v.page_count().unwrap();
            let error = v
                .daily_note(DAY, "Unwanted replacement".into(), true)
                .unwrap_err();
            assert!(
                matches!(error, VaultError::Invalid(message) if message.contains("occupied by an unrelated page"))
            );
            assert_eq!(v.load_page(day).unwrap(), before);
            assert_eq!(v.page_count().unwrap(), before_count);
        }
    }
}

#[test]
fn occupied_journal_key_rejects_unrelated_live_and_trashed_pages() {
    for trash in [false, true] {
        let t = TempVault::new();
        let v = t.open();
        let id = "000000000000000000000000JR";
        v.import_updates(id.into(), unrelated_snapshot(id)).unwrap();
        insert(
            &v,
            id,
            None,
            None,
            block("keep", BlockKind::Paragraph, "Keep unrelated data"),
        );
        if trash {
            v.delete_page(id.into()).unwrap();
        }
        let before = v.load_page(id.into()).unwrap();
        let before_count = v.page_count().unwrap();
        let error = v.daily_note(DAY, "Day".into(), true).unwrap_err();
        assert!(
            matches!(error, VaultError::Invalid(message) if message.contains("occupied by an unrelated page"))
        );
        assert_eq!(v.load_page(id.into()).unwrap(), before);
        assert_eq!(v.page_count().unwrap(), before_count);
    }
}

#[test]
fn occupied_document_without_index_row_is_reused_or_rejected_without_replacement() {
    let source = TempVault::new();
    let donor = source.open();
    let day = donor
        .daily_note(DAY, "Original orphan title".into(), true)
        .unwrap()
        .unwrap();
    donor.append_log(DAY, "Orphan log retained".into()).unwrap();
    let journal = donor.find_by_role("journal".into()).unwrap().unwrap();
    let expected = donor.load_page(day.clone()).unwrap();
    for unrelated in [false, true] {
        let t = TempVault::new();
        let v = t.open();
        v.import_updates(
            journal.clone(),
            donor.export_updates(journal.clone(), vec![]).unwrap(),
        )
        .unwrap();
        let path = t.0.join(".vault/docs").join(format!("{day}.loro"));
        let bytes = if unrelated {
            unrelated_snapshot(&day)
        } else {
            donor.export_updates(day.clone(), vec![]).unwrap()
        };
        fs::write(&path, &bytes).unwrap();
        assert_eq!(v.page_count().unwrap(), 1);
        if unrelated {
            assert!(v.daily_note(DAY, "Do not overwrite".into(), true).is_err());
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert_eq!(v.page_count().unwrap(), 1);
        } else {
            assert_eq!(
                v.daily_note(DAY, "Do not rename".into(), true).unwrap(),
                Some(day.clone())
            );
            assert_eq!(v.load_page(day.clone()).unwrap(), expected);
            assert_eq!(v.page_count().unwrap(), 2);
        }
    }
}

#[test]
fn malformed_daily_dates_are_rejected_before_creating_any_page() {
    let t = TempVault::new();
    let v = t.open();
    for day in [DAY + 1, DAY + 3_600_000, i64::MIN, i64::MAX] {
        for create in [false, true] {
            assert!(matches!(
                v.daily_note(day, "Bad day".into(), create),
                Err(VaultError::Invalid(_))
            ));
        }
    }
    assert_eq!(v.page_count().unwrap(), 0);
}

#[test]
fn journal_default_template_with_legacy_missing_flag_keeps_duplicate_sections() {
    let t = TempVault::new();
    let v = t.open();
    v.daily_note(DAY, "Day".into(), true).unwrap();
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    let template = v.create_template(journal.clone(), "Legacy template".into()).unwrap();
    for (id, name) in [("t0", "Tasks"), ("t1", "Tasks"), ("l0", "Log"), ("l1", "Log")] {
        insert(&v, &template, None, None, block(id, BlockKind::Toggle, name));
    }
    dated(&v, &template, DAY, false);
    v.set_default_template(journal, Some(template.clone())).unwrap();
    let legacy = LoroDoc::new();
    legacy.import(&v.export_updates(template.clone(), vec![]).unwrap()).unwrap();
    legacy.get_map("page").insert("istemplate", false).unwrap();
    legacy.commit();
    v.import_updates(template.clone(), legacy.export(ExportMode::Snapshot).unwrap()).unwrap();
    let before = v.load_page(template.clone()).unwrap();
    let vv = v.page_versions().unwrap();
    assert_eq!(before.blocks.len(), 4);
    assert_eq!(v.load_page(template.clone()).unwrap(), before);
    assert_eq!(v.page_versions().unwrap(), vv);
    v.flush().unwrap(); drop(v);
    assert_eq!(t.open().load_page(template).unwrap(), before);
}

#[test]
fn existing_notes_and_journal_default_template_are_preserved_and_new_notes_skip_it() {
    let t = TempVault::new();
    let v = t.open();
    let existing = v
        .daily_note(DAY, "My original title".into(), true)
        .unwrap()
        .unwrap();
    insert(
        &v,
        &existing,
        None,
        None,
        block("original", BlockKind::Heading2, "Plan"),
    );
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    let template = v
        .create_template(journal.clone(), "My daily template".into())
        .unwrap();
    insert(
        &v,
        &template,
        None,
        None,
        block("template-content", BlockKind::Paragraph, "Keep this"),
    );
    v.set_default_template(journal.clone(), Some(template.clone()))
        .unwrap();
    dated(&v, &template, DAY + DAY_MS, false);
    let before_note = v.load_page(existing.clone()).unwrap();
    let before_template = v.load_page(template.clone()).unwrap();
    let before_journal = v.load_page(journal.clone()).unwrap();

    assert_eq!(
        v.daily_note(DAY, "New title".into(), true).unwrap(),
        Some(existing.clone())
    );
    let next = v
        .daily_note(DAY + DAY_MS, "Tomorrow".into(), true)
        .unwrap()
        .unwrap();
    assert_ne!(next, template);
    assert_fixed_sections(&v.load_page(next).unwrap().blocks);
    assert_eq!(v.load_page(existing).unwrap(), before_note);
    assert_eq!(v.load_page(template.clone()).unwrap(), before_template);
    assert_eq!(v.load_page(journal.clone()).unwrap(), before_journal);
    assert_eq!(v.default_template(journal.clone()).unwrap(), Some(template));
    assert_eq!(v.list_templates(journal).unwrap().len(), 1);
}

#[test]
fn append_log_uses_the_shared_clock_and_keeps_the_existing_log_in_place() {
    // The clock hook is process-wide: isolate it from parallel test vaults.
    const CHILD: &str = "NOTES_DAY_LOG_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "append_log_uses_the_shared_clock_and_keeps_the_existing_log_in_place",
            ])
            .env(CHILD, "1")
            .env("TZ", "Europe/Berlin")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let clock_ms = DAY + 12 * 3_600_000 + 34 * 60_000;
    Vault::set_test_clock(Some(clock_ms));
    let expected_time = Local
        .timestamp_millis_opt(clock_ms)
        .single()
        .unwrap()
        .format("%H:%M")
        .to_string();
    // Unix honours TZ; Windows uses the configured device zone instead.
    #[cfg(unix)]
    assert_eq!(expected_time, "14:34");
    let t = TempVault::new();
    let v = t.open();
    // A first log creates both the note and its sole root Log toggle.
    let created = v.append_log(DAY, "First".into()).unwrap();
    let first = v.load_page(created.clone()).unwrap();
    assert_eq!(first.title, "Tuesday 6 October 2026");
    // The first log has already added an entry to Log, so only the order is
    // fixed here; the empty new-note shape is checked by the other tests.
    assert_sections(&first.blocks);
    let tasks = &first.blocks[0];
    let log = &first.blocks[1];
    assert_eq!(log.children[0].text, format!("{expected_time}  First"));
    insert(
        &v,
        &created,
        None,
        Some(&log.id),
        block("after-log", BlockKind::Paragraph, "My later content"),
    );
    assert_eq!(v.append_log(DAY, "Second".into()).unwrap(), created);
    assert_eq!(v.append_log(DAY, "Third".into()).unwrap(), created);
    let page = v.load_page(created.clone()).unwrap();
    assert_eq!(
        page.blocks
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        vec![tasks.id.as_str(), log.id.as_str(), "after-log"]
    );
    assert_eq!(page.blocks[2].text, "My later content");
    assert!(page.blocks[0].children.is_empty());
    assert_eq!(
        page.blocks[1]
            .children
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>(),
        vec![
            format!("{expected_time}  First"),
            format!("{expected_time}  Second"),
            format!("{expected_time}  Third"),
        ]
    );
    assert!(page.blocks[1]
        .children
        .iter()
        .all(|b| b.kind == BlockKind::Paragraph));
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(v.list_children(Some(journal), 0, 100).unwrap().len(), 1);

    // Existing content (including a nested Log) stays in place, ahead of the
    // note's own Tasks and Log sections. Reusing the note keeps its title.
    let next = v
        .daily_note(DAY + DAY_MS, "Custom title".into(), true)
        .unwrap()
        .unwrap();
    insert(
        &v,
        &next,
        None,
        None,
        block("user-root", BlockKind::Toggle, "Work"),
    );
    insert(
        &v,
        &next,
        Some("user-root"),
        None,
        block("nested-log", BlockKind::Toggle, "Log"),
    );
    assert_eq!(v.append_log(DAY + DAY_MS, "Next day".into()).unwrap(), next);
    let page = v.load_page(next).unwrap();
    assert_eq!(page.title, "Custom title");
    assert_eq!(page.blocks.len(), 3);
    assert_eq!(page.blocks[0].id, "user-root");
    assert_eq!(page.blocks[0].children[0].id, "nested-log");
    assert_eq!(page.blocks[1].text, "Tasks");
    assert!(page.blocks[1].children.is_empty());
    assert_eq!(page.blocks[2].text, "Log");
    assert_eq!(
        page.blocks[2].children[0].text,
        format!("{expected_time}  Next day")
    );
    v.flush().unwrap();
    drop(v);
    Vault::set_test_clock(None);
}

#[test]
fn independent_vault_handles_create_one_daily_note() {
    // Cover both creating the Journal and adding a day to an existing one.
    for existing_journal in [false, true] {
        let t = TempVault::new();
        let a = t.open();
        let b = t.open();
        if existing_journal {
            a.daily_note(DAY - DAY_MS, "Previous day".into(), true)
                .unwrap();
        }
        let barrier = std::sync::Barrier::new(2);
        let notes = std::thread::scope(|scope| {
            let handles: Vec<_> = [&a, &b]
                .into_iter()
                .map(|v| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        v.daily_note(DAY, "Today".into(), true).unwrap().unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(notes[0], notes[1]);
        // Foreground metadata commits must be visible without a flush/reindex.
        for v in [&a, &b] {
            assert_eq!(v.list_children(None, 0, 100).unwrap().len(), 1);
            let journal = v.find_by_role("journal".into()).unwrap().unwrap();
            let rows = v.list_children(Some(journal), 0, 100).unwrap();
            assert_eq!(rows.len(), if existing_journal { 2 } else { 1 });
            assert_eq!(rows.iter().filter(|r| r.id == notes[0]).count(), 1);
            assert_eq!(
                v.daily_note(DAY, String::new(), false).unwrap(),
                Some(notes[0].clone())
            );
            let page = v.load_page(notes[0].clone()).unwrap();
            assert_sections(&page.blocks);
            assert!(page.blocks.iter().all(|section| section.children.is_empty()));
        }
    }
}

#[test]
fn concurrent_logs_create_one_note_and_one_log_without_losing_lines() {
    let t = TempVault::new();
    let v = t.open();
    let barrier = std::sync::Barrier::new(6);
    let pages = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let barrier = &barrier;
                let v = &v;
                scope.spawn(move || {
                    barrier.wait();
                    v.append_log(DAY, format!("Entry {i}")).unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(pages.iter().all(|p| p == &pages[0]));
    let page = v.load_page(pages[0].clone()).unwrap();
    assert_eq!(page.blocks.len(), 2);
    assert_eq!(page.blocks[0].text, "Tasks");
    assert!(page.blocks[0].children.is_empty());
    let log = &page.blocks[1];
    assert_eq!((log.kind.clone(), log.text.as_str()), (BlockKind::Toggle, "Log"));
    assert_eq!(log.children.len(), 6);
    for i in 0..6 {
        assert_eq!(
            log.children
                .iter()
                .filter(|b| b.text.ends_with(&format!("  Entry {i}")))
                .count(),
            1
        );
    }
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(v.list_children(Some(journal), 0, 100).unwrap().len(), 1);
}

#[test]
fn note_days_are_sorted_distinct_half_open_and_exclude_timed_trash_and_templates() {
    let t = TempVault::new();
    let v = t.open();
    assert!(v
        .day_notes_in_range(DAY, DAY + 10 * DAY_MS)
        .unwrap()
        .is_empty());
    assert_eq!(
        v.page_count().unwrap(),
        0,
        "range reads never create Journal"
    );
    v.daily_note(DAY + DAY_MS, "Second day".into(), true)
        .unwrap();
    v.daily_note(DAY, "First day".into(), true).unwrap();
    v.daily_note(DAY + 2 * DAY_MS, "Upper edge".into(), true)
        .unwrap();
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    v.rename_page(journal.clone(), "Renamed diary".into())
        .unwrap();
    let duplicate = v
        .create_page_from(journal.clone(), "Duplicate date".into(), None)
        .unwrap();
    dated(&v, &duplicate, DAY, false);
    let timed = v
        .create_page_from(journal.clone(), "Timed row".into(), None)
        .unwrap();
    dated(&v, &timed, DAY + 3 * DAY_MS, true);
    let trashed = v
        .daily_note(DAY + 4 * DAY_MS, "Trash".into(), true)
        .unwrap()
        .unwrap();
    v.delete_page(trashed).unwrap();
    let template = v
        .create_template(journal.clone(), "Template".into())
        .unwrap();
    dated(&v, &template, DAY + 5 * DAY_MS, false);
    v.set_default_template(journal, Some(template)).unwrap();
    let unrelated = v.create_page(None, "2026-10-12".into()).unwrap();
    dated(&v, &unrelated, DAY + 6 * DAY_MS, false);

    assert_eq!(
        v.day_notes_in_range(DAY, DAY + 2 * DAY_MS).unwrap(),
        vec![DAY, DAY + DAY_MS]
    );
    assert_eq!(
        v.day_notes_in_range(DAY, DAY + 10 * DAY_MS).unwrap(),
        vec![DAY, DAY + DAY_MS, DAY + 2 * DAY_MS]
    );
    assert_eq!(
        v.day_notes_in_range(DAY + DAY_MS, DAY + 2 * DAY_MS)
            .unwrap(),
        vec![DAY + DAY_MS]
    );
    assert!(v.day_notes_in_range(DAY, DAY).unwrap().is_empty());
    assert!(v.day_notes_in_range(DAY + DAY_MS, DAY).unwrap().is_empty());
    assert!(v
        .daily_note(DAY + 3 * DAY_MS, String::new(), false)
        .unwrap()
        .is_none());
    assert!(v
        .daily_note(DAY + 4 * DAY_MS, String::new(), false)
        .unwrap()
        .is_none());
    assert!(v
        .daily_note(DAY + 5 * DAY_MS, String::new(), false)
        .unwrap()
        .is_none());
}

#[test]
fn journal_page_dates_stay_out_of_calendar_but_their_dated_blocks_remain() {
    let t = TempVault::new();
    let v = t.open();
    let note = v.daily_note(DAY, "Day note".into(), true).unwrap().unwrap();
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    v.set_agenda_settings(
        journal.clone(),
        AgendaSettings {
            role: AgendaRole::Events,
            date_prop: None,
        },
    )
    .unwrap();
    // The same Journal note may also be a task note. Both page-level date
    // sources stay hidden; its dated blocks still belong in the calendar.
    v.type_data_set(
        note.clone(),
        "tasks".into(),
        vec!["date".into()],
        "\"2026-10-06\"".into(),
    )
    .unwrap();
    for (id, kind) in [("todo", BlockKind::Todo), ("mention", BlockKind::Paragraph)] {
        let mut b = block(id, kind, "Call @2026-10-06");
        b.marks.push(Mark {
            start: 5,
            end: 16,
            kind: MarkKind::Date("2026-10-06".into()),
        });
        insert(&v, &note, None, None, b);
    }
    let other = v.create_page(None, "Other dates".into()).unwrap();
    let schema = v.get_schema(journal).unwrap();
    v.set_schema(other.clone(), schema).unwrap();
    let ordinary = v.create_page(Some(other), "Ordinary event".into()).unwrap();
    dated(&v, &ordinary, DAY, false);
    v.flush().unwrap();
    let agenda = v.agenda(DAY, DAY + DAY_MS, DAY, DAY + DAY_MS).unwrap();
    assert_eq!(
        agenda.iter().filter(|i| i.page == note && i.block.is_none()).count(),
        2,
        "both database and task-note date sources were indexed"
    );
    let items = v
        .calendar_items(DAY, DAY + DAY_MS, DAY, DAY + DAY_MS)
        .unwrap();
    assert!(!items.iter().any(|i| i.page == note && i.block.is_none()));
    assert!(items
        .iter()
        .any(|i| i.page == note && i.block.as_deref() == Some("todo")));
    assert!(items
        .iter()
        .any(|i| i.page == note && i.block.as_deref() == Some("mention")));
    assert!(items
        .iter()
        .any(|i| i.page == ordinary && i.block.is_none()));
}

#[test]
fn new_day_notes_have_empty_tasks_then_log_and_ignore_the_journal_template() {
    let t = TempVault::new();
    let v = t.open();
    let first = v.daily_note(DAY, "First".into(), true).unwrap().unwrap();
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    let template = v
        .create_template(journal.clone(), "Template".into())
        .unwrap();
    insert(
        &v,
        &template,
        None,
        None,
        block("template-body", BlockKind::Paragraph, "Not copied"),
    );
    v.set_default_template(journal, Some(template)).unwrap();

    let second = v
        .daily_note(DAY + DAY_MS, "Second".into(), true)
        .unwrap()
        .unwrap();
    assert_fixed_sections(&v.load_page(second).unwrap().blocks);
    // The note made before the template keeps the same shape, unchanged.
    assert_fixed_sections(&v.load_page(first).unwrap().blocks);
}

#[test]
fn link_task_adds_one_link_to_tasks_and_reports_whether_it_added() {
    let t = TempVault::new();
    let v = t.open();
    let task1 = dated_task(&v, "Write report", "2026-10-06");
    let task2 = dated_task(&v, "Call dentist", "2026-10-06");

    assert!(v.link_task_to_day(DAY, task1.clone()).unwrap());
    let note = v.daily_note(DAY, String::new(), false).unwrap().unwrap();
    let before = v.load_page(note.clone()).unwrap();
    assert!(!v.link_task_to_day(DAY, task1.clone()).unwrap());
    assert_eq!(v.load_page(note.clone()).unwrap(), before, "no-op changes nothing");
    assert!(v.link_task_to_day(DAY, task2.clone()).unwrap());

    let page = v.load_page(note).unwrap();
    assert_eq!(page.blocks.len(), 2);
    assert_eq!(page.blocks[0].text, "Tasks");
    let links: Vec<_> = page.blocks[0]
        .children
        .iter()
        .map(|b| (b.kind.clone(), b.text.clone(), b.target.clone()))
        .collect();
    assert_eq!(
        links,
        vec![
            (BlockKind::LinkToPage, String::new(), Some(task1)),
            (BlockKind::LinkToPage, String::new(), Some(task2)),
        ]
    );
    assert!(page.blocks[1].children.is_empty());
}

#[test]
fn links_outside_the_tasks_section_do_not_count_but_nested_ones_inside_do() {
    let t = TempVault::new();
    let v = t.open();
    let note = v.daily_note(DAY, "Day".into(), true).unwrap().unwrap();
    let page = v.load_page(note.clone()).unwrap();
    let (tasks_id, log_id) = (page.blocks[0].id.clone(), page.blocks[1].id.clone());

    // A link under Log is outside the section, so the task is still added.
    let elsewhere = dated_task(&v, "Elsewhere", "2026-10-06");
    link_block(&v, &note, Some(&log_id), None, "stray", &elsewhere);
    assert!(v.link_task_to_day(DAY, elsewhere.clone()).unwrap());

    // A link nested deeper inside Tasks does count as already linked.
    let nested = dated_task(&v, "Nested", "2026-10-06");
    insert(
        &v,
        &note,
        Some(&tasks_id),
        None,
        block("sub", BlockKind::Toggle, "Sub"),
    );
    link_block(&v, &note, Some("sub"), None, "nested-link", &nested);
    assert!(!v.link_task_to_day(DAY, nested.clone()).unwrap());

    let page = v.load_page(note).unwrap();
    assert_eq!(page.blocks[0].children.len(), 2);
    assert_eq!(page.blocks[0].children[0].id, "sub");
    assert_eq!(
        page.blocks[0].children[1].target.as_deref(),
        Some(elsewhere.as_str())
    );
    assert_eq!(page.blocks[1].children.len(), 1);
    assert_eq!(page.blocks[1].children[0].id, "stray");
}

#[test]
fn older_notes_without_tasks_get_the_section_above_log_and_keep_their_content() {
    let t = TempVault::new();
    let v = t.open();
    let note = v.daily_note(DAY, "Old note".into(), true).unwrap().unwrap();
    // Make the note the older shape: no Tasks, a user block, and a Log entry.
    let before = v.load_page(note.clone()).unwrap().blocks;
    let (tasks_id, log_id) = (before[0].id.clone(), before[1].id.clone());
    v.apply(note.clone(), vec![BlockOp::Delete { id: tasks_id }])
        .unwrap();
    insert(&v, &note, None, None, block("work", BlockKind::Toggle, "Work"));
    insert(
        &v,
        &note,
        Some(&log_id),
        None,
        block("old-entry", BlockKind::Paragraph, "07:00  Old entry"),
    );

    let task = dated_task(&v, "Old task", "2026-10-06");
    assert!(v.link_task_to_day(DAY, task.clone()).unwrap());

    let page = v.load_page(note).unwrap();
    assert_eq!(
        page.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(),
        vec!["Work", "Tasks", "Log"]
    );
    assert!(page.blocks[0].children.is_empty());
    assert_eq!(
        page.blocks[1].children[0].target.as_deref(),
        Some(task.as_str())
    );
    assert_eq!(page.blocks[1].children.len(), 1);
    assert_eq!(page.blocks[2].children.len(), 1);
    assert_eq!(page.blocks[2].children[0].text, "07:00  Old entry");
}

#[test]
fn older_notes_without_log_get_tasks_at_the_end_and_log_still_follows_it() {
    let t = TempVault::new();
    let v = t.open();
    let note = v.daily_note(DAY, "Old note".into(), true).unwrap().unwrap();
    let ids: Vec<String> = v
        .load_page(note.clone())
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.id.clone())
        .collect();
    v.apply(
        note.clone(),
        ids.into_iter().map(|id| BlockOp::Delete { id }).collect(),
    )
    .unwrap();
    assert!(v.load_page(note.clone()).unwrap().blocks.is_empty());

    let task = dated_task(&v, "Old task", "2026-10-06");
    assert!(v.link_task_to_day(DAY, task).unwrap());
    let texts = |v: &Vault| {
        v.load_page(note.clone())
            .unwrap()
            .blocks
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(texts(&v), vec!["Tasks"]);

    v.append_log(DAY, "Entry".into()).unwrap();
    assert_eq!(texts(&v), vec!["Tasks", "Log"]);
    let page = v.load_page(note).unwrap();
    assert_eq!(page.blocks[1].children.len(), 1);
}

#[test]
fn links_and_logs_share_one_day_note_with_tasks_before_log() {
    let t = TempVault::new();
    let v = t.open();
    let task1 = dated_task(&v, "One", "2026-10-06");
    let task2 = dated_task(&v, "Two", "2026-10-06");
    assert!(v.link_task_to_day(DAY, task1.clone()).unwrap());
    let note = v.append_log(DAY, "Between".into()).unwrap();
    assert!(v.link_task_to_day(DAY, task2.clone()).unwrap());
    assert_eq!(v.append_log(DAY, "After".into()).unwrap(), note);
    assert_eq!(
        v.daily_note(DAY, String::new(), false).unwrap(),
        Some(note.clone())
    );
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(v.list_children(Some(journal), 0, 100).unwrap().len(), 1);

    let page = v.load_page(note).unwrap();
    assert_eq!(
        page.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(),
        vec!["Tasks", "Log"]
    );
    assert_eq!(
        page.blocks[0]
            .children
            .iter()
            .map(|b| b.target.clone())
            .collect::<Vec<_>>(),
        vec![Some(task1), Some(task2)]
    );
    assert_eq!(page.blocks[1].children.len(), 2);
}

#[test]
fn invalid_tasks_are_rejected_before_any_day_note_is_created() {
    let t = TempVault::new();
    let v = t.open();
    // Only missing, trashed and tasks-data-less pages are rejected.
    let plain = v.create_page(None, "Plain page".into()).unwrap();
    let trashed = dated_task(&v, "Trashed", "2026-10-06");
    v.delete_page(trashed.clone()).unwrap();

    for id in [plain, trashed, "missing-page".to_string()] {
        assert!(v.link_task_to_day(DAY, id).is_err());
    }
    assert!(v.find_by_role("journal".into()).unwrap().is_none());
    assert!(v.daily_note(DAY, String::new(), false).unwrap().is_none());
}

#[test]
fn concurrent_links_add_each_task_once_and_share_the_section_with_logs() {
    let t = TempVault::new();
    let v = t.open();
    let tasks: Vec<PageId> = (0..4)
        .map(|i| dated_task(&v, &format!("Task {i}"), "2026-10-06"))
        .collect();
    let barrier = std::sync::Barrier::new(10);
    let mut added = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let barrier = &barrier;
                let v = &v;
                let tasks = &tasks;
                scope.spawn(move || {
                    barrier.wait();
                    if i < 8 {
                        // Eight threads race to link four tasks, two each.
                        v.link_task_to_day(DAY, tasks[i % 4].clone())
                            .unwrap()
                            .then_some(i % 4)
                    } else {
                        v.append_log(DAY, format!("Entry {i}")).unwrap();
                        None
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    added.sort();
    assert_eq!(added, vec![0, 1, 2, 3], "exactly one racer adds each task");

    let note = v.daily_note(DAY, String::new(), false).unwrap().unwrap();
    let page = v.load_page(note).unwrap();
    assert_eq!(page.blocks.len(), 2);
    assert_eq!(page.blocks[0].text, "Tasks");
    let targets: Vec<String> = page.blocks[0]
        .children
        .iter()
        .map(|b| b.target.clone().unwrap())
        .collect();
    assert_eq!(targets.len(), 4);
    for task in &tasks {
        assert_eq!(targets.iter().filter(|t| *t == task).count(), 1);
    }
    assert_eq!(page.blocks[1].text, "Log");
    assert_eq!(page.blocks[1].children.len(), 2);
    let journal = v.find_by_role("journal".into()).unwrap().unwrap();
    assert_eq!(v.list_children(Some(journal), 0, 100).unwrap().len(), 1);
}

#[test]
fn link_accepts_undated_tasks_that_have_only_a_status() {
    let t = TempVault::new();
    let v = t.open();
    let task = status_task(&v, "Someday");
    assert!(v.link_task_to_day(DAY, task.clone()).unwrap());
    assert!(!v.link_task_to_day(DAY, task.clone()).unwrap());

    let note = v.daily_note(DAY, String::new(), false).unwrap().unwrap();
    let page = v.load_page(note).unwrap();
    assert_eq!(page.blocks[0].children.len(), 1);
    assert_eq!(
        page.blocks[0].children[0].target.as_deref(),
        Some(task.as_str())
    );
}

#[test]
fn page_link_marks_inside_tasks_count_as_already_linked() {
    let t = TempVault::new();
    let v = t.open();
    let task = dated_task(&v, "Marked", "2026-10-06");
    let note = v.daily_note(DAY, "Day".into(), true).unwrap().unwrap();
    let tasks_id = v.load_page(note.clone()).unwrap().blocks[0].id.clone();
    // A paragraph in Tasks whose text links to the task with a PageLink mark.
    let mut mention = block("mention", BlockKind::Paragraph, "See task");
    mention.marks.push(Mark {
        start: 4,
        end: 8,
        kind: MarkKind::PageLink(task.clone()),
    });
    insert(&v, &note, Some(&tasks_id), None, mention);

    assert!(!v.link_task_to_day(DAY, task.clone()).unwrap());
    let page = v.load_page(note).unwrap();
    assert_eq!(page.blocks[0].children.len(), 1);
    assert_eq!(page.blocks[0].children[0].id, "mention");
    assert!(page.blocks[0].children[0].target.is_none());
}
