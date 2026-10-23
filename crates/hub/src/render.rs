//! A page as a small HTML fragment for the web viewer and shares: the
//! page's markdown (the same `Vault::blocks_markdown` that MCP `read_page`
//! shows) through pulldown-cmark, a properties table, and databases as plain
//! tables. Raw HTML in the markdown is never passed through; page links are
//! links only when the viewer may open the target; images and files point
//! at `asset_base/<attachment id>` and only for attachments the page uses.

use std::collections::{BTreeSet, HashMap};

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use vault_core::{Block, BlockKind, MarkKind, Page, PropDef, Vault};

/// Rows shown for one database table.
const MAX_ROWS: u32 = 200;
/// Characters of the preview card description.
const DESCRIPTION_CHARS: usize = 200;

pub(crate) struct RenderOpts<'a> {
    /// Where the page's attachments are served, without a trailing slash,
    /// e.g. `/s/<token>/a` or `/open/<page>/a`.
    pub asset_base: &'a str,
    /// Links to other pages go to `/open/<id>` (a signed-in viewer); else
    /// they show as plain text.
    pub link_pages: bool,
}

pub(crate) struct Rendered {
    /// The title as shown (`Untitled` when empty), without the icon.
    pub title: String,
    pub icon: Option<String>,
    /// The properties table, or empty.
    pub props_html: String,
    pub body_html: String,
    /// The first ~200 characters of the page's text, for preview cards.
    pub description: String,
    /// The first image attachment, for preview cards.
    pub first_image: Option<String>,
}

pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Whether `s` is an attachment id: a lowercase SHA-256 hex digest.
pub(crate) fn is_attachment_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn display_title(page: &Page) -> String {
    let t = page.title.trim();
    if t.is_empty() {
        "Untitled".to_string()
    } else {
        t.to_string()
    }
}

/// The attachments `page` shows: image and file blocks anywhere in its
/// tree, and its cover. Only these are served with the page.
pub(crate) fn page_assets(page: &Page) -> BTreeSet<String> {
    fn walk(blocks: &[Block], out: &mut BTreeSet<String>) {
        for b in blocks {
            if matches!(b.kind, BlockKind::Image | BlockKind::File) {
                if let Some(t) = b.target.as_deref().filter(|t| is_attachment_id(t)) {
                    out.insert(t.to_string());
                }
            }
            walk(&b.children, out);
        }
    }
    let mut out = BTreeSet::new();
    walk(&page.blocks, &mut out);
    if let Some(c) = page.cover.as_deref().filter(|c| is_attachment_id(c)) {
        out.insert(c.to_string());
    }
    out
}

/// The first image block's attachment, in reading order.
fn first_image(blocks: &[Block]) -> Option<String> {
    for b in blocks {
        if b.kind == BlockKind::Image {
            if let Some(t) = b.target.as_deref().filter(|t| is_attachment_id(t)) {
                return Some(t.to_string());
            }
        }
        if let Some(t) = first_image(&b.children) {
            return Some(t);
        }
    }
    None
}

/// Plain text of the page for the card description: prose blocks only,
/// cut at a word boundary near [`DESCRIPTION_CHARS`].
fn description(blocks: &[Block]) -> String {
    fn walk(blocks: &[Block], out: &mut String) {
        for b in blocks {
            if out.chars().count() > DESCRIPTION_CHARS {
                return;
            }
            let prose = matches!(
                b.kind,
                BlockKind::Paragraph
                    | BlockKind::Heading1
                    | BlockKind::Heading2
                    | BlockKind::Heading3
                    | BlockKind::Bulleted
                    | BlockKind::Numbered
                    | BlockKind::Todo
                    | BlockKind::Quote
                    | BlockKind::Toggle
                    | BlockKind::ToggleHeading1
                    | BlockKind::ToggleHeading2
                    | BlockKind::ToggleHeading3
                    | BlockKind::Callout
            );
            if prose {
                let t = b.text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !t.is_empty() {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(&t);
                }
            }
            walk(&b.children, out);
        }
    }
    let mut s = String::new();
    walk(blocks, &mut s);
    if s.chars().count() <= DESCRIPTION_CHARS {
        return s;
    }
    let cut: String = s.chars().take(DESCRIPTION_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > DESCRIPTION_CHARS / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end_matches([',', '.', ';', ':', ' ']))
}

/// How the markdown names each linked page, mirroring the core's
/// `link_names` for the same blocks: the title when it is usable and unique
/// among them, else the id. Returned as name → id.
fn link_targets(v: &Vault, blocks: &[Block]) -> HashMap<String, String> {
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
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut out = HashMap::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let title = v.load_page(id.clone()).ok().map(|p| p.title);
        let name = match title {
            Some(t)
                if !t.trim().is_empty()
                    && t.trim() == t
                    && !t.contains(['[', ']', '|', '#', '^', '\n'])
                    && !used.contains(&t.to_lowercase()) =>
            {
                used.insert(t.to_lowercase());
                t
            }
            _ => id.clone(),
        };
        out.insert(name, id);
    }
    out
}

/// Whether `page` can be opened by a signed-in viewer: it exists and is not
/// in the trash.
fn live_page(v: &Vault, id: &str) -> Option<Page> {
    v.load_page(id.to_string()).ok().filter(|p| !p.in_trash)
}

fn page_link_html(v: &Vault, id: &str, text_html: &str, opts: &RenderOpts) -> (String, String) {
    if opts.link_pages && live_page(v, id).is_some() {
        (
            format!("<a class=\"pagelink\" href=\"/open/{}\">", escape(id)),
            format!("{text_html}</a>"),
        )
    } else {
        ("<span class=\"pagelink\">".into(), format!("{text_html}</span>"))
    }
}

/// The attachment id in a markdown blob path (`../.vault/blobs/ab/<id>`).
fn blob_of(dest: &str) -> Option<&str> {
    let id = dest.strip_prefix("../.vault/blobs/")?.split('/').nth(1)?;
    is_attachment_id(id).then_some(id)
}

fn safe_url(dest: &str) -> bool {
    let d = dest.trim().to_ascii_lowercase();
    d.starts_with("https://") || d.starts_with("http://") || d.starts_with("mailto:")
}

/// Markdown (from `Vault::blocks_markdown`) to HTML.
fn markdown_html(
    v: &Vault,
    md: &str,
    names: &HashMap<String, String>,
    assets: &BTreeSet<String>,
    opts: &RenderOpts,
) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_WIKILINKS;
    // What closes the link or image that is open (they do not nest).
    let mut close: Option<String> = None;
    let mut in_alt = false;
    let mut events: Vec<Event> = Vec::new();
    for ev in Parser::new_ext(md, options) {
        // Inside an <img alt="..."> only escaped text may appear.
        if in_alt {
            match ev {
                Event::End(TagEnd::Image) => {}
                Event::Text(t) | Event::Code(t) => {
                    events.push(Event::Html(CowStr::from(escape(&t))));
                    continue;
                }
                Event::SoftBreak | Event::HardBreak => {
                    events.push(Event::Html(CowStr::from(" ")));
                    continue;
                }
                _ => continue,
            }
        }
        match ev {
            // No raw HTML: the core's metadata comments are dropped and any
            // other HTML shows as text.
            Event::Html(h) | Event::InlineHtml(h) => {
                let t = h.trim();
                if !(t.starts_with("<!--") && t.ends_with("-->")) {
                    events.push(Event::Text(CowStr::from(h.into_string())));
                }
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let (open, end) = if matches!(link_type, LinkType::WikiLink { .. }) {
                    match names.get(dest_url.as_ref()) {
                        Some(id) => {
                            let (o, c) = page_link_html(v, id, "", opts);
                            (o, c)
                        }
                        None => ("<span class=\"pagelink\">".into(), "</span>".into()),
                    }
                } else if let Some(id) = blob_of(&dest_url) {
                    if assets.contains(id) {
                        (
                            format!(
                                "<a class=\"file\" href=\"{}/{}\" download>",
                                escape(opts.asset_base),
                                id
                            ),
                            "</a>".into(),
                        )
                    } else {
                        ("<span>".into(), "</span>".into())
                    }
                } else if safe_url(&dest_url) {
                    let t = if title.is_empty() {
                        String::new()
                    } else {
                        format!(" title=\"{}\"", escape(&title))
                    };
                    (
                        format!(
                            "<a href=\"{}\"{t} rel=\"noopener noreferrer nofollow\">",
                            escape(dest_url.trim())
                        ),
                        "</a>".into(),
                    )
                } else {
                    ("<span>".into(), "</span>".into())
                };
                events.push(Event::Html(CowStr::from(open)));
                close = Some(end);
            }
            Event::End(TagEnd::Link) => {
                let end = close.take().unwrap_or_else(|| "</span>".into());
                events.push(Event::Html(CowStr::from(end)));
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                ..
            }) => {
                if matches!(link_type, LinkType::WikiLink { .. }) {
                    // `![[Page]]`: a link-to-page block.
                    let (open, end) = match names.get(dest_url.as_ref()) {
                        Some(id) => page_link_html(v, id, "", opts),
                        None => ("<span class=\"pagelink\">".into(), "</span>".into()),
                    };
                    events.push(Event::Html(CowStr::from(format!(
                        "<span class=\"subpage\">↳ {open}"
                    ))));
                    close = Some(format!("{end}</span>"));
                } else if let Some(id) = blob_of(&dest_url).filter(|id| assets.contains(*id)) {
                    events.push(Event::Html(CowStr::from(format!(
                        "<figure><img src=\"{}/{}\" alt=\"",
                        escape(opts.asset_base),
                        id
                    ))));
                    close = Some("\"></figure>".into());
                    in_alt = true;
                } else {
                    events.push(Event::Html(CowStr::from("<span>")));
                    close = Some("</span>".into());
                }
            }
            Event::End(TagEnd::Image) => {
                in_alt = false;
                let end = close.take().unwrap_or_else(|| "</span>".into());
                events.push(Event::Html(CowStr::from(end)));
            }
            e => events.push(e),
        }
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

fn schema_of(v: &Vault, page: &Page) -> Vec<PropDef> {
    let Some(parent) = page.parent.clone() else {
        return Vec::new();
    };
    match v.load_page(parent.clone()) {
        Ok(p) if p.is_database() => v.get_schema(parent).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn props_html(v: &Vault, page: &Page) -> String {
    if page.properties.is_empty() {
        return String::new();
    }
    let schema = schema_of(v, page);
    let mut out = String::from("<table class=\"props\"><tbody>");
    for p in &page.properties {
        if p.key.starts_with('$') {
            continue;
        }
        let def = schema.iter().find(|d| d.key == p.key);
        let name = def.map(|d| d.name.as_str()).unwrap_or(&p.key);
        let value = vault_tools::props::display(v, &p.value, def);
        out.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            escape(name),
            escape(&value)
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// A database's rows as a plain table: title, then each property.
fn database_table(v: &Vault, db: &str, opts: &RenderOpts) -> String {
    let Some(db_page) = live_page(v, db) else {
        return String::new();
    };
    let schema = v.get_schema(db.to_string()).unwrap_or_default();
    let rows = v
        .query(Some(db.to_string()), None, vec![], 0, MAX_ROWS)
        .map(|r| (r.rows, r.total))
        .unwrap_or_default();
    let mut out = format!(
        "<div class=\"db\"><div class=\"db-title\">{}</div><div class=\"db-scroll\"><table><thead><tr><th>Name</th>",
        escape(&display_title(&db_page))
    );
    for d in &schema {
        out.push_str(&format!("<th>{}</th>", escape(&d.name)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in &rows.0 {
        let title = if row.title.trim().is_empty() {
            "Untitled".to_string()
        } else {
            row.title.clone()
        };
        let (open, close) = page_link_html(v, &row.id, &escape(&title), opts);
        out.push_str(&format!("<tr><td>{open}{close}</td>"));
        for d in &schema {
            let value = row
                .properties
                .iter()
                .find(|p| p.key == d.key)
                .map(|p| vault_tools::props::display(v, &p.value, Some(d)))
                .unwrap_or_default();
            out.push_str(&format!("<td>{}</td>", escape(&value)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    if rows.1 > rows.0.len() as u32 {
        out.push_str(&format!(
            "<p class=\"muted\">{} of {} rows</p>",
            rows.0.len(),
            rows.1
        ));
    }
    out.push_str("</div>");
    out
}

/// Blocks to HTML: runs of ordinary blocks through markdown, inline
/// databases as tables.
fn blocks_html(v: &Vault, blocks: &[Block], assets: &BTreeSet<String>, opts: &RenderOpts) -> String {
    let mut html = String::new();
    let mut run: Vec<Block> = Vec::new();
    let flush = |run: &mut Vec<Block>, html: &mut String| {
        if run.is_empty() {
            return;
        }
        let names = link_targets(v, run);
        let md = v.blocks_markdown(std::mem::take(run));
        html.push_str(&markdown_html(v, &md, &names, assets, opts));
    };
    for b in blocks {
        match (&b.kind, b.target.as_deref()) {
            (BlockKind::Database, Some(db)) => {
                flush(&mut run, &mut html);
                html.push_str(&database_table(v, db, opts));
            }
            _ => run.push(b.clone()),
        }
    }
    flush(&mut run, &mut html);
    html
}

pub(crate) fn render_page(v: &Vault, page: &Page, opts: &RenderOpts) -> Rendered {
    let assets = page_assets(page);
    let mut body_html = blocks_html(v, &page.blocks, &assets, opts);
    if page.is_database() {
        body_html.push_str(&database_table(v, &page.id, opts));
    }
    Rendered {
        title: display_title(page),
        icon: page.icon.clone().filter(|i| !i.trim().is_empty()),
        props_html: props_html(v, page),
        body_html,
        description: description(&page.blocks),
        first_image: first_image(&page.blocks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_ids_and_blob_paths() {
        let id = "a".repeat(64);
        assert!(is_attachment_id(&id));
        assert!(!is_attachment_id(&"A".repeat(64)));
        assert!(!is_attachment_id("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert_eq!(blob_of(&format!("../.vault/blobs/aa/{id}")), Some(id.as_str()));
        assert_eq!(blob_of("https://example.com/x.png"), None);
    }

    #[test]
    fn only_web_and_mail_links_are_links() {
        assert!(safe_url("https://example.com"));
        assert!(safe_url("mailto:a@example.com"));
        assert!(!safe_url("javascript:alert(1)"));
        assert!(!safe_url(" JaVaScRiPt:alert(1)"));
        assert!(!safe_url("data:text/html,x"));
    }

    #[test]
    fn descriptions_are_cut_at_a_word() {
        let b = |t: &str| Block {
            id: "b".into(),
            kind: BlockKind::Paragraph,
            text: t.into(),
            marks: vec![],
            checked: false,
            children: vec![],
            target: None,
            color: None,
            width: None,
        };
        assert_eq!(description(&[b("Hello"), b("world")]), "Hello world");
        let long = "word ".repeat(80);
        let d = description(&[b(&long)]);
        assert!(d.ends_with('…'), "{d}");
        assert!(d.chars().count() <= DESCRIPTION_CHARS + 1, "{d}");
    }
}
