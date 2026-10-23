//! The page tree: sibling order, moves, duplicates, trash, favorites, icon,
//! cover and display options.
//!
//! Sibling order is the `order` key in each page's document (see `order.rs`);
//! the trash is the `trashed` time in the trashed page's document. The index
//! mirrors both (`sort_key`, and `in_trash` for the whole subtree).

use crate::attach;
use crate::blockdiff::insert_ops;
use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::ids::{new_id, now_ms, stamp_ms};
use crate::index::Index;
use crate::order;
use crate::types::{AttachmentId, Change, PageId, PagePrefs, PageSummary, TrashedPage};
use crate::vault::{fresh_ids, lock, Vault};

/// Whether the page is in the trash; NotFound if it does not exist.
fn trash_state(db: &Index, id: &str) -> Result<bool> {
    db.in_trash(id)?.ok_or_else(|| not_found("page", id))
}

fn live(db: &Index, id: &str) -> Result<()> {
    if trash_state(db, id)? {
        return Err(invalid(format!("page {id} is in the trash")));
    }
    Ok(())
}

impl Vault {
    /// Children of `parent` (`None`: top-level pages) in sibling order, as
    /// the sidebar shows them. Pages in the trash are left out.
    pub fn list_children(
        &self,
        parent: Option<PageId>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<PageSummary>> {
        lock(&self.inner.db).children(parent.as_deref(), offset, limit)
    }

    /// Moves a page under `new_parent` (`None`: top level), right after the
    /// sibling `after` (`None`: first). Its subtree moves with it.
    pub fn move_page(
        &self,
        id: PageId,
        new_parent: Option<PageId>,
        after: Option<PageId>,
    ) -> Result<()> {
        let db = lock(&self.inner.db);
        live(&db, &id)?;
        if let Some(p) = &new_parent {
            live(&db, p)?;
            if db.is_ancestor(&id, p)? {
                return Err(invalid("cannot move a page into itself"));
            }
        }
        let mut sibs = db.child_keys(new_parent.as_deref())?;
        sibs.retain(|(s, _)| *s != id);
        // Empty or colliding keys (old vaults, concurrent moves): re-key.
        if sibs.iter().any(|(_, k)| k.is_empty()) || sibs.windows(2).any(|w| w[0].1 >= w[1].1) {
            let keys = order::spread(sibs.len());
            for ((sid, k), nk) in sibs.iter_mut().zip(keys) {
                self.inner
                    .edit(sid, |d| d.set_meta_str("order", Some(&nk)))?;
                self.inner.sync_meta(&db, sid)?;
                *k = nk;
            }
        }
        let pos = match &after {
            None => 0,
            Some(a) => {
                sibs.iter()
                    .position(|(s, _)| s == a)
                    .ok_or_else(|| invalid(format!("page {a} is not a child of the new parent")))?
                    + 1
            }
        };
        let lo = pos.checked_sub(1).map(|i| sibs[i].1.as_str());
        let hi = sibs.get(pos).map(|x| x.1.as_str());
        let key = order::between(lo, hi);
        self.inner.edit(&id, |d| {
            d.set_meta_str("parent", new_parent.as_deref())?;
            d.set_meta_str("order", Some(&key))
        })?;
        self.inner.sync_meta(&db, &id)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    /// Copies the page and its subtree (blocks, properties, icon, cover,
    /// schema and views; not the favorite flag) right after the original.
    /// The copy's title gets " (copy)". Returns the copy's id.
    pub fn duplicate_page(&self, id: PageId) -> Result<PageId> {
        let db = lock(&self.inner.db);
        live(&db, &id)?;
        let parent = db.parent_of(&id)?.flatten();
        let sibs = db.child_keys(parent.as_deref())?;
        let pos = sibs.iter().position(|(s, _)| *s == id).unwrap_or(0);
        let lo = sibs.get(pos).map(|x| x.1.as_str());
        let hi = sibs.get(pos + 1).map(|x| x.1.as_str());
        let key = order::between(lo, hi);
        let copy = self.copy_tree(&db, &id, parent, key, true)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(copy)
    }

    fn copy_tree(
        &self,
        db: &Index,
        src: &str,
        parent: Option<String>,
        key: String,
        root: bool,
    ) -> Result<PageId> {
        let (blocks, props, meta, schema, views) = self.inner.read(src, |d| {
            (
                fresh_ids(&d.blocks()),
                d.props(),
                d.meta(),
                d.schema(),
                d.views(),
            )
        })?;
        let id = new_id();
        let title = if root {
            format!("{} (copy)", meta.title)
        } else {
            meta.title.clone()
        };
        let mut doc = PageDoc::new_page(
            self.inner.actor_peer(),
            &id,
            &title,
            parent.as_deref(),
            stamp_ms(),
        )?;
        doc.set_meta_str("order", Some(&key))?;
        doc.set_meta_str("icon", meta.icon.as_deref())?;
        doc.set_meta_str("cover", meta.cover.as_deref())?;
        doc.set_prefs(&meta.prefs)?;
        doc.set_meta_str("agenda_role", meta.agenda_role.as_deref())?;
        doc.set_meta_str("agenda_date", meta.agenda_date.as_deref())?;
        if parent.is_none() && !meta.is_template {
            // A duplicated top-level page starts uncoloured, whatever its source.
            doc.set_meta_str("color", Some("none"))?;
        } else {
            doc.set_meta_str("color", meta.color.as_deref())?;
        }
        doc.apply(&insert_ops(&blocks))?;
        for p in &props {
            doc.set_prop(&p.key, &p.value)?;
        }
        doc.set_page_type(&meta.page_type)?;
        if meta.page_type == crate::types::PAGE_TYPE_DATABASE {
            doc.set_schema(&schema.unwrap_or_default())?;
            doc.set_views(&views)?;
        }
        if meta.is_template {
            doc.set_is_template(true)?;
        }
        self.inner.install(db, doc)?;
        let mut template = None;
        for (child, ckey) in db.child_keys(Some(src))? {
            let c = self.copy_tree(db, &child, Some(id.clone()), ckey, false)?;
            if meta.template.as_deref() == Some(child.as_str()) {
                template = Some(c);
            }
        }
        if let Some(t) = template {
            self.inner
                .edit(&id, |d| d.set_meta_str("template", Some(&t)))?;
            self.inner.sync_meta(db, &id)?;
        }
        Ok(id)
    }

    /// Moves the page and its subtree to the trash (`delete_page`).
    pub(crate) fn trash_page(&self, id: PageId) -> Result<()> {
        let db = lock(&self.inner.db);
        // Already in the trash (itself or through an ancestor): nothing to do.
        if trash_state(&db, &id)? {
            return Ok(());
        }
        self.inner
            .edit(&id, |d| d.set_meta_i64("trashed", Some(stamp_ms())))?;
        self.inner.sync_meta(&db, &id)?;
        let pages = db.subtree(&id)?;
        for (pid, md) in &pages {
            self.inner.remove_md(&db, pid, md.as_deref())?;
        }
        self.inner
            .index_committed(&db, pages.iter().map(|(id, _)| id.clone()).collect());
        drop(db);
        self.inner.emit(
            pages
                .into_iter()
                .map(|(p, _)| Change::PageDeleted(p))
                .chain([Change::TreeChanged]),
        );
        Ok(())
    }

    /// Brings a page in the trash back with its subtree (pages inside it
    /// that were trashed on their own stay in the trash). If its parent is
    /// gone or still in the trash, it comes back at the top level.
    pub fn restore_page(&self, id: PageId) -> Result<()> {
        let db = lock(&self.inner.db);
        if !trash_state(&db, &id)? {
            return Ok(());
        }
        let parent = db.parent_of(&id)?.flatten();
        let parent_ok = match &parent {
            Some(p) => db.in_trash(p)? == Some(false),
            None => true,
        };
        let key = if parent_ok {
            None
        } else {
            Some(order::between(db.last_child_key(None)?.as_deref(), None))
        };
        self.inner.edit(&id, |d| {
            d.set_meta_i64("trashed", None)?;
            if let Some(k) = &key {
                d.set_meta_str("parent", None)?;
                d.set_meta_str("order", Some(k))?;
            }
            Ok(())
        })?;
        self.inner.sync_meta(&db, &id)?;
        let mut changes = Vec::new();
        for (pid, _) in db.subtree(&id)? {
            if db.in_trash(&pid)? == Some(false) {
                self.inner.rerender(&pid);
                changes.push(Change::PageCreated(pid));
            }
        }
        self.inner.index_committed(
            &db,
            db.subtree(&id)?.into_iter().map(|(id, _)| id).collect(),
        );
        drop(db);
        changes.push(Change::TreeChanged);
        self.inner.emit(changes);
        Ok(())
    }

    /// Pages moved to the trash (each with its subtree), newest first.
    pub fn list_trash(&self) -> Result<Vec<TrashedPage>> {
        lock(&self.inner.db).trash()
    }

    /// Deletes a page in the trash and its subtree for good: documents,
    /// logs and index rows. Attachments stay until `gc_attachments`.
    pub fn purge_page(&self, id: PageId) -> Result<()> {
        let _purge = lock(&self.inner.purge_sync);
        let db = lock(&self.inner.db);
        if !trash_state(&db, &id)? {
            return Err(invalid(format!("page {id} is not in the trash")));
        }
        let ids = self.purge(&db, &id)?;
        drop(db);
        self.inner.emit(
            ids.into_iter()
                .map(Change::PageDeleted)
                .chain([Change::TreeChanged]),
        );
        Ok(())
    }

    fn purge(&self, db: &Index, id: &str) -> Result<Vec<String>> {
        let pages = db.subtree(id)?;
        let ids: Vec<String> = pages.iter().map(|(p, _)| p.clone()).collect();
        // Capture the actual documents, including unflushed local edits.
        let mut records = Vec::with_capacity(ids.len());
        for pid in &ids {
            let slot = self.inner.slot(pid)?;
            let st = lock(&slot.state);
            records.push(crate::sync::PurgeRecord {
                page: pid.clone(),
                version: Some(st.doc.doc.oplog_vv().encode()),
                device: Some(self.inner.device.clone()),
            });
        }
        self.inner.add_purge_records(&records)?;
        for (pid, md) in &pages {
            self.inner.purge_files(pid, md.as_deref(), None)?;
        }
        db.delete(&ids)?;
        self.inner.index_committed(db, ids.clone());
        Ok(ids)
    }

    /// Purges everything in the trash. Returns the number of pages deleted.
    pub fn empty_trash(&self) -> Result<u32> {
        let _purge = lock(&self.inner.purge_sync);
        let db = lock(&self.inner.db);
        let mut all = Vec::new();
        for t in db.trash()? {
            if db.exists(&t.page.id)? {
                all.extend(self.purge(&db, &t.page.id)?);
            }
        }
        drop(db);
        let n = all.len() as u32;
        if n > 0 {
            self.inner.emit(
                all.into_iter()
                    .map(Change::PageDeleted)
                    .chain([Change::TreeChanged]),
            );
        }
        Ok(n)
    }

    /// Adds the page to or removes it from the favorites.
    pub fn set_favorite(&self, id: PageId, favorite: bool) -> Result<()> {
        let db = lock(&self.inner.db);
        trash_state(&db, &id)?;
        // Favorites are listed by this time: keep it increasing.
        let at = now_ms().max(db.last_favorite()?.map_or(0, |t| t + 1));
        self.inner.edit(&id, |d| {
            let cur = d.meta().favorite;
            match (favorite, cur) {
                (true, None) => d.set_meta_i64("favorite", Some(at)),
                (false, Some(_)) => d.set_meta_i64("favorite", None),
                _ => Ok(()),
            }
        })?;
        self.inner.sync_meta(&db, &id)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    /// Favorite pages not in the trash, in the order they were favorited.
    pub fn list_favorites(&self) -> Result<Vec<PageSummary>> {
        lock(&self.inner.db).favorites()
    }

    /// Sets the page icon (an emoji; `None` or empty removes it).
    pub fn set_icon(&self, id: PageId, icon: Option<String>) -> Result<()> {
        let icon = icon.filter(|s| !s.trim().is_empty());
        let db = lock(&self.inner.db);
        trash_state(&db, &id)?;
        self.inner
            .edit(&id, |d| d.set_meta_str("icon", icon.as_deref()))?;
        self.inner.sync_meta(&db, &id)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    /// Sets the page cover to an attachment (`None` removes it).
    pub fn set_cover(&self, id: PageId, cover: Option<AttachmentId>) -> Result<()> {
        if let Some(c) = &cover {
            if !attach::is_attachment_id(c) || !attach::blob_path(&self.inner.root, c).exists() {
                return Err(not_found("attachment", c));
            }
        }
        let db = lock(&self.inner.db);
        trash_state(&db, &id)?;
        self.inner
            .edit(&id, |d| d.set_meta_str("cover", cover.as_deref()))?;
        self.inner.sync_meta(&db, &id)
    }

    /// Sets the page's display options (small text, full width, lock, cover
    /// position). They are stored in the page document and sync with it.
    pub fn set_page_prefs(&self, id: PageId, prefs: PagePrefs) -> Result<()> {
        if prefs.cover_position.is_some_and(|p| p > 1000) {
            return Err(invalid("cover position must be 0 to 1000"));
        }
        trash_state(&lock(&self.inner.db), &id)?;
        self.inner.edit(&id, |d| d.set_prefs(&prefs))
    }
}
