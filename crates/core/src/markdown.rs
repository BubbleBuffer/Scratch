//! Markdown + YAML frontmatter written from a page's document. The format is
//! read back by `mdparse` (for `rescan` and `import_markdown`), so every
//! construct written here parses back to the same blocks:
//!
//! - Page links are Obsidian wikilinks, `[[Title]]` or `[[Title|text]]`; the
//!   frontmatter's `links` map keeps title -> page id, and each target as
//!   written (`Home#Intro`, `Notes/Home.md`) -> page id. Titles that are empty,
//!   ambiguous or unusable in a wikilink are written as the page id.
//! - `LinkToPage` blocks are `![[Title]]`, images `![caption](../.vault/blobs/..)`,
//!   files `[name](../.vault/blobs/..)`, bookmarks `<url> caption`, empty
//!   paragraphs `<br>`.
//! - What markdown has no syntax for is kept in an HTML comment at the end
//!   of the block's first line, which viewers do not show: toggles are list
//!   items or headings with `<!-- toggle -->`, callouts quotes with
//!   `<!-- callout icon=💡 -->`, block colours `<!-- color=red -->`. Column
//!   lists, columns and tables of contents are a line holding only the
//!   comment (`<!-- columns -->`, `<!-- column -->`, `<!-- toc -->`), the
//!   columns' blocks indented under them.
//! - Unknown kinds and chat references use a readable blockquote with
//!   `<!-- block=<hex-json> -->` on its first line. The payload preserves
//!   the raw kind, target, fallback line count and checked state.
//! - Underline is `<u>`, text colours `<span data-color="red">`, date
//!   mentions `<time datetime="2026-10-01">`.
//! - Query cards are fenced `query` blocks; inline query chips are
//!   `<span data-query="…">label</span>` with an escaped query attribute.
//! - View cards are fenced `view` JSON objects, with an optional `page` key
//!   carrying the target page id alongside the stored settings and source.
//! - Tables are written line by line as they were read, so their text
//!   round-trips. Code fences open with the block's info string (`rust`);
//!   an unlabelled fence has none.
//! - A resolved `.md` link is written as the link the user wrote
//!   (`[text](Notes/Home.md#Intro)`), and a resolved wikilink with a target
//!   as written (`[[Home#Intro|text]]`): each is a page link and its target
//!   over the same text, and only the target is written.
//! - Properties are top-level frontmatter keys by display name.
//! - The page role is the `role:` key (`journal`, `calendars`, `nodes`,
//!   `feedback`). A property named `role` is written as `prop:role`, and an
//!   unknown `role:` value is read back as that property.

use std::collections::{BTreeMap, HashMap};

use crate::doc::{embed_title, normalize_marks, PageMeta};
use crate::mdparse::{is_web, md_target};
use crate::types::{Block, BlockKind, Mark, MarkKind, PropDef, PropValue, Property};

/// Frontmatter keys the core writes itself; a property with one of these
/// names is written as `prop:<name>`.
pub(crate) const RESERVED_KEYS: &[&str] = &[
    "id",
    "title",
    "parent",
    "created",
    "updated",
    "icon",
    "cover",
    "favorite",
    "collection",
    "type",
    "type-data",
    "links",
    "role",
];

/// The frontmatter key of the page role.
pub(crate) const ROLE_KEY: &str = "role";

/// Page roles a markdown file may declare as `role:`. Any other value is an
/// ordinary property named `role`.
pub const ROLES: &[&str] = &["journal", "calendars", "nodes", "feedback"];

/// What the renderer needs from outside the page.
pub(crate) struct RenderCtx<'a> {
    /// Title of a page id (None if unknown).
    pub titles: &'a dyn Fn(&str) -> Option<String>,
    /// The schema of the page's collection, if the page is a row.
    pub schema: Option<&'a [PropDef]>,
}

fn no_titles(_: &str) -> Option<String> {
    None
}

impl RenderCtx<'static> {
    pub fn none() -> RenderCtx<'static> {
        RenderCtx {
            titles: &no_titles,
            schema: None,
        }
    }
}

/// `<slug>-<id>.md`
pub(crate) fn file_name(title: &str, id: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.chars().count() >= 50 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "untitled" } else { slug };
    format!("{slug}-{id}.md")
}

/// The file name stem of a page in a mounted folder: its title, with the
/// characters Windows does not allow in file names replaced ("Untitled"
/// when empty).
pub(crate) fn plain_stem(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if (c as u32) < 0x20 => ' ',
            c => c,
        })
        .take(120)
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    let s = s.trim_start().to_string();
    if s.is_empty() {
        return "Untitled".into();
    }
    let upper = s.to_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    let reserved = matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("{s}_")
    } else {
        s
    }
}

/// `render` for a file in a mounted folder: the frontmatter keeps only
/// what the user wrote there (properties, icon, type and the `links` map); the
/// id, title (the file name), parent and times stay in the vault. No
/// frontmatter at all when nothing is left.
pub(crate) fn render_plain(
    meta: &PageMeta,
    props: &[Property],
    blocks: &[Block],
    ctx: &RenderCtx,
) -> String {
    let full = render(meta, props, blocks, ctx);
    let Some(rest) = full.strip_prefix("---\n") else {
        return full;
    };
    let Some(end) = rest.find("\n---\n") else {
        return full;
    };
    let (fm, body) = (&rest[..end + 1], &rest[end + 5..]);
    let mut kept = String::new();
    let mut skip = false;
    for line in fm.lines() {
        if !line.starts_with(' ') {
            let key = line.split(':').next().unwrap_or("");
            skip = matches!(
                key,
                "id" | "title" | "parent" | "created" | "updated" | "favorite" | "collection"
            );
        }
        if !skip {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if kept.is_empty() {
        // No frontmatter left: no blank line before the text either.
        body.trim_start_matches('\n').to_string()
    } else {
        format!("---\n{kept}---\n{body}")
    }
}

pub(crate) fn yaml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A frontmatter key: plain when it is simple (letters, digits, spaces,
/// `_`, `-`, `.`), else a quoted string.
pub(crate) fn yaml_key(s: &str) -> String {
    let simple = s
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        && !s.ends_with(' ')
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no"
        );
    if simple {
        s.to_string()
    } else {
        yaml_str(s)
    }
}

/// Milliseconds since the epoch as an ISO 8601 UTC timestamp.
pub(crate) fn iso8601(ms: i64) -> String {
    let (y, m, d) = civil_from_ms(ms);
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let sod = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        sod / 3600,
        (sod / 60) % 60,
        sod % 60
    )
}

/// `YYYY-MM-DD` of a UTC time.
pub(crate) fn iso_date(ms: i64) -> String {
    let (y, m, d) = civil_from_ms(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

pub(crate) fn civil_from_ms(ms: i64) -> (i64, i64, i64) {
    let days = ms.div_euclid(1000).div_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Days since the epoch of a civil date (Hinnant's days_from_civil).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Blob path as written in markdown, relative to `pages/`.
pub(crate) fn blob_link(id: &str) -> String {
    if id.len() >= 2 {
        format!("../.vault/blobs/{}/{id}", &id[..2])
    } else {
        String::new()
    }
}

fn format_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

/// A date value as written in frontmatter.
pub(crate) fn date_text(start: i64, end: Option<i64>, has_time: bool) -> String {
    let one = |ms: i64| if has_time { iso8601(ms) } else { iso_date(ms) };
    match end {
        Some(e) => format!("{} → {}", one(start), one(e)),
        None => one(start),
    }
}

/// Link targets of the page and how each is written: title, or the id when
/// the title cannot be used (unknown, empty, duplicate, or with `[]|#^`).
pub(crate) fn link_names(
    blocks: &[Block],
    props: &[Property],
    ctx: &RenderCtx,
) -> HashMap<String, String> {
    fn walk(b: &[Block], out: &mut Vec<String>) {
        for x in b {
            for m in &x.marks {
                if let MarkKind::PageLink(id) = &m.kind {
                    out.push(id.clone());
                }
            }
            if matches!(x.kind, BlockKind::LinkToPage | BlockKind::Database) {
                if let Some(t) = &x.target {
                    out.push(t.clone());
                }
            }
            walk(&x.children, out);
        }
    }
    let mut ids = Vec::new();
    walk(blocks, &mut ids);
    for p in props {
        if let PropValue::Relation(r) = &p.value {
            ids.extend(r.iter().cloned());
        }
    }
    let mut names: HashMap<String, String> = HashMap::new();
    let mut used: HashMap<String, String> = HashMap::new();
    for id in ids {
        if names.contains_key(&id) {
            continue;
        }
        let name = match (ctx.titles)(&id) {
            Some(t)
                if !t.trim().is_empty()
                    && t.trim() == t
                    && !t.contains(['[', ']', '|', '#', '^', '\n'])
                    && !used.contains_key(&t.to_lowercase()) =>
            {
                used.insert(t.to_lowercase(), id.clone());
                t
            }
            _ => id.clone(),
        };
        names.insert(id, name);
    }
    names
}

fn yaml_value(v: &PropValue, def: Option<&PropDef>, names: &HashMap<String, String>) -> String {
    let opt = |id: &str| -> String {
        def.and_then(|d| d.options.iter().find(|o| o.id == id))
            .map(|o| o.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match v {
        PropValue::Empty => "null".into(),
        PropValue::Text(s) | PropValue::Url(s) => yaml_str(s),
        PropValue::Number(x) => format_number(*x),
        PropValue::Checkbox(b) => b.to_string(),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => yaml_str(&date_text(*start_ms, *end_ms, *has_time)),
        PropValue::Select(id) | PropValue::Status(id) => yaml_str(&opt(id)),
        PropValue::MultiSelect(ids) => format!(
            "[{}]",
            ids.iter()
                .map(|i| yaml_str(&opt(i)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PropValue::Files(ids) => format!(
            "[{}]",
            ids.iter()
                .map(|i| yaml_str(&blob_link(i)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PropValue::Error(_) => "null".into(),
        PropValue::Relation(ids) => format!(
            "[{}]",
            ids.iter()
                .map(|i| yaml_str(&format!(
                    "[[{}]]",
                    names.get(i).cloned().unwrap_or_else(|| i.clone())
                )))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The frontmatter key of a property: its display name (schema) or key.
pub(crate) fn prop_key_name(key: &str, schema: Option<&[PropDef]>) -> String {
    let name = schema
        .and_then(|s| s.iter().find(|d| d.key == key))
        .map(|d| d.name.clone())
        .unwrap_or_else(|| key.to_string());
    if name.is_empty() || RESERVED_KEYS.contains(&name.as_str()) || name.starts_with("prop:") {
        format!("prop:{name}")
    } else {
        name
    }
}

pub(crate) fn render(
    meta: &PageMeta,
    props: &[Property],
    blocks: &[Block],
    ctx: &RenderCtx,
) -> String {
    let names = link_names(blocks, props, ctx);
    let mut type_data = crate::type_data::meta_type_data(&meta.type_data);
    let type_markdown = type_data
        .get(crate::type_data::MARKDOWN_KEY)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if type_markdown.is_some() {
        type_data[crate::type_data::MARKDOWN_KEY] = serde_json::Value::Bool(true);
    }
    let mut out = String::with_capacity(256 + blocks.len() * 64);
    out.push_str("---\n");
    out.push_str(&format!("id: {}\n", meta.id));
    out.push_str(&format!("title: {}\n", yaml_str(&meta.title)));
    match &meta.parent {
        Some(p) => out.push_str(&format!("parent: {p}\n")),
        None => out.push_str("parent: null\n"),
    }
    out.push_str(&format!("created: {}\n", iso8601(meta.created)));
    out.push_str(&format!("updated: {}\n", iso8601(meta.updated)));
    if let Some(i) = &meta.icon {
        out.push_str(&format!("icon: {}\n", yaml_str(i)));
    }
    if let Some(c) = &meta.cover {
        out.push_str(&format!("cover: {c}\n"));
    }
    if meta.favorite.is_some() {
        out.push_str("favorite: true\n");
    }
    if meta.page_type != crate::types::PAGE_TYPE_DOCUMENT {
        out.push_str(&format!("type: {}\n", yaml_str(&meta.page_type)));
    }
    if let Some(role) = &meta.role {
        // Known roles are plain words; anything else is quoted as text.
        let value = if ROLES.contains(&role.as_str()) {
            role.clone()
        } else {
            yaml_str(role)
        };
        out.push_str(&format!("{ROLE_KEY}: {value}\n"));
    }
    // One line of raw JSON; the front matter reader takes that line back as
    // text (`type_data::split_frontmatter`), so its ` #` and quotes survive.
    if !meta.type_data.is_empty() {
        out.push_str(&format!("{}: {}\n", crate::type_data::FM_KEY, type_data));
    }
    // Schema order first, then the rest by key.
    let mut ordered: Vec<&Property> = Vec::new();
    if let Some(s) = ctx.schema {
        for d in s {
            ordered.extend(props.iter().filter(|p| p.key == d.key));
        }
    }
    let rest: Vec<&Property> = props
        .iter()
        .filter(|p| !ordered.iter().any(|o| o.key == p.key))
        .collect();
    ordered.extend(rest);
    for p in ordered {
        if p.value == PropValue::Empty {
            continue;
        }
        let def = ctx.schema.and_then(|s| s.iter().find(|d| d.key == p.key));
        out.push_str(&format!(
            "{}: {}\n",
            yaml_key(&prop_key_name(&p.key, ctx.schema)),
            yaml_value(&p.value, def, &names)
        ));
    }
    // Titles, then the targets as written (`Home#Intro`, `Notes/Home.md`) of
    // the page links that have one: each maps to its page, whatever the
    // title is now.
    let mut links: BTreeMap<String, String> = names
        .iter()
        .filter(|(id, name)| id != name)
        .map(|(id, name)| (name.clone(), id.clone()))
        .collect();
    for (target, id) in written_targets(blocks) {
        if !is_web(&target) {
            // Distinguish preserved import targets from ordinary page mentions.
            links.insert(format!("[[{target}]]"), id.clone());
        }
        links.insert(target, id);
    }
    if !links.is_empty() {
        out.push_str("links:\n");
        for (name, id) in &links {
            out.push_str(&format!("  {}: {id}\n", yaml_key(name)));
        }
    }
    out.push_str("---\n");
    match type_markdown {
        Some(body)
            if blocks.iter().all(|block| {
                block.kind == BlockKind::Paragraph
                    && block.text.is_empty()
                    && block.children.is_empty()
            }) =>
        {
            out.push_str(&body)
        }
        _ => render_blocks(blocks, "", &names, ctx.titles, &mut out),
    }
    out
}

fn is_list(k: &BlockKind) -> bool {
    matches!(
        k,
        BlockKind::Bulleted | BlockKind::Numbered | BlockKind::Todo
    )
}

/// Escapes what would start a block at the beginning of a line.
fn escape_line_start(line: &str) -> String {
    let b = line.as_bytes();
    if matches!(b.first(), Some(b'#' | b'-' | b'+' | b'=')) {
        return format!("\\{line}");
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && matches!(b.get(digits), Some(b'.' | b')')) {
        return format!("{}\\{}", &line[..digits], &line[digits..]);
    }
    line.to_string()
}

pub(crate) fn render_blocks(
    blocks: &[Block],
    indent: &str,
    names: &HashMap<String, String>,
    titles: &dyn Fn(&str) -> Option<String>,
    out: &mut String,
) {
    let mut number = 0u32;
    let mut prev: Option<&BlockKind> = None;
    for b in blocks {
        number = if b.kind == BlockKind::Numbered {
            if prev == Some(&BlockKind::Numbered) {
                number.saturating_add(1)
            } else {
                b.numbered_start.unwrap_or(1)
            }
        } else {
            0
        };
        // Blank line between blocks, except inside a run of list items and
        // before a list item that is the first child of its parent.
        let tight = match prev {
            Some(p) => is_list(p) && is_list(&b.kind),
            None => !indent.is_empty() && is_list(&b.kind),
        };
        if !tight {
            out.push('\n');
        }
        prev = Some(&b.kind);
        let inline = || render_inline_with(&b.text, &b.marks, names);
        let target = b.target.clone().unwrap_or_default();
        let meta = block_meta(b);
        let (marker, text) = match &b.kind {
            BlockKind::Toggle => ("- ".into(), inline()),
            BlockKind::ToggleHeading1 => ("# ".into(), inline()),
            BlockKind::ToggleHeading2 => ("## ".into(), inline()),
            BlockKind::ToggleHeading3 => ("### ".into(), inline()),
            BlockKind::Callout => ("> ".into(), inline()),
            BlockKind::ChatRef | BlockKind::Unknown(_) => ("> ".into(), inline()),
            BlockKind::Embed => ("> ".into(), embed_title(b.target.as_deref(), titles)),
            BlockKind::ColumnList | BlockKind::Column | BlockKind::TableOfContents => {
                (String::new(), String::new())
            }
            BlockKind::Paragraph if b.text.is_empty() => (String::new(), "<br>".to_string()),
            BlockKind::Paragraph => (String::new(), inline()),
            BlockKind::Heading1 => ("# ".into(), inline()),
            BlockKind::Heading2 => ("## ".into(), inline()),
            BlockKind::Heading3 => ("### ".into(), inline()),
            BlockKind::Bulleted => ("- ".into(), inline()),
            BlockKind::Numbered => (format!("{number}. "), inline()),
            BlockKind::Todo => (
                (if b.checked { "- [x] " } else { "- [ ] " }).into(),
                inline(),
            ),
            BlockKind::Quote => ("> ".into(), inline()),
            BlockKind::Divider => ("---".into(), String::new()),
            BlockKind::LinkToPage => (
                String::new(),
                format!(
                    "![[{}]]",
                    names.get(&target).cloned().unwrap_or(target.clone())
                ),
            ),
            BlockKind::Database => (
                String::new(),
                format!(
                    "![[{}]] {}",
                    names.get(&target).cloned().unwrap_or(target.clone()),
                    crate::mdparse::DATABASE_SUFFIX
                ),
            ),
            BlockKind::Image => {
                let mut alt = String::new();
                escape(&b.text.replace('\n', " "), &mut alt);
                (String::new(), format!("![{alt}]({})", blob_link(&target)))
            }
            BlockKind::File => {
                let mut name = String::new();
                escape(&b.text.replace('\n', " "), &mut name);
                (String::new(), format!("[{name}]({})", blob_link(&target)))
            }
            BlockKind::Bookmark => {
                let mut cap = String::new();
                escape(&b.text.replace('\n', " "), &mut cap);
                let t = target.replace(['<', '>', ' ', '\n'], "");
                if cap.is_empty() {
                    (String::new(), format!("<{t}>"))
                } else {
                    (String::new(), format!("<{t}> {cap}"))
                }
            }
            BlockKind::Code | BlockKind::Query | BlockKind::View => {
                // The info string (`rust`) is the target; empty when unlabelled.
                let query = b.kind == BlockKind::Query;
                let view = b.kind == BlockKind::View;
                let mut body = b.text.clone();
                if view {
                    if let Some(page) = &b.target {
                        if let Ok(serde_json::Value::Object(mut object)) = serde_json::from_str(&body) {
                            object.insert("page".into(), serde_json::Value::String(page.clone()));
                            body = serde_json::to_string(&object).expect("view JSON object");
                        }
                    }
                }
                let info = if view {
                    "view"
                } else if query {
                    "query"
                } else {
                    b.target.as_deref().unwrap_or("")
                };
                let fence = if query || view {
                    // Stored query/source text can itself contain literal fences.
                    "`".repeat(3.max(longest_backtick_run(&body) + 1))
                } else if b.text.contains("```") || info.contains('`') {
                    "~~~~".into()
                } else {
                    "```".into()
                };
                out.push_str(&format!("{indent}{fence}{info}{}\n", meta_suffix(&meta)));
                let lines: Vec<&str> = if query || view {
                    body.split('\n').collect()
                } else {
                    b.text.lines().collect()
                };
                for line in lines {
                    out.push_str(&format!("{indent}{line}\n"));
                }
                out.push_str(&format!("{indent}{fence}\n"));
                (String::new(), String::new())
            }
            BlockKind::Table => {
                // The source includes its own indentation and row endings.
                out.push_str(&b.text);
                out.push('\n');
                (String::new(), String::new())
            }
        };
        if !matches!(b.kind, BlockKind::Code | BlockKind::Query | BlockKind::View | BlockKind::Table) {
            let generic = matches!(
                b.kind,
                BlockKind::ChatRef | BlockKind::Unknown(_) | BlockKind::Embed
            );
            let cont = if matches!(b.kind, BlockKind::Quote | BlockKind::Callout) || generic {
                format!("{indent}> ")
            } else {
                format!("{indent}{}", " ".repeat(marker.chars().count()))
            };
            // An embed's title is plain text, so it is escaped like paragraphs.
            let raw_first = (matches!(
                b.kind,
                BlockKind::Paragraph | BlockKind::ChatRef | BlockKind::Unknown(_)
            ) && !b.text.is_empty())
                || (b.kind == BlockKind::Embed && !text.is_empty());
            let mut lines = text.split('\n');
            out.push_str(indent);
            out.push_str(&marker);
            let first = lines.next().unwrap_or("");
            if raw_first {
                out.push_str(&escape_line_start(first));
            } else {
                out.push_str(first);
            }
            if !meta.is_empty() {
                // `marker` may end in a space already.
                // An empty fallback leaves the marker's own space in place.
                if (generic && !text.is_empty())
                    || (!out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty())
                {
                    out.push(' ');
                }
                out.push_str(&format!("<!-- {meta} -->"));
            }
            out.push('\n');
            for l in lines {
                out.push_str(&cont);
                out.push_str(&escape_line_start(l));
                out.push('\n');
            }
        }
        if !b.children.is_empty() {
            let width = match b.kind {
                BlockKind::Numbered => format!("{number}. ").len(),
                _ => 2,
            };
            let child_indent = format!("{indent}{}", " ".repeat(width));
            render_blocks(&b.children, &child_indent, names, titles, out);
        }
    }
}

/// The words of a block's metadata comment (empty if it needs none).
fn block_meta(b: &Block) -> String {
    let mut words: Vec<String> = Vec::new();
    match &b.kind {
        BlockKind::ChatRef | BlockKind::Unknown(_) | BlockKind::Embed => {
            // An embed's fallback is one line: its title (see `embed_title`).
            let lines = match b.kind {
                BlockKind::Embed => 1,
                _ => b.text.split('\n').count(),
            };
            let block = crate::mdparse::AnnotatedBlock {
                kind: b.kind.as_str().to_owned(),
                target: b.target.clone(),
                lines,
                checked: b.checked,
            };
            words.push(format!(
                "block={}",
                crate::mdparse::encode_annotation(&block)
            ));
        }
        BlockKind::Toggle
        | BlockKind::ToggleHeading1
        | BlockKind::ToggleHeading2
        | BlockKind::ToggleHeading3 => words.push("toggle".into()),
        BlockKind::Callout => {
            words.push("callout".into());
            if let Some(icon) = b.target.as_deref().filter(|t| meta_value_ok(t)) {
                words.push(format!("icon={icon}"));
            }
        }
        BlockKind::ColumnList => words.push("columns".into()),
        BlockKind::Column => words.push("column".into()),
        BlockKind::TableOfContents => words.push("toc".into()),
        _ => {}
    }
    if let Some(c) = b.color.as_deref().filter(|c| meta_value_ok(c)) {
        words.push(format!("color={c}"));
    }
    if let Some(w) = b.width.filter(|w| *w > 0 && *w < 100) {
        words.push(format!("width={w}"));
    }
    words.join(" ")
}

/// Values in a metadata comment cannot hold spaces or end the comment.
fn meta_value_ok(v: &str) -> bool {
    !v.is_empty()
        && !v.contains(char::is_whitespace)
        && !v.contains("--")
        && !v.contains('>')
        && !v.contains('=')
}

fn meta_suffix(meta: &str) -> String {
    if meta.is_empty() {
        String::new()
    } else {
        format!(" <!-- {meta} -->")
    }
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '~' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
}

/// The length of the longest run of backticks in `s` (0 when there are none).
fn longest_backtick_run(s: &str) -> usize {
    s.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// The backticks that open and close a code span over `text`: one more than
/// the longest backtick run in `text`, and a space each side when `text`
/// starts or ends with a backtick, or its own edge spaces would be stripped.
fn code_span_delims(text: &str) -> (String, String) {
    let ticks = "`".repeat(longest_backtick_run(text) + 1);
    let padded_spaces = text.starts_with(' ')
        && text.ends_with(' ')
        && text.chars().any(|c| c != ' ');
    if text.starts_with('`') || text.ends_with('`') || padded_spaces {
        (format!("{ticks} "), format!(" {ticks}"))
    } else {
        (ticks.clone(), ticks)
    }
}

fn open_tag(k: &MarkKind, names: &HashMap<String, String>) -> String {
    match k {
        MarkKind::Link(_) => "[".into(),
        MarkKind::PageLink(id) => format!(
            "[[{}|",
            names.get(id).cloned().unwrap_or_else(|| id.clone())
        ),
        MarkKind::Bold => "**".into(),
        MarkKind::Italic => "*".into(),
        MarkKind::Strike => "~~".into(),
        MarkKind::Code => "`".into(),
        MarkKind::Underline => "<u>".into(),
        MarkKind::Color(c) => format!("<span data-color=\"{}\">", attr_value(c)),
        MarkKind::Date(d) => format!("<time datetime=\"{}\">", attr_value(d)),
        MarkKind::Query(spec) => format!("<span data-query=\"{}\">", query_attr(spec)),
    }
}

fn attr_value(v: &str) -> String {
    v.replace(['"', '<', '>', '\n'], "")
}

/// Unlike cosmetic attributes, a query's stored string must be lossless.
fn query_attr(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(ch),
        }
    }
    out
}

fn close_tag(k: &MarkKind, names: &HashMap<String, String>) -> String {
    match k {
        // A note link is written back exactly as its target was read.
        MarkKind::Link(url) if md_link(url) => format!("]({url})"),
        MarkKind::Link(url) => format!("]({})", url.replace(' ', "%20").replace(')', "%29")),
        MarkKind::PageLink(_) => "]]".into(),
        MarkKind::Underline => "</u>".into(),
        MarkKind::Color(_) | MarkKind::Query(_) => "</span>".into(),
        MarkKind::Date(_) => "</time>".into(),
        other => open_tag(other, names),
    }
}

/// Inline markdown for a text with marks (page links written by id).
#[cfg(test)]
pub(crate) fn render_inline(text: &str, marks: &[Mark]) -> String {
    render_inline_with(text, marks, &HashMap::new())
}

/// A mark as it is written: its span, and the tags around its text.
struct Written {
    kind: MarkKind,
    start: u32,
    end: u32,
    open: String,
    close: String,
}

/// The link target written over the same text as `m`, if any.
fn link_on<'a>(marks: &'a [Mark], m: &Mark) -> Option<&'a str> {
    marks.iter().find_map(|o| match &o.kind {
        MarkKind::Link(url) if o.start == m.start && o.end == m.end => Some(url.as_str()),
        _ => None,
    })
}

/// Whether a page link is written over the same text as `m`.
fn page_on(marks: &[Mark], m: &Mark) -> bool {
    marks
        .iter()
        .any(|o| matches!(o.kind, MarkKind::PageLink(_)) && o.start == m.start && o.end == m.end)
}

/// A link to a note (`Notes/Home.md#Intro`), as opposed to a web link.
fn md_link(url: &str) -> bool {
    !is_web(url) && md_target(url)
}

/// Each page link's target as written (`Home#Intro`, `Notes/Home.md`) -> the
/// page's id, for the page links that have one.
fn written_targets(blocks: &[Block]) -> Vec<(String, String)> {
    fn walk(blocks: &[Block], out: &mut Vec<(String, String)>) {
        for b in blocks {
            for m in &b.marks {
                if let MarkKind::Link(url) = &m.kind {
                    if let Some(id) = b.marks.iter().find_map(|page| match &page.kind {
                        MarkKind::PageLink(id) if page.start <= m.start && page.end >= m.end => {
                            Some(id)
                        }
                        _ => None,
                    }) {
                        out.push((url.clone(), id.clone()));
                    }
                }
            }
            walk(&b.children, out);
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// Inline markdown for a text with marks. Marks are opened in a fixed order
/// (query metadata outermost, code innermost) and properly nested: where a mark ends,
/// every mark opened after it is closed and reopened.
pub(crate) fn render_inline_with(
    text: &str,
    marks: &[Mark],
    names: &HashMap<String, String>,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    fn is_emph(k: &MarkKind) -> bool {
        matches!(k, MarkKind::Bold | MarkKind::Italic | MarkKind::Strike)
    }
    fn order(k: &MarkKind) -> u8 {
        match k {
            MarkKind::Link(_)
            | MarkKind::PageLink(_)
            | MarkKind::Date(_)
            | MarkKind::Query(_) => 0,
            MarkKind::Color(_) => 1,
            MarkKind::Bold => 2,
            MarkKind::Italic => 3,
            MarkKind::Strike => 4,
            MarkKind::Underline => 5,
            MarkKind::Code => 6,
        }
    }
    // Emphasis over whitespace only cannot be written (`** **` is literal).
    let marks: Vec<Mark> = normalize_marks(marks)
        .into_iter()
        .filter(|m| {
            !is_emph(&m.kind)
                || chars
                    .get(m.start as usize..m.end as usize)
                    .is_some_and(|s| s.iter().any(|c| !c.is_whitespace()))
        })
        .collect();
    // Loro can coalesce adjacent mentions of the same page even when their
    // authored targets differ. Split them at target boundaries for export.
    let marks: Vec<Mark> = marks
        .iter()
        .flat_map(|m| {
            if !matches!(m.kind, MarkKind::PageLink(_)) {
                return vec![m.clone()];
            }
            let mut bounds = vec![m.start, m.end];
            for link in &marks {
                if matches!(link.kind, MarkKind::Link(_)) {
                    if link.start > m.start && link.start < m.end {
                        bounds.push(link.start);
                    }
                    if link.end > m.start && link.end < m.end {
                        bounds.push(link.end);
                    }
                }
            }
            bounds.sort_unstable();
            bounds.dedup();
            bounds
                .windows(2)
                .map(|span| Mark {
                    start: span[0],
                    end: span[1],
                    kind: m.kind.clone(),
                })
                .collect()
        })
        .collect();
    // A link and a page link over the same text are one pair, written once.
    // Imported links retain their authored target; ordinary mentions use
    // the page's current title.
    let mut written: Vec<Written> = Vec::new();
    for m in &marks {
        let (open, close) = match &m.kind {
            MarkKind::PageLink(_) => match link_on(&marks, m) {
                Some(url) if md_link(url) || is_web(url) => continue,
                Some(url) => (format!("[[{url}|"), "]]".to_string()),
                None => (open_tag(&m.kind, names), close_tag(&m.kind, names)),
            },
            MarkKind::Link(url) if page_on(&marks, m) && !md_link(url) && !is_web(url) => continue,
            MarkKind::Code => {
                let span: String = chars[m.start as usize..m.end as usize].iter().collect();
                code_span_delims(&span)
            }
            _ => (open_tag(&m.kind, names), close_tag(&m.kind, names)),
        };
        written.push(Written {
            kind: m.kind.clone(),
            start: m.start,
            end: m.end,
            open,
            close,
        });
    }
    if written.is_empty() {
        let mut out = String::with_capacity(text.len());
        escape(text, &mut out);
        return out;
    }
    let mut bounds: Vec<u32> = vec![0, chars.len() as u32];
    for w in &written {
        bounds.push(w.start);
        bounds.push(w.end);
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out = String::with_capacity(text.len() + written.len() * 4);
    // Indexes into `written`: a mark is the same mark in every segment it covers.
    let mut stack: Vec<usize> = Vec::new();
    // Whitespace characters at the end of `out` written as segment text.
    let mut tail_ws = 0usize;
    for w in bounds.windows(2) {
        let (s, e) = (w[0], w[1]);
        let mut active: Vec<usize> = (0..written.len())
            .filter(|&i| written[i].start <= s && written[i].end >= e)
            .collect();
        active.sort_by_key(|&i| (
            !matches!(written[i].kind, MarkKind::Query(_)),
            order(&written[i].kind),
            i,
        ));
        // Only one link (web or page) can be open: nested links do not exist.
        let mut seen_link = false;
        active.retain(|&i| {
            if order(&written[i].kind) == 0 && !matches!(written[i].kind, MarkKind::Query(_)) {
                let keep = !seen_link;
                seen_link = true;
                keep
            } else {
                true
            }
        });
        // Keep the longest prefix of the stack that is still active and in order.
        let keep = stack
            .iter()
            .zip(active.iter())
            .take_while(|(a, b)| a == b)
            .count();
        // Emphasis delimiters must touch text, not whitespace (`*a *`
        // does not close): whitespace at the edge of an emphasis moves
        // outside it.
        let closes_emph = stack[keep..].iter().any(|&i| is_emph(&written[i].kind));
        let mut moved = String::new();
        if closes_emph {
            while tail_ws > 0 {
                moved.insert(0, out.pop().expect("whitespace"));
                tail_ws -= 1;
            }
        }
        while stack.len() > keep {
            let i = stack.pop().unwrap();
            out.push_str(&written[i].close);
        }
        out.push_str(&moved);
        let mut seg: String = chars[s as usize..e as usize].iter().collect();
        if active[keep..].iter().any(|&i| is_emph(&written[i].kind)) {
            let lead: String = seg.chars().take_while(|c| c.is_whitespace()).collect();
            if lead.len() < seg.len() {
                out.push_str(&lead);
                seg.drain(..lead.len());
            }
        }
        for &i in &active[keep..] {
            out.push_str(&written[i].open);
            stack.push(i);
        }
        let in_code = stack
            .last()
            .is_some_and(|&i| written[i].kind == MarkKind::Code);
        tail_ws = if in_code {
            0
        } else {
            seg.chars().rev().take_while(|c| c.is_whitespace()).count()
        };
        if in_code {
            out.push_str(&seg);
        } else {
            escape(&seg, &mut out);
        }
    }
    let mut moved = String::new();
    if stack.iter().any(|&i| is_emph(&written[i].kind)) {
        while tail_ws > 0 {
            moved.insert(0, out.pop().expect("whitespace"));
            tail_ws -= 1;
        }
    }
    while let Some(i) = stack.pop() {
        out.push_str(&written[i].close);
    }
    out.push_str(&moved);
    simplify_wikilinks(&out)
}

/// `[[X|X]]` -> `[[X]]`.
fn simplify_wikilinks(s: &str) -> String {
    if !s.contains("[[") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("[[") {
        // Skip escaped brackets (`\[\[` never matches "[[").
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match (after.find('|'), after.find("]]")) {
            (Some(bar), Some(end)) if bar < end && after[..bar] == after[bar + 1..end] => {
                out.push_str("[[");
                out.push_str(&after[..bar]);
                out.push_str("]]");
                rest = &after[end + 2..];
            }
            _ => {
                out.push_str("[[");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_790_000_000_123), "2026-09-21T14:13:20.123Z");
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 9, 21) * 86_400_000, 1_789_948_800_000);
        assert_eq!(
            iso_date(days_from_civil(2024, 2, 29) * 86_400_000),
            "2024-02-29"
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(file_name("Hello, World!", "X"), "hello-world-X.md");
        assert_eq!(file_name("Über Café", "X"), "über-café-X.md");
        assert_eq!(file_name("  ", "X"), "untitled-X.md");
    }

    #[test]
    fn nested_marks() {
        let m = |s, e, kind| Mark {
            start: s,
            end: e,
            kind,
        };
        assert_eq!(
            render_inline(
                "ab cd",
                &[m(0, 5, MarkKind::Bold), m(3, 5, MarkKind::Italic)]
            ),
            "**ab *cd***"
        );
        assert_eq!(
            render_inline(
                "ab cd",
                &[m(0, 3, MarkKind::Italic), m(0, 5, MarkKind::Bold)]
            ),
            "***ab* cd**"
        );
        // Whitespace at the edges of emphasis moves outside the delimiters.
        assert_eq!(render_inline("a b", &[m(0, 2, MarkKind::Bold)]), "**a** b");
        assert_eq!(render_inline("a b", &[m(1, 3, MarkKind::Italic)]), "a *b*");
        assert_eq!(render_inline("a b", &[m(1, 2, MarkKind::Bold)]), "a b");
        assert_eq!(
            render_inline("go here", &[m(3, 7, MarkKind::Link("https://x.y".into()))]),
            "go [here](https://x.y)"
        );
        assert_eq!(render_inline("a*b", &[]), "a\\*b");
        let names: HashMap<String, String> = [("P1".to_string(), "Home".to_string())].into();
        assert_eq!(
            render_inline_with(
                "see Home now",
                &[m(4, 8, MarkKind::PageLink("P1".into()))],
                &names
            ),
            "see [[Home]] now"
        );
        assert_eq!(
            render_inline_with(
                "see home",
                &[
                    m(4, 8, MarkKind::PageLink("P1".into())),
                    m(4, 6, MarkKind::Bold)
                ],
                &names
            ),
            "see [[Home|**ho**me]]"
        );
    }

    #[test]
    fn paired_targets_are_written_as_the_user_wrote_them() {
        let m = |s, e, kind| Mark {
            start: s,
            end: e,
            kind,
        };
        let names: HashMap<String, String> = [("P1".to_string(), "Home".to_string())].into();
        // Imported wikilinks keep their authored target and anchor.
        assert_eq!(
            render_inline_with(
                "see home",
                &[
                    m(4, 8, MarkKind::Link("Old#Top".into())),
                    m(4, 8, MarkKind::PageLink("P1".into())),
                ],
                &names
            ),
            "see [[Old#Top|home]]"
        );
        // A `.md` link keeps its target spelling, percent-encoding included.
        assert_eq!(
            render_inline_with(
                "go here",
                &[
                    m(3, 7, MarkKind::PageLink("P1".into())),
                    m(3, 7, MarkKind::Link("Notes/My%20Page.md#Intro".into())),
                ],
                &names
            ),
            "go [here](Notes/My%20Page.md#Intro)"
        );
        // Without a page link (unresolved), the same target is written as is.
        assert_eq!(
            render_inline_with(
                "go here",
                &[m(3, 7, MarkKind::Link("Gone.MD".into()))],
                &names
            ),
            "go [here](Gone.MD)"
        );
    }
}
