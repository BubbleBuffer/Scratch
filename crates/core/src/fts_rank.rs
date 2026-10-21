//! FTS5 auxiliary functions `search_edits` and `search_correction` for
//! corrected search.
//!
//! The caller sends one MATCH query per keystroke: an AND of query-word
//! groups, where each group ORs its phrases, for example
//! `(quikc* OR quick OR quack) AND (fox)`. The JSON phrase list must follow
//! the MATCH query's phrase order exactly. FTS5 numbers phrases from left to
//! right across the whole expression, and entry `i` of the list describes
//! phrase `i`.
//!
//! For each row, both functions pick the cheapest phrase per group that occurs
//! in that row. Ties go to the earlier query position.
//! - `search_edits` returns the sum of the picked costs, as an integer.
//! - `search_correction` returns the picked display words joined by single
//!   spaces, in ascending group order.
//!
//! A row for which some group has no occurring phrase gets NULL. MATCH never
//! returns such a row, so this only shows up if the list and query disagree.
//!
//! Usage: call [`register`] once per connection. The first argument is the
//! FTS5 table name, and the query must MATCH on that table:
//!
//! ```sql
//! SELECT rowid, search_edits(notes, ?1), search_correction(notes, ?1)
//! FROM notes WHERE notes MATCH ?2
//! ```
//!
//! Limits: the functions exist only in queries on an FTS5 table with a MATCH.
//! The JSON phrase count must equal the MATCH phrase count, or the call fails.
//! Only the native `libsqlite3-sys` ffi is used. The wasm ffi is not covered.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::panic::{self, AssertUnwindSafe};
use std::ptr;

use rusqlite::{ffi, Connection};
use serde::{Deserialize, Serialize};

/// One entry of the flattened phrase list. Entry `i` describes FTS5 phrase `i`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Phrase {
    /// Query-word group. Groups are ANDed, and phrases inside a group are ORed.
    pub(crate) group: u32,
    /// Edit cost of this candidate. A typed prefix costs 0.
    pub(crate) edits: u32,
    /// Plain word for display. Not an FTS expression.
    pub(crate) term: String,
}

/// Decoded phrase list, stored as FTS5 auxdata for the running query.
struct Decoded {
    /// The JSON it was decoded from. A different JSON argument re-decodes.
    json: Vec<u8>,
    phrases: Vec<Phrase>,
    /// One entry per group, in ascending group number. Each holds phrase
    /// indexes ordered by (edits, query position), so the first one that
    /// occurs in a row is the pick.
    groups: Vec<Vec<usize>>,
}

impl Decoded {
    fn parse(json: &[u8]) -> Result<Decoded, String> {
        let phrases: Vec<Phrase> = serde_json::from_slice(json)
            .map_err(|e| format!("search: invalid phrase JSON: {e}"))?;
        let mut by_group: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (index, phrase) in phrases.iter().enumerate() {
            by_group.entry(phrase.group).or_default().push(index);
        }
        let groups: Vec<Vec<usize>> = by_group
            .into_values()
            .map(|mut indexes| {
                // Stable sort: equal costs keep query order.
                indexes.sort_by_key(|&index| phrases[index].edits);
                indexes
            })
            .collect();
        Ok(Decoded {
            json: json.to_vec(),
            phrases,
            groups,
        })
    }
}

/// FTS5 calls this when the cached value is replaced or its query ends.
unsafe extern "C" fn drop_decoded(value: *mut c_void) {
    if !value.is_null() {
        drop(Box::from_raw(value as *mut Decoded));
    }
}

#[derive(Clone, Copy)]
enum Output {
    Edits,
    Correction,
}

enum Answer {
    Null,
    Int(i64),
    Text(String),
}

unsafe extern "C" fn search_edits_cb(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    ctx: *mut ffi::sqlite3_context,
    n_val: c_int,
    args: *mut *mut ffi::sqlite3_value,
) {
    answer(api, fts, ctx, n_val, args, Output::Edits);
}

unsafe extern "C" fn search_correction_cb(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    ctx: *mut ffi::sqlite3_context,
    n_val: c_int,
    args: *mut *mut ffi::sqlite3_value,
) {
    answer(api, fts, ctx, n_val, args, Output::Correction);
}

/// Computes one row's answer and passes it to SQLite. A panic is turned into
/// an SQL error here, so no unwinding crosses the C callback boundary.
unsafe fn answer(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    ctx: *mut ffi::sqlite3_context,
    n_val: c_int,
    args: *mut *mut ffi::sqlite3_value,
    output: Output,
) {
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        evaluate(api, fts, n_val, args, output)
    }));
    match outcome {
        Ok(Ok(Answer::Null)) => ffi::sqlite3_result_null(ctx),
        Ok(Ok(Answer::Int(value))) => ffi::sqlite3_result_int64(ctx, value),
        Ok(Ok(Answer::Text(text))) => match c_int::try_from(text.len()) {
            // SQLITE_TRANSIENT: SQLite copies the bytes before `text` is dropped.
            Ok(len) => ffi::sqlite3_result_text(
                ctx,
                text.as_ptr() as *const c_char,
                len,
                ffi::SQLITE_TRANSIENT(),
            ),
            Err(_) => set_error(ctx, "search: correction is too long"),
        },
        Ok(Err(message)) => set_error(ctx, &message),
        Err(_) => set_error(ctx, "search: internal error"),
    }
}

/// Sets an SQL error result. SQLite copies the message.
unsafe fn set_error(ctx: *mut ffi::sqlite3_context, message: &str) {
    let len = c_int::try_from(message.len()).unwrap_or(c_int::MAX);
    ffi::sqlite3_result_error(ctx, message.as_ptr() as *const c_char, len);
}

/// Works out this row's answer. It uses only FTS5's position lookups, which
/// read this row's index entries. It never reads the body or runs another query.
///
/// # Safety
/// Must be called from an FTS5 auxiliary callback, with the pointers SQLite
/// passed to that callback.
unsafe fn evaluate(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    n_val: c_int,
    args: *mut *mut ffi::sqlite3_value,
    output: Output,
) -> Result<Answer, String> {
    let api = api.as_ref().ok_or("search: no FTS5 API")?;
    if api.iVersion < 2 {
        return Err("search: FTS5 API version 2 or later is required".to_string());
    }
    // FTS5 has already dropped the table-name argument, so this is the JSON only.
    if n_val != 1 || args.is_null() {
        return Err("search: expected one JSON argument".to_string());
    }
    let arg = *args;
    if arg.is_null() {
        return Err("search: missing JSON argument".to_string());
    }
    if ffi::sqlite3_value_type(arg) == ffi::SQLITE_NULL {
        return Ok(Answer::Null);
    }

    let json = value_bytes(arg)?;
    let decoded = decoded_for(api, fts, json)?;
    let matched = phrase_count(api, fts)?;
    if matched != decoded.phrases.len() {
        return Err(format!(
            "search: JSON lists {} phrases but the MATCH query has {matched}",
            decoded.phrases.len()
        ));
    }

    let mut picks: Vec<usize> = Vec::with_capacity(decoded.groups.len());
    for group in &decoded.groups {
        match first_match(api, fts, group)? {
            Some(phrase) => picks.push(phrase),
            // Every group must occur for MATCH to return the row, so a miss
            // means there is no answer for it.
            None => return Ok(Answer::Null),
        }
    }

    Ok(match output {
        Output::Edits => Answer::Int(
            picks
                .iter()
                .map(|&phrase| i64::from(decoded.phrases[phrase].edits))
                .sum(),
        ),
        Output::Correction => Answer::Text(
            picks
                .iter()
                .map(|&phrase| decoded.phrases[phrase].term.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
    })
}

/// Returns the decoded list for this query. It parses and stores the list on
/// first use, or when the JSON argument changes. FTS5 keeps the stored value
/// until this function's slot is replaced or the query's cursor is reset. The
/// returned reference is used only inside the current callback, so it is safe.
///
/// # Safety
/// `fts` must be the context from the current callback.
unsafe fn decoded_for<'a>(
    api: &ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    json: &[u8],
) -> Result<&'a Decoded, String> {
    let get = api
        .xGetAuxdata
        .ok_or("search: FTS5 API lacks xGetAuxdata")?;
    let cached = get(fts, 0) as *const Decoded;
    if let Some(cached) = cached.as_ref() {
        if cached.json.as_slice() == json {
            return Ok(cached);
        }
    }

    let set = api
        .xSetAuxdata
        .ok_or("search: FTS5 API lacks xSetAuxdata")?;
    let decoded = Box::into_raw(Box::new(Decoded::parse(json)?));
    let rc = set(fts, decoded as *mut c_void, Some(drop_decoded));
    if rc != ffi::SQLITE_OK {
        // On failure FTS5 has already called drop_decoded on this pointer. In
        // the bundled source, fts5ApiSetAuxdata does this when it cannot
        // allocate its record. So the pointer must not be used or freed here.
        return Err(format!(
            "search: could not cache the phrase list (code {rc})"
        ));
    }
    Ok(&*decoded)
}

/// Copies nothing. The slice is valid only during the current callback.
///
/// # Safety
/// `value` must be an argument value passed to the current callback.
unsafe fn value_bytes<'a>(value: *mut ffi::sqlite3_value) -> Result<&'a [u8], String> {
    // Read the text before the byte count, as SQLite requires.
    let text = ffi::sqlite3_value_text(value);
    if text.is_null() {
        return Err("search: could not read the JSON argument (out of memory?)".to_string());
    }
    let len = usize::try_from(ffi::sqlite3_value_bytes(value))
        .map_err(|_| "search: negative JSON length".to_string())?;
    Ok(std::slice::from_raw_parts(text, len))
}

/// The number of phrases in the current query's MATCH expression.
///
/// # Safety
/// `fts` must be the context from the current callback.
unsafe fn phrase_count(
    api: &ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
) -> Result<usize, String> {
    let count = api
        .xPhraseCount
        .ok_or("search: FTS5 API lacks xPhraseCount")?;
    usize::try_from(count(fts)).map_err(|_| "search: negative phrase count".to_string())
}

/// The first phrase in `group` (already in pick order) that occurs in the
/// current row. xPhraseFirst is used as an existence test. It stops at the
/// first position in the row, so it never walks every occurrence.
///
/// # Safety
/// `fts` must be the context from the current callback.
unsafe fn first_match(
    api: &ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    group: &[usize],
) -> Result<Option<usize>, String> {
    let first = api
        .xPhraseFirst
        .ok_or("search: FTS5 API lacks xPhraseFirst")?;
    for &phrase in group {
        let mut iter = ffi::Fts5PhraseIter {
            a: ptr::null(),
            b: ptr::null(),
        };
        let mut column: c_int = -1;
        let mut offset: c_int = -1;
        let rc = first(fts, phrase as c_int, &mut iter, &mut column, &mut offset);
        if rc != ffi::SQLITE_OK {
            return Err(format!("search: phrase lookup failed (code {rc})"));
        }
        // FTS5 sets the column to -1 when the phrase has no occurrence in this row.
        if column >= 0 {
            return Ok(Some(phrase));
        }
    }
    Ok(None)
}

/// Registers `search_edits` and `search_correction` on `conn`. Call once per
/// connection, before running queries that use them.
pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    // SAFETY: `handle` is valid while `conn` is open. Everything below runs
    // synchronously on this connection, and the API pointer is not stored.
    unsafe {
        let db = conn.handle();
        let api = fts5_api(db)?;
        create_function(db, api, b"search_edits\0", Some(search_edits_cb))?;
        create_function(db, api, b"search_correction\0", Some(search_correction_cb))?;
    }
    Ok(())
}

/// Gets the FTS5 extension API for `db`. `SELECT fts5(?1)` writes the API
/// pointer into a pointer bound with the type name `fts5_api_ptr`.
///
/// The returned pointer belongs to SQLite and is valid while `db` is open.
/// Do not keep it past that.
///
/// # Safety
/// `db` must be an open connection handle.
unsafe fn fts5_api(db: *mut ffi::sqlite3) -> rusqlite::Result<*mut ffi::fts5_api> {
    let mut stmt: *mut ffi::sqlite3_stmt = ptr::null_mut();
    let rc = ffi::sqlite3_prepare_v2(
        db,
        c"SELECT fts5(?1)".as_ptr(),
        -1,
        &mut stmt,
        ptr::null_mut(),
    );
    if rc != ffi::SQLITE_OK {
        return Err(db_error(db, rc));
    }

    let mut api: *mut ffi::fts5_api = ptr::null_mut();
    let rc = ffi::sqlite3_bind_pointer(
        stmt,
        1,
        &mut api as *mut *mut ffi::fts5_api as *mut c_void,
        c"fts5_api_ptr".as_ptr(),
        None,
    );
    if rc != ffi::SQLITE_OK {
        let err = db_error(db, rc);
        ffi::sqlite3_finalize(stmt);
        return Err(err);
    }

    // The statement returns one row, whose value is NULL. fts5() sets `api` as a side effect.
    let rc = ffi::sqlite3_step(stmt);
    let step_error = if rc != ffi::SQLITE_ROW {
        Some(db_error(db, rc))
    } else {
        None
    };
    // Finalize returns the step's error again. We already captured it above.
    ffi::sqlite3_finalize(stmt);
    if let Some(err) = step_error {
        return Err(err);
    }
    if api.is_null() {
        return Err(api_error(
            "search: FTS5 is not available on this connection",
        ));
    }
    Ok(api)
}

/// Registers one auxiliary function through the FTS5 API.
///
/// # Safety
/// `api` must come from `fts5_api(db)` for the same open connection.
unsafe fn create_function(
    db: *mut ffi::sqlite3,
    api: *mut ffi::fts5_api,
    name: &[u8],
    callback: ffi::fts5_extension_function,
) -> rusqlite::Result<()> {
    let create = (*api)
        .xCreateFunction
        .ok_or_else(|| api_error("search: FTS5 API lacks xCreateFunction"))?;
    let rc = create(
        api,
        name.as_ptr() as *const c_char,
        ptr::null_mut(),
        callback,
        None,
    );
    if rc != ffi::SQLITE_OK {
        return Err(db_error(db, rc));
    }
    Ok(())
}

/// The error SQLite last reported on `db`, with the code `rc`.
///
/// # Safety
/// `db` must be an open connection handle.
unsafe fn db_error(db: *mut ffi::sqlite3, rc: c_int) -> rusqlite::Error {
    let message = ffi::sqlite3_errmsg(db);
    let text = if message.is_null() {
        None
    } else {
        Some(CStr::from_ptr(message).to_string_lossy().into_owned())
    };
    rusqlite::Error::SqliteFailure(ffi::Error::new(rc), text)
}

fn api_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_ERROR),
        Some(message.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// (rowid, search_edits, search_correction)
    type Row = (i64, Option<i64>, Option<String>);

    fn phrase(group: u32, edits: u32, term: &str) -> Phrase {
        Phrase {
            group,
            edits,
            term: term.to_string(),
        }
    }

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE notes USING fts5(body);
             INSERT INTO notes(rowid, body) VALUES
               (1, 'the quick brown fox'),
               (2, 'a quikc fox'),
               (3, 'quack quick fox'),
               (4, 'quack fox'),
               (5, 'quick brown dog'),
               (6, 'quikc dgo');",
        )
        .unwrap();
        register(&conn).unwrap();
        conn
    }

    /// Runs both functions for one MATCH query.
    fn run(conn: &Connection, json: &str, match_query: &str) -> rusqlite::Result<Vec<Row>> {
        let mut stmt = conn.prepare(
            "SELECT rowid, search_edits(notes, ?1), search_correction(notes, ?1)
             FROM notes WHERE notes MATCH ?2 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![json, match_query], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        rows.collect()
    }

    #[test]
    fn lowest_edits_win_within_a_group() {
        let conn = fixture();
        let json = serde_json::to_string(&[
            phrase(0, 0, "quikc"),
            phrase(0, 1, "quick"),
            phrase(0, 1, "quack"),
            phrase(1, 0, "fox"),
        ])
        .unwrap();
        // Rows 5 and 6 have no fox, so MATCH drops them.
        let rows = run(&conn, &json, "(quikc* OR quick OR quack) AND (fox)").unwrap();
        assert_eq!(
            rows,
            vec![
                (1, Some(1), Some("quick fox".to_string())),
                (2, Some(0), Some("quikc fox".to_string())),
                (3, Some(1), Some("quick fox".to_string())),
                (4, Some(1), Some("quack fox".to_string())),
            ]
        );
    }

    #[test]
    fn ties_go_to_the_earlier_query_position() {
        let conn = fixture();
        // Both corrections cost 1. "quack" comes first in the query, so it wins where both occur.
        let json = serde_json::to_string(&[
            phrase(0, 0, "quikc"),
            phrase(0, 1, "quack"),
            phrase(0, 1, "quick"),
            phrase(1, 0, "fox"),
        ])
        .unwrap();
        let rows = run(&conn, &json, "(quikc* OR quack OR quick) AND (fox)").unwrap();
        assert_eq!(
            rows,
            vec![
                (1, Some(1), Some("quick fox".to_string())),
                (2, Some(0), Some("quikc fox".to_string())),
                (3, Some(1), Some("quack fox".to_string())),
                (4, Some(1), Some("quack fox".to_string())),
            ]
        );
    }

    #[test]
    fn groups_are_anded_and_costs_summed_in_group_order() {
        let conn = fixture();
        let json = serde_json::to_string(&[
            phrase(0, 0, "quikc"),
            phrase(0, 1, "quick"),
            phrase(1, 0, "dgo"),
            phrase(1, 1, "dog"),
        ])
        .unwrap();
        // Rows 1-4 have no dgo or dog, so MATCH drops them.
        let rows = run(&conn, &json, "(quikc* OR quick) AND (dgo* OR dog)").unwrap();
        assert_eq!(
            rows,
            vec![
                (5, Some(2), Some("quick dog".to_string())),
                (6, Some(0), Some("quikc dgo".to_string())),
            ]
        );
    }

    #[test]
    fn no_matching_rows_gives_no_rows() {
        let conn = fixture();
        let json = serde_json::to_string(&[phrase(0, 0, "zebra")]).unwrap();
        assert!(run(&conn, &json, "zebra").unwrap().is_empty());
    }

    #[test]
    fn phrase_count_must_match_the_match_query() {
        let conn = fixture();
        let json = serde_json::to_string(&[phrase(0, 0, "quikc")]).unwrap();
        // The MATCH has two phrases and the list has one, so the call fails.
        assert!(run(&conn, &json, "quikc* OR quick").is_err());
    }

    #[test]
    fn different_json_arguments_in_one_query_do_not_mix() {
        let conn = fixture();
        let a = serde_json::to_string(&[
            phrase(0, 0, "quikc"),
            phrase(0, 1, "quick"),
            phrase(0, 1, "quack"),
            phrase(1, 0, "fox"),
        ])
        .unwrap();
        // Same phrase order as `a`, with different costs.
        let b = serde_json::to_string(&[
            phrase(0, 5, "quikc"),
            phrase(0, 0, "quick"),
            phrase(0, 0, "quack"),
            phrase(1, 0, "fox"),
        ])
        .unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT rowid, search_edits(notes, ?1), search_edits(notes, ?2)
                 FROM notes WHERE notes MATCH ?3 ORDER BY rowid",
            )
            .unwrap();
        let rows = stmt
            .query_map(
                params![a, b, "(quikc* OR quick OR quack) AND (fox)"],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<(i64, i64, i64)>>>()
            .unwrap();
        assert_eq!(rows, vec![(1, 1, 0), (2, 0, 5), (3, 1, 0), (4, 1, 0)]);
    }

    #[test]
    fn null_json_gives_null() {
        let conn = fixture();
        let mut stmt = conn
            .prepare(
                "SELECT search_edits(notes, NULL) FROM notes WHERE notes MATCH 'fox' ORDER BY rowid",
            )
            .unwrap();
        let values = stmt
            .query_map([], |row| row.get::<_, Option<i64>>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(values, vec![None, None, None, None]);
    }

    #[test]
    fn phrase_list_encodes_with_plain_field_names() {
        let json = serde_json::to_string(&[phrase(2, 1, "quick")]).unwrap();
        assert_eq!(json, r#"[{"group":2,"edits":1,"term":"quick"}]"#);
    }
}
