use std::path::PathBuf;
use vault_core::*;

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn vault() -> (Dir, Vault) {
    let path = std::env::temp_dir().join(format!("conflict_resolution_{}", new_id()));
    let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
    (Dir(path), vault)
}
fn named_vault(device: &str) -> (Dir, Vault) {
    let path = std::env::temp_dir().join(format!("conflict_resolution_named_{}", new_id()));
    std::fs::create_dir_all(path.join(".vault")).unwrap();
    std::fs::write(path.join(".vault/device"), device).unwrap();
    let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
    (Dir(path), vault)
}
fn copy(from: &Vault, to: &Vault, page: &str) {
    to.import_updates(
        page.into(),
        from.export_updates(page.into(), vec![]).unwrap(),
    )
    .unwrap();
}
fn insert(id: &str, parent: Option<&str>, after: Option<&str>, text: &str) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(Into::into),
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
fn text(id: &str, text: &str) -> BlockOp {
    BlockOp::SetText {
        id: id.into(),
        text: text.into(),
        marks: vec![],
    }
}

#[test]
fn public_pick_restores_only_required_parent_chain_and_undo_removes_it() {
    let (_ad, a) = vault();
    let (_bd, b) = vault();
    let page = a.create_page(None, "Nested text".into()).unwrap();
    a.apply(
        page.clone(),
        vec![
            insert("outside", None, None, "Outside"),
            insert("grand", None, Some("outside"), "Grand"),
            insert("unneeded", Some("grand"), None, "Unneeded sibling"),
            insert("parent", Some("grand"), Some("unneeded"), "Parent"),
            insert("target", Some("parent"), None, "Original"),
        ],
    )
    .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![text("target", "Cloud choice")])
        .unwrap();
    b.apply(page.clone(), vec![text("target", "Local choice")])
        .unwrap();
    copy(&a, &b, &page);
    let conflict = b.list_conflicts(Some(page.clone())).unwrap().remove(0);
    let side = conflict.sides.iter().position(|side| {
        matches!(&side.state, SideState::Present { content } if content[0].text == "Cloud choice")
    }).unwrap() as u32;
    b.apply(
        page.clone(),
        vec![
            BlockOp::Delete { id: "grand".into() },
            text("outside", "Later unrelated edit"),
        ],
    )
    .unwrap();
    b.resolve_conflict(
        page.clone(),
        conflict.id.clone(),
        ConflictResolution::UseSide(side),
    )
    .unwrap();
    let blocks = b.load_page(page.clone()).unwrap().blocks;
    assert_eq!(blocks[0].text, "Later unrelated edit");
    assert_eq!(blocks[1].id, "grand");
    assert_eq!(blocks[1].children.len(), 1);
    assert_eq!(blocks[1].children[0].id, "parent");
    assert_eq!(blocks[1].children[0].children[0].id, "target");
    assert_eq!(blocks[1].children[0].children[0].text, "Cloud choice");
    let log = b.conflict_log(Some(page.clone())).unwrap();
    assert_eq!(
        log[0].applied.last().unwrap().restored_ancestors,
        ["grand", "parent"]
    );
    assert!(!log[0].applied.last().unwrap().fallback);
    b.undo_conflict(page.clone(), conflict.id.clone()).unwrap();
    let blocks = b.load_page(page.clone()).unwrap().blocks;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "Later unrelated edit");
    let pending = b.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(pending[0].id, conflict.id);
    assert_eq!(pending[0].status, ConflictStatus::Pending);
    assert!(pending[0].applied.last().unwrap().undone_at.is_some());
    assert!(!pending[0].undoable());
    assert_eq!(
        b.conflict_log(Some(page.clone())).unwrap()[0].id,
        conflict.id
    );
    assert!(b.undo_conflict(page.clone(), conflict.id.clone()).is_err());
    b.resolve_conflict(page.clone(), conflict.id, ConflictResolution::Keep)
        .unwrap();
    let log = b.conflict_log(Some(page)).unwrap();
    // The text rule applied at detection, the undone pick, then Keep.
    assert_eq!(log[0].applied.len(), 3);
    assert!(
        matches!(&log[0].applied[0].by, ResolutionBy::Auto { rule } if rule == "later-text-wins")
    );
    assert!(log[0].applied[0].undone_at.is_none());
    assert!(log[0].applied[1].undone_at.is_some());
    assert!(log[0].applied[2].undone_at.is_none());
}

/// The side the text rule shows: later `time_ms`, then the greater device id.
fn rule_text(conflict: &Conflict) -> String {
    let side = conflict
        .sides
        .iter()
        .max_by(|x, y| {
            x.time_ms
                .cmp(&y.time_ms)
                .then_with(|| x.device.as_bytes().cmp(y.device.as_bytes()))
        })
        .unwrap();
    match &side.state {
        SideState::Present { content } => content[0].text.clone(),
        _ => panic!("text sides are present"),
    }
}

#[test]
fn text_rule_converges_on_both_devices_and_undo_reopens_the_merged_text() {
    let (_ad, a) = vault();
    let (_bd, b) = vault();
    let page = a.create_page(None, "Both edited".into()).unwrap();
    a.apply(
        page.clone(),
        vec![insert("target", None, None, "the cat sat")],
    )
    .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![text("target", "the black cat sat")])
        .unwrap();
    b.apply(page.clone(), vec![text("target", "the cat sat down")])
        .unwrap();
    // Both devices detect the same event independently.
    let from_a = a.export_updates(page.clone(), vec![]).unwrap();
    let from_b = b.export_updates(page.clone(), vec![]).unwrap();
    a.import_updates(page.clone(), from_b).unwrap();
    b.import_updates(page.clone(), from_a).unwrap();
    let on_a = a.list_conflicts(Some(page.clone())).unwrap();
    let on_b = b.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(on_a.len(), 1);
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_a[0].id, on_b[0].id);
    let expected = rule_text(&on_a[0]);
    assert_eq!(expected, rule_text(&on_b[0]));
    // Each saved record orders its own sides, so the winning index is read from
    // the record being checked: the rule applied the side whose text is expected.
    let winner_of = |conflict: &Conflict| {
        conflict.sides.iter().position(|side|
        matches!(&side.state, SideState::Present { content } if content[0].text == expected)).unwrap() as u32
    };
    for vault in [&a, &b] {
        assert_eq!(
            vault.load_page(page.clone()).unwrap().blocks[0].text,
            expected
        );
    }
    for _ in 0..2 {
        copy(&a, &b, &page);
        copy(&b, &a, &page);
    }
    for vault in [&a, &b] {
        assert_eq!(
            vault.load_page(page.clone()).unwrap().blocks[0].text,
            expected
        );
        let pending = vault.list_conflicts(Some(page.clone())).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, ConflictStatus::Pending);
        assert_eq!(pending[0].applied.len(), 1);
        assert!(matches!(&pending[0].applied[0].by,
            ResolutionBy::Auto { rule } if rule == "later-text-wins"));
        assert_eq!(
            pending[0].applied[0].resolution,
            Some(ConflictResolution::UseSide(winner_of(&pending[0])))
        );
        assert!(pending[0].undoable());
    }
    // Undo restores the pre-rule merge; the entry stays pending and the
    // undone application shows in the log.
    let id = on_b[0].id.clone();
    b.undo_conflict(page.clone(), id.clone()).unwrap();
    assert_eq!(
        b.load_page(page.clone()).unwrap().blocks[0].text,
        "the black cat sat down"
    );
    let pending = b.list_conflicts(Some(page.clone())).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, id);
    assert!(pending[0].applied[0].undone_at.is_some());
    assert!(!pending[0].undoable());
    assert_eq!(b.conflict_log(Some(page.clone())).unwrap()[0].id, id);
    assert!(b.undo_conflict(page.clone(), id.clone()).is_err());
    copy(&b, &a, &page);
    assert_eq!(
        a.load_page(page.clone()).unwrap().blocks[0].text,
        "the black cat sat down"
    );
    assert_eq!(a.list_conflicts(Some(page.clone())).unwrap().len(), 1);
    // The user's tap still decides.
    b.resolve_conflict(page.clone(), id.clone(), ConflictResolution::Keep)
        .unwrap();
    assert!(b.list_conflicts(Some(page.clone())).unwrap().is_empty());
    let log = b.conflict_log(Some(page)).unwrap();
    assert!(log
        .iter()
        .any(|c| c.id == id
            && c.applied.last().unwrap().resolution == Some(ConflictResolution::Keep)));
}

#[test]
fn public_page_purge_pick_and_undo_sync_from_retained_archive() {
    let (_ad, a) = vault();
    let (_bd, b) = vault();
    let (_cd, c) = vault();
    let page = a.create_page(None, "Purge choice".into()).unwrap();
    a.apply(page.clone(), vec![insert("target", None, None, "Original")])
        .unwrap();
    copy(&a, &b, &page);
    copy(&a, &c, &page);
    b.delete_page(page.clone()).unwrap();
    b.purge_page(page.clone()).unwrap();
    a.apply(page.clone(), vec![text("target", "Offline edit")])
        .unwrap();
    copy(&a, &b, &page);
    let conflict = b.list_conflicts(Some(page.clone())).unwrap().remove(0);
    let side = conflict
        .sides
        .iter()
        .position(|side| matches!(side.state, SideState::Purged { .. }))
        .unwrap() as u32;
    let a_version = a
        .sync_page_versions()
        .unwrap()
        .into_iter()
        .find(|version| version.page == page)
        .unwrap()
        .version;
    b.resolve_conflict(
        page.clone(),
        conflict.id.clone(),
        ConflictResolution::UseSide(side),
    )
    .unwrap();
    assert_eq!(
        b.conflict_log(Some(page.clone())).unwrap()[0]
            .applied
            .last()
            .unwrap()
            .resolution,
        Some(ConflictResolution::UseSide(side))
    );
    assert!(matches!(
        b.load_page(page.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert!(!b.list_versions(page.clone()).unwrap().is_empty());
    assert!(b
        .sync_page_versions()
        .unwrap()
        .iter()
        .any(|version| version.page == page));
    assert_eq!(
        b.conflict_log(Some(page.clone())).unwrap()[0].id,
        conflict.id
    );
    a.import_updates(
        page.clone(),
        b.export_updates(page.clone(), a_version).unwrap(),
    )
    .unwrap();
    copy(&b, &c, &page);
    for vault in [&a, &b, &c] {
        assert!(matches!(
            vault.load_page(page.clone()),
            Err(VaultError::NotFound(_))
        ));
    }
    let a_version = a
        .sync_page_versions()
        .unwrap()
        .into_iter()
        .find(|version| version.page == page)
        .unwrap()
        .version;
    b.undo_conflict(page.clone(), conflict.id.clone()).unwrap();
    a.import_updates(
        page.clone(),
        b.export_updates(page.clone(), a_version).unwrap(),
    )
    .unwrap();
    copy(&b, &c, &page);
    for vault in [&a, &b, &c] {
        assert_eq!(
            vault.load_page(page.clone()).unwrap().blocks[0].text,
            "Offline edit"
        );
        let pending = vault.list_conflicts(Some(page.clone())).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, conflict.id);
        assert_eq!(pending[0].applied.len(), 2);
        assert!(pending[0].applied.last().unwrap().undone_at.is_some());
        assert_eq!(vault.conflict_log(Some(page.clone())).unwrap().len(), 1);
    }
    b.reindex().unwrap();
    assert_eq!(b.list_conflicts(Some(page)).unwrap().len(), 1);
}

#[test]
fn local_purge_choice_keeps_the_choosing_device_on_every_replica() {
    // A causal successor wins even when the previous purger's id sorts later.
    let (_ad, a) = named_vault("a-chooser");
    let (_bd, b) = named_vault("z-old-purger");
    let (_cd, c) = vault();
    let (_dd, d) = vault();
    let page = a.create_page(None, "Chosen purge".into()).unwrap();
    a.apply(page.clone(), vec![insert("target", None, None, "Original")])
        .unwrap();
    copy(&a, &b, &page);
    copy(&a, &c, &page);
    b.delete_page(page.clone()).unwrap();
    b.purge_page(page.clone()).unwrap();
    // D holds only B's older tombstone; A's later choice must replace its identity.
    d.import_purge_records(b.purge_records()).unwrap();
    assert_eq!(
        d.purge_records()[0].device.as_deref(),
        Some(b.device_id().as_str())
    );
    a.apply(page.clone(), vec![text("target", "Offline edit")])
        .unwrap();
    // B's purged page has no archive to export, so A receives the purge record itself.
    a.import_purge_records(b.purge_records()).unwrap();
    let conflict = a.list_conflicts(Some(page.clone())).unwrap().remove(0);
    let side = conflict
        .sides
        .iter()
        .position(|side| matches!(side.state, SideState::Purged { .. }))
        .unwrap() as u32;
    a.resolve_conflict(
        page.clone(),
        conflict.id.clone(),
        ConflictResolution::UseSide(side),
    )
    .unwrap();
    // A's choice is the newest purge, so it names A; B and C keep A's identity.
    let chooser = a.device_id();
    assert_eq!(
        a.purge_records()[0].device.as_deref(),
        Some(chooser.as_str())
    );
    copy(&a, &b, &page);
    copy(&a, &c, &page);
    copy(&a, &d, &page);
    for vault in [&a, &b, &c, &d] {
        let records = vault.purge_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].device.as_deref(), Some(chooser.as_str()));
    }
}

#[test]
fn keeping_a_purged_archive_refreshes_it_without_losing_the_purging_device() {
    let (_ad, a) = vault();
    let (_bd, b) = vault();
    let (_cd, c) = vault();
    let page = a.create_page(None, "Kept purge".into()).unwrap();
    a.apply(page.clone(), vec![insert("target", None, None, "Original")])
        .unwrap();
    copy(&a, &b, &page);
    copy(&a, &c, &page);
    b.delete_page(page.clone()).unwrap();
    b.purge_page(page.clone()).unwrap();
    let purger = b.device_id();
    a.apply(page.clone(), vec![text("target", "Offline edit")])
        .unwrap();
    copy(&a, &b, &page);
    let conflict = b.list_conflicts(Some(page.clone())).unwrap().remove(0);
    // Undo the automatic edit-beats-delete application: its before-state is
    // purged, and ordinary undo reopens this entry for review.
    b.undo_conflict(page.clone(), conflict.id.clone()).unwrap();
    assert!(matches!(
        b.load_page(page.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(
        b.list_conflicts(Some(page.clone())).unwrap()[0].status,
        ConflictStatus::Pending
    );
    // Keep leaves the page purged and refreshes the archive's coverage.
    b.resolve_conflict(page.clone(), conflict.id.clone(), ConflictResolution::Keep)
        .unwrap();
    assert!(matches!(
        b.load_page(page.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(
        b.purge_records()[0].device.as_deref(),
        Some(purger.as_str())
    );
    copy(&b, &a, &page);
    copy(&b, &c, &page);
    for vault in [&a, &b, &c] {
        let records = vault.purge_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].device.as_deref(), Some(purger.as_str()));
    }
}

#[test]
fn pure_choice_sync_preserves_a_concurrent_unrelated_edit_without_another_conflict() {
    let (_ad, a) = vault();
    let (_bd, b) = vault();
    let page = a.create_page(None, "Concurrent choice".into()).unwrap();
    a.apply(
        page.clone(),
        vec![
            insert("outside", None, None, "Outside"),
            insert("grand", None, Some("outside"), "Grand"),
            insert("parent", Some("grand"), None, "Parent"),
            insert("target", Some("parent"), None, "Original"),
        ],
    )
    .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![text("target", "Cloud choice")])
        .unwrap();
    b.apply(page.clone(), vec![text("target", "Local choice")])
        .unwrap();
    copy(&a, &b, &page);
    copy(&b, &a, &page);
    let conflict = b.list_conflicts(Some(page.clone())).unwrap().remove(0);
    let side = conflict.sides.iter().position(|side| {
        matches!(&side.state, SideState::Present { content } if content[0].text == "Cloud choice")
    }).unwrap() as u32;
    b.apply(page.clone(), vec![BlockOp::Delete { id: "grand".into() }])
        .unwrap();
    copy(&b, &a, &page);
    a.apply(
        page.clone(),
        vec![text("outside", "Concurrent unrelated edit")],
    )
    .unwrap();
    b.resolve_conflict(
        page.clone(),
        conflict.id.clone(),
        ConflictResolution::UseSide(side),
    )
    .unwrap();
    copy(&b, &a, &page);
    assert_eq!(
        a.load_page(page.clone()).unwrap().blocks[0].text,
        "Concurrent unrelated edit"
    );
    assert!(a.list_conflicts(Some(page.clone())).unwrap().is_empty());
    let log = a.conflict_log(Some(page.clone())).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].id, conflict.id);
    copy(&a, &b, &page);
    assert!(b.list_conflicts(Some(page.clone())).unwrap().is_empty());
    assert_eq!(
        b.load_page(page).unwrap().blocks[0].text,
        "Concurrent unrelated edit"
    );
}

#[test]
fn explicit_history_restore_uses_archived_content_without_creating_another_conflict() {
    let (_ad, a) = vault();
    let (_bd, b) = vault();
    let page = a.create_page(None, "Historical restore".into()).unwrap();
    a.apply(page.clone(), vec![insert("target", None, None, "Original")])
        .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![text("target", "Cloud")])
        .unwrap();
    b.apply(page.clone(), vec![text("target", "Local")])
        .unwrap();
    copy(&a, &b, &page);
    let conflict = b.list_conflicts(Some(page.clone())).unwrap().remove(0);
    b.resolve_conflict(page.clone(), conflict.id.clone(), ConflictResolution::Keep)
        .unwrap();
    let version = b.list_versions(page.clone()).unwrap()[0].id.clone();
    let expected = b
        .load_page_at(page.clone(), version.clone())
        .unwrap()
        .blocks;
    b.delete_page(page.clone()).unwrap();
    b.purge_page(page.clone()).unwrap();
    assert!(matches!(
        b.load_page(page.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(
        b.load_page_at(page.clone(), version.clone())
            .unwrap()
            .blocks,
        expected
    );
    b.restore_version(page.clone(), version).unwrap();
    assert_eq!(b.load_page(page.clone()).unwrap().blocks, expected);
    assert!(b.list_conflicts(Some(page.clone())).unwrap().is_empty());
    assert_eq!(
        b.conflict_log(Some(page.clone())).unwrap()[0].id,
        conflict.id
    );
    copy(&b, &a, &page);
    assert_eq!(a.load_page(page.clone()).unwrap().blocks, expected);
    assert_eq!(a.conflict_log(Some(page)).unwrap().len(), 1);
}

#[test]
fn legacy_id_only_purge_keeps_conflict_log_and_historical_page_without_becoming_causal() {
    let (_ad, a) = vault();
    let (bd, b) = vault();
    let page = a.create_page(None, "Legacy purge history".into()).unwrap();
    a.apply(page.clone(), vec![insert("target", None, None, "Original")])
        .unwrap();
    copy(&a, &b, &page);
    a.apply(page.clone(), vec![text("target", "Cloud")])
        .unwrap();
    b.apply(page.clone(), vec![text("target", "Local")])
        .unwrap();
    copy(&a, &b, &page);
    let conflict = b.list_conflicts(Some(page.clone())).unwrap().remove(0);
    b.resolve_conflict(page.clone(), conflict.id.clone(), ConflictResolution::Keep)
        .unwrap();
    let version = b.list_versions(page.clone()).unwrap()[0].id.clone();
    let expected = b
        .load_page_at(page.clone(), version.clone())
        .unwrap()
        .blocks;
    b.import_purged(vec![page.clone()]).unwrap();
    assert!(matches!(
        b.load_page(page.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert!(b
        .purge_records()
        .iter()
        .any(|record| record.page == page && record.version.is_none()));
    assert_eq!(
        b.conflict_log(Some(page.clone())).unwrap()[0].id,
        conflict.id
    );
    assert_eq!(
        b.load_page_at(page.clone(), version.clone())
            .unwrap()
            .blocks,
        expected
    );
    b.flush().unwrap();
    drop(b);
    let reopened = Vault::open(bd.0.to_string_lossy().into_owned()).unwrap();
    assert!(matches!(
        reopened.load_page(page.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(
        reopened.conflict_log(Some(page.clone())).unwrap()[0].id,
        conflict.id
    );
    assert_eq!(
        reopened.load_page_at(page, version).unwrap().blocks,
        expected
    );
}
