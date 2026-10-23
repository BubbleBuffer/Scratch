//! Display-only overlays over the two recorded versions. Offsets, like marks,
//! count Unicode scalar values. Applying a choice never consumes this diff.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::types::{Block, Conflict, ConflictScope, ConflictSide, PageId, SideState};
use crate::vault::Vault;

/// Display overlays in the recorded side order. Indices still identify the
/// original sides for UseSide, even when Local is the second side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictDiff {
    pub scope: ConflictScope,
    pub sides: Vec<ConflictDiffSide>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictDiffSide {
    pub device: String,
    pub local: bool,
    /// The recorded lifecycle state, independent of the displayed blocks.
    pub state: SideState,
    pub blocks: Vec<ConflictDiffBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictDiffChange {
    Unchanged,
    Modified,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictDiffSpan {
    pub start: u32,
    pub end: u32,
    pub change: ConflictDiffChange,
}

/// `block.children` is empty; the annotated tree is in `children`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictDiffBlock {
    pub block: Block,
    pub change: ConflictDiffChange,
    pub spans: Vec<ConflictDiffSpan>,
    pub children: Vec<ConflictDiffBlock>,
}

// Bound both allocation and work for unusually large changed paragraphs.
// Beyond this size a single replacement of the differing middle is a valid
// diff, preserving the common prefix/suffix and every character on each side.
const MAX_DIFF_CELLS: usize = 1_000_000;

fn push(spans: &mut Vec<ConflictDiffSpan>, pos: usize, change: ConflictDiffChange) {
    if let Some(last) = spans.last_mut() {
        if last.change == change && last.end == pos as u32 {
            last.end += 1;
            return;
        }
    }
    spans.push(ConflictDiffSpan {
        start: pos as u32,
        end: pos as u32 + 1,
        change,
    });
}

fn text_spans(local: &str, cloud: &str) -> (Vec<ConflictDiffSpan>, Vec<ConflictDiffSpan>) {
    use ConflictDiffChange::{Added, Removed, Unchanged};
    let a: Vec<char> = local.chars().collect();
    let b: Vec<char> = cloud.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let aa = &a[prefix..a.len() - suffix];
    let bb = &b[prefix..b.len() - suffix];
    let mut left = Vec::new();
    let mut right = Vec::new();
    for i in 0..prefix {
        push(&mut left, i, Unchanged);
        push(&mut right, i, Unchanged);
    }
    let cells = (aa.len() + 1).checked_mul(bb.len() + 1);
    if cells.is_none_or(|n| n > MAX_DIFF_CELLS) {
        for i in prefix..a.len() - suffix {
            push(&mut left, i, Removed);
        }
        for j in prefix..b.len() - suffix {
            push(&mut right, j, Added);
        }
    } else {
        let width = bb.len() + 1;
        let mut lcs = vec![0u32; cells.unwrap()];
        for i in (0..aa.len()).rev() {
            for j in (0..bb.len()).rev() {
                lcs[i * width + j] = if aa[i] == bb[j] {
                    1 + lcs[(i + 1) * width + j + 1]
                } else {
                    lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < aa.len() || j < bb.len() {
            if i < aa.len() && j < bb.len() && aa[i] == bb[j] {
                push(&mut left, prefix + i, Unchanged);
                push(&mut right, prefix + j, Unchanged);
                i += 1;
                j += 1;
            } else if i < aa.len()
                && (j == bb.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])
            {
                push(&mut left, prefix + i, Removed);
                i += 1;
            } else {
                push(&mut right, prefix + j, Added);
                j += 1;
            }
        }
    }
    for i in a.len() - suffix..a.len() {
        push(&mut left, i, Unchanged);
    }
    for j in b.len() - suffix..b.len() {
        push(&mut right, j, Unchanged);
    }
    (left, right)
}

fn index<'a>(blocks: &'a [Block], out: &mut HashMap<&'a str, &'a Block>) {
    for block in blocks {
        out.insert(&block.id, block);
        index(&block.children, out);
    }
}

fn own_content_equal(a: &Block, b: &Block) -> bool {
    a.kind == b.kind
        && a.text == b.text
        && a.marks == b.marks
        && a.checked == b.checked
        && a.target == b.target
        && a.color == b.color
        && a.width == b.width
}

fn annotated(
    blocks: &[Block],
    opposite: &HashMap<&str, &Block>,
    local: bool,
) -> Vec<ConflictDiffBlock> {
    blocks
        .iter()
        .map(|block| {
            let other = opposite.get(block.id.as_str()).copied();
            let change = match other {
                None if local => ConflictDiffChange::Removed,
                None => ConflictDiffChange::Added,
                Some(other) if own_content_equal(block, other) => ConflictDiffChange::Unchanged,
                Some(_) => ConflictDiffChange::Modified,
            };
            let spans = if let Some(other) = other {
                let (left, right) = if local {
                    text_spans(&block.text, &other.text)
                } else {
                    text_spans(&other.text, &block.text)
                };
                if local {
                    left
                } else {
                    right
                }
            } else if block.text.is_empty() {
                Vec::new()
            } else {
                vec![ConflictDiffSpan {
                    start: 0,
                    end: block.text.chars().count() as u32,
                    change,
                }]
            };
            let mut own = block.clone();
            own.children.clear();
            ConflictDiffBlock {
                block: own,
                change,
                spans,
                children: annotated(&block.children, opposite, local),
            }
        })
        .collect()
}

pub(crate) fn diff_blocks(
    local: &[Block],
    cloud: &[Block],
) -> (Vec<ConflictDiffBlock>, Vec<ConflictDiffBlock>) {
    let (mut left, mut right) = (HashMap::new(), HashMap::new());
    index(local, &mut left);
    index(cloud, &mut right);
    (
        annotated(local, &right, true),
        annotated(cloud, &left, false),
    )
}

fn find_block<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    for block in blocks {
        if block.id == id {
            return Some(block);
        }
        if let Some(found) = find_block(&block.children, id) {
            return Some(found);
        }
    }
    None
}

fn scoped_blocks(blocks: Vec<Block>, scope: &ConflictScope) -> Vec<Block> {
    match scope {
        ConflictScope::Page => blocks,
        ConflictScope::Block { block } => find_block(&blocks, block)
            .map(|found| {
                let mut own = found.clone();
                own.children.clear();
                vec![own]
            })
            .unwrap_or_default(),
        ConflictScope::Subtree { root } => find_block(&blocks, root)
            .map(|found| vec![found.clone()])
            .unwrap_or_default(),
    }
}

fn side_blocks(doc: &PageDoc, scope: &ConflictScope, side: &ConflictSide) -> Result<Vec<Block>> {
    match &side.state {
        SideState::Deleted | SideState::Purged { .. } => Ok(Vec::new()),
        SideState::Present { content } => match &side.frontiers {
            Some(bytes) => {
                let frontiers =
                    loro::Frontiers::decode(bytes).map_err(|error| invalid(error.to_string()))?;
                let mut version = doc.fork_at(&frontiers)?;
                Ok(scoped_blocks(version.blocks(), scope))
            }
            None => Ok(scoped_blocks(content.clone(), scope)),
        },
        SideState::Trashed { .. } => {
            if !matches!(scope, ConflictScope::Page) {
                return Err(invalid("trashed conflict side requires page scope"));
            }
            let bytes = side
                .frontiers
                .as_ref()
                .ok_or_else(|| invalid("trashed conflict side has no historical frontiers"))?;
            let frontiers =
                loro::Frontiers::decode(bytes).map_err(|error| invalid(error.to_string()))?;
            let mut version = doc.fork_at(&frontiers)?;
            Ok(version.blocks())
        }
    }
}

fn diff_record(doc: &PageDoc, conflict: Conflict, device: &str) -> Result<ConflictDiff> {
    if conflict.sides.len() != 2 {
        return Err(invalid("conflict diff requires exactly two recorded sides"));
    }
    let left_content = side_blocks(doc, &conflict.scope, &conflict.sides[0])?;
    let right_content = side_blocks(doc, &conflict.scope, &conflict.sides[1])?;
    // With no local version, use record order for deterministic overlays but
    // never label either foreign version Local.
    let local = conflict.sides.iter().position(|side| side.device == device);
    let (left, right) = if local == Some(1) {
        let (right, left) = diff_blocks(&right_content, &left_content);
        (left, right)
    } else {
        diff_blocks(&left_content, &right_content)
    };
    let sides = conflict
        .sides
        .into_iter()
        .zip([left, right])
        .zip([left_content, right_content])
        .map(|((side, blocks), content)| {
            let state = match side.state {
                SideState::Present { .. } => SideState::Present { content },
                state => state,
            };
            ConflictDiffSide {
                local: side.device == device,
                device: side.device,
                state,
                blocks,
            }
        })
        .collect();
    Ok(ConflictDiff {
        scope: conflict.scope,
        sides,
    })
}

impl Vault {
    /// Rebuilds the recorded scoped versions and overlays their differences.
    /// Reads the record and its historical versions from the same active or
    /// retained archive document. No diff data participates in resolution.
    pub fn conflict_diff(&self, page: PageId, id: String) -> Result<ConflictDiff> {
        let doc = self.load_conflict_history_doc(&page)?;
        let (_, json) = doc
            .conflicts()
            .into_iter()
            .find(|(key, _)| key == &id)
            .ok_or_else(|| not_found("conflict", &id))?;
        let conflict = crate::sync::read_conflict(&page, &json)
            .ok_or_else(|| invalid("invalid conflict record"))?;
        diff_record(&doc, conflict, &self.inner.device)
    }
}

#[cfg(test)]
mod wrapper_tests {
    use super::*;
    use crate::types::{BlockKind, BlockOp, ConflictStatus, NewBlock};
    use std::path::PathBuf;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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

    fn setup() -> (Temp, Vault, PageId, Conflict) {
        let path = std::env::temp_dir().join(format!("conflict_diff_api_{}", crate::new_id()));
        let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
        let page = vault.create_page(None, "Diff".into()).unwrap();
        let conflict = vault
            .inner
            .edit(&page, |doc| {
                doc.apply(&[
                    insert(None, None, "shared", "Shared"),
                    insert(None, Some("shared"), "cloud-only", "Cloud only"),
                    insert(Some("cloud-only"), None, "nested", "Nested cloud child"),
                ])?;
                doc.doc.commit();
                let cloud = doc.version();
                doc.apply(&[
                    BlockOp::Delete {
                        id: "cloud-only".into(),
                    },
                    insert(None, Some("shared"), "local-only", "Local only"),
                ])?;
                doc.doc.commit();
                let local = doc.version();
                let conflict = Conflict {
                    id: "page-record".into(),
                    format: 2,
                    page: page.clone(),
                    block: String::new(),
                    time_ms: 10,
                    scope: ConflictScope::Page,
                    // Empty cached content proves frontiers supply the full tree.
                    sides: vec![
                        ConflictSide::new(
                            "cloud-device".into(),
                            1,
                            Some(cloud),
                            SideState::Present { content: vec![] },
                            None,
                        ),
                        ConflictSide::new(
                            vault.device_id(),
                            2,
                            Some(local),
                            SideState::Present { content: vec![] },
                            None,
                        ),
                    ],
                    status: ConflictStatus::Pending,
                    applied: vec![],
                };
                crate::sync::write_conflict(doc, &conflict)?;
                Ok(conflict)
            })
            .unwrap();
        (Temp(path), vault, page, conflict)
    }

    #[test]
    fn conflict_diff_api_local_side_one_preserves_indices_and_whole_page_trees() {
        let (_dir, vault, page, record) = setup();
        let before = vault.inner.read(&page, |doc| doc.blocks()).unwrap();
        let diff = vault.conflict_diff(page.clone(), record.id).unwrap();
        assert_eq!(diff.scope, ConflictScope::Page);
        assert_eq!(diff.sides[0].device, "cloud-device");
        assert!(!diff.sides[0].local);
        assert!(diff.sides[1].local);
        assert_eq!(
            diff.sides[0].blocks[0].change,
            ConflictDiffChange::Unchanged
        );
        assert_eq!(diff.sides[0].blocks[1].block.id, "cloud-only");
        assert_eq!(diff.sides[0].blocks[1].change, ConflictDiffChange::Added);
        assert_eq!(diff.sides[0].blocks[1].children[0].block.id, "nested");
        assert_eq!(
            diff.sides[0].blocks[1].children[0].change,
            ConflictDiffChange::Added
        );
        assert_eq!(diff.sides[1].blocks[1].block.id, "local-only");
        assert_eq!(diff.sides[1].blocks[1].change, ConflictDiffChange::Removed);
        let SideState::Present { content } = &diff.sides[0].state else {
            panic!("present state");
        };
        assert_eq!(content[1].children[0].id, "nested");
        assert_eq!(
            vault.inner.read(&page, |doc| doc.blocks()).unwrap(),
            before,
            "diff never edits the page"
        );
    }

    #[test]
    fn conflict_diff_api_trashed_page_rebuilds_historical_content() {
        let (_dir, vault, page, mut record) = setup();
        record.sides[0].state = SideState::Trashed { at: 42 };
        vault
            .inner
            .edit(&page, |doc| crate::sync::write_conflict(doc, &record))
            .unwrap();
        let diff = vault.conflict_diff(page, record.id).unwrap();
        assert_eq!(diff.sides[0].state, SideState::Trashed { at: 42 });
        assert_eq!(diff.sides[0].blocks[1].block.id, "cloud-only");
        assert_eq!(diff.sides[0].blocks[1].children[0].block.id, "nested");
        assert_eq!(diff.sides[0].blocks[1].change, ConflictDiffChange::Added);
    }

    #[test]
    fn conflict_diff_api_purged_page_reads_retained_history_without_restoring_page() {
        let (_dir, vault, page, record) = setup();
        let before = vault
            .conflict_diff(page.clone(), record.id.clone())
            .unwrap();
        vault.delete_page(page.clone()).unwrap();
        vault.purge_page(page.clone()).unwrap();
        assert!(matches!(
            vault.load_page(page.clone()),
            Err(crate::VaultError::NotFound(_))
        ));

        let archived = vault.conflict_diff(page.clone(), record.id).unwrap();
        assert_eq!(archived, before);
        assert_eq!(archived.sides[0].blocks[1].children[0].block.id, "nested");
        assert!(matches!(
            vault.load_page(page),
            Err(crate::VaultError::NotFound(_))
        ));
    }

    #[test]
    fn conflict_diff_api_legacy_record_without_frontiers_uses_saved_content() {
        let (_dir, vault, page, _) = setup();
        let saved = Block {
            id: "legacy".into(),
            kind: BlockKind::Paragraph,
            text: "Saved 🐈 text".into(),
            marks: vec![],
            checked: false,
            children: vec![],
            target: None,
            color: None,
            width: None,
        };
        let json = serde_json::json!({"block": "legacy", "time_ms": 7, "sides": [
            {"device": "cloud-device", "block": saved}, {"device": vault.device_id(), "block": null}
        ]})
        .to_string();
        vault
            .inner
            .edit(&page, |doc| doc.set_conflict("legacy", &json))
            .unwrap();
        let diff = vault.conflict_diff(page, "legacy".into()).unwrap();
        assert_eq!(
            diff.scope,
            ConflictScope::Block {
                block: "legacy".into()
            }
        );
        assert_eq!(diff.sides[0].blocks[0].block.text, "Saved 🐈 text");
        assert_eq!(diff.sides[0].blocks[0].change, ConflictDiffChange::Added);
        assert!(diff.sides[1].blocks.is_empty());
        assert!(diff.sides[1].local);
    }

    #[test]
    fn conflict_diff_api_foreign_sides_keep_order_without_claiming_local() {
        let (_dir, vault, page, mut record) = setup();
        record.sides[1].device = "second-cloud-device".into();
        vault
            .inner
            .edit(&page, |doc| crate::sync::write_conflict(doc, &record))
            .unwrap();
        let diff = vault.conflict_diff(page, record.id).unwrap();
        assert!(diff.sides.iter().all(|side| !side.local));
        assert_eq!(diff.sides[0].device, "cloud-device");
        assert_eq!(diff.sides[0].blocks[1].change, ConflictDiffChange::Removed);
        assert_eq!(diff.sides[1].device, "second-cloud-device");
        assert_eq!(diff.sides[1].blocks[1].change, ConflictDiffChange::Added);
    }

    #[test]
    fn conflict_diff_api_malformed_count_and_missing_id_are_errors() {
        let (_dir, vault, page, mut record) = setup();
        record.sides.pop();
        vault
            .inner
            .edit(&page, |doc| crate::sync::write_conflict(doc, &record))
            .unwrap();
        let error = vault
            .conflict_diff(page.clone(), record.id)
            .unwrap_err()
            .to_string();
        assert!(error.contains("exactly two"), "{error}");
        assert!(vault
            .conflict_diff(page, "absent".into())
            .unwrap_err()
            .to_string()
            .contains("conflict"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BlockKind;
    fn block(id: &str, text: &str, children: Vec<Block>) -> Block {
        Block {
            id: id.into(),
            kind: BlockKind::Paragraph,
            text: text.into(),
            marks: vec![],
            checked: false,
            children,
            target: None,
            color: None,
            width: None,
        }
    }
    #[test]
    fn conflict_diff_text_marks_added_removed_and_unicode_offsets() {
        let (left, right) = diff_blocks(
            &[block("a", "🐈 likes tea", vec![])],
            &[block("a", "🐈 likes milk", vec![])],
        );
        assert_eq!(left[0].change, ConflictDiffChange::Modified);
        assert_eq!(
            left[0].spans[0],
            ConflictDiffSpan {
                start: 0,
                end: 8,
                change: ConflictDiffChange::Unchanged
            }
        );
        assert_eq!(left[0].spans[1].change, ConflictDiffChange::Removed);
        assert_eq!(right[0].spans[1].change, ConflictDiffChange::Added);
    }
    #[test]
    fn conflict_diff_added_and_removed_subtrees_keep_every_descendant() {
        let subtree = block(
            "parent",
            "Heading",
            vec![block(
                "child",
                "Nested",
                vec![block("leaf", "Leaf", vec![])],
            )],
        );
        let (removed, empty) = diff_blocks(std::slice::from_ref(&subtree), &[]);
        assert!(empty.is_empty());
        assert_eq!(removed[0].change, ConflictDiffChange::Removed);
        assert_eq!(
            removed[0].children[0].children[0].spans[0].change,
            ConflictDiffChange::Removed
        );
        let (empty, added) = diff_blocks(&[], &[subtree]);
        assert!(empty.is_empty());
        assert_eq!(added[0].change, ConflictDiffChange::Added);
        assert_eq!(
            added[0].children[0].children[0].change,
            ConflictDiffChange::Added
        );
    }
    #[test]
    fn conflict_diff_whole_page_preserves_order_and_unchanged_blocks() {
        let (left, right) = diff_blocks(
            &[block("a", "same", vec![]), block("b", "gone", vec![])],
            &[block("a", "same", vec![]), block("c", "new", vec![])],
        );
        assert_eq!(left[0].change, ConflictDiffChange::Unchanged);
        assert_eq!(right[0].change, ConflictDiffChange::Unchanged);
        assert_eq!(left[1].block.id, "b");
        assert_eq!(right[1].block.id, "c");
        assert_eq!(left[1].change, ConflictDiffChange::Removed);
        assert_eq!(right[1].change, ConflictDiffChange::Added);
    }
    #[test]
    fn conflict_diff_large_replacement_is_bounded_and_covers_each_side() {
        let a = "a".repeat(2000);
        let b = "b".repeat(2000);
        let (left, right) = text_spans(&a, &b);
        assert_eq!(
            left,
            vec![ConflictDiffSpan {
                start: 0,
                end: 2000,
                change: ConflictDiffChange::Removed
            }]
        );
        assert_eq!(
            right,
            vec![ConflictDiffSpan {
                start: 0,
                end: 2000,
                change: ConflictDiffChange::Added
            }]
        );
    }
}
