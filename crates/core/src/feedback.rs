//! Feedback reports are ordinary child pages of a hidden system page.
use rusqlite::OptionalExtension;

use crate::doc::PageDoc;
use crate::error::{invalid, Result};
use crate::ids::stamp_ms;
use crate::types::PageId;
use crate::vault::{lock, Vault};

// Devices initializing the same vault must create the same document.
const FEEDBACK_PAGE: &str = "000000000000000000000000FB";

impl Vault {
    /// Returns the hidden Feedback page, creating it on first use.
    pub fn feedback_page(&self) -> Result<PageId> {
        // Keep lookup and installation under one lock for concurrent callers.
        let db = lock(&self.inner.db);
        if let Some(id) = db
            .conn
            .prepare_cached("SELECT id FROM pages WHERE role = 'feedback' AND in_trash = 0 ORDER BY created_ms, id LIMIT 1")?
            .query_row([], |r| r.get::<_, String>(0))
            .optional()?
        {
            return Ok(id);
        }
        if db.exists(FEEDBACK_PAGE)? || self.inner.is_purged(FEEDBACK_PAGE) {
            return Err(invalid(
                "feedback page has lost its role, is in trash or was purged",
            ));
        }
        let doc = PageDoc::new_page(
            self.inner.actor_peer(),
            FEEDBACK_PAGE,
            "Feedback",
            None,
            stamp_ms(),
        )?;
        doc.set_meta_str("role", Some("feedback"))?;
        self.inner.install(&db, doc)
    }
}
