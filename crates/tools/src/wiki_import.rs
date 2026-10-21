//! `import_markdown`: imports markdown archives into the Projects database.
//!
//! - Projects is a top-level database found by the page property [`MARKER`]
//!   and created on first use. Its rows are found by exact name and keep
//!   their ids. `status` is set to `imported` when a row is created or has
//!   no status; any other status (such as `cut over`) is kept.
//! - A single root is imported directly below its row. Several roots each
//!   get a child container of the row, marked [`ROOT_MARKER`] and found by
//!   root name, so reimports reuse them.
//! - Archives are plain tar files, base64 encoded. Every archive is checked
//!   and unpacked into a temporary tree before the vault is touched. The core
//!   importer then handles identity, reimports, orphans and secret skips.
//! - `dry_run` runs the same steps against a fresh empty vault in a temporary
//!   directory. It never reads or writes the caller's vault.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::path::Path;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tar::EntryType;
use vault_core::{
    ImportReport, PageId, PageSummary, PropDef, PropKind, PropValue, Property, SelectOption, Vault,
};

use crate::props;
use crate::{err, Result, ToolError};

/// The largest raw archive accepted per root, in bytes (the decoded tar).
pub const DEFAULT_SOURCE_LIMIT: usize = 64 << 20;
/// The page property that marks the Projects database.
pub const MARKER: &str = "ai.projects";
/// The page property that marks a root container of a multi-root project.
pub const ROOT_MARKER: &str = "ai.project_root";
pub const TITLE: &str = "Projects";
pub const IMPORTED: &str = "imported";
/// The literal select id, with a space; it is settled and must not change.
pub const CUT_OVER: &str = "cut over";

// The database/row lookup and the core's source-key scan must see the prior
// import's writes when several session hooks reach the same hub together.
static IMPORT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn def(key: &str, name: &str, kind: PropKind, options: Vec<SelectOption>) -> PropDef {
    PropDef {
        key: key.into(),
        name: name.into(),
        kind,
        options,
        config: String::new(),
    }
}

fn opt(id: &str, name: &str, color: &str) -> SelectOption {
    SelectOption {
        id: id.into(),
        name: name.into(),
        color: color.into(),
    }
}

/// The Projects database's schema.
fn schema() -> Vec<PropDef> {
    vec![
        def(
            "status",
            "Status",
            PropKind::Select,
            vec![
                opt(IMPORTED, "Imported", "blue"),
                opt(CUT_OVER, "Cut over", "green"),
            ],
        ),
        def("repo_path", "Repo path", PropKind::Text, vec![]),
        def("git_remote", "Git remote", PropKind::Text, vec![]),
        def("last_activity", "Last activity", PropKind::Date, vec![]),
    ]
}

/// Every child of `parent` (`None`: top level), in pages of 200.
fn children(v: &Vault, parent: Option<&PageId>) -> Result<Vec<PageSummary>> {
    let mut all = Vec::new();
    loop {
        let page = v.list_children(parent.cloned(), all.len() as u32, 200)?;
        let last = page.len() < 200;
        all.extend(page);
        if last {
            return Ok(all);
        }
    }
}

fn has_flag(props: &[Property], key: &str) -> bool {
    props
        .iter()
        .any(|p| p.key == key && p.value == PropValue::Checkbox(true))
}

fn current(props: &[Property], key: &str) -> PropValue {
    props
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.value.clone())
        .unwrap_or(PropValue::Empty)
}

/// The Projects database, found by its marker (the oldest one if two
/// devices made one each before syncing).
fn collection(v: &Vault) -> Result<Option<PageId>> {
    let mut found: Option<PageSummary> = None;
    for top in children(v, None)? {
        if top.is_database()
            && has_flag(&top.properties, MARKER)
            && found.as_ref().is_none_or(|f| top.created_ms < f.created_ms)
        {
            found = Some(top);
        }
    }
    Ok(found.map(|p| p.id))
}

/// The Projects database, created if missing. An existing one gets any
/// schema keys it lacks; its other settings are left alone.
fn ensure_collection(v: &Vault) -> Result<PageId> {
    if let Some(id) = collection(v)? {
        let mut defs = v.get_schema(id.clone())?;
        let missing: Vec<PropDef> = schema()
            .into_iter()
            .filter(|d| !defs.iter().any(|x| x.key == d.key))
            .collect();
        if !missing.is_empty() {
            defs.extend(missing);
            v.set_schema(id.clone(), defs)?;
        }
        return Ok(id);
    }
    let id = v.create_page(None, TITLE.into())?;
    v.make_collection(id.clone())?;
    v.set_schema(id.clone(), schema())?;
    v.set_property(id.clone(), MARKER.into(), PropValue::Checkbox(true))?;
    Ok(id)
}

fn write(v: &Vault, row: &PageId, key: &str, value: PropValue) -> Result<()> {
    match value {
        PropValue::Empty => v.remove_property(row.clone(), key.into())?,
        value => v.set_property(row.clone(), key.into(), value)?,
    }
    Ok(())
}

fn text_value(s: &str) -> PropValue {
    if s.is_empty() {
        PropValue::Empty
    } else {
        PropValue::Text(s.to_string())
    }
}

/// The row named `input.name`, created if missing. Returns its id, whether
/// it was created, and whether any of its properties changed. Only changed
/// properties are written.
fn upsert_row(v: &Vault, db: &PageId, input: &Input) -> Result<(PageId, bool, bool)> {
    let existing = children(v, Some(db))?
        .into_iter()
        .filter(|row| row.title == input.name)
        .min_by_key(|row| row.created_ms);
    let desired = vec![
        ("repo_path", text_value(&input.repo_path)),
        ("git_remote", text_value(&input.git_remote)),
        ("last_activity", input.last_activity.clone()),
    ];
    let Some(row) = existing else {
        let id = v.create_page(Some(db.clone()), input.name.clone())?;
        for (key, value) in desired {
            if value != PropValue::Empty {
                write(v, &id, key, value)?;
            }
        }
        write(v, &id, "status", PropValue::Select(IMPORTED.into()))?;
        return Ok((id, true, false));
    };
    let mut writes: Vec<(&str, PropValue)> = desired
        .into_iter()
        .filter(|(key, value)| current(&row.properties, key) != *value)
        .collect();
    let status_kept = matches!(
        current(&row.properties, "status"),
        PropValue::Select(s) | PropValue::Status(s) | PropValue::Text(s) if !s.is_empty()
    );
    if !status_kept {
        writes.push(("status", PropValue::Select(IMPORTED.into())));
    }
    let updated = !writes.is_empty();
    for (key, value) in writes {
        write(v, &row.id, key, value)?;
    }
    Ok((row.id, false, updated))
}

/// The container of a root under `row`, created if missing.
fn root_container(v: &Vault, row: &PageId, name: &str) -> Result<PageId> {
    let found = children(v, Some(row))?
        .into_iter()
        .filter(|c| c.title == name && has_flag(&c.properties, ROOT_MARKER))
        .min_by_key(|c| c.created_ms);
    if let Some(c) = found {
        return Ok(c.id);
    }
    let id = v.create_page(Some(row.clone()), name.into())?;
    v.set_property(id.clone(), ROOT_MARKER.into(), PropValue::Checkbox(true))?;
    Ok(id)
}

/// One root, checked and unpacked. The tree is removed when this drops.
struct Root {
    name: String,
    exclude: Vec<String>,
    tree: tempfile::TempDir,
}

/// The whole payload, validated and unpacked before any vault write.
struct Input {
    name: String,
    repo_path: String,
    git_remote: String,
    last_activity: PropValue,
    exclude: Vec<String>,
    roots: Vec<Root>,
    dry_run: bool,
}

/// One root's import result. The import counts are flattened in beside the
/// unlinked-file accounting, which the CLI reads back. Both new fields default
/// so replies from an older hub still parse.
#[derive(Debug, Serialize, Deserialize)]
pub struct WikiImportReport {
    #[serde(flatten)]
    pub import: ImportReport,
    /// Number of files in `unlinked_files`.
    #[serde(default)]
    pub not_imported: u32,
    /// Non-markdown source files no imported page links to, relative to the
    /// root with forward slashes, sorted. Skipped (secret) files are not listed.
    #[serde(default)]
    pub unlinked_files: Vec<String>,
}

#[derive(Serialize)]
struct RootOutput {
    name: String,
    report: WikiImportReport,
}

#[derive(Serialize)]
struct Output {
    project_page: PageId,
    row_created: bool,
    row_updated: bool,
    dry_run: bool,
    counts_relative_to: &'static str,
    roots: Vec<RootOutput>,
}

fn text_field(a: &Value, key: &str, label: &str) -> Result<String> {
    match a.get(key).and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(ToolError(format!("{label} is required"))),
    }
}

fn optional_text(a: &Value, key: &str, label: &str) -> Result<String> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.trim().to_string()),
        Some(_) => err(format!("{label} must be text")),
    }
}

fn string_list(v: Option<&Value>, label: &str) -> Result<Vec<String>> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_str().map(str::trim) {
                Some(s) if !s.is_empty() => Ok(s.to_string()),
                _ => Err(ToolError(format!(
                    "{label} must be a list of non-empty text"
                ))),
            })
            .collect(),
        Some(_) => err(format!("{label} must be a list of text")),
    }
}

fn source_err(source: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError(format!("root {source:?}: {e}"))
}

/// The `last_activity` value. RFC 3339 with an offset (as git's `%cI` and
/// file mtimes give) is stored as its UTC instant with milliseconds. Date
/// forms without an offset are read as `props::date_value` reads them. A
/// missing, null or empty value is `Empty`: a source may have no commits.
fn activity_value(a: &Value) -> Result<PropValue> {
    let text = match a.get("last_activity") {
        None | Some(Value::Null) => return Ok(PropValue::Empty),
        Some(Value::String(s)) => s.trim(),
        Some(_) => return err("last_activity must be an RFC 3339 time or ISO date"),
    };
    if text.is_empty() {
        return Ok(PropValue::Empty);
    }
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(text) {
        return Ok(PropValue::Date {
            start_ms: time.timestamp_millis(),
            end_ms: None,
            has_time: true,
        });
    }
    props::date_value(&Value::String(text.to_string()))
        .map_err(|e| ToolError(format!("last_activity: {e}")))
}

impl Input {
    /// Checks the whole payload and unpacks every archive. Nothing is written.
    fn parse(a: &Value, source_limit: usize) -> Result<Self> {
        let name = text_field(a, "name", "name")?;
        let repo_path = optional_text(a, "repo_path", "repo_path")?;
        let git_remote = optional_text(a, "git_remote", "git_remote")?;
        let last_activity = activity_value(a)?;
        let exclude = string_list(a.get("exclude"), "exclude")?;
        let dry_run = match a.get("dry_run") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return err("dry_run must be true or false"),
        };
        let items = a
            .get("roots")
            .and_then(Value::as_array)
            .filter(|r| !r.is_empty())
            .ok_or_else(|| ToolError("roots must be a non-empty list".into()))?;
        let mut names = HashSet::new();
        let mut roots = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let root_name = text_field(item, "name", &format!("roots[{i}].name"))?;
            if !names.insert(root_name.clone()) {
                return err(format!("duplicate root name {root_name:?}"));
            }
            let encoded = text_field(
                item,
                "archive_base64",
                &format!("roots[{i}].archive_base64"),
            )?;
            let root_exclude = string_list(item.get("exclude"), &format!("roots[{i}].exclude"))?;
            let tree = unpack(&root_name, &encoded, source_limit)?;
            roots.push(Root {
                name: root_name,
                exclude: root_exclude,
                tree,
            });
        }
        Ok(Self {
            name,
            repo_path,
            git_remote,
            last_activity,
            exclude,
            roots,
            dry_run,
        })
    }
}

/// Decodes one base64 plain tar into a new temporary tree. The size check
/// uses the encoded length, so an oversized archive is refused before decoding.
fn unpack(source: &str, encoded: &str, limit: usize) -> Result<tempfile::TempDir> {
    if encoded.len() % 4 != 0 {
        return Err(source_err(source, "archive_base64 is not valid base64"));
    }
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|b| *b == b'=')
        .count()
        .min(2);
    let decoded_len = encoded.len() / 4 * 3 - padding;
    if decoded_len > limit {
        return Err(source_err(
            source,
            format!("archive is {decoded_len} bytes, over the {limit} byte source limit"),
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| source_err(source, format!("archive_base64 is not valid base64: {e}")))?;
    let tree = tempfile::tempdir().map_err(|e| source_err(source, e))?;
    extract(source, &bytes, tree.path())?;
    Ok(tree)
}

/// The safe parts of an archive path: relative, no `..`, no backslash, no
/// drive letter. `.` and empty parts are dropped, so `./a.md` is `a.md`.
fn safe_parts(raw: &[u8]) -> std::result::Result<Vec<&str>, &'static str> {
    let path = std::str::from_utf8(raw).map_err(|_| "is not UTF-8")?;
    if path.contains('\\') {
        return Err("uses a backslash");
    }
    if path.contains('\0') {
        return Err("contains a NUL byte");
    }
    if path.starts_with('/') {
        return Err("is absolute");
    }
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return Err("has a drive letter");
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        let component = part.as_bytes();
        if component.len() >= 2 && component[1] == b':' && component[0].is_ascii_alphabetic() {
            return Err("has a drive letter component");
        }
        if cfg!(windows) && part.contains(':') {
            return Err("has a Windows stream separator");
        }
        match part {
            "" | "." => {}
            ".." => return Err("contains .."),
            _ => parts.push(part),
        }
    }
    Ok(parts)
}

/// Writes the regular files and directories of `bytes` under `root`.
/// Links and special entries are refused. Nothing is written outside `root`.
fn extract(source: &str, bytes: &[u8], root: &Path) -> Result<()> {
    let not_tar = |e: io::Error| source_err(source, format!("not a plain tar archive: {e}"));
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    for entry in archive.entries().map_err(not_tar)? {
        let mut entry = entry.map_err(not_tar)?;
        let kind = entry.header().entry_type();
        if matches!(kind, EntryType::XGlobalHeader) {
            continue;
        }
        let raw_path = entry.path_bytes().into_owned();
        let shown = String::from_utf8_lossy(&raw_path).into_owned();
        let trailing_slash = raw_path.ends_with(b"/");
        let is_dir = match kind {
            EntryType::Directory => true,
            EntryType::Regular | EntryType::Continuous => trailing_slash,
            EntryType::Link | EntryType::Symlink => {
                return Err(source_err(
                    source,
                    format!("entry {shown:?} is a link; links are refused"),
                ))
            }
            _ => {
                return Err(source_err(
                    source,
                    format!("entry {shown:?} is not a regular file or directory"),
                ))
            }
        };
        let parts = safe_parts(&raw_path)
            .map_err(|why| source_err(source, format!("entry {shown:?} {why}")))?;
        if parts.is_empty() {
            continue;
        }
        let mut target = root.to_path_buf();
        for part in &parts {
            target.push(part);
        }
        if !target.starts_with(root) {
            return Err(source_err(
                source,
                format!("entry {shown:?} leaves the archive"),
            ));
        }
        if is_dir {
            if entry.header().size().unwrap_or(0) > 0 {
                return Err(source_err(
                    source,
                    format!("entry {shown:?} is a directory with content"),
                ));
            }
            fs::create_dir_all(&target).map_err(|e| source_err(source, e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| source_err(source, e))?;
            }
            let mut file = File::create(&target).map_err(|e| source_err(source, e))?;
            io::copy(&mut entry, &mut file).map_err(|e| source_err(source, e))?;
        }
    }
    Ok(())
}

/// The state of the Projects row and its imports after a run.
struct Projected {
    project_page: PageId,
    row_created: bool,
    row_updated: bool,
    roots: Vec<RootOutput>,
}

/// Writes the row and imports every root into `v`.
fn project(v: &Vault, input: &Input) -> Result<Projected> {
    let db = ensure_collection(v)?;
    let (row, row_created, row_updated) = upsert_row(v, &db, input)?;
    let several = input.roots.len() > 1;
    let mut roots = Vec::new();
    for root in &input.roots {
        let parent = if several {
            root_container(v, &row, &root.name)?
        } else {
            row.clone()
        };
        let excludes: Vec<String> = input.exclude.iter().chain(&root.exclude).cloned().collect();
        let dir = root.tree.path().to_string_lossy().into_owned();
        let (import, unlinked_files) =
            v.import_markdown_reporting_unlinked(dir, Some(parent), excludes)?;
        let report = WikiImportReport {
            import,
            not_imported: u32::try_from(unlinked_files.len()).unwrap_or(u32::MAX),
            unlinked_files,
        };
        roots.push(RootOutput {
            name: root.name.clone(),
            report,
        });
    }
    Ok(Projected {
        project_page: row,
        row_created,
        row_updated,
        roots,
    })
}

/// Runs `import_markdown` with `a` against `v` and returns the JSON result.
/// With `dry_run`, `v` is not used: the work happens in a fresh empty vault
/// in a temporary directory, which is dropped before the directory is removed.
pub fn call(v: &Vault, a: &Value, source_limit: usize) -> Result<String> {
    let input = Input::parse(a, source_limit)?;
    let (projected, relative_to) = if input.dry_run {
        let scratch = tempfile::tempdir().map_err(|e| ToolError(format!("dry run: {e}")))?;
        let projected = {
            let empty = Vault::open(scratch.path().join("vault").to_string_lossy().into_owned())?;
            project(&empty, &input)?
        };
        (projected, "empty vault")
    } else {
        let _guard = IMPORT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (project(v, &input)?, "existing vault")
    };
    let output = Output {
        project_page: projected.project_page,
        row_created: projected.row_created,
        row_updated: projected.row_updated,
        dry_run: input.dry_run,
        counts_relative_to: relative_to,
        roots: projected.roots,
    };
    serde_json::to_string(&output).map_err(|e| ToolError(e.to_string()))
}
