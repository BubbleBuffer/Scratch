//! Markdown in: `import_markdown` (a folder such as an Obsidian vault) and
//! `rescan` (the vault's own `pages/*.md` edited outside the app).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use loro::{ExportMode, Frontiers};

use crate::attach;
use crate::blockdiff::{assign_ids, diff_ops, insert_ops};
use crate::collections::normalize;
use crate::doc::PageDoc;
use crate::error::Result;
use crate::ids::{new_id, now_ms};
use crate::index::{fold, Index};
use crate::markdown::{self, RenderCtx, ROLE_KEY, ROLES, RESERVED_KEYS};
use crate::mdparse::{parse_blocks, parse_iso, Resolver, Yaml};
use crate::order;
use crate::type_data::{fm_type_data, meta_type_data, split_frontmatter, MARKDOWN_KEY};
use crate::types::{
    Attachment, Block, BlockKind, Change, ImportReport, ImportSkippedFile, ImportUnresolvedTarget,
    PageId, PropDef, PropKind, PropValue, PAGE_TYPE_DATABASE, PAGE_TYPE_DOCUMENT,
};
use crate::vault::{fresh_ids, lock, prop_map, Vault};
use serde_json::Value;

// ------------------------------------------------------------ values

/// `2026-10-01`, a date-time, or a range `a → b`.
pub(crate) fn parse_date_text(s: &str) -> Option<PropValue> {
    let s = s.trim();
    let (a, b) = match s.split_once('→').or_else(|| s.split_once(" -> ")) {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (s, None),
    };
    let (start_ms, has_time) = parse_iso(a)?;
    let end_ms = match b {
        Some(b) => Some(parse_iso(b)?.0),
        None => None,
    };
    Some(PropValue::Date {
        start_ms,
        end_ms,
        has_time,
    })
}

fn is_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && !s.contains(char::is_whitespace)
}

fn wikilink(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    Some(inner.split('|').next().unwrap_or(inner).trim())
}

/// A list value's items as text (a string is split at commas).
fn items(y: &Yaml) -> Vec<String> {
    match y {
        Yaml::List(v) => v.iter().filter_map(|x| x.to_text()).collect(),
        Yaml::Str(s) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        other => other.to_text().into_iter().collect(),
    }
}

fn relation(y: &Yaml, r: &mut dyn Resolver) -> PropValue {
    let ids = items(y)
        .iter()
        .filter_map(|t| {
            let name = wikilink(t).unwrap_or(t);
            let id = r.page(name);
            if id.is_none() {
                r.unresolved(name);
            }
            id
        })
        .collect();
    PropValue::Relation(ids)
}

/// A frontmatter value of a known kind (`def` maps option names to ids).
fn parse_typed(y: &Yaml, kind: PropKind, def: Option<&PropDef>, r: &mut dyn Resolver) -> PropValue {
    let text = y.to_text();
    let option = |name: &str| -> Option<String> {
        match def {
            Some(d) => {
                let f = fold(name.trim());
                d.options
                    .iter()
                    .find(|o| fold(&o.name) == f)
                    .map(|o| o.id.clone())
            }
            None => Some(name.trim().to_string()),
        }
    };
    let v = match kind {
        PropKind::Text => text.map(PropValue::Text).unwrap_or(PropValue::Empty),
        PropKind::Url => text.map(PropValue::Url).unwrap_or(PropValue::Empty),
        PropKind::Email | PropKind::Phone => text.map(PropValue::Text).unwrap_or(PropValue::Empty),
        PropKind::Number => match y {
            Yaml::Num(n) => PropValue::Number(*n),
            Yaml::Str(s) => s
                .trim()
                .parse::<f64>()
                .map(PropValue::Number)
                .unwrap_or(PropValue::Empty),
            _ => PropValue::Empty,
        },
        PropKind::Checkbox => match y {
            Yaml::Bool(b) => PropValue::Checkbox(*b),
            Yaml::Str(s) if s.eq_ignore_ascii_case("true") => PropValue::Checkbox(true),
            Yaml::Str(s) if s.eq_ignore_ascii_case("false") => PropValue::Checkbox(false),
            _ => PropValue::Empty,
        },
        PropKind::Date => text
            .and_then(|t| parse_date_text(&t))
            .unwrap_or(PropValue::Empty),
        PropKind::Select => text
            .and_then(|t| option(&t))
            .map(PropValue::Select)
            .unwrap_or(PropValue::Empty),
        PropKind::Status => text
            .and_then(|t| option(&t))
            .map(PropValue::Status)
            .unwrap_or(PropValue::Empty),
        PropKind::MultiSelect => {
            PropValue::MultiSelect(items(y).iter().filter_map(|t| option(t)).collect())
        }
        PropKind::Relation => relation(y, r),
        PropKind::Files => PropValue::Files(
            items(y)
                .iter()
                .filter_map(|t| {
                    let last = t.rsplit(['/', '\\']).next().unwrap_or(t);
                    if attach::is_attachment_id(last) {
                        Some(last.to_string())
                    } else {
                        r.file(t)
                    }
                })
                .collect(),
        ),
        PropKind::CreatedTime | PropKind::UpdatedTime | PropKind::Formula | PropKind::Rollup => {
            PropValue::Empty
        }
    };
    normalize(v).unwrap_or(PropValue::Empty)
}

/// A frontmatter value of unknown kind.
fn infer(y: &Yaml, r: &mut dyn Resolver) -> PropValue {
    let v = match y {
        Yaml::Bool(b) => PropValue::Checkbox(*b),
        Yaml::Num(n) => PropValue::Number(*n),
        Yaml::Str(s) => {
            if let Some(d) = parse_date_text(s) {
                d
            } else if is_url(s) {
                PropValue::Url(s.clone())
            } else if wikilink(s).is_some() {
                relation(y, r)
            } else {
                PropValue::Text(s.clone())
            }
        }
        Yaml::List(v) => {
            let all_links = !v.is_empty()
                && v.iter()
                    .all(|x| x.as_str().is_some_and(|s| wikilink(s).is_some()));
            if all_links {
                relation(y, r)
            } else {
                PropValue::MultiSelect(items(y))
            }
        }
        Yaml::Null | Yaml::Map(_) => PropValue::Empty,
    };
    normalize(v).unwrap_or(PropValue::Empty)
}

/// The page role a file declares: its `role:` text when it is a known role.
/// Any other `role:` value is an ordinary property (see `is_property_key`).
fn fm_role(fm: &[(String, Yaml)]) -> Option<String> {
    fm_text(fm, ROLE_KEY).filter(|r| ROLES.contains(&r.as_str()))
}

/// Whether a frontmatter key is a property. `role` is the page role when
/// `role` names a known role; otherwise it is an ordinary property.
fn is_property_key(k: &str, role: Option<&str>) -> bool {
    if k == ROLE_KEY {
        return role.is_none();
    }
    !RESERVED_KEYS.contains(&k) && k != PAGE_TYPE_KEY
}

/// Properties of an imported page: every frontmatter key that is a property
/// (see `is_property_key`); `tags` is a multi-select (`#` stripped).
fn import_props(fm: &[(String, Yaml)], r: &mut dyn Resolver) -> Vec<(String, PropValue)> {
    let mut out = Vec::new();
    let role = fm_role(fm);
    for (k, y) in fm {
        let key = k.strip_prefix("prop:").unwrap_or(k).trim().to_string();
        if key.is_empty() || key.starts_with('$') || !is_property_key(k, role.as_deref()) {
            continue;
        }
        let v = if key.eq_ignore_ascii_case("tags") {
            let tags: Vec<String> = match y {
                Yaml::Str(s) => s
                    .split([',', ' '])
                    .map(|t| t.trim().trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect(),
                _ => items(y)
                    .into_iter()
                    .map(|t| t.trim_start_matches('#').to_string())
                    .collect(),
            };
            normalize(PropValue::MultiSelect(tags)).unwrap_or(PropValue::Empty)
        } else {
            infer(y, r)
        };
        if v != PropValue::Empty {
            out.push((key, v));
        }
    }
    out
}

fn fm_text(fm: &[(String, Yaml)], key: &str) -> Option<String> {
    fm.iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.to_text())
        .filter(|s| !s.trim().is_empty())
}

/// The front matter key of the page type. Reserved: not a property.
const PAGE_TYPE_KEY: &str = "type";

/// The page type a file declares: its `type:` text, or the old
/// `collection: true` (a database). `None` when the file says neither, so
/// a file without the line leaves the page's type as it is.
fn fm_page_type(fm: &[(String, Yaml)]) -> Option<String> {
    if let Some(t) = fm
        .iter()
        .find(|(k, _)| k == PAGE_TYPE_KEY)
        .and_then(|(_, v)| v.to_text())
    {
        return Some(t);
    }
    let legacy = fm
        .iter()
        .any(|(k, v)| k == "collection" && matches!(v, Yaml::Bool(true)));
    legacy.then(|| PAGE_TYPE_DATABASE.to_string())
}

// ------------------------------------------------------------ paths

/// `a/./b/../c` -> `a/c`, with `/` separators.
fn norm_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in p.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|p| *p != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            x => out.push(x),
        }
    }
    out.join("/")
}

fn join_rel(dir: &str, p: &str) -> String {
    if dir.is_empty() {
        norm_path(p)
    } else {
        norm_path(&format!("{dir}/{p}"))
    }
}

fn decode(p: &str) -> String {
    crate::mdparse::decode_path(p)
}

fn store_attachment(root: &Path, db: &Index, src: &Path) -> Result<String> {
    let (id, size) = attach::store_file(root, src)?;
    if db.attachment(&id)?.is_some() {
        return Ok(id);
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| id.clone());
    db.put_attachment(
        &Attachment {
            id: id.clone(),
            mime: attach::mime_of(&name).to_string(),
            name,
            size,
        },
        now_ms(),
    )?;
    Ok(id)
}

// ------------------------------------------------------------ import

/// Points embeds at the new ids of the pages this import created from their
/// source ids. Targets that are not in `ids`, or that map to `None` (shared
/// by several files), are left exactly as they were.
fn remap_embeds(blocks: &mut [Block], ids: &HashMap<String, Option<String>>) {
    for b in blocks {
        if b.kind == BlockKind::Embed {
            if let Some(new) = b
                .target
                .as_ref()
                .and_then(|t| ids.get(t))
                .and_then(Clone::clone)
            {
                b.target = Some(new);
            }
        }
        remap_embeds(&mut b.children, ids);
    }
}

/// Rewrites exact source-id string values throughout add-on data. Object
/// keys and the reserved readable body retain their original spelling;
/// unknown or ambiguous source ids remain unchanged, as for embeds.
fn remap_type_data(
    data: &mut serde_json::Map<String, Value>,
    ids: &HashMap<String, Option<String>>,
) {
    fn remap_value(value: &mut Value, ids: &HashMap<String, Option<String>>) {
        match value {
            Value::String(text) => {
                if let Some(Some(new)) = ids.get(text.as_str()) {
                    *text = new.clone();
                }
            }
            Value::Array(items) => {
                for item in items {
                    remap_value(item, ids);
                }
            }
            Value::Object(map) => {
                for value in map.values_mut() {
                    remap_value(value, ids);
                }
            }
            _ => {}
        }
    }
    for (key, value) in data {
        if key != MARKDOWN_KEY {
            remap_value(value, ids);
        }
    }
}

struct Node {
    id: String,
    source_path: String,
    source_file: String,
    existing: bool,
    title: String,
    md: Option<PathBuf>,
    /// Folder of the markdown file, relative to the import root.
    dir: String,
    children: Vec<Node>,
}

#[derive(Default)]
struct Catalog {
    /// Folded relative path without `.md` -> page id.
    by_rel: HashMap<String, String>,
    /// Folded file stem -> page id, or `None` when ambiguous.
    by_stem: HashMap<String, Option<String>>,
    /// Folded relative path -> attachment file.
    files_by_rel: HashMap<String, PathBuf>,
    /// Folded file name -> attachment file, or `None` when ambiguous.
    files_by_name: HashMap<String, Option<PathBuf>>,
    /// Every non-markdown source file that was not skipped: its relative
    /// path as written in the source (forward slashes), and its file.
    files: Vec<(String, PathBuf)>,
    denied: Vec<String>,
    skipped: Vec<ImportSkippedFile>,
    /// Frontmatter `id` of a page file (the id it was exported with) -> new
    /// page id. A source id that more than one file carries maps to `None`.
    by_source_id: HashMap<String, Option<String>>,
}

impl Catalog {
    fn add_page(&mut self, rel: &str, stem: &str, id: &str) {
        self.by_rel.insert(fold(rel), id.to_string());
        match self.by_stem.entry(fold(stem)) {
            Entry::Vacant(e) => {
                e.insert(Some(id.to_string()));
            }
            Entry::Occupied(mut e) if e.get().as_deref() != Some(id) => {
                e.insert(None);
            }
            _ => {}
        }
    }

    fn skip(&mut self, path: String, reason: &str) {
        if self.denied.contains(&fold(&path)) {
            return;
        }
        self.denied.push(fold(&path));
        self.skipped.push(ImportSkippedFile {
            source_file: path,
            reason: reason.to_string(),
        });
    }

    fn denied(&self, path: &str) -> bool {
        let path = fold(path);
        self.denied.iter().any(|p| {
            if p.ends_with('/') {
                path.starts_with(p) || path == p.trim_end_matches('/')
            } else {
                path == *p
            }
        })
    }

    fn add_source(&mut self, path: &Path, id: &str) {
        let Some(source) = fs::read(path)
            .ok()
            .and_then(|b| fm_text(&split_frontmatter(&String::from_utf8_lossy(&b)).0, "id"))
        else {
            return;
        };
        match self.by_source_id.entry(source) {
            Entry::Vacant(e) => {
                e.insert(Some(id.to_string()));
            }
            Entry::Occupied(mut e) => {
                e.insert(None);
            }
        }
    }
}

const IMPORT_SOURCE_PATH: &str = "$import_source_path";
const IMPORT_PARENT: &str = "$import_parent";

/// Small relative-path glob matcher: `*`/`?` stay in one segment, `**`
/// crosses directories. Plain directory paths exclude their whole subtree.
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

fn secret_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.starts_with("credentials")
        || name.starts_with("id_rsa")
        || [".pem", ".key", ".p12", ".keystore", ".jks"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

fn excluded(path: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|pattern| {
        let pattern = pattern.replace('\\', "/");
        let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
        !pattern.is_empty()
            && (glob_matches(pattern, path)
                || (!pattern.contains(['*', '?']) && path.starts_with(&format!("{pattern}/"))))
    })
}

fn sorted_entries(
    dir: &Path,
    rel: &str,
    cat: &mut Catalog,
    excludes: &[String],
) -> Result<Vec<(String, PathBuf, bool)>> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let kind = e.file_type()?;
        let is_dir = kind.is_dir();
        let rel_path = join_rel(rel, &name);
        let reason = if kind.is_symlink() {
            Some("symbolic link")
        } else if excluded(&rel_path, excludes) {
            Some("excluded by caller")
        } else if !is_dir && secret_file(&name) {
            Some("secret file pattern")
        } else if name.starts_with('.') {
            Some("dot directory or file")
        } else {
            None
        };
        if let Some(reason) = reason {
            cat.skip(
                if is_dir {
                    format!("{rel_path}/")
                } else {
                    rel_path
                },
                reason,
            );
            continue;
        }
        v.push((name, e.path(), is_dir));
    }
    v.sort_by(|a, b| {
        a.0.to_lowercase()
            .cmp(&b.0.to_lowercase())
            .then(a.0.cmp(&b.0))
    });
    Ok(v)
}

fn scan(
    dir: &Path,
    rel: &str,
    cat: &mut Catalog,
    existing: &HashMap<String, String>,
    excludes: &[String],
    consumed: Option<&Path>,
) -> Result<Vec<Node>> {
    let entries = sorted_entries(dir, rel, cat, excludes)?;
    let md_paths: HashMap<String, PathBuf> = entries
        .iter()
        .filter(|(n, p, d)| {
            !d && n.to_lowercase().ends_with(".md") && consumed != Some(p.as_path())
        })
        .map(|(n, p, _)| (fold(&n[..n.len() - 3]), p.clone()))
        .collect();
    let folder_names: HashSet<String> = entries
        .iter()
        .filter(|(_, _, d)| *d)
        .map(|(n, _, _)| fold(n))
        .collect();
    let mut out = Vec::new();
    for (name, path, is_dir) in entries {
        let rel_path = join_rel(rel, &name);
        if is_dir {
            let source_path = format!("{rel_path}/");
            let old = existing.get(&source_path);
            let id = old.cloned().unwrap_or_else(new_id);
            // A note next to the folder with the same name is its page.
            let local = sorted_entries(&path, &rel_path, cat, excludes)?;
            let inner_body = ["index.md", "readme.md"].iter().find_map(|body| {
                local
                    .iter()
                    .find(|(n, _, d)| !d && n.eq_ignore_ascii_case(body))
                    .map(|(_, p, _)| p.clone())
            });
            let md = md_paths.get(&fold(&name)).cloned().or(inner_body);
            let is_inner = md
                .as_ref()
                .is_some_and(|p| p.parent() == Some(path.as_path()));
            let source_file = md
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| join_rel(if is_inner { &rel_path } else { rel }, &n.to_string_lossy()))
                .unwrap_or_else(|| source_path.clone());
            if let Some(p) = &md {
                if is_inner {
                    let body_stem = p.file_stem().unwrap().to_string_lossy();
                    cat.add_page(&join_rel(&rel_path, &body_stem), &body_stem, &id);
                }
                cat.add_source(p, &id);
            }
            let children = scan(
                &path,
                &rel_path,
                cat,
                existing,
                excludes,
                if is_inner { md.as_deref() } else { None },
            )?;
            if md.is_none() && children.is_empty() {
                continue;
            }
            cat.add_page(&rel_path, &name, &id);
            out.push(Node {
                id,
                source_path,
                source_file,
                existing: old.is_some(),
                title: name,
                md,
                dir: if is_inner { rel_path } else { rel.to_string() },
                children,
            });
        } else if name.to_lowercase().ends_with(".md") {
            if consumed == Some(path.as_path()) {
                continue;
            }
            let stem = &name[..name.len() - 3];
            if folder_names.contains(&fold(stem)) {
                continue; // taken by the folder
            }
            let old = existing.get(&rel_path);
            let id = old.cloned().unwrap_or_else(new_id);
            cat.add_page(&join_rel(rel, stem), stem, &id);
            cat.add_source(&path, &id);
            out.push(Node {
                id,
                source_path: rel_path.clone(),
                source_file: rel_path,
                existing: old.is_some(),
                title: stem.to_string(),
                md: Some(path),
                dir: rel.to_string(),
                children: Vec::new(),
            });
        } else {
            cat.files.push((rel_path.clone(), path.clone()));
            cat.files_by_rel.insert(fold(&rel_path), path.clone());
            match cat.files_by_name.entry(fold(&name)) {
                Entry::Vacant(e) => {
                    e.insert(Some(path));
                }
                Entry::Occupied(mut e) => {
                    e.insert(None);
                }
            }
        }
    }
    Ok(out)
}

struct ImportResolver<'a> {
    cat: &'a Catalog,
    root: &'a Path,
    db: &'a Index,
    dir: String,
    source_file: String,
    stored: HashMap<PathBuf, String>,
    unresolved: u32,
    resolved: u32,
    unresolved_targets: Vec<ImportUnresolvedTarget>,
    error: Option<crate::VaultError>,
}

impl ImportResolver<'_> {
    fn resolve_page(&mut self, target: &str, markdown_link: bool) -> Option<String> {
        // A raw fragment is removed before decoding: `%23` can be part of
        // a real file name. Keep the original spelling in diagnostics/marks.
        let t = decode(target.trim().split('#').next().unwrap_or_default());
        let secret_target = secret_file(t.rsplit(['/', '\\']).next().unwrap_or(&t));
        let markdown = t.to_ascii_lowercase().ends_with(".md");
        let t = if markdown { &t[..t.len() - 3] } else { &t };
        let root = fold(&norm_path(t));
        let relative = fold(&join_rel(&self.dir, t));
        let requested = if markdown_link && !t.starts_with('/') {
            &relative
        } else {
            &root
        };
        // Catalog paths retain the `.md` extension, unlike the lookup aliases.
        // Reject an explicitly skipped source before trying a matching stem.
        let requested_source = if markdown {
            format!("{requested}.md")
        } else {
            requested.clone()
        };
        if self.cat.denied(&requested_source) || secret_target {
            return None;
        }
        let found = if markdown_link && !t.starts_with('/') {
            self.cat.by_rel.get(&relative)
        } else {
            self.cat
                .by_rel
                .get(&root)
                .or_else(|| self.cat.by_rel.get(&relative))
        }
        .cloned()
        .or_else(|| {
            if relative == ".." || relative.starts_with("../") {
                return None;
            }
            let stem = root.rsplit('/').next().unwrap_or(&root);
            self.cat.by_stem.get(stem).and_then(Clone::clone)
        });
        if found.is_some() {
            self.resolved += 1;
        }
        found
    }
}

impl Resolver for ImportResolver<'_> {
    fn preserve_wikilink_target(&self, _target: &str) -> bool {
        true
    }

    fn markdown_page(&mut self, target: &str) -> Option<String> {
        self.resolve_page(target, true)
    }

    fn page(&mut self, target: &str) -> Option<String> {
        self.resolve_page(target, false)
    }

    fn file(&mut self, path: &str) -> Option<String> {
        let p = decode(path.trim().split('#').next().unwrap_or_default());
        let relative = join_rel(&self.dir, &p);
        let root = norm_path(&p);
        // An explicitly skipped target must never resolve to an allowed
        // attachment elsewhere just because their basenames coincide.
        if relative == ".."
            || relative.starts_with("../")
            || self.cat.denied(&relative)
            || secret_file(p.rsplit(['/', '\\']).next().unwrap_or(&p))
        {
            return None;
        }
        let found = self
            .cat
            .files_by_rel
            .get(&fold(&relative))
            .or_else(|| {
                if self.cat.denied(&root) {
                    None
                } else {
                    self.cat.files_by_rel.get(&fold(&root))
                }
            })
            .or_else(|| {
                if self.cat.denied(&root) {
                    return None;
                }
                let name = p.rsplit(['/', '\\']).next().unwrap_or(&p);
                self.cat
                    .files_by_name
                    .get(&fold(name))
                    .and_then(Option::as_ref)
            })?
            .clone();
        if let Some(id) = self.stored.get(&found) {
            return Some(id.clone());
        }
        match store_attachment(self.root, self.db, &found) {
            Ok(id) => {
                self.stored.insert(found, id.clone());
                Some(id)
            }
            Err(e) => {
                self.error.get_or_insert(e);
                None
            }
        }
    }

    fn unresolved(&mut self, target: &str) {
        self.unresolved += 1;
        self.unresolved_targets.push(ImportUnresolvedTarget {
            source_file: self.source_file.clone(),
            target: target.to_string(),
        });
    }
}

// ------------------------------------------------------------ rescan

struct RescanResolver<'a> {
    links: HashMap<String, String>,
    db: &'a Index,
    root: &'a Path,
    pages_dir: PathBuf,
    error: Option<crate::VaultError>,
}

impl Resolver for RescanResolver<'_> {
    fn preserve_wikilink_target(&self, target: &str) -> bool {
        self.links
            .contains_key(&fold(&format!("[[{}]]", target.trim())))
    }

    fn markdown_page(&mut self, target: &str) -> Option<String> {
        self.page(target)
    }

    fn page(&mut self, target: &str) -> Option<String> {
        let decoded = decode(target.trim().split('#').next().unwrap_or_default());
        let t = if decoded.to_ascii_lowercase().ends_with(".md") {
            &decoded[..decoded.len() - 3]
        } else {
            &decoded
        };
        let basename = t.rsplit(['/', '\\']).next().unwrap_or(t);
        if let Some(id) = self.links.get(&fold(&format!("[[{}]]", target.trim()))) {
            if self.db.exists(id).unwrap_or(false) {
                return Some(id.clone());
            }
        }
        // Exported links can map either the exact original spelling or the
        // human-readable title. Older markdown supplied only a basename.
        for name in [target.trim(), decoded.as_str(), t, basename] {
            if let Some(id) = self.links.get(&fold(name)) {
                if self.db.exists(id).unwrap_or(false) {
                    return Some(id.clone());
                }
            }
        }
        for name in [t, basename] {
            if self.db.exists(name).unwrap_or(false) {
                return Some(name.to_string());
            }
            // `<slug>-<id>.md` links written by other tools.
            if let Some((_, id)) = name.rsplit_once('-') {
                if self.db.exists(id).unwrap_or(false) {
                    return Some(id.to_string());
                }
            }
        }
        self.db
            .id_by_title(t)
            .ok()
            .flatten()
            .or_else(|| self.db.id_by_title(basename).ok().flatten())
    }

    fn file(&mut self, path: &str) -> Option<String> {
        let p = decode(path.trim());
        let last = p.rsplit(['/', '\\']).next().unwrap_or(&p);
        if attach::is_attachment_id(last) && attach::blob_path(self.root, last).exists() {
            return Some(last.to_string());
        }
        // Retain rescan's existing boundary; import catalogs handle `..`
        // relative to source directories separately.
        let src = self.pages_dir.join(norm_path(&p).trim_start_matches("../"));
        if !src.is_file() {
            return None;
        }
        match store_attachment(self.root, self.db, &src) {
            Ok(id) => Some(id),
            Err(e) => {
                self.error.get_or_insert(e);
                None
            }
        }
    }
}

/// Frontmatter entries that are properties: display name -> value.
fn prop_entries(fm: &[(String, Yaml)]) -> HashMap<String, Yaml> {
    let role = fm_role(fm);
    fm.iter()
        .filter(|(k, _)| is_property_key(k, role.as_deref()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

impl Vault {
    /// Imports markdown beneath `parent`, reusing pages identified by that
    /// parent and their relative source path. Removed sources remain as pages.
    /// Folder bodies prefer an adjacent same-named note, then index.md, README.md.
    pub fn import_markdown(&self, dir: String, parent: Option<PageId>) -> Result<ImportReport> {
        self.import_markdown_with_excludes(dir, parent, Vec::new())
    }

    /// As `import_markdown`, with relative paths or `*`, `?`, `**` glob exclusions.
    /// Secret file patterns, dot entries and symbolic links are always skipped.
    pub fn import_markdown_with_excludes(
        &self,
        dir: String,
        parent: Option<PageId>,
        excludes: Vec<String>,
    ) -> Result<ImportReport> {
        Ok(self
            .import_markdown_reporting_unlinked(dir, parent, excludes)?
            .0)
    }

    /// As `import_markdown_with_excludes`, and also the non-markdown source
    /// files no imported page links to. The paths are relative to the import
    /// root, spelled as in the source, sorted, and never include skipped
    /// files. Only the reporting differs: what is imported is the same.
    pub fn import_markdown_reporting_unlinked(
        &self,
        dir: String,
        parent: Option<PageId>,
        excludes: Vec<String>,
    ) -> Result<(ImportReport, Vec<String>)> {
        let src = PathBuf::from(&dir);
        let db = lock(&self.inner.db);
        if let Some(p) = &parent {
            if db.in_trash(p)? != Some(false) {
                return Err(crate::error::not_found("page", p));
            }
        }
        let scope = parent.as_deref().unwrap_or("");
        let mut existing = HashMap::new();
        let mut seen = HashSet::new();
        let mut pending: Vec<String> = db
            .child_keys(parent.as_deref())?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        while let Some(id) = pending.pop() {
            if id == scope || !seen.insert(id.clone()) {
                continue;
            }
            pending.extend(db.child_keys(Some(&id))?.into_iter().map(|(id, _)| id));
            let props = self.inner.read(&id, |d| prop_map(&d.props()))?;
            if props.get(IMPORT_PARENT) != Some(&PropValue::Text(scope.to_string())) {
                continue;
            }
            if let Some(PropValue::Text(path)) = props.get(IMPORT_SOURCE_PATH) {
                if existing.insert(path.clone(), id).is_some() {
                    return Err(crate::VaultError::Invalid(format!(
                        "duplicate import source path: {path}"
                    )));
                }
            }
        }
        // A user may have moved an imported page outside this subtree. Its
        // original import identity still applies; use indexed properties to
        // find it without loading every unrelated page's document.
        for page in db.list(None, 0, u32::MAX)? {
            if page.id == scope || seen.contains(&page.id) {
                continue;
            }
            let props = prop_map(&page.properties);
            if props.get(IMPORT_PARENT) != Some(&PropValue::Text(scope.to_string())) {
                continue;
            }
            if let Some(PropValue::Text(path)) = props.get(IMPORT_SOURCE_PATH) {
                if existing.insert(path.clone(), page.id).is_some() {
                    return Err(crate::VaultError::Invalid(format!(
                        "duplicate import source path: {path}"
                    )));
                }
            }
        }
        let mut cat = Catalog::default();
        let nodes = scan(&src, "", &mut cat, &existing, &excludes, None)?;
        let mut selected = HashSet::new();
        let mut selected_ids = Vec::new();
        fn sources(nodes: &[Node], out: &mut HashSet<String>, ids: &mut Vec<PageId>) {
            for node in nodes {
                out.insert(node.source_path.clone());
                ids.push(node.id.clone());
                sources(&node.children, out, ids);
            }
        }
        sources(&nodes, &mut selected, &mut selected_ids);
        let mut orphaned_sources: Vec<String> = existing
            .keys()
            .filter(|path| !selected.contains(*path))
            .cloned()
            .collect();
        orphaned_sources.sort();
        let mut report = ImportReport {
            top_pages: nodes.iter().map(|n| n.id.clone()).collect(),
            orphaned: orphaned_sources.len() as u32,
            orphaned_sources,
            skipped_files: cat.skipped.clone(),
            ..ImportReport::default()
        };
        report
            .skipped_files
            .sort_by(|a, b| a.source_file.cmp(&b.source_file));
        let tx = db.write_tx()?;
        let mut r = ImportResolver {
            cat: &cat,
            root: &self.inner.root,
            db: &db,
            dir: String::new(),
            source_file: String::new(),
            stored: HashMap::new(),
            unresolved: 0,
            resolved: 0,
            unresolved_targets: Vec::new(),
            error: None,
        };
        self.import_nodes(&db, &nodes, parent.as_deref(), scope, &mut r, &mut report)?;
        if let Some(e) = r.error.take() {
            return Err(e);
        }
        report.pages = report.created;
        report.attachments = r.stored.values().collect::<HashSet<_>>().len() as u32;
        report.unresolved_links = r.unresolved;
        report.resolved_links = r.resolved;
        report.unresolved_targets = std::mem::take(&mut r.unresolved_targets);
        // The resolver reparses every page, so `stored` holds each file a page
        // links to, reused or new. Matching by path (not by hash) keeps two
        // files with identical bytes apart.
        let mut unlinked: Vec<String> = r
            .cat
            .files
            .iter()
            .filter(|(_, path)| !r.stored.contains_key(path))
            .map(|(rel, _)| rel.clone())
            .collect();
        unlinked.sort();
        tx.commit()?;
        if report.created + report.updated > 0 {
            self.inner.index_committed(&db, selected_ids);
        }
        // Newly imported embeds may have been rendered before their targets
        // existed. Only a changing import schedules work for linking pages.
        let mut linking = HashSet::new();
        if report.created + report.updated > 0 {
            for id in r.cat.by_source_id.values().flatten() {
                linking.extend(db.link_sources(id)?);
            }
        }
        drop(db);
        for src in linking {
            self.inner.rerender(&src);
        }
        if report.created + report.updated > 0 {
            self.inner.emit([Change::TreeChanged]);
        }
        Ok((report, unlinked))
    }

    fn import_nodes(
        &self,
        db: &Index,
        nodes: &[Node],
        parent: Option<&str>,
        scope: &str,
        r: &mut ImportResolver,
        report: &mut ImportReport,
    ) -> Result<()> {
        let mut anchors = Vec::with_capacity(nodes.len());
        for n in nodes {
            let key = if n.existing {
                let meta = self.inner.read(&n.id, |d| d.meta())?;
                (meta.parent.as_deref() == parent).then_some(meta.order)
            } else {
                None
            };
            anchors.push(key);
        }
        // With no reused siblings append after existing, hand-created pages.
        // Otherwise preserve every compatible imported key and fit additions
        // between them; deleting a source does not re-key its surviving siblings.
        let mut last = if anchors.iter().all(Option::is_none) {
            db.last_child_key(parent)?
        } else {
            None
        };
        for (i, n) in nodes.iter().enumerate() {
            let next = anchors[i + 1..]
                .iter()
                .flatten()
                .find(|k| last.as_ref().is_none_or(|p| *k > p));
            let key = match &anchors[i] {
                Some(key) if last.as_ref().is_none_or(|p| key > p) => key.clone(),
                _ => order::between(last.as_deref(), next.map(String::as_str)),
            };
            last = Some(key.clone());
            self.import_node(db, n, parent, &key, scope, r, report)?;
            self.import_nodes(db, &n.children, Some(&n.id), scope, r, report)?;
        }
        Ok(())
    }

    fn import_node(
        &self,
        db: &Index,
        n: &Node,
        parent: Option<&str>,
        key: &str,
        scope: &str,
        r: &mut ImportResolver,
        report: &mut ImportReport,
    ) -> Result<()> {
        let text = match &n.md {
            Some(p) => String::from_utf8_lossy(&fs::read(p)?).into_owned(),
            None => String::new(),
        };
        let (fm, body) = split_frontmatter(&text);
        let mut type_data = fm_type_data(&fm, body);
        r.dir = n.dir.clone();
        r.source_file = n.source_file.clone();
        let title = fm_text(&fm, "title").unwrap_or_else(|| n.title.clone());
        let mut parsed = if type_data
            .as_ref()
            .is_some_and(|td| td.contains_key(MARKDOWN_KEY))
        {
            Vec::new()
        } else {
            parse_blocks(body, r)
        };
        remap_embeds(&mut parsed, &r.cat.by_source_id);
        if let Some(data) = type_data.as_mut() {
            remap_type_data(data, &r.cat.by_source_id);
        }
        fn counts(blocks: &[Block], report: &mut ImportReport) {
            for block in blocks {
                if block.kind.as_str() == "table" {
                    report.tables += 1;
                }
                if block.kind == BlockKind::Image {
                    report.images += 1;
                }
                counts(&block.children, report);
            }
        }
        counts(&parsed, report);
        let mut props = import_props(&fm, r);
        props.push((
            IMPORT_SOURCE_PATH.to_string(),
            PropValue::Text(n.source_path.clone()),
        ));
        props.push((
            IMPORT_PARENT.to_string(),
            PropValue::Text(scope.to_string()),
        ));
        if let Some(e) = r.error.take() {
            return Err(e);
        }
        let page_type = fm_page_type(&fm).unwrap_or_else(|| PAGE_TYPE_DOCUMENT.to_string());
        let icon = fm_text(&fm, "icon");
        let role = fm_role(&fm);
        if !n.existing {
            let blocks = fresh_ids(&parsed);
            let mut doc =
                PageDoc::new_page(self.inner.actor_peer(), &n.id, &title, parent, now_ms())?;
            doc.set_meta_str("order", Some(key))?;
            doc.set_meta_str("icon", icon.as_deref())?;
            doc.set_page_type(&page_type)?;
            if let Some(role) = role.as_deref() {
                doc.set_meta_str("role", Some(role))?;
            }
            if let Some(td) = type_data {
                doc.type_data_reconcile(&td)?;
            }
            doc.apply(&insert_ops(&blocks))?;
            for (k, v) in &props {
                doc.set_prop(k, v)?;
            }
            self.inner.install(db, doc)?;
            report.created += 1;
            return Ok(());
        }
        let (current, meta, old_props) = self
            .inner
            .read(&n.id, |d| (d.blocks(), d.meta(), d.props()))?;
        assign_ids(&current, &mut parsed);
        let ops = diff_ops(&current, &parsed);
        let wanted: HashMap<_, _> = props.into_iter().collect();
        let old = prop_map(&old_props);
        let changed_props: Vec<_> = wanted
            .iter()
            .filter(|(k, v)| old.get(*k) != Some(*v))
            .collect();
        let removed_props: Vec<_> = old
            .keys()
            .filter(|k| !k.starts_with('$') && !wanted.contains_key(*k))
            .collect();
        let wanted_data = type_data.unwrap_or_default();
        let data_changed = meta_type_data(&meta.type_data) != Value::Object(wanted_data.clone());
        let tree_changed =
            meta.parent.as_deref() != parent || meta.order != key || meta.title != title;
        let role_changed = role.as_deref().is_some_and(|r| meta.role.as_deref() != Some(r));
        let changed = !ops.is_empty()
            || tree_changed
            || meta.icon != icon
            || meta.page_type != page_type
            || data_changed
            || role_changed
            || !changed_props.is_empty()
            || !removed_props.is_empty();
        if !changed {
            report.unchanged += 1;
            return Ok(());
        }
        self.inner.edit(&n.id, |doc| {
            if meta.title != title {
                doc.set_title(&title)?;
            }
            doc.set_meta_str("parent", parent)?;
            doc.set_meta_str("order", Some(key))?;
            doc.set_meta_str("icon", icon.as_deref())?;
            doc.set_page_type(&page_type)?;
            if let Some(role) = role.as_deref().filter(|_| role_changed) {
                doc.set_meta_str("role", Some(role))?;
            }
            if data_changed {
                doc.type_data_reconcile(&wanted_data)?;
            }
            if !ops.is_empty() {
                doc.apply(&ops)?;
            }
            for (k, v) in &changed_props {
                doc.set_prop(k, v)?;
            }
            for k in &removed_props {
                doc.remove_prop(k)?;
            }
            Ok(())
        })?;
        self.inner.sync_meta(db, &n.id)?;
        if meta.title != title {
            for source in db.link_sources(&n.id)? {
                self.inner.rerender(&source);
            }
        }
        report.updated += 1;
        Ok(())
    }

    /// Reads markdown files in `pages/` changed outside the app (their hash
    /// differs from the one last written) and applies the differences to the
    /// pages' documents: title, icon, properties and blocks (unchanged
    /// blocks keep their ids). A new `.md` file without a known id becomes a
    /// new page; a missing file is written again. Flushes first. Returns the
    /// number of pages changed or created.
    pub fn rescan(&self) -> Result<u32> {
        self.flush()?;
        let pages_dir = self.inner.pages_dir();
        let db = lock(&self.inner.db);
        let indexed: HashMap<String, (String, Option<String>)> = db
            .md_files()?
            .into_iter()
            .filter_map(|(id, md, hash)| md.map(|m| (m, (id, hash))))
            .collect();
        let mut names: Vec<String> = fs::read_dir(&pages_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_lowercase().ends_with(".md") && !n.starts_with('.'))
            .collect();
        names.sort();
        let mut changed = 0;
        let mut seen: HashSet<String> = HashSet::new();
        for name in names {
            let bytes = fs::read(pages_dir.join(&name))?;
            let hash = attach::sha256_hex(&bytes);
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if let Some((id, h)) = indexed.get(&name) {
                seen.insert(id.clone());
                if h.as_deref() != Some(hash.as_str())
                    && self.update_from_md(&db, id, &text, &name, &hash, &pages_dir)?
                {
                    changed += 1;
                }
                continue;
            }
            let (fm, _) = split_frontmatter(&text);
            match fm_text(&fm, "id") {
                Some(fid) if db.exists(&fid)? => {
                    // A known page's file under another name (renamed or
                    // copied); pages in the trash are left alone.
                    if db.in_trash(&fid)? == Some(false) && seen.insert(fid.clone()) {
                        self.update_from_md(&db, &fid, &text, &name, &hash, &pages_dir)?;
                        changed += 1;
                    }
                }
                _ => {
                    self.page_from_md(&db, &text, &name, &hash, None, &pages_dir)?;
                    changed += 1;
                }
            }
        }
        for (id, md, _) in db.md_files()? {
            // Files of mounted folders (absolute paths): `rescan_mounts`.
            let missing = match &md {
                Some(m) if std::path::Path::new(m).is_absolute() => continue,
                Some(m) => !pages_dir.join(m).exists(),
                None => self.inner.mount_folder_of(&db, &id).is_none(),
            };
            if missing {
                self.inner.rerender(&id);
            }
        }
        drop(db);
        Ok(changed + self.rescan_mounts()?)
    }

    /// A new page from a markdown file `name` (in `pages/`, or an absolute
    /// path in a mounted folder). `place`: its parent and title, instead of
    /// the frontmatter's. `dir`: where relative image paths start.
    pub(crate) fn page_from_md(
        &self,
        db: &Index,
        text: &str,
        name: &str,
        hash: &str,
        place: Option<(&str, &str)>,
        dir: &std::path::Path,
    ) -> Result<()> {
        let (fm, body) = split_frontmatter(text);
        let type_data = fm_type_data(&fm, body);
        let mut r = RescanResolver {
            links: links_map(&fm),
            db,
            root: &self.inner.root,
            pages_dir: dir.to_path_buf(),
            error: None,
        };
        let stem = name.strip_suffix(".md").unwrap_or(name);
        let (title, parent) = match place {
            Some((p, t)) => (t.to_string(), Some(p.to_string())),
            None => (
                fm_text(&fm, "title").unwrap_or_else(|| stem.to_string()),
                fm_text(&fm, "parent").filter(|p| db.in_trash(p).ok() == Some(Some(false))),
            ),
        };
        let blocks = if type_data
            .as_ref()
            .is_some_and(|td| td.contains_key(MARKDOWN_KEY))
        {
            Vec::new()
        } else {
            fresh_ids(&parse_blocks(body, &mut r))
        };
        let props = import_props(&fm, &mut r);
        if let Some(e) = r.error.take() {
            return Err(e);
        }
        let key = order::between(db.last_child_key(parent.as_deref())?.as_deref(), None);
        let mut doc = PageDoc::new_page(
            self.inner.peer,
            &new_id(),
            &title,
            parent.as_deref(),
            now_ms(),
        )?;
        doc.set_meta_str("order", Some(&key))?;
        doc.set_meta_str("icon", fm_text(&fm, "icon").as_deref())?;
        doc.set_page_type(&fm_page_type(&fm).unwrap_or_else(|| PAGE_TYPE_DOCUMENT.to_string()))?;
        if let Some(role) = fm_role(&fm) {
            doc.set_meta_str("role", Some(role.as_str()))?;
        }
        if let Some(td) = type_data {
            doc.type_data_reconcile(&td)?;
        }
        doc.apply(&insert_ops(&blocks))?;
        let schema = match &parent {
            Some(p) => db.schema_of(p)?,
            None => None,
        };
        for (k, v) in &props {
            if schema.is_none() {
                doc.set_prop(k, v)?;
            }
        }
        // The file becomes the page's markdown file (the writer renames it).
        self.inner
            .install_with_md(db, doc, Some((name.to_string(), hash.to_string())))?;
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    /// Applies an edited markdown file to its page. Returns whether the page
    /// changed. `dir`: where relative image paths start.
    pub(crate) fn update_from_md(
        &self,
        db: &Index,
        id: &str,
        text: &str,
        name: &str,
        hash: &str,
        dir: &std::path::Path,
    ) -> Result<bool> {
        let (fm, body) = split_frontmatter(text);
        let file_type_data = fm_type_data(&fm, body);
        let has_type_markdown = file_type_data
            .as_ref()
            .is_some_and(|td| td.contains_key(MARKDOWN_KEY));
        let schema = match db.parent_of(id)?.flatten() {
            Some(p) => db.schema_of(&p)?,
            None => None,
        };
        let mut r = RescanResolver {
            links: links_map(&fm),
            db,
            root: &self.inner.root,
            pages_dir: dir.to_path_buf(),
            error: None,
        };
        // The base is the document as of the last time the file was written:
        // the outside edits are the differences between the file and the base
        // rendered again (so formatting the parser does not round-trip exactly
        // does not turn into edits). If the page changed in the app since,
        // the edits are made on a fork at the base and merged by Loro, so
        // neither side's changes are lost. Without a known base (after a
        // reindex) the current document is the base.
        let base = db.md_version(id)?;
        let (cur_blocks, meta, cur_props, mut fork) = self.inner.read(id, |d| -> Result<_> {
            if let Some(f) = base.as_deref().and_then(|b| Frontiers::decode(b).ok()) {
                let vv = d.doc.oplog_vv();
                if f != d.doc.oplog_frontiers() && f.iter().all(|x| vv.includes_id(x)) {
                    let mut fk = d.fork_at(&f)?;
                    return Ok((fk.blocks(), fk.meta(), fk.props(), Some(fk)));
                }
            }
            Ok((d.blocks(), d.meta(), d.props(), None))
        })??;
        let titles = |pid: &str| db.title_of(pid).ok().flatten();
        let ctx = RenderCtx {
            titles: &titles,
            schema: schema.as_deref(),
        };
        let current = markdown::render(&meta, &cur_props, &cur_blocks, &ctx);
        let (cur_fm, cur_body) = split_frontmatter(&current);

        let stored_type_data = meta_type_data(&meta.type_data);
        let ops = if has_type_markdown {
            // A marked body is verbatim type data, never editable blocks.
            diff_ops(&cur_blocks, &[])
        } else if cur_body.trim_end() != body.trim_end()
            || (cur_blocks.is_empty() && stored_type_data.get(MARKDOWN_KEY).is_some())
        {
            let mut parsed = parse_blocks(body, &mut r);
            assign_ids(&cur_blocks, &mut parsed);
            diff_ops(&cur_blocks, &parsed)
        } else {
            Vec::new()
        };
        let title = fm_text(&fm, "title").filter(|t| *t != meta.title);
        let icon = fm_text(&fm, "icon");
        let icon_changed = icon != fm_text(&cur_fm, "icon");
        // A file that declares no type leaves the page's type alone.
        let page_type = fm_page_type(&fm).filter(|t| *t != meta.page_type);
        // A file without `type-data` leaves the add-on data alone, as above.
        let type_data = file_type_data.filter(|t| Value::Object(t.clone()) != stored_type_data);
        // A file without a known `role:` leaves the page's role alone.
        let role = fm_role(&fm).filter(|r| meta.role.as_deref() != Some(r.as_str()));

        let file_props = prop_entries(&fm);
        let cur_entries = prop_entries(&cur_fm);
        let existing = prop_map(&cur_props);
        let key_of = |name: &str| -> Option<(String, Option<&PropDef>)> {
            let n = name.strip_prefix("prop:").unwrap_or(name);
            match &schema {
                Some(s) => {
                    let f = fold(n);
                    s.iter()
                        .find(|d| fold(&d.name) == f)
                        .map(|d| (d.key.clone(), Some(d)))
                }
                None => Some((n.to_string(), None)),
            }
        };
        let mut prop_changes: Vec<(String, PropValue)> = Vec::new();
        for (name, y) in &file_props {
            if cur_entries.get(name) == Some(y) {
                continue;
            }
            let Some((key, def)) = key_of(name) else {
                continue;
            };
            let v = match (def, existing.get(&key).and_then(|v| v.kind())) {
                (Some(d), _) => parse_typed(y, d.kind, Some(d), &mut r),
                (None, Some(kind)) => parse_typed(y, kind, None, &mut r),
                (None, None) => infer(y, &mut r),
            };
            prop_changes.push((key, v));
        }
        // A property the file still names under another spelling (`role: x`
        // for `prop:role`) is set above, so it is not removed here.
        let file_keys: HashSet<String> = file_props
            .keys()
            .filter_map(|name| key_of(name).map(|(key, _)| key))
            .collect();
        for name in cur_entries.keys() {
            if !file_props.contains_key(name) {
                if let Some((key, _)) = key_of(name) {
                    if !file_keys.contains(&key) {
                        prop_changes.push((key, PropValue::Empty));
                    }
                }
            }
        }
        if let Some(e) = r.error.take() {
            return Err(e);
        }
        let unchanged = ops.is_empty()
            && title.is_none()
            && !icon_changed
            && page_type.is_none()
            && type_data.is_none()
            && role.is_none()
            && prop_changes.is_empty();
        let change = |d: &mut PageDoc| -> Result<()> {
            if !ops.is_empty() {
                d.apply(&ops)?;
            }
            if let Some(t) = &title {
                d.set_title(t)?;
            }
            if icon_changed {
                d.set_meta_str("icon", icon.as_deref())?;
            }
            if let Some(t) = &page_type {
                d.set_page_type(t)?;
            }
            if let Some(r) = role.as_deref() {
                d.set_meta_str("role", Some(r))?;
            }
            if let Some(td) = &type_data {
                d.type_data_reconcile(td)?;
            }
            for (k, v) in &prop_changes {
                if *v == PropValue::Empty {
                    d.remove_prop(k)?;
                } else {
                    d.set_prop(k, v)?;
                }
            }
            Ok(())
        };
        // The outside edits made on the fork, as Loro updates.
        let mut fork_updates = None;
        if let Some(fk) = fork.as_mut() {
            if !unchanged {
                let vv = fk.doc.oplog_vv();
                change(fk)?;
                fk.register_peer(&self.inner.device)?;
                fk.doc.commit();
                fork_updates = Some(fk.doc.export(ExportMode::updates(&vv))?);
            }
        }
        // Record the file first, so the writer replaces it once the edit is
        // flushed (it leaves files alone whose hash is not the recorded one).
        // The file matches the fork, or the document once the edits are in.
        let version = match &fork {
            Some(fk) => Some(fk.version()),
            None if unchanged => Some(self.inner.read(id, |d| d.version())?),
            None => None,
        };
        db.set_md(id, name, hash, version.as_deref())?;
        if unchanged {
            return Ok(false);
        }
        match fork_updates {
            Some(bytes) => self.inner.edit(id, |d| d.import_updates(&bytes))?,
            None => self.inner.edit(id, change)?,
        }
        self.inner.sync_meta(db, id)?;
        if title.is_some() {
            for src in db.link_sources(id)? {
                self.inner.rerender(&src);
            }
            self.inner.emit([Change::TreeChanged]);
        }
        Ok(true)
    }
}

/// The frontmatter `links` map: folded name -> page id.
fn links_map(fm: &[(String, Yaml)]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some((_, Yaml::Map(m))) = fm.iter().find(|(k, _)| k == "links") {
        for (name, v) in m {
            if let Some(id) = v.to_text() {
                out.insert(fold(name), id);
            }
        }
    }
    out
}

#[cfg(test)]
mod page_type_tests {
    use super::*;

    fn fm(text: &str) -> Vec<(String, Yaml)> {
        split_frontmatter(text).0
    }

    #[test]
    fn the_declared_type_is_read_from_front_matter() {
        let typed = |s: &str| fm_page_type(&fm(s));
        assert_eq!(
            typed("---\ntype: \"database\"\n---\n").as_deref(),
            Some("database")
        );
        assert_eq!(typed("---\ntype: kanban\n---\n").as_deref(), Some("kanban"));
        // The old flag reads as a database; a stored type wins over it.
        assert_eq!(
            typed("---\ncollection: true\n---\n").as_deref(),
            Some("database")
        );
        assert_eq!(
            typed("---\ncollection: true\ntype: document\n---\n").as_deref(),
            Some("document")
        );
        // A file that says nothing about its type declares none.
        assert_eq!(typed("---\ncollection: false\n---\n"), None);
        assert_eq!(typed("---\ntitle: \"x\"\n---\n"), None);
        assert_eq!(typed("no front matter"), None);
    }
}
