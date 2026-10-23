//! Complete page histories retained for conflict review after physical purge.
//!
//! These are the original page's Loro documents, never a global conflict
//! document. The Vault owns lifecycle/index state and durable archive I/O.

use crate::doc::PageDoc;
use crate::error::{invalid, Result};

pub(crate) fn validate(page: &str, doc: &PageDoc) -> Result<()> {
    if page.is_empty() || page.contains(['/', '\\']) || page == "." || page == ".." {
        return Err(invalid("invalid conflict archive page id"));
    }
    if doc.meta().id != page {
        return Err(invalid("conflict archive belongs to another page"));
    }
    if doc.doc.is_shallow() {
        return Err(invalid("conflict archive requires complete page history"));
    }
    Ok(())
}

/// Merge complete histories without changing the caller's live document. The
/// original frontiers remain addressable after cached/disk/archive branches
/// have all been retained. The archive's content is not a lifecycle decision.
pub(crate) fn merge(page: &str, current: &PageDoc, other: Option<&PageDoc>) -> Result<PageDoc> {
    current.doc.commit();
    let mut merged = PageDoc::from_bytes(&current.full_snapshot()?, &[])?;
    validate(page, &merged)?;
    if let Some(other) = other {
        other.doc.commit();
        let complete = PageDoc::from_bytes(&other.full_snapshot()?, &[])?;
        validate(page, &complete)?;
        merged.import_updates(&complete.full_snapshot()?)?;
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockKind, BlockOp, NewBlock};

    #[test]
    fn merging_retains_both_branch_frontiers_and_does_not_edit_the_sources() {
        let mut a = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        a.apply(&[BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: "block".into(),
                kind: BlockKind::Paragraph,
                text: "Original".into(),
                marks: vec![],
                checked: false,
            },
        }])
        .unwrap();
        a.doc.commit();
        let mut b = PageDoc::from_bytes(&a.snapshot().unwrap(), &[]).unwrap();
        b.set_peer(2).unwrap();
        a.set_title("Cached branch").unwrap();
        a.doc.commit();
        let a_frontiers = a.doc.oplog_frontiers();
        b.apply(&[BlockOp::SetText {
            id: "block".into(),
            text: "Disk branch".into(),
            marks: vec![],
        }])
        .unwrap();
        b.doc.commit();
        let b_frontiers = b.doc.oplog_frontiers();
        let merged = merge("page", &a, Some(&b)).unwrap();
        let old_a = merged.fork_at(&a_frontiers).unwrap();
        let mut old_b = merged.fork_at(&b_frontiers).unwrap();
        assert_eq!(old_a.meta().title, "Cached branch");
        assert_eq!(old_b.blocks()[0].text, "Disk branch");
        assert_eq!(a.blocks()[0].text, "Original");
        assert_eq!(b.meta().title, "Page");
    }

    #[test]
    fn merging_rejects_a_different_page() {
        let a = PageDoc::new_page(1, "a", "A", None, 0).unwrap();
        let b = PageDoc::new_page(2, "b", "B", None, 0).unwrap();
        assert!(merge("a", &a, Some(&b)).is_err());
    }

    #[test]
    fn durable_archive_edits_keep_history_without_creating_an_active_page() {
        let directory =
            std::env::temp_dir().join(format!("vault-conflict-archive-{}", crate::ids::new_id()));
        let vault = crate::Vault::open(directory.to_string_lossy().into_owned()).unwrap();
        let source = PageDoc::new_page(1, "page", "Page", None, 0).unwrap();
        source.doc.commit();
        let original = source.doc.oplog_frontiers();
        vault.inner.store_conflict_archive("page", &source).unwrap();
        assert_eq!(vault.inner.archive_page_ids().unwrap(), ["page"]);
        assert!(matches!(
            vault.load_page("page".into()),
            Err(crate::VaultError::NotFound(_))
        ));
        vault
            .inner
            .edit_conflict_archive("page", |doc| {
                doc.set_conflict("record", "retained record")?;
                Ok(())
            })
            .unwrap();
        // A stale writer must merge the newer durable archive rather than
        // replacing its resolution operations with the original snapshot.
        vault.inner.store_conflict_archive("page", &source).unwrap();
        let saved = vault.inner.read_conflict_archive("page").unwrap();
        assert_eq!(
            saved.conflicts(),
            [("record".into(), "retained record".into())]
        );
        assert_eq!(saved.fork_at(&original).unwrap().meta().title, "Page");
        assert!(vault.page_versions().unwrap().is_empty());
        drop(vault);
        let reopened = crate::Vault::open(directory.to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            reopened
                .inner
                .read_conflict_archive("page")
                .unwrap()
                .conflicts()
                .len(),
            1
        );
        assert!(matches!(
            reopened.load_page("page".into()),
            Err(crate::VaultError::NotFound(_))
        ));
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
