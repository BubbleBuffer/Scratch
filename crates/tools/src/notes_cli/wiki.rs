//! `notes wiki import`: packs one project wiki folder and sends it to the hub.
//!
//! Local rules run before packing, so skipped files are never transmitted:
//! symbolic links, dot entries, secret file patterns and caller excludes.
//! Everything here is read-only with respect to the repository.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::wiki_import::WikiImportReport;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use vault_core::ImportReport;

use super::{tool_text, Client, Config};

/// Hub tool that receives the packed wiki (`tools.rs`, the hidden `import_markdown`).
pub const TOOL: &str = "import_markdown";
/// The project whose wiki is never imported; its refusal needs no hub.
pub const REFUSED_PROJECT: &str = "notes-app";

pub const REFUSED_LINE: &str =
    "wiki import: project notes-app is not imported from its wiki; nothing sent";
pub const NOTHING_LINE: &str =
    "wiki import: nothing to import (no DIR given, and the git working tree has no wiki/ or docs/)";

/// Parsed command-line options. `excludes` is repeatable; every value is a
/// relative path or glob, matched the same way as the core importer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    pub project: Option<String>,
    pub dry_run: bool,
    pub excludes: Vec<String>,
    pub report: Option<PathBuf>,
    pub dir: Option<PathBuf>,
}

/// Where a wiki lives and which project it belongs to. Built without config
/// or network access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub root: PathBuf,
    pub root_name: String,
    pub project: String,
    /// The main repository root, or the directory itself outside Git.
    pub repo_root: PathBuf,
    pub in_git: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
    NothingToImport,
    Refused,
    Ready(Source),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Relative source path; skipped directories end in `/`.
    pub path: String,
    pub reason: &'static str,
}

/// Everything the hub call needs, with the archive already packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub project: String,
    pub repo_path: String,
    pub git_remote: Option<String>,
    pub last_activity: Option<String>,
    pub root_name: String,
    pub excludes: Vec<String>,
    pub archive: Vec<u8>,
    pub local_skipped: Vec<Skipped>,
    pub packed_files: usize,
    pub dry_run: bool,
}

/// The hub's tool text. Field names are the hub contract.
#[derive(Debug, Deserialize)]
pub struct Reply {
    pub project_page: String,
    pub row_created: bool,
    pub row_updated: bool,
    pub dry_run: bool,
    pub counts_relative_to: String,
    pub roots: Vec<RootReply>,
}

#[derive(Debug, Deserialize)]
pub struct RootReply {
    pub name: String,
    pub report: WikiImportReport,
}

/// Resolves the wiki directory and project name. Runs no hub or config code,
/// so a refused or empty run never needs credentials.
pub fn resolve(opts: &Options, cwd: &Path) -> Result<Resolved> {
    // An explicit notes-app name refuses before any directory is read.
    if opts.project.as_deref() == Some(REFUSED_PROJECT) {
        return Ok(Resolved::Refused);
    }
    if opts.dir.is_none() && opts.project.is_none() {
        let common = git_output(
            cwd,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .and_then(|path| fs::canonicalize(path).ok());
        if common
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(sanitize)
            .as_deref()
            == Some(REFUSED_PROJECT)
        {
            return Ok(Resolved::Refused);
        }
    }
    let root = match &opts.dir {
        Some(dir) => {
            let root = fs::canonicalize(cwd.join(dir))
                .map_err(|_| anyhow!("cannot read wiki directory"))?;
            if !root.is_dir() {
                bail!("wiki path is not a directory");
            }
            plain_path(root)
        }
        None => {
            let Some(top) = git_output(cwd, &["rev-parse", "--show-toplevel"]) else {
                return Ok(Resolved::NothingToImport);
            };
            let top = PathBuf::from(top);
            match ["wiki", "docs"]
                .iter()
                .map(|name| top.join(name))
                .find(|path| path.is_dir())
            {
                Some(path) => plain_path(
                    fs::canonicalize(path).map_err(|_| anyhow!("cannot read wiki directory"))?,
                ),
                None => return Ok(Resolved::NothingToImport),
            }
        }
    };

    // Same rule as session-start.sh: the parent of Git's common directory, so
    // linked worktrees share the main repository's name.
    let common = git_output(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .and_then(|path| fs::canonicalize(path).ok())
    .map(plain_path);
    let (repo_root, in_git) = match common.as_deref().and_then(Path::parent) {
        Some(parent) => (parent.to_path_buf(), true),
        None => (root.clone(), false),
    };

    let base = if in_git {
        repo_root.file_name()
    } else {
        root.file_name()
    };
    let project = match &opts.project {
        Some(name) => {
            if !valid_project_name(name) {
                bail!("invalid project name; use letters, digits, '.', '_' or '-'");
            }
            name.clone()
        }
        None => base
            .and_then(|name| name.to_str())
            .map(sanitize)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| anyhow!("cannot name the wiki project"))?,
    };
    if project == REFUSED_PROJECT {
        return Ok(Resolved::Refused);
    }
    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("cannot name the wiki folder"))?;
    Ok(Resolved::Ready(Source {
        root,
        root_name,
        project,
        repo_root,
        in_git,
    }))
}

/// Packs the folder and collects the git facts. Does not modify the repository.
pub fn prepare(source: &Source, opts: &Options) -> Result<Prepared> {
    let packed = pack(&source.root, &opts.excludes)?;
    Ok(Prepared {
        project: source.project.clone(),
        repo_path: source.repo_root.to_string_lossy().into_owned(),
        git_remote: git_remote(source),
        last_activity: last_activity(source, packed.newest),
        root_name: source.root_name.clone(),
        excludes: opts.excludes.clone(),
        archive: packed.archive,
        local_skipped: packed.skipped,
        packed_files: packed.files,
        dry_run: opts.dry_run,
    })
}

impl Prepared {
    /// The hub payload. The archive is base64 of a plain tar.
    pub fn payload(&self) -> Value {
        json!({
            "name": self.project,
            "repo_path": self.repo_path,
            "git_remote": self.git_remote,
            "last_activity": self.last_activity,
            "roots": [{
                "name": self.root_name,
                "archive_base64": base64::engine::general_purpose::STANDARD.encode(&self.archive),
                "exclude": self.excludes,
            }],
            "dry_run": self.dry_run,
        })
    }
}

/// Sends the prepared import through the existing client. Failures are
/// returned as errors; the client's timeouts keep an unreachable hub fast.
pub fn send(client: &Client, prepared: &Prepared) -> Result<Reply> {
    let result = client.call(TOOL, &prepared.payload())?;
    let text = tool_text(&result)?;
    parse_reply(&text)
}

/// Accepts only complete file-accounting reports. The shared report type's
/// serde defaults support older consumers, but a missing category is unknown,
/// not zero: the CLI must not claim that every source file was accounted for.
pub fn parse_reply(text: &str) -> Result<Reply> {
    let value: Value =
        serde_json::from_str(text).map_err(|_| anyhow!("invalid wiki import response"))?;
    let roots = value
        .get("roots")
        .and_then(Value::as_array)
        .filter(|roots| !roots.is_empty())
        .ok_or_else(|| anyhow!("invalid wiki import response"))?;
    for root in roots {
        let report = root
            .get("report")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("invalid wiki import response"))?;
        if !report.contains_key("not_imported") || !report.contains_key("unlinked_files") {
            bail!("notes hub wiki import response lacks unlinked-file reporting; update the hub");
        }
    }
    let reply: Reply =
        serde_json::from_value(value).map_err(|_| anyhow!("invalid wiki import response"))?;
    for root in &reply.roots {
        if root.report.not_imported as usize != root.report.unlinked_files.len() {
            bail!("invalid wiki import response: unlinked-file count does not match its list");
        }
    }
    Ok(reply)
}

/// The whole command. `config` loads only after the wiki is known to need
/// importing, so a refusal or an empty run never reads credentials.
pub fn run(opts: &Options, cwd: &Path, config: impl FnOnce() -> Result<Config>) -> Result<String> {
    let source = match resolve(opts, cwd)? {
        Resolved::NothingToImport => return Ok(NOTHING_LINE.into()),
        Resolved::Refused => return Ok(REFUSED_LINE.into()),
        Resolved::Ready(source) => source,
    };
    let config = config()?;
    let token = config.token.clone();
    let client = Client::new(config)?;
    let prepared = prepare(&source, opts)?;
    let reply = send(&client, &prepared).map_err(|e| anyhow!(redact(&e.to_string(), &token)))?;
    let mut text = summary(&reply, &prepared);
    if let Some(path) = &opts.report {
        fs::write(cwd.join(path), report(&reply, &prepared, &token))
            .map_err(|_| anyhow!("cannot write report file"))?;
        text.push_str(&format!("\nreport: {}", path.display()));
    }
    Ok(redact(&text, &token))
}

/// One short line per root, with the counts a user checks first.
pub fn summary(reply: &Reply, prepared: &Prepared) -> String {
    let mode = if reply.dry_run {
        " (dry run, nothing written)"
    } else {
        ""
    };
    let mut lines = vec![format!("wiki import: project {}{mode}", prepared.project)];
    let row = if reply.row_created {
        "created"
    } else if reply.row_updated {
        "updated"
    } else {
        "unchanged"
    };
    lines.push(format!("project page {}: row {row}", reply.project_page));
    for (index, root) in reply.roots.iter().enumerate() {
        let r = &root.report.import;
        // The CLI sends exactly one root, so local skips belong to the first.
        let local = if index == 0 {
            prepared.local_skipped.len()
        } else {
            0
        };
        let hub = r.skipped_files.len();
        lines.push(format!(
            "{}: pages {}, created {}, updated {}, unchanged {}, orphaned {}, attachments {}, images {}, tables {}, links {} resolved / {} unresolved, skipped {} ({local} local, {hub} hub), not imported {} (not linked from any page)",
            root.name,
            imported_page_count(r),
            r.created,
            r.updated,
            r.unchanged,
            r.orphaned,
            r.attachments,
            r.images,
            r.tables,
            r.resolved_links,
            r.unresolved_links,
            local + hub,
            root.report.not_imported,
        ));
    }
    lines.push(format!("counts relative to: {}", reply.counts_relative_to));
    lines.join("\n")
}

fn imported_page_count(report: &ImportReport) -> u64 {
    u64::from(report.created) + u64::from(report.updated) + u64::from(report.unchanged)
}

/// Full markdown for `--report`: every count, unresolved targets, orphaned
/// sources, unlinked files and each skipped file with its reason. Credentials
/// are redacted.
pub fn report(reply: &Reply, prepared: &Prepared, token: &str) -> String {
    let mut out = vec![
        format!("# Wiki import: {}", prepared.project),
        String::new(),
        format!("- Project page: {}", reply.project_page),
        format!(
            "- Project row: {}",
            if reply.row_created {
                "created"
            } else if reply.row_updated {
                "updated"
            } else {
                "unchanged"
            }
        ),
        format!("- Dry run: {}", if reply.dry_run { "yes" } else { "no" }),
        format!("- Counts relative to: {}", reply.counts_relative_to),
        format!("- Repo path: {}", prepared.repo_path),
        format!(
            "- Git remote: {}",
            prepared.git_remote.as_deref().unwrap_or("none")
        ),
        format!(
            "- Last activity: {}",
            prepared.last_activity.as_deref().unwrap_or("unknown")
        ),
        format!("- Files packed: {}", prepared.packed_files),
        format!(
            "- Excludes: {}",
            if prepared.excludes.is_empty() {
                "none".to_string()
            } else {
                prepared
                    .excludes
                    .iter()
                    .map(|e| format!("`{e}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
    ];
    for (index, root) in reply.roots.iter().enumerate() {
        let r = &root.report.import;
        out.extend([
            String::new(),
            format!("## Root: {}", root.name),
            String::new(),
            "| Count | Value |".into(),
            "|---|---|".into(),
            format!("| Pages | {} |", imported_page_count(r)),
            format!("| Created | {} |", r.created),
            format!("| Updated | {} |", r.updated),
            format!("| Unchanged | {} |", r.unchanged),
            format!("| Orphaned | {} |", r.orphaned),
            format!("| Attachments | {} |", r.attachments),
            format!("| Images | {} |", r.images),
            format!("| Tables | {} |", r.tables),
            format!("| Resolved links | {} |", r.resolved_links),
            format!("| Unresolved links | {} |", r.unresolved_links),
            format!(
                "| Not imported (not linked from any page) | {} |",
                root.report.not_imported
            ),
            String::new(),
            "### Not imported: not linked from any page".into(),
            String::new(),
            list_or_none(root.report.unlinked_files.iter().map(|s| format!("`{s}`"))),
            String::new(),
            "### Unresolved link targets".into(),
            String::new(),
            list_or_none(
                r.unresolved_targets
                    .iter()
                    .map(|t| format!("`{}` → `{}`", t.source_file, t.target)),
            ),
            String::new(),
            "### Orphaned sources".into(),
            String::new(),
            list_or_none(r.orphaned_sources.iter().map(|s| format!("`{s}`"))),
            String::new(),
            "### Skipped files".into(),
            String::new(),
        ]);
        // The CLI sends exactly one root, so local skips belong to the first.
        let local: Vec<String> = if index == 0 {
            prepared
                .local_skipped
                .iter()
                .map(|s| format!("`{}`: {} (local, not transmitted)", s.path, s.reason))
                .collect()
        } else {
            Vec::new()
        };
        let hub = r
            .skipped_files
            .iter()
            .map(|s| format!("`{}`: {} (hub)", s.source_file, s.reason));
        out.push(list_or_none(local.into_iter().chain(hub)));
    }
    redact(&out.join("\n"), token) + "\n"
}

fn list_or_none(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items.map(|item| format!("- {item}")).collect();
    if items.is_empty() {
        "- None.".into()
    } else {
        items.join("\n")
    }
}

/// Replaces the token and removes credentials from `scheme://user:pass@host`.
pub fn redact(text: &str, token: &str) -> String {
    let text = if token.is_empty() {
        text.to_string()
    } else {
        text.replace(token, "[redacted]")
    };
    redact_urls(&text)
}

fn redact_urls(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let (head, tail) = rest.split_at(index + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ')'))
            .unwrap_or(tail.len());
        let (authority, after) = tail.split_at(end);
        match authority.rsplit_once('@') {
            Some((_, host)) => {
                out.push_str("[redacted]@");
                out.push_str(host);
            }
            None => out.push_str(authority),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Keeps scheme, host, port and path of a git remote. Userinfo, query and
/// fragment are dropped: any of them can carry a token, and none may reach
/// the vault or a report.
pub fn strip_credentials(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    if let Some((scheme, rest)) = url.split_once("://") {
        let end = rest.find('/').unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        return format!("{scheme}://{host}{tail}");
    }
    // scp-style `[user[:password]@]host:path`: an `@` before any `/`.
    if let Some(at) = url.find('@') {
        if !url[..at].contains('/') && url[at + 1..].contains(':') {
            return url[at + 1..].to_string();
        }
    }
    url.to_string()
}

fn git_remote(source: &Source) -> Option<String> {
    if !source.in_git {
        return None;
    }
    git_output(&source.root, &["config", "--get", "remote.origin.url"])
        .map(|url| strip_credentials(&url))
}

/// The last commit touching the wiki folder, else the newest packed file's mtime.
fn last_activity(source: &Source, newest: Option<SystemTime>) -> Option<String> {
    if source.in_git {
        if let Some(date) = git_output(&source.root, &["log", "-1", "--format=%cI", "--", "."]) {
            return Some(date);
        }
    }
    newest
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|age| DateTime::<Utc>::from_timestamp(age.as_secs() as i64, 0))
        .map(|time| time.to_rfc3339())
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Removes the Windows verbatim prefix from canonical paths.
fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().into_owned();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// Mirrors session-start.sh's `tr -c 'A-Za-z0-9._-' '_'`, which works on
/// bytes: each byte outside the set becomes `_`, so a non-ASCII character
/// becomes one `_` per UTF-8 byte.
pub fn sanitize(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
                b as char
            } else {
                '_'
            }
        })
        .collect()
}

fn valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A packed archive plus what was left out and why.
#[derive(Debug, Default)]
pub struct Packed {
    pub archive: Vec<u8>,
    pub skipped: Vec<Skipped>,
    pub files: usize,
    pub newest: Option<SystemTime>,
}

/// Packs the folder as a plain tar with paths relative to `root`. Folders are
/// kept even when empty, so the tree survives; excluded, secret, dot and
/// symbolic-link entries are never packed. Files stream from disk one at a
/// time. The size limit is the hub's, not a client cap.
pub fn pack(root: &Path, excludes: &[String]) -> Result<Packed> {
    let mut walk = Walk {
        excludes,
        skipped: Vec::new(),
        items: Vec::new(),
    };
    walk.dir(root, "")?;
    let mut builder = tar::Builder::new(Vec::new());
    builder.mode(tar::HeaderMode::Deterministic);
    let mut packed = Packed::default();
    for item in walk.items {
        match item {
            Item::Dir(relative) => {
                let mut header = tar::Header::new_gnu();
                header.set_size(0);
                header.set_mode(0o755);
                header.set_entry_type(tar::EntryType::Directory);
                builder
                    .append_data(&mut header, relative.as_str(), io::empty())
                    .map_err(|_| anyhow!("cannot pack wiki folders"))?;
            }
            Item::File(file) => {
                let opened = fs::File::open(&file.absolute)
                    .and_then(|handle| Ok((handle.metadata()?.len(), handle)));
                let Ok((size, handle)) = opened else {
                    walk.skipped.push(Skipped {
                        path: file.relative,
                        reason: "cannot read file",
                    });
                    continue;
                };
                let mut header = tar::Header::new_gnu();
                header.set_size(size);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                builder
                    .append_data(&mut header, file.relative.as_str(), handle)
                    .map_err(|_| anyhow!("cannot pack wiki files"))?;
                packed.newest = packed.newest.max(file.modified);
                packed.files += 1;
            }
        }
    }
    builder
        .finish()
        .map_err(|_| anyhow!("cannot pack wiki files"))?;
    packed.archive = builder
        .into_inner()
        .map_err(|_| anyhow!("cannot pack wiki files"))?;
    packed.skipped = walk.skipped;
    Ok(packed)
}

struct Entry {
    relative: String,
    absolute: PathBuf,
    modified: Option<SystemTime>,
}

/// One archive entry, in walk order: a folder is written before its contents.
enum Item {
    Dir(String),
    File(Entry),
}

struct Walk<'a> {
    excludes: &'a [String],
    skipped: Vec<Skipped>,
    items: Vec<Item>,
}

impl Walk<'_> {
    fn dir(&mut self, dir: &Path, relative: &str) -> Result<()> {
        let mut entries = fs::read_dir(dir)
            .and_then(|entries| entries.collect::<io::Result<Vec<_>>>())
            .map_err(|_| anyhow!("cannot read wiki directory"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                self.skipped.push(Skipped {
                    path: join(relative, &entry.file_name().to_string_lossy()),
                    reason: "name is not UTF-8",
                });
                continue;
            };
            let path = join(relative, &name);
            let kind = entry
                .file_type()
                .map_err(|_| anyhow!("cannot read wiki directory"))?;
            let reason = if kind.is_symlink() {
                Some("symbolic link")
            } else if excluded(&path, self.excludes) {
                Some("excluded by caller")
            } else if !kind.is_dir() && secret_file(&name) {
                Some("secret file pattern")
            } else if name.starts_with('.') {
                Some("dot directory or file")
            } else {
                None
            };
            if let Some(reason) = reason {
                let shown = if kind.is_dir() {
                    format!("{path}/")
                } else {
                    path
                };
                self.skipped.push(Skipped {
                    path: shown,
                    reason,
                });
                continue;
            }
            if kind.is_dir() {
                self.items.push(Item::Dir(path.clone()));
                self.dir(&entry.path(), &path)?;
            } else if kind.is_file() {
                let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
                self.items.push(Item::File(Entry {
                    relative: path,
                    absolute: entry.path(),
                    modified,
                }));
            } else {
                self.skipped.push(Skipped {
                    path,
                    reason: "not a regular file",
                });
            }
        }
        Ok(())
    }
}

fn join(relative: &str, name: &str) -> String {
    if relative.is_empty() {
        name.to_string()
    } else {
        format!("{relative}/{name}")
    }
}

/// Mirrors `vault_core`'s secret file patterns. Keep the two lists in step.
pub fn secret_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.starts_with("credentials")
        || name.starts_with("id_rsa")
        || [".pem", ".key", ".p12", ".keystore", ".jks"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

/// Mirrors `vault_core`'s exclude rules: plain paths exclude their subtree,
/// and globs use `*`, `?` (one segment) and `**` (any depth).
pub fn excluded(path: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|pattern| {
        let pattern = pattern.replace('\\', "/");
        let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
        !pattern.is_empty()
            && (glob_matches(pattern, path)
                || (!pattern.contains(['*', '?']) && path.starts_with(&format!("{pattern}/"))))
    })
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    fn matches(p: &[u8], s: &[u8], memo: &mut HashMap<(usize, usize), bool>) -> bool {
        let key = (p.len(), s.len());
        if let Some(result) = memo.get(&key) {
            return *result;
        }
        let result = match p.first() {
            None => s.is_empty(),
            Some(b'*') => {
                let recursive = p.get(1) == Some(&b'*');
                let next = if recursive { 2 } else { 1 };
                matches(&p[next..], s, memo)
                    || (recursive && p.get(next) == Some(&b'/') && matches(&p[next + 1..], s, memo))
                    || (!s.is_empty() && (recursive || s[0] != b'/') && matches(p, &s[1..], memo))
            }
            Some(b'?') => !s.is_empty() && s[0] != b'/' && matches(&p[1..], &s[1..], memo),
            Some(c) => s.first() == Some(c) && matches(&p[1..], &s[1..], memo),
        };
        memo.insert(key, result);
        result
    }
    matches(pattern.as_bytes(), path.as_bytes(), &mut HashMap::new())
}
