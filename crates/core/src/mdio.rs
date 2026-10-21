//! Markdown in and out of pages, for AI assistants (the MCP server) and the
//! agent runner: a page or a list of blocks as markdown (the same syntax as
//! the vault's `.md` files, without frontmatter), markdown parsed into
//! blocks, and edits that insert or replace blocks from markdown.

use std::collections::HashMap;

use crate::attach;
use crate::blockdiff::{assign_ids, diff_ops, insert_ops};
use crate::error::{invalid, Result};
use crate::index::Index;
use crate::markdown::{self, RenderCtx};
use crate::mdparse::{self, Resolver};
use crate::types::{Block, BlockId, BlockOp, PageId};
use crate::vault::{fresh_ids, lock, peer_of, Vault};

/// Resolves wikilinks by page id or title (live pages first) and
/// attachment links by attachment id.
struct TitleResolver<'a> {
    db: &'a Index,
    root: &'a std::path::Path,
}

impl Resolver for TitleResolver<'_> {
    fn page(&mut self, target: &str) -> Option<String> {
        let t = target.trim();
        let t = t.strip_suffix(".md").unwrap_or(t);
        if self.db.exists(t).unwrap_or(false) {
            return Some(t.to_string());
        }
        self.db.id_by_title(t).ok().flatten()
    }

    fn file(&mut self, path: &str) -> Option<String> {
        let last = path.trim().rsplit(['/', '\\']).next().unwrap_or(path);
        (attach::is_attachment_id(last) && attach::blob_path(self.root, last).exists())
            .then(|| last.to_string())
    }
}

/// Parses and renders Markdown with the vault's syntax, without opening a
/// vault, allocating page/block ids, or resolving page and attachment links.
///
/// This is a pure serialization normalizer: inline marks, block formatting,
/// code fence languages and whitespace follow the existing parser/renderer.
/// Code content remains significant to the extent supported by that grammar.
/// It cannot infer equivalence when a vault resolves an id, title, heading or
/// `.md` link into a different displayed page-link target or alias.
pub fn normalize_markdown(markdown: &str) -> String {
    struct Unresolved;

    impl Resolver for Unresolved {
        fn page(&mut self, _target: &str) -> Option<String> {
            None
        }

        fn file(&mut self, _path: &str) -> Option<String> {
            None
        }
    }

    let blocks = mdparse::parse_blocks(markdown, &mut Unresolved);
    let ctx = RenderCtx::none();
    let names = markdown::link_names(&blocks, &[], &ctx);
    let mut out = String::new();
    markdown::render_blocks(&blocks, "", &names, ctx.titles, &mut out);
    out.trim_matches('\n').to_string()
}

impl Vault {
    /// Whether this instance holds the vault's lock (`.vault/lock`), that
    /// is, it is the only instance writing as this device. A second
    /// instance on the same folder (another process, such as the app) gets
    /// `false`.
    pub fn is_owner(&self) -> bool {
        self.inner.peer == peer_of(&self.inner.device)
    }

    /// The page's blocks as markdown (no frontmatter, no title). Page links
    /// are wikilinks by title (by id where the title is ambiguous).
    pub fn page_markdown(&self, id: PageId) -> Result<String> {
        let page = self.load_page(id)?;
        Ok(self.blocks_markdown(page.blocks))
    }

    /// Blocks as markdown, as in [`Vault::page_markdown`].
    pub fn blocks_markdown(&self, blocks: Vec<Block>) -> String {
        let db = lock(&self.inner.db);
        let titles = |pid: &str| db.title_of(pid).ok().flatten();
        let ctx = RenderCtx {
            titles: &titles,
            schema: None,
        };
        let names = markdown::link_names(&blocks, &[], &ctx);
        let mut out = String::new();
        markdown::render_blocks(&blocks, "", &names, &titles, &mut out);
        // No blank lines around the blocks.
        out.trim_matches('\n').to_string()
    }

    /// Parses markdown into blocks with new ids. Wikilinks resolve to pages
    /// by id or title; unresolved ones stay text.
    pub fn parse_markdown(&self, markdown: String) -> Vec<Block> {
        let db = lock(&self.inner.db);
        let mut r = TitleResolver {
            db: &db,
            root: &self.inner.root,
        };
        fresh_ids(&mdparse::parse_blocks(&markdown, &mut r))
    }

    /// Inserts the blocks of `markdown` into page `id` under `parent` (None:
    /// top level) after `after` (None: as the first child; see
    /// [`BlockOp::Insert`]). With `after` None and `at_end` true they are
    /// appended after the last child instead. Returns the new top-level
    /// block ids. One change, all or nothing.
    pub fn insert_markdown(
        &self,
        id: PageId,
        parent: Option<BlockId>,
        after: Option<BlockId>,
        at_end: bool,
        markdown: String,
    ) -> Result<Vec<BlockId>> {
        let blocks = self.parse_markdown(markdown);
        if blocks.is_empty() {
            return Ok(Vec::new());
        }
        let after = match (after, at_end) {
            (Some(a), _) => Some(a),
            (None, false) => None,
            (None, true) => {
                let page = self.load_page(id.clone())?;
                let siblings = match &parent {
                    None => Some(&page.blocks),
                    Some(p) => find(&page.blocks, p).map(|b| &b.children),
                };
                siblings
                    .ok_or_else(|| {
                        invalid(format!("no block {}", parent.clone().unwrap_or_default()))
                    })?
                    .last()
                    .map(|b| b.id.clone())
            }
        };
        let ops = insert_after(&blocks, parent, after);
        let ids = blocks.iter().map(|b| b.id.clone()).collect();
        self.apply(id, ops)?;
        Ok(ids)
    }

    /// Replaces blocks of page `id` with the blocks of `markdown`.
    ///
    /// - `blocks` empty: the whole page. Unchanged blocks keep their ids
    ///   (the same diff as outside edits to a `.md` file).
    /// - Otherwise the listed blocks (with their children) are deleted and
    ///   the new blocks take the place of the first one. Returns the new
    ///   top-level block ids. One change, all or nothing.
    pub fn replace_markdown(
        &self,
        id: PageId,
        blocks: Vec<BlockId>,
        markdown: String,
    ) -> Result<Vec<BlockId>> {
        let page = self.load_page(id.clone())?;
        let mut parsed = self.parse_markdown(markdown);
        if blocks.is_empty() {
            assign_ids(&page.blocks, &mut parsed);
            let ops = diff_ops(&page.blocks, &parsed);
            self.apply(id, ops)?;
            return Ok(parsed.into_iter().map(|b| b.id).collect());
        }
        let first = &blocks[0];
        let parent = parent_of(&page.blocks, None, first)
            .ok_or_else(|| invalid(format!("no block {first} in page")))?;
        for b in &blocks[1..] {
            if find(&page.blocks, b).is_none() {
                return Err(invalid(format!("no block {b} in page")));
            }
        }
        // Insert after the first block, then delete the old ones.
        let mut ops = insert_after(&parsed, parent, Some(first.clone()));
        ops.extend(blocks.iter().map(|b| BlockOp::Delete { id: b.clone() }));
        self.apply(id, ops)?;
        Ok(parsed.into_iter().map(|b| b.id).collect())
    }
}

/// Ops inserting `blocks` (with their ids) under `parent` after `after`.
fn insert_after(blocks: &[Block], parent: Option<BlockId>, after: Option<BlockId>) -> Vec<BlockOp> {
    // `insert_ops` builds an empty page: retarget its top-level inserts.
    let top: HashMap<&str, usize> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    let mut ops = insert_ops(blocks);
    for op in &mut ops {
        if let BlockOp::Insert {
            parent: p,
            after: a,
            block,
        } = op
        {
            if p.is_none() {
                if let Some(&i) = top.get(block.id.as_str()) {
                    *p = parent.clone();
                    if i == 0 {
                        *a = after.clone();
                    }
                }
            }
        }
    }
    ops
}

fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    for b in blocks {
        if b.id == id {
            return Some(b);
        }
        if let Some(x) = find(&b.children, id) {
            return Some(x);
        }
    }
    None
}

/// The parent of block `id` (Some(None): top level; None: not found).
fn parent_of(blocks: &[Block], parent: Option<&BlockId>, id: &str) -> Option<Option<BlockId>> {
    for b in blocks {
        if b.id == id {
            return Some(parent.cloned());
        }
        if let Some(x) = parent_of(&b.children, Some(&b.id), id) {
            return Some(x);
        }
    }
    None
}

#[cfg(test)]
mod normalization_tests {
    use super::normalize_markdown;

    #[test]
    fn rich_markdown_uses_existing_serialization_and_is_repeatable() {
        let source = "\r\n#### Heading ####\r\n\r\n_italic_ and **bold** [site](https://example.com/path).\r\n\r\n* first\r\n+ second\r\n\r\n```rust\r\nlet value = 1;  \r\n```\r\n";
        let expected = "### Heading\n\n*italic* and **bold** [site](https://example.com/path).\n\n- first\n- second\n\n```rust\nlet value = 1;  \n```";
        assert_eq!(normalize_markdown(source), expected);
        assert_eq!(normalize_markdown(expected), expected);
    }

    #[test]
    fn full_metadata_body_keeps_nested_yaml_and_detects_edits() {
        let source = "# Source body\n\n_italic_\n\n## Imported metadata\n\n```yaml\nmetadata:\n  modified: original\n  nested:\n    values: [one, two]\n```\n";
        let normalized = normalize_markdown(source);
        assert!(normalized.contains("## Imported metadata\n\n```yaml\nmetadata:\n  modified: original\n  nested:\n    values: [one, two]\n```"));
        assert_eq!(normalize_markdown(&normalized), normalized);
        assert_ne!(
            normalize_markdown(&source.replace("modified: original", "modified: changed")),
            normalized
        );
        assert_ne!(
            normalize_markdown(&source.replace("Source body", "Edited body")),
            normalized
        );
    }

    #[test]
    fn meaningful_text_code_and_paragraph_changes_stay_distinct() {
        for (left, right) in [
            ("first\n\nsecond", "first\nsecond"),
            ("- item\n\nparagraph", "- item\ncontinuation"),
            ("```\nx  \n```", "```\nx\n```"),
            ("*text*", "\\*text\\*"),
            (
                "[site](https://example.com/a)",
                "[site](https://example.com/b)",
            ),
            ("[[Target|Alias]]", "[[Target|Other]]"),
        ] {
            assert_ne!(normalize_markdown(left), normalize_markdown(right));
        }
    }

    #[test]
    fn normalization_does_not_guess_resolved_page_identity() {
        // A vault might render an id by its current title, or resolve a .md
        // link. Without that vault's lookup data these forms stay distinct.
        assert_ne!(
            normalize_markdown("[[page-id]]"),
            normalize_markdown("[[Current title|page-id]]")
        );
        assert_ne!(
            normalize_markdown("[alias](page.md)"),
            normalize_markdown("[[page|alias]]")
        );
    }
}
