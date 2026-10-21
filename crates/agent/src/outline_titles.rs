//! Notes-owned title policy. Explicit renames are durable; outline titles are
//! automatic display names and never replace a name chosen by the user.

use std::path::Path;

use anyhow::Result;
use bubblepaw_core::Store;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use vault_tools::outline::OutlineStore;

pub(crate) fn user_named_ids(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_title_policy')",
        [], |row| row.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt =
        conn.prepare("SELECT session_id FROM session_title_policy WHERE user_named=1")?;
    let ids = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

fn policy_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Let the outline store own its schema and migration first.
    drop(OutlineStore::open(path)?);
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_title_policy(
            session_id TEXT PRIMARY KEY,
            user_named INTEGER NOT NULL CHECK(user_named IN (0,1))
        );",
    )?;
    Ok(conn)
}

/// Marks intent before renaming. An automatic update holds the same outline
/// write lock through its rename, so either ordering leaves the user name last.
pub(crate) fn rename_user(
    store: &Store,
    path: &Path,
    session: &str,
    title: &str,
) -> Result<bubblepaw_core::Session> {
    store.session(session)?;
    let mut conn = policy_connection(path)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO session_title_policy(session_id,user_named) VALUES(?1,1)
         ON CONFLICT(session_id) DO UPDATE SET user_named=1",
        [session],
    )?;
    tx.commit()?;
    // Persisting the flag first also preserves explicit rename intent if a
    // crash happens between the two databases' commits.
    store.rename_session(session, title)
}

pub(crate) fn refresh(store: &Store, path: &Path, session: &str) -> Result<()> {
    let current = store.session(session)?;
    if path.exists() {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_title_policy')",
            [], |row| row.get(0),
        )?;
        if exists
            && conn
                .query_row(
                    "SELECT user_named FROM session_title_policy WHERE session_id=?1",
                    [session],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false)
        {
            return Ok(());
        }
    }
    let title = match OutlineStore::read_session(path, session)? {
        Some(chapters) => chapters.last().map(|c| c.title.clone()),
        None => store
            .context_chapters(session)?
            .last()
            .map(|c| c.title.clone()),
    };
    if title
        .as_ref()
        .is_none_or(|t| t.trim().is_empty() || t == &current.title)
    {
        return Ok(());
    }
    let mut conn = policy_connection(path)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let sticky = tx
        .query_row(
            "SELECT user_named FROM session_title_policy WHERE session_id=?1",
            [session],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    if !sticky {
        let title = match OutlineStore::read_session(path, session)? {
            Some(chapters) => chapters.last().map(|c| c.title.clone()),
            None => store
                .context_chapters(session)?
                .last()
                .map(|c| c.title.clone()),
        };
        if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
            if store.session(session)?.title != title {
                store.rename_session(session, title)?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_tools::outline::OutlineChapter;

    #[test]
    fn display_policy_preserves_raw_titles_and_explicit_colon_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat_outline.sqlite");
        assert!(user_named_ids(&path).unwrap().is_empty());
        assert!(!path.exists());
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let session = store.create_session("New chat").unwrap();
        OutlineStore::open(&path)
            .unwrap()
            .save_session(
                &session.id,
                &[chapter("Mouse Keys: Restore the cursor", "m1")],
            )
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(
            store.session(&session.id).unwrap().title,
            "Mouse Keys: Restore the cursor"
        );
        assert!(user_named_ids(&path).unwrap().is_empty());
        rename_user(
            &store,
            &path,
            &session.id,
            "My chosen title: retain all words",
        )
        .unwrap();
        assert_eq!(user_named_ids(&path).unwrap(), [session.id.clone()]);
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(
            store.session(&session.id).unwrap().title,
            "My chosen title: retain all words"
        );
    }

    fn chapter(title: &str, anchor: &str) -> OutlineChapter {
        OutlineChapter {
            id: format!("chapter-{anchor}"),
            title: title.into(),
            summary: String::new(),
            decisions: Vec::new(),
            ongoing: true,
            first_message_id: anchor.into(),
            last_message_id: anchor.into(),
            started_at: "2026-10-10T10:00:00Z".into(),
            ended_at: None,
            sections: Vec::new(),
        }
    }

    #[test]
    fn auto_title_follows_the_latest_chapter_and_its_retitles() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let session = store.create_session("New chat").unwrap();
        let path = dir.path().join("chat_outline.sqlite");
        let mut outline = OutlineStore::open(&path).unwrap();
        outline
            .save_session(&session.id, &[chapter("First", "m1")])
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(store.session(&session.id).unwrap().title, "First");
        outline
            .save_session(
                &session.id,
                &[chapter("First", "m1"), chapter("Current", "m2")],
            )
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(store.session(&session.id).unwrap().title, "Current");
        outline
            .save_session(
                &session.id,
                &[
                    chapter("Retitled old", "m1"),
                    chapter("Retitled current", "m2"),
                ],
            )
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(
            store.session(&session.id).unwrap().title,
            "Retitled current"
        );
    }

    #[test]
    fn explicit_rename_is_sticky_across_a_reopen_and_new_chapters() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("agent.sqlite");
        let path = dir.path().join("chat_outline.sqlite");
        let store = Store::open(&db).unwrap();
        let session = store.create_session("New chat").unwrap();
        let mut outline = OutlineStore::open(&path).unwrap();
        outline
            .save_session(&session.id, &[chapter("Automatic", "m1")])
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        rename_user(&store, &path, &session.id, "My chosen name").unwrap();
        drop(store);
        let store = Store::open(&db).unwrap();
        outline
            .save_session(&session.id, &[chapter("Later", "m2")])
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(store.session(&session.id).unwrap().title, "My chosen name");
    }

    #[test]
    fn choosing_the_current_name_is_still_an_explicit_rename() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let session = store.create_session("Same name").unwrap();
        let path = dir.path().join("chat_outline.sqlite");
        rename_user(&store, &path, &session.id, "Same name").unwrap();
        let mut outline = OutlineStore::open(&path).unwrap();
        outline
            .save_session(&session.id, &[chapter("Later", "m1")])
            .unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(store.session(&session.id).unwrap().title, "Same name");
    }

    #[test]
    fn empty_outline_preserves_the_current_title() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let session = store.create_session("Original").unwrap();
        let path = dir.path().join("chat_outline.sqlite");
        let mut outline = OutlineStore::open(&path).unwrap();
        outline.save_session(&session.id, &[]).unwrap();
        refresh(&store, &path, &session.id).unwrap();
        assert_eq!(store.session(&session.id).unwrap().title, "Original");
    }
}
