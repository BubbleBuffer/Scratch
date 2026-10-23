//! Historical conflict detection and edit-beats-delete application.
//!
//! Sync owns persistence/index effects. This module returns new records after
//! applying their rule; journal repair uses the same side/id helpers to log its
//! existing restoration, without changing that restoration algorithm.

use std::collections::{HashMap, HashSet};

use loro::{Frontiers, ToJson, VersionVector, ID};

use crate::doc::PageDoc;
use crate::error::{invalid, Result};
use crate::types::{
    Anchor, AppliedResolution, BeforeState, Block, Conflict, ConflictResolution, ConflictScope,
    ConflictSide, ConflictStatus, ResolutionBy, SideState,
};

pub(crate) const EDIT_RULE: &str = "edit-beats-delete";
pub(crate) const JOURNAL_RULE: &str = "journal";
/// Text edited on both sides: the block takes one side's own content.
pub(crate) const TEXT_RULE: &str = "later-text-wins";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn before_state(doc: &PageDoc) -> Option<BeforeState> {
    doc.meta().trashed.map(|at| BeforeState::Trashed { at })
}

/// Canonical scope and unordered causal versions identify the same event on
/// both receivers. Wall clocks and display side order are deliberately not
/// part of this identity.
pub(crate) fn event_id(scope: &ConflictScope, sides: &[ConflictSide]) -> Result<String> {
    let mut versions = Vec::with_capacity(sides.len());
    for side in sides {
        let mut ids: Vec<_> = match &side.frontiers {
            Some(bytes) => Frontiers::decode(bytes)
                .map_err(|error| invalid(error.to_string()))?
                .iter()
                .map(|id| (id.peer, id.counter))
                .collect(),
            None => Vec::new(),
        };
        ids.sort_unstable();
        versions.push(ids);
    }
    versions.sort();
    let bytes =
        serde_json::to_vec(&(scope, versions)).map_err(|error| invalid(error.to_string()))?;
    Ok(format!("v2:{}", crate::sync::sha256_hex(&bytes)))
}

fn device_between(
    peers: &HashMap<String, String>,
    other: &VersionVector,
    source: &VersionVector,
) -> String {
    let mut best = None;
    for (peer, &end) in source.iter() {
        let n = end - other.get(peer).copied().unwrap_or(0);
        if n > 0 && best.is_none_or(|(count, id)| n > count || (n == count && *peer < id)) {
            best = Some((n, *peer));
        }
    }
    best.map(|(_, peer)| {
        peers
            .get(&peer.to_string())
            .cloned()
            .unwrap_or_else(|| peer.to_string())
    })
    .unwrap_or_default()
}

fn side_time(doc: &PageDoc) -> i64 {
    doc.doc
        .oplog_frontiers()
        .iter()
        .filter_map(|id| {
            doc.doc
                .get_change(id)
                .map(|change| change.timestamp.saturating_mul(1000))
        })
        .max()
        .filter(|time| *time > 0)
        .unwrap_or_else(|| doc.meta().updated)
}

#[derive(Clone)]
struct Entry<'a> {
    block: &'a Block,
    anchor: Anchor,
}

fn index<'a>(blocks: &'a [Block]) -> (HashMap<&'a str, Entry<'a>>, Vec<&'a str>) {
    fn walk<'a>(
        blocks: &'a [Block],
        parent: Option<&str>,
        out: &mut HashMap<&'a str, Entry<'a>>,
        order: &mut Vec<&'a str>,
    ) {
        let mut previous = None;
        for block in blocks {
            out.insert(
                block.id.as_str(),
                Entry {
                    block,
                    anchor: Anchor {
                        parent: parent.map(str::to_owned),
                        previous: previous.map(str::to_owned),
                    },
                },
            );
            order.push(block.id.as_str());
            walk(&block.children, Some(&block.id), out, order);
            previous = Some(block.id.as_str());
        }
    }
    let mut out = HashMap::new();
    let mut order = Vec::new();
    walk(blocks, None, &mut out, &mut order);
    (out, order)
}

fn own_content(block: &Block) -> Block {
    Block {
        children: Vec::new(),
        ..block.clone()
    }
}

/// Ignore incidental timestamps, trash state and lifecycle/derived metadata
/// when deciding whether the live side actually edited a note.
fn page_content(doc: &PageDoc) -> serde_json::Value {
    let mut value = doc.doc.get_deep_value().to_json_value();
    if let Some(root) = value.as_object_mut() {
        root.retain(|name, _| {
            matches!(
                name.as_str(),
                "page"
                    | "blocks"
                    | "props"
                    | "schema"
                    | "schema_order"
                    | "views"
                    | "view_order"
                    | "type_data"
            )
        });
        if let Some(page) = root.get_mut("page").and_then(|value| value.as_object_mut()) {
            for key in [
                "updated",
                "trashed",
                "purge_restore",
                "purge_restore_warning",
            ] {
                page.remove(key);
            }
        }
    }
    value
}

struct Frame<'a> {
    doc: &'a PageDoc,
    blocks: &'a [Block],
    index: HashMap<&'a str, Entry<'a>>,
}

fn side(
    frame: &Frame<'_>,
    other: &VersionVector,
    scope: &ConflictScope,
    peers: &HashMap<String, String>,
    deleted: &mut crate::day_repair::DeletedBlocks<'_>,
) -> Result<ConflictSide> {
    let (state, anchor) = match scope {
        ConflictScope::Page => (
            frame.doc.meta().trashed.map_or_else(
                || SideState::Present {
                    content: frame.blocks.to_vec(),
                },
                |at| SideState::Trashed { at },
            ),
            None,
        ),
        ConflictScope::Block { block } | ConflictScope::Subtree { root: block } => {
            match frame.index.get(block.as_str()) {
                Some(entry) => (
                    SideState::Present {
                        content: vec![entry.block.clone()],
                    },
                    Some(entry.anchor.clone()),
                ),
                None => (
                    SideState::Deleted,
                    deleted.before_block_deletion_anchor(block)?,
                ),
            }
        }
    };
    Ok(ConflictSide::new(
        device_between(peers, other, &frame.doc.doc.oplog_vv()),
        side_time(frame.doc),
        Some(frame.doc.version()),
        state,
        anchor,
    ))
}

/// Build one historical side for the journal restore callsites.
pub(crate) fn historical_side(
    doc: &PageDoc,
    other: &VersionVector,
    scope: &ConflictScope,
    peers: &HashMap<String, String>,
) -> Result<ConflictSide> {
    let mut view = doc.fork_at(&doc.doc.oplog_frontiers())?;
    let blocks = view.blocks();
    let (items, _) = index(&blocks);
    let frame = Frame {
        doc,
        blocks: &blocks,
        index: items,
    };
    side(
        &frame,
        other,
        scope,
        peers,
        &mut crate::day_repair::DeletedBlocks::new(doc),
    )
}

fn record(page: &str, scope: ConflictScope, sides: Vec<ConflictSide>) -> Result<Conflict> {
    Ok(Conflict {
        page: page.to_owned(),
        id: event_id(&scope, &sides)?,
        format: 2,
        block: scope.anchor_id().to_owned(),
        time_ms: now_ms(),
        scope,
        sides,
        status: ConflictStatus::Pending,
        applied: Vec::new(),
    })
}

/// Log exactly the restoration the journal algorithm just performed. The
/// deletion frame is captured at its causal deletion operation, not synthesized
/// from the later merged/restored state.
pub(crate) fn journal_record(
    doc: &PageDoc,
    scope: ConflictScope,
    deletion: &PageDoc,
    editing: &PageDoc,
    before: Vec<u8>,
    before_state: Option<BeforeState>,
) -> Result<Conflict> {
    let peers = doc.peers();
    let sides = vec![
        historical_side(deletion, &editing.doc.oplog_vv(), &scope, &peers)?,
        historical_side(editing, &deletion.doc.oplog_vv(), &scope, &peers)?,
    ];
    let mut conflict = record(&doc.meta().id, scope, sides)?;
    let application = AppliedResolution {
        undone_at: None,
        time_ms: conflict.time_ms,
        by: ResolutionBy::Auto {
            rule: JOURNAL_RULE.into(),
        },
        before,
        before_state,
        fallback: false,
        restored_ancestors: Vec::new(),
        // The restored content is the editing side, sides[1].
        resolution: Some(ConflictResolution::UseSide(1)),
    };
    conflict.applied.push(application.clone());
    conflict.status = ConflictStatus::Resolved(application);
    Ok(conflict)
}

fn block_ids(block: &Block, out: &mut HashSet<String>) {
    out.insert(block.id.clone());
    for child in &block.children {
        block_ids(child, out);
    }
}

fn footprint(conflict: &Conflict) -> HashSet<String> {
    let mut out = HashSet::new();
    if !conflict.scope.anchor_id().is_empty() {
        out.insert(conflict.scope.anchor_id().to_owned());
    }
    for side in &conflict.sides {
        if let SideState::Present { content } = &side.state {
            for block in content {
                block_ids(block, &mut out);
            }
        }
    }
    out
}

fn journal(conflict: &Conflict) -> bool {
    conflict.applied.iter().any(|application| {
        matches!(&application.by,
        ResolutionBy::Auto { rule } if rule == JOURNAL_RULE)
    })
}

/// Journal repair owns deletion/restoration, but content edited on both sides
/// still needs review. A deleted ancestor hides the deleting side's last edit;
/// use its real last-visible frame, retaining frontiers for a later side pick.
fn journal_content_side(
    frame: &Frame<'_>,
    deleted: &mut crate::day_repair::DeletedBlocks<'_>,
    id: &str,
    other: &VersionVector,
    peers: &HashMap<String, String>,
) -> Result<Option<ConflictSide>> {
    let scope = ConflictScope::Block {
        block: id.to_owned(),
    };
    let side = if frame.index.contains_key(id) {
        side(frame, other, &scope, peers, deleted)?
    } else if let Some(prior) = deleted.before_block_deletion_frame(id)? {
        historical_side(prior, other, &scope, peers)?
    } else {
        return Ok(None);
    };
    Ok(matches!(side.state, SideState::Present { .. }).then_some(side))
}

fn own_side_content(side: &ConflictSide) -> Option<Block> {
    match &side.state {
        SideState::Present { content } => content.first().map(own_content),
        _ => None,
    }
}

/// The side whose text the text rule shows: the later `time_ms` wins; on a
/// tie the lexicographically greater `device` string (byte order) wins. Equal
/// devices fall back to the greater sorted frontier IDs, so every receiver
/// picks the same side whatever its local/remote order.
pub(crate) fn text_winner(sides: &[ConflictSide]) -> usize {
    let ids = |side: &ConflictSide| -> Vec<(u64, i32)> {
        let mut ids: Vec<_> = side
            .frontiers
            .as_deref()
            .and_then(|bytes| Frontiers::decode(bytes).ok())
            .map(|f| f.iter().map(|id| (id.peer, id.counter)).collect())
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    };
    (0..sides.len())
        .max_by(|&a, &b| {
            let (x, y) = (&sides[a], &sides[b]);
            x.time_ms
                .cmp(&y.time_ms)
                .then_with(|| x.device.as_bytes().cmp(y.device.as_bytes()))
                .then_with(|| ids(x).cmp(&ids(y)))
        })
        .unwrap_or(0)
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

/// Field edits that turn `current` into `target`, in a fixed order.
fn own_ops(current: &Block, target: &Block) -> Vec<crate::types::BlockOp> {
    use crate::types::BlockOp;
    let id = target.id.clone();
    let mut ops = Vec::new();
    if current.kind != target.kind {
        ops.push(BlockOp::SetKind {
            id: id.clone(),
            kind: target.kind.clone(),
        });
    }
    if current.text != target.text || current.marks != target.marks {
        ops.push(BlockOp::SetText {
            id: id.clone(),
            text: target.text.clone(),
            marks: target.marks.clone(),
        });
    }
    if current.checked != target.checked {
        ops.push(BlockOp::SetChecked {
            id: id.clone(),
            checked: target.checked,
        });
    }
    if current.target != target.target {
        ops.push(BlockOp::SetTarget {
            id: id.clone(),
            target: target.target.clone(),
        });
    }
    if current.color != target.color {
        ops.push(BlockOp::SetColor {
            id: id.clone(),
            color: target.color.clone(),
        });
    }
    if current.width != target.width {
        ops.push(BlockOp::SetWidth {
            id,
            width: target.width,
        });
    }
    ops
}

/// Apply the text rule to a both-sides block record. The edit is generated in
/// a fork at the union of both sides with a peer derived from the event id and
/// a fixed timestamp, so every device that detects the same event produces the
/// identical Loro change; replicas dedupe it instead of interleaving text.
fn apply_text_rule(
    doc: &mut PageDoc,
    conflict: &Conflict,
    merged: &VersionVector,
) -> Result<Option<AppliedResolution>> {
    let ConflictScope::Block { block } = &conflict.scope else {
        return Ok(None);
    };
    let winner = text_winner(&conflict.sides);
    let Some(target) = own_side_content(&conflict.sides[winner]) else {
        return Ok(None);
    };
    let mut fork = doc.fork_at(&doc.doc.vv_to_frontiers(merged))?;
    let Some(current) = find_block(&fork.blocks(), block).map(own_content) else {
        return Ok(None);
    };
    doc.doc.commit();
    let before = doc.version();
    let before_state = before_state(doc);
    let ops = own_ops(&current, &target);
    if !ops.is_empty() {
        fork.set_peer(crate::vault::peer_of(&format!(
            "{TEXT_RULE}:{}",
            conflict.id
        )))?;
        fork.apply(&ops)?;
        fork.doc.commit_with(
            loro::CommitOptions::new()
                .commit_msg("sync:resolve-conflict")
                .timestamp(conflict.sides[winner].time_ms.max(0) / 1000),
        );
        let bytes = fork.doc.export(loro::ExportMode::updates(merged))?;
        doc.import_updates(&bytes)?;
    }
    Ok(Some(AppliedResolution {
        undone_at: None,
        time_ms: conflict.time_ms,
        by: ResolutionBy::Auto {
            rule: TEXT_RULE.into(),
        },
        before,
        before_state,
        fallback: false,
        restored_ancestors: Vec::new(),
        resolution: Some(ConflictResolution::UseSide(winner as u32)),
    }))
}

/// Find, apply and return new concurrent events. Existing records are retained,
/// including older logs for the same block; only an identical event id is skipped.
/// `recovered` is the current import's actual journal repair footprint. Existing
/// Journal records made by this import add their materialized footprint for
/// consolidated heading aliases. Older logs never suppress a later new event.
pub(crate) fn detect(
    doc: &mut PageDoc,
    local_vv: &VersionVector,
    local_f: &Frontiers,
    new_vv: &VersionVector,
    recovered: &HashSet<String>,
    auto_resolve: bool,
) -> Result<Vec<Conflict>> {
    let heads: Vec<_> = new_vv
        .iter()
        .filter(|(peer, &end)| end > local_vv.get(peer).copied().unwrap_or(0))
        .map(|(peer, &end)| ID::new(*peer, end - 1))
        .collect();
    if heads.is_empty() {
        return Ok(Vec::new());
    }
    let remote_f = Frontiers::from(heads);
    let Some(remote_vv) = doc.doc.frontiers_to_vv(&remote_f) else {
        return Ok(Vec::new());
    };
    if remote_vv.includes_vv(local_vv) {
        return Ok(Vec::new());
    }
    let common_vv = local_vv.intersection(&remote_vv);
    let common_f = doc.doc.vv_to_frontiers(&common_vv);
    let mut base_doc = doc.fork_at(&common_f)?;
    let mut local_doc = doc.fork_at(local_f)?;
    let mut remote_doc = doc.fork_at(&remote_f)?;
    let base_blocks = base_doc.blocks();
    let local_blocks = local_doc.blocks();
    let remote_blocks = remote_doc.blocks();
    let (base, order) = index(&base_blocks);
    let (local, _) = index(&local_blocks);
    let (remote, _) = index(&remote_blocks);
    let local_frame = Frame {
        doc: &local_doc,
        blocks: &local_blocks,
        index: local,
    };
    let remote_frame = Frame {
        doc: &remote_doc,
        blocks: &remote_blocks,
        index: remote,
    };
    let peers = doc.peers();
    let mut local_deleted = crate::day_repair::DeletedBlocks::new(&local_doc);
    let mut remote_deleted = crate::day_repair::DeletedBlocks::new(&remote_doc);
    let page = doc.meta().id;
    let existing: Vec<_> = doc
        .conflicts()
        .into_iter()
        .filter_map(|(_, json)| crate::sync::read_conflict(&page, &json))
        .collect();
    let existing_ids: HashSet<_> = existing.iter().map(|record| record.id.as_str()).collect();
    let mut journal_covered = recovered.clone();
    let mut journal_page = false;
    for conflict in existing.iter().filter(|record| {
        journal(record)
            && record.applied.iter().any(|application| {
                Frontiers::decode(&application.before)
                    .ok()
                    .and_then(|frontiers| doc.doc.frontiers_to_vv(&frontiers))
                    .is_some_and(|before| before.includes_vv(new_vv))
            })
    }) {
        if matches!(conflict.scope, ConflictScope::Page) {
            journal_page = true;
        }
        journal_covered.extend(footprint(conflict));
    }

    // Page trash/edit is the maximal scope and suppresses all child events.
    let page_editing = match (local_doc.meta().trashed, remote_doc.meta().trashed) {
        (Some(_), None) if page_content(&remote_doc) != page_content(&base_doc) => Some(1),
        (None, Some(_)) if page_content(&local_doc) != page_content(&base_doc) => Some(0),
        _ => None,
    };
    let mut candidates = Vec::new();
    let mut journal_content = Vec::new();
    if let Some(editing) = page_editing.filter(|_| !journal_page) {
        candidates.push((ConflictScope::Page, Some(editing)));
    } else {
        let mut covered = HashSet::new();
        for id in order {
            if covered.contains(id) {
                continue;
            }
            let original = base[id].block;
            if journal_page || journal_covered.contains(id) {
                let left =
                    journal_content_side(&local_frame, &mut local_deleted, id, &remote_vv, &peers)?;
                let right =
                    journal_content_side(&remote_frame, &mut remote_deleted, id, local_vv, &peers)?;
                if let (Some(left), Some(right)) = (left, right) {
                    let original = own_content(original);
                    let left_content = own_side_content(&left).expect("present side");
                    let right_content = own_side_content(&right).expect("present side");
                    if left_content != original
                        && right_content != original
                        && left_content != right_content
                    {
                        let conflict = record(
                            &page,
                            ConflictScope::Block { block: id.into() },
                            vec![left, right],
                        )?;
                        if !existing_ids.contains(conflict.id.as_str()) {
                            journal_content.push(conflict);
                        }
                    }
                }
                continue;
            }
            let left = local_frame.index.get(id);
            let right = remote_frame.index.get(id);
            let editing = match (left, right) {
                (None, Some(entry)) if entry.block != original => Some(1),
                (Some(entry), None) if entry.block != original => Some(0),
                _ => None,
            };
            if let Some(editing) = editing {
                let edited = if editing == 0 {
                    left.unwrap().block
                } else {
                    right.unwrap().block
                };
                let subtree = !original.children.is_empty() || !edited.children.is_empty();
                let scope = if subtree {
                    ConflictScope::Subtree { root: id.into() }
                } else {
                    ConflictScope::Block { block: id.into() }
                };
                if subtree {
                    block_ids(original, &mut covered);
                    block_ids(edited, &mut covered);
                }
                candidates.push((scope, Some(editing)));
            } else if let (Some(left), Some(right)) = (left, right) {
                let (base, left, right) = (
                    own_content(original),
                    own_content(left.block),
                    own_content(right.block),
                );
                if left != base && right != base && left != right {
                    candidates.push((ConflictScope::Block { block: id.into() }, None));
                }
            }
        }
    }

    let mut out = journal_content;
    for (scope, editing) in candidates {
        let sides = vec![
            side(&local_frame, &remote_vv, &scope, &peers, &mut local_deleted)?,
            side(&remote_frame, local_vv, &scope, &peers, &mut remote_deleted)?,
        ];
        let mut conflict = record(&page, scope, sides)?;
        if existing_ids.contains(conflict.id.as_str()) {
            continue;
        }
        if let Some(editing) = editing {
            doc.doc.commit();
            let before = doc.version();
            let before_state = before_state(doc);
            let source = if editing == 0 {
                &local_doc
            } else {
                &remote_doc
            };
            let root = (!matches!(conflict.scope, ConflictScope::Page))
                .then_some(conflict.scope.anchor_id());
            let anchor = conflict.sides[editing]
                .anchor
                .as_ref()
                .map(|anchor| (anchor.parent.as_deref(), anchor.previous.as_deref()));
            let effect = crate::conflict_restore::restore(
                doc,
                source,
                root,
                matches!(
                    conflict.scope,
                    ConflictScope::Subtree { .. } | ConflictScope::Page
                ),
                anchor,
            )?;
            let application = AppliedResolution {
                undone_at: None,
                time_ms: conflict.time_ms,
                by: ResolutionBy::Auto {
                    rule: EDIT_RULE.into(),
                },
                before,
                before_state,
                fallback: effect.fallback_to_root,
                restored_ancestors: effect.restored_ancestors,
                // The editing side is the one restored.
                resolution: Some(ConflictResolution::UseSide(editing as u32)),
            };
            conflict.applied.push(application.clone());
            if auto_resolve {
                conflict.status = ConflictStatus::Resolved(application);
            }
        } else {
            // Text edited on both sides: one side's text, still pending
            // review whatever the auto-resolve switch says.
            let mut merged = local_vv.clone();
            merged.merge(&remote_vv);
            if let Some(application) = apply_text_rule(doc, &conflict, &merged)? {
                conflict.applied.push(application);
            }
        }
        out.push(conflict);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockKind, BlockOp, NewBlock};

    fn insert(parent: Option<&str>, after: Option<&str>, id: &str, text: &str) -> BlockOp {
        BlockOp::Insert {
            parent: parent.map(str::to_owned),
            after: after.map(str::to_owned),
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Paragraph,
                text: text.into(),
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
    fn setup() -> PageDoc {
        let mut doc = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        doc.apply(&[
            insert(None, None, "root", "Root"),
            insert(None, Some("root"), "outside", "Outside"),
        ])
        .unwrap();
        doc.doc.commit();
        doc
    }
    fn branch(doc: &PageDoc, peer: u64) -> PageDoc {
        let mut branch = doc.fork_at(&doc.doc.oplog_frontiers()).unwrap();
        branch.set_peer(peer).unwrap();
        branch.register_peer(&format!("device-{peer}")).unwrap();
        branch
    }
    fn import(local: &mut PageDoc, remote: &PageDoc, policy: bool) -> Vec<Conflict> {
        local.doc.commit();
        remote.doc.commit();
        let vv = local.doc.oplog_vv();
        let frontier = local.doc.oplog_frontiers();
        local.import_updates(&remote.snapshot().unwrap()).unwrap();
        let after = local.doc.oplog_vv();
        detect(local, &vv, &frontier, &after, &HashSet::new(), policy).unwrap()
    }

    #[test]
    fn leaf_delete_edit_applies_edit_and_remains_pending_with_undo_version() {
        let base = setup();
        let mut deleted = branch(&base, 2);
        let mut editing = branch(&base, 3);
        deleted
            .apply(&[BlockOp::Delete { id: "root".into() }])
            .unwrap();
        editing.apply(&[text("root", "Editing side")]).unwrap();
        let conflicts = import(&mut deleted, &editing, false);
        assert_eq!(conflicts.len(), 1);
        let conflict = &conflicts[0];
        assert_eq!(
            conflict.scope,
            ConflictScope::Block {
                block: "root".into()
            }
        );
        assert_eq!(conflict.status, ConflictStatus::Pending);
        assert_eq!(conflict.applied.len(), 1);
        assert!(!conflict.applied[0].before.is_empty());
        assert_eq!(deleted.blocks()[0].text, "Editing side");
        assert!(conflict.sides.iter().all(|side| side.frontiers.is_some()));
        assert!(matches!(conflict.sides[0].state, SideState::Deleted));
    }

    #[test]
    fn nested_delete_edit_restores_entire_subtree_and_suppresses_child_events() {
        let mut base = setup();
        base.apply(&[
            insert(Some("root"), None, "parent", "Parent"),
            insert(Some("parent"), None, "deep", "Before"),
            insert(Some("root"), Some("parent"), "untouched", "Untouched"),
            insert(
                Some("root"),
                Some("untouched"),
                "removed",
                "Restore this too",
            ),
        ])
        .unwrap();
        base.doc.commit();
        let mut deleted = branch(&base, 2);
        let mut editing = branch(&base, 3);
        deleted
            .apply(&[
                BlockOp::Delete {
                    id: "removed".into(),
                },
                BlockOp::Delete { id: "root".into() },
                text("outside", "Keep outside deletion-side edit"),
            ])
            .unwrap();
        editing.apply(&[text("deep", "Deep offline edit")]).unwrap();
        let conflicts = import(&mut deleted, &editing, true);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0].scope,
            ConflictScope::Subtree {
                root: "root".into()
            }
        );
        assert!(matches!(conflicts[0].status, ConflictStatus::Resolved(_)));
        let blocks = deleted.blocks();
        assert_eq!(
            blocks[0]
                .children
                .iter()
                .map(|block| block.id.as_str())
                .collect::<Vec<_>>(),
            ["parent", "untouched", "removed"]
        );
        assert_eq!(blocks[0].children[0].children[0].text, "Deep offline edit");
        assert_eq!(blocks[1].text, "Keep outside deletion-side edit");
    }

    #[test]
    fn trash_edit_is_one_page_event_and_restores_the_full_editing_note() {
        let base = setup();
        let mut deleted = branch(&base, 2);
        let mut editing = branch(&base, 3);
        deleted
            .apply(&[BlockOp::Delete { id: "root".into() }])
            .unwrap();
        deleted.set_meta_i64("trashed", Some(42)).unwrap();
        editing.apply(&[text("outside", "Offline edit")]).unwrap();
        editing.set_title("Edited title").unwrap();
        let conflicts = import(&mut deleted, &editing, false);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].scope, ConflictScope::Page);
        assert_eq!(deleted.meta().trashed, None);
        assert_eq!(deleted.meta().title, "Edited title");
        assert_eq!(deleted.blocks().len(), 2);
        assert!(matches!(
            conflicts[0].sides[0].state,
            SideState::Trashed { at: 42 }
        ));
        assert!(matches!(
            conflicts[0].applied[0].before_state,
            Some(BeforeState::Trashed { at: 42 })
        ));
    }

    #[test]
    fn text_text_applies_later_side_and_stays_pending_even_with_auto_switch_on() {
        let base = setup();
        let mut left = branch(&base, 2);
        let mut right = branch(&base, 3);
        left.apply(&[text("root", "Left")]).unwrap();
        right.apply(&[text("root", "Right")]).unwrap();
        left.doc.commit();
        right.doc.commit();
        let vv = left.doc.oplog_vv();
        let frontier = left.doc.oplog_frontiers();
        left.import_updates(&right.snapshot().unwrap()).unwrap();
        let after = left.doc.oplog_vv();
        let conflicts = detect(&mut left, &vv, &frontier, &after, &HashSet::new(), true).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].status, ConflictStatus::Pending);
        assert_eq!(conflicts[0].applied.len(), 1);
        assert!(matches!(&conflicts[0].applied[0].by,
            ResolutionBy::Auto { rule } if rule == TEXT_RULE));
        assert!(!conflicts[0].applied[0].before.is_empty());
        // Right committed last; on a same-second tie device-3 > device-2.
        assert_eq!(text_winner(&conflicts[0].sides), 1);
        assert_eq!(left.blocks()[0].text, "Right");
    }

    /// Commit everything pending with one fixed change timestamp (seconds).
    fn commit_at(doc: &PageDoc, seconds: i64) {
        doc.doc
            .commit_with(loro::CommitOptions::new().timestamp(seconds));
    }

    fn detect_both(
        left: &PageDoc,
        right: &PageDoc,
    ) -> (PageDoc, Vec<Conflict>, PageDoc, Vec<Conflict>) {
        let mut a = left.fork_at(&left.doc.oplog_frontiers()).unwrap();
        a.set_peer(2).unwrap();
        let mut b = right.fork_at(&right.doc.oplog_frontiers()).unwrap();
        b.set_peer(3).unwrap();
        let ca = import(&mut a, right, false);
        let cb = import(&mut b, left, false);
        (a, ca, b, cb)
    }

    #[test]
    fn independent_detection_converges_on_the_later_sides_text() {
        let mut base = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        base.apply(&[insert(None, None, "root", "Review on Monday")])
            .unwrap();
        commit_at(&base, 10);
        let mut left = branch(&base, 2);
        let mut right = branch(&base, 3);
        // The earlier side rewrote the day; the later side keeps "Monday",
        // which the merge deleted, so the rule must also re-insert text.
        left.apply(&[text("root", "Review on Friday")]).unwrap();
        commit_at(&left, 100);
        right
            .apply(&[text("root", "RReview on Monday morning")])
            .unwrap();
        commit_at(&right, 200);
        let (mut a, ca, mut b, cb) = detect_both(&left, &right);
        assert_eq!(ca.len(), 1);
        assert_eq!(cb.len(), 1);
        assert_eq!(ca[0].id, cb[0].id);
        for record in [&ca[0], &cb[0]] {
            assert_eq!(record.status, ConflictStatus::Pending);
            assert!(matches!(&record.applied[0].by,
                ResolutionBy::Auto { rule } if rule == TEXT_RULE));
        }
        assert_eq!(a.blocks()[0].text, "RReview on Monday morning");
        assert_eq!(b.blocks()[0].text, "RReview on Monday morning");
        // Exchanging the independently generated rule edits changes nothing.
        crate::sync::write_conflict(&a, &ca[0]).unwrap();
        crate::sync::write_conflict(&b, &cb[0]).unwrap();
        a.doc.commit();
        b.doc.commit();
        let from_a = PageDoc::from_bytes(&a.snapshot().unwrap(), &[]).unwrap();
        let again_a = import(&mut a, &b, false);
        let again_b = import(&mut b, &from_a, false);
        assert!(again_a.is_empty(), "{again_a:?}");
        assert!(again_b.is_empty(), "{again_b:?}");
        assert_eq!(a.blocks()[0].text, "RReview on Monday morning");
        assert_eq!(b.blocks()[0].text, "RReview on Monday morning");
        assert_eq!(a.doc.oplog_vv(), b.doc.oplog_vv());
    }

    #[test]
    fn time_tie_resolves_by_greater_device_identically_on_both_receivers() {
        let mut base = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        base.apply(&[insert(None, None, "root", "Base")]).unwrap();
        commit_at(&base, 10);
        // device-9 > device-3 by bytes, although device-3 is applied later.
        let mut left = branch(&base, 9);
        let mut right = branch(&base, 3);
        left.apply(&[text("root", "From nine")]).unwrap();
        commit_at(&left, 100);
        right.apply(&[text("root", "From three")]).unwrap();
        commit_at(&right, 100);
        let (mut a, ca, mut b, cb) = detect_both(&left, &right);
        assert_eq!(ca[0].id, cb[0].id);
        assert_eq!(ca[0].sides[0].time_ms, ca[0].sides[1].time_ms);
        let wa = text_winner(&ca[0].sides);
        let wb = text_winner(&cb[0].sides);
        assert_eq!(ca[0].sides[wa].device, "device-9");
        assert_eq!(cb[0].sides[wb].device, "device-9");
        assert_eq!(a.blocks()[0].text, "From nine");
        assert_eq!(b.blocks()[0].text, "From nine");
    }

    #[test]
    fn text_winner_order_does_not_depend_on_side_order() {
        let side = |device: &str, time_ms: i64| {
            ConflictSide::new(device.into(), time_ms, None, SideState::Deleted, None)
        };
        let later = vec![side("a", 2), side("z", 1)];
        assert_eq!(text_winner(&later), 0);
        let tie = vec![side("device-b", 5), side("device-a", 5)];
        assert_eq!(text_winner(&tie), 0);
        let reversed = vec![side("device-a", 5), side("device-b", 5)];
        assert_eq!(text_winner(&reversed), 1);
    }

    #[test]
    fn identical_event_ids_are_independent_of_receiver_and_reimport_skips_application() {
        let base = setup();
        let mut left = branch(&base, 2);
        let mut right = branch(&base, 3);
        left.apply(&[text("root", "Left")]).unwrap();
        right.apply(&[text("root", "Right")]).unwrap();
        left.doc.commit();
        right.doc.commit();
        let mut a = left.fork_at(&left.doc.oplog_frontiers()).unwrap();
        let mut b = right.fork_at(&right.doc.oplog_frontiers()).unwrap();
        let ca = import(&mut a, &right, false);
        let cb = import(&mut b, &left, false);
        assert_eq!(ca[0].id, cb[0].id);
        crate::sync::write_conflict(&a, &ca[0]).unwrap();
        a.doc.commit();
        assert!(import(&mut a, &right, false).is_empty());
    }
}
