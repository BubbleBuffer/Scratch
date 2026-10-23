//! The notes-owned chat outline: chapters and sections of one agent session.
//!
//! Kept in `<data_dir>/agent/chat_outline.sqlite`, beside BubblePaw's
//! `agent.sqlite` but not inside it. Each chapter is one JSON document holding
//! its sections; anchors are message ids of the session. Saving a session
//! replaces its outline in one transaction and keeps the chapters' order. A
//! session that was saved with no chapters still has an outline (empty), which
//! is what tells it apart from a session that was never saved.
//!
//! Edits (`upsert_chapter`, `upsert_section`, `close_chapter`) check their
//! anchors against the session's message order and run in one transaction, so a
//! refused edit writes nothing. Each session has a revision: 0 until its outline
//! first changes, then one more for every change. An edit that changes nothing
//! keeps the revision.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

/// File name inside the agent's data directory.
pub const FILE_NAME: &str = "chat_outline.sqlite";

/// One session's outline, chapters in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatOutline {
    pub chapters: Vec<OutlineChapter>,
}

/// A span of one session's messages, from `first_message_id` to `last_message_id`
/// (both inclusive, in session order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineChapter {
    pub id: String,
    pub title: String,
    pub summary: String,
    /// The sections' decisions, in order, for the whole chapter.
    pub decisions: Vec<String>,
    /// True while the chapter is still open (the session's current chapter).
    pub ongoing: bool,
    pub first_message_id: String,
    pub last_message_id: String,
    /// RFC 3339 time of the first anchor message.
    pub started_at: String,
    /// RFC 3339 time of the last anchor message; None while the chapter is open.
    pub ended_at: Option<String>,
    pub sections: Vec<OutlineSection>,
}

/// A part of a chapter. Sections are empty for chapters that only came from
/// BubblePaw's context chapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineSection {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub decisions: Vec<String>,
    pub first_message_id: String,
    pub last_message_id: String,
}

/// An outline store error, with the message to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineError(pub String);

impl std::fmt::Display for OutlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OutlineError {}

impl From<rusqlite::Error> for OutlineError {
    fn from(e: rusqlite::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<serde_json::Error> for OutlineError {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, OutlineError>;

// `sessions` is the save marker: one row per session that has had an outline
// saved, and `revision` counts that session's changes. Version 1 stores have no
// `revision` column; `migrate` adds it and keeps every row.
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS chapters(
    session_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>0),
    id TEXT NOT NULL UNIQUE,
    data TEXT NOT NULL,
    PRIMARY KEY(session_id,ordinal)
);
CREATE TABLE IF NOT EXISTS sessions(
    session_id TEXT PRIMARY KEY,
    saved_at TEXT NOT NULL
);";

/// How long a connection waits for another one's write before failing. The agent
/// and a bridge call can both hold a connection to the same file.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The outline database, open for reading and writing.
pub struct OutlineStore {
    conn: Connection,
}

impl OutlineStore {
    /// Opens the database at `path`, creating it and its folder if needed, and
    /// brings an older store up to the current schema.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| OutlineError(format!("create {}: {e}", parent.display())))?;
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// The outline saved for `session_id`: `Some` with its chapters (possibly none,
    /// when an empty outline was saved), or `None` when nothing was ever saved.
    /// Never creates the file: a missing database is simply `None`.
    pub fn read_session(path: &Path, session_id: &str) -> Result<Option<Vec<OutlineChapter>>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = open_read_only(path)?;
        if schema_pending(&conn)? {
            return Ok(None);
        }
        read_chapters(&conn, session_id)
    }

    /// The revision of `session_id` in the database at `path`, read-only. A missing
    /// file, or a session with no change, is revision 0. Never creates the file.
    pub fn read_revision(path: &Path, session_id: &str) -> Result<u64> {
        if !path.exists() {
            return Ok(0);
        }
        let conn = open_read_only(path)?;
        if schema_pending(&conn)? {
            return Ok(0);
        }
        revision_of(&conn, session_id)
    }

    /// As [`OutlineStore::read_session`], on the open database.
    pub fn session(&self, session_id: &str) -> Result<Option<Vec<OutlineChapter>>> {
        read_chapters(&self.conn, session_id)
    }

    /// As [`OutlineStore::read_revision`], on the open database.
    pub fn revision(&self, session_id: &str) -> Result<u64> {
        revision_of(&self.conn, session_id)
    }

    /// Replaces the session's outline with `chapters`, in the order given, and marks
    /// the session as saved (even when `chapters` is empty). Checks that every
    /// chapter and section has an id, a title and both anchors. Anchors are not
    /// checked against the session's messages; use the edits below for that.
    pub fn save_session(&mut self, session_id: &str, chapters: &[OutlineChapter]) -> Result<()> {
        check_session(session_id)?;
        for chapter in chapters {
            check_entry(
                "chapter",
                &chapter.id,
                &chapter.title,
                &chapter.first_message_id,
                &chapter.last_message_id,
            )?;
            for section in &chapter.sections {
                check_entry(
                    "section",
                    &section.id,
                    &section.title,
                    &section.first_message_id,
                    &section.last_message_id,
                )?;
            }
        }
        self.edit(session_id, |current| {
            *current = chapters.to_vec();
            Ok(())
        })?;
        Ok(())
    }

    /// Adds `chapter`, or replaces the chapter with the same `first_message_id`.
    /// The chapter's sections replace the existing sections with the same
    /// `first_message_id`; the chapter's other sections are kept. `order` is the
    /// session's message ids in session order: every anchor must be one of them,
    /// and the first must not come after the last. Returns the new revision.
    pub fn upsert_chapter(
        &mut self,
        session_id: &str,
        order: &[String],
        chapter: OutlineChapter,
    ) -> Result<u64> {
        self.edit(session_id, |chapters| {
            let positions = positions(order);
            check_chapter(&chapter, &positions)?;
            let mut chapter = chapter;
            match chapters
                .iter()
                .position(|c| c.first_message_id == chapter.first_message_id)
            {
                Some(index) => {
                    let kept = std::mem::take(&mut chapters[index].sections);
                    chapter.sections = merge_sections(kept, chapter.sections);
                    check_sections(&chapter, &positions)?;
                    sort_sections(&mut chapter.sections, &positions);
                    chapters[index] = chapter;
                }
                None => {
                    check_sections(&chapter, &positions)?;
                    sort_sections(&mut chapter.sections, &positions);
                    let start = start_of(&chapter, &positions);
                    let at = chapters
                        .iter()
                        .position(|c| start_of(c, &positions) > start)
                        .unwrap_or(chapters.len());
                    chapters.insert(at, chapter);
                }
            }
            Ok(())
        })
    }

    /// Adds `section` to the chapter starting at `chapter_first_message_id`, or
    /// replaces the section with the same `first_message_id`. The section must lie
    /// inside the chapter. The chapter's other sections are kept. Returns the new
    /// revision.
    pub fn upsert_section(
        &mut self,
        session_id: &str,
        order: &[String],
        chapter_first_message_id: &str,
        section: OutlineSection,
    ) -> Result<u64> {
        self.edit(session_id, |chapters| {
            let positions = positions(order);
            let index = find_chapter(chapters, chapter_first_message_id)?;
            let chapter = &mut chapters[index];
            let kept = std::mem::take(&mut chapter.sections);
            chapter.sections = merge_sections(kept, vec![section]);
            check_sections(chapter, &positions)?;
            sort_sections(&mut chapter.sections, &positions);
            Ok(())
        })
    }

    /// Closes the chapter starting at `first_message_id`: its last anchor becomes
    /// `last_message_id`, it gets `ended_at` and is no longer ongoing. Closing a
    /// chapter that is already closed updates it the same way. Its sections must
    /// still lie inside it. Returns the new revision.
    pub fn close_chapter(
        &mut self,
        session_id: &str,
        order: &[String],
        first_message_id: &str,
        last_message_id: &str,
        ended_at: &str,
    ) -> Result<u64> {
        self.edit(session_id, |chapters| {
            let positions = positions(order);
            let index = find_chapter(chapters, first_message_id)?;
            let chapter = &mut chapters[index];
            check_time("ended_at", &chapter.id, ended_at)?;
            chapter.last_message_id = last_message_id.into();
            chapter.ended_at = Some(ended_at.into());
            chapter.ongoing = false;
            check_chapter(chapter, &positions)?;
            check_sections(chapter, &positions)
        })
    }

    /// Applies `change` to the session's chapters in one transaction. Writes and
    /// counts a change only when the chapters differ from what is stored. Any error
    /// rolls the whole edit back. Returns the revision after the edit.
    fn edit(
        &mut self,
        session_id: &str,
        change: impl FnOnce(&mut Vec<OutlineChapter>) -> Result<()>,
    ) -> Result<u64> {
        check_session(session_id)?;
        // Reserve the writer before reading: concurrent live outline updates
        // must wait rather than deadlock while upgrading two shared locks.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let before = read_chapters(&tx, session_id)?;
        let mut chapters = before.clone().unwrap_or_default();
        change(&mut chapters)?;
        check_ids(&tx, session_id, &chapters)?;
        if before.as_deref() != Some(chapters.as_slice()) {
            write_chapters(&tx, session_id, &chapters)?;
            count_change(&tx, session_id)?;
        }
        let revision = revision_of(&tx, session_id)?;
        tx.commit()?;
        Ok(revision)
    }
}

fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

// Creating the SQLite file precedes the first schema transaction. A reader
// can arrive in that tiny window; later versioned schema errors still surface.
fn schema_pending(conn: &Connection) -> Result<bool> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != 0 {
        return Ok(false);
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='chapters')",
        [],
        |r| r.get(0),
    )?;
    Ok(!exists)
}

/// Creates the schema when the file is new and adds `sessions.revision` to a
/// version 1 store. Existing rows are kept.
fn migrate(conn: &mut Connection) -> Result<()> {
    if conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))? == 2 {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA)?;
    if !has_column(&tx, "sessions", "revision")? {
        tx.execute_batch(
            "ALTER TABLE sessions ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK(revision>=0);",
        )?;
    }
    tx.pragma_update(None, "user_version", 2)?;
    tx.commit()?;
    Ok(())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(conn
        .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name=?2")?
        .exists(params![table, column])?)
}

fn check_session(session_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        return Err(OutlineError("session id is empty".into()));
    }
    Ok(())
}

fn check_entry(kind: &str, id: &str, title: &str, first: &str, last: &str) -> Result<()> {
    if id.trim().is_empty() || title.trim().is_empty() {
        return Err(OutlineError(format!("{kind} needs an id and a title")));
    }
    if first.trim().is_empty() || last.trim().is_empty() {
        return Err(OutlineError(format!(
            "{kind} {id} needs both message anchors"
        )));
    }
    Ok(())
}

/// Where each message of the session sits in session order, by id.
type Positions<'a> = HashMap<&'a str, usize>;

fn positions(order: &[String]) -> Positions<'_> {
    order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect()
}

/// The session positions of an entry's two anchors, first not after last.
fn span(
    positions: &Positions,
    kind: &str,
    owner: &str,
    first: &str,
    last: &str,
) -> Result<(usize, usize)> {
    let anchor = |id: &str| {
        positions.get(id).copied().ok_or_else(|| {
            OutlineError(format!(
                "{kind} {owner} anchors {id}, which is not a message of this session"
            ))
        })
    };
    let (start, end) = (anchor(first)?, anchor(last)?);
    if start > end {
        return Err(OutlineError(format!(
            "{kind} {owner} ends before it starts"
        )));
    }
    Ok((start, end))
}

fn start_of(chapter: &OutlineChapter, positions: &Positions) -> usize {
    positions
        .get(chapter.first_message_id.as_str())
        .copied()
        .unwrap_or(usize::MAX)
}

fn check_time(what: &str, owner: &str, value: &str) -> Result<()> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| {
            OutlineError(format!(
                "{what} of {owner} is not an RFC 3339 time: {value}"
            ))
        })
}

/// Checks a chapter's own fields, anchors and times. Its sections are checked by
/// [`check_sections`].
fn check_chapter(chapter: &OutlineChapter, positions: &Positions) -> Result<()> {
    check_entry(
        "chapter",
        &chapter.id,
        &chapter.title,
        &chapter.first_message_id,
        &chapter.last_message_id,
    )?;
    span(
        positions,
        "chapter",
        &chapter.id,
        &chapter.first_message_id,
        &chapter.last_message_id,
    )?;
    check_time("started_at", &chapter.id, &chapter.started_at)?;
    match (chapter.ongoing, chapter.ended_at.as_deref()) {
        (true, None) => {}
        (false, Some(ended)) => check_time("ended_at", &chapter.id, ended)?,
        _ => {
            return Err(OutlineError(format!(
                "chapter {} needs ended_at once closed, and none while ongoing",
                chapter.id
            )))
        }
    }
    Ok(())
}

/// Checks every section of `chapter`: entries, unique ids and starts, anchors in
/// session order, and each section inside the chapter.
fn check_sections(chapter: &OutlineChapter, positions: &Positions) -> Result<()> {
    let (start, end) = span(
        positions,
        "chapter",
        &chapter.id,
        &chapter.first_message_id,
        &chapter.last_message_id,
    )?;
    let mut ids = HashSet::new();
    let mut starts = HashSet::new();
    for section in &chapter.sections {
        check_entry(
            "section",
            &section.id,
            &section.title,
            &section.first_message_id,
            &section.last_message_id,
        )?;
        if !ids.insert(section.id.as_str()) || !starts.insert(section.first_message_id.as_str()) {
            return Err(OutlineError(format!(
                "chapter {} has two sections with the same id or first message ({})",
                chapter.id, section.id
            )));
        }
        let (a, b) = span(
            positions,
            "section",
            &section.id,
            &section.first_message_id,
            &section.last_message_id,
        )?;
        if a < start || b > end {
            return Err(OutlineError(format!(
                "section {} is outside chapter {}",
                section.id, chapter.id
            )));
        }
    }
    Ok(())
}

/// The sections a chapter keeps, with `incoming` replacing those that have the
/// same `first_message_id`. Incoming sections that match nothing are added.
fn merge_sections(kept: Vec<OutlineSection>, incoming: Vec<OutlineSection>) -> Vec<OutlineSection> {
    let mut sections = kept;
    for section in incoming {
        match sections
            .iter()
            .position(|s| s.first_message_id == section.first_message_id)
        {
            Some(index) => sections[index] = section,
            None => sections.push(section),
        }
    }
    sections
}

fn sort_sections(sections: &mut [OutlineSection], positions: &Positions) {
    sections.sort_by_key(|s| {
        positions
            .get(s.first_message_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

fn find_chapter(chapters: &[OutlineChapter], first_message_id: &str) -> Result<usize> {
    chapters
        .iter()
        .position(|c| c.first_message_id == first_message_id)
        .ok_or_else(|| {
            OutlineError(format!(
                "no chapter starts at {first_message_id} in this session's outline"
            ))
        })
}

/// Chapter ids are unique across the store, so a chapter id used by another
/// session is refused here rather than failing on the table's unique key.
fn check_ids(conn: &Connection, session_id: &str, chapters: &[OutlineChapter]) -> Result<()> {
    let mut ids = HashSet::new();
    for chapter in chapters {
        if !ids.insert(chapter.id.as_str()) {
            return Err(OutlineError(format!(
                "chapter id {} is used by two chapters",
                chapter.id
            )));
        }
        let other: Option<String> = conn
            .query_row(
                "SELECT session_id FROM chapters WHERE id=?1 AND session_id<>?2",
                params![chapter.id, session_id],
                |row| row.get(0),
            )
            .optional()?;
        if other.is_some() {
            return Err(OutlineError(format!(
                "chapter id {} is already used by another session",
                chapter.id
            )));
        }
    }
    Ok(())
}

/// Replaces the session's chapter rows with `chapters`, in order.
fn write_chapters(conn: &Connection, session_id: &str, chapters: &[OutlineChapter]) -> Result<()> {
    conn.execute("DELETE FROM chapters WHERE session_id=?1", [session_id])?;
    for (index, chapter) in chapters.iter().enumerate() {
        let ordinal =
            i64::try_from(index + 1).map_err(|_| OutlineError("too many chapters".into()))?;
        conn.execute(
            "INSERT INTO chapters(session_id,ordinal,id,data) VALUES(?1,?2,?3,?4)",
            params![
                session_id,
                ordinal,
                chapter.id,
                serde_json::to_string(chapter)?
            ],
        )?;
    }
    Ok(())
}

/// Marks the session as saved and counts one change: its revision goes up by one,
/// starting from 1 for a session with no row yet.
fn count_change(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions(session_id,saved_at,revision)
         VALUES(?1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),1)
         ON CONFLICT(session_id) DO UPDATE SET saved_at=excluded.saved_at,revision=revision+1",
        [session_id],
    )?;
    Ok(())
}

/// The session's revision on `conn`: 0 when it has no row or the store has no
/// revision column yet.
fn revision_of(conn: &Connection, session_id: &str) -> Result<u64> {
    if !has_column(conn, "sessions", "revision")? {
        return Ok(0);
    }
    let revision: Option<i64> = conn
        .query_row(
            "SELECT revision FROM sessions WHERE session_id=?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;
    u64::try_from(revision.unwrap_or(0))
        .map_err(|_| OutlineError("outline revision is negative".into()))
}

fn read_chapters(conn: &Connection, session_id: &str) -> Result<Option<Vec<OutlineChapter>>> {
    let mut stmt =
        conn.prepare("SELECT data FROM chapters WHERE session_id=?1 ORDER BY ordinal")?;
    let chapters = stmt
        .query_map([session_id], |row| row.get::<_, String>(0))?
        .map(|data| Ok(serde_json::from_str::<OutlineChapter>(&data?)?))
        .collect::<Result<Vec<_>>>()?;
    if !chapters.is_empty() {
        return Ok(Some(chapters));
    }
    Ok(if has_marker(conn, session_id)? {
        Some(Vec::new())
    } else {
        None
    })
}

/// Whether an outline was saved for the session, even an empty one.
fn has_marker(conn: &Connection, session_id: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sessions WHERE session_id=?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_allow_the_file_creation_window_before_schema_commit() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let creating = Connection::open(&path).unwrap();
        assert!(OutlineStore::read_session(&path, "chat").unwrap().is_none());
        assert_eq!(OutlineStore::read_revision(&path, "chat").unwrap(), 0);
        drop(creating);
        let mut store = OutlineStore::open(&path).unwrap();
        store.save_session("chat", &[]).unwrap();
        assert_eq!(
            OutlineStore::read_session(&path, "chat").unwrap(),
            Some(vec![])
        );
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn chapter(id: &str, first: &str, last: &str) -> OutlineChapter {
        OutlineChapter {
            id: id.into(),
            title: format!("Title {id}"),
            summary: "Summary".into(),
            decisions: vec!["Context boundary selected from 2 candidates".into()],
            ongoing: false,
            first_message_id: first.into(),
            last_message_id: last.into(),
            started_at: "2026-10-08T00:00:00Z".into(),
            ended_at: Some("2026-10-08T00:05:00Z".into()),
            sections: vec![OutlineSection {
                id: format!("{id}-s1"),
                title: "Section".into(),
                summary: "Part".into(),
                decisions: vec![],
                first_message_id: first.into(),
                last_message_id: last.into(),
            }],
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("outline_test_{}", vault_core::new_id()))
    }

    /// Session message ids `m1`..`m{count}`, in order.
    fn order(count: usize) -> Vec<String> {
        (1..=count).map(|i| format!("m{i}")).collect()
    }

    #[test]
    fn concurrent_connections_merge_live_updates_without_lock_upgrade_failures() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        let mut store = OutlineStore::open(&path).unwrap();
        store
            .upsert_chapter("chat", &order(8), open_chapter("c", "m1", "m8"))
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|i| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut writer = OutlineStore::open(&path).unwrap();
                    writer
                        .upsert_section(
                            "chat",
                            &order(8),
                            "m1",
                            section(
                                &format!("s{i}"),
                                &format!("m{}", i * 2 + 1),
                                &format!("m{}", i * 2 + 2),
                            ),
                        )
                        .unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let chapters = OutlineStore::read_session(&path, "chat").unwrap().unwrap();
        assert_eq!(chapters[0].sections.len(), 4);
        assert_eq!(OutlineStore::read_revision(&path, "chat").unwrap(), 5);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn section(id: &str, first: &str, last: &str) -> OutlineSection {
        OutlineSection {
            id: id.into(),
            title: format!("Section {id}"),
            summary: "Part".into(),
            decisions: vec![],
            first_message_id: first.into(),
            last_message_id: last.into(),
        }
    }

    /// An open chapter with no sections, so each test adds exactly what it checks.
    fn open_chapter(id: &str, first: &str, last: &str) -> OutlineChapter {
        let mut c = chapter(id, first, last);
        c.ongoing = true;
        c.ended_at = None;
        c.sections.clear();
        c
    }

    #[test]
    fn save_replaces_a_session_and_keeps_order() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        let mut store = OutlineStore::open(&path).unwrap();
        assert!(store.session("s").unwrap().is_none());

        let mut open = chapter("a", "m3", "m4");
        open.ongoing = true;
        open.ended_at = None;
        store
            .save_session("s", &[chapter("b", "m1", "m2"), open])
            .unwrap();
        let read = OutlineStore::read_session(&path, "s").unwrap().unwrap();
        assert_eq!(
            read.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(
            read.iter().map(|c| c.ongoing).collect::<Vec<_>>(),
            [false, true]
        );
        assert_eq!(read[0].sections.len(), 1);
        assert_eq!(
            read[0].decisions,
            ["Context boundary selected from 2 candidates"]
        );
        assert!(OutlineStore::read_session(&path, "other")
            .unwrap()
            .is_none());

        store
            .save_session("s", &[chapter("c", "m5", "m5")])
            .unwrap();
        let read = store.session("s").unwrap().unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, "c");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_empty_outline_is_not_the_same_as_no_outline() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        let mut store = OutlineStore::open(&path).unwrap();
        assert!(store.session("s").unwrap().is_none());

        store.save_session("s", &[]).unwrap();
        assert_eq!(store.session("s").unwrap(), Some(Vec::new()));
        assert_eq!(
            OutlineStore::read_session(&path, "s").unwrap(),
            Some(Vec::new())
        );
        assert!(store.session("unsaved").unwrap().is_none());

        // Saving the empty outline removes the chapters, not the marker.
        store
            .save_session("s", &[chapter("a", "m1", "m1")])
            .unwrap();
        store.save_session("s", &[]).unwrap();
        assert_eq!(store.session("s").unwrap(), Some(Vec::new()));

        assert!(OutlineStore::read_session(&dir.join("missing.sqlite"), "s")
            .unwrap()
            .is_none());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refuses_entries_without_anchors() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let err = store
            .save_session("s", &[chapter("a", "", "m1")])
            .unwrap_err();
        assert!(err.to_string().contains("message anchors"), "{err}");
        assert!(
            store.session("s").unwrap().is_none(),
            "a refused save writes nothing"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revision_is_zero_until_a_change_and_rises_by_one_per_change() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        assert_eq!(OutlineStore::read_revision(&path, "s").unwrap(), 0);
        assert!(!path.exists(), "a revision query never creates the file");

        let mut store = OutlineStore::open(&path).unwrap();
        assert_eq!(store.revision("s").unwrap(), 0);
        let ids = order(4);
        assert_eq!(
            store
                .upsert_chapter("s", &ids, open_chapter("a", "m1", "m2"))
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .upsert_section("s", &ids, "m1", section("a-2", "m2", "m2"))
                .unwrap(),
            2
        );
        store.save_session("s", &[]).unwrap();
        assert_eq!(store.revision("s").unwrap(), 3);
        assert_eq!(OutlineStore::read_revision(&path, "s").unwrap(), 3);
        assert_eq!(store.revision("other").unwrap(), 0);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_op_edits_keep_the_revision() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(4);
        let first = store
            .upsert_chapter("s", &ids, open_chapter("a", "m1", "m2"))
            .unwrap();
        assert_eq!(first, 1);

        // The same chapter again changes nothing.
        assert_eq!(
            store
                .upsert_chapter("s", &ids, open_chapter("a", "m1", "m2"))
                .unwrap(),
            1
        );
        // The same section again changes nothing.
        let s = section("a-2", "m2", "m2");
        assert_eq!(store.upsert_section("s", &ids, "m1", s.clone()).unwrap(), 2);
        assert_eq!(store.upsert_section("s", &ids, "m1", s).unwrap(), 2);
        // Closing with the values already stored changes nothing either.
        let closed = store
            .close_chapter("s", &ids, "m1", "m3", "2026-10-08T00:05:00Z")
            .unwrap();
        assert_eq!(closed, 3);
        assert_eq!(
            store
                .close_chapter("s", &ids, "m1", "m3", "2026-10-08T00:05:00Z")
                .unwrap(),
            3
        );
        // Saving what is already saved changes nothing.
        let saved = store.session("s").unwrap().unwrap();
        store.save_session("s", &saved).unwrap();
        assert_eq!(store.revision("s").unwrap(), 3);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_chapter_keeps_the_sections_it_does_not_name() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(6);
        let mut chapter = open_chapter("a", "m1", "m6");
        chapter.sections = vec![section("s1", "m1", "m2"), section("s2", "m3", "m4")];
        store.upsert_chapter("s", &ids, chapter).unwrap();

        // A later update that names only a new section, and a new title.
        let mut update = open_chapter("a", "m1", "m6");
        update.title = "Renamed".into();
        update.sections = vec![section("s3", "m5", "m6")];
        store.upsert_chapter("s", &ids, update).unwrap();

        let stored = store.session("s").unwrap().unwrap();
        assert_eq!(stored[0].title, "Renamed");
        assert_eq!(
            stored[0]
                .sections
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "s2", "s3"]
        );

        // A section with the same first message replaces its earlier version.
        let mut update = open_chapter("a", "m1", "m6");
        update.sections = vec![section("s2-new", "m3", "m4")];
        store.upsert_chapter("s", &ids, update).unwrap();
        let stored = store.session("s").unwrap().unwrap();
        assert_eq!(
            stored[0]
                .sections
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "s2-new", "s3"]
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_section_adds_replaces_and_keeps_siblings() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(6);
        store
            .upsert_chapter("s", &ids, open_chapter("a", "m1", "m6"))
            .unwrap();
        store
            .upsert_section("s", &ids, "m1", section("late", "m5", "m6"))
            .unwrap();
        store
            .upsert_section("s", &ids, "m1", section("early", "m1", "m2"))
            .unwrap();
        // Same first message as "early": replaced, the other section stays.
        store
            .upsert_section("s", &ids, "m1", section("early-v2", "m1", "m3"))
            .unwrap();

        let stored = store.session("s").unwrap().unwrap();
        let sections = &stored[0].sections;
        assert_eq!(
            sections.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["early-v2", "late"],
            "sections are kept in message order"
        );
        assert_eq!(sections[0].last_message_id, "m3");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_section_refuses_an_unknown_chapter_or_a_section_outside_it() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        let mut store = OutlineStore::open(&path).unwrap();
        let ids = order(6);
        store
            .upsert_chapter("s", &ids, open_chapter("a", "m1", "m3"))
            .unwrap();
        let before = store.session("s").unwrap();

        let err = store
            .upsert_section("s", &ids, "m2", section("x", "m2", "m2"))
            .unwrap_err();
        assert!(err.to_string().contains("no chapter starts at m2"), "{err}");

        let err = store
            .upsert_section("s", &ids, "m1", section("x", "m2", "m5"))
            .unwrap_err();
        assert!(err.to_string().contains("outside chapter"), "{err}");

        assert_eq!(
            store.session("s").unwrap(),
            before,
            "refusals write nothing"
        );
        assert_eq!(store.revision("s").unwrap(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_chapter_sets_the_end_and_keeps_sections_inside() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(6);
        store
            .upsert_chapter("s", &ids, open_chapter("a", "m1", "m2"))
            .unwrap();
        store
            .upsert_section("s", &ids, "m1", section("s1", "m1", "m2"))
            .unwrap();
        let revision = store
            .close_chapter("s", &ids, "m1", "m4", "2026-10-08T00:09:00Z")
            .unwrap();
        assert_eq!(revision, 3);

        let stored = store.session("s").unwrap().unwrap();
        assert!(!stored[0].ongoing);
        assert_eq!(stored[0].last_message_id, "m4");
        assert_eq!(stored[0].ended_at.as_deref(), Some("2026-10-08T00:09:00Z"));
        assert_eq!(stored[0].sections.len(), 1, "sections are kept");

        // A section that ends after the new last anchor blocks the close.
        store
            .upsert_section("s", &ids, "m1", section("s2", "m3", "m6"))
            .unwrap_err();
        store
            .upsert_section("s", &ids, "m1", section("s2", "m3", "m4"))
            .unwrap();
        let err = store
            .close_chapter("s", &ids, "m1", "m3", "2026-10-08T00:10:00Z")
            .unwrap_err();
        assert!(err.to_string().contains("outside chapter"), "{err}");
        assert_eq!(
            store.revision("s").unwrap(),
            4,
            "the refused close changed nothing"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_chapter_refuses_an_unknown_chapter_and_bad_times() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(6);
        store
            .upsert_chapter("s", &ids, open_chapter("a", "m1", "m2"))
            .unwrap();

        let err = store
            .close_chapter("s", &ids, "m9", "m9", "2026-10-08T00:09:00Z")
            .unwrap_err();
        assert!(err.to_string().contains("no chapter starts at m9"), "{err}");

        let err = store
            .close_chapter("s", &ids, "m1", "m2", "yesterday")
            .unwrap_err();
        assert!(err.to_string().contains("RFC 3339"), "{err}");
        assert!(store.session("s").unwrap().unwrap()[0].ongoing);
        assert_eq!(store.revision("s").unwrap(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_refuse_anchors_that_are_not_in_the_session_in_order() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(3);
        store
            .upsert_chapter("s", &ids, open_chapter("a", "m1", "m2"))
            .unwrap();
        let before = store.session("s").unwrap();

        // An anchor from another session (or a deleted message) is not in this order.
        let mut foreign = open_chapter("b", "m1", "other-session-msg");
        foreign.id = "b".into();
        let err = store.upsert_chapter("s", &ids, foreign).unwrap_err();
        assert!(
            err.to_string().contains("not a message of this session"),
            "{err}"
        );

        // A chapter whose last message comes before its first.
        let err = store
            .upsert_chapter("s", &ids, open_chapter("c", "m3", "m1"))
            .unwrap_err();
        assert!(err.to_string().contains("ends before it starts"), "{err}");

        // A section anchored outside the session.
        let err = store
            .upsert_section("s", &ids, "m1", section("x", "m1", "m9"))
            .unwrap_err();
        assert!(
            err.to_string().contains("not a message of this session"),
            "{err}"
        );

        assert_eq!(store.session("s").unwrap(), before);
        assert_eq!(store.revision("s").unwrap(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_refuse_a_chapter_id_used_by_another_session_and_bad_open_flags() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        let ids = order(3);
        store
            .upsert_chapter("s1", &ids, open_chapter("shared", "m1", "m2"))
            .unwrap();
        let err = store
            .upsert_chapter("s2", &ids, open_chapter("shared", "m1", "m2"))
            .unwrap_err();
        assert!(err.to_string().contains("another session"), "{err}");
        assert!(store.session("s2").unwrap().is_none());

        // Ongoing chapters have no end; closed chapters need one.
        let mut bad = open_chapter("c", "m1", "m2");
        bad.ended_at = Some("2026-10-08T00:05:00Z".into());
        let err = store.upsert_chapter("s1", &ids, bad).unwrap_err();
        assert!(err.to_string().contains("ended_at"), "{err}");

        let empty = store.upsert_chapter("", &ids, open_chapter("c", "m1", "m2"));
        assert!(empty
            .unwrap_err()
            .to_string()
            .contains("session id is empty"));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_version_one_store_migrates_in_place_and_keeps_its_rows() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        // The store as version 1 wrote it: no revision column.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE chapters(
                    session_id TEXT NOT NULL,
                    ordinal INTEGER NOT NULL CHECK(ordinal>0),
                    id TEXT NOT NULL UNIQUE,
                    data TEXT NOT NULL,
                    PRIMARY KEY(session_id,ordinal)
                );
                CREATE TABLE sessions(session_id TEXT PRIMARY KEY, saved_at TEXT NOT NULL);
                PRAGMA user_version=1;",
            )
            .unwrap();
            let legacy = chapter("legacy", "m1", "m2");
            conn.execute(
                "INSERT INTO chapters(session_id,ordinal,id,data) VALUES('s',1,'legacy',?1)",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions(session_id,saved_at) VALUES('s','2026-10-08T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions(session_id,saved_at) VALUES('empty','2026-10-08T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Reading the legacy file before any open changes nothing and reports 0.
        assert_eq!(OutlineStore::read_revision(&path, "s").unwrap(), 0);
        assert_eq!(
            OutlineStore::read_session(&path, "s").unwrap().unwrap(),
            vec![chapter("legacy", "m1", "m2")]
        );

        let mut store = OutlineStore::open(&path).unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(
            store.session("s").unwrap().unwrap(),
            vec![chapter("legacy", "m1", "m2")],
            "the legacy chapter survives the migration"
        );
        assert_eq!(store.session("empty").unwrap(), Some(Vec::new()));
        assert_eq!(store.revision("s").unwrap(), 0);

        let ids = order(3);
        assert_eq!(
            store
                .upsert_section("s", &ids, "m1", section("legacy-s", "m1", "m1"))
                .unwrap(),
            1
        );
        drop(store);
        // Reopening keeps the revision and does not migrate twice.
        let store = OutlineStore::open(&path).unwrap();
        assert_eq!(store.revision("s").unwrap(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_keeps_order_and_marker_on_a_new_file() {
        let dir = temp_dir();
        let mut store = OutlineStore::open(&dir.join(FILE_NAME)).unwrap();
        store
            .save_session("s", &[chapter("a", "m1", "m1"), chapter("b", "m2", "m2")])
            .unwrap();
        let read = store.session("s").unwrap().unwrap();
        assert_eq!(
            read.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
