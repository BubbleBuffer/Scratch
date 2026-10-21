//! Links and backlinks, attachments, page history and the change feed.

use std::path::Path;

use crate::attach;
use crate::blockdiff::diff_ops;
use crate::error::{not_found, Result};
use crate::history;
use crate::ids::now_ms;
use crate::types::{
    Attachment, AttachmentId, Change, ChangeBatch, Page, PageId, PageSummary, Version,
};
use crate::vault::{lock, page_of, prop_map, Vault};

impl Vault {
    /// Pages not in the trash that link to `id` (inline page links and link
    /// blocks), most recently updated first. Reflects edits once they are
    /// flushed (like `search`).
    pub fn backlinks(&self, id: PageId) -> Result<Vec<PageSummary>> {
        lock(&self.inner.db).backlinks(&id)
    }

    fn register_attachment(&self, id: &str, name: &str, size: i64) -> Result<()> {
        lock(&self.inner.db).put_attachment(
            &Attachment {
                id: id.to_string(),
                name: name.to_string(),
                mime: attach::mime_of(name).to_string(),
                size,
            },
            now_ms(),
        )
    }

    /// Copies a file into the vault's blob store (content-addressed: the same
    /// bytes are stored once) and returns its attachment id. Reference it
    /// from an `Image` or `File` block's target or as a cover.
    pub fn add_attachment_from_path(&self, path: String) -> Result<AttachmentId> {
        let src = Path::new(&path);
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let (id, size) = attach::store_file(&self.inner.root, src)?;
        self.register_attachment(&id, &name, size)?;
        Ok(id)
    }

    /// Stores bytes as an attachment named `name`; see
    /// `add_attachment_from_path`.
    pub fn add_attachment_bytes(&self, name: String, bytes: Vec<u8>) -> Result<AttachmentId> {
        let (id, size) = attach::store_bytes(&self.inner.root, &bytes)?;
        self.register_attachment(&id, &name, size)?;
        Ok(id)
    }

    /// The attachment's file path, for the interface to display or open.
    pub fn attachment_path(&self, id: AttachmentId) -> Result<String> {
        if !attach::is_attachment_id(&id) {
            return Err(not_found("attachment", &id));
        }
        let p = attach::blob_path(&self.inner.root, &id);
        if !p.exists() {
            return Err(not_found("attachment", &id));
        }
        Ok(p.to_string_lossy().into_owned())
    }

    /// Name (of the first upload), MIME type and size of an attachment.
    pub fn attachment_info(&self, id: AttachmentId) -> Result<Attachment> {
        let path = self.attachment_path(id.clone())?;
        if let Some(a) = lock(&self.inner.db).attachment(&id)? {
            return Ok(a);
        }
        Ok(Attachment {
            size: std::fs::metadata(path)?.len() as i64,
            name: id.clone(),
            mime: "application/octet-stream".into(),
            id,
        })
    }

    /// Deletes every stored attachment that no page (in the trash or not)
    /// uses as an image, file or cover. Flushes first. Returns the number
    /// of blobs removed.
    pub fn gc_attachments(&self) -> Result<u32> {
        self.flush()?;
        let db = lock(&self.inner.db);
        let used = db.referenced_blobs()?;
        let mut n = 0;
        for (id, _) in attach::list_blobs(&self.inner.root)? {
            if !used.contains(&id) {
                crate::vault::remove_if_exists(&attach::blob_path(&self.inner.root, &id))?;
                db.delete_attachment(&id)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// The page's versions, newest first. A version groups the changes of
    /// one device until 5 idle minutes pass.
    pub fn list_versions(&self, page: PageId) -> Result<Vec<Version>> {
        let doc = self.load_conflict_history_doc(&page)?;
        Ok(history::versions(&doc))
    }

    /// The page as it was at a version (from `list_versions`).
    pub fn load_page_at(&self, page: PageId, version: String) -> Result<Page> {
        let frontiers = history::decode_version(&version)?;
        let doc = self.load_conflict_history_doc(&page)?;
        let mut old = doc.fork_at(&frontiers)?;
        let blocks = old.blocks();
        let trashed = old.meta().trashed.is_some();
        Ok(page_of(&old, blocks, trashed))
    }

    /// Makes the page look as it did at `version` (blocks, title, icon,
    /// cover, properties), recorded as a new change so no history is lost.
    /// Its place in the tree, trash state, favorite flag, schema and views
    /// are kept.
    pub fn restore_version(&self, page: PageId, version: String) -> Result<()> {
        let frontiers = history::decode_version(&version)?;
        let doc = self.load_conflict_history_doc(&page)?;
        let mut old = doc.fork_at(&frontiers)?;
        let (old_blocks, old_meta, old_props) = (old.blocks(), old.meta(), old.props());
        // A cover deleted by `gc_attachments` since is not restored.
        let cover = old_meta
            .cover
            .clone()
            .filter(|c| attach::blob_path(&self.inner.root, c).exists());
        let purged = self.inner.is_purged(&page);
        let edit = |d: &mut crate::doc::PageDoc| {
            let cur = d.blocks();
            let ops = diff_ops(&cur, &old_blocks);
            if !ops.is_empty() {
                d.apply(&ops)?;
            }
            let meta = d.meta();
            if meta.title != old_meta.title {
                d.set_title(&old_meta.title)?;
            }
            d.set_meta_str("icon", old_meta.icon.as_deref())?;
            d.set_meta_str("cover", cover.as_deref())?;
            let want = prop_map(&old_props);
            for p in d.props() {
                if !want.contains_key(&p.key) {
                    d.remove_prop(&p.key)?;
                }
            }
            for p in &old_props {
                d.set_prop(&p.key, &p.value)?;
            }
            if purged {
                d.set_meta_i64("trashed", old_meta.trashed)?;
            }
            Ok(meta.title != old_meta.title)
        };
        let renamed = if purged {
            self.restore_archived_page_from_history(&page, edit)?
        } else {
            self.inner.edit(&page, edit)?
        };
        let db = lock(&self.inner.db);
        self.inner.sync_meta(&db, &page)?;
        if renamed {
            for src in db.link_sources(&page)? {
                self.inner.rerender(&src);
            }
            drop(db);
            self.inner.emit([Change::TreeChanged]);
        }
        Ok(())
    }

    /// Changes since `since_seq` (0 at first): pages changed, created or
    /// deleted, and whether the page tree changed. Pass the returned `seq`
    /// next time. `reset` means the feed cannot tell (too far behind, or a
    /// seq from before the vault was opened): reload everything. Repeated
    /// changes to one page are reported once. This includes index-commit
    /// notifications; sync transport uses its separate `poll_sync_changes`
    /// cursor so index-only work cannot start another sync.
    pub fn poll_changes(&self, since_seq: u32) -> ChangeBatch {
        self.inner.poll(since_seq)
    }
}
