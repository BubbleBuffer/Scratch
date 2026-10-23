//! Apply a conflict choice and restore its scoped before-state.
//!
//! The caller persists the returned application in the conflict record and
//! handles page existence/purge transport. This module edits page content only;
//! historical frontiers, rather than the display blocks, define format 2 picks.

use loro::Frontiers;
use std::collections::HashSet;

use crate::conflict_restore::{restore_tagged, undo_restore_tagged, RestoreEffect};
use crate::doc::PageDoc;
use crate::error::{invalid, Result};
use crate::types::{
    AppliedResolution, BeforeState, Block, BlockOp, Conflict, ConflictResolution, ConflictScope,
    NewBlock, ResolutionBy, SideState,
};

/// Lifecycle work the Vault must apply along with the edited document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageAction {
    Unchanged,
    Present,
    Trashed { at: i64 },
    Purged { tombstone_version: Vec<u8> },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolutionOutcome {
    pub applied: AppliedResolution,
    pub page_action: PageAction,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UndoOutcome {
    pub page_action: PageAction,
    pub fallback: bool,
    pub restored_ancestors: Vec<String>,
}

fn scope(conflict: &Conflict) -> (Option<&str>, bool) {
    match &conflict.scope {
        ConflictScope::Block { block } => (Some(block), false),
        ConflictScope::Subtree { root } => (Some(root), true),
        ConflictScope::Page => (None, true),
    }
}

fn check_page(doc: &PageDoc, conflict: &Conflict) -> Result<()> {
    if doc.meta().id != conflict.page {
        return Err(invalid("conflict belongs to another page"));
    }
    if scope(conflict).0.is_some_and(str::is_empty) {
        return Err(invalid("conflict scope must name a block"));
    }
    Ok(())
}

fn historical(doc: &PageDoc, bytes: &[u8]) -> Result<PageDoc> {
    if bytes.is_empty() {
        return Err(invalid("conflict version is missing"));
    }
    let frontiers =
        Frontiers::decode(bytes).map_err(|e| invalid(format!("invalid conflict version: {e}")))?;
    doc.fork_at(&frontiers)
}

fn find_block<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|block| {
        if block.id == id {
            Some(block)
        } else {
            find_block(&block.children, id)
        }
    })
}

fn collect_ids(blocks: &[Block], ids: &mut Vec<String>) {
    for block in blocks {
        ids.push(block.id.clone());
        collect_ids(&block.children, ids);
    }
}

fn add_effect(target: &mut RestoreEffect, extra: RestoreEffect) {
    target.fallback_to_root |= extra.fallback_to_root;
    for id in extra.restored_ancestors {
        if !target.restored_ancestors.contains(&id) {
            target.restored_ancestors.push(id);
        }
    }
}

/// Journal consolidation may move a restored section's children under another
/// heading and remove its original root. The saved content identifies those
/// scoped nodes even after that move; the surviving heading is never the scope.
fn undo_subtree_footprint(
    doc: &mut PageDoc,
    before: &mut PageDoc,
    conflict: &Conflict,
) -> Result<RestoreEffect> {
    let mut footprint = vec![conflict.scope.anchor_id().to_owned()];
    for side in &conflict.sides {
        if let SideState::Present { content } = &side.state {
            collect_ids(content, &mut footprint);
        }
    }
    let mut seen = HashSet::new();
    footprint.retain(|id| seen.insert(id.clone()));
    let blocks = before.blocks();
    let mut before_order = Vec::new();
    collect_ids(&blocks, &mut before_order);
    let before_ids: HashSet<_> = before_order.iter().cloned().collect();
    // The ordinary subtree restore already restored every node still below
    // this root in the before-version. Revisit only nodes outside that tree.
    let mut covered = Vec::new();
    if let Some(root) = find_block(&blocks, conflict.scope.anchor_id()) {
        collect_ids(std::slice::from_ref(root), &mut covered);
    }
    let covered: HashSet<_> = covered.into_iter().collect();
    // Delete introduced scoped nodes wherever consolidation moved them. A
    // parent deletion can already hide descendants, so recheck each live ID.
    for id in &footprint {
        if !before_ids.contains(id) && find_block(&doc.blocks(), id).is_some() {
            doc.apply(&[BlockOp::Delete { id: id.clone() }])?;
        }
    }
    let mut effect = RestoreEffect::default();
    // Restore own content and placement root-to-leaf, without replacing the
    // keeper's content or children outside this historical footprint.
    for id in before_order {
        if seen.contains(&id) && !covered.contains(&id) {
            add_effect(
                &mut effect,
                restore_tagged(
                    doc,
                    before,
                    Some(&id),
                    false,
                    None,
                    Some("sync:undo-conflict"),
                )?,
            );
        }
    }
    Ok(effect)
}

/// Apply Keep or UseSide. Keep records the current version, including when an
/// automatic rule already applied the editing side, without editing content.
/// `before_state` is supplied only for a purged page whose retained history is
/// available to this caller; ordinary trash state is read from the document.
pub(crate) fn resolve(
    doc: &mut PageDoc,
    conflict: &Conflict,
    resolution: &ConflictResolution,
    by: ResolutionBy,
    time_ms: i64,
    before_state: Option<BeforeState>,
) -> Result<ResolutionOutcome> {
    check_page(doc, conflict)?;
    if before_state.is_some() && !matches!(before_state, Some(BeforeState::Purged { .. })) {
        return Err(invalid(
            "only purge state may override the document before-state",
        ));
    }
    if before_state.is_some() && !matches!(conflict.scope, ConflictScope::Page) {
        return Err(invalid("purged before-state requires a page conflict"));
    }
    // Commit pending page edits before taking frontiers. In particular a caller
    // may record a conflict immediately after applying the always-on edit rule.
    doc.doc.commit();
    let before_state =
        before_state.or_else(|| doc.meta().trashed.map(|at| BeforeState::Trashed { at }));
    let before = if matches!(before_state, Some(BeforeState::Purged { .. })) {
        Vec::new()
    } else {
        doc.version()
    };
    let (effect, page_action) = match resolution {
        ConflictResolution::Keep => (RestoreEffect::default(), PageAction::Unchanged),
        ConflictResolution::UseSide(index) => {
            let side = conflict
                .sides
                .get(*index as usize)
                .ok_or_else(|| invalid(format!("no side {index}")))?;
            let (root, include_children) = scope(conflict);
            match (&conflict.scope, &side.state) {
                (ConflictScope::Page, SideState::Deleted) => {
                    return Err(invalid(
                        "a page conflict side is present, trashed or purged",
                    ));
                }
                (_, SideState::Trashed { .. } | SideState::Purged { .. }) if root.is_some() => {
                    return Err(invalid("a block conflict side is present or deleted"));
                }
                _ => {}
            }
            if let SideState::Purged { tombstone_version } = &side.state {
                // No page version exists on this side. Never manufacture one
                // from a purge version vector or discard retained page history.
                (
                    RestoreEffect::default(),
                    PageAction::Purged {
                        tombstone_version: tombstone_version.clone(),
                    },
                )
            } else if conflict.format < 2 {
                if !matches!(conflict.scope, ConflictScope::Block { .. }) {
                    return Err(invalid("legacy conflicts cover one block"));
                }
                (
                    restore_legacy(
                        doc,
                        root.expect("block scope"),
                        &side.state,
                        side.anchor.as_ref(),
                    )?,
                    PageAction::Unchanged,
                )
            } else {
                let mut source = historical(
                    doc,
                    side.frontiers
                        .as_deref()
                        .ok_or_else(|| invalid("conflict side has no historical version"))?,
                )?;
                if let Some(root) = root {
                    let present = find_block(&source.blocks(), root).is_some();
                    if present != matches!(side.state, SideState::Present { .. }) {
                        return Err(invalid(
                            "conflict side state does not match its historical scope",
                        ));
                    }
                }
                let anchor = side
                    .anchor
                    .as_ref()
                    .map(|a| (a.parent.as_deref(), a.previous.as_deref()));
                let effect = restore_tagged(
                    doc,
                    &source,
                    root,
                    include_children,
                    anchor,
                    Some("sync:resolve-conflict"),
                )?;
                let action = if root.is_none() {
                    match side.state {
                        SideState::Present { .. } => {
                            doc.set_meta_i64("trashed", None)?;
                            PageAction::Present
                        }
                        SideState::Trashed { at } => {
                            doc.set_meta_i64("trashed", Some(at))?;
                            PageAction::Trashed { at }
                        }
                        _ => unreachable!("page states checked above"),
                    }
                } else {
                    PageAction::Unchanged
                };
                (effect, action)
            }
        }
    };
    Ok(ResolutionOutcome {
        applied: AppliedResolution {
            time_ms,
            undone_at: None,
            by,
            before,
            before_state,
            fallback: effect.fallback_to_root,
            restored_ancestors: effect.restored_ancestors,
            resolution: Some(resolution.clone()),
        },
        page_action,
    })
}

/// Undo the latest applied choice. Record status/history changes stay with the
/// caller so the content edit and its synced record are saved together.
pub(crate) fn undo(doc: &mut PageDoc, conflict: &Conflict) -> Result<UndoOutcome> {
    check_page(doc, conflict)?;
    if !conflict.undoable() {
        return Err(invalid("legacy conflicts cannot be undone"));
    }
    let applied = conflict
        .applied
        .last()
        .ok_or_else(|| invalid("conflict has no applied resolution"))?;
    if let Some(BeforeState::Purged { tombstone_version }) = &applied.before_state {
        if !applied.before.is_empty() || !matches!(conflict.scope, ConflictScope::Page) {
            return Err(invalid("invalid purged resolution before-state"));
        }
        return Ok(UndoOutcome {
            page_action: PageAction::Purged {
                tombstone_version: tombstone_version.clone(),
            },
            fallback: false,
            restored_ancestors: Vec::new(),
        });
    }
    let mut before = historical(doc, &applied.before)?;
    let (root, include_children) = scope(conflict);
    let mut effect = undo_restore_tagged(
        doc,
        &before,
        root,
        include_children,
        &applied.restored_ancestors,
        Some("sync:undo-conflict"),
    )?;
    if matches!(conflict.scope, ConflictScope::Subtree { .. }) {
        add_effect(
            &mut effect,
            undo_subtree_footprint(doc, &mut before, conflict)?,
        );
    }
    let page_action = if root.is_none() {
        match applied.before_state {
            Some(BeforeState::Trashed { at }) => {
                doc.set_meta_i64("trashed", Some(at))?;
                PageAction::Trashed { at }
            }
            None => {
                doc.set_meta_i64("trashed", None)?;
                PageAction::Present
            }
            Some(BeforeState::Purged { .. }) => unreachable!("purge handled above"),
        }
    } else {
        PageAction::Unchanged
    };
    Ok(UndoOutcome {
        page_action,
        fallback: effect.fallback_to_root,
        restored_ancestors: effect.restored_ancestors,
    })
}

/// Format 1 has display content but no historical identities or ancestor
/// versions. Restore its single block; it remains non-undoable by format.
fn restore_legacy(
    doc: &mut PageDoc,
    root: &str,
    state: &SideState,
    anchor: Option<&crate::types::Anchor>,
) -> Result<RestoreEffect> {
    let blocks = doc.blocks();
    let present = find_block(&blocks, root).is_some();
    let block = match state {
        SideState::Present { content } if content.len() == 1 => Some(&content[0]),
        SideState::Deleted => None,
        _ => return Err(invalid("legacy side must hold one block or a deletion")),
    };
    let mut effect = RestoreEffect::default();
    let mut ops = Vec::new();
    match (block, present) {
        (None, true) => ops.push(BlockOp::Delete { id: root.into() }),
        (None, false) => {}
        (Some(block), present) => {
            if !present {
                let parent = anchor.and_then(|a| a.parent.as_deref());
                let parent_blocks = match parent {
                    Some(parent) => find_block(&blocks, parent).map(|b| &b.children),
                    None => Some(&blocks),
                };
                effect.fallback_to_root = parent_blocks.is_none();
                let siblings = parent_blocks.unwrap_or(&blocks);
                let previous = anchor.and_then(|a| a.previous.as_deref());
                let after = if effect.fallback_to_root || anchor.is_none() {
                    siblings.last().map(|b| b.id.clone())
                } else {
                    previous.and_then(|id| {
                        siblings
                            .iter()
                            .find(|b| b.id == id)
                            .map(|b| b.id.clone())
                            .or_else(|| siblings.last().map(|b| b.id.clone()))
                    })
                };
                ops.push(BlockOp::Insert {
                    parent: if effect.fallback_to_root {
                        None
                    } else {
                        parent.map(str::to_owned)
                    },
                    after,
                    block: NewBlock {
                        id: root.into(),
                        kind: block.kind.clone(),
                        text: block.text.clone(),
                        marks: block.marks.clone(),
                        checked: block.checked,
                    },
                });
            } else {
                ops.push(BlockOp::SetKind {
                    id: root.into(),
                    kind: block.kind.clone(),
                });
                ops.push(BlockOp::SetText {
                    id: root.into(),
                    text: block.text.clone(),
                    marks: block.marks.clone(),
                });
                ops.push(BlockOp::SetChecked {
                    id: root.into(),
                    checked: block.checked,
                });
            }
            ops.push(BlockOp::SetTarget {
                id: root.into(),
                target: block.target.clone(),
            });
            ops.push(BlockOp::SetColor {
                id: root.into(),
                color: block.color.clone(),
            });
            ops.push(BlockOp::SetWidth {
                id: root.into(),
                width: block.width,
            });
        }
    }
    doc.apply(&ops)?;
    Ok(effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Anchor, BlockKind, ConflictSide, ConflictStatus};

    fn insert(parent: Option<&str>, after: Option<&str>, id: &str, value: &str) -> BlockOp {
        BlockOp::Insert {
            parent: parent.map(str::to_owned),
            after: after.map(str::to_owned),
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Paragraph,
                text: value.into(),
                marks: vec![],
                checked: false,
            },
        }
    }

    fn text(id: &str, value: &str) -> BlockOp {
        BlockOp::SetText {
            id: id.into(),
            text: value.into(),
            marks: vec![],
        }
    }

    fn nested() -> PageDoc {
        let mut doc = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        doc.apply(&[
            insert(None, None, "left", "Left"),
            insert(None, Some("left"), "grand", "Grand"),
            insert(None, Some("grand"), "right", "Right"),
            insert(Some("grand"), None, "hidden", "Unneeded sibling"),
            insert(Some("grand"), Some("hidden"), "parent", "Parent"),
            insert(Some("parent"), None, "target", "Chosen root"),
            insert(Some("target"), None, "first", "First"),
            insert(Some("target"), Some("first"), "second", "Second"),
        ])
        .unwrap();
        doc
    }

    fn present(doc: &mut PageDoc, scope: &ConflictScope, anchor: Option<Anchor>) -> ConflictSide {
        doc.doc.commit();
        let blocks = doc.blocks();
        let content = match scope {
            ConflictScope::Page => blocks,
            ConflictScope::Block { block } => {
                let mut block = find_block(&blocks, block).unwrap().clone();
                block.children.clear();
                vec![block]
            }
            ConflictScope::Subtree { root } => vec![find_block(&blocks, root).unwrap().clone()],
        };
        ConflictSide::new(
            "device".into(),
            10,
            Some(doc.version()),
            SideState::Present { content },
            anchor,
        )
    }

    fn conflict(scope: ConflictScope, side: ConflictSide) -> Conflict {
        Conflict {
            page: "page".into(),
            id: "conflict".into(),
            format: 2,
            block: scope.anchor_id().into(),
            time_ms: 10,
            scope,
            sides: vec![side],
            status: ConflictStatus::Pending,
            applied: vec![],
        }
    }

    fn pick(doc: &mut PageDoc, conflict: &mut Conflict) -> ResolutionOutcome {
        let outcome = resolve(
            doc,
            conflict,
            &ConflictResolution::UseSide(0),
            ResolutionBy::User,
            20,
            None,
        )
        .unwrap();
        conflict.applied.push(outcome.applied.clone());
        conflict.status = ConflictStatus::Resolved(outcome.applied.clone());
        outcome
    }

    fn root_ids(doc: &mut PageDoc) -> Vec<String> {
        doc.blocks().into_iter().map(|b| b.id).collect()
    }

    fn target(doc: &mut PageDoc) -> Block {
        find_block(&doc.blocks(), "target").unwrap().clone()
    }

    #[test]
    fn keep_records_user_before_without_reapplying_the_rule() {
        let mut doc = nested();
        let scope = ConflictScope::Subtree {
            root: "target".into(),
        };
        let side = present(&mut doc, &scope, None);
        let mut conflict = conflict(scope, side);
        doc.apply(&[text("target", "Already applied editing version")])
            .unwrap();
        doc.doc.commit();
        let expected_before = doc.version();
        let expected_blocks = doc.blocks();
        let auto = AppliedResolution {
            undone_at: None,
            time_ms: 15,
            by: ResolutionBy::Auto {
                rule: "edit_wins".into(),
            },
            before: conflict.sides[0].frontiers.clone().unwrap(),
            before_state: None,
            fallback: false,
            restored_ancestors: vec![],
            resolution: Some(ConflictResolution::UseSide(0)),
        };
        conflict.applied.push(auto.clone());
        let result = resolve(
            &mut doc,
            &conflict,
            &ConflictResolution::Keep,
            ResolutionBy::User,
            20,
            None,
        )
        .unwrap();
        assert_eq!(result.applied.before, expected_before);
        assert_eq!(result.applied.by, ResolutionBy::User);
        assert_eq!(result.applied.time_ms, 20);
        assert_eq!(result.page_action, PageAction::Unchanged);
        assert_eq!(doc.blocks(), expected_blocks);
        assert_eq!(
            conflict.applied,
            vec![auto],
            "caller retains the automatic application"
        );
    }

    #[test]
    fn block_pick_uses_history_and_preserves_current_children() {
        let mut doc = nested();
        let scope = ConflictScope::Block {
            block: "target".into(),
        };
        let mut side = present(&mut doc, &scope, None);
        // Display materialization is deliberately stale; the version wins.
        if let SideState::Present { content } = &mut side.state {
            content[0].text = "Wrong display text".into();
        }
        let mut conflict = conflict(scope, side);
        doc.apply(&[text("target", "Merged"), text("first", "Current child")])
            .unwrap();
        pick(&mut doc, &mut conflict);
        assert_eq!(target(&mut doc).text, "Chosen root");
        assert_eq!(target(&mut doc).children[0].text, "Current child");
        doc.apply(&[
            text("right", "Later unrelated edit"),
            text("first", "Later child"),
        ])
        .unwrap();
        undo(&mut doc, &conflict).unwrap();
        assert_eq!(target(&mut doc).text, "Merged");
        assert_eq!(target(&mut doc).children[0].text, "Later child");
        assert_eq!(doc.blocks()[2].text, "Later unrelated edit");
    }

    #[test]
    fn subtree_pick_and_undo_restore_the_complete_scope() {
        let mut doc = nested();
        let scope = ConflictScope::Subtree {
            root: "target".into(),
        };
        let side = present(&mut doc, &scope, None);
        let mut conflict = conflict(scope, side);
        doc.apply(&[
            text("target", "Before pick"),
            text("second", "Changed child"),
            BlockOp::Delete { id: "first".into() },
            insert(Some("target"), Some("second"), "extra", "Extra"),
        ])
        .unwrap();
        let before_target = target(&mut doc);
        pick(&mut doc, &mut conflict);
        let chosen = target(&mut doc);
        assert_eq!(chosen.text, "Chosen root");
        assert_eq!(
            chosen
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(chosen.children[1].text, "Second");
        doc.apply(&[text("left", "Unrelated after pick")]).unwrap();
        undo(&mut doc, &conflict).unwrap();
        assert_eq!(target(&mut doc), before_target);
        assert_eq!(doc.blocks()[0].text, "Unrelated after pick");
    }

    #[test]
    fn deleted_side_pick_deletes_scope_and_undo_revives_before_subtree() {
        let mut doc = nested();
        doc.apply(&[BlockOp::Delete {
            id: "target".into(),
        }])
        .unwrap();
        doc.doc.commit();
        let side = ConflictSide::new(
            "device".into(),
            10,
            Some(doc.version()),
            SideState::Deleted,
            Some(Anchor {
                parent: Some("parent".into()),
                previous: None,
            }),
        );
        let mut conflict = conflict(
            ConflictScope::Subtree {
                root: "target".into(),
            },
            side,
        );
        // An explicit later edit has a new live physical root with the same
        // logical block ID; undo must use the actual before-version identity.
        doc.apply(&[
            insert(Some("parent"), None, "target", "Later root"),
            insert(Some("target"), None, "later-child", "Later child"),
        ])
        .unwrap();
        let before_target = target(&mut doc);
        pick(&mut doc, &mut conflict);
        assert!(find_block(&doc.blocks(), "target").is_none());
        undo(&mut doc, &conflict).unwrap();
        assert_eq!(target(&mut doc), before_target);
    }

    #[test]
    fn deleted_parent_pick_restores_chain_in_order_and_undo_removes_it() {
        let mut doc = nested();
        let scope = ConflictScope::Subtree {
            root: "target".into(),
        };
        let side = present(
            &mut doc,
            &scope,
            Some(Anchor {
                parent: Some("parent".into()),
                previous: None,
            }),
        );
        let mut conflict = conflict(scope, side);
        doc.apply(&[BlockOp::Delete { id: "grand".into() }])
            .unwrap();
        let outcome = pick(&mut doc, &mut conflict);
        assert_eq!(outcome.applied.restored_ancestors, ["grand", "parent"]);
        assert!(!outcome.applied.fallback);
        assert_eq!(root_ids(&mut doc), ["left", "grand", "right"]);
        let blocks = doc.blocks();
        assert_eq!(
            blocks[1]
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["parent"]
        );
        assert_eq!(blocks[1].children[0].children[0].id, "target");
        assert_eq!(blocks[1].children[0].children[0].children.len(), 2);
        doc.apply(&[
            text("right", "Later right"),
            insert(None, Some("right"), "later", "Later"),
        ])
        .unwrap();
        undo(&mut doc, &conflict).unwrap();
        assert_eq!(root_ids(&mut doc), ["left", "right", "later"]);
        assert_eq!(doc.blocks()[1].text, "Later right");
    }

    #[test]
    fn unavailable_historical_ancestor_appends_at_root_and_is_logged() {
        let mut doc = nested();
        let scope = ConflictScope::Subtree {
            root: "target".into(),
        };
        let side = present(
            &mut doc,
            &scope,
            Some(Anchor {
                parent: Some("unknown-parent".into()),
                previous: None,
            }),
        );
        let mut conflict = conflict(scope, side);
        let result = pick(&mut doc, &mut conflict);
        assert!(result.applied.fallback);
        assert!(result.applied.restored_ancestors.is_empty());
        assert!(conflict.applied.last().unwrap().fallback);
        assert_eq!(root_ids(&mut doc), ["left", "grand", "right", "target"]);
        undo(&mut doc, &conflict).unwrap();
        assert_eq!(root_ids(&mut doc), ["left", "grand", "right"]);
        assert_eq!(target(&mut doc).children.len(), 2);
    }

    #[test]
    fn journal_undo_handles_children_consolidated_under_a_surviving_heading() {
        let mut doc = nested();
        let scope = ConflictScope::Subtree {
            root: "target".into(),
        };
        let side = present(&mut doc, &scope, None);
        let mut conflict = conflict(scope, side);
        // The merge before repair already retained the first child under the
        // keeper, with its current edit; the old section root is absent.
        doc.apply(&[
            BlockOp::Move {
                id: "first".into(),
                parent: Some("grand".into()),
                after: Some("parent".into()),
            },
            text("first", "Before repair first"),
            BlockOp::Delete {
                id: "target".into(),
            },
        ])
        .unwrap();
        let result = resolve(
            &mut doc,
            &conflict,
            &ConflictResolution::UseSide(0),
            ResolutionBy::Auto {
                rule: "journal".into(),
            },
            20,
            None,
        )
        .unwrap();
        conflict.applied.push(result.applied);
        // Same-day heading consolidation removes the restored root but keeps
        // both restored children under a surviving heading.
        doc.apply(&[
            BlockOp::Move {
                id: "first".into(),
                parent: Some("grand".into()),
                after: Some("parent".into()),
            },
            BlockOp::Move {
                id: "second".into(),
                parent: Some("grand".into()),
                after: Some("first".into()),
            },
            BlockOp::Delete {
                id: "target".into(),
            },
            text("grand", "Keep later heading edit"),
            insert(
                Some("grand"),
                Some("second"),
                "unrelated",
                "Keep later sibling",
            ),
        ])
        .unwrap();
        undo(&mut doc, &conflict).unwrap();
        let blocks = doc.blocks();
        assert_eq!(blocks[1].text, "Keep later heading edit");
        assert_eq!(
            blocks[1]
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["hidden", "parent", "first", "unrelated"]
        );
        assert_eq!(blocks[1].children[2].text, "Before repair first");
        assert!(find_block(&blocks, "target").is_none());
        assert!(find_block(&blocks, "second").is_none());
        assert_eq!(
            find_block(&blocks, "unrelated").unwrap().text,
            "Keep later sibling"
        );
    }

    #[test]
    fn page_present_pick_untrashes_and_undo_restores_full_trashed_state() {
        let mut doc = nested();
        let side = present(&mut doc, &ConflictScope::Page, None);
        let mut conflict = conflict(ConflictScope::Page, side);
        doc.set_title("Trashed title").unwrap();
        doc.set_meta_i64("trashed", Some(50)).unwrap();
        doc.apply(&[text("left", "Trashed content")]).unwrap();
        let result = pick(&mut doc, &mut conflict);
        assert_eq!(result.page_action, PageAction::Present);
        assert_eq!(
            result.applied.before_state,
            Some(BeforeState::Trashed { at: 50 })
        );
        assert_eq!(doc.meta().trashed, None);
        assert_eq!(doc.meta().title, "Page");
        assert_eq!(doc.blocks()[0].text, "Left");
        let undone = undo(&mut doc, &conflict).unwrap();
        assert_eq!(undone.page_action, PageAction::Trashed { at: 50 });
        assert_eq!(doc.meta().trashed, Some(50));
        assert_eq!(doc.meta().title, "Trashed title");
        assert_eq!(doc.blocks()[0].text, "Trashed content");
    }

    #[test]
    fn page_trash_pick_restores_its_version_and_lifecycle() {
        let mut doc = nested();
        doc.set_meta_i64("trashed", Some(10)).unwrap();
        doc.doc.commit();
        let side = ConflictSide::new(
            "device".into(),
            10,
            Some(doc.version()),
            SideState::Trashed { at: 10 },
            None,
        );
        let mut conflict = conflict(ConflictScope::Page, side);
        doc.set_meta_i64("trashed", None).unwrap();
        doc.set_title("Live title").unwrap();
        let result = pick(&mut doc, &mut conflict);
        assert_eq!(result.page_action, PageAction::Trashed { at: 10 });
        assert_eq!(doc.meta().title, "Page");
        assert_eq!(doc.meta().trashed, Some(10));
        assert_eq!(
            undo(&mut doc, &conflict).unwrap().page_action,
            PageAction::Present
        );
        assert_eq!(doc.meta().title, "Live title");
        assert_eq!(doc.meta().trashed, None);
    }

    #[test]
    fn purged_before_has_no_fake_frontiers_and_undo_returns_purge_action() {
        let mut doc = nested();
        let side = present(&mut doc, &ConflictScope::Page, None);
        let mut conflict = conflict(ConflictScope::Page, side);
        let purge = BeforeState::Purged {
            tombstone_version: vec![7, 8],
        };
        let result = resolve(
            &mut doc,
            &conflict,
            &ConflictResolution::UseSide(0),
            ResolutionBy::User,
            20,
            Some(purge.clone()),
        )
        .unwrap();
        assert!(result.applied.before.is_empty());
        assert_eq!(result.applied.before_state, Some(purge));
        conflict.applied.push(result.applied);
        let before_blocks = doc.blocks();
        let undone = undo(&mut doc, &conflict).unwrap();
        assert_eq!(
            undone.page_action,
            PageAction::Purged {
                tombstone_version: vec![7, 8]
            }
        );
        assert_eq!(
            doc.blocks(),
            before_blocks,
            "vault owns actual purge transport"
        );
    }

    #[test]
    fn purged_side_returns_lifecycle_action_without_reconstructing_a_version() {
        let mut doc = nested();
        let side = ConflictSide::new(
            "device".into(),
            10,
            None,
            SideState::Purged {
                tombstone_version: vec![1, 2],
            },
            None,
        );
        let conflict = conflict(ConflictScope::Page, side);
        let before_blocks = doc.blocks();
        let result = resolve(
            &mut doc,
            &conflict,
            &ConflictResolution::UseSide(0),
            ResolutionBy::User,
            20,
            None,
        )
        .unwrap();
        assert_eq!(
            result.page_action,
            PageAction::Purged {
                tombstone_version: vec![1, 2]
            }
        );
        assert!(!result.applied.before.is_empty());
        assert_eq!(doc.blocks(), before_blocks);
    }

    #[test]
    fn legacy_materialized_block_is_restored_but_cannot_be_undone() {
        let mut doc = nested();
        let scope = ConflictScope::Block {
            block: "target".into(),
        };
        let mut side = present(&mut doc, &scope, None);
        side.frontiers = None;
        let mut conflict = conflict(scope, side);
        conflict.format = 1;
        doc.apply(&[BlockOp::Delete {
            id: "target".into(),
        }])
        .unwrap();
        pick(&mut doc, &mut conflict);
        assert_eq!(root_ids(&mut doc), ["left", "grand", "right", "target"]);
        assert_eq!(target(&mut doc).text, "Chosen root");
        assert!(target(&mut doc).children.is_empty());
        assert!(undo(&mut doc, &conflict).is_err());
    }

    #[test]
    fn invalid_pick_does_not_change_page_content() {
        let mut doc = nested();
        let scope = ConflictScope::Block {
            block: "target".into(),
        };
        let mut side = present(&mut doc, &scope, None);
        side.frontiers = None;
        let conflict = conflict(scope, side);
        let before_blocks = doc.blocks();
        assert!(resolve(
            &mut doc,
            &conflict,
            &ConflictResolution::UseSide(0),
            ResolutionBy::User,
            20,
            None
        )
        .is_err());
        assert!(resolve(
            &mut doc,
            &conflict,
            &ConflictResolution::UseSide(1),
            ResolutionBy::User,
            20,
            None
        )
        .is_err());
        assert_eq!(doc.blocks(), before_blocks);
    }
}
