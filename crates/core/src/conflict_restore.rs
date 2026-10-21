//! Restore historical conflict scopes without replacing unrelated page state.
//!
//! The source must be a historical fork of the target's Loro history. Callers
//! retain its frontiers, the returned ancestor IDs and any root fallback in the
//! synced resolution record. Page existence/purge state is handled by the caller.

use std::collections::{HashMap, HashSet};

use loro::event::DiffBatch;
use loro::{
    Container, ContainerID, ContainerTrait, LoroTree, LoroValue, TreeID, TreeNode, TreeParentId, ValueOrContainer,
};

use crate::doc::PageDoc;
use crate::error::{invalid, Result};

/// Saved parent and previous sibling. `None` for the tuple means derive the
/// position from the historical source; `(None, None)` means page-root start.
pub(crate) type Anchor<'a> = (Option<&'a str>, Option<&'a str>);

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RestoreEffect {
    pub fallback_to_root: bool,
    /// Missing ancestors revived by this edit, in root-to-leaf order.
    pub restored_ancestors: Vec<String>,
}

fn logical_id(tree: &LoroTree, node: TreeID) -> Option<String> {
    match tree.get_meta(node).ok()?.get("id")? {
        ValueOrContainer::Value(LoroValue::String(id)) => Some(id.to_string()),
        _ => None,
    }
}

fn visible_ids(tree: &LoroTree) -> HashMap<String, TreeID> {
    visible_nodes(tree)
        .into_iter()
        .filter_map(|node| logical_id(tree, node).map(|id| (id, node)))
        .collect()
}

fn visible_nodes(tree: &LoroTree) -> Vec<TreeID> {
    tree.roots()
        .into_iter()
        .flat_map(|root| descendants(tree, root))
        .collect()
}

fn descendants(tree: &LoroTree, root: TreeID) -> Vec<TreeID> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        let mut children = tree.children(node).unwrap_or_default();
        children.reverse();
        stack.extend(children);
    }
    out
}

fn historical_anchor(tree: &LoroTree, node: &TreeNode) -> (Option<String>, Option<String>) {
    let parent = match node.parent {
        TreeParentId::Node(parent) => logical_id(tree, parent),
        _ => None,
    };
    let siblings = tree.children(node.parent).unwrap_or_default();
    let previous = siblings
        .iter()
        .position(|id| *id == node.id)
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| logical_id(tree, siblings[i]));
    (parent, previous)
}

/// Move an already revived node. A missing/moved previous sibling means the
/// end of this parent, while no previous sibling means the beginning.
fn place(
    tree: &LoroTree,
    node: TreeID,
    parent: TreeParentId,
    previous: Option<&str>,
) -> Result<()> {
    let siblings: Vec<_> = tree
        .children(parent)
        .unwrap_or_default()
        .into_iter()
        .filter(|id| *id != node)
        .collect();
    let index = match previous {
        None => 0,
        Some(previous) => siblings
            .iter()
            .position(|id| logical_id(tree, *id).as_deref() == Some(previous))
            .map_or(siblings.len(), |i| i + 1),
    };
    tree.mov_to(node, parent, index)?;
    Ok(())
}

fn block_containers(tree: &LoroTree, nodes: &[TreeID]) -> HashSet<ContainerID> {
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

/// Add-on data can contain nested CRDT maps/lists, unlike the JSON strings in
/// props/schema/views. Include only containers reachable from this page-facing
/// root, so newly added derived metadata can never leak into a page restore.
fn add_containers(container: Container, allowed: &mut HashSet<ContainerID>) {
    match container {
        Container::Map(map) => {
            allowed.insert(map.id());
            if let LoroValue::Map(values) = map.get_value() {
                for key in values.keys() {
                    if let Some(ValueOrContainer::Container(child)) = map.get(key) {
                        add_containers(child, allowed);
                    }
                }
            }
        }
        Container::List(list) => {
            allowed.insert(list.id());
            for i in 0..list.len() {
                if let Some(ValueOrContainer::Container(child)) = list.get(i) {
                    add_containers(child, allowed);
                }
            }
        }
        _ => {}
    }
}

fn restore_content(
    doc: &mut PageDoc,
    source: &PageDoc,
    nodes: &[TreeID],
    whole_page: bool,
    message: Option<&str>,
) -> Result<()> {
    // This generation guards cached Vault instances against edits to a
    // replaced document. It describes the current runtime installation, not
    // the historical page's user content. Warning/trash remain side content.
    let purge_generation = if whole_page {
        match doc.doc.get_map("page").get("purge_restore") {
            Some(ValueOrContainer::Value(value)) => Some(value),
            Some(_) => return Err(invalid("invalid purge restoration generation")),
            None => None,
        }
    } else {
        None
    };
    if let Some(message) = message { doc.doc.set_next_commit_message(message); }
    doc.doc.commit();
    let source_tree = source.doc.get_tree("blocks");
    let mut allowed = block_containers(&source_tree, nodes);
    if whole_page {
        // These are the page-facing roots defined by PageDoc, not the peers,
        // conflicts, resolution/log records or vault settings.
        for name in ["page", "props", "schema", "views"] {
            allowed.insert(source.doc.get_map(name).id());
        }
        for name in ["schema_order", "view_order"] {
            allowed.insert(source.doc.get_list(name).id());
        }
        add_containers(
            Container::Map(source.doc.get_map("type_data")),
            &mut allowed,
        );
        add_containers(Container::Map(doc.doc.get_map("type_data")), &mut allowed);
    }
    let diff = doc
        .doc
        .diff(&doc.doc.oplog_frontiers(), &source.doc.oplog_frontiers())?;
    let mut filtered = DiffBatch::default();
    for (id, change) in diff.iter() {
        // The tree is set explicitly with original TreeIDs. Applying a
        // backwards tree diff can introduce alias identities.
        if allowed.contains(id) {
            filtered
                .push(id.clone(), change.clone())
                .expect("unique diff container");
        }
    }
    doc.doc.apply_diff(filtered)?;
    if whole_page {
        let page = doc.doc.get_map("page");
        match purge_generation {
            Some(value) => {
                page.insert("purge_restore", value)?;
            }
            None => {
                if page.get("purge_restore").is_some() {
                    page.delete("purge_restore")?;
                }
            }
        }
    }
    doc.invalidate_block_ids();
    Ok(())
}

/// Make a scope match `source`. With no root, restore the complete page-facing
/// state. With a root, restore its own fields and optionally its entire subtree.
/// An absent root in the source represents Deleted and uses ordinary tree Delete
/// semantics. For a present Block, current children keep their content/position.
pub(crate) fn restore(
    doc: &mut PageDoc,
    source: &PageDoc,
    root: Option<&str>,
    include_children: bool,
    anchor: Option<Anchor<'_>>,
) -> Result<RestoreEffect> {
    restore_tagged(doc, source, root, include_children, anchor, None)
}

/// Explicit choices tag every internal commit so imports recognize their edits.
pub(crate) fn restore_tagged(
    doc: &mut PageDoc,
    source: &PageDoc,
    root: Option<&str>,
    include_children: bool,
    anchor: Option<Anchor<'_>>,
    message: Option<&str>,
) -> Result<RestoreEffect> {
    if doc.meta().id != source.meta().id {
        return Err(invalid("conflict source belongs to another page"));
    }
    if let Some(message) = message { doc.doc.set_next_commit_message(message); }
    doc.doc.commit();
    let tree = doc.doc.get_tree("blocks");
    let source_tree = source.doc.get_tree("blocks");
    let source_nodes: HashMap<_, _> = source_tree
        .get_nodes(false)
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
    let source_ids = visible_ids(&source_tree);
    let current_ids = visible_ids(&tree);
    let current_visible: HashSet<_> = visible_nodes(&tree).into_iter().collect();
    let mut effect = RestoreEffect::default();

    let Some(root) = root else {
        let selected: Vec<_> = source_tree
            .roots()
            .into_iter()
            .flat_map(|node| descendants(&source_tree, node))
            .collect();
        let desired: Vec<_> = selected.iter().map(|id| source_nodes[id].clone()).collect();
        doc.invalidate_block_ids();
        crate::day_repair::move_nodes(&doc.doc, &desired)?;
        let selected_set: HashSet<_> = selected.iter().copied().collect();
        // Nodes moved to a selected parent remain visible until explicitly
        // deleted; remove all current extras, including nested ones.
        for node in visible_nodes(&tree) {
            if !selected_set.contains(&node) {
                tree.delete(node)?;
            }
        }
        restore_content(doc, source, &selected, true, message)?;
        return Ok(effect);
    };

    let Some(&source_root) = source_ids.get(root) else {
        if let Some(&current_root) = current_ids.get(root) {
            doc.invalidate_block_ids();
            tree.delete(current_root)?;
        }
        return Ok(effect);
    };
    let selected = if include_children {
        descendants(&source_tree, source_root)
    } else {
        vec![source_root]
    };
    let selected_set: HashSet<_> = selected.iter().copied().collect();
    let (parent, previous) = match anchor {
        Some((parent, previous)) => (parent.map(str::to_owned), previous.map(str::to_owned)),
        None => historical_anchor(&source_tree, &source_nodes[&source_root]),
    };
    if let Some(parent) = parent.as_deref() {
        let subtree: HashSet<_> = descendants(&source_tree, source_root).into_iter().collect();
        if source_ids
            .get(parent)
            .is_some_and(|node| subtree.contains(node))
        {
            return Err(invalid("conflict anchor is inside its own scope"));
        }
        if let Some(&parent_node) = current_ids.get(parent) {
            let mut ancestor = parent_node;
            loop {
                if ancestor == source_root {
                    return Err(invalid("conflict anchor is inside its own scope"));
                }
                match tree.parent(ancestor) {
                    Some(TreeParentId::Node(parent)) => ancestor = parent,
                    _ => break,
                }
            }
        }
    }

    // Find only the missing chain required by the anchor. A source version
    // unable to rebuild that chain takes the explicitly approved root fallback.
    let mut ancestors = Vec::new();
    let mut required = parent.clone();
    while let Some(id) = required {
        if current_ids.contains_key(&id) {
            break;
        }
        let Some(&node) = source_ids.get(&id) else {
            effect.fallback_to_root = true;
            ancestors.clear();
            break;
        };
        if selected_set.contains(&node) {
            return Err(invalid("conflict anchor is inside its own scope"));
        }
        ancestors.push(node);
        required = historical_anchor(&source_tree, &source_nodes[&node]).0;
    }
    ancestors.reverse();
    effect.restored_ancestors = ancestors
        .iter()
        .filter_map(|id| logical_id(&source_tree, *id))
        .collect();
    let restored_set: HashSet<_> = ancestors.iter().copied().collect();

    // Reviving an ancestor automatically reveals children still attached to it.
    // Explicitly delete hidden off-chain child roots before that can happen.
    // A Block similarly preserves only children visible before this restore.
    doc.invalidate_block_ids();
    let shallow_nodes: Vec<_> = ancestors
        .iter()
        .copied()
        .chain((!include_children).then_some(source_root))
        .collect();
    for ancestor in shallow_nodes {
        for child in tree.nodes() {
            if tree.parent(child) == Some(TreeParentId::Node(ancestor))
                && !selected_set.contains(&child)
                && !restored_set.contains(&child)
                && !current_visible.contains(&child)
            {
                tree.delete(child)?;
            }
        }
    }

    // Reuse live ancestor identities. Only newly restored ancestors get their
    // own historical content/placement; existing parents are left untouched.
    let parent_for = |id: Option<&str>| -> Result<TreeParentId> {
        match id {
            None => Ok(TreeParentId::Root),
            Some(id) => current_ids
                .get(id)
                .or_else(|| source_ids.get(id))
                .copied()
                .map(TreeParentId::Node)
                .ok_or_else(|| invalid(format!("missing restored parent {id}"))),
        }
    };
    for ancestor in &ancestors {
        let mut node = source_nodes[ancestor].clone();
        let (ancestor_parent, ancestor_previous) = historical_anchor(&source_tree, &node);
        node.parent = parent_for(ancestor_parent.as_deref())?;
        crate::day_repair::move_nodes(&doc.doc, std::slice::from_ref(&node))?;
        place(&tree, node.id, node.parent, ancestor_previous.as_deref())?;
    }
    let root_parent = if effect.fallback_to_root {
        TreeParentId::Root
    } else {
        parent_for(parent.as_deref())?
    };
    let mut desired: Vec<_> = selected.iter().map(|id| source_nodes[id].clone()).collect();
    desired[0].parent = root_parent;
    crate::day_repair::move_nodes(&doc.doc, &desired)?;
    if effect.fallback_to_root {
        let end = tree
            .children(TreeParentId::Root)
            .unwrap_or_default()
            .into_iter()
            .filter(|id| *id != source_root)
            .count();
        tree.mov_to(source_root, TreeParentId::Root, end)?;
    } else {
        place(&tree, source_root, root_parent, previous.as_deref())?;
    }
    if include_children {
        for node in descendants(&tree, source_root) {
            if !selected_set.contains(&node) {
                tree.delete(node)?;
            }
        }
    }
    let content_nodes: Vec<_> = ancestors.into_iter().chain(selected).collect();
    restore_content(doc, source, &content_nodes, false, message)?;
    Ok(effect)
}

/// Restore the scoped before-state and remove exactly the ancestors this
/// resolution introduced. Deleting ancestors uses ordinary Delete semantics;
/// later edits within that restored structure are part of the reverted state.
/// Unrelated roots/siblings and their metadata/text containers are untouched.
#[cfg(test)]
pub(crate) fn undo_restore(
    doc: &mut PageDoc,
    before: &PageDoc,
    root: Option<&str>,
    include_children: bool,
    introduced_ancestors: &[String],
) -> Result<RestoreEffect> {
    undo_restore_tagged(doc, before, root, include_children, introduced_ancestors, None)
}

pub(crate) fn undo_restore_tagged(
    doc: &mut PageDoc,
    before: &PageDoc,
    root: Option<&str>,
    include_children: bool,
    introduced_ancestors: &[String],
    message: Option<&str>,
) -> Result<RestoreEffect> {
    let effect = restore_tagged(doc, before, root, include_children, None, message)?;
    let tree = doc.doc.get_tree("blocks");
    let before_ids = visible_ids(&before.doc.get_tree("blocks"));
    for ancestor in introduced_ancestors.iter().rev() {
        if !before_ids.contains_key(ancestor) {
            if let Some(node) = visible_ids(&tree).get(ancestor).copied() {
                doc.invalidate_block_ids();
                tree.delete(node)?;
            }
        }
    }
    Ok(effect)
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

    fn fork(doc: &PageDoc) -> PageDoc {
        doc.doc.commit();
        doc.fork_at(&doc.doc.oplog_frontiers()).unwrap()
    }

    fn ids(doc: &mut PageDoc) -> Vec<String> {
        doc.blocks().into_iter().map(|b| b.id).collect()
    }

    fn nested() -> PageDoc {
        let mut doc = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        doc.apply(&[
            insert(None, None, "left", "Left"),
            insert(None, Some("left"), "grand", "Grandparent"),
            insert(None, Some("grand"), "right", "Right"),
            insert(Some("grand"), None, "hidden", "Off-chain sibling"),
            insert(Some("grand"), Some("hidden"), "parent", "Parent"),
            insert(Some("parent"), None, "target", "Chosen root"),
            insert(Some("target"), None, "first", "First"),
            insert(Some("target"), Some("first"), "second", "Second"),
        ])
        .unwrap();
        doc
    }

    #[test]
    fn subtree_restores_original_identity_children_and_order() {
        let mut doc = nested();
        let source = fork(&doc);
        let physical = visible_ids(&doc.doc.get_tree("blocks"));
        doc.apply(&[
            BlockOp::Delete { id: "first".into() },
            text("second", "Changed second"),
            insert(Some("target"), Some("second"), "extra", "Extra"),
            text("right", "Unrelated current edit"),
        ])
        .unwrap();
        let effect = restore(&mut doc, &source, Some("target"), true, None).unwrap();
        assert_eq!(effect, RestoreEffect::default());
        let tree = doc.doc.get_tree("blocks");
        let current = visible_ids(&tree);
        for id in ["target", "first", "second"] {
            assert_eq!(current[id], physical[id]);
        }
        assert!(!current.contains_key("extra"));
        let page = doc.blocks();
        let target = &page[1].children[1].children[0];
        assert_eq!(
            target
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(target.children[1].text, "Second");
        assert_eq!(page[2].text, "Unrelated current edit");
    }

    #[test]
    fn block_restores_own_content_preserving_current_children() {
        let mut doc = nested();
        let source = fork(&doc);
        doc.apply(&[
            text("target", "Changed root"),
            text("first", "Current child"),
        ])
        .unwrap();
        restore(&mut doc, &source, Some("target"), false, None).unwrap();
        let page = doc.blocks();
        let target = &page[1].children[1].children[0];
        assert_eq!(target.text, "Chosen root");
        assert_eq!(target.children[0].text, "Current child");
    }

    #[test]
    fn missing_ancestors_restore_only_required_chain_and_undo_removes_it() {
        let mut doc = nested();
        let source = fork(&doc);
        let physical = visible_ids(&doc.doc.get_tree("blocks"));
        doc.apply(&[BlockOp::Delete { id: "grand".into() }])
            .unwrap();
        let before = fork(&doc);
        let effect = restore(&mut doc, &source, Some("target"), true, None).unwrap();
        assert_eq!(effect.restored_ancestors, ["grand", "parent"]);
        assert!(!effect.fallback_to_root);
        assert_eq!(ids(&mut doc), ["left", "grand", "right"]);
        let visible = visible_ids(&doc.doc.get_tree("blocks"));
        assert!(!visible.contains_key("hidden"));
        for id in ["grand", "parent", "target", "first", "second"] {
            assert_eq!(visible[id], physical[id]);
        }
        doc.apply(&[
            text("right", "Edit after resolution"),
            insert(None, Some("right"), "later", "Later"),
        ])
        .unwrap();
        undo_restore(
            &mut doc,
            &before,
            Some("target"),
            true,
            &effect.restored_ancestors,
        )
        .unwrap();
        assert_eq!(ids(&mut doc), ["left", "right", "later"]);
        assert_eq!(doc.blocks()[1].text, "Edit after resolution");
        let visible = visible_ids(&doc.doc.get_tree("blocks"));
        for id in ["grand", "parent", "target", "hidden"] {
            assert!(!visible.contains_key(id));
        }
    }

    #[test]
    fn undo_restores_before_scope_without_reverting_unrelated_content() {
        let mut doc = nested();
        let chosen = fork(&doc);
        doc.apply(&[text("target", "Before pick"), text("first", "Before child")])
            .unwrap();
        let before = fork(&doc);
        restore(&mut doc, &chosen, Some("target"), true, None).unwrap();
        doc.apply(&[text("left", "Keep unrelated edit")]).unwrap();
        undo_restore(&mut doc, &before, Some("target"), true, &[]).unwrap();
        let page = doc.blocks();
        assert_eq!(page[0].text, "Keep unrelated edit");
        let target = &page[1].children[1].children[0];
        assert_eq!(target.text, "Before pick");
        assert_eq!(target.children[0].text, "Before child");
    }

    #[test]
    fn missing_historical_parent_falls_back_to_root_end() {
        let mut doc = nested();
        let source = fork(&doc);
        let effect = restore(
            &mut doc,
            &source,
            Some("target"),
            true,
            Some((Some("missing"), None)),
        )
        .unwrap();
        assert!(effect.fallback_to_root);
        assert!(effect.restored_ancestors.is_empty());
        assert_eq!(ids(&mut doc), ["left", "grand", "right", "target"]);
    }

    #[test]
    fn missing_previous_sibling_appends_to_parent() {
        let mut doc = nested();
        let source = fork(&doc);
        doc.apply(&[BlockOp::Delete { id: "first".into() }])
            .unwrap();
        restore(
            &mut doc,
            &source,
            Some("first"),
            false,
            Some((Some("target"), Some("missing"))),
        )
        .unwrap();
        let page = doc.blocks();
        assert_eq!(
            page[1].children[1].children[0]
                .children
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
    }

    #[test]
    fn whole_page_restores_content_and_addon_data_but_keeps_derived_metadata() {
        let mut doc = nested();
        doc.set_meta_str("purge_restore", Some("historical-generation"))
            .unwrap();
        doc.type_data_set(
            "addon",
            &["nested".into()],
            &serde_json::json!({"value": "before"}),
        )
        .unwrap();
        let source = fork(&doc);
        doc.apply(&[
            text("left", "Changed"),
            insert(None, Some("right"), "extra", "Extra"),
        ])
        .unwrap();
        doc.set_title("Changed title").unwrap();
        doc.set_meta_str("purge_restore", Some("current-generation"))
            .unwrap();
        doc.type_data_set(
            "addon",
            &["nested".into(), "value".into()],
            &serde_json::json!("after"),
        )
        .unwrap();
        doc.doc
            .get_map("conflicts")
            .insert("record", "keep")
            .unwrap();
        doc.doc
            .get_map("conflict_log")
            .insert("entry", "keep log")
            .unwrap();
        doc.register_peer("current-device").unwrap();
        restore(&mut doc, &source, None, true, None).unwrap();
        assert_eq!(doc.meta().title, "Page");
        assert_eq!(ids(&mut doc), ["left", "grand", "right"]);
        assert_eq!(doc.blocks()[0].text, "Left");
        let addon_data = doc.type_data_value();
        assert_eq!(
            addon_data["addon"],
            serde_json::json!({"nested": {"value": "before"}})
        );
        assert!(doc.doc.get_map("conflicts").get("record").is_some());
        assert!(doc.doc.get_map("conflict_log").get("entry").is_some());
        assert!(matches!(doc.doc.get_map("page").get("purge_restore"),
            Some(ValueOrContainer::Value(LoroValue::String(value))) if value.as_ref() == "current-generation"));
        assert!(doc
            .peers()
            .values()
            .any(|device| device == "current-device"));
    }
}
