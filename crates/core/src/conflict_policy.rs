//! Apply the vault-wide switch and expire resolved conflict logs.
//!
//! All helpers operate on the supplied document only. They never acquire vault,
//! database or page locks, so hot import and archive callers can use them safely.

use crate::conflict_settings::CONFLICT_LOG_RETENTION_MS;
use crate::doc::PageDoc;
use crate::error::Result;
use crate::types::{Conflict, ConflictStatus, ResolutionBy, SideState};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PolicyEffect {
    pub resolved: u32,
    pub trimmed: u32,
}

impl PolicyEffect {
    pub(crate) fn changed(self) -> bool {
        self.resolved != 0 || self.trimmed != 0
    }
}

fn rule_applied_delete_edit(record: &Conflict) -> bool {
    record
        .sides
        .iter()
        .any(|side| matches!(side.state, SideState::Present { .. }))
        && record.sides.iter().any(|side| {
            matches!(
                side.state,
                SideState::Deleted | SideState::Trashed { .. } | SideState::Purged { .. }
            )
        })
}

/// Dismiss already applied automatic entries; never reapply content and never
/// reopen past resolutions when the switch is turned off. Text/text remains
/// pending regardless of the switch. Journal entries are always log-only.
///
/// Retention counts from the latest resolution or its undo. At exactly 30 days
/// a resolved log expires. Pending records survive; only consumed undo history
/// expires, together with the older applications it superseded.
pub(crate) fn apply(doc: &PageDoc, enabled: bool, now_ms: i64) -> Result<PolicyEffect> {
    let page = doc.meta().id;
    let cutoff = now_ms.saturating_sub(CONFLICT_LOG_RETENTION_MS);
    let mut effect = PolicyEffect::default();
    for (key, json) in doc.conflicts() {
        let Some(mut record) = crate::sync::read_conflict(&page, &json) else {
            continue;
        };
        if record.status == ConflictStatus::Pending {
            if let Some(latest) = record.applied.last() {
                let dismiss = match &latest.by {
                    ResolutionBy::Auto { rule } if rule == crate::conflict_detect::JOURNAL_RULE => {
                        rule_applied_delete_edit(&record)
                    }
                    ResolutionBy::Auto { rule } if rule == crate::conflict_detect::EDIT_RULE => {
                        enabled && latest.undone_at.is_none() && rule_applied_delete_edit(&record)
                    }
                    _ => false,
                };
                if dismiss {
                    record.status = ConflictStatus::Resolved(latest.clone());
                    crate::sync::write_conflict(doc, &record)?;
                    effect.resolved += 1;
                }
            }
        }
        if let ConflictStatus::Resolved(latest) = &record.status {
            if latest.undone_at.unwrap_or(latest.time_ms) <= cutoff {
                crate::sync::retain_conflict_history(doc)?;
                doc.remove_conflict(&key)?;
                effect.trimmed += 1;
            }
        } else if let Some(expired_undo) = record.applied.iter().rposition(|applied| {
            applied.undone_at.is_some_and(|undone_at| undone_at <= cutoff)
        }) {
            // Never reveal a consumed older auto application as the latest
            // application: a later sweep would otherwise dismiss this entry.
            record.applied.drain(..=expired_undo);
            crate::sync::write_conflict(doc, &record)?;
            effect.trimmed += 1;
        }
    }
    Ok(effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AppliedResolution, ConflictScope, ConflictSide};

    fn page() -> PageDoc {
        PageDoc::new_page(1, "page", "Page", None, 0).unwrap()
    }

    fn record(
        doc: &PageDoc,
        id: &str,
        time_ms: i64,
        rule: Option<&str>,
        text_text: bool,
        resolved: bool,
    ) -> Conflict {
        let frontiers = Some(doc.version());
        let sides = vec![
            ConflictSide::new(
                "left".into(),
                0,
                frontiers.clone(),
                SideState::Present {
                    content: Vec::new(),
                },
                None,
            ),
            ConflictSide::new(
                "right".into(),
                0,
                frontiers,
                if text_text {
                    SideState::Present {
                        content: Vec::new(),
                    }
                } else {
                    SideState::Deleted
                },
                None,
            ),
        ];
        let applied: Vec<_> = rule
            .map(|rule| AppliedResolution {
                time_ms,
                by: ResolutionBy::Auto { rule: rule.into() },
                before: doc.version(),
                before_state: None,
                fallback: false,
                restored_ancestors: Vec::new(),
                undone_at: None,
                resolution: None,
            })
            .into_iter()
            .collect();
        let status = if resolved {
            ConflictStatus::Resolved(applied.last().unwrap().clone())
        } else {
            ConflictStatus::Pending
        };
        Conflict {
            page: "page".into(),
            id: id.into(),
            format: 2,
            block: id.into(),
            time_ms: 0,
            scope: ConflictScope::Block { block: id.into() },
            sides,
            status,
            applied,
        }
    }

    fn records(doc: &PageDoc) -> Vec<Conflict> {
        doc.conflicts()
            .into_iter()
            .filter_map(|(_, json)| crate::sync::read_conflict("page", &json))
            .collect()
    }

    #[test]
    fn off_keeps_rule_pending_on_dismisses_without_content_change_and_text_stays_pending() {
        let doc = page();
        for entry in [
            record(&doc, "rule", 100, Some("edit-beats-delete"), false, false),
            record(&doc, "text", 100, None, true, false),
            record(
                &doc,
                "text-rule",
                100,
                Some("edit-beats-delete"),
                true,
                false,
            ),
        ] {
            crate::sync::write_conflict(&doc, &entry).unwrap();
        }
        assert!(!apply(&doc, false, 101).unwrap().changed());
        let content = doc.doc.get_map("page").get_value();
        let effect = apply(&doc, true, 101).unwrap();
        assert_eq!(effect.resolved, 1);
        assert_eq!(effect.trimmed, 0);
        let entries = records(&doc);
        assert!(matches!(
            entries
                .iter()
                .find(|entry| entry.id == "rule")
                .unwrap()
                .status,
            ConflictStatus::Resolved(_)
        ));
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.id == "text")
                .unwrap()
                .status,
            ConflictStatus::Pending
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.id == "text-rule")
                .unwrap()
                .status,
            ConflictStatus::Pending
        );
        assert_eq!(doc.doc.get_map("page").get_value(), content);
        assert!(!apply(&doc, false, 102).unwrap().changed());
    }

    #[test]
    fn journal_is_log_only_even_with_switch_off() {
        let doc = page();
        crate::sync::write_conflict(
            &doc,
            &record(&doc, "journal", 100, Some("journal"), false, false),
        )
        .unwrap();
        assert_eq!(apply(&doc, false, 101).unwrap().resolved, 1);
        assert!(matches!(
            records(&doc)[0].status,
            ConflictStatus::Resolved(_)
        ));
    }

    #[test]
    fn thirty_day_boundary_uses_latest_application_and_never_trims_pending() {
        let doc = page();
        let now = 3 * CONFLICT_LOG_RETENTION_MS;
        let boundary = now - CONFLICT_LOG_RETENTION_MS;
        for entry in [
            record(
                &doc,
                "expired",
                boundary - 1,
                Some("edit-beats-delete"),
                false,
                true,
            ),
            record(
                &doc,
                "boundary",
                boundary,
                Some("edit-beats-delete"),
                false,
                true,
            ),
            record(
                &doc,
                "retained",
                boundary + 1,
                Some("edit-beats-delete"),
                false,
                true,
            ),
            record(&doc, "pending", 0, Some("edit-beats-delete"), false, false),
            record(&doc, "text", 0, None, true, false),
        ] {
            crate::sync::write_conflict(&doc, &entry).unwrap();
        }
        let effect = apply(&doc, false, now).unwrap();
        assert_eq!(
            effect,
            PolicyEffect {
                resolved: 0,
                trimmed: 2
            }
        );
        let mut ids: Vec<_> = records(&doc).into_iter().map(|entry| entry.id).collect();
        ids.sort();
        assert_eq!(ids, ["pending", "retained", "text"]);
    }

    #[test]
    fn old_detection_time_does_not_expire_a_recent_resolution() {
        let doc = page();
        let now = 3 * CONFLICT_LOG_RETENTION_MS;
        let mut entry = record(
            &doc,
            "recent",
            now - 1,
            Some("edit-beats-delete"),
            false,
            true,
        );
        entry.time_ms = 0;
        crate::sync::write_conflict(&doc, &entry).unwrap();
        assert!(!apply(&doc, false, now).unwrap().changed());
        assert_eq!(records(&doc).len(), 1);
    }

    #[test]
    fn explicitly_undone_pending_stays_pending_with_either_switch_state() {
        for enabled in [false, true] {
            let doc = page();
            let mut entry = record(&doc, "undone", 100, Some("edit-beats-delete"), false, false);
            entry.applied.last_mut().unwrap().undone_at = Some(200);
            crate::sync::write_conflict(&doc, &entry).unwrap();

            assert!(!apply(&doc, enabled, 201).unwrap().changed());
            assert_eq!(records(&doc), vec![entry]);
        }
    }

    #[test]
    fn journal_undo_is_log_only_with_either_switch_state() {
        for enabled in [false, true] {
            let doc = page();
            let mut entry = record(&doc, "journal", 100, Some("journal"), false, false);
            entry.applied.last_mut().unwrap().undone_at = Some(200);
            crate::sync::write_conflict(&doc, &entry).unwrap();

            assert_eq!(apply(&doc, enabled, 201).unwrap().resolved, 1);
            let stored = records(&doc).pop().unwrap();
            assert_eq!(stored.status, ConflictStatus::Resolved(entry.applied[0].clone()));
            assert_eq!(stored.applied, entry.applied);
        }
    }

    #[test]
    fn pending_undo_retention_removes_consumed_prefix_without_deleting_or_redismissing_record() {
        let doc = page();
        let now = 3 * CONFLICT_LOG_RETENTION_MS;
        let boundary = now - CONFLICT_LOG_RETENTION_MS;
        let mut expired = record(&doc, "expired-undo", 10, Some("edit-beats-delete"), false, false);
        let mut consumed = expired.applied[0].clone();
        consumed.time_ms = 20;
        consumed.undone_at = Some(boundary);
        expired.applied.push(consumed);
        let mut fresh = record(&doc, "fresh-undo", 10, Some("edit-beats-delete"), false, false);
        fresh.applied[0].undone_at = Some(boundary + 1);
        let untouched = record(&doc, "awaiting-ok", 10, Some("edit-beats-delete"), false, false);
        for entry in [&expired, &fresh, &untouched] {
            crate::sync::write_conflict(&doc, entry).unwrap();
        }

        assert_eq!(apply(&doc, false, now).unwrap().trimmed, 1);
        let entries = records(&doc);
        let expired = entries.iter().find(|entry| entry.id == "expired-undo").unwrap();
        assert_eq!(expired.status, ConflictStatus::Pending);
        assert!(expired.applied.is_empty());
        assert_eq!(entries.iter().find(|entry| entry.id == "fresh-undo").unwrap(), &fresh);
        assert_eq!(entries.iter().find(|entry| entry.id == "awaiting-ok").unwrap(), &untouched);

        // Turning the switch on can dismiss awaiting OK, but neither undo.
        assert_eq!(apply(&doc, true, now).unwrap().resolved, 1);
        let entries = records(&doc);
        assert_eq!(entries.iter().find(|entry| entry.id == "expired-undo").unwrap().status, ConflictStatus::Pending);
        assert_eq!(entries.iter().find(|entry| entry.id == "fresh-undo").unwrap().status, ConflictStatus::Pending);
    }

    #[test]
    fn journal_undo_log_retention_counts_from_undo_and_expires_at_thirty_days() {
        let doc = page();
        let now = 3 * CONFLICT_LOG_RETENTION_MS;
        let boundary = now - CONFLICT_LOG_RETENTION_MS;
        for (id, undone_at) in [("expired", boundary), ("fresh", boundary + 1)] {
            let mut entry = record(&doc, id, 10, Some("journal"), false, true);
            entry.applied[0].undone_at = Some(undone_at);
            entry.status = ConflictStatus::Resolved(entry.applied[0].clone());
            crate::sync::write_conflict(&doc, &entry).unwrap();
        }

        assert_eq!(apply(&doc, false, now).unwrap().trimmed, 1);
        let entries = records(&doc);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "fresh");
        assert_eq!(entries[0].applied[0].undone_at, Some(boundary + 1));
    }

    #[test]
    fn trimming_final_current_or_legacy_log_preserves_synced_history_marker() {
        for format in [1, 2] {
            let doc = page();
            let mut entry = record(&doc, "expired", 0, Some("edit-beats-delete"), false, true);
            entry.format = format;
            crate::sync::write_conflict(&doc, &entry).unwrap();
            // Simulate a record written before the durable marker existed.
            doc.doc.get_map("conflict-history").delete("retained").unwrap();
            assert!(!crate::sync::has_conflict_history(&doc));

            assert_eq!(apply(&doc, false, CONFLICT_LOG_RETENTION_MS).unwrap().trimmed, 1);
            assert!(doc.conflicts().is_empty());
            assert!(crate::sync::has_conflict_history(&doc));
            let synced = PageDoc::from_bytes(&doc.snapshot().unwrap(), &[]).unwrap();
            assert!(synced.conflicts().is_empty());
            assert!(crate::sync::has_conflict_history(&synced));
        }
    }
}
