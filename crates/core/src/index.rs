//! The SQLite index: page metadata, properties (one typed row per page and
//! property), links, attachment references and an FTS5 table over title and
//! body. Vault rows can be rebuilt from Loro documents; external search
//! sources retain their own rows and reconcile them from their stores.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use crate::search_exec::Candidates;
use crate::search_query::SearchSpec;
use rusqlite::types::Value;
use std::time::Instant;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::fuzzy::{self, Scratch, EDIT_PENALTY};
use crate::types::{
    Attachment, ExternalSearchRow, PageId, PageSummary, PropDef, PropValue, Property, SearchHit,
    SearchIndexStats, SearchResults, SearchSpecResults, TrashedPage,
};

/// An index of another version is rebuilt from the vault documents. Version
/// 14 excludes view settings JSON from searchable text. Rebuilding preserves
/// the existing document representation.
const SCHEMA_VERSION: i64 = 14;

/// Broad queries exceed the larger of 200 documents or 5% of live pages.
/// No letter-count rule: selectivity comes from this vault's indexed words.
const SEARCH_BREADTH: (u32, u32) = (200, 20);

/// Separator of ids in the `t` column of multi-valued properties.
pub(crate) const SEP: char = '\u{1f}';

pub(crate) struct Index {
    pub conn: Connection,
    /// Search's live-page count, guarded against local and writer commits.
    search_count_cache: std::cell::Cell<Option<(i64, u64, u32, u32)>>,
    /// Set while a page-only search runs: external rows are excluded before
    /// ranking, the limit and corrections (`search_spec_pages`).
    pub(crate) page_scope: std::cell::Cell<bool>,
    /// Titles and the word index for typo correction, rebuilt lazily.
    fuzzy: std::cell::RefCell<fuzzy::Cache>,
    titles_guard: std::cell::Cell<Option<(i64, u64)>>,
    title_positions: std::cell::RefCell<HashMap<String, usize>>,
    pub(crate) search_filter_cache: std::cell::RefCell<crate::search_exec::Cache>,
    /// Bulk writes in progress: computed properties wait (`computed.rs`).
    pub defer: std::cell::Cell<bool>,
    /// Rows whose computed values were refreshed because a related page
    /// changed: the vault reports them as changed (`take_touched`).
    pub touched: std::cell::RefCell<Vec<String>>,
}

/// Page fields that change through metadata edits (not block edits). Written
/// on the caller's thread so queries see them at once.
#[derive(Debug, Clone, Default)]
pub(crate) struct MetaRow {
    pub id: String,
    pub title: String,
    pub parent: Option<String>,
    pub created: i64,
    pub updated: i64,
    pub sort_key: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub role: Option<String>,
    pub agenda_role: Option<String>,
    pub agenda_date: Option<String>,
    pub cover: Option<String>,
    pub favorite: Option<i64>,
    pub trashed: Option<i64>,
    /// The page type (`document`, `database`, or a kept unknown type).
    pub page_type: String,
    /// The schema as JSON (`Vec<PropDef>`), for databases.
    pub schema: Option<String>,
    pub template: Option<String>,
    pub is_template: bool,
    pub props: Vec<Property>,
    /// Task-note status projected only into SQLite, never document properties
    /// or the property list used to render Markdown.
    pub task_status: Option<String>,
}

/// Fields derived from the blocks and the files, written by the writer.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContentRow {
    pub body: String,
    pub md_path: Option<String>,
    pub md_hash: Option<String>,
    /// The document version the markdown file was rendered from (the base
    /// for merging outside edits in `rescan`).
    pub md_version: Option<Vec<u8>>,
    pub links: Vec<String>,
    pub refs: Vec<String>,
    /// The document's encoded oplog version vector, for sync (unchanged
    /// when `None`).
    pub vv: Option<Vec<u8>>,
    pub todos: crate::agenda::BlockAgenda,
}

/// A page's (id, markdown file name, hash of the file as written).
pub(crate) type MdFile = (String, Option<String>, Option<String>);
/// A page's markdown file with its hash and document version.
pub(crate) type MdRow = (String, String, Option<String>, Option<Vec<u8>>);

/// Columns of one property row: (t, tf, n, d0, d1).
pub(crate) type PropCols = (
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<i64>,
    Option<i64>,
);

pub(crate) fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// `\x1fa\x1fb\x1f` for a set of ids (sorted, deduplicated).
pub(crate) fn join_ids(ids: &[String]) -> String {
    let mut v: Vec<&String> = ids.iter().collect();
    v.sort();
    v.dedup();
    let mut out = String::from(SEP);
    for id in v {
        out.push_str(id);
        out.push(SEP);
    }
    out
}

pub(crate) fn prop_cols(v: &PropValue) -> PropCols {
    match v {
        PropValue::Empty => (None, None, None, None, None),
        PropValue::Text(s) | PropValue::Url(s) => {
            (Some(s.clone()), Some(fold(s)), None, None, None)
        }
        PropValue::Number(x) => (None, None, Some(*x), None, None),
        PropValue::Checkbox(b) => (None, None, Some(if *b { 1.0 } else { 0.0 }), None, None),
        PropValue::Date {
            start_ms, end_ms, ..
        } => (None, None, None, Some(*start_ms), *end_ms),
        PropValue::Select(id) | PropValue::Status(id) => (Some(id.clone()), None, None, None, None),
        PropValue::MultiSelect(ids) | PropValue::Relation(ids) | PropValue::Files(ids) => {
            (Some(join_ids(ids)), None, None, None, None)
        }
        PropValue::Error(_) => (None, None, None, None, None),
    }
}

const SUMMARY_COLS: &str = "p.rowid, p.id, p.title, p.parent, p.updated_ms, p.created_ms, p.icon,
    p.page_type, p.favorite IS NOT NULL,
    (SELECT count(*) FROM pages c WHERE c.parent = p.id AND c.in_trash = 0
       AND c.is_template = 0 AND c.role IS NOT 'calendars' AND c.role IS NOT 'nodes'
       AND c.role IS NOT 'feedback'
       AND c.id IS NOT p.template)";

/// Visible pages `p`: excludes the system collection roles (the calendar page
/// and the Nodes/Feedback collections) and templates, including their parent's
/// default template (marked or not). Node records are ordinary pages here.
/// Search's projection has no role column, so consult existing page metadata.
pub(crate) const NOT_TEMPLATE: &str = "p.is_template = 0 AND NOT EXISTS(SELECT 1 FROM pages hidden
     WHERE hidden.id = p.id AND hidden.role IN ('calendars', 'nodes', 'feedback')) AND NOT EXISTS(SELECT 1 FROM pages tc
     WHERE tc.id = p.parent AND tc.template = p.id)";

/// Global discovery excludes the whole Feedback subtree. UNION also terminates
/// safely for malformed parent cycles; the existing parent index serves recursion.
/// Explicit child listings omit this predicate so reports remain addressable.
pub(crate) const NOT_FEEDBACK: &str = "p.id NOT IN (
    WITH RECURSIVE feedback(id) AS (
        SELECT id FROM pages WHERE role = 'feedback'
        UNION
        SELECT child.id FROM pages child JOIN feedback ON child.parent = feedback.id
    ) SELECT id FROM feedback)";

impl Index {
    /// Opens (creating if needed) the index. Returns whether the schema had to
    /// be created, in which case the caller should reindex.
    pub fn open(path: &Path) -> Result<(Index, bool)> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        // A 16 MB page cache (the default is 2 MB) keeps the pages and props
        // tables of a 10,000-row collection in memory: queries that read
        // every row of a collection are 3x faster with it.
        conn.pragma_update(None, "cache_size", -16_000)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let fresh = version != SCHEMA_VERSION;
        if fresh {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS search_rows_insert;
                 DROP TRIGGER IF EXISTS search_rows_update;
                 DROP TRIGGER IF EXISTS search_rows_delete;
                 DROP TRIGGER IF EXISTS search_rows_external_insert;
                 DROP TRIGGER IF EXISTS search_rows_external_update;
                 DROP TRIGGER IF EXISTS search_rows_external_delete;
                 DROP TABLE IF EXISTS search_rows;
                 DROP TABLE IF EXISTS search_multi;
                 DROP TABLE IF EXISTS pages;
                 DROP TABLE IF EXISTS fts;
                 DROP TABLE IF EXISTS props;
                 DROP TABLE IF EXISTS links;
                 DROP TABLE IF EXISTS refs;
                 DROP TABLE IF EXISTS attachments;
                 DROP TABLE IF EXISTS conflicts;
                 DROP TABLE IF EXISTS agenda;
                 DROP TABLE IF EXISTS meta;
                 CREATE TABLE pages(
                   rowid INTEGER PRIMARY KEY,
                   id TEXT NOT NULL UNIQUE,
                   title TEXT NOT NULL,
                   title_fold TEXT NOT NULL DEFAULT '',
                   parent TEXT,
                   created_ms INTEGER NOT NULL,
                   updated_ms INTEGER NOT NULL,
                   md_path TEXT,
                   md_hash TEXT,
                   md_version BLOB,
                   sort_key TEXT NOT NULL DEFAULT '',
                   icon TEXT,
                   color TEXT,
                   role TEXT,
                   agenda_role TEXT,
                   agenda_date TEXT,
                   todos_total INTEGER NOT NULL DEFAULT 0,
                   todos_done INTEGER NOT NULL DEFAULT 0,
                   task_done INTEGER,
                   cover TEXT,
                   favorite INTEGER,
                   trashed INTEGER,
                   in_trash INTEGER NOT NULL DEFAULT 0,
                   page_type TEXT NOT NULL DEFAULT 'document',
                   schema TEXT,
                   template TEXT,
                   is_template INTEGER NOT NULL DEFAULT 0,
                   vv BLOB
                 );
                 CREATE INDEX pages_updated ON pages(in_trash, updated_ms DESC, id DESC);
                 CREATE INDEX pages_parent ON pages(parent, in_trash, updated_ms DESC, id DESC);
                 CREATE INDEX pages_children ON pages(parent, in_trash, sort_key, created_ms, id, title_fold, updated_ms);
                 CREATE INDEX pages_favorite ON pages(favorite) WHERE favorite IS NOT NULL;
                 CREATE INDEX pages_trashed ON pages(trashed) WHERE trashed IS NOT NULL;
                 CREATE INDEX pages_title ON pages(title_fold);
                 CREATE INDEX pages_role ON pages(role) WHERE role IS NOT NULL;
                 CREATE TABLE agenda(page INTEGER NOT NULL, block TEXT NOT NULL DEFAULT '',
                   kind INTEGER NOT NULL, d0 INTEGER, d1 INTEGER,
                   has_time INTEGER NOT NULL, done INTEGER NOT NULL, title TEXT NOT NULL,
                   source INTEGER NOT NULL DEFAULT 0, task_status TEXT,
                   calendar TEXT, assignee TEXT,
                   PRIMARY KEY(page, block, source)) WITHOUT ROWID;
                 CREATE INDEX agenda_date ON agenda(d0);
                 CREATE INDEX agenda_done_date ON agenda(done, d0);
                 CREATE TABLE props(
                   page INTEGER NOT NULL,
                   key TEXT NOT NULL,
                   t TEXT, tf TEXT, n REAL, d0 INTEGER, d1 INTEGER,
                   json TEXT NOT NULL,
                   PRIMARY KEY(page, key)
                 ) WITHOUT ROWID;
                 CREATE INDEX props_t ON props(key, t);
                 CREATE INDEX props_n ON props(key, n);
                 CREATE INDEX props_d ON props(key, d0);
                 CREATE TABLE links(src TEXT NOT NULL, dst TEXT NOT NULL, PRIMARY KEY(src, dst)) WITHOUT ROWID;
                 CREATE INDEX links_dst ON links(dst, src);
                 CREATE TABLE refs(page TEXT NOT NULL, blob TEXT NOT NULL, PRIMARY KEY(page, blob)) WITHOUT ROWID;
                 CREATE INDEX refs_blob ON refs(blob);
                 CREATE TABLE conflicts(
                   page TEXT NOT NULL, id TEXT NOT NULL, anchor TEXT NOT NULL,
                   json TEXT NOT NULL, scope TEXT NOT NULL DEFAULT 'block',
                   PRIMARY KEY(page, id)
                 ) WITHOUT ROWID;
                 CREATE TABLE attachments(
                   id TEXT PRIMARY KEY, name TEXT NOT NULL, mime TEXT NOT NULL,
                   size INTEGER NOT NULL, created_ms INTEGER NOT NULL
                 ) WITHOUT ROWID;
                 CREATE VIRTUAL TABLE fts USING fts5(
                   title, body,
                   tokenize = 'unicode61 remove_diacritics 2',
                   prefix = '1 2 3'
                 );",
            )?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        // External rows are never dropped by a rebuild. Their text shares the
        // FTS table under negative rowids; a rebuilt FTS table gets it back.
        conn.execute_batch(
            // Local bookkeeping is not part of page documents or sync. Like
            // other additive tables, old indexes gain it without a rebuild.
            "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE IF NOT EXISTS search_external(
               rowid INTEGER PRIMARY KEY,
               source TEXT NOT NULL, id TEXT NOT NULL,
               title TEXT NOT NULL, body TEXT NOT NULL,
               page_type TEXT NOT NULL, target TEXT,
               updated_ms INTEGER NOT NULL,
               UNIQUE(source, id)
             );",
        )?;
        if fresh {
            conn.execute(
                "INSERT INTO fts(rowid, title, body) SELECT -rowid, title, body FROM search_external",
                [],
            )?;
        }
        // Additive, so an index of this version is not rebuilt: the conflict
        // mirror gains its `scope` column in place. Existing rows were all
        // format 1 (one block), which the default describes.
        let scope_columns: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('conflicts') WHERE name = 'scope'",
            [],
            |r| r.get(0),
        )?;
        if scope_columns == 0 {
            conn.execute_batch(
                "ALTER TABLE conflicts ADD COLUMN scope TEXT NOT NULL DEFAULT 'block';",
            )?;
        }
        // Rows were keyed by block. A format 1 record's id is its block, so
        // each row keeps its key as both id and anchor. Rebuilt in one
        // transaction: a failure drops the connection and rolls it back.
        let id_columns: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('conflicts') WHERE name = 'id'",
            [],
            |r| r.get(0),
        )?;
        if id_columns == 0 {
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE conflicts_v2(
                   page TEXT NOT NULL, id TEXT NOT NULL, anchor TEXT NOT NULL,
                   json TEXT NOT NULL, scope TEXT NOT NULL DEFAULT 'block',
                   PRIMARY KEY(page, id)
                 ) WITHOUT ROWID;
                 INSERT INTO conflicts_v2(page, id, anchor, json, scope)
                   SELECT page, block, block, json, scope FROM conflicts;
                 DROP TABLE conflicts;
                 ALTER TABLE conflicts_v2 RENAME TO conflicts;
                 COMMIT;",
            )?;
        }
        // Connection-local tokenizer tables over the existing index. Both
        // tokenizers use unicode61's same diacritic setting; the token view
        // normalizes estimator prefixes. Search reads `search_rows`, the
        // maintained projection of vault pages and external rows.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE temp.search_terms USING fts5vocab(main, fts, instance);
             CREATE VIRTUAL TABLE temp.search_vocab USING fts5vocab(main, fts, row);
             CREATE VIRTUAL TABLE temp.search_tokenizer USING fts3tokenize(unicode61, 'remove_diacritics=2');",
        )?;
        crate::fts_rank::register(&conn)?;
        crate::search_exec::initialize(&conn)?;
        Ok((
            Index {
                conn,
                search_count_cache: std::cell::Cell::new(None),
                page_scope: Default::default(),
                fuzzy: Default::default(),
                titles_guard: Default::default(),
                title_positions: Default::default(),
                search_filter_cache: Default::default(),
                defer: std::cell::Cell::new(false),
                touched: Default::default(),
            },
            fresh,
        ))
    }

    pub(crate) fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()?
            .flatten())
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [key, value])?;
        Ok(())
    }

    /// Live pages, excluding system collections and the Feedback subtree.
    /// Template counting retains its existing behavior.
    pub fn page_count(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT count(*) FROM pages p WHERE in_trash = 0 AND role IS NOT 'calendars'
                 AND role IS NOT 'nodes' AND {NOT_FEEDBACK}"),
                [],
                |r| r.get::<_, i64>(0),
            )? as u32)
    }

    /// Whether the page exists (in the trash or not).
    pub fn exists(&self, id: &str) -> Result<bool> {
        Ok(self.rowid(id)?.is_some())
    }

    pub fn rowid(&self, id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached("SELECT rowid FROM pages WHERE id = ?1")?
            .query_row([id], |r| r.get(0))
            .optional()?)
    }

    pub fn in_trash(&self, id: &str) -> Result<Option<bool>> {
        Ok(self
            .conn
            .prepare_cached("SELECT in_trash FROM pages WHERE id = ?1")?
            .query_row([id], |r| r.get::<_, i64>(0))
            .optional()?
            .map(|v| v != 0))
    }

    pub fn parent_of(&self, id: &str) -> Result<Option<Option<String>>> {
        Ok(self
            .conn
            .prepare_cached("SELECT parent FROM pages WHERE id = ?1")?
            .query_row([id], |r| r.get(0))
            .optional()?)
    }

    pub fn md(&self, id: &str) -> Result<(Option<String>, Option<String>)> {
        Ok(self
            .conn
            .prepare_cached("SELECT md_path, md_hash FROM pages WHERE id = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .unwrap_or((None, None)))
    }

    pub fn title_of(&self, id: &str) -> Result<Option<String>> {
        let page: Option<String> = self
            .conn
            .prepare_cached("SELECT title FROM pages WHERE id = ?1")?
            .query_row([id], |r| r.get(0))
            .optional()?;
        if page.is_some() { return Ok(page); }
        Ok(self.find_indexed_note(id)?.map(|hit| hit.title))
    }

    /// Exact external metadata lookup. Ambiguous source-local ids stay
    /// unresolved; opaque navigation targets are returned intact.
    pub fn find_indexed_note(&self, id: &str) -> Result<Option<SearchHit>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source, id, title, body, page_type, target
             FROM search_external WHERE id = ?1 ORDER BY source LIMIT 2",
        )?;
        let mut rows = stmt.query([id])?;
        let Some(row) = rows.next()? else { return Ok(None) };
        let hit = SearchHit {
            source: row.get(0)?, id: row.get(1)?, title: row.get(2)?,
            snippet: row.get(3)?, page_type: row.get(4)?, target: row.get(5)?,
            fuzzy: false, corrected: None, matched_filters: Vec::new(),
        };
        if rows.next()?.is_some() { return Ok(None); }
        Ok(Some(hit))
    }

    /// A page with this title (case-insensitive): live pages first, then
    /// the most recently updated.
    pub fn id_by_title(&self, title: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id FROM pages WHERE title_fold = ?1
                 ORDER BY in_trash, updated_ms DESC LIMIT 1",
            )?
            .query_row([fold(title.trim())], |r| r.get(0))
            .optional()?)
    }

    /// The collection schema of a page (`None` if it is no collection).
    pub fn schema_of(&self, id: &str) -> Result<Option<Vec<PropDef>>> {
        let json: Option<Option<String>> = self
            .conn
            .prepare_cached("SELECT schema FROM pages WHERE id = ?1 AND page_type = 'database'")?
            .query_row([id], |r| r.get(0))
            .optional()?;
        Ok(json
            .flatten()
            .map(|j| serde_json::from_str(&j).unwrap_or_default()))
    }

    /// (parent, marked as a template) of a page.
    pub fn parent_and_template(&self, id: &str) -> Result<Option<(Option<String>, bool)>> {
        Ok(self
            .conn
            .prepare_cached("SELECT parent, is_template FROM pages WHERE id = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))
            .optional()?)
    }

    pub fn template_of(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached("SELECT template FROM pages WHERE id = ?1")?
            .query_row([id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    /// Summaries for the given rowids, in that order.
    pub fn summaries(&self, rowids: &[i64]) -> Result<Vec<PageSummary>> {
        if rowids.is_empty() {
            return Ok(Vec::new());
        }
        let list = rowids
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut by_row: HashMap<i64, PageSummary> = HashMap::with_capacity(rowids.len());
        {
            let mut st = self.conn.prepare(&format!(
                "SELECT {SUMMARY_COLS} FROM pages p WHERE p.rowid IN ({list})"
            ))?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                by_row.insert(
                    r.get(0)?,
                    PageSummary {
                        id: r.get(1)?,
                        title: r.get(2)?,
                        parent: r.get(3)?,
                        updated_ms: r.get(4)?,
                        created_ms: r.get(5)?,
                        icon: r.get(6)?,
                        page_type: r.get(7)?,
                        favorite: r.get::<_, i64>(8)? != 0,
                        child_count: r.get::<_, i64>(9)? as u32,
                        properties: Vec::new(),
                    },
                );
            }
        }
        {
            let mut st = self.conn.prepare(&format!(
                "SELECT page, key, json FROM props WHERE page IN ({list}) ORDER BY page, key"
            ))?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let page: i64 = r.get(0)?;
                let json: String = r.get(2)?;
                if let (Some(s), Ok(value)) = (by_row.get_mut(&page), serde_json::from_str(&json)) {
                    s.properties.push(Property {
                        key: r.get(1)?,
                        value,
                    });
                }
            }
        }
        Ok(rowids.iter().filter_map(|r| by_row.remove(r)).collect())
    }

    fn rowids(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare_cached(sql)?;
        let v = st
            .query_map(p, |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }

    pub fn list(&self, parent: Option<&str>, offset: u32, limit: u32) -> Result<Vec<PageSummary>> {
        let rows = match parent {
            None => self.rowids(
                &format!(
                    "SELECT rowid FROM pages p WHERE in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}
                     ORDER BY updated_ms DESC, id DESC LIMIT ?1 OFFSET ?2"
                ),
                params![limit, offset],
            )?,
            Some(p) => self.rowids(
                &format!(
                    "SELECT rowid FROM pages p WHERE parent = ?3 AND in_trash = 0 AND {NOT_TEMPLATE}
                     ORDER BY updated_ms DESC, id DESC LIMIT ?1 OFFSET ?2"
                ),
                params![limit, offset, p],
            )?,
        };
        self.summaries(&rows)
    }

    /// Live children in sibling order, templates left out.
    pub fn children(
        &self,
        parent: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<PageSummary>> {
        let rows = self.rowids(
            &format!(
                "SELECT rowid FROM pages p WHERE parent IS ?3 AND in_trash = 0 AND {NOT_TEMPLATE}
                 ORDER BY sort_key, created_ms, id LIMIT ?1 OFFSET ?2"
            ),
            params![limit, offset, parent],
        )?;
        self.summaries(&rows)
    }

    /// The live templates of `parent` (marked, or its default), in sibling
    /// order.
    pub fn templates(&self, parent: &str) -> Result<Vec<PageSummary>> {
        let rows = self.rowids(
            "SELECT rowid FROM pages p WHERE parent = ?1 AND in_trash = 0 AND (is_template = 1
               OR EXISTS(SELECT 1 FROM pages tc WHERE tc.id = p.parent AND tc.template = p.id))
             ORDER BY sort_key, created_ms, id",
            params![parent],
        )?;
        self.summaries(&rows)
    }

    /// (id, sort key) of the live children, in sibling order.
    pub fn child_keys(&self, parent: Option<&str>) -> Result<Vec<(String, String)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, sort_key FROM pages WHERE parent IS ?1 AND in_trash = 0
             ORDER BY sort_key, created_ms, id",
        )?;
        let v = st
            .query_map(params![parent], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn last_child_key(&self, parent: Option<&str>) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached("SELECT max(sort_key) FROM pages WHERE parent IS ?1 AND in_trash = 0")?
            .query_row(params![parent], |r| r.get(0))?)
    }

    pub fn favorites(&self) -> Result<Vec<PageSummary>> {
        let rows = self.rowids(
            "SELECT rowid FROM pages WHERE favorite IS NOT NULL AND in_trash = 0
             ORDER BY favorite, id",
            [],
        )?;
        self.summaries(&rows)
    }

    /// The latest favorite time.
    pub fn last_favorite(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached("SELECT max(favorite) FROM pages")?
            .query_row([], |r| r.get(0))?)
    }

    /// Pages moved to the trash themselves (not those inside a trashed page).
    pub fn trash(&self) -> Result<Vec<TrashedPage>> {
        let mut st = self.conn.prepare_cached(
            "SELECT rowid, trashed FROM pages WHERE trashed IS NOT NULL ORDER BY trashed DESC, id",
        )?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
        let sums = self.summaries(&ids)?;
        Ok(sums
            .into_iter()
            .zip(rows)
            .map(|(page, (_, t))| TrashedPage {
                page,
                trashed_ms: t,
            })
            .collect())
    }

    /// Live pages linking to `id`, most recently updated first.
    pub fn backlinks(&self, id: &str) -> Result<Vec<PageSummary>> {
        let rows = self.rowids(
            "SELECT p.rowid FROM links l CROSS JOIN pages p ON p.id = l.src
             WHERE l.dst = ?1 AND p.in_trash = 0 AND p.id != ?1
             ORDER BY p.updated_ms DESC, p.id DESC",
            [id],
        )?;
        self.summaries(&rows)
    }

    /// Every page linking to a row of the external `source`, such as an embed
    /// of a chat. Their index text shows the row's title.
    pub fn link_sources_of_source(&self, source: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare_cached(
            "SELECT DISTINCT src FROM links
             WHERE dst IN (SELECT id FROM search_external WHERE source = ?1)",
        )?;
        let v = st
            .query_map([source], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// Every page (in the trash or not) linking to `id`.
    pub fn link_sources(&self, id: &str) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT src FROM links WHERE dst = ?1 AND src != ?1")?;
        let v = st
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// Apply filters before title selection, adaptive breadth, FTS ranking,
    /// and typo correction. A correction can never enlarge the candidate set.
    pub fn search_spec(
        &self,
        spec: &SearchSpec,
        limit: u32,
        offset: u32,
        full: bool,
    ) -> Result<SearchSpecResults> {
        let mut results = self.search_spec_rows(spec, limit, offset, full)?;
        for hit in &mut results.hits {
            self.publish_hit(hit)?;
        }
        Ok(results)
    }

    /// Whether a committed vault page can now enter this query. The registry
    /// separately checks the previous hits for removals/changes. This is a
    /// conservative eligibility probe: no ranking, snippets, or full search.
    pub(crate) fn search_spec_matches_ids(&self, spec: &SearchSpec, ids: &[PageId]) -> Result<bool> {
        if ids.is_empty() {
            return Ok(false);
        }
        // Item searches re-run on every relevant commit; agenda rows have
        // no cheap per-page probe.
        if spec.selects_items() {
            return Ok(true);
        }
        let positive: Vec<_> = spec.terms.iter().filter(|term| !term.exclude).collect();
        if positive.is_empty()
            && spec.operators.is_empty()
            && !spec.terms.iter().any(|term| term.exclude)
            && spec.sort.is_none()
            && spec.warnings.is_empty()
        {
            return Ok(false);
        }
        let fts = spec_fts_query(spec);
        if !positive.is_empty() && fts.is_none() {
            return Ok(false);
        }
        let query = positive.iter().map(|term| term.value.as_str()).collect::<Vec<_>>().join(" ");
        let words = fuzzy::query_words(&query);
        // Parameter one is the only typed rarray binding. Resolve stable page
        // ids to numeric rowids inside the same SQL statement/read snapshot.
        let array = Rc::new(ids.iter().cloned().map(Value::Text).collect::<Vec<_>>());
        let mut parameters = vec![Value::Null];
        let predicate = crate::search_exec::probe_predicate(self, spec, &mut parameters)?;
        let exact = if let Some(fts) = fts {
            parameters.push(Value::Text(fts));
            format!("p.rowid IN (SELECT rowid FROM fts WHERE fts MATCH ?{}
                AND +rowid IN (SELECT rowid FROM committed))", parameters.len())
        } else {
            "1".to_string()
        };
        let sql = format!(
            "WITH committed(rowid) AS MATERIALIZED (
                SELECT rowid FROM pages WHERE id IN rarray(?1)
             ), probe_schemas(value) AS MATERIALIZED (
                SELECT json_group_array(schema) FROM pages WHERE schema IS NOT NULL
             )
             SELECT p.title, {exact},
                CASE WHEN {exact} THEN '' ELSE coalesce((SELECT body FROM fts WHERE rowid = p.rowid), '') END
             FROM search_rows p
             WHERE p.rowid IN (SELECT rowid FROM committed) AND {predicate}"
        );
        let mut bindings: Vec<&dyn rusqlite::ToSql> = parameters.iter()
            .map(|value| value as &dyn rusqlite::ToSql).collect();
        bindings[0] = &array;
        let mut statement = self.conn.prepare_cached(&sql)?;
        let mut rows = statement.query(bindings.as_slice())?;
        while let Some(row) = rows.next()? {
            if row.get::<_, bool>(1)? {
                return Ok(true);
            }
            let title: String = row.get(0)?;
            let body: String = row.get(2)?;
            if probe_text_matches(&query, &words, title, &body) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Gives a hit its public identity. Search works on `search_rows` ids,
    /// where an external row is `source`, a unit separator and its local id.
    fn publish_hit(&self, hit: &mut SearchHit) -> Result<()> {
        if let Some((source, id)) = hit.id.split_once(SEP) {
            let (page_type, target) = self.conn.query_row(
                "SELECT page_type, target FROM search_external WHERE source = ?1 AND id = ?2",
                params![source, id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )?;
            hit.source = source.to_string();
            hit.id = id.to_string();
            hit.page_type = page_type;
            hit.target = target;
        } else {
            hit.source = crate::types::VAULT_SOURCE.to_string();
            hit.page_type = self
                .conn
                .query_row(
                    "SELECT page_type FROM pages WHERE id = ?1",
                    [&hit.id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_else(crate::types::default_page_type);
            hit.target = None;
        }
        Ok(())
    }

    fn search_spec_rows(
        &self,
        spec: &SearchSpec,
        limit: u32,
        offset: u32,
        full: bool,
    ) -> Result<SearchSpecResults> {
        if spec.selects_items() {
            return self.item_page_results(spec, limit, offset);
        }
        // SQLite total_changes includes rolled-back writes. Keep searches
        // inside a caller-owned transaction out of the persistent caches,
        // including subtree values whose epoch may repeat after rollback.
        let _cache_scope = (!self.conn.is_autocommit())
            .then(|| crate::search_exec::IsolatedCache::new(&self.search_filter_cache));
        // The writer uses another WAL connection. Pin one read snapshot so
        // candidate eligibility and FTS/content cannot straddle its commit.
        let _snapshot = self
            .conn
            .is_autocommit()
            .then(|| self.conn.unchecked_transaction())
            .transpose()?;
        let _: i64 = self
            .conn
            .query_row("SELECT value FROM search_tree_epoch", [], |r| r.get(0))?;
        let candidates = crate::search_exec::candidates(self, spec)?;
        if let Some(set) = candidates.as_ref().filter(|c| !c.small()) {
            let set = Arc::clone(set);
            self.conn.create_scalar_function(
                "search_candidate",
                1,
                rusqlite::functions::FunctionFlags::SQLITE_UTF8
                    | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
                move |ctx| Ok(set.rows.contains(&ctx.get::<i64>(0)?)),
            )?;
        }
        let candidate = candidates.as_deref();
        let mut warnings = spec.warnings.clone();
        if let Some(set) = candidate {
            warnings.extend(set.warnings.iter().cloned());
        }
        let positive: Vec<_> = spec.terms.iter().filter(|t| !t.exclude).collect();
        let query = positive
            .iter()
            .map(|t| t.value.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let q = spec_fts_query(spec);
        if positive.is_empty() {
            // A truly empty search retains the original empty-search behavior.
            if spec.operators.is_empty()
                && !spec.terms.iter().any(|t| t.exclude)
                && spec.sort.is_none()
                && spec.warnings.is_empty()
            {
                return Ok(SearchSpecResults {
                    hits: Vec::new(),
                    total_bounded: 0,
                    narrowed: false,
                    warnings,
                });
            }
            let total = if let Some(set) = candidate {
                set.ids.len().min(201) as u32
            } else {
                self.conn.prepare_cached(&format!("SELECT count(*) FROM (SELECT rowid FROM search_rows p WHERE p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{} LIMIT 201)", self.scope_clause("p.rowid")))?.query_row([], |r| r.get(0))?
            };
            let mut hits = self.search_sorted(spec, limit, offset, candidate, None)?;
            crate::search_exec::decorate(self, spec, &mut hits)?;
            return Ok(SearchSpecResults {
                hits,
                total_bounded: total,
                narrowed: false,
                warnings,
            });
        }
        let Some(q) = q else {
            return Ok(SearchSpecResults {
                hits: Vec::new(),
                total_bounded: 0,
                narrowed: false,
                warnings,
            });
        };
        if spec.sort.is_some() {
            // Sorting text results needs their complete eligibility set, not
            // a ranked prefix. The unranked collection avoids body/snippet reads.
            let ids = self.search_matching_ids(&q, &query, candidate, u32::MAX)?;
            let total = ids.len().min(201) as u32;
            let mut hits = self.search_sorted(spec, limit, offset, candidate, Some(&ids))?;
            self.label_sorted_corrections(&q, &query, &mut hits)?;
            crate::search_exec::decorate(self, spec, &mut hits)?;
            return Ok(SearchSpecResults {
                hits,
                total_bounded: total,
                narrowed: false,
                warnings,
            });
        }
        let fetch = limit.saturating_add(offset);
        let (mut hits, narrowed, membership_query) = if full {
            (
                self.search_full_filtered(&query, Some(&q), fetch, candidate)?,
                false,
                None,
            )
        } else {
            let (results, membership_query) =
                self.search_adaptive_filtered(&query, Some(&q), fetch, candidate)?;
            (results.hits, results.narrowed, membership_query)
        };
        let total = if hits.len() >= 201 {
            201
        } else {
            self.search_matching_ids(
                membership_query.as_deref().unwrap_or(&q),
                &query,
                candidate,
                201,
            )?
            .len()
            .min(201) as u32
        };
        hits = hits
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        crate::search_exec::decorate(self, spec, &mut hits)?;
        Ok(SearchSpecResults {
            hits,
            total_bounded: total,
            narrowed,
            warnings,
        })
    }

    fn filtered_count(&self, q: &str, candidates: Option<&Candidates>, cap: u32) -> Result<u32> {
        let restriction = self.restriction(candidates, 3);
        let sql = format!("SELECT count(*) FROM (SELECT fts.rowid FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
            WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction} LIMIT ?2)");
        Ok(self
            .conn
            .prepare_cached(&sql)?
            .query_row(params![q, cap, search_array(candidates)], |r| r.get(0))?)
    }

    fn search_matching_ids(
        &self,
        q: &str,
        query: &str,
        candidates: Option<&Candidates>,
        cap: u32,
    ) -> Result<HashSet<String>> {
        let restriction = self.restriction(candidates, 3);
        let sql = format!(
            "SELECT p.id FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
            WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction} LIMIT ?2"
        );
        let mut ids: HashSet<String> = self
            .conn
            .prepare_cached(&sql)?
            .query_map(params![q, cap, search_array(candidates)], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if ids.len() >= cap as usize {
            return Ok(ids);
        }
        ids.extend(
            self.search_titles(query, cap, candidates)?
                .into_iter()
                .map(|h| h.id),
        );
        if ids.len() >= cap as usize {
            return Ok(ids);
        }
        let words = fuzzy::query_words(query);
        if words.is_empty() {
            return Ok(ids);
        }
        let missing = self.missing_words(&words)?;
        if !missing.iter().any(|m| *m) {
            return Ok(ids);
        }
        self.refresh_search_titles()?;
        // Only the bounded union count (or complete sort membership) is
        // needed here. Ranking and correction labels belong to returned hits.
        let titles = self.fuzzy.borrow_mut().fuzzy_title_ids_scoped(
            &words,
            &missing,
            &ids,
            cap as usize - ids.len(),
            candidates.map(|c| &c.ids),
            self.page_scope.get(),
        );
        ids.extend(titles);
        if ids.len() >= cap as usize {
            return Ok(ids);
        }
        self.refresh_fuzzy()?;
        let (body, _) = self.body_corrections(
            &self.fuzzy.borrow().words,
            &words,
            &missing,
            &ids,
            cap as usize - ids.len(),
            false,
            candidates,
            true,
        )?;
        ids.extend(body.into_iter().map(|h| h.id));
        Ok(ids)
    }

    /// A sort override changes ordering, but retains fuzzy labels and the
    /// correction/snippet on the selected rows. Work stays bounded by output.
    fn label_sorted_corrections(&self, q: &str, query: &str, hits: &mut [SearchHit]) -> Result<()> {
        if hits.is_empty() {
            return Ok(());
        }
        let words = fuzzy::query_words(query);
        let missing = self.missing_words(&words)?;
        if !missing.iter().any(|m| *m) {
            return Ok(());
        }
        self.refresh_search_titles()?;
        self.refresh_fuzzy()?;
        for chunk in hits.chunks_mut(crate::search_exec::SMALL_CANDIDATES) {
            let ids: HashSet<String> = chunk.iter().map(|h| h.id.clone()).collect();
            let encoded = serde_json::to_string(&ids)
                .map_err(|e| crate::error::VaultError::Index(e.to_string()))?;
            let rows = self
                .conn
                .prepare_cached(
                    "SELECT rowid FROM pages WHERE id IN (SELECT value FROM json_each(?1))",
                )?
                .query_map([&encoded], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let subset = Candidates {
                ids,
                rows: rows.iter().copied().collect(),
                array: rows.into_iter().map(Value::Integer).collect(),
                warnings: Vec::new(),
            };
            let mut exact: HashSet<String> = self
                .conn
                .prepare_cached(
                    "SELECT p.id FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
                WHERE fts MATCH ?1 AND p.id IN (SELECT value FROM json_each(?2))",
                )?
                .query_map(params![q, encoded], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            exact.extend(
                fuzzy::title_hits_scoped(
                    &self.fuzzy.borrow().titles,
                    query,
                    &words,
                    &HashSet::new(),
                    chunk.len(),
                    false,
                    Some(&subset.ids),
                    self.page_scope.get(),
                )
                .into_iter()
                .map(|h| h.id),
            );
            let titles = self.fuzzy.borrow_mut().fuzzy_title_hits_scoped(
                &words,
                &missing,
                &exact,
                chunk.len(),
                Some(&subset.ids),
                self.page_scope.get(),
            );
            let mut corrected: HashMap<String, SearchHit> =
                titles.into_iter().map(|h| (h.id.clone(), h)).collect();
            exact.extend(corrected.keys().cloned());
            let (body, _) = self.body_corrections(
                &self.fuzzy.borrow().words,
                &words,
                &missing,
                &exact,
                chunk.len(),
                false,
                Some(&subset),
                false,
            )?;
            corrected.extend(body.into_iter().map(|h| (h.id.clone(), h)));
            for hit in chunk {
                if let Some(value) = corrected.remove(&hit.id) {
                    *hit = value;
                }
            }
        }
        Ok(())
    }

    fn search_sorted(
        &self,
        spec: &SearchSpec,
        limit: u32,
        offset: u32,
        candidates: Option<&Candidates>,
        ids: Option<&HashSet<String>>,
    ) -> Result<Vec<SearchHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut parameters = Vec::new();
        let mut predicate = format!("p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}");
        if let Some(ids) = ids {
            parameters
                .push(Value::Text(serde_json::to_string(ids).map_err(|e| {
                    crate::error::VaultError::Index(e.to_string())
                })?));
            predicate.push_str(" AND p.id IN (SELECT value FROM json_each(?1))");
        }
        let candidate_parameter = parameters.len() + 1;
        let array = search_array(candidates);
        // Use a placeholder in the vector for the one typed rarray binding.
        parameters.push(Value::Null);
        let from = if candidates.is_some_and(Candidates::small) {
            // Visit the eligible rows directly; an ordered-index walk can
            // otherwise test thousands of rows to fill a sparse result page.
            format!("rarray(?{candidate_parameter}) selected CROSS JOIN search_rows p ON p.rowid = selected.value")
        } else {
            predicate.push_str(&search_restriction(
                candidates,
                "p.rowid",
                candidate_parameter,
            ));
            "search_rows p".to_string()
        };
        predicate.push_str(&self.scope_clause("p.rowid"));
        let order = crate::search_exec::order_sql(spec, &mut parameters);
        parameters.push(Value::Integer(limit as i64));
        let limit_parameter = parameters.len();
        parameters.push(Value::Integer(offset as i64));
        let offset_parameter = parameters.len();
        let sql = format!("SELECT p.id, p.title, '' FROM {from} WHERE {predicate} ORDER BY {order} LIMIT ?{limit_parameter} OFFSET ?{offset_parameter}");
        let mut bindings: Vec<&dyn rusqlite::ToSql> = parameters
            .iter()
            .map(|v| v as &dyn rusqlite::ToSql)
            .collect();
        bindings[candidate_parameter - 1] = &array;
        Ok(self
            .conn
            .prepare_cached(&sql)?
            .query_map(bindings.as_slice(), search_hit)?
            .collect::<rusqlite::Result<_>>()?)
    }

    #[cfg(test)]
    pub fn search_adaptive(&self, query: &str, limit: u32) -> Result<SearchResults> {
        let q = fts_query(query);
        self.search_adaptive_filtered(query, q.as_deref(), limit, None)
            .map(|(results, _)| results)
    }

    fn search_adaptive_filtered(
        &self,
        query: &str,
        fts: Option<&str>,
        limit: u32,
        candidates: Option<&Candidates>,
    ) -> Result<(SearchResults, Option<String>)> {
        if limit == 0 {
            return Ok((
                SearchResults {
                    hits: Vec::new(),
                    narrowed: false,
                },
                None,
            ));
        }
        let Some(q) = fts else {
            return Ok((
                SearchResults {
                    hits: Vec::new(),
                    narrowed: false,
                },
                None,
            ));
        };
        let (narrowed, membership_query) = self.search_breadth_filtered(q, candidates)?;
        let hits = if narrowed {
            // Title matches first, then unranked body matches fill the rest.
            let mut hits = self.search_titles(query, limit, candidates)?;
            let fill = self.search_body_fill(
                membership_query.as_deref().unwrap_or(q),
                limit,
                &hits,
                candidates,
            )?;
            hits.extend(fill);
            hits
        } else {
            let titles = self.search_title_hits(query, q, limit, candidates)?;
            let body = self.search_ranked(q, query, limit, &titles, candidates)?;
            merge_titles(titles, body, limit)
        };
        // A broad corrected query can also narrow here, after its expansion.
        let (hits, corrected_narrowed) =
            self.with_corrections(query, limit, hits, true, candidates)?;
        Ok((
            SearchResults {
                hits,
                narrowed: narrowed || corrected_narrowed,
            },
            membership_query,
        ))
    }

    /// A bounded posting-list count, without ranking, body reads or snippets.
    /// Counting the conjunction also handles multi-word narrowing: it cannot
    /// be broader than any preceding term. Trashed/template postings can
    /// conservatively overestimate breadth, but never hide full API results.
    fn search_breadth_filtered(
        &self,
        q: &str,
        candidates: Option<&Candidates>,
    ) -> Result<(bool, Option<String>)> {
        let Some(threshold) = self.broad_threshold()? else {
            return Ok((false, None));
        };
        // No text conjunction can match more rows than its hard filter.
        if candidates.is_some_and(|set| set.rows.len() <= threshold as usize) {
            return Ok((false, None));
        }
        if candidates.is_some() || q.split_whitespace().any(|term| !term.ends_with('*')) {
            return Ok((
                self.filtered_count(q, candidates, threshold + 1)? > threshold,
                None,
            ));
        }
        let (count_query, membership_query) = self.search_prefix_queries(q)?;
        let Some(count_query) = count_query else {
            // All prefixes exceeded the capped dictionary scan. Live pages
            // give a conservative upper bound, so continue narrowing.
            return Ok((true, membership_query));
        };
        let count: u32 = self
            .conn
            .prepare_cached(&format!(
                "SELECT count(*) FROM (SELECT rowid FROM fts WHERE fts MATCH ?1{} LIMIT ?2)",
                self.scope_clause("rowid")
            ))?
            .query_row(params![count_query, threshold + 1], |r| r.get(0))?;
        let narrowed = count > threshold;
        Ok((narrowed, if narrowed { membership_query } else { None }))
    }

    /// The match count above which a search is broad. Small corpora never narrow.
    fn broad_threshold(&self) -> Result<Option<u32>> {
        let pages = self.search_page_count()?;
        if pages <= SEARCH_BREADTH.0 {
            return Ok(None);
        }
        Ok(Some(SEARCH_BREADTH.0.max(pages / SEARCH_BREADTH.1)))
    }

    #[cfg(test)]
    fn search_count_query(&self, q: &str) -> Result<Option<String>> {
        self.search_prefix_queries(q).map(|(count, _)| count)
    }

    /// The estimator may omit overflowing prefixes; result membership cannot.
    /// Reuse its exact-term expansions within this call for unranked multiword
    /// fill and bounded totals, retaining every overflowing prefix verbatim.
    /// Ranked hits, snippets, phrases and correction queries keep their MATCH.
    fn search_prefix_queries(&self, q: &str) -> Result<(Option<String>, Option<String>)> {
        let tokens = self
            .conn
            .prepare_cached(
                "SELECT token FROM search_tokenizer WHERE input = ?1 ORDER BY position",
            )?
            .query_map([q], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if tokens.is_empty() {
            return Ok((Some(q.into()), None));
        }
        // Only a conjunction of standalone quoted words has the same document
        // set after replacing its prefixes with exact-term ORs. A phrase or
        // operator must retain its original positional/boolean semantics.
        let reuse = tokens.len() > 1
            && q.split_whitespace().count() == tokens.len()
            && q.split_whitespace().all(|part| {
                part.strip_prefix('"')
                    .and_then(|word| word.strip_suffix("\"*"))
                    .is_some_and(|word| !word.is_empty() && word.chars().all(char::is_alphanumeric))
            });
        let mut parts = Vec::with_capacity(tokens.len());
        let mut exact = reuse.then(|| Vec::with_capacity(tokens.len()));
        for (position, token) in tokens.into_iter().enumerate() {
            if token.chars().count() <= 3 {
                // These lengths already have native prefix posting indexes.
                parts.push(format!("\"{token}\"*"));
                if let Some(exact) = &mut exact {
                    exact.push(format!("\"{token}\"*"));
                }
                continue;
            }
            let upper = format!("{token}\u{10ffff}");
            let mut lower = token.clone();
            let mut terms = Vec::new();
            let mut finished = false;
            for _ in 0..=16 {
                let next: Option<String> = self
                    .conn
                    .prepare_cached(
                        "SELECT term FROM search_terms WHERE term >= ?1 AND term < ?2 LIMIT 1",
                    )?
                    .query_row(params![lower, upper], |r| r.get(0))
                    .optional()?;
                let Some(term) = next else {
                    finished = true;
                    break;
                };
                // A strict > constraint filters duplicate instances after
                // reading them. An inclusive seek just above the term skips
                // its whole posting list. U+0001 cannot be a unicode61 token
                // character, and sorts before every possible continuation.
                lower = format!("{term}\u{1}");
                terms.push(format!("\"{term}\""));
            }
            if !finished {
                // Dropping an overflowing term from the breadth-only
                // conjunction gives an upper bound without building its
                // position union. Other selective terms still make this
                // query selective. Actual searches retain every term.
                // Preserve the original prefix in actual result membership.
                if let Some(exact) = &mut exact {
                    exact.push(q.split_whitespace().nth(position).unwrap().to_string());
                }
                continue;
            }
            if !terms.is_empty() {
                // Exact-term OR streams document ids lazily. The equivalent
                // long-prefix MATCH eagerly builds large position unions.
                let expanded = format!("({})", terms.join(" OR "));
                if let Some(exact) = &mut exact {
                    exact.push(expanded.clone());
                }
                parts.push(expanded);
            } else {
                // A prefix with no indexed term has no matches.
                parts.push(format!("\"{token}\"*"));
                if let Some(exact) = &mut exact {
                    exact.push(format!("\"{token}\"*"));
                }
            }
        }
        Ok((
            (!parts.is_empty()).then(|| parts.join(" AND ")),
            exact.map(|parts| parts.join(" AND ")),
        ))
    }

    fn search_page_count(&self) -> Result<u32> {
        // data_version observes commits by the background writer's separate
        // connection; total_changes observes every write on this connection.
        // This avoids scanning the live-pages index on every keystroke while
        // keeping additions, trash changes and reindexing immediately visible.
        let version: i64 = self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?;
        let changes = self.conn.total_changes();
        let (all, pages) = match self.search_count_cache.get() {
            Some((cached_version, cached_changes, all, pages))
                if version == cached_version && changes == cached_changes =>
            {
                (all, pages)
            }
            _ => {
                // Live external rows are searchable hits too, so they count
                // toward breadth. `search_rows` holds both kinds of live row.
                let (all, pages): (i64, i64) = self.conn.query_row(
                    &format!("SELECT count(*), coalesce(sum(rowid > 0), 0) FROM search_rows p WHERE in_trash = 0 AND {NOT_FEEDBACK}"),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let counts = (all as u32, pages as u32);
                self.search_count_cache
                    .set(Some((version, changes, counts.0, counts.1)));
                counts
            }
        };
        Ok(if self.page_scope.get() { pages } else { all })
    }

    /// The page-only scope as a predicate on `column` (a rowid).
    pub(crate) fn scope_clause(&self, column: &str) -> String {
        if self.page_scope.get() {
            format!(" AND {column} > 0")
        } else {
            String::new()
        }
    }

    /// The FTS restriction for candidates, narrowed to vault pages when scoped.
    fn restriction(&self, candidates: Option<&Candidates>, parameter: usize) -> String {
        format!(
            "{}{}",
            search_restriction(candidates, "fts.rowid", parameter),
            self.scope_clause("fts.rowid")
        )
    }

    /// Search over vault pages only. External rows are excluded before
    /// ranking, the limit and corrections, so any number of external hits
    /// cannot push page hits out of the results.
    pub fn search_spec_pages(
        &self,
        spec: &SearchSpec,
        limit: u32,
        offset: u32,
    ) -> Result<SearchSpecResults> {
        self.page_scope.set(true);
        let result = self.search_spec(spec, limit, offset, false);
        self.page_scope.set(false);
        result
    }

    fn search_titles(
        &self,
        query: &str,
        limit: u32,
        candidates: Option<&Candidates>,
    ) -> Result<Vec<SearchHit>> {
        self.refresh_search_titles()?;
        let words = fuzzy::query_words(query);
        Ok(fuzzy::title_hits_scoped(
            &self.fuzzy.borrow().titles,
            query,
            &words,
            &HashSet::new(),
            limit as usize,
            false,
            candidates.map(|c| &c.ids),
            self.page_scope.get(),
        ))
    }

    /// Title hits in their title-first order, each with the snippet FTS gives
    /// the page for `fts`. Title ids are excluded from body searches, so one
    /// snippet query over these ids is their only source. Broad (narrowed)
    /// searches skip this and keep their unsnippeted hits.
    fn search_title_hits(
        &self,
        query: &str,
        fts: &str,
        limit: u32,
        candidates: Option<&Candidates>,
    ) -> Result<Vec<SearchHit>> {
        let mut titles = self.search_titles(query, limit, candidates)?;
        if titles.is_empty() {
            return Ok(titles);
        }
        let ids = serde_json::to_string(&titles.iter().map(|h| &h.id).collect::<Vec<_>>())
            .map_err(|e| crate::error::VaultError::Index(e.to_string()))?;
        // Resolve these ids once. A direct json_each join can instead scan
        // the JSON list for every corpus-wide FTS match.
        let snippets: HashMap<String, String> = self
            .conn
            .prepare_cached(
                "SELECT p.id, snippet(fts, 1, '', '', '…', 12)
                 FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
                 WHERE fts MATCH ?1 AND +fts.rowid IN (
                   SELECT selected.rowid FROM json_each(?2) AS ids
                   JOIN search_rows selected ON selected.id = ids.value
                 )",
            )?
            .query_map(params![fts, ids], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for hit in &mut titles {
            if let Some(snippet) = snippets.get(&hit.id) {
                hit.snippet.clone_from(snippet);
            }
        }
        Ok(titles)
    }

    /// Catch writer/external commits without rebuilding the word index. Local
    /// metadata writes also patch this list immediately (`write_meta`).
    fn refresh_search_titles(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?;
        let guard = (version, self.conn.total_changes());
        if self.titles_guard.get() == Some(guard) {
            return Ok(());
        }
        let titles = self
            .conn
            .prepare_cached(
                &format!("SELECT p.id, p.title, p.updated_ms FROM search_rows p WHERE p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}"),
            )?
            .query_map([], |r| {
                Ok(fuzzy::TitleEntry::new(r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        *self.title_positions.borrow_mut() = titles
            .iter()
            .enumerate()
            .map(|(n, t)| (t.id.clone(), n))
            .collect();
        let mut cache = self.fuzzy.borrow_mut();
        cache.clear_title_matches();
        cache.titles = titles;
        self.titles_guard.set(Some(guard));
        Ok(())
    }

    fn update_search_title(&self, m: &MetaRow, in_trash: bool) {
        // Only the foreground connection initializes a title cache. The
        // writer connection need not retain a second copy of all titles.
        if self.titles_guard.get().is_none() {
            return;
        }
        let mut cache = self.fuzzy.borrow_mut();
        cache.clear_title_matches();
        let mut positions = self.title_positions.borrow_mut();
        if in_trash
            || m.is_template
            || matches!(m.role.as_deref(), Some("calendars" | "nodes" | "feedback"))
        {
            if let Some(n) = positions.remove(&m.id) {
                cache.titles.swap_remove(n);
                if let Some(moved) = cache.titles.get(n) {
                    positions.insert(moved.id.clone(), n);
                }
            }
        } else {
            let entry = fuzzy::TitleEntry::new(m.id.clone(), m.title.clone(), m.updated);
            if let Some(&n) = positions.get(&m.id) {
                cache.titles[n] = entry;
            } else {
                positions.insert(m.id.clone(), cache.titles.len());
                cache.titles.push(entry);
            }
        }
    }

    /// Body matches by recency then id, without bm25 or snippets, skipping the
    /// pages already returned as title matches. Returns at most `limit` hits.
    fn search_body_fill(
        &self,
        q: &str,
        limit: u32,
        titles: &[SearchHit],
        candidates: Option<&Candidates>,
    ) -> Result<Vec<SearchHit>> {
        let remaining = limit.saturating_sub(titles.len() as u32) as usize;
        if remaining == 0 {
            return Ok(Vec::new());
        }
        // Each title hit can occupy at most one of the scanned rows.
        let scan = (remaining + titles.len()) as u32;
        // Stable across reindex: FTS rowids change, page recency and id do not.
        let restriction = self.restriction(candidates, 3);
        let sql = format!(
            "SELECT p.id, p.title, ''
             FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
             WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction}
             ORDER BY p.updated_ms DESC, p.id LIMIT ?2"
        );
        let mut st = self.conn.prepare_cached(&sql)?;
        let rows = st
            .query_map(params![q, scan, search_array(candidates)], search_hit)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter(|h| !titles.iter().any(|t| t.id == h.id))
            .take(remaining)
            .collect())
    }

    #[cfg(test)]
    pub fn search_full(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
        let q = fts_query(query);
        self.search_full_filtered(query, q.as_deref(), limit, None)
    }

    fn search_full_filtered(
        &self,
        query: &str,
        fts: Option<&str>,
        limit: u32,
        candidates: Option<&Candidates>,
    ) -> Result<Vec<SearchHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some(q) = fts else {
            return Ok(Vec::new());
        };
        let titles = self.search_title_hits(query, q, limit, candidates)?;
        let body = self.search_ranked(q, query, limit, &titles, candidates)?;
        let hits = merge_titles(titles, body, limit);
        let (hits, _) = self.with_corrections(query, limit, hits, false, candidates)?;
        Ok(hits)
    }

    /// Per query word: true when the word is missing from the FTS vocabulary.
    /// A word before the last is missing when no posting exists for that
    /// exact term. The last word, still being typed, is missing when no
    /// vocabulary term starts with it. Each word costs one indexed lookup on
    /// the live fts5vocab table, which needs no cache rebuild.
    fn missing_words(&self, words: &[String]) -> Result<Vec<bool>> {
        let last = words.len() - 1;
        let mut exact = self
            .conn
            .prepare_cached("SELECT 1 FROM search_vocab WHERE term = ?1 LIMIT 1")?;
        let mut from = self
            .conn
            .prepare_cached("SELECT term FROM search_vocab WHERE term >= ?1 LIMIT 1")?;
        let mut missing = Vec::with_capacity(words.len());
        for (i, word) in words.iter().enumerate() {
            let absent = if i == last {
                // The first term at or after the prefix is the only one to test.
                let next: Option<String> = from.query_row([word], |r| r.get(0)).optional()?;
                !next.is_some_and(|t| t.starts_with(word.as_str()))
            } else {
                exact.query_row([word], |_| Ok(())).optional()?.is_none()
            };
            missing.push(absent);
        }
        Ok(missing)
    }

    /// Fill remaining slots with fuzzy title hits, then correct body words.
    /// Only words missing from the vocabulary (`missing_words`) are corrected;
    /// when every word exists, no fuzzy work runs. The exact hits keep their
    /// places above every corrected hit. The result count plays no part.
    fn with_corrections(
        &self,
        query: &str,
        limit: u32,
        mut hits: Vec<SearchHit>,
        allow_fill: bool,
        candidates: Option<&Candidates>,
    ) -> Result<(Vec<SearchHit>, bool)> {
        if limit == 0 || hits.len() >= limit as usize {
            return Ok((hits, false));
        }
        let words = fuzzy::query_words(query);
        if words.is_empty() {
            return Ok((hits, false));
        }
        let missing = self.missing_words(&words)?;
        if !missing.iter().any(|m| *m) {
            return Ok((hits, false));
        }
        self.refresh_search_titles()?;
        let mut skip: HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
        let room = limit as usize - hits.len();
        let titles = self.fuzzy.borrow_mut().fuzzy_title_hits_scoped(
            &words,
            &missing,
            &skip,
            room,
            candidates.map(|c| &c.ids),
            self.page_scope.get(),
        );
        skip.extend(titles.iter().map(|h| h.id.clone()));
        hits.extend(titles);
        let room = limit as usize - hits.len();
        let mut narrowed = false;
        if room > 0 {
            self.refresh_fuzzy()?;
            let cache = self.fuzzy.borrow();
            let (body, filled) = self.body_corrections(
                &cache.words,
                &words,
                &missing,
                &skip,
                room,
                allow_fill,
                candidates,
                false,
            )?;
            hits.extend(body);
            narrowed = filled;
        }
        Ok((hits, narrowed))
    }

    /// Rebuilds the vocabulary only when stale. Local changes and writer
    /// commits invalidate it, while populated exact/prefix searches never
    /// check or rebuild this word index. Titles have their own cheap guard.
    pub fn refresh_fuzzy(&self) -> Result<()> {
        self.refresh_search_titles()?;
        let version: i64 = self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?;
        let guard = (version, self.conn.total_changes());
        if self.fuzzy.borrow().guard == Some(guard) {
            return Ok(());
        }
        let start = Instant::now();
        let terms = self
            .conn
            .prepare_cached("SELECT term, doc FROM search_vocab")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let words = fuzzy::WordIndex::build(terms);
        let build_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut cache = self.fuzzy.borrow_mut();
        cache.guard = Some(guard);
        cache.words = words;
        cache.build_ms = build_ms;
        Ok(())
    }

    /// The built word index's size; zeros before the first build.
    pub fn search_index_stats(&self) -> SearchIndexStats {
        let cache = self.fuzzy.borrow();
        SearchIndexStats {
            words: cache.words.words.len() as u32,
            deletion_keys: cache.words.key_count() as u32,
            estimated_bytes: cache.words.estimated_bytes(),
            build_ms: cache.build_ms,
        }
    }

    /// Body matches for corrected query words, in one FTS query. A word is its
    /// typed form OR its candidates. Last-word candidates were selected by
    /// distance to word starts, but are complete indexed vocabulary terms.
    /// Rows rank by bm25 plus `EDIT_PENALTY` per edit (bm25 is lower-better).
    /// Snippets are read only for the returned rows.
    fn body_corrections(
        &self,
        words: &fuzzy::WordIndex,
        query_words: &[String],
        missing: &[bool],
        skip: &HashSet<String>,
        room: usize,
        allow_fill: bool,
        candidates: Option<&Candidates>,
        count_only: bool,
    ) -> Result<(Vec<SearchHit>, bool)> {
        let last = query_words.len() - 1;
        let mut scratch = Scratch::default();
        let mut phrases = Vec::new();
        let mut groups = Vec::with_capacity(query_words.len());
        let mut any = false;
        let mut fixed_edits = Some(0u32);
        for (i, word) in query_words.iter().enumerate() {
            // Words present in the vocabulary keep their typed term alone.
            // Only missing words are expanded, whatever the other words do.
            let candidates = if !missing[i] {
                Vec::new()
            } else {
                let chars: Vec<char> = word.chars().collect();
                if i == last {
                    words.correct_prefix(&chars, &mut scratch)
                } else {
                    words.correct(&chars, &mut scratch)
                }
            };
            // If every possible matching term in each group has the same
            // cost, the per-row edit penalty is constant. Avoid position
            // decoding for ranking in that case, keeping actual corrections
            // for the limited result rows. Mixed costs still use search_edits.
            let start = words
                .words
                .partition_point(|(term, _)| term.as_str() < word.as_str());
            let prefix_exists = words
                .words
                .get(start)
                .is_some_and(|(term, _)| term.starts_with(word));
            let mut costs = prefix_exists
                .then_some(0)
                .into_iter()
                .chain(candidates.iter().map(|c| c.edits));
            let Some(cost) = costs.next() else {
                return Ok((Vec::new(), false));
            };
            if costs.any(|other| other != cost) {
                fixed_edits = None;
            } else if let Some(total) = &mut fixed_edits {
                *total += cost;
            }
            let mut terms = vec![format!("\"{word}\"*")];
            phrases.push(crate::fts_rank::Phrase {
                group: i as u32,
                edits: 0,
                term: word.clone(),
            });
            for candidate in &candidates {
                // Prefix correction already expanded to up to eight actual
                // vocabulary words. Expanding those words again with `*`
                // would bypass that cap and inflate FTS posting-list work.
                terms.push(format!("\"{}\"", candidate.term));
                phrases.push(crate::fts_rank::Phrase {
                    group: i as u32,
                    edits: candidate.edits,
                    term: candidate.term.clone(),
                });
            }
            any |= !candidates.is_empty();
            groups.push(format!("({})", terms.join(" OR ")));
        }
        if !any {
            return Ok((Vec::new(), false));
        }
        let q = groups.join(" AND ");
        if count_only {
            let excluded = serde_json::to_string(skip)
                .map_err(|e| crate::error::VaultError::Index(e.to_string()))?;
            let restriction = self.restriction(candidates, 4);
            let sql = format!(
                "SELECT p.id, p.title, '' FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
                WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction}
                AND p.id NOT IN (SELECT value FROM json_each(?3)) LIMIT ?2"
            );
            let hits = self
                .conn
                .prepare_cached(&sql)?
                .query_map(
                    params![q, room as u32, excluded, search_array(candidates)],
                    search_hit,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok((hits, false));
        }
        let spec = serde_json::to_string(&phrases)
            .map_err(|e| crate::error::VaultError::Index(e.to_string()))?;
        let excluded = serde_json::to_string(skip)
            .map_err(|e| crate::error::VaultError::Index(e.to_string()))?;
        // Broad expansions with one constant edit cost fill like exact broad
        // searches: no bm25 or edit ranking, no snippets. Every matching row
        // then carries the same penalty, so recency order loses nothing that
        // the penalty would have decided. Mixed costs stay on the ranked path.
        if let (true, Some(_)) = (allow_fill, fixed_edits) {
            if let Some(threshold) = self.broad_threshold()? {
                let count = self.filtered_count(&q, candidates, threshold + 1)?;
                if count > threshold {
                    let restriction = self.restriction(candidates, 5);
                    let sql = format!(
                        "SELECT p.id, p.title, '', search_correction(fts, ?2)
                             FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
                             WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction}
                               AND p.id NOT IN (SELECT value FROM json_each(?4))
                             ORDER BY p.updated_ms DESC, p.id LIMIT ?3"
                    );
                    let hits = self
                        .conn
                        .prepare_cached(&sql)?
                        .query_map(
                            params![q, spec, room as u32, excluded, search_array(candidates)],
                            |r| {
                                Ok(SearchHit {
                                    id: r.get(0)?,
                                    title: r.get(1)?,
                                    snippet: r.get(2)?,
                                    fuzzy: true,
                                    corrected: Some(r.get(3)?),
                                    matched_filters: Vec::new(),
                                    source: String::new(),
                                    page_type: String::new(),
                                    target: None,
                                })
                            },
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    return Ok((hits, true));
                }
            }
        }
        // FTS5 supplies the matched phrase IDs to our auxiliary function.
        // Ranking does not load bodies or issue a query per candidate/hit.
        // Snippets and actual matched corrections are evaluated after LIMIT.
        let restriction = self.restriction(candidates, 7);
        let sql = format!(
            "WITH ranked AS MATERIALIZED (
               SELECT fts.rowid AS rowid, p.id, p.title, p.updated_ms,
                      bm25(fts, 10.0, 1.0) + ?5 *
                        CASE WHEN ?6 IS NULL THEN search_edits(fts, ?2) ELSE ?6 END AS score
               FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
               WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction}
                 AND p.id NOT IN (SELECT value FROM json_each(?4))
               ORDER BY score, p.updated_ms DESC, p.id LIMIT ?3
             )
             SELECT ranked.id, ranked.title, snippet(fts, 1, '', '', '…', 12),
                    search_correction(fts, ?2)
             FROM fts CROSS JOIN ranked ON fts.rowid = ranked.rowid
             WHERE fts MATCH ?1 AND +fts.rowid IN (SELECT rowid FROM ranked)
             ORDER BY ranked.score, ranked.updated_ms DESC, ranked.id"
        );
        let mut st = self.conn.prepare_cached(&sql)?;
        let hits = st
            .query_map(
                params![
                    q,
                    spec,
                    room as u32,
                    excluded,
                    EDIT_PENALTY,
                    fixed_edits,
                    search_array(candidates)
                ],
                |r| {
                    Ok(SearchHit {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        snippet: r.get(2)?,
                        fuzzy: true,
                        corrected: Some(r.get(3)?),
                        matched_filters: Vec::new(),
                        source: String::new(),
                        page_type: String::new(),
                        target: None,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((hits, false))
    }

    fn search_ranked(
        &self,
        q: &str,
        query: &str,
        limit: u32,
        titles: &[SearchHit],
        candidates: Option<&Candidates>,
    ) -> Result<Vec<SearchHit>> {
        let remaining = limit.saturating_sub(titles.len() as u32);
        if remaining == 0 {
            return Ok(Vec::new());
        }
        // Title hits already have their final rank. Avoid ranking and reading
        // snippets for duplicates, particularly pages with very large bodies.
        let excluded = serde_json::to_string(&titles.iter().map(|h| &h.id).collect::<Vec<_>>())
            .map_err(|e| crate::error::VaultError::Index(e.to_string()))?;
        let restriction = self.restriction(candidates, 5);
        let sql = format!(
            "WITH ranked AS MATERIALIZED (
               SELECT fts.rowid AS rowid, p.id, p.title,
                      p.title_fold = ?3 AS exact, bm25(fts, 10.0, 1.0) AS score,
                      p.updated_ms
               FROM fts CROSS JOIN search_rows p ON p.rowid = fts.rowid
               WHERE fts MATCH ?1 AND p.in_trash = 0 AND {NOT_TEMPLATE} AND {NOT_FEEDBACK}{restriction}
                 AND p.id NOT IN (SELECT value FROM json_each(?4))
               ORDER BY exact DESC, score, p.updated_ms DESC, p.id
               LIMIT ?2
             )
             SELECT ranked.id, ranked.title, snippet(fts, 1, '', '', '…', 12)
             FROM fts CROSS JOIN ranked ON fts.rowid = ranked.rowid
             WHERE fts MATCH ?1 AND +fts.rowid IN (SELECT rowid FROM ranked)
             ORDER BY ranked.exact DESC, ranked.score, ranked.updated_ms DESC, ranked.id"
        );
        let mut st = self.conn.prepare_cached(&sql)?;
        let hits = st
            .query_map(
                params![
                    q,
                    remaining,
                    fold(query.trim()),
                    excluded,
                    search_array(candidates)
                ],
                search_hit,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(hits)
    }

    /// Inserts or replaces a page's full row (metadata, properties, content).
    pub fn upsert(&self, m: &MetaRow, c: &ContentRow) -> Result<()> {
        let rowid = match self.rowid(&m.id)? {
            Some(r) => r,
            None => {
                self.conn
                    .prepare_cached(
                        "INSERT INTO pages(id, title, created_ms, updated_ms) VALUES (?1, ?2, ?3, ?4)",
                    )?
                    .execute(params![m.id, m.title, m.created, m.updated])?;
                let rowid = self.conn.last_insert_rowid();
                self.conn
                    .prepare_cached("INSERT INTO fts(rowid, title, body) VALUES (?1, ?2, '')")?
                    .execute(params![rowid, m.title])?;
                rowid
            }
        };
        self.write_meta(rowid, m, true)?;
        self.write_content(rowid, &m.id, m.updated, c)
    }

    /// Writes the metadata and properties of an existing page. Returns false
    /// if the page is not in the index.
    pub fn update_meta(&self, m: &MetaRow) -> Result<bool> {
        // The foreground connection and background writer share this index.
        // Keep properties and their derived rows in one write transaction;
        // callers doing a bulk write already own the transaction.
        let tx = self
            .conn
            .is_autocommit()
            .then(|| self.write_tx())
            .transpose()?;
        let found = match self.rowid(&m.id)? {
            Some(rowid) => {
                self.write_meta(rowid, m, false)?;
                true
            }
            None => false,
        };
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(found)
    }

    fn write_meta(&self, rowid: i64, m: &MetaRow, exact_updated: bool) -> Result<()> {
        let parent_in_trash: bool = match &m.parent {
            Some(p) => self.in_trash(p)?.unwrap_or(false),
            None => false,
        };
        let in_trash = m.trashed.is_some() || parent_in_trash;
        let old: (String, Option<String>, i64, Option<String>, Option<String>, Option<String>) = self
            .conn
            .prepare_cached("SELECT title, parent, in_trash, schema, agenda_role, agenda_date FROM pages WHERE rowid = ?1")?
            .query_row([rowid], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })?;
        let sql = if exact_updated {
            "UPDATE pages SET title = ?2, title_fold = ?3, parent = ?4, created_ms = ?5,
               updated_ms = ?6, sort_key = ?7, icon = ?8, cover = ?9, favorite = ?10,
               trashed = ?11, in_trash = ?12, page_type = ?13, schema = ?14, template = ?15,
               is_template = ?16, color = ?17, role = ?18, agenda_role = ?19, agenda_date = ?20
             WHERE rowid = ?1"
        } else {
            "UPDATE pages SET title = ?2, title_fold = ?3, parent = ?4, created_ms = ?5,
               updated_ms = max(updated_ms, ?6), sort_key = ?7, icon = ?8, cover = ?9,
               favorite = ?10, trashed = ?11, in_trash = ?12, page_type = ?13,
               schema = ?14, template = ?15, is_template = ?16, color = ?17, role = ?18, agenda_role = ?19, agenda_date = ?20
             WHERE rowid = ?1"
        };
        self.conn.prepare_cached(sql)?.execute(params![
            rowid,
            m.title,
            fold(&m.title),
            m.parent,
            m.created,
            m.updated,
            m.sort_key,
            m.icon,
            m.cover,
            m.favorite,
            m.trashed,
            in_trash as i64,
            m.page_type,
            m.schema,
            m.template,
            m.is_template as i64,
            m.color,
            m.role,
            m.agenda_role,
            m.agenda_date,
        ])?;
        if old.0 != m.title {
            self.conn
                .prepare_cached("UPDATE fts SET title = ?2 WHERE rowid = ?1")?
                .execute(params![rowid, m.title])?;
        }
        if old.2 != in_trash as i64 || old.1 != m.parent {
            self.refresh_trash(&m.id)?;
        }
        self.write_props(rowid, &m.props, m.task_status.as_deref())?;
        let old_schema = (old.3 != m.schema).then_some(old.3.as_deref());
        self.computed_after_write(&m.id, old_schema)?;
        if !self.defer.get() {
            self.refresh_agenda(&m.id, None)?;
            if old.3 != m.schema || old.4 != m.agenda_role || old.5 != m.agenda_date {
                self.refresh_collection_agenda(&m.id)?;
            }
        }
        self.update_search_title(m, in_trash);
        Ok(())
    }

    fn write_props(&self, rowid: i64, props: &[Property], task_status: Option<&str>) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM props WHERE page = ?1")?
            .execute([rowid])?;
        let mut st = self.conn.prepare_cached(
            "INSERT INTO props(page, key, t, tf, n, d0, d1, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let projected_status = task_status.map(|status| Property {
            key: "status".into(),
            value: PropValue::Status(status.into()),
        });
        // Task-note status owns this indexed key on task pages. Preserve the
        // source property itself and all properties of non-task pages.
        for p in props.iter()
            .filter(|p| task_status.is_none() || p.key != "status")
            .chain(projected_status.iter()) {
            if p.value == PropValue::Empty {
                continue;
            }
            let (t, mut tf, n, d0, d1) = prop_cols(&p.value);
            if p.key == "status" {
                if let Some(status) = task_status {
                    // A real Text schema sharing this key keeps its existing
                    // text comparison semantics when searching task rows too.
                    tf = Some(fold(status));
                }
            }
            let json = serde_json::to_string(&p.value).unwrap_or_default();
            st.execute(params![rowid, p.key, t, tf, n, d0, d1, json])?;
        }
        Ok(())
    }

    /// Writes the writer-derived fields. Returns false if the page is not in
    /// the index.
    pub fn update_content(&self, id: &str, updated: i64, c: &ContentRow) -> Result<bool> {
        match self.rowid(id)? {
            Some(rowid) => {
                self.write_content(rowid, id, updated, c)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn write_content(&self, rowid: i64, id: &str, updated: i64, c: &ContentRow) -> Result<()> {
        self.conn
            .prepare_cached(
                "UPDATE pages SET updated_ms = max(updated_ms, ?2),
                   md_version = CASE WHEN ?3 IS NULL THEN md_version ELSE ?5 END,
                   md_path = coalesce(?3, md_path), md_hash = coalesce(?4, md_hash),
                   vv = coalesce(?6, vv)
                 WHERE rowid = ?1",
            )?
            .execute(params![
                rowid,
                updated,
                c.md_path,
                c.md_hash,
                c.md_version,
                c.vv
            ])?;
        self.conn
            .prepare_cached("UPDATE fts SET body = ?2 WHERE rowid = ?1")?
            .execute(params![rowid, c.body])?;
        self.set_set("links", "src", "dst", id, &c.links)?;
        self.set_set("refs", "page", "blob", id, &c.refs)?;
        self.refresh_agenda(id, Some(&c.todos))
    }

    fn set_set(&self, table: &str, a: &str, b: &str, id: &str, items: &[String]) -> Result<()> {
        self.conn
            .prepare_cached(&format!("DELETE FROM {table} WHERE {a} = ?1"))?
            .execute([id])?;
        let mut st = self.conn.prepare_cached(&format!(
            "INSERT OR IGNORE INTO {table}({a}, {b}) VALUES (?1, ?2)"
        ))?;
        for x in items {
            st.execute(params![id, x])?;
        }
        Ok(())
    }

    /// Recomputes `in_trash` for the page and everything below it.
    pub fn refresh_trash(&self, id: &str) -> Result<()> {
        let (parent, trashed): (Option<String>, Option<i64>) = match self
            .conn
            .prepare_cached("SELECT parent, trashed FROM pages WHERE id = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
        {
            Some(x) => x,
            None => return Ok(()),
        };
        let parent_in = match &parent {
            Some(p) => self.in_trash(p)?.unwrap_or(false),
            None => false,
        };
        let flag = trashed.is_some() || parent_in;
        self.conn
            .prepare_cached(
                "WITH RECURSIVE t(id, flag) AS (
                   SELECT ?1, ?2
                   UNION
                   SELECT p.id, (t.flag OR p.trashed IS NOT NULL) FROM pages p JOIN t ON p.parent = t.id
                 )
                 UPDATE pages SET in_trash = (SELECT max(flag) FROM t WHERE t.id = pages.id)
                 WHERE id IN (SELECT id FROM t)",
            )?
            .execute(params![id, flag as i64])?;
        Ok(())
    }

    /// Recomputes `in_trash` for every page (after a rebuild).
    pub fn refresh_all_trash(&self) -> Result<()> {
        self.conn.execute_batch(
            "UPDATE pages SET in_trash = 0;
             WITH RECURSIVE t(id) AS (
               SELECT id FROM pages WHERE trashed IS NOT NULL
               UNION
               SELECT p.id FROM pages p JOIN t ON p.parent = t.id
             )
             UPDATE pages SET in_trash = 1 WHERE id IN (SELECT id FROM t);",
        )?;
        Ok(())
    }

    /// The page and all its descendants, the page first: (id, md path).
    pub fn subtree(&self, id: &str) -> Result<Vec<(String, Option<String>)>> {
        let mut st = self.conn.prepare(
            "WITH RECURSIVE t(id) AS (SELECT ?1 UNION SELECT p.id FROM pages p JOIN t ON p.parent = t.id)
             SELECT p.id, p.md_path FROM t JOIN pages p ON p.id = t.id",
        )?;
        let v = st
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Whether `ancestor` is `id` or one of its ancestors.
    pub fn is_ancestor(&self, ancestor: &str, id: &str) -> Result<bool> {
        let mut cur = Some(id.to_string());
        let mut depth = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return Ok(true);
            }
            depth += 1;
            if depth > 10_000 {
                break;
            }
            cur = self.parent_of(&c)?.flatten();
        }
        Ok(false)
    }

    pub fn delete(&self, ids: &[String]) -> Result<()> {
        let dependents = self.dependents_of(ids)?;
        let tx = self.write_tx()?;
        for id in ids {
            tx.execute(
                "DELETE FROM fts WHERE rowid = (SELECT rowid FROM pages WHERE id = ?1)",
                [id],
            )?;
            tx.execute(
                "DELETE FROM props WHERE page = (SELECT rowid FROM pages WHERE id = ?1)",
                [id],
            )?;
            tx.execute("DELETE FROM links WHERE src = ?1", [id])?;
            tx.execute("DELETE FROM refs WHERE page = ?1", [id])?;
            // Conflict history is retained independently from loadable pages;
            // purge archives still own these mirrored records.
            tx.execute(
                "DELETE FROM agenda WHERE page = (SELECT rowid FROM pages WHERE id = ?1)",
                [id],
            )?;
            tx.execute("DELETE FROM pages WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        self.refresh_ids(&dependents)
    }

    /// Adds or replaces an external row. Its text joins the FTS table under
    /// the negated external rowid, so it ranks and corrects like page text.
    pub fn upsert_search_row(&self, source: &str, row: &ExternalSearchRow) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO search_external(source, id, title, body, page_type, target, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source, id) DO UPDATE SET title = excluded.title, body = excluded.body,
               page_type = excluded.page_type, target = excluded.target,
               updated_ms = excluded.updated_ms",
            params![
                source,
                row.id,
                row.title,
                row.body,
                row.page_type,
                row.target,
                row.updated_ms
            ],
        )?;
        let rowid: i64 = tx.query_row(
            "SELECT rowid FROM search_external WHERE source = ?1 AND id = ?2",
            params![source, row.id],
            |r| r.get(0),
        )?;
        tx.execute("DELETE FROM fts WHERE rowid = ?1", [-rowid])?;
        tx.execute(
            "INSERT INTO fts(rowid, title, body) VALUES (?1, ?2, ?3)",
            params![-rowid, row.title, row.body],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Removes one external row; a missing row is not an error.
    pub fn delete_search_row(&self, source: &str, id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let rowid: Option<i64> = tx
            .query_row(
                "SELECT rowid FROM search_external WHERE source = ?1 AND id = ?2",
                params![source, id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(rowid) = rowid {
            tx.execute("DELETE FROM fts WHERE rowid = ?1", [-rowid])?;
            tx.execute("DELETE FROM search_external WHERE rowid = ?1", [rowid])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes every external row of one source. Other sources are untouched.
    pub fn clear_search_source(&self, source: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM fts WHERE rowid IN (SELECT -rowid FROM search_external WHERE source = ?1)",
            [source],
        )?;
        tx.execute("DELETE FROM search_external WHERE source = ?1", [source])?;
        tx.commit()?;
        Ok(())
    }

    /// Clears everything but the attachments table and external search rows.
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM agenda; DELETE FROM fts WHERE rowid > 0; DELETE FROM pages; DELETE FROM props; DELETE FROM links; DELETE FROM refs;
             DELETE FROM conflicts; DELETE FROM meta;",
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------- sync

    /// Every page's id and encoded version vector (None if not recorded).
    pub fn versions(&self) -> Result<Vec<(String, Option<Vec<u8>>)>> {
        let mut st = self.conn.prepare("SELECT id, vv FROM pages")?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Replaces a page's open conflicts: (record id, anchor block, conflict
    /// JSON, scope).
    pub fn set_conflicts(
        &self,
        page: &str,
        items: &[(String, String, String, &str)],
    ) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM conflicts WHERE page = ?1")?
            .execute([page])?;
        let mut st = self.conn.prepare_cached(
            "INSERT INTO conflicts(page, id, anchor, json, scope) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (id, anchor, j, s) in items {
            st.execute(params![page, id, anchor, j, s])?;
        }
        Ok(())
    }

    /// Open conflicts (page, conflict JSON) of one page or of all pages,
    /// by page, anchor block and record id.
    pub fn conflicts(&self, page: Option<&str>) -> Result<Vec<(String, String)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT page, json FROM conflicts WHERE ?1 IS NULL OR page = ?1 ORDER BY page, anchor, id",
        )?;
        let v = st
            .query_map([page], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    // ------------------------------------------------------------ attachments

    pub fn put_attachment(&self, a: &Attachment, created: i64) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO attachments(id, name, mime, size, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO NOTHING",
            )?
            .execute(params![a.id, a.name, a.mime, a.size, created])?;
        Ok(())
    }

    pub fn attachment(&self, id: &str) -> Result<Option<Attachment>> {
        Ok(self
            .conn
            .prepare_cached("SELECT id, name, mime, size FROM attachments WHERE id = ?1")?
            .query_row([id], |r| {
                Ok(Attachment {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    mime: r.get(2)?,
                    size: r.get(3)?,
                })
            })
            .optional()?)
    }

    pub fn referenced_blobs(&self) -> Result<std::collections::HashSet<String>> {
        let mut st = self.conn.prepare("SELECT DISTINCT blob FROM refs")?;
        let v = st
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<std::collections::HashSet<String>>>()?;
        Ok(v)
    }

    pub fn delete_attachment(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM attachments WHERE id = ?1")?
            .execute([id])?;
        Ok(())
    }

    pub fn all_attachment_ids(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT id FROM attachments")?;
        let v = st
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// Every live page's (id, md path, md hash), for rescan.
    pub fn md_files(&self) -> Result<Vec<MdFile>> {
        let mut st = self
            .conn
            .prepare("SELECT id, md_path, md_hash FROM pages WHERE in_trash = 0")?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Records the page's markdown file, the hash of its content and the
    /// document version it matches.
    /// A write transaction that takes the write lock up front (`BEGIN
    /// IMMEDIATE`). A deferred one that reads first fails at once with
    /// SQLITE_BUSY under WAL when the other connection committed in between,
    /// instead of waiting out the busy timeout.
    pub fn write_tx(&self) -> rusqlite::Result<rusqlite::Transaction<'_>> {
        rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Immediate)
    }

    pub fn set_md(&self, id: &str, name: &str, hash: &str, version: Option<&[u8]>) -> Result<()> {
        self.conn
            .prepare_cached(
                "UPDATE pages SET md_path = ?2, md_hash = ?3, md_version = ?4 WHERE id = ?1",
            )?
            .execute(params![id, name, hash, version])?;
        Ok(())
    }

    /// The document version the page's markdown file was written from.
    pub fn md_version(&self, id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .prepare_cached("SELECT md_version FROM pages WHERE id = ?1")?
            .query_row([id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    /// Every recorded markdown file, trashed pages too: page id, file,
    /// hash and version.
    pub fn md_rows(&self) -> Result<Vec<MdRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, md_path, md_hash, md_version FROM pages WHERE md_path IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn clear_md(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(
                "UPDATE pages SET md_path = NULL, md_hash = NULL, md_version = NULL WHERE id = ?1",
            )?
            .execute([id])?;
        Ok(())
    }

    /// Child ids (live or trashed) of a page.
    pub fn child_ids(&self, id: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id FROM pages WHERE parent = ?1 ORDER BY sort_key, created_ms, id",
        )?;
        let v = st
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }
}

fn probe_text_matches(query: &str, words: &[String], title: String, body: &str) -> bool {
    let mut text_words = fuzzy::query_words(&title);
    let titles = [fuzzy::TitleEntry::new(String::new(), title, 0)];
    // Missing-word flags depend on the whole corpus. Allow every word to use
    // edits here: this can wake an extra query but cannot hide a fuzzy title.
    if !fuzzy::title_hits_scoped(
        &titles, query, words, &HashSet::new(), 1, true, None, false,
    ).is_empty() {
        return true;
    }
    text_words.extend(fuzzy::query_words(body));
    text_words.sort_unstable();
    text_words.dedup();
    // Body corrections match title OR body for each word. A page-local word
    // index deliberately ignores the global top-eight and missing-word gates:
    // any globally selected correction on this page also has a local match.
    // FTS unicode61 folds more Unicode than fuzzy::plain; retain uncertain
    // non-ASCII bodies rather than introducing tokenizer false negatives.
    if !body.is_ascii() || !titles[0].title.is_ascii() {
        return true;
    }
    let vocabulary = fuzzy::WordIndex::build(text_words.into_iter().map(|word| (word, 1)).collect());
    let mut scratch = Scratch::default();
    words.iter().enumerate().all(|(i, word)| {
        if vocabulary.words.iter().any(|(term, _)| term.starts_with(word)) {
            return true;
        }
        let chars: Vec<_> = word.chars().collect();
        if i + 1 == words.len() {
            !vocabulary.correct_prefix(&chars, &mut scratch).is_empty()
        } else {
            !vocabulary.correct(&chars, &mut scratch).is_empty()
        }
    })
}

fn spec_fts_query(spec: &SearchSpec) -> Option<String> {
    let mut parts = Vec::new();
    for term in spec.terms.iter().filter(|t| !t.exclude) {
        let words = fuzzy::query_words(&term.value);
        if words.is_empty() {
            continue;
        }
        if term.phrase {
            parts.push(format!("\"{}\"", words.join(" ")));
        } else {
            // Keep the existing per-word prefix matching while typing more
            // words. Quoted phrases remain exact hard constraints.
            parts.extend(words.iter().map(|word| format!("\"{word}\"*")));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn search_array(candidates: Option<&Candidates>) -> Option<Rc<Vec<Value>>> {
    candidates
        .filter(|c| c.small())
        .map(|c| Rc::new(c.array.clone()))
}

fn search_restriction(candidates: Option<&Candidates>, column: &str, parameter: usize) -> String {
    match candidates {
        Some(c) if c.small() => c.clause(column, parameter),
        Some(c) => format!("{} AND ?{parameter} IS NULL", c.clause(column, parameter)),
        None => format!(" AND ?{parameter} IS NULL"),
    }
}

fn search_hit(r: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        id: r.get(0)?,
        title: r.get(1)?,
        snippet: r.get(2)?,
        fuzzy: false,
        corrected: None,
        matched_filters: Vec::new(),
        source: String::new(),
        page_type: String::new(),
        target: None,
    })
}

/// Title tiers precede body matches; all uncorrected results precede the
/// correction pass. Title hits arrive with their snippets (`search_title_hits`).
fn merge_titles(mut titles: Vec<SearchHit>, body: Vec<SearchHit>, limit: u32) -> Vec<SearchHit> {
    let mut seen: HashSet<String> = titles.iter().map(|h| h.id.clone()).collect();
    for hit in body {
        if titles.len() == limit as usize {
            break;
        }
        if seen.insert(hit.id.clone()) {
            titles.push(hit);
        }
    }
    titles
}

/// Turns user input into an FTS5 query: every word becomes a quoted prefix
/// term, all of them required. Returns None when there is no word.
#[cfg(test)]
pub(crate) fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

#[cfg(test)]
mod conflict_mirror_tests {
    use super::*;

    #[test]
    fn existing_vault_fixture_migrates_and_keeps_page_and_legacy_record() {
        let root = std::env::temp_dir().join(format!("conflict-vault-fixture-{}", crate::new_id()));
        let legacy = r#"{"block":"b1","time_ms":7,"sides":[{"device":"dev-a","block":null},{"device":"dev-b","block":null}]}"#;
        let page;
        {
            let vault = crate::Vault::open(root.to_string_lossy().into_owned()).unwrap();
            page = vault.create_page(None, "Existing vault".into()).unwrap();
            vault.inner.edit(&page, |doc| doc.set_conflict("b1", legacy)).unwrap();
            vault.flush().unwrap();
        }
        {
            let (index, _) = Index::open(&root.join(".vault/index.sqlite")).unwrap();
            index.conn.execute_batch(
                "DROP TABLE conflicts;
                 CREATE TABLE conflicts(page TEXT NOT NULL, block TEXT NOT NULL, json TEXT NOT NULL,
                   PRIMARY KEY(page, block)) WITHOUT ROWID;"
            ).unwrap();
            index.conn.execute("INSERT INTO conflicts(page, block, json) VALUES (?1, 'b1', ?2)",
                params![page, legacy]).unwrap();
        }
        {
            let vault = crate::Vault::open(root.to_string_lossy().into_owned()).unwrap();
            assert_eq!(vault.load_page(page.clone()).unwrap().title, "Existing vault");
            let records = vault.list_conflicts(Some(page)).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].id, "b1");
            assert_eq!(records[0].format, 1);
            assert!(!records[0].undoable());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Reads (id, anchor, scope) of a page's conflict rows.
    fn mirror_row(index: &Index, page: &str) -> Vec<(String, String, String)> {
        let mut st = index
            .conn
            .prepare("SELECT id, anchor, scope FROM conflicts WHERE page = ?1 ORDER BY id")
            .unwrap();
        let rows: Vec<(String, String, String)> = st
            .query_map([page], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows
    }

    /// A format 1 index, as written before `scope` and record ids existed:
    /// rows keyed by block, no other columns. Its rows keep their block as
    /// both id and anchor, and gain scope `block` in place.
    #[test]
    fn format_1_mirror_rows_gain_record_ids_and_scope_in_place() {
        let dir = std::env::temp_dir().join(format!("vault-index-ids-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.sqlite");
        let _ = std::fs::remove_file(&path);
        let legacy = r#"{"block":"b1","time_ms":7,"sides":[{"device":"dev-a","block":null},{"device":"dev-b","block":null}]}"#;
        {
            let (index, fresh) = Index::open(&path).unwrap();
            assert!(fresh);
            index
                .conn
                .execute_batch(
                    "DROP TABLE conflicts;
                     CREATE TABLE conflicts(
                       page TEXT NOT NULL, block TEXT NOT NULL, json TEXT NOT NULL,
                       PRIMARY KEY(page, block)
                     ) WITHOUT ROWID;",
                )
                .unwrap();
            index
                .conn
                .execute(
                    "INSERT INTO conflicts(page, block, json) VALUES ('p1', 'b1', ?1)",
                    [legacy],
                )
                .unwrap();
        }
        let (index, fresh) = Index::open(&path).unwrap();
        assert!(!fresh, "an index of this version is migrated, not rebuilt");
        assert_eq!(
            index.conflicts(None).unwrap(),
            vec![("p1".to_string(), legacy.to_string())]
        );
        assert_eq!(
            mirror_row(&index, "p1"),
            vec![("b1".to_string(), "b1".to_string(), "block".to_string())]
        );
        // Distinct record ids may share an anchor block, and the scope is kept.
        index
            .set_conflicts(
                "p2",
                &[
                    ("r1".into(), "root".into(), "{}".into(), "subtree"),
                    ("r2".into(), "root".into(), "{}".into(), "subtree"),
                ],
            )
            .unwrap();
        assert_eq!(
            mirror_row(&index, "p2"),
            vec![
                ("r1".to_string(), "root".to_string(), "subtree".to_string()),
                ("r2".to_string(), "root".to_string(), "subtree".to_string()),
            ]
        );
        assert_eq!(index.conflicts(Some("p1")).unwrap().len(), 1);
        drop(index);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scope-era index (scope column, no record ids) keeps its scope and
    /// gets each row's block as its id and anchor.
    #[test]
    fn scope_era_mirror_rows_gain_record_ids_in_place() {
        let dir = std::env::temp_dir().join(format!("vault-index-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.sqlite");
        let _ = std::fs::remove_file(&path);
        {
            let (index, _) = Index::open(&path).unwrap();
            index
                .conn
                .execute_batch(
                    "DROP TABLE conflicts;
                     CREATE TABLE conflicts(
                       page TEXT NOT NULL, block TEXT NOT NULL, json TEXT NOT NULL,
                       scope TEXT NOT NULL DEFAULT 'block',
                       PRIMARY KEY(page, block)
                     ) WITHOUT ROWID;
                     INSERT INTO conflicts(page, block, json, scope)
                       VALUES ('p1', 'b1', '{}', 'subtree');",
                )
                .unwrap();
        }
        let (index, fresh) = Index::open(&path).unwrap();
        assert!(!fresh);
        assert_eq!(
            mirror_row(&index, "p1"),
            vec![("b1".to_string(), "b1".to_string(), "subtree".to_string())]
        );
        drop(index);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn fixture(pages: u32, matching: u32) -> Index {
        fixture_at(Path::new(":memory:"), pages, matching)
    }

    #[test]
    fn commit_probe_checks_only_requested_pages_and_keeps_hard_terms() {
        let index = fixture(3, 1);
        let probe = |query: &str, ids: &[&str]| {
            index.search_spec_matches_ids(
                &crate::search_query::parse_search(query).spec,
                &ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            ).unwrap()
        };
        assert!(probe("garden", &["page-1"]));
        assert!(!probe("garden", &["page-2", "page-3"]));
        assert!(!probe("garden", &["absent"]));
        assert!(!probe("garden", &[]));
        assert!(!probe("", &["page-1"]));
        assert!(probe("\"garden gardening\"", &["page-1"]));
        assert!(!probe("\"gardening garden\"", &["page-1"]));
        assert!(!probe("garden -singularrare", &["page-1"]));
        assert!(probe("-singularrare", &["page-2"]));
        assert!(probe("ard", &["page-1"])); // title substring, not an FTS prefix
        assert!(probe("gadr", &["page-1"])); // fuzzy title
        index.conn.execute("UPDATE pages SET in_trash = 1 WHERE rowid = 1", []).unwrap();
        assert!(!probe("garden", &["page-1"]));
    }

    #[test]
    fn commit_probe_keeps_body_corrections_and_mixed_title_body_words() {
        let index = fixture(2, 1);
        index.conn.execute("UPDATE pages SET title = 'Zebra', title_fold = 'zebra' WHERE rowid = 1", []).unwrap();
        index.conn.execute("UPDATE fts SET title = 'Zebra', body = 'garden' WHERE rowid = 1", []).unwrap();
        for query in ["gardne", "zebra gardne"] {
            let spec = crate::search_query::parse_search(query).spec;
            assert!(!index.search_spec(&spec, 10, 0, false).unwrap().hits.is_empty());
            assert!(index.search_spec_matches_ids(&spec, &["page-1".into()]).unwrap());
            assert!(!index.search_spec_matches_ids(&spec, &["page-2".into()]).unwrap());
        }
    }

    #[test]
    fn commit_probe_reuses_typed_filters_subtrees_and_schema_tag_aliases() {
        let index = fixture(3, 1);
        index.conn.execute("UPDATE pages SET parent = 'page-1' WHERE rowid = 2", []).unwrap();
        let defs = vec![PropDef {
            key: "labels".into(), name: "Tags".into(), kind: crate::types::PropKind::MultiSelect,
            options: vec![crate::types::SelectOption {
                id: "option-work".into(), name: "Work/Meetings".into(), color: String::new(),
            }], config: String::new(),
        }, PropDef {
            key: "born".into(), name: "Born".into(), kind: crate::types::PropKind::CreatedTime,
            options: Vec::new(), config: String::new(),
        }];
        let schema = serde_json::to_string(&defs).unwrap();
        index.conn.execute("UPDATE pages SET schema = ?1 WHERE rowid = 1", [schema]).unwrap();
        let tags = serde_json::to_string(&PropValue::MultiSelect(vec!["option-work".into()])).unwrap();
        index.conn.execute("INSERT INTO props(page, key, json) VALUES (2, 'labels', ?1)", [tags]).unwrap();
        let context = crate::search_query::SearchContext::local(defs, vec![crate::search_query::SearchPage {
            id: "page-1".into(), title: "Garden manual 1".into(), updated_ms: 1,
        }], 0);
        let queries = ["tag:work", "-tag:work", "in:[[page-1]]", "-in:[[page-1]]", "has:born", "-has:born", "title:notes", "type:document", "created:2026"];
        for query in queries {
            let spec = crate::search_query::parse_search_with_context(query, &context).spec;
            assert!(spec.warnings.is_empty(), "{query}: {:?}", spec.warnings);
            let expected: HashSet<_> = index.search_spec(&spec, 100, 0, false).unwrap().hits.into_iter().map(|hit| hit.id).collect();
            for id in ["page-1", "page-2", "page-3"] {
                assert_eq!(index.search_spec_matches_ids(&spec, &[id.into()]).unwrap(), expected.contains(id), "{query}: {id}");
            }
        }
    }

    fn fixture_at(path: &Path, pages: u32, matching: u32) -> Index {
        let (index, _) = Index::open(path).unwrap();
        let tx = index.write_tx().unwrap();
        {
            let mut pages_st = index
                .conn
                .prepare(
                    "INSERT INTO pages(rowid,id,title,title_fold,created_ms,updated_ms)
                 VALUES (?1,?2,?3,?4,0,?1)",
                )
                .unwrap();
            let mut fts_st = index
                .conn
                .prepare("INSERT INTO fts(rowid,title,body) VALUES (?1,?2,?3)")
                .unwrap();
            for i in 1..=pages {
                let title = if i <= matching {
                    format!("Garden manual {i}")
                } else {
                    format!("Notes {i}")
                };
                let body = if i == 1 {
                    "garden gardening singularrare"
                } else if i <= matching {
                    "garden"
                } else {
                    "ordinary text"
                };
                pages_st
                    .execute(params![i, format!("page-{i}"), title, fold(&title)])
                    .unwrap();
                fts_st.execute(params![i, title, body]).unwrap();
            }
        }
        tx.commit().unwrap();
        index
    }

    #[test]
    fn adaptive_threshold_scales_with_vault_size() {
        let small = fixture(200, 200);
        assert!(!small.search_adaptive("g", 10).unwrap().narrowed);
        let large = fixture(10_000, 600);
        let results = large.search_adaptive("g", 10).unwrap();
        assert!(results.narrowed);
        assert_eq!(results.hits.len(), 10);
        assert!(results.hits.iter().all(|h| h.snippet.is_empty()));
        let larger = fixture(20_000, 600);
        assert!(!larger.search_adaptive("g", 10).unwrap().narrowed);
    }

    #[test]
    fn narrowing_flips_on_selective_prefix_and_combination() {
        let index = fixture(10_000, 600);
        assert!(index.search_adaptive("g", 10).unwrap().narrowed);
        let selective = index.search_adaptive("gardening", 10).unwrap();
        assert!(!selective.narrowed);
        assert_eq!(selective.hits[0].id, "page-1");
        assert!(selective.hits[0].snippet.contains("gardening"));
        let multiword = index.search_adaptive("singularrare g", 10).unwrap();
        assert!(!multiword.narrowed);
        assert_eq!(multiword.hits[0].id, "page-1");
        // API search always retains body matches; keystroke search fills
        // narrowed results from body matches when titles are empty.
        let only_body = index.search_adaptive("singular", 10).unwrap();
        assert_eq!(only_body.hits.len(), 1);
        index
            .conn
            .execute("UPDATE fts SET title = 'Notes'", [])
            .unwrap();
        index
            .conn
            .execute("UPDATE pages SET title = 'Notes', title_fold = 'notes'", [])
            .unwrap();
        let empty_titles = index.search_adaptive("g", 10).unwrap();
        assert!(empty_titles.narrowed);
        assert_eq!(empty_titles.hits.len(), 10);
        assert!(empty_titles.hits.iter().all(|h| h.snippet.is_empty()));
        assert_eq!(index.search_full("g", 1_000).unwrap().len(), 600);
    }

    #[test]
    fn broad_multiword_with_no_title_match_fills_from_body() {
        let index = fixture(10_000, 600);
        // Titles no longer contain either word, so only body matches remain.
        index
            .conn
            .execute("UPDATE fts SET title = 'Notes'", [])
            .unwrap();
        index
            .conn
            .execute("UPDATE pages SET title = 'Notes', title_fold = 'notes'", [])
            .unwrap();
        // "garden" and "gard" both match the 600 body pages: broad, narrowed.
        let results = index.search_adaptive("garden gard", 10).unwrap();
        assert!(results.narrowed);
        assert_eq!(results.hits.len(), 10);
        assert!(results.hits.iter().all(|h| h.snippet.is_empty()));
        let unique: std::collections::HashSet<_> = results.hits.iter().map(|h| &h.id).collect();
        assert_eq!(unique.len(), 10);
        // Caller limit is honoured exactly, including small limits.
        assert_eq!(
            index.search_adaptive("garden gard", 3).unwrap().hits.len(),
            3
        );
        assert!(index
            .search_adaptive("garden gard", 0)
            .unwrap()
            .hits
            .is_empty());
    }

    #[test]
    fn narrowed_body_fill_orders_by_recency_then_id() {
        let index = fixture(10_000, 600);
        index
            .conn
            .execute("UPDATE fts SET title = 'Notes'", [])
            .unwrap();
        index
            .conn
            .execute("UPDATE pages SET title = 'Notes', title_fold = 'notes'", [])
            .unwrap();
        // updated_ms equals the rowid, so the newest matching body pages lead.
        let ids = |q: &str, n: u32| -> Vec<String> {
            index
                .search_adaptive(q, n)
                .unwrap()
                .hits
                .into_iter()
                .map(|h| h.id)
                .collect()
        };
        assert_eq!(ids("garden gard", 3), ["page-600", "page-599", "page-598"]);
        // Ties on updated_ms fall back to id order.
        index
            .conn
            .execute(
                "UPDATE pages SET updated_ms = 1000 WHERE rowid IN (10, 20)",
                [],
            )
            .unwrap();
        assert_eq!(ids("garden gard", 2), ["page-10", "page-20"]);
    }

    #[test]
    fn narrowed_fill_puts_titles_first_and_excludes_them_from_body() {
        let index = fixture(10_000, 600);
        index
            .conn
            .execute("UPDATE fts SET title = 'Notes'", [])
            .unwrap();
        index
            .conn
            .execute("UPDATE pages SET title = 'Notes', title_fold = 'notes'", [])
            .unwrap();
        // page-5 matches by title and by body: it must appear once, first.
        index
            .conn
            .execute(
                "UPDATE fts SET title = 'Garden gardening guide' WHERE rowid = 5",
                [],
            )
            .unwrap();
        index
            .conn
            .execute(
                "UPDATE pages SET title = 'Garden gardening guide', title_fold = 'garden gardening guide' WHERE rowid = 5",
                [],
            )
            .unwrap();
        // page-2 is trashed and page-3 is a template: both must be excluded.
        index
            .conn
            .execute("UPDATE pages SET in_trash = 1 WHERE rowid = 2", [])
            .unwrap();
        index
            .conn
            .execute("UPDATE pages SET is_template = 1 WHERE rowid = 3", [])
            .unwrap();
        let results = index.search_adaptive("garden gard", 10).unwrap();
        assert!(results.narrowed);
        assert_eq!(results.hits.len(), 10);
        assert_eq!(results.hits[0].id, "page-5");
        let ids: Vec<&str> = results.hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids.iter().filter(|id| **id == "page-5").count(), 1);
        assert!(!ids.contains(&"page-2"));
        assert!(!ids.contains(&"page-3"));
        assert!(results.hits.iter().all(|h| h.snippet.is_empty()));
    }

    #[test]
    fn exact_title_is_first_on_both_paths() {
        for pages in [200, 10_000] {
            let index = fixture(pages, pages.min(600));
            index.conn.execute(
                "UPDATE pages SET title = 'g', title_fold = 'g', updated_ms = 0 WHERE rowid = 1", [],
            ).unwrap();
            index
                .conn
                .execute("UPDATE fts SET title = 'g' WHERE rowid = 1", [])
                .unwrap();
            let results = index.search_adaptive("g", 10).unwrap();
            assert_eq!(results.narrowed, pages > 200);
            assert_eq!(results.hits[0].id, "page-1");
            assert_eq!(index.search_full("g", 10).unwrap()[0].id, "page-1");
        }
    }

    #[test]
    fn breadth_page_count_cache_observes_local_and_writer_changes() {
        let path =
            std::env::temp_dir().join(format!("notes-count-cache-{}.sqlite", crate::new_id()));
        let index = fixture_at(&path, 201, 201);
        assert!(index.search_adaptive("g", 10).unwrap().narrowed);
        index
            .conn
            .execute("UPDATE pages SET in_trash = 1 WHERE rowid = 1", [])
            .unwrap();
        assert!(!index.search_adaptive("g", 10).unwrap().narrowed);
        let writer = Connection::open(&path).unwrap();
        writer
            .execute("UPDATE pages SET in_trash = 0 WHERE rowid = 1", [])
            .unwrap();
        assert!(index.search_adaptive("g", 10).unwrap().narrowed);
        drop(writer);
        drop(index);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn filtered_caches_do_not_retain_rolled_back_properties_or_subtrees() {
        let index = fixture(3, 0);
        let operator = |key: &str, value: &str| SearchSpec {
            operators: vec![crate::search_query::SearchOperator {
                key: key.into(),
                value: value.into(),
                negated: false,
                property: false,
                resolved_id: None,
                filter: None,
                date_future: false,
            }],
            ..Default::default()
        };
        let ids = |spec: &SearchSpec| -> HashSet<String> {
            index
                .search_spec(spec, 50, 0, false)
                .unwrap()
                .hits
                .into_iter()
                .map(|hit| hit.id)
                .collect()
        };
        let expected = |values: &[&str]| -> HashSet<String> {
            values.iter().map(|id| id.to_string()).collect()
        };
        let json = serde_json::to_string(&PropValue::MultiSelect(vec!["work".into()])).unwrap();
        index
            .conn
            .execute(
                "INSERT INTO props(page,key,json) VALUES (1,'tags',?1)",
                [json],
            )
            .unwrap();
        let tag = operator("tag", "work");
        assert_eq!(ids(&tag), expected(&["page-1"]));
        {
            let tx = index.write_tx().unwrap();
            tx.execute("DELETE FROM props WHERE page = 1", []).unwrap();
            assert!(ids(&tag).is_empty());
            tx.rollback().unwrap();
        }
        assert_eq!(ids(&tag), expected(&["page-1"]));

        let tree = operator("in", "[[page-1]]");
        assert_eq!(ids(&tree), expected(&["page-1"]));
        {
            let tx = index.write_tx().unwrap();
            tx.execute("UPDATE pages SET parent = 'page-1' WHERE rowid = 2", [])
                .unwrap();
            assert_eq!(ids(&tree), expected(&["page-1", "page-2"]));
            tx.rollback().unwrap();
        }
        {
            // The second uncommitted tree change reaches the same epoch as
            // the first one, but describes a different subtree.
            let tx = index.write_tx().unwrap();
            tx.execute("UPDATE pages SET parent = 'page-1' WHERE rowid = 3", [])
                .unwrap();
            assert_eq!(ids(&tree), expected(&["page-1", "page-3"]));
            tx.rollback().unwrap();
        }
        assert_eq!(ids(&tree), expected(&["page-1"]));
    }

    #[test]
    fn filtered_schema_cache_does_not_retain_rolled_back_option_names() {
        let index = fixture(2, 0);
        let schema = |name: &str| {
            serde_json::to_string(&vec![PropDef {
                key: "opaque-tags".into(),
                name: "Tags".into(),
                kind: crate::types::PropKind::MultiSelect,
                options: vec![crate::types::SelectOption {
                    id: "opaque-option".into(),
                    name: name.into(),
                    color: String::new(),
                }],
                config: String::new(),
            }])
            .unwrap()
        };
        index
            .conn
            .execute(
                "UPDATE pages SET schema = ?1 WHERE rowid = 1",
                [schema("Work")],
            )
            .unwrap();
        index
            .conn
            .execute(
                "INSERT INTO props(page, key, json) VALUES (2, 'opaque-tags', ?1)",
                [
                    serde_json::to_string(&PropValue::MultiSelect(vec!["opaque-option".into()]))
                        .unwrap(),
                ],
            )
            .unwrap();
        let search = |value: &str| {
            index
                .search_spec(
                    &SearchSpec {
                        operators: vec![crate::search_query::SearchOperator {
                            key: "tag".into(),
                            value: value.into(),
                            negated: false,
                            property: false,
                            resolved_id: None,
                            filter: None,
                            date_future: false,
                        }],
                        ..Default::default()
                    },
                    50,
                    0,
                    false,
                )
                .unwrap()
        };
        assert_eq!(search("work").hits[0].matched_filters[0].value, "Work");
        {
            let tx = index.write_tx().unwrap();
            tx.execute(
                "UPDATE pages SET schema = ?1 WHERE rowid = 1",
                [schema("Home")],
            )
            .unwrap();
            assert!(search("work").hits.is_empty());
            assert_eq!(search("home").hits[0].matched_filters[0].value, "Home");
            tx.rollback().unwrap();
        }
        assert!(search("home").hits.is_empty());
        assert_eq!(search("work").hits[0].matched_filters[0].value, "Work");
    }

    #[test]
    fn estimator_rewrite_preserves_prefix_document_sets_and_diacritics() {
        let index = fixture(10_000, 600);
        let body = format!(
            "garden gardening singularrare Über café {}",
            (0..17)
                .map(|i| format!("prefixword{i}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        index
            .conn
            .execute("UPDATE fts SET body=?1 WHERE rowid=1", [body])
            .unwrap();
        for query in [
            "g",
            "gard",
            "garden",
            "gardening",
            "garden man",
            "garde sing",
            "gar manual",
            "übe",
            "über",
            "café",
        ] {
            let q = fts_query(query).unwrap();
            let rewrite = index.search_count_query(&q).unwrap().unwrap();
            let count = |query: &str| -> u32 {
                index
                    .conn
                    .query_row(
                        "SELECT count(*) FROM (SELECT rowid FROM fts WHERE fts MATCH ?1 LIMIT 501)",
                        [query],
                        |r| r.get(0),
                    )
                    .unwrap()
            };
            assert_eq!(count(&q), count(&rewrite), "{query}: {rewrite}");
        }
        assert!(index
            .search_count_query("\"garden\"*")
            .unwrap()
            .unwrap()
            .contains("\"gardening\""));
        assert!(index
            .search_count_query("\"prefixword\"*")
            .unwrap()
            .is_none());
        assert!(index.search_adaptive("prefixword", 10).unwrap().narrowed);
        assert_eq!(index.search_full("prefixword", 10).unwrap().len(), 1);
        assert!(
            !index
                .search_adaptive("singularrare prefixword", 10)
                .unwrap()
                .narrowed
        );
    }

    #[test]
    fn multiword_membership_rewrite_retains_complete_sets_and_overflow_prefixes() {
        let index = fixture(64, 0);
        let body = format!(
            "coffee coffeehouse morning café Über {} {}",
            (0..16)
                .map(|i| format!("boundary{i}"))
                .collect::<Vec<_>>()
                .join(" "),
            (0..17)
                .map(|i| format!("overflow{i}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        for (row, body) in [
            (1, body.as_str()),
            (2, "coffeepot morning cafe overflow16"),
            (3, "coffeehouse morning boundary15"),
            (4, "morning unrelated"),
            (5, "overflow16 unrelated"),
        ] {
            index
                .conn
                .execute("UPDATE fts SET body=?1 WHERE rowid=?2", params![body, row])
                .unwrap();
        }
        let ids = |q: &str| -> Vec<i64> {
            index
                .conn
                .prepare("SELECT rowid FROM fts WHERE fts MATCH ?1 ORDER BY rowid")
                .unwrap()
                .query_map([q], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        for query in [
            "coffee mor",
            "café mor",
            "Über mor",
            "boundary mor",
            "overflow mor",
            "absent mor",
        ] {
            let q = fts_query(query).unwrap();
            let (_, exact) = index.search_prefix_queries(&q).unwrap();
            let exact = exact.unwrap();
            assert_eq!(ids(&q), ids(&exact), "{query}: {exact}");
            let original_fill = index.search_body_fill(&q, 10, &[], None).unwrap();
            let rewritten_fill = index.search_body_fill(&exact, 10, &[], None).unwrap();
            assert_eq!(
                serde_json::to_value(original_fill).unwrap(),
                serde_json::to_value(rewritten_fill).unwrap()
            );
            assert_eq!(
                index.search_matching_ids(&q, query, None, 201).unwrap(),
                index.search_matching_ids(&exact, query, None, 201).unwrap()
            );
        }
        let (_, at_cap) = index
            .search_prefix_queries("\"boundary\"* \"mor\"*")
            .unwrap();
        assert!(!at_cap.unwrap().contains("\"boundary\"*"));
        let (_, overflow) = index
            .search_prefix_queries("\"overflow\"* \"mor\"*")
            .unwrap();
        assert!(overflow.unwrap().contains("\"overflow\"*"));
        for q in [
            "\"coffee morning\"",
            "\"coffee-morning\"*",
            "\"coffee\"* OR \"mor\"*",
            "\"coffee\"* NOT \"mor\"*",
            "\"coffee\"*",
        ] {
            assert!(index.search_prefix_queries(q).unwrap().1.is_none(), "{q}");
        }
    }
}
