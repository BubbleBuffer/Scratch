//! Page templates. Any page can hold templates for its new children: child
//! pages marked `istemplate`. One of them can be the default (the parent's
//! `template` meta), which `create_page` copies into each new child.
//! Templates are left out of child lists, page lists, search and queries.
//! A database's default template is the same mechanism with the database
//! as the parent; a default set before templates were marked counts as a
//! template too.

use crate::blockdiff::insert_ops;
use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::index::Index;
use crate::types::{Block, BlockKind, BlockOp, Change, NewBlock, PageId, PageSummary, Property};
use crate::vault::{fresh_ids, lock, Vault};

/// What a new child page starts from.
pub(crate) enum Start {
    /// The parent's default template, if any.
    Default,
    /// Nothing.
    Empty,
    /// This template of the parent.
    Template(PageId),
}

/// Fails unless `t` is a live template of `parent`.
pub(crate) fn check_template(db: &Index, parent: &str, t: &str) -> Result<()> {
    if db.templates(parent)?.iter().any(|s| s.id == t) {
        Ok(())
    } else {
        Err(invalid(format!("page {t} is not a template of {parent}")))
    }
}

/// What a template gives a page: blocks (with fresh ids), properties, icon
/// and cover. Its title and sub-pages are not copied.
pub(crate) struct TemplateCopy {
    blocks: Vec<Block>,
    props: Vec<Property>,
    icon: Option<String>,
    cover: Option<String>,
}

impl TemplateCopy {
    pub fn of(src: &mut PageDoc) -> Self {
        let m = src.meta();
        TemplateCopy {
            blocks: fresh_ids(&src.blocks()),
            props: src.props(),
            icon: m.icon,
            cover: m.cover,
        }
    }

    /// Copies into `doc`. The blocks replace a body of empty paragraphs and
    /// follow any other body; properties, icon and cover fill what is unset.
    pub fn apply_to(&self, doc: &mut PageDoc) -> Result<()> {
        let have = doc.blocks();
        let blank = have
            .iter()
            .all(|b| b.kind == BlockKind::Paragraph && b.text.is_empty() && b.children.is_empty());
        let mut ops: Vec<BlockOp> = Vec::new();
        let mut after = None;
        if blank {
            ops.extend(have.iter().map(|b| BlockOp::Delete { id: b.id.clone() }));
        } else {
            after = have.last().map(|b| b.id.clone());
        }
        let mut inserts = insert_ops(&self.blocks);
        if let Some(BlockOp::Insert { after: a, .. }) = inserts.first_mut() {
            *a = after;
        }
        ops.extend(inserts);
        doc.apply(&ops)?;
        let set: Vec<String> = doc.props().into_iter().map(|p| p.key).collect();
        for p in &self.props {
            if !set.contains(&p.key) {
                doc.set_prop(&p.key, &p.value)?;
            }
        }
        let m = doc.meta();
        if m.icon.is_none() {
            doc.set_meta_str("icon", self.icon.as_deref())?;
        }
        if m.cover.is_none() {
            doc.set_meta_str("cover", self.cover.as_deref())?;
        }
        Ok(())
    }
}

impl TemplateCopy {
    /// Appends the copied blocks at the end of `doc`, under a divider and a
    /// "Merged from" line naming `merged_from`. Properties, icon and cover
    /// fill only what `doc` has not set. Used by `merge_pages`.
    pub fn append_after_divider(&self, doc: &mut PageDoc, merged_from: &str) -> Result<()> {
        let have = doc.blocks();
        let divider = crate::ids::new_id();
        let note = crate::ids::new_id();
        let plain = |id: &str, kind: BlockKind, text: String| NewBlock {
            id: id.to_owned(),
            kind,
            text,
            marks: vec![],
            checked: false,
        };
        let mut ops = vec![
            BlockOp::Insert {
                parent: None,
                after: have.last().map(|b| b.id.clone()),
                block: plain(&divider, BlockKind::Divider, String::new()),
            },
            BlockOp::Insert {
                parent: None,
                after: Some(divider),
                block: plain(&note, BlockKind::Paragraph, format!("Merged from {merged_from}")),
            },
        ];
        let mut inserts = insert_ops(&self.blocks);
        if let Some(BlockOp::Insert { after, .. }) = inserts.first_mut() {
            *after = Some(note);
        }
        ops.extend(inserts);
        doc.apply(&ops)?;
        let set: Vec<String> = doc.props().into_iter().map(|p| p.key).collect();
        for p in &self.props {
            if !set.contains(&p.key) {
                doc.set_prop(&p.key, &p.value)?;
            }
        }
        let m = doc.meta();
        if m.icon.is_none() {
            doc.set_meta_str("icon", self.icon.as_deref())?;
        }
        if m.cover.is_none() {
            doc.set_meta_str("cover", self.cover.as_deref())?;
        }
        Ok(())
    }
}

impl Vault {
    /// The templates for new children of `parent`, in sibling order.
    pub fn list_templates(&self, parent: PageId) -> Result<Vec<PageSummary>> {
        let db = lock(&self.inner.db);
        if !db.exists(&parent)? {
            return Err(not_found("page", &parent));
        }
        db.templates(&parent)
    }

    /// Creates an empty template for new children of `parent` (at the end
    /// of its children) and returns its id.
    pub fn create_template(&self, parent: PageId, title: String) -> Result<PageId> {
        self.create_child(Some(parent), title, Start::Empty, true)
    }

    /// Creates a child of `parent` from its template `template` (`None`: an
    /// empty page, even when the parent has a default template).
    pub fn create_page_from(
        &self,
        parent: PageId,
        title: String,
        template: Option<PageId>,
    ) -> Result<PageId> {
        let start = match template {
            Some(t) => Start::Template(t),
            None => Start::Empty,
        };
        self.create_child(Some(parent), title, start, false)
    }

    /// Creates the page `id` under `parent`, as a copy of the parent's
    /// template `template` (`None`: an empty page). The caller's id is used
    /// exactly. If `id` is in use (a page, the trash, or a purge), nothing
    /// changes and the result is `(id, false)`; a trashed page is not restored.
    /// Otherwise the result is `(id, true)`.
    pub fn create_page_from_identified(
        &self,
        id: String,
        parent: String,
        title: String,
        template: Option<String>,
    ) -> Result<(String, bool)> {
        self.create_unused_page(&id, &parent, &title, template, |_| Ok(()))
    }

    /// Makes a child page a template of its parent, or a plain page again
    /// (which also clears it as the parent's default).
    pub fn set_template(&self, page: PageId, on: bool) -> Result<()> {
        let db = lock(&self.inner.db);
        let parent = db
            .parent_of(&page)?
            .ok_or_else(|| not_found("page", &page))?
            .ok_or_else(|| invalid("a top-level page cannot be a template"))?;
        if !on && db.template_of(&parent)?.as_deref() == Some(page.as_str()) {
            self.inner
                .edit(&parent, |d| d.set_meta_str("template", None))?;
            self.inner.sync_meta(&db, &parent)?;
        }
        self.inner.edit(&page, |d| d.set_is_template(on))?;
        self.inner.sync_meta(&db, &page)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    /// Copies the template `template` of the page's parent into `page`
    /// (`TemplateCopy::apply_to`): the template picker of a new page.
    pub fn apply_template(&self, page: PageId, template: PageId) -> Result<()> {
        let db = lock(&self.inner.db);
        let parent = db
            .parent_of(&page)?
            .ok_or_else(|| not_found("page", &page))?
            .ok_or_else(|| invalid("a top-level page has no templates"))?;
        check_template(&db, &parent, &template)?;
        let copy = self.inner.read(&template, TemplateCopy::of)?;
        self.inner.edit(&page, |d| copy.apply_to(d))?;
        self.inner.sync_meta(&db, &page)
    }
}
