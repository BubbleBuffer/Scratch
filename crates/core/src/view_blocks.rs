//! Contents views live in ordinary blocks. Legacy saved views remain intact
//! while a page's first open adds their block counterparts.

use std::collections::{HashMap, HashSet};

use loro::{LoroMap, LoroTree, LoroValue, TreeID, TreeParentId, ValueOrContainer};

use crate::doc::PageDoc;
use crate::error::Result;
use crate::types::{Block, BlockKind, ViewDef, ViewKind, PAGE_TYPE_DATABASE, PAGE_TYPE_DOCUMENT};

/// Helpers for the JSON payload of a `view` block.
pub struct ViewBlock;

fn list_def() -> ViewDef {
    ViewDef {
        id: String::new(),
        name: "List".into(),
        kind: ViewKind::List,
        filter: None,
        sorts: Vec::new(),
        group_by: None,
        date_prop: None,
        visible_props: Vec::new(),
        card_cover: None,
        config: String::new(),
    }
}

impl ViewBlock {
    /// Reads a view without trusting its JSON identity. Missing fields use
    /// List defaults; invalid JSON or invalid field types fall back to List.
    /// The block itself is always the view's identity.
    pub fn def(block: &Block) -> ViewDef {
        let mut fallback = list_def();
        fallback.id = block.id.clone();
        let Ok(serde_json::Value::Object(mut data)) = serde_json::from_str(&block.text) else {
            return fallback;
        };
        data.remove("id");
        let mut defaults = serde_json::to_value(&fallback)
            .expect("ViewDef serializes")
            .as_object()
            .expect("ViewDef is an object")
            .clone();
        defaults.extend(data);
        serde_json::from_value(serde_json::Value::Object(defaults)).unwrap_or(fallback)
    }
}

/// Serializes a view's settings for a contents block. Identity belongs to
/// the block, so it is omitted from the stored payload.
pub fn view_block_text(def: &ViewDef) -> String {
    let mut value = serde_json::to_value(def).expect("ViewDef serializes");
    let object = value.as_object_mut().expect("ViewDef is an object");
    object.remove("id");
    object.insert("source".into(), "contents".into());
    value.to_string()
}

fn meta_str(meta: &LoroMap, key: &str) -> Option<String> {
    match meta.get(key) {
        Some(ValueOrContainer::Value(LoroValue::String(value))) => Some(value.to_string()),
        _ => None,
    }
}

/// Length framing keeps even imported legacy ids unambiguous.
fn stable_id(domain: &str, parts: &[&str]) -> String {
    let mut input = domain.to_owned();
    for part in parts {
        input.push_str(&format!("\0{}:{part}", part.len()));
    }
    format!(
        "view-{}",
        &crate::attach::sha256_hex(input.as_bytes())[..32]
    )
}

/// A prior copy may remain visible when a concurrent move restores its
/// physical source. Keep the usual deterministic id when free, then probe
/// deterministically against every visible identity before creating a copy.
fn reserve_copy_id(candidate: String, reserved: &mut HashSet<String>) -> String {
    let mut fresh = candidate.clone();
    let mut attempt = 0_u64;
    while !reserved.insert(fresh.clone()) {
        attempt += 1;
        fresh = stable_id(
            "notes-app:view-copy-collision:v1",
            &[&candidate, &attempt.to_string()],
        );
    }
    fresh
}

fn migration_pending(doc: &PageDoc) -> bool {
    let meta = doc.doc.get_map("page");
    crate::doc::read_page_type(&meta) == PAGE_TYPE_DATABASE
        && !matches!(
            meta.get("views_migrated"),
            Some(ValueOrContainer::Value(LoroValue::I64(1)))
        )
}

impl PageDoc {
    /// Records that the page has no saved views left to migrate.
    pub(crate) fn mark_view_blocks_current(&mut self) -> Result<()> {
        self.doc.get_map("page").insert("views_migrated", 1_i64)?;
        Ok(())
    }

    /// A cheap preflight lets ordinary opens avoid a write transaction and
    /// leaves their decoded block cache cold. Repairs still run after sync,
    /// including on databases already converted to documents.
    pub(crate) fn needs_view_block_upgrade(&self) -> bool {
        if migration_pending(self) {
            return true;
        }
        let tree = self.doc.get_tree("blocks");
        let mut ids = HashSet::new();
        tree.get_nodes(false).into_iter().any(|node| {
            let Ok(meta) = tree.get_meta(node.id) else {
                return false;
            };
            meta_str(&meta, "kind").as_deref() == Some("view")
                && meta_str(&meta, "id").is_some_and(|id| !ids.insert(id))
        })
    }

    /// First-open migration and physical-node repair are one durable edit.
    /// Legacy schema/views maps remain available to older clients.
    pub(crate) fn upgrade_view_blocks(&mut self) -> Result<bool> {
        let mut changed = false;
        if migration_pending(self) {
            let meta = self.meta();
            let views = self.views();
            if !views.is_empty() {
                let title_only = self
                    .schema()
                    .unwrap_or_default()
                    .iter()
                    .all(|prop| prop.key == "$title");
                let migrated = if title_only {
                    let mut list = list_def();
                    list.id = views[0].id.clone();
                    vec![list]
                } else {
                    views
                };
                let tree = self.doc.get_tree("blocks");
                for view in migrated {
                    let id = stable_id("notes-app:view-migration:v1", &[&meta.id, &view.id]);
                    let node = tree.create_at(TreeParentId::Root, tree.roots().len())?;
                    let block_meta = tree.get_meta(node)?;
                    block_meta.insert("id", id)?;
                    block_meta.insert("kind", "view")?;
                    block_meta.insert("checked", false)?;
                    let text = block_meta.insert_container("text", loro::LoroText::new())?;
                    text.insert(0, &view_block_text(&view))?;
                }
                if title_only {
                    self.set_page_type(PAGE_TYPE_DOCUMENT)?;
                }
            }
            self.doc.get_map("page").insert("views_migrated", 1_i64)?;
            changed = true;
        }
        let tree = self.doc.get_tree("blocks");
        let mut first = HashMap::new();
        let mut reserved = tree
            .get_nodes(false)
            .into_iter()
            .filter_map(|node| meta_str(&tree.get_meta(node.id).ok()?, "id"))
            .collect();
        changed |=
            self.repair_view_children(&tree, TreeParentId::Root, &mut first, &mut reserved)?;
        if changed {
            self.invalidate_block_ids();
        }
        Ok(changed)
    }

    fn repair_view_children(
        &self,
        tree: &LoroTree,
        parent: TreeParentId,
        first: &mut HashMap<String, TreeID>,
        reserved: &mut HashSet<String>,
    ) -> Result<bool> {
        let mut changed = false;
        // Snapshot physical siblings before inserting replacements. Each is
        // processed once, and every lookup/deletion uses its actual TreeID.
        for mut node in tree.children(parent).unwrap_or_default() {
            let meta = tree.get_meta(node)?;
            if meta_str(&meta, "kind").as_deref() == Some(BlockKind::View.as_str()) {
                if let Some(id) = meta_str(&meta, "id") {
                    if let Some(&keep) = first.get(&id) {
                        let identical =
                            match (self.block_at_tree_node(keep), self.block_at_tree_node(node)) {
                                (Some(a), Some(b)) => {
                                    a.text == b.text
                                        && a.target == b.target
                                        && a.children == b.children
                                }
                                _ => false,
                            };
                        if identical {
                            tree.delete(node)?;
                            changed = true;
                            continue;
                        }
                        let page = self.meta().id;
                        let physical = node.to_string();
                        let replacement_id = reserve_copy_id(
                            stable_id("notes-app:view-duplicate:v1", &[&page, &id, &physical]),
                            reserved,
                        );
                        let siblings = tree.children(parent).unwrap_or_default();
                        let index = siblings
                            .iter()
                            .position(|&sibling| sibling == node)
                            .expect("visible duplicate is a child of its parent");
                        let replacement = copy_subtree(
                            tree,
                            node,
                            parent,
                            index,
                            &replacement_id,
                            true,
                            reserved,
                        )?;
                        tree.get_meta(replacement)?
                            .insert("id", replacement_id.clone())?;
                        tree.delete(node)?;
                        node = replacement;
                        first.insert(replacement_id, node);
                        changed = true;
                    } else {
                        first.insert(id, node);
                    }
                }
            }
            changed |=
                self.repair_view_children(tree, TreeParentId::Node(node), first, reserved)?;
        }
        Ok(changed)
    }
}

fn copy_subtree(
    tree: &LoroTree,
    source: TreeID,
    parent: TreeParentId,
    index: usize,
    replacement_id: &str,
    root: bool,
    reserved: &mut HashSet<String>,
) -> Result<TreeID> {
    let copy = tree.create_at(parent, index)?;
    let from = tree.get_meta(source)?;
    let to = tree.get_meta(copy)?;
    // Loro copies attached containers recursively on insertion. Preserve
    // unknown metadata as well as text attributes, colours and block state.
    for key in from.keys() {
        match from.get(&key) {
            Some(ValueOrContainer::Value(value)) => to.insert(&key, value)?,
            Some(ValueOrContainer::Container(container)) => {
                to.insert_container(&key, container)?;
            }
            None => {}
        }
    }
    if !root {
        let physical = source.to_string();
        to.insert(
            "id",
            reserve_copy_id(
                stable_id("notes-app:view-child-copy:v1", &[replacement_id, &physical]),
                reserved,
            ),
        )?;
    }
    // Clone children rather than moving them: concurrent repairs must not
    // split one subtree between their independently created replacements.
    for (index, child) in tree
        .children(source)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        copy_subtree(
            tree,
            child,
            TreeParentId::Node(copy),
            index,
            replacement_id,
            false,
            reserved,
        )?;
    }
    Ok(copy)
}
