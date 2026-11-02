//! Ordered index migrations.
//!
//! The index is a cache of the vault documents. A change to it is one of:
//!
//! * **Additive** (a new column, table or index, with or without data to fill
//!   in): add a [`Step`] to [`STEPS`] with the next number. It runs in place
//!   when the vault opens, in milliseconds, in its own transaction. If rows
//!   need a value, give the step a backfill: it runs on a background thread in
//!   small batches, its cursor is kept in the index (`meta`, `backfill:<n>`) so
//!   an exit mid-way is continued at the next open, and readers treat a row
//!   that is not filled in yet as having no value.
//! * **Breaking** (old rows are wrong and no backfill can fix them): bump
//!   `SCHEMA_VERSION` in `index.rs`. The index is then rebuilt from the
//!   documents, in batches that survive an exit (`Vault::reindex`), behind the
//!   "Updating search index" boot screen.
//!
//! Never bump the version for an additive change.

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;

/// `meta` key of the number of the last step applied.
const STEP_KEY: &str = "schema_step";

/// Rows a backfill batch handles at most.
pub(crate) const BACKFILL_BATCH: usize = 500;

/// One batch of a backfill: handles up to `limit` rows after `cursor`, inside
/// the transaction the caller opened, and returns the new cursor, or `None`
/// when no rows are left.
pub(crate) type BackfillFn = fn(&Connection, i64, usize) -> Result<Option<i64>>;

pub(crate) struct Step {
    /// Position in the list: ascending, never reused or reordered.
    pub n: i64,
    #[allow(dead_code)]
    pub name: &'static str,
    /// Idempotent where it can be: an index rebuilt from scratch runs every
    /// step again over a schema that already has the change.
    pub apply: fn(&Connection) -> Result<()>,
    pub backfill: Option<BackfillFn>,
}

/// The steps since the current breaking generation, oldest first.
pub(crate) const STEPS: &[Step] = &[
    Step { n: 1, name: "external search rows", apply: external_search_rows, backfill: None },
    Step { n: 2, name: "conflict mirror scope", apply: conflict_scope, backfill: None },
    Step { n: 3, name: "conflict mirror record ids", apply: conflict_ids, backfill: None },
];

pub(crate) fn external_search_rows(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS search_external(
           rowid INTEGER PRIMARY KEY,
           source TEXT NOT NULL, id TEXT NOT NULL,
           title TEXT NOT NULL, body TEXT NOT NULL,
           page_type TEXT NOT NULL, target TEXT,
           updated_ms INTEGER NOT NULL,
           UNIQUE(source, id)
         );",
    )?;
    Ok(())
}

/// The conflict mirror gains its `scope` column. Existing rows were all
/// format 1 (one block), which the default describes.
fn conflict_scope(conn: &Connection) -> Result<()> {
    if !has_column(conn, "conflicts", "scope")? {
        conn.execute_batch("ALTER TABLE conflicts ADD COLUMN scope TEXT NOT NULL DEFAULT 'block';")?;
    }
    Ok(())
}

/// Rows were keyed by block. A format 1 record's id is its block, so each row
/// keeps its key as both id and anchor.
fn conflict_ids(conn: &Connection) -> Result<()> {
    if !has_column(conn, "conflicts", "id")? {
        conn.execute_batch(
            "CREATE TABLE conflicts_v2(
               page TEXT NOT NULL, id TEXT NOT NULL, anchor TEXT NOT NULL,
               json TEXT NOT NULL, scope TEXT NOT NULL DEFAULT 'block',
               PRIMARY KEY(page, id)
             ) WITHOUT ROWID;
             INSERT INTO conflicts_v2(page, id, anchor, json, scope)
               SELECT page, block, block, json, scope FROM conflicts;
             DROP TABLE conflicts;
             ALTER TABLE conflicts_v2 RENAME TO conflicts;",
        )?;
    }
    Ok(())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info(?1) WHERE name = ?2",
        [table, column],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

fn put(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

fn backfill_key(n: i64) -> String {
    format!("backfill:{n}")
}

/// Applies the steps the index has not seen, each with its marker in one
/// transaction (a stop in the middle leaves the step undone, not half done).
pub(crate) fn apply(conn: &Connection, steps: &[Step]) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);")?;
    let done: i64 = get(conn, STEP_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    for step in steps.iter().filter(|s| s.n > done) {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            (step.apply)(conn)?;
            if step.backfill.is_some() && get(conn, &backfill_key(step.n))?.is_none() {
                put(conn, &backfill_key(step.n), "0")?;
            }
            put(conn, STEP_KEY, &step.n.to_string())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Steps whose backfill has not finished.
pub(crate) fn pending(conn: &Connection, steps: &'static [Step]) -> Result<Vec<&'static Step>> {
    let mut out = Vec::new();
    for step in steps.iter().filter(|s| s.backfill.is_some()) {
        if get(conn, &backfill_key(step.n))?.as_deref() != Some("done") {
            out.push(step);
        }
    }
    Ok(out)
}

/// Runs one batch of the step's backfill in its own small transaction.
/// Returns true when the backfill is finished.
pub(crate) fn backfill_batch(conn: &Connection, step: &Step, limit: usize) -> Result<bool> {
    let Some(run) = step.backfill else { return Ok(true) };
    let key = backfill_key(step.n);
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<bool> {
        // Read under the write lock: another process may have moved it.
        let cursor: i64 = match get(conn, &key)?.as_deref() {
            Some("done") => return Ok(true),
            Some(v) => v.parse().unwrap_or(0),
            None => 0,
        };
        let next = run(conn, cursor, limit)?;
        put(conn, &key, &next.map_or("done".to_string(), |c| c.to_string()))?;
        Ok(next.is_none())
    })();
    match result {
        Ok(done) => {
            conn.execute_batch("COMMIT")?;
            Ok(done)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use std::path::PathBuf;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("migrate_{tag}_{}", crate::ids::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("index.sqlite")
    }

    fn add_flag(conn: &Connection) -> Result<()> {
        conn.execute_batch("ALTER TABLE pages ADD COLUMN flag INTEGER;")?;
        Ok(())
    }

    /// Sets `flag` on the next `limit` pages after the rowid `cursor`.
    fn fill_flag(conn: &Connection, cursor: i64, limit: usize) -> Result<Option<i64>> {
        let rows: Vec<i64> = conn
            .prepare("SELECT rowid FROM pages WHERE rowid > ?1 ORDER BY rowid LIMIT ?2")?
            .query_map([cursor, limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for r in &rows {
            conn.execute("UPDATE pages SET flag = 1 WHERE rowid = ?1", [r])?;
        }
        Ok(if rows.len() < limit { None } else { rows.last().copied() })
    }

    static NEW_STEPS: &[Step] = &[
        Step { n: 1, name: "a", apply: |_| Ok(()), backfill: None },
        Step { n: 2, name: "flag", apply: add_flag, backfill: Some(fill_flag) },
    ];

    fn insert_pages(conn: &Connection, n: usize) {
        for i in 0..n {
            conn.execute(
                "INSERT INTO pages(id, title, created_ms, updated_ms) VALUES (?1, 't', 0, 0)",
                [format!("p{i}")],
            )
            .unwrap();
        }
    }

    /// A bare index: the tables the test steps touch.
    fn raw(path: &PathBuf) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pages(rowid INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
               title TEXT NOT NULL, created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);",
        )
        .unwrap();
        conn
    }

    fn flagged(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM pages WHERE flag = 1", [], |r| r.get(0)).unwrap()
    }

    /// An index of the current generation that predates a step gains it in
    /// place: the rows stay, nothing is rebuilt.
    #[test]
    fn an_additive_step_runs_in_place_on_an_older_index() {
        let path = temp("additive");
        {
            let conn = raw(&path);
            insert_pages(&conn, 3);
            // The index as of an earlier build: only the first step is known.
            apply(&conn, &NEW_STEPS[..1]).unwrap();
        }
        let conn = raw(&path);
        apply(&conn, NEW_STEPS).unwrap();
        assert_eq!(get(&conn, STEP_KEY).unwrap().as_deref(), Some("2"));
        assert_eq!(flagged(&conn), 0, "a step's data is filled by its backfill, not by the step");
        let n: i64 = conn.query_row("SELECT count(*) FROM pages", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3, "rows are kept");
        // Applying again does nothing.
        apply(&conn, NEW_STEPS).unwrap();
    }

    /// The real steps bring an index of the current generation from before
    /// they existed (no marker, an old conflict mirror) up to date.
    #[test]
    fn the_listed_steps_upgrade_an_index_without_a_marker() {
        let path = temp("legacy");
        {
            let (index, _) = Index::open(&path).unwrap();
            index.set_reindex_pending(false).unwrap();
            insert_pages(&index.conn, 2);
            index
                .conn
                .execute_batch(
                    "DELETE FROM meta WHERE key = 'schema_step';
                     DROP TABLE conflicts;
                     CREATE TABLE conflicts(page TEXT NOT NULL, block TEXT NOT NULL, json TEXT NOT NULL,
                       PRIMARY KEY(page, block)) WITHOUT ROWID;
                     INSERT INTO conflicts VALUES ('p0', 'b1', '{}');",
                )
                .unwrap();
        }
        let (index, fresh) = Index::open(&path).unwrap();
        assert!(!fresh, "no rebuild");
        let row: (String, String, String) = index
            .conn
            .query_row("SELECT id, anchor, scope FROM conflicts", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!(row, ("b1".into(), "b1".into(), "block".into()));
        assert_eq!(get(&index.conn, STEP_KEY).unwrap().as_deref(), Some("3"));
    }

    /// A backfill that stops after some batches continues from its cursor.
    #[test]
    fn an_interrupted_backfill_resumes_from_its_cursor() {
        let path = temp("backfill");
        {
            let conn = raw(&path);
            insert_pages(&conn, 10);
            apply(&conn, NEW_STEPS).unwrap();
            assert_eq!(pending(&conn, NEW_STEPS).unwrap().len(), 1);
            // Two batches of three, then the app exits.
            assert!(!backfill_batch(&conn, &NEW_STEPS[1], 3).unwrap());
            assert!(!backfill_batch(&conn, &NEW_STEPS[1], 3).unwrap());
            assert_eq!(flagged(&conn), 6);
        }
        let conn = raw(&path);
        apply(&conn, NEW_STEPS).unwrap();
        let pend = pending(&conn, NEW_STEPS).unwrap();
        assert_eq!(pend.len(), 1, "the backfill is still pending after the restart");
        assert_eq!(get(&conn, "backfill:2").unwrap().as_deref(), Some("6"), "the cursor was kept");
        let mut batches = 0;
        while !backfill_batch(&conn, pend[0], 3).unwrap() {
            batches += 1;
        }
        assert_eq!(batches, 1, "only the remaining rows are handled");
        assert_eq!(flagged(&conn), 10);
        assert!(pending(&conn, NEW_STEPS).unwrap().is_empty());
    }
}
