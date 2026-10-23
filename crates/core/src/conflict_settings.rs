//! Vault-wide conflict policy in the existing hidden, synced settings page.
//! The value travels as ordinary page type-data, never device-local metadata.
use crate::doc::PageDoc;
use crate::error::{invalid, Result};
use crate::vault::{lock, Inner, Vault};
use std::sync::atomic::Ordering;

pub const CONFLICT_LOG_RETENTION_DAYS: i64 = 30;
pub const CONFLICT_LOG_RETENTION_MS: i64 = CONFLICT_LOG_RETENTION_DAYS * 86_400_000;
pub(crate) const SETTINGS_PAGE: &str = "000000000000000000000000CA";
const SETTINGS_ADDON: &str = "sync";
const AUTO_RESOLVE_KEY: &str = "conflict_auto_resolve";

pub(crate) fn auto_resolve_in(doc: &PageDoc) -> bool {
    doc.type_data_value()
        .get(SETTINGS_ADDON)
        .and_then(|value| value.get(AUTO_RESOLVE_KEY))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

impl Inner {
    /// Hot import already holds database/page locks. This read takes neither.
    pub(crate) fn conflict_auto_resolve_cached(&self) -> bool {
        self.conflict_auto_resolve_cache.load(Ordering::Acquire)
    }

    /// Refresh from a settings document already locked by its caller. Return
    /// a transition so sync can schedule a vault sweep after releasing locks.
    /// Both deterministic and older role-based settings pages are supported.
    pub(crate) fn refresh_conflict_policy_from_doc(&self, doc: &PageDoc) -> Option<bool> {
        let meta = doc.meta();
        if meta.id != SETTINGS_PAGE && meta.role.as_deref() != Some("calendars") {
            return None;
        }
        let enabled = auto_resolve_in(doc);
        let previous = self
            .conflict_auto_resolve_cache
            .swap(enabled, Ordering::AcqRel);
        (previous != enabled).then_some(enabled)
    }
}

impl Vault {
    /// Absent policy means review is required. Reading an empty vault does
    /// not create a settings document.
    pub fn conflict_auto_resolve(&self) -> Result<bool> {
        self.refresh_conflict_policy()
    }

    /// Open/read refresh uses the existing role page only. Hot/cold import
    /// calls the nonlocking document hook instead.
    pub(crate) fn refresh_conflict_policy(&self) -> Result<bool> {
        let enabled = match self.find_by_role("calendars".into())? {
            Some(page) => self.inner.read(&page, |doc| auto_resolve_in(doc))?,
            None => false,
        };
        self.inner
            .conflict_auto_resolve_cache
            .store(enabled, Ordering::Release);
        Ok(enabled)
    }

    pub fn set_conflict_auto_resolve(&self, enabled: bool) -> Result<()> {
        // This initializes the deterministic hidden document under the
        // existing calendar lock, including for concurrent first use.
        self.list_calendars()?;
        let page = self
            .find_by_role("calendars".into())?
            .ok_or_else(|| invalid("synced settings page is missing"))?;
        self.inner.edit(&page, |doc| {
            doc.type_data_set(
                SETTINGS_ADDON,
                &[AUTO_RESOLVE_KEY.into()],
                &serde_json::json!(enabled),
            )
        })?;
        self.refresh_conflict_policy()?;
        self.sweep_conflict_policy()?;
        Ok(())
    }

    /// Called outside import locks after a settings transition and on open.
    /// Only pages represented in the conflict index are inspected, avoiding a
    /// document scan of the entire vault. Purged pages use their retained
    /// history archives and keep the same conflict index current.
    pub(crate) fn sweep_conflict_policy(&self) -> Result<u32> {
        let enabled = self.inner.conflict_auto_resolve_cached();
        let now = chrono::Utc::now().timestamp_millis();
        let db = lock(&self.inner.db);
        let mut pages: Vec<_> = db
            .conflicts(None)?
            .into_iter()
            .map(|(page, _)| page)
            .collect();
        pages.sort();
        pages.dedup();
        let tx = db.write_tx()?;
        let mut changes = 0u32;
        for page in pages {
            let apply_policy = |doc: &mut PageDoc| {
                let effect = crate::conflict_policy::apply(doc, enabled, now)?;
                if effect.changed() {
                    doc.doc.set_next_commit_message("sync:conflict-metadata");
                }
                Ok((effect, doc.conflicts()))
            };
            let edited = if db.exists(&page)? {
                self.inner.edit(&page, apply_policy)
            } else {
                self.inner.edit_conflict_archive(&page, apply_policy)
            };
            let (effect, records) = match edited {
                Ok(result) => result,
                Err(crate::error::VaultError::NotFound(_)) => continue,
                Err(error) => return Err(error),
            };
            if effect.changed() {
                db.set_conflicts(&page, &crate::sync::mirror_rows(&page, &records))?;
                changes = changes
                    .saturating_add(effect.resolved)
                    .saturating_add(effect.trimmed);
            }
        }
        tx.commit()?;
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn vault() -> (Temp, Vault) {
        let path = std::env::temp_dir().join(format!("conflict_policy_{}", crate::new_id()));
        let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
        (Temp(path), vault)
    }
    fn sync(from: &Vault, to: &Vault) {
        for version in from.page_versions().unwrap() {
            let bytes = from.export_updates(version.page.clone(), vec![]).unwrap();
            let snapshot = PageDoc::from_bytes(&bytes, &[]).unwrap();
            let settings = snapshot.meta().role.as_deref() == Some("calendars");
            let expected = auto_resolve_in(&snapshot);
            if settings {
                let sender = from
                    .inner
                    .read(&version.page, |doc| auto_resolve_in(doc))
                    .unwrap();
                assert_eq!(
                    expected, sender,
                    "settings export differs from sender; page={}",
                    version.page
                );
            }
            to.import_updates(version.page.clone(), bytes).unwrap();
            if settings {
                let (role, actual, data) = to
                    .inner
                    .read(&version.page, |doc| {
                        (
                            doc.meta().role,
                            auto_resolve_in(doc),
                            doc.type_data_value().get(SETTINGS_ADDON).cloned(),
                        )
                    })
                    .unwrap();
                assert_eq!(
                    role.as_deref(),
                    Some("calendars"),
                    "receiver settings role changed; page={}",
                    version.page
                );
                assert_eq!(
                    actual, expected,
                    "receiver settings data changed; page={}, sync={data:?}",
                    version.page
                );
                assert_eq!(to.inner.conflict_auto_resolve_cached(), expected,
                    "cache hook missed imported settings; page={}, role={role:?}, flag={actual}, sync={data:?}", version.page);
            }
        }
    }
    #[test]
    fn conflict_auto_resolve_default_and_both_switch_states() {
        let (_dir, vault) = vault();
        let before = vault.page_versions().unwrap();
        assert!(!vault.conflict_auto_resolve().unwrap());
        assert_eq!(vault.page_versions().unwrap(), before);
        assert!(!vault.inner.conflict_auto_resolve_cached());
        vault.set_conflict_auto_resolve(true).unwrap();
        assert!(vault.conflict_auto_resolve().unwrap());
        assert!(vault.inner.conflict_auto_resolve_cached());
        vault.set_conflict_auto_resolve(false).unwrap();
        assert!(!vault.conflict_auto_resolve().unwrap());
        assert!(!vault.inner.conflict_auto_resolve_cached());
    }
    #[test]
    fn conflict_auto_resolve_setting_syncs_both_directions_and_reopens() {
        let (dir, a) = vault();
        let (_other, b) = vault();
        a.set_conflict_auto_resolve(true).unwrap();
        assert!(
            a.inner.conflict_auto_resolve_cached(),
            "sender setter did not refresh its cache"
        );
        sync(&a, &b);
        assert!(b.inner.conflict_auto_resolve_cached());
        assert!(b.conflict_auto_resolve().unwrap());
        assert!(b.inner.conflict_auto_resolve_cached());
        b.set_conflict_auto_resolve(false).unwrap();
        sync(&b, &a);
        assert!(!a.inner.conflict_auto_resolve_cached());
        assert!(!a.conflict_auto_resolve().unwrap());
        assert!(!a.inner.conflict_auto_resolve_cached());
        a.flush().unwrap();
        drop(a);
        let reopened = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
        assert!(!reopened.conflict_auto_resolve().unwrap());
    }

    #[test]
    fn enabled_policy_cache_is_initialized_on_reopen() {
        let (dir, vault) = vault();
        vault.set_conflict_auto_resolve(true).unwrap();
        vault.flush().unwrap();
        drop(vault);
        let reopened = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
        assert!(reopened.inner.conflict_auto_resolve_cached());
        assert!(reopened.conflict_auto_resolve().unwrap());
    }

    #[test]
    fn policy_sweep_trims_purged_logs_and_preserves_archived_pending_undo() {
        use crate::types::{
            AppliedResolution, Conflict, ConflictScope, ConflictSide, ConflictStatus, ResolutionBy,
            SideState,
        };
        let (_dir, vault) = vault();
        let page = vault.create_page(None, "Archived conflict".into()).unwrap();
        let records = vault
            .inner
            .edit(&page, |doc| {
                let applied = AppliedResolution {
                    time_ms: 0,
                    by: ResolutionBy::Auto {
                        rule: "edit-beats-delete".into(),
                    },
                    before: doc.version(),
                    before_state: None,
                    fallback: false,
                    restored_ancestors: Vec::new(),
                    undone_at: None,
                    resolution: None,
                };
                let mut resolved = Conflict {
                    page: page.clone(),
                    id: "expired-log".into(),
                    format: 2,
                    block: "body".into(),
                    time_ms: 0,
                    scope: ConflictScope::Block {
                        block: "body".into(),
                    },
                    sides: vec![
                        ConflictSide::new(
                            "left".into(),
                            0,
                            Some(doc.version()),
                            SideState::Present {
                                content: Vec::new(),
                            },
                            None,
                        ),
                        ConflictSide::new(
                            "right".into(),
                            0,
                            Some(doc.version()),
                            SideState::Deleted,
                            None,
                        ),
                    ],
                    status: ConflictStatus::Resolved(applied.clone()),
                    applied: vec![applied],
                };
                crate::sync::write_conflict(doc, &resolved)?;
                resolved.id = "pending-undo".into();
                resolved.status = ConflictStatus::Pending;
                resolved.applied[0].undone_at = Some(0);
                crate::sync::write_conflict(doc, &resolved)?;
                Ok(doc.conflicts())
            })
            .unwrap();
        lock(&vault.inner.db)
            .set_conflicts(&page, &crate::sync::mirror_rows(&page, &records))
            .unwrap();
        vault.delete_page(page.clone()).unwrap();
        vault.purge_page(page.clone()).unwrap();
        assert_eq!(
            vault
                .inner
                .read_conflict_archive(&page)
                .unwrap()
                .conflicts()
                .len(),
            2
        );

        assert_eq!(vault.sweep_conflict_policy().unwrap(), 2);
        assert!(vault.conflict_log(Some(page.clone())).unwrap().is_empty());
        let pending = vault.list_conflicts(Some(page.clone())).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "pending-undo");
        assert!(pending[0].applied.is_empty());
        let archived = vault.inner.read_conflict_archive(&page).unwrap();
        assert_eq!(archived.conflicts().len(), 1);
        assert_eq!(archived.conflicts()[0].0, "pending-undo");
        assert!(matches!(
            vault.load_page(page),
            Err(crate::VaultError::NotFound(_))
        ));
    }

    #[test]
    fn enabling_policy_dismisses_an_existing_rule_application_without_reapplying_it() {
        use crate::types::{BlockKind, BlockOp, ConflictStatus, NewBlock};
        let (_dir, vault) = vault();
        let page = vault.create_page(None, "Conflict".into()).unwrap();
        vault
            .apply(
                page.clone(),
                vec![BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: NewBlock {
                        id: "body".into(),
                        kind: BlockKind::Paragraph,
                        text: "Before".into(),
                        marks: Vec::new(),
                        checked: false,
                    },
                }],
            )
            .unwrap();
        let (conflict, records) = vault
            .inner
            .edit(&page, |doc| {
                doc.doc.commit();
                let mut editing = doc.fork_at(&doc.doc.oplog_frontiers())?;
                editing.set_peer(doc.doc.peer_id() ^ 1)?;
                editing.register_peer("offline-editing-device")?;
                editing.apply(&[BlockOp::SetText {
                    id: "body".into(),
                    text: "Editing side".into(),
                    marks: Vec::new(),
                }])?;
                editing.doc.commit();
                doc.apply(&[BlockOp::Delete { id: "body".into() }])?;
                doc.doc.commit();
                let local_vv = doc.doc.oplog_vv();
                let local_f = doc.doc.oplog_frontiers();
                doc.import_updates(&editing.snapshot()?)?;
                let after = doc.doc.oplog_vv();
                let conflicts = crate::conflict_detect::detect(
                    doc,
                    &local_vv,
                    &local_f,
                    &after,
                    &std::collections::HashSet::new(),
                    false,
                )?;
                let conflict = conflicts.into_iter().next().unwrap();
                crate::sync::write_conflict(doc, &conflict)?;
                Ok((conflict, doc.conflicts()))
            })
            .unwrap();
        lock(&vault.inner.db)
            .set_conflicts(&page, &crate::sync::mirror_rows(&page, &records))
            .unwrap();
        assert_eq!(vault.list_conflicts(Some(page.clone())).unwrap().len(), 1);
        let before_application = conflict.applied.clone();
        vault.set_conflict_auto_resolve(true).unwrap();
        assert!(vault.list_conflicts(Some(page.clone())).unwrap().is_empty());
        let stored = vault
            .inner
            .read(&page, |doc| {
                let (_, json) = doc
                    .conflicts()
                    .into_iter()
                    .find(|(id, _)| id == &conflict.id)
                    .unwrap();
                crate::sync::read_conflict(&page, &json).unwrap()
            })
            .unwrap();
        assert!(matches!(stored.status, ConflictStatus::Resolved(_)));
        assert_eq!(stored.applied, before_application);
        assert_eq!(
            vault.load_page(page.clone()).unwrap().blocks[0].text,
            "Editing side"
        );
        vault.set_conflict_auto_resolve(false).unwrap();
        assert!(vault.list_conflicts(Some(page)).unwrap().is_empty());
    }

    #[test]
    fn document_refresh_is_lock_free_ignores_unrelated_docs_and_reports_transitions() {
        let (_dir, vault) = vault();
        let unrelated = PageDoc::new_page(9, "ordinary", "Ordinary", None, 0).unwrap();
        unrelated
            .type_data_set(
                SETTINGS_ADDON,
                &[AUTO_RESOLVE_KEY.into()],
                &serde_json::json!(true),
            )
            .unwrap();
        assert_eq!(
            vault.inner.refresh_conflict_policy_from_doc(&unrelated),
            None
        );
        assert!(!vault.inner.conflict_auto_resolve_cached());
        let doc = PageDoc::new_page(10, SETTINGS_PAGE, "Settings", None, 0).unwrap();
        doc.set_meta_str("role", Some("calendars")).unwrap();
        doc.type_data_set(
            SETTINGS_ADDON,
            &[AUTO_RESOLVE_KEY.into()],
            &serde_json::json!(true),
        )
        .unwrap();
        let _already_held_database = lock(&vault.inner.db);
        assert_eq!(
            vault.inner.refresh_conflict_policy_from_doc(&doc),
            Some(true)
        );
        assert!(vault.inner.conflict_auto_resolve_cached());
        assert_eq!(vault.inner.refresh_conflict_policy_from_doc(&doc), None);
        doc.type_data_set(
            SETTINGS_ADDON,
            &[AUTO_RESOLVE_KEY.into()],
            &serde_json::json!(false),
        )
        .unwrap();
        assert_eq!(
            vault.inner.refresh_conflict_policy_from_doc(&doc),
            Some(false)
        );
    }
}
