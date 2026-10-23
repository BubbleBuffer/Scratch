//! Journal day sections keep their established repair behavior and log each
//! restoration as one resolved `auto(journal)` conflict.
use std::collections::{HashMap, HashSet};

use loro::event::DiffBatch;
use loro::{
    Container, ContainerID, ContainerTrait, Frontiers, IdSpan, JsonChange, JsonMapOp, JsonOp,
    JsonOpContent, JsonSchema, JsonTreeOp, LoroDoc, LoroValue, ToJson, TreeID, TreeNode,
    TreeParentId, ValueOrContainer, VersionVector, ID,
};
use rusqlite::OptionalExtension;

use crate::doc::PageDoc;
use crate::error::Result;
use crate::index::Index;
use crate::types::{
    Anchor, Block, BlockKind, BlockOp, Conflict, ConflictScope, ConflictStatus, PropKind,
    PropValue, ResolutionBy, SideState,
};
use crate::vault::{lock, Vault};

const DAY_NOTES_REPAIRED_VERSION: u32 = 1;
const REPAIR_VERSION_KEY: &str = "day_notes_repaired_version";

pub(crate) fn is_day(db: &Index, doc: &PageDoc) -> Result<bool> {
    let meta = doc.meta();
    let Some(parent) = meta.parent else {
        return Ok(false);
    };
    if db.template_of(&parent)?.as_deref() == Some(meta.id.as_str()) {
        return Ok(false);
    }
    if meta.is_template
        || db
            .conn
            .query_row("SELECT role FROM pages WHERE id=?1", [&parent], |r| {
                r.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten()
            .as_deref()
            != Some("journal")
    {
        return Ok(false);
    }
    let Some(schema) = db.schema_of(&parent)? else {
        return Ok(false);
    };
    let Some(date) = schema.iter().find(|p| p.kind == PropKind::Date) else {
        return Ok(false);
    };
    Ok(doc.props().iter().any(|p| {
        p.key == date.key
            && matches!(p.value,
        PropValue::Date { start_ms, has_time: false, .. } if start_ms.rem_euclid(86_400_000) == 0)
    }))
}

fn vv_at(doc: &PageDoc, id: ID) -> Option<VersionVector> {
    doc.doc.frontiers_to_vv(&Frontiers::from(vec![id]))
}

fn before(doc: &PageDoc, id: ID) -> Result<Option<(VersionVector, PageDoc)>> {
    let Some(mut vv) = vv_at(doc, id) else {
        return Ok(None);
    };
    vv.set_end(id);
    let frontiers = doc.doc.vv_to_frontiers(&vv);
    if doc.doc.is_shallow()
        && doc
            .doc
            .shallow_since_frontiers()
            .iter()
            .any(|id| !vv.includes_id(id))
    {
        return Ok(None);
    }
    Ok(Some((vv, doc.fork_at(&frontiers)?)))
}

/// Latest independent editing views concurrent with the deletion. Walking
/// change boundaries also handles a stored merged snapshot with later commits.
fn editors(doc: &PageDoc, deletion: ID) -> Result<Vec<(u32, ID, VersionVector, PageDoc)>> {
    let Some(deleted_vv) = vv_at(doc, deletion) else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for (peer, end) in doc.doc.oplog_vv().iter() {
        let mut counter = *end - 1;
        while counter >= 0 {
            let id = ID::new(*peer, counter);
            let Some(change) = doc.doc.get_change(id) else {
                break;
            };
            let Some(vv) = vv_at(doc, id) else { break };
            if !vv.includes_id(deletion) {
                if !deleted_vv.includes_id(id) {
                    let frontier = Frontiers::from(vec![id]);
                    let fork = doc.fork_at(&frontier)?;
                    out.push((
                        change.lamport + (counter - change.id.counter) as u32,
                        id,
                        vv,
                        fork,
                    ));
                }
                break;
            }
            counter = change.id.counter - 1;
        }
    }
    out.sort_by_key(|(lamport, id, _, _)| (*lamport, id.peer, id.counter));
    out.reverse();
    Ok(out)
}

fn node_block(doc: &mut PageDoc, node: TreeID) -> Option<Block> {
    let tree = doc.doc.get_tree("blocks");
    let mut stack = doc.blocks();
    let id = match tree.get_meta(node).ok()?.get("id")? {
        ValueOrContainer::Value(LoroValue::String(id)) => id.to_string(),
        _ => return None,
    };
    while let Some(block) = stack.pop() {
        if block.id == id {
            return Some(block);
        }
        stack.extend(block.children);
    }
    None
}

/// Transient content lookup for one conflict side. A section deletion hides
/// many blocks at once; index their identities and fork its predecessor once,
/// rather than repeatedly decoding the same full historical page.
pub(crate) struct DeletedBlocks<'a> {
    source: &'a PageDoc,
    nodes: Option<HashMap<String, TreeID>>,
    predecessors: HashMap<ID, Option<PageDoc>>,
}

impl<'a> DeletedBlocks<'a> {
    pub(crate) fn new(source: &'a PageDoc) -> Self {
        Self {
            source,
            nodes: None,
            predecessors: HashMap::new(),
        }
    }

    /// Content just before this block or one of its ancestors disappeared.
    /// Only restoration uses this view; ordinary item conflicts are unchanged.
    #[cfg(test)]
    pub(crate) fn before_block_deletion(&mut self, id: &str) -> Result<Option<Block>> {
        let Some((target, deletion)) = self.before_block_deletion_node(id)? else {
            return Ok(None);
        };
        Ok(self
            .predecessors
            .get(&deletion)
            .and_then(Option::as_ref)
            .and_then(|prior| prior.block_at_tree_node(target)))
    }

    /// Position from that same cached last-visible view, including moves that
    /// happened after the common version and before the deletion.
    pub(crate) fn before_block_deletion_anchor(&mut self, id: &str) -> Result<Option<Anchor>> {
        let Some((target, deletion)) = self.before_block_deletion_node(id)? else {
            return Ok(None);
        };
        let prior = self
            .predecessors
            .get(&deletion)
            .and_then(Option::as_ref)
            .expect("loaded visible predecessor");
        let tree = prior.doc.get_tree("blocks");
        let parent = match tree.parent(target) {
            Some(TreeParentId::Node(parent)) => Some(parent),
            Some(TreeParentId::Root) => None,
            _ => return Ok(None),
        };
        let siblings = tree
            .children(parent.map_or(TreeParentId::Root, TreeParentId::Node))
            .unwrap_or_default();
        let previous = siblings
            .iter()
            .position(|node| *node == target)
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| logical_id(&tree, siblings[i]));
        Ok(Some(Anchor {
            parent: parent.and_then(|node| logical_id(&tree, node)),
            previous,
        }))
    }

    /// The same predecessor exposes historical page identity when the note was
    /// moved or retimed after its section was deleted.
    pub(crate) fn before_block_deletion_frame(&mut self, id: &str) -> Result<Option<&PageDoc>> {
        let Some((_, deletion)) = self.before_block_deletion_node(id)? else {
            return Ok(None);
        };
        Ok(self.predecessors.get(&deletion).and_then(Option::as_ref))
    }

    fn before_block_deletion_node(&mut self, id: &str) -> Result<Option<(TreeID, ID)>> {
        let nodes = self.nodes.get_or_insert_with(|| {
            let tree = self.source.doc.get_tree("blocks");
            let mut nodes = HashMap::new();
            for node in tree.nodes() {
                if let Some(id) = logical_id(&tree, node) {
                    nodes.entry(id).or_insert(node);
                }
            }
            nodes
        });
        let Some(&target) = nodes.get(id) else {
            return Ok(None);
        };
        let mut predecessor = None;
        loop {
            let view = match predecessor {
                None => self.source,
                Some(id) => self
                    .predecessors
                    .get(&id)
                    .and_then(Option::as_ref)
                    .expect("loaded predecessor"),
            };
            let tree = view.doc.get_tree("blocks");
            let mut node = target;
            let deletion = loop {
                match tree.parent(node) {
                    Some(TreeParentId::Node(parent)) => node = parent,
                    Some(TreeParentId::Deleted) => break tree.get_last_move_id(&node),
                    _ => return Ok(None),
                }
            };
            let Some(deletion) = deletion else {
                return Ok(None);
            };
            if !self.predecessors.contains_key(&deletion) {
                // Distinct item deletions can span many full-page views.
                // Bound temporary memory; section repair normally shares
                // only one or two predecessors across all its items.
                if self.predecessors.len() >= 8 {
                    self.predecessors.clear();
                }
                self.predecessors
                    .insert(deletion, before(self.source, deletion)?.map(|(_, doc)| doc));
            }
            let Some(prior) = self.predecessors.get(&deletion).and_then(Option::as_ref) else {
                return Ok(None);
            };
            if prior.block_at_tree_node(target).is_some() {
                return Ok(Some((target, deletion)));
            }
            predecessor = Some(deletion);
        }
    }
}

fn section(doc: &mut PageDoc, node: TreeID) -> Option<Block> {
    if doc.doc.get_tree("blocks").parent(node) != Some(TreeParentId::Root) {
        return None;
    }
    let block = node_block(doc, node)?;
    (block.kind == BlockKind::Toggle && matches!(block.text.as_str(), "Tasks" | "Log"))
        .then_some(block)
}

fn descendants(doc: &PageDoc, root: TreeID) -> Vec<TreeID> {
    let tree = doc.doc.get_tree("blocks");
    let mut nodes = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        nodes.push(node);
        let mut children = tree.children(node).unwrap_or_default();
        children.reverse();
        stack.extend(children);
    }
    nodes
}

fn block_ids(doc: &PageDoc, nodes: &[TreeID]) -> HashSet<String> {
    let tree = doc.doc.get_tree("blocks");
    nodes
        .iter()
        .filter_map(|node| match tree.get_meta(*node).ok()?.get("id")? {
            ValueOrContainer::Value(LoroValue::String(id)) => Some(id.to_string()),
            _ => None,
        })
        .collect()
}

/// The high-level move API rejects a directly deleted TreeID. Import one
/// ordinary move under this document's actual peer/counter and causal context,
/// retaining the physical node identity even when two devices restore it.
pub(crate) fn move_nodes(doc: &LoroDoc, nodes: &[TreeNode]) -> Result<()> {
    if nodes.is_empty() {
        return Ok(());
    }
    doc.commit();
    let frontiers = doc.oplog_frontiers();
    let timestamp = frontiers
        .iter()
        .filter_map(|id| doc.get_change(id).map(|c| c.timestamp))
        .max()
        .unwrap_or(0);
    let lamport = frontiers
        .iter()
        .filter_map(|id| {
            let change = doc.get_change(id)?;
            Some(change.lamport + (id.counter - change.id.counter) as u32 + 1)
        })
        .max()
        .unwrap_or(0);
    let peer = doc.peer_id();
    let counter = doc.oplog_vv().get(&peer).copied().unwrap_or(0);
    let tree = doc.get_tree("blocks");
    let ops = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| JsonOp {
            counter: counter + i as i32,
            container: tree.id(),
            content: JsonOpContent::Tree(JsonTreeOp::Move {
                target: node.id,
                parent: match node.parent {
                    TreeParentId::Root => None,
                    TreeParentId::Node(parent) => Some(parent),
                    _ => unreachable!("visible source node"),
                },
                fractional_index: node.fractional_index.clone(),
            }),
        })
        .collect();
    doc.import_json_updates(JsonSchema {
        schema_version: 1,
        start_version: frontiers.clone(),
        peers: None,
        changes: vec![JsonChange {
            id: ID::new(peer, counter),
            timestamp,
            deps: frontiers.iter().collect(),
            lamport,
            msg: None,
            ops,
        }],
    })?;
    Ok(())
}

fn content(doc: &PageDoc) -> serde_json::Value {
    let mut value = doc.doc.get_deep_value().to_json_value();
    if let Some(root) = value.as_object_mut() {
        root.remove("peers");
        root.remove("conflicts");
        root.remove("conflict-history");
        root.remove("conflict-lifecycle");
        if let Some(page) = root.get_mut("page").and_then(|p| p.as_object_mut()) {
            page.remove("updated");
            page.remove("trashed");
        }
    }
    value
}

fn trash_id(doc: &PageDoc) -> Option<ID> {
    let map = doc.doc.get_map("page");
    let peer = map.get_last_editor("trashed")?;
    let end = doc.doc.oplog_vv().get(&peer).copied()?;
    for change in doc
        .doc
        .export_json_in_id_span(IdSpan::new(peer, 0, end))
        .iter()
        .rev()
    {
        for op in change.ops.iter().rev() {
            if op.container == map.id()
                && matches!(&op.content,
                JsonOpContent::Map(JsonMapOp::Insert { key, value: LoroValue::I64(_) }) if key == "trashed")
            {
                return Some(ID::new(peer, op.counter));
            }
        }
    }
    None
}

fn containers(doc: &PageDoc, nodes: &[TreeID]) -> HashSet<ContainerID> {
    let tree = doc.doc.get_tree("blocks");
    let mut out = HashSet::new();
    for node in nodes {
        out.insert(node.associated_meta_container());
        if let Ok(meta) = tree.get_meta(*node) {
            if let Some(ValueOrContainer::Container(Container::Text(text))) = meta.get("text") {
                out.insert(text.id());
            }
        }
    }
    out
}

fn restore(
    doc: &mut PageDoc,
    source: &PageDoc,
    root: Option<TreeID>,
    base: &PageDoc,
) -> Result<HashSet<String>> {
    let tree = doc.doc.get_tree("blocks");
    let source_tree = source.doc.get_tree("blocks");
    let source_nodes: Vec<_> = if let Some(root) = root {
        descendants(source, root)
    } else {
        source_tree.get_nodes(false).iter().map(|n| n.id).collect()
    };
    let mut scope: HashSet<_> = source_nodes.iter().copied().collect();
    if let Some(root) = root {
        scope.extend(descendants(doc, root));
        scope.extend(descendants(base, root));
    } else {
        scope.extend(tree.nodes());
    }
    let mut desired: Vec<_> = source_tree
        .get_nodes(false)
        .into_iter()
        .filter(|n| scope.contains(&n.id))
        .collect();
    // Retain shared physical children at their visible positions while
    // applying offline content edits, regardless of which heading survives.
    let mut visible_sections = Vec::new();
    if let Some(root) = root {
        let mut source_view = source.fork_at(&source.doc.oplog_frontiers())?;
        if let Some(restored) = section(&mut source_view, root) {
            visible_sections = tree
                .roots()
                .into_iter()
                .filter_map(|node| {
                    section(doc, node)
                        .filter(|b| b.text == restored.text)
                        .map(|b| (b.id, node))
                })
                .collect();
            visible_sections.sort_by(|a, b| a.0.cmp(&b.0));
            let retained: HashSet<_> = visible_sections
                .iter()
                .flat_map(|(_, node)| descendants(doc, *node))
                .collect();
            desired.retain(|n| !retained.contains(&n.id));
        }
    }
    // Raw imports can apply part of a batch before failing, just like the
    // ordinary PageDoc import path. Never retain its old logical-id cache.
    doc.invalidate_block_ids();
    move_nodes(&doc.doc, &desired)?;
    let desired_ids: HashSet<_> = source_tree
        .get_nodes(false)
        .into_iter()
        .filter(|n| scope.contains(&n.id))
        .map(|n| n.id)
        .collect();
    for node in &scope {
        if !desired_ids.contains(node) && !tree.is_node_deleted(node).unwrap_or(true) {
            tree.delete(*node)?;
        }
    }
    doc.doc.commit();
    // Restore all page-facing fields from the editing view. For a section,
    // constrain the diff to its physical subtree and metadata/text containers.
    let diff = doc
        .doc
        .diff(&doc.doc.oplog_frontiers(), &source.doc.oplog_frontiers())?;
    let mut filtered = DiffBatch::default();
    let allowed = containers(source, &desired_ids.iter().copied().collect::<Vec<_>>());
    for (cid, diff) in diff.iter() {
        // A backwards tree diff can include delete/recreate intermediate
        // history. The explicit same-ID moves above already set its final
        // structure; replaying that history would allocate alias TreeIDs.
        if cid == &tree.id() {
            continue;
        }
        if root.is_none() {
            if cid != &doc.doc.get_map("peers").id() && cid != &doc.doc.get_map("conflicts").id() {
                filtered
                    .push(cid.clone(), diff.clone())
                    .expect("unique diff container");
            }
        } else if allowed.contains(cid) {
            filtered
                .push(cid.clone(), diff.clone())
                .expect("unique diff container");
        }
    }
    doc.doc.apply_diff(filtered)?;
    doc.invalidate_block_ids();
    if let (Some(root), Some((_, visible))) = (root, visible_sections.first()) {
        // Assemble the item order under the visible heading first. The final
        // consolidation can then retain either heading without changing it.
        for (_, extra) in &visible_sections[1..] {
            append_section_children(doc, *visible, *extra)?;
        }
        append_section_children(doc, *visible, root)?;
    }
    let mut ids = block_ids(source, &source_nodes);
    ids.extend(block_ids(doc, &scope.into_iter().collect::<Vec<_>>()));
    Ok(ids)
}

fn logical_id(tree: &loro::LoroTree, node: TreeID) -> Option<String> {
    match tree.get_meta(node).ok()?.get("id")? {
        ValueOrContainer::Value(LoroValue::String(id)) => Some(id.to_string()),
        _ => None,
    }
}

/// Append extra items in their original order, retaining a shared logical
/// item's keeper position and any children contributed by either side.
fn merge_children(tree: &loro::LoroTree, keeper: TreeID, extra: TreeID) -> Result<()> {
    let mut kept = tree.children(keeper).unwrap_or_default();
    for child in tree.children(extra).unwrap_or_default() {
        let id = logical_id(tree, child);
        let duplicate = kept
            .iter()
            .copied()
            .find(|n| id.is_some() && logical_id(tree, *n) == id);
        if let Some(existing) = duplicate {
            merge_children(tree, existing, child)?;
            tree.delete(child)?;
        } else {
            tree.mov_to(child, keeper, kept.len())?;
            kept.push(child);
        }
    }
    Ok(())
}

/// Normal sections use logical BlockOp moves. Only physical aliases need
/// tree-level addressing, since a BlockOp cannot distinguish their nodes.
fn append_section_children(doc: &mut PageDoc, keeper: TreeID, extra: TreeID) -> Result<bool> {
    let tree = doc.doc.get_tree("blocks");
    let mut ids = HashSet::new();
    let aliases = descendants(doc, keeper)
        .into_iter()
        .chain(descendants(doc, extra))
        .filter_map(|n| logical_id(&tree, n))
        .any(|id| !ids.insert(id));
    if aliases {
        merge_children(&tree, keeper, extra)?;
        doc.invalidate_block_ids();
    } else if let (Some(keeper), Some(extra)) = (node_block(doc, keeper), node_block(doc, extra)) {
        let mut after = keeper.children.last().map(|b| b.id.clone());
        let mut ops = Vec::new();
        for child in extra.children {
            ops.push(BlockOp::Move {
                id: child.id.clone(),
                parent: Some(keeper.id.clone()),
                after,
            });
            after = Some(child.id);
        }
        doc.apply(&ops)?;
    }
    Ok(aliases)
}

/// Ordinary pages can retain many deleted nodes after markdown rewrites.
/// Inspect scalar parent/date history once before any historical fork.
fn was_journal_child(db: &Index, doc: &PageDoc) -> Result<bool> {
    let meta = doc.meta();
    if meta.is_template {
        return Ok(false);
    }
    if let Some(parent) = &meta.parent {
        if db.template_of(parent)?.as_deref() == Some(meta.id.as_str()) {
            return Ok(false);
        }
    }
    let mut stmt = db
        .conn
        .prepare_cached("SELECT id FROM pages WHERE role='journal'")?;
    let journals: HashSet<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if journals.is_empty() {
        return Ok(false);
    }
    let mut dates = HashSet::new();
    for journal in &journals {
        if let Some(schema) = db.schema_of(journal)? {
            if let Some(date) = schema.iter().find(|p| p.kind == PropKind::Date) {
                dates.insert(date.key.clone());
            }
        }
    }
    let page = doc.doc.get_map("page").id();
    let props = doc.doc.get_map("props").id();
    let mut saw_parent = false;
    let mut saw_date = false;
    for (peer, end) in doc.doc.oplog_vv().iter() {
        for change in doc.doc.export_json_in_id_span(IdSpan::new(*peer, 0, *end)) {
            for op in change.ops {
                if let JsonOpContent::Map(JsonMapOp::Insert {
                    key,
                    value: LoroValue::String(value),
                }) = op.content
                {
                    if op.container == page && key == "parent" && journals.contains(value.as_str())
                    {
                        saw_parent = true;
                    }
                    if op.container == props
                        && dates.contains(&key)
                        && matches!(
                            serde_json::from_str::<PropValue>(value.as_str()),
                            Ok(PropValue::Date { start_ms, has_time: false, .. }) if start_ms.rem_euclid(86_400_000) == 0
                        )
                    {
                        saw_date = true;
                    }
                    if saw_parent && saw_date {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

fn is_delete_edit_record(conflict: &Conflict) -> bool {
    conflict.format >= 2
        && conflict
            .sides
            .iter()
            .any(|side| matches!(side.state, SideState::Present { .. }))
        && conflict.sides.iter().any(|side| {
            matches!(
                side.state,
                SideState::Deleted | SideState::Trashed { .. } | SideState::Purged { .. }
            )
        })
        && conflict.applied.iter().any(|application| {
            matches!(&application.by,
            ResolutionBy::Auto { rule } if rule == crate::conflict_detect::EDIT_RULE)
        })
}

/// Parent/schema metadata can arrive after the generic rule restored a page or
/// section. Promote that existing record; don't create a second repair entry,
/// and never classify an ordinary two-present text conflict as journal repair.
fn promote_journal_records(db: &Index, doc: &PageDoc) -> Result<()> {
    let page = doc.meta().id;
    for (_, json) in doc.conflicts() {
        let Some(mut conflict) = crate::sync::read_conflict(&page, &json) else {
            continue;
        };
        if !is_delete_edit_record(&conflict) {
            continue;
        }
        if !matches!(
            conflict.scope,
            ConflictScope::Page | ConflictScope::Subtree { .. }
        ) {
            continue;
        }
        let mut eligible = false;
        for side in &conflict.sides {
            let Some(bytes) = &side.frontiers else {
                continue;
            };
            let Ok(frontiers) = Frontiers::decode(bytes) else {
                continue;
            };
            let Ok(mut historical) = doc.fork_at(&frontiers) else {
                continue;
            };
            let identified = is_day(db, &historical)?;
            match &conflict.scope {
                ConflictScope::Page => {
                    if identified {
                        eligible = true;
                        break;
                    }
                    if let Some(deletion) = trash_id(&historical) {
                        if let Some((_, prior)) = before(&historical, deletion)? {
                            if is_day(db, &prior)? {
                                eligible = true;
                                break;
                            }
                        }
                    }
                }
                ConflictScope::Subtree { root } => {
                    let tree = historical.doc.get_tree("blocks");
                    let node = tree
                        .nodes()
                        .into_iter()
                        .find(|node| logical_id(&tree, *node).as_deref() == Some(root.as_str()));
                    if identified
                        && node.is_some_and(|node| section(&mut historical, node).is_some())
                    {
                        eligible = true;
                        break;
                    }
                    let mut deleted = DeletedBlocks::new(&historical);
                    if let Some(prior) = deleted.before_block_deletion_frame(root)? {
                        if is_day(db, prior)? {
                            let tree = prior.doc.get_tree("blocks");
                            let root_node = tree.nodes().into_iter().find(|node| {
                                logical_id(&tree, *node).as_deref() == Some(root.as_str())
                            });
                            if root_node.is_some_and(|node| {
                                tree.parent(node) == Some(TreeParentId::Root)
                                    && prior.block_at_tree_node(node).is_some_and(|block| {
                                        block.kind == BlockKind::Toggle
                                            && matches!(block.text.as_str(), "Tasks" | "Log")
                                    })
                            }) {
                                eligible = true;
                                break;
                            }
                        }
                    }
                }
                ConflictScope::Block { .. } => {}
            }
        }
        if !eligible {
            continue;
        }
        for application in &mut conflict.applied {
            if matches!(&application.by, ResolutionBy::Auto { rule } if rule == crate::conflict_detect::EDIT_RULE)
            {
                application.by = ResolutionBy::Auto {
                    rule: crate::conflict_detect::JOURNAL_RULE.into(),
                };
            }
        }
        if let Some(latest) = conflict.applied.last().cloned() {
            conflict.status = ConflictStatus::Resolved(latest);
            crate::sync::write_conflict(doc, &conflict)?;
        }
    }
    Ok(())
}

fn log_repair(doc: &PageDoc, conflict: Conflict) -> Result<()> {
    // Preserve the first application's before-version if the same causal repair
    // was already logged on this or another device.
    if !doc.conflicts().iter().any(|(id, _)| id == &conflict.id) {
        crate::sync::write_conflict(doc, &conflict)?;
    }
    Ok(())
}

pub(crate) fn normalize(db: &Index, doc: &mut PageDoc) -> Result<HashSet<String>> {
    doc.doc.commit();
    promote_journal_records(db, doc)?;
    let mut recovered = HashSet::new();
    if doc.meta().trashed.is_some() {
        if let Some(deletion) = trash_id(doc) {
            if let Some((deleted_vv, original)) = before(doc, deletion)? {
                if is_day(db, &original)? {
                    for (_, _, vv, editor) in editors(doc, deletion)? {
                        let base =
                            doc.fork_at(&doc.doc.vv_to_frontiers(&vv.intersection(&deleted_vv)))?;
                        if editor.meta().trashed.is_none() && content(&editor) != content(&base) {
                            doc.doc.commit();
                            let before_frontiers = doc.version();
                            let before_state = crate::conflict_detect::before_state(doc);
                            let deletion_frame = doc.fork_at(&Frontiers::from(vec![deletion]))?;
                            recovered.extend(restore(doc, &editor, None, &base)?);
                            let conflict = crate::conflict_detect::journal_record(
                                doc,
                                ConflictScope::Page,
                                &deletion_frame,
                                &editor,
                                before_frontiers,
                                before_state,
                            )?;
                            log_repair(doc, conflict)?;
                            break;
                        }
                    }
                }
            }
        }
    }
    // Deletion-version identity also covers an editor who moved/retimed the
    // note. Ordinary seed consolidation only applies to its current identity.
    let current_day = is_day(db, doc)?;
    if doc.meta().trashed.is_some() {
        return Ok(recovered);
    }
    if !current_day && !was_journal_child(db, doc)? {
        return Ok(recovered);
    }
    let tree = doc.doc.get_tree("blocks");
    let mut deleted: Vec<_> = tree
        .nodes()
        .into_iter()
        .filter(|n| tree.parent(*n) == Some(TreeParentId::Deleted))
        .collect();
    deleted.sort_by_key(|n| (n.peer, n.counter));
    for node in deleted {
        let Some(deletion) = tree.get_last_move_id(&node) else {
            continue;
        };
        let Some((deleted_vv, mut original)) = before(doc, deletion)? else {
            continue;
        };
        if !is_day(db, &original)? || section(&mut original, node).is_none() {
            continue;
        }
        for (_, _, vv, mut editor) in editors(doc, deletion)? {
            if editor.doc.get_tree("blocks").parent(node) != Some(TreeParentId::Root) {
                continue;
            }
            let Some(edited) = node_block(&mut editor, node) else {
                continue;
            };
            let mut base = doc.fork_at(&doc.doc.vv_to_frontiers(&vv.intersection(&deleted_vv)))?;
            if section(&mut base, node).as_ref() != Some(&edited) {
                doc.doc.commit();
                let before_frontiers = doc.version();
                let before_state = crate::conflict_detect::before_state(doc);
                let deletion_frame = doc.fork_at(&Frontiers::from(vec![deletion]))?;
                recovered.extend(restore(doc, &editor, Some(node), &base)?);
                let conflict = crate::conflict_detect::journal_record(
                    doc,
                    ConflictScope::Subtree {
                        root: edited.id.clone(),
                    },
                    &deletion_frame,
                    &editor,
                    before_frontiers,
                    before_state,
                )?;
                log_repair(doc, conflict)?;
                break;
            }
        }
    }
    if !current_day {
        return Ok(recovered);
    }
    let mut changed = false;
    for name in ["Tasks", "Log"] {
        let mut sections: Vec<_> = tree
            .roots()
            .into_iter()
            .filter_map(|node| {
                section(doc, node)
                    .filter(|b| b.text == name)
                    .map(|b| (b.id, node))
            })
            .collect();
        sections.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| (a.1.peer, a.1.counter).cmp(&(b.1.peer, b.1.counter)))
        });
        if let Some((_, keeper)) = sections.first() {
            for (_, extra) in &sections[1..] {
                if append_section_children(doc, *keeper, *extra)? {
                    tree.delete(*extra)?;
                    doc.invalidate_block_ids();
                } else if let Some(id) = logical_id(&tree, *extra) {
                    doc.apply(&[BlockOp::Delete { id }])?;
                }
                changed = true;
            }
        }
    }
    if changed {
        doc.invalidate_block_ids();
    }
    Ok(recovered)
}

impl Vault {
    /// The legacy startup sweep runs once per repair version on this copy.
    /// Losing the local index marker is harmless: the sweep is idempotent.
    pub(crate) fn repair_day_notes_on_open(&self) -> Result<()> {
        let current = || -> Result<bool> {
            Ok(lock(&self.inner.db)
                .get_meta(REPAIR_VERSION_KEY)?
                .and_then(|value| value.parse::<u32>().ok())
                .is_some_and(|version| version >= DAY_NOTES_REPAIRED_VERSION))
        };
        if current()? {
            return Ok(());
        }
        let _local = lock(&self.inner.daily_notes);
        let _copy = self.inner.daily_notes_lock()?;
        if current()? {
            return Ok(());
        }
        self.repair_day_notes()?;
        // Snapshots are durably written by flush before the marker can make
        // a later open skip them. An error leaves the old/missing marker.
        self.flush()?;
        lock(&self.inner.db).set_meta(REPAIR_VERSION_KEY, &DAY_NOTES_REPAIRED_VERSION.to_string())
    }

    pub(crate) fn repair_day_note(&self, id: &str) -> Result<()> {
        if self.inner.is_purged(id) {
            return Ok(());
        }
        let db = lock(&self.inner.db);
        let eligible: bool = db.conn.query_row(
            "SELECT p.is_template=0 AND (p.in_trash=1 OR parent.role='journal') FROM pages p LEFT JOIN pages parent ON parent.id=p.parent WHERE p.id=?1",
            [id], |r| r.get::<_, Option<bool>>(0),
        ).optional()?.flatten().unwrap_or(false);
        if !eligible {
            return Ok(());
        }
        let changed = self.inner.edit(id, |doc| {
            let vv = doc.doc.oplog_vv();
            normalize(&db, doc)?;
            doc.doc.commit();
            Ok(doc.doc.oplog_vv() != vv)
        })?;
        if changed {
            self.inner.sync_meta(&db, id)?;
            let records = self.inner.read(id, |doc| doc.conflicts())?;
            db.set_conflicts(id, &crate::sync::mirror_rows(id, &records))?;
        }
        Ok(())
    }

    pub(crate) fn repair_day_notes(&self) -> Result<()> {
        let ids: Vec<String> = {
            let db = lock(&self.inner.db);
            let mut stmt = db.conn.prepare_cached("SELECT p.id FROM pages p LEFT JOIN pages parent ON parent.id=p.parent WHERE p.is_template=0 AND (parent.role='journal' OR p.in_trash=1)")?;
            let ids = stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids
        };
        for id in ids {
            self.repair_day_note(&id)?;
        }
        Ok(())
    }

    pub(crate) fn repair_imported_day_notes(&self, updated: &[String]) -> Result<()> {
        let mut ids: HashSet<String> = updated.iter().cloned().collect();
        {
            let db = lock(&self.inner.db);
            for parent in updated {
                let role = db
                    .conn
                    .query_row("SELECT role FROM pages WHERE id=?1", [parent], |r| {
                        r.get::<_, Option<String>>(0)
                    })
                    .optional()?
                    .flatten();
                if role.as_deref() == Some("journal") {
                    ids.extend(db.child_ids(parent)?);
                }
            }
        }
        for id in ids {
            self.repair_day_note(&id)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NewBlock;
    use std::time::Instant;

    fn insert(
        parent: Option<&str>,
        after: Option<&str>,
        id: &str,
        kind: BlockKind,
        text: &str,
    ) -> BlockOp {
        BlockOp::Insert {
            parent: parent.map(str::to_owned),
            after: after.map(str::to_owned),
            block: NewBlock {
                id: id.into(),
                kind,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }
    }

    fn journal_index(known: bool) -> (Index, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("vault-journal-conflict-{}", crate::ids::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let (db, _) = Index::open(&root.join("index.sqlite")).unwrap();
        let schema = serde_json::to_string(&vec![crate::types::PropDef {
            key: "date".into(),
            name: "Date".into(),
            kind: PropKind::Date,
            options: Vec::new(),
            config: String::new(),
        }])
        .unwrap();
        db.conn.execute("INSERT INTO pages(id,title,created_ms,updated_ms,page_type,role,schema) VALUES('journal','Journal',0,0,'database',?1,?2)",
            rusqlite::params![known.then_some("journal"), schema]).unwrap();
        (db, root)
    }

    fn journal_page() -> PageDoc {
        let mut doc = PageDoc::new_page(1, "day", "Day", Some("journal"), 0).unwrap();
        doc.set_prop(
            "date",
            &PropValue::Date {
                start_ms: 0,
                end_ms: None,
                has_time: false,
            },
        )
        .unwrap();
        doc.apply(&[
            insert(None, None, "tasks", BlockKind::Toggle, "Tasks"),
            insert(None, Some("tasks"), "log", BlockKind::Toggle, "Log"),
            insert(Some("tasks"), None, "old", BlockKind::Paragraph, "Before"),
            insert(
                Some("tasks"),
                Some("old"),
                "untouched",
                BlockKind::Paragraph,
                "Untouched",
            ),
        ])
        .unwrap();
        doc.doc.commit();
        doc
    }

    fn edit_text(id: &str, text: &str) -> BlockOp {
        BlockOp::SetText {
            id: id.into(),
            text: text.into(),
            marks: Vec::new(),
        }
    }

    fn repair_pair(common: &PageDoc) -> (PageDoc, PageDoc) {
        let mut deleted = common.fork_at(&common.doc.oplog_frontiers()).unwrap();
        let mut editing = common.fork_at(&common.doc.oplog_frontiers()).unwrap();
        deleted.set_peer(2).unwrap();
        editing.set_peer(3).unwrap();
        deleted.register_peer("deleting-device").unwrap();
        editing.register_peer("editing-device").unwrap();
        (deleted, editing)
    }

    fn records(doc: &PageDoc) -> Vec<Conflict> {
        let page = doc.meta().id;
        doc.conflicts()
            .into_iter()
            .filter_map(|(_, json)| crate::sync::read_conflict(&page, &json))
            .collect()
    }

    fn assert_one_journal_log(doc: &PageDoc) -> Conflict {
        let records = records(doc);
        assert_eq!(
            records
                .iter()
                .filter(|record| record.status == ConflictStatus::Pending)
                .count(),
            0
        );
        assert_eq!(records.len(), 1);
        let record = records.into_iter().next().unwrap();
        let latest = record.applied.last().unwrap();
        assert_eq!(
            latest.by,
            ResolutionBy::Auto {
                rule: "journal".into()
            }
        );
        assert_eq!(record.status, ConflictStatus::Resolved(latest.clone()));
        assert!(!latest.before.is_empty());
        // The journal applies the editing side, which is side 1.
        assert_eq!(
            latest.resolution,
            Some(crate::types::ConflictResolution::UseSide(1))
        );
        record
    }

    #[test]
    fn journal_section_repair_logs_once_in_both_import_directions_and_keeps_identity() {
        for editing_receives in [false, true] {
            let (db, temporary) = journal_index(true);
            let common = journal_page();
            let mut original = common.doc.get_tree("blocks").nodes();
            original.sort_by_key(|node| (node.peer, node.counter));
            let (mut deleted, mut editing) = repair_pair(&common);
            deleted
                .apply(&[BlockOp::Delete { id: "tasks".into() }])
                .unwrap();
            editing.apply(&[edit_text("old", "Offline edit")]).unwrap();
            deleted.doc.commit();
            editing.doc.commit();
            let (mut merged, incoming) = if editing_receives {
                (editing, deleted)
            } else {
                (deleted, editing)
            };
            let local_vv = merged.doc.oplog_vv();
            let local_frontiers = merged.doc.oplog_frontiers();
            merged
                .import_updates(&incoming.snapshot().unwrap())
                .unwrap();
            let raw_version = merged.doc.oplog_vv();
            let before = merged.version();
            let recovered = normalize(&db, &mut merged).unwrap();
            let record = assert_one_journal_log(&merged);
            assert_eq!(
                record.scope,
                ConflictScope::Subtree {
                    root: "tasks".into()
                }
            );
            assert_eq!(record.applied[0].before, before);
            assert_eq!(merged.blocks()[0].children[0].text, "Offline edit");
            let mut physical = merged.doc.get_tree("blocks").nodes();
            physical.sort_by_key(|node| (node.peer, node.counter));
            assert_eq!(physical, original);
            assert!(crate::conflict_detect::detect(
                &mut merged,
                &local_vv,
                &local_frontiers,
                &raw_version,
                &recovered,
                false
            )
            .unwrap()
            .is_empty());
            let logged_id = record.id;
            let version = merged.doc.oplog_vv();
            normalize(&db, &mut merged).unwrap();
            merged.doc.commit();
            assert_eq!(records(&merged).len(), 1);
            assert_eq!(records(&merged)[0].id, logged_id);
            // Commit the initial log before comparing subsequent no-op passes.
            let current = merged.doc.oplog_vv();
            normalize(&db, &mut merged).unwrap();
            merged.doc.commit();
            assert_eq!(merged.doc.oplog_vv(), current);
            assert!(current.includes_vv(&version));
            drop(db);
            std::fs::remove_dir_all(temporary).unwrap();
        }
    }

    #[test]
    fn journal_trash_repair_logs_page_once_with_prior_trash_state() {
        let (db, temporary) = journal_index(true);
        let common = journal_page();
        let (mut deleted, mut editing) = repair_pair(&common);
        deleted.set_meta_i64("trashed", Some(42)).unwrap();
        editing
            .apply(&[edit_text("old", "Offline page edit")])
            .unwrap();
        deleted.doc.commit();
        editing.doc.commit();
        deleted
            .import_updates(&editing.snapshot().unwrap())
            .unwrap();
        let before = deleted.version();
        normalize(&db, &mut deleted).unwrap();
        let record = assert_one_journal_log(&deleted);
        assert_eq!(record.scope, ConflictScope::Page);
        assert_eq!(record.applied[0].before, before);
        assert_eq!(
            record.applied[0].before_state,
            Some(crate::types::BeforeState::Trashed { at: 42 })
        );
        assert_eq!(deleted.meta().trashed, None);
        drop(db);
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn later_journal_identity_promotes_the_existing_rule_record_without_text_text_promotion() {
        let (db, temporary) = journal_index(false);
        let common = journal_page();
        let (mut deleted, mut editing) = repair_pair(&common);
        deleted
            .apply(&[BlockOp::Delete { id: "tasks".into() }])
            .unwrap();
        editing.apply(&[edit_text("old", "Offline edit")]).unwrap();
        deleted.doc.commit();
        editing.doc.commit();
        let local_vv = deleted.doc.oplog_vv();
        let local_frontiers = deleted.doc.oplog_frontiers();
        deleted
            .import_updates(&editing.snapshot().unwrap())
            .unwrap();
        let raw = deleted.doc.oplog_vv();
        let found = crate::conflict_detect::detect(
            &mut deleted,
            &local_vv,
            &local_frontiers,
            &raw,
            &HashSet::new(),
            false,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, ConflictStatus::Pending);
        crate::sync::write_conflict(&deleted, &found[0]).unwrap();
        deleted.doc.commit();
        let id = found[0].id.clone();
        let before = found[0].applied[0].before.clone();
        db.conn
            .execute("UPDATE pages SET role='journal' WHERE id='journal'", [])
            .unwrap();
        normalize(&db, &mut deleted).unwrap();
        let promoted = assert_one_journal_log(&deleted);
        assert_eq!(promoted.id, id);
        assert_eq!(promoted.applied[0].before, before);
        let (mut left, mut right) = repair_pair(&common);
        left.apply(&[edit_text("old", "Left")]).unwrap();
        right.apply(&[edit_text("old", "Right")]).unwrap();
        left.doc.commit();
        right.doc.commit();
        let vv = left.doc.oplog_vv();
        let frontier = left.doc.oplog_frontiers();
        left.import_updates(&right.snapshot().unwrap()).unwrap();
        let after = left.doc.oplog_vv();
        let text_conflict = crate::conflict_detect::detect(
            &mut left,
            &vv,
            &frontier,
            &after,
            &HashSet::new(),
            false,
        )
        .unwrap();
        assert_eq!(text_conflict.len(), 1);
        crate::sync::write_conflict(&left, &text_conflict[0]).unwrap();
        normalize(&db, &mut left).unwrap();
        assert_eq!(records(&left)[0].status, ConflictStatus::Pending);
        drop(db);
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn restore_keeps_a_nondeleted_desired_node_that_does_not_need_a_move() {
        let mut doc = PageDoc::new_page(1, "day", "Day", None, 0).unwrap();
        doc.apply(&[
            insert(None, None, "a", BlockKind::Toggle, "Tasks"),
            insert(None, Some("a"), "z", BlockKind::Toggle, "Tasks"),
            insert(
                Some("z"),
                None,
                "shared",
                BlockKind::Paragraph,
                "Unchanged shared content",
            ),
        ])
        .unwrap();
        doc.doc.commit();
        let source = doc.fork_at(&doc.doc.oplog_frontiers()).unwrap();
        let tree = doc.doc.get_tree("blocks");
        let node = tree
            .nodes()
            .into_iter()
            .find(|n| logical_id(&tree, *n).as_deref() == Some("shared"))
            .unwrap();
        let root = tree
            .nodes()
            .into_iter()
            .find(|n| logical_id(&tree, *n).as_deref() == Some("z"))
            .unwrap();
        doc.apply(&[
            BlockOp::Move {
                id: "shared".into(),
                parent: Some("a".into()),
                after: None,
            },
            BlockOp::Delete { id: "z".into() },
        ])
        .unwrap();
        doc.doc.commit();
        let position = tree.get_last_move_id(&node);
        let parent = tree.parent(node);
        let before = node_block(&mut doc, node).unwrap();
        let recovered = restore(&mut doc, &source, Some(root), &source).unwrap();
        assert!(recovered.contains("shared"));
        assert!(!tree.is_node_deleted(&node).unwrap());
        assert_eq!(tree.parent(node), parent);
        assert_eq!(tree.get_last_move_id(&node), position);
        assert_eq!(node_block(&mut doc, node), Some(before));
        assert_eq!(tree.children(root).unwrap(), vec![]);
    }

    #[test]
    fn deleted_content_follows_and_reuses_nested_deletion_predecessors() {
        let mut doc = PageDoc::new_page(1, "nested", "Nested", None, 0).unwrap();
        doc.apply(&[
            insert(None, None, "root", BlockKind::Toggle, "Tasks"),
            insert(
                Some("root"),
                None,
                "child",
                BlockKind::Toggle,
                "Edited parent",
            ),
            insert(
                Some("child"),
                None,
                "first",
                BlockKind::Paragraph,
                "Edited first side",
            ),
            insert(
                Some("child"),
                Some("first"),
                "second",
                BlockKind::Paragraph,
                "Edited second side",
            ),
        ])
        .unwrap();
        doc.doc.commit();
        let tree = doc.doc.get_tree("blocks");
        let root = tree.roots()[0];
        let child = tree.children(root).unwrap()[0];
        tree.delete(root).unwrap();
        doc.doc.commit();
        tree.delete(child).unwrap();
        doc.doc.commit();
        let mut lookup = DeletedBlocks::new(&doc);
        for _ in 0..2 {
            assert_eq!(
                lookup.before_block_deletion("first").unwrap().unwrap().text,
                "Edited first side"
            );
            assert_eq!(
                lookup
                    .before_block_deletion("second")
                    .unwrap()
                    .unwrap()
                    .text,
                "Edited second side"
            );
        }
        assert_eq!(lookup.predecessors.len(), 2);
    }

    #[test]
    #[ignore = "manual production-profile benchmark; run --release --ignored --nocapture"]
    fn bench_before_block_deletion_5000_blocks_300_restored_ids() {
        const BLOCKS: usize = 5000;
        const CALLS: usize = 300;
        let mut common = PageDoc::new_page(1, "bench-day", "Day", None, 0).unwrap();
        let mut ops = vec![insert(None, None, "section", BlockKind::Toggle, "Tasks")];
        for i in 0..BLOCKS - 1 {
            ops.push(insert(
                Some("section"),
                (i > 0).then(|| format!("item-{}", i - 1)).as_deref(),
                &format!("item-{i}"),
                BlockKind::Paragraph,
                &format!("Original content for item {i}"),
            ));
        }
        common.apply(&ops).unwrap();
        common.doc.commit();
        let base = common.doc.oplog_frontiers();
        let mut editor = common.fork_at(&base).unwrap();
        editor.set_peer(2).unwrap();
        editor
            .apply(
                &(0..CALLS)
                    .map(|i| BlockOp::SetText {
                        id: format!("item-{i}"),
                        text: format!("Offline edit {i}"),
                        marks: vec![],
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        editor.doc.commit();
        let mut deleted = common.fork_at(&base).unwrap();
        deleted.set_peer(3).unwrap();
        deleted
            .apply(&[BlockOp::Delete {
                id: "section".into(),
            }])
            .unwrap();
        deleted.doc.commit();
        let deleting_frontiers = deleted.doc.oplog_frontiers();
        // Match detect_conflicts: fork the deleting side from the imported
        // history, then recover its last live content through the ancestor.
        deleted.doc.import(&editor.snapshot().unwrap()).unwrap();
        let deleting_view = deleted.fork_at(&deleting_frontiers).unwrap();
        assert!(deleting_view.doc.get_tree("blocks").roots().is_empty());
        let mut timings = Vec::with_capacity(CALLS);
        let mut deleted_blocks = DeletedBlocks::new(&deleting_view);
        for i in 0..CALLS {
            let start = Instant::now();
            let block = deleted_blocks
                .before_block_deletion(&format!("item-{i}"))
                .unwrap()
                .unwrap();
            timings.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(block.text, format!("Original content for item {i}"));
        }
        let mean = timings.iter().sum::<f64>() / CALLS as f64;
        let cold = timings[0];
        timings.sort_by(f64::total_cmp);
        println!("before_block_deletion profile={} blocks={BLOCKS} restored_ids={CALLS} calls={CALLS} cold_first_ms={cold:.3} mean_ms_per_call={mean:.3} p50_ms={:.3} p95_ms={:.3} max_ms={:.3}", if cfg!(debug_assertions) { "debug" } else { "release" }, timings[CALLS/2], timings[CALLS*95/100], timings[CALLS-1]);
    }
}
