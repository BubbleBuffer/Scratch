use std::{fs, path::PathBuf};
use vault_core::*;

const DAY: i64 = 1_791_244_800_000; // 2026-10-06 UTC.
const DAY_MS: i64 = 86_400_000;

struct TempVault(PathBuf);
impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_move_date_{}", new_id())))
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

fn block(id: &str, kind: BlockKind, text: &str, marks: Vec<Mark>) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind,
        text: text.into(),
        marks,
        checked: true,
    }
}

fn mark(text: &str, part: &str, kind: MarkKind) -> Mark {
    let start = text[..text.find(part).unwrap()].chars().count() as u32;
    Mark {
        start,
        end: start + part.chars().count() as u32,
        kind,
    }
}

fn insert(v: &Vault, page: &str, parent: Option<&str>, after: Option<&str>, block: NewBlock) {
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: parent.map(Into::into),
            after: after.map(Into::into),
            block,
        }],
    )
    .unwrap();
}

fn fixture(v: &Vault, iso: &str) -> PageId {
    let page = v.create_page(None, "Dates".into()).unwrap();
    insert(
        v,
        &page,
        None,
        None,
        block("before", BlockKind::Paragraph, "before", vec![]),
    );
    insert(
        v,
        &page,
        None,
        Some("before"),
        block("parent", BlockKind::Toggle, "parent", vec![]),
    );
    insert(
        v,
        &page,
        None,
        Some("parent"),
        block("after", BlockKind::Paragraph, "after", vec![]),
    );
    let text = "🐈 café @bad first @today then @later";
    let marks = vec![
        mark(text, "@bad", MarkKind::Date("invalid".into())),
        mark(text, "@today", MarkKind::Date(iso.into())),
        mark(text, "@later", MarkKind::Date("2026-10-09".into())),
        mark(text, "café @bad first @today", MarkKind::Bold),
        mark(
            text,
            "first @today then",
            MarkKind::Link("https://example.com".into()),
        ),
    ];
    insert(
        v,
        &page,
        Some("parent"),
        None,
        block("task", BlockKind::Todo, text, marks),
    );
    insert(
        v,
        &page,
        Some("task"),
        None,
        block("child", BlockKind::Paragraph, "child", vec![]),
    );
    v.flush().unwrap();
    page
}

fn queries(v: &Vault) -> (Vec<AgendaItem>, Vec<AgendaItem>) {
    let from = DAY - 30 * DAY_MS;
    let to = DAY + 30 * DAY_MS;
    (
        v.agenda(from, to, from, to).unwrap(),
        v.calendar_items(from, to, from, to).unwrap(),
    )
}

fn reference(v: &Vault, page: &str) -> DateMarkRef {
    let (agenda, calendar) = queries(v);
    let a = agenda
        .iter()
        .find(|i| i.page == page && i.block.as_deref() == Some("task"))
        .unwrap();
    let c = calendar
        .iter()
        .find(|i| i.page == page && i.block.as_deref() == Some("task"))
        .unwrap();
    assert_eq!(a.date_mark, c.date_mark);
    a.date_mark.clone().unwrap()
}

fn task(v: &Vault, page: &str) -> Block {
    v.load_page(page.into()).unwrap().blocks[1].children[0].clone()
}

#[test]
fn moves_selected_date_only_and_survives_reindex_and_reopen() {
    let t = TempVault::new();
    let v = t.open();
    let page = fixture(&v, "2026-10-06T12:34:56.789Z");
    let old_ref = reference(&v, &page);
    assert_eq!(old_ref.start, 18); // Unicode scalars, including the cat emoji.
    let before = v.load_page(page.clone()).unwrap();
    let versions = v.page_versions().unwrap();
    let seq = v.poll_changes(0).seq;
    v.move_date_mark(page.clone(), "task".into(), old_ref.clone(), DAY + DAY_MS)
        .unwrap();
    assert_ne!(v.page_versions().unwrap(), versions);
    assert_eq!(
        v.poll_changes(seq)
            .changes
            .into_iter()
            .filter(|change| !matches!(change, Change::IndexCommitted { .. }))
            .collect::<Vec<_>>(),
        vec![Change::PageChanged(page.clone())]
    );
    let after = v.load_page(page.clone()).unwrap();
    let mut expected = before.blocks;
    let selected = expected[1].children[0]
        .marks
        .iter_mut()
        .find(|m| m.kind == MarkKind::Date(old_ref.iso.clone()))
        .unwrap();
    selected.kind = MarkKind::Date("2026-10-07T12:34:56.789Z".into());
    assert_eq!(after.blocks, expected); // Includes order, children, text and every range.
    v.flush().unwrap();
    assert!(v
        .poll_changes(seq)
        .changes
        .into_iter()
        .any(|change| match change {
            Change::IndexCommitted { generation, pages } => generation > 0 && pages.contains(&page),
            _ => false,
        }));
    for items in [queries(&v).0, queries(&v).1] {
        let item = items
            .iter()
            .find(|i| i.block.as_deref() == Some("task"))
            .unwrap();
        assert_eq!(item.start_ms, DAY + DAY_MS + 45_296_789);
        assert_eq!(
            item.date_mark.as_ref().unwrap().iso,
            "2026-10-07T12:34:56.789Z"
        );
    }
    v.reindex().unwrap();
    assert_eq!(reference(&v, &page).iso, "2026-10-07T12:34:56.789Z");
    drop(v);
    let v = t.open();
    assert_eq!(v.load_page(page.clone()).unwrap().blocks, expected);
    assert_eq!(reference(&v, &page).iso, "2026-10-07T12:34:56.789Z");
}

#[test]
fn preserves_exact_suffix_and_same_day_is_a_noop() {
    for suffix in ["", "T12:34", "T12:34Z", "T12:34:56", "T12:34:56.007Z"] {
        let t = TempVault::new();
        let v = t.open();
        let page = fixture(&v, &format!("2026-10-06{suffix}"));
        let r = reference(&v, &page);
        let before = v.load_page(page.clone()).unwrap();
        let versions = v.page_versions().unwrap();
        let seq = v.poll_changes(0).seq;
        v.move_date_mark(page.clone(), "task".into(), r.clone(), DAY)
            .unwrap();
        assert_eq!(v.load_page(page.clone()).unwrap(), before);
        assert_eq!(v.page_versions().unwrap(), versions);
        assert!(v.poll_changes(seq).changes.is_empty());
        v.move_date_mark(page.clone(), "task".into(), r, DAY - DAY_MS)
            .unwrap();
        v.flush().unwrap();
        assert_eq!(reference(&v, &page).iso, format!("2026-10-05{suffix}"));
    }
}

#[test]
fn stale_or_invalid_references_do_not_mutate_document() {
    for change in [
        "gone",
        "mark_gone",
        "replaced",
        "same_day_iso",
        "range",
        "selected",
        "kind",
        "moved",
    ] {
        let t = TempVault::new();
        let v = t.open();
        let page = fixture(&v, "2026-10-06");
        let r = reference(&v, &page);
        let mut b = task(&v, &page);
        match change {
            "gone" => v
                .apply(page.clone(), vec![BlockOp::Delete { id: "task".into() }])
                .unwrap(),
            "kind" => v
                .apply(
                    page.clone(),
                    vec![BlockOp::SetKind {
                        id: "task".into(),
                        kind: BlockKind::Paragraph,
                    }],
                )
                .unwrap(),
            "moved" => v
                .move_date_mark(page.clone(), "task".into(), r.clone(), DAY + DAY_MS)
                .unwrap(),
            _ => {
                if change == "mark_gone" {
                    b.marks.retain(|m| m.kind != MarkKind::Date(r.iso.clone()));
                }
                for m in &mut b.marks {
                    if m.kind == MarkKind::Date("2026-10-06".into()) {
                        if change == "replaced" {
                            m.kind = MarkKind::Date("2026-10-08".into());
                        }
                        if change == "same_day_iso" {
                            m.kind = MarkKind::Date("2026-10-06T00:00Z".into());
                        }
                        if change == "range" {
                            m.end -= 1;
                        }
                    }
                    if change == "selected" && m.kind == MarkKind::Date("invalid".into()) {
                        m.kind = MarkKind::Date("2026-10-05".into());
                    }
                }
                v.apply(
                    page.clone(),
                    vec![BlockOp::SetText {
                        id: b.id,
                        text: b.text,
                        marks: b.marks,
                    }],
                )
                .unwrap();
            }
        }
        let before = v.load_page(page.clone()).unwrap();
        let versions = v.page_versions().unwrap();
        let seq = v.poll_changes(0).seq;
        let error = v
            .move_date_mark(page.clone(), "task".into(), r, DAY + 2 * DAY_MS)
            .unwrap_err();
        assert!(
            error.to_string().contains("stale date mark reference"),
            "{change}: {error}"
        );
        assert_eq!(v.load_page(page).unwrap(), before, "{change}");
        assert_eq!(v.page_versions().unwrap(), versions, "{change}");
        assert!(v.poll_changes(seq).changes.is_empty(), "{change}");
    }
}

#[test]
fn rejects_invalid_days_and_cannot_change_task_note_dates() {
    let t = TempVault::new();
    let v = t.open();
    let page = fixture(&v, "2026-10-06");
    let r = reference(&v, &page);
    let before = v.load_page(page.clone()).unwrap();
    for day in [DAY + 1, DAY + 3_600_000, i64::MAX, i64::MIN] {
        assert!(v
            .move_date_mark(page.clone(), "task".into(), r.clone(), day)
            .is_err());
        assert_eq!(v.load_page(page.clone()).unwrap(), before);
    }
    let note = v.create_page(None, "Task note".into()).unwrap();
    v.type_data_set(
        note.clone(),
        "tasks".into(),
        vec!["date".into()],
        "\"2026-10-06\"".into(),
    )
    .unwrap();
    v.flush().unwrap();
    let before = v.load_page(note.clone()).unwrap();
    let items = queries(&v).1;
    assert!(items
        .iter()
        .find(|i| i.page == note && i.block.is_none())
        .unwrap()
        .date_mark
        .is_none());
    assert!(v
        .move_date_mark(note.clone(), "task".into(), r, DAY + DAY_MS)
        .is_err());
    assert_eq!(v.load_page(note).unwrap(), before);
}

#[test]
fn reference_to_later_mark_with_identical_iso_is_rejected() {
    let t = TempVault::new();
    let v = t.open();
    let page = fixture(&v, "2026-10-06");
    let mut b = task(&v, &page);
    let later = b
        .marks
        .iter_mut()
        .find(|m| m.kind == MarkKind::Date("2026-10-09".into()))
        .unwrap();
    later.kind = MarkKind::Date("2026-10-06".into());
    let forged = DateMarkRef {
        start: later.start,
        end: later.end,
        iso: "2026-10-06".into(),
    };
    v.apply(
        page.clone(),
        vec![BlockOp::SetText {
            id: b.id,
            text: b.text,
            marks: b.marks,
        }],
    )
    .unwrap();
    v.flush().unwrap();
    assert_ne!(reference(&v, &page), forged);
    let before = v.load_page(page.clone()).unwrap();
    let versions = v.page_versions().unwrap();
    let seq = v.poll_changes(0).seq;
    assert!(v
        .move_date_mark(page.clone(), "task".into(), forged, DAY + DAY_MS)
        .is_err());
    assert_eq!(v.load_page(page).unwrap(), before);
    assert_eq!(v.page_versions().unwrap(), versions);
    assert!(v.poll_changes(seq).changes.is_empty());
}

#[test]
fn mismatched_index_date_withholds_transient_reference() {
    let t = TempVault::new();
    let v = t.open();
    let page = fixture(&v, "2026-10-06");
    // Model the interval between a document edit and its background reindex
    // deterministically, without racing the writer thread.
    let conn = rusqlite::Connection::open(t.0.join(".vault/index.sqlite")).unwrap();
    conn.execute(
        "UPDATE agenda SET d0 = ?1 WHERE block = 'task'",
        [DAY + DAY_MS],
    )
    .unwrap();
    for items in [queries(&v).0, queries(&v).1] {
        let item = items
            .iter()
            .find(|i| i.block.as_deref() == Some("task"))
            .unwrap();
        assert_eq!(item.start_ms, DAY + DAY_MS);
        assert!(item.date_mark.is_none());
    }
    drop(conn);
    v.reindex().unwrap();
    assert_eq!(reference(&v, &page).iso, "2026-10-06");
}
