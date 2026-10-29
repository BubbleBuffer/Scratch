//! Reads markdown back into blocks: everything `markdown.rs` writes, plus the
//! common constructs of Obsidian vaults (wikilinks with aliases and headings,
//! embeds, `*`/`_`/`+` lists, `***` dividers, `h4`-`h6`) and GFM pipe tables,
//! kept verbatim. Not a CommonMark parser: unknown syntax stays as literal
//! text.
//!
//! Also a small YAML subset for frontmatter: `key: scalar`, flow lists
//! `[a, "b"]`, block lists (`- a`) and one level of `key: value` maps.

use crate::types::{Block, BlockKind, Mark, MarkKind};

/// Marks an embedded page link (`![[Title]]`) as an inline database block.
pub(crate) const DATABASE_SUFFIX: &str = "<!-- database -->";

/// Generic fallback metadata. Hex-encoded JSON fits safely inside an HTML
/// comment, even when a raw kind or target contains whitespace or `-->`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct AnnotatedBlock {
    pub kind: String,
    pub target: Option<String>,
    pub lines: usize,
    pub checked: bool,
}

pub(crate) fn encode_annotation(block: &AnnotatedBlock) -> String {
    serde_json::to_vec(block)
        .expect("block annotation JSON")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_annotation(raw: &str) -> Option<AnnotatedBlock> {
    if !raw.len().is_multiple_of(2) || !raw.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = raw
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect();
    let block: AnnotatedBlock = serde_json::from_slice(&bytes?).ok()?;
    (block.lines > 0).then_some(block)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Yaml {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Yaml::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The value as text (numbers and booleans formatted).
    pub fn to_text(&self) -> Option<String> {
        match self {
            Yaml::Str(s) => Some(s.clone()),
            Yaml::Num(n) => Some(if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.0}")
            } else {
                n.to_string()
            }),
            Yaml::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }
}

/// Resolves links while parsing.
pub(crate) trait Resolver {
    /// A wikilink target (without alias or heading) -> page id.
    fn page(&mut self, target: &str) -> Option<String>;
    /// Importers retain authored targets; ordinary page mentions keep their
    /// existing representation unless saved metadata requests preservation.
    fn preserve_wikilink_target(&self, _target: &str) -> bool {
        false
    }
    /// A `.md` link target as written (path and `#fragment` included) -> page
    /// id. By default the file's name is looked up: `Notes/My%20Page.md#Intro`
    /// is the page `My Page`.
    fn markdown_page(&mut self, target: &str) -> Option<String> {
        let path = decode_path(target.split('#').next().unwrap_or(""));
        let last = path.rsplit(['/', '\\']).next().unwrap_or("");
        // `.md` is ASCII, so cutting its three bytes stays on a char boundary.
        let stem = if md_target(last) {
            &last[..last.len() - 3]
        } else {
            last
        };
        self.page(stem)
    }
    /// A local file path or embed name -> attachment id.
    fn file(&mut self, path: &str) -> Option<String>;
    /// Called for each wikilink or `.md` link that did not resolve.
    fn unresolved(&mut self, _target: &str) {}
}

// ---------------------------------------------------------------- frontmatter

/// Splits `---` frontmatter from the body. Returns (entries, body).
pub(crate) fn split_frontmatter(src: &str) -> (Vec<(String, Yaml)>, &str) {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let Some(rest) = src
        .strip_prefix("---\n")
        .or_else(|| src.strip_prefix("---\r\n"))
    else {
        return (Vec::new(), src);
    };
    let mut end = None;
    let mut pos = 0;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            end = Some((pos, pos + line.len()));
            break;
        }
        pos += line.len();
    }
    match end {
        Some((yaml_end, body_start)) => (parse_yaml(&rest[..yaml_end]), &rest[body_start..]),
        None => (Vec::new(), src),
    }
}

fn indent_of(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

/// Byte offset after `n` columns of indentation.
fn strip_indent(line: &str, n: usize) -> &str {
    let mut cols = 0;
    for (i, c) in line.char_indices() {
        if cols >= n {
            return &line[i..];
        }
        match c {
            ' ' => cols += 1,
            '\t' => cols += 4,
            _ => return &line[i..],
        }
    }
    ""
}

pub(crate) fn parse_yaml(src: &str) -> Vec<(String, Yaml)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') || indent_of(line) > 0 {
            continue;
        }
        let Some((key, rest)) = split_key(line) else {
            continue;
        };
        let rest = rest.trim();
        if !rest.is_empty() {
            out.push((key, parse_scalar_or_flow(rest)));
            continue;
        }
        // Nested block: a list or a map.
        let mut items = Vec::new();
        let mut map = Vec::new();
        while i < lines.len() && (lines[i].trim().is_empty() || indent_of(lines[i]) > 0) {
            let l = lines[i].trim();
            i += 1;
            if l.is_empty() {
                continue;
            }
            if let Some(item) = l
                .strip_prefix("- ")
                .or(if l == "-" { Some("") } else { None })
            {
                items.push(parse_scalar_or_flow(item.trim()));
            } else if let Some((k, v)) = split_key(l) {
                map.push((k, parse_scalar_or_flow(v.trim())));
            }
        }
        let value = if !items.is_empty() {
            Yaml::List(items)
        } else if !map.is_empty() {
            Yaml::Map(map)
        } else {
            Yaml::Null
        };
        out.push((key, value));
    }
    out
}

/// `key: rest` with a plain or quoted key.
fn split_key(line: &str) -> Option<(String, &str)> {
    let line = line.trim_start();
    if line.starts_with('"') || line.starts_with('\'') {
        let (key, used) = parse_quoted(line)?;
        let rest = line[used..].trim_start().strip_prefix(':')?;
        return Some((key, rest));
    }
    let idx = line
        .find(": ")
        .or_else(|| line.strip_suffix(':').map(|s| s.len()))?;
    Some((
        line[..idx].trim().to_string(),
        &line[(idx + 1).min(line.len())..],
    ))
}

/// A quoted string at the start of `s`: (value, bytes used).
fn parse_quoted(s: &str) -> Option<(String, usize)> {
    let q = s.chars().next()?;
    let mut out = String::new();
    let mut it = s.char_indices().skip(1).peekable();
    while let Some((i, c)) = it.next() {
        if c == q {
            if q == '\'' && it.peek().map(|x| x.1) == Some('\'') {
                it.next();
                out.push('\'');
                continue;
            }
            return Some((out, i + 1));
        }
        if c == '\\' && q == '"' {
            let (_, e) = it.next()?;
            match e {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'x' | 'u' | 'U' => {
                    let len = match e {
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    let hex: String = (0..len).filter_map(|_| it.next().map(|x| x.1)).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            }
            continue;
        }
        out.push(c);
    }
    None
}

fn parse_scalar_or_flow(s: &str) -> Yaml {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        let mut items = Vec::new();
        let mut cur = String::new();
        let mut quote: Option<char> = None;
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            match quote {
                Some(q) => {
                    cur.push(c);
                    if c == '\\' && q == '"' {
                        if let Some(n) = chars.next() {
                            cur.push(n);
                        }
                    } else if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => {
                        quote = Some(c);
                        cur.push(c);
                    }
                    ',' => {
                        if !cur.trim().is_empty() {
                            items.push(parse_scalar(cur.trim()));
                        }
                        cur.clear();
                    }
                    _ => cur.push(c),
                },
            }
        }
        if !cur.trim().is_empty() {
            items.push(parse_scalar(cur.trim()));
        }
        return Yaml::List(items);
    }
    parse_scalar(s)
}

fn parse_scalar(s: &str) -> Yaml {
    if s.starts_with('"') || s.starts_with('\'') {
        if let Some((v, _)) = parse_quoted(s) {
            return Yaml::Str(v);
        }
    }
    // Strip a trailing comment.
    let s = match s.find(" #") {
        Some(i) => s[..i].trim_end(),
        None => s,
    };
    match s {
        "" | "null" | "~" | "Null" | "NULL" => Yaml::Null,
        "true" | "True" | "TRUE" => Yaml::Bool(true),
        "false" | "False" | "FALSE" => Yaml::Bool(false),
        _ => match s.parse::<f64>() {
            Ok(n) if n.is_finite() && !s.starts_with('+') && !s.ends_with('.') => Yaml::Num(n),
            _ => Yaml::Str(s.to_string()),
        },
    }
}

/// Parses an ISO date (`YYYY-MM-DD`) or date-time (`YYYY-MM-DDTHH:MM[:SS[.mmm]][Z]`,
/// UTC; a space instead of `T` works). Returns (ms, has_time).
pub(crate) fn parse_iso(s: &str) -> Option<(i64, bool)> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = crate::markdown::days_from_civil(y, m, d);
    let mut ms = days * 86_400_000;
    if b.len() == 10 {
        return Some((ms, false));
    }
    if !matches!(b[10], b'T' | b' ') || b.len() < 16 || b[13] != b':' {
        return None;
    }
    let (hh, mm) = (num(11..13)?, num(14..16)?);
    let mut ss = 0;
    let mut milli = 0;
    if b.len() >= 19 && b[16] == b':' {
        ss = num(17..19)?;
        if b.len() >= 23 && b[19] == b'.' {
            milli = num(20..23)?;
        }
    }
    ms += ((hh * 60 + mm) * 60 + ss) * 1000 + milli;
    Some((ms, true))
}

// ---------------------------------------------------------------- blocks

struct PBlock {
    kind: BlockKind,
    /// Raw inline markdown (or code text).
    raw: String,
    literal: bool,
    checked: bool,
    target: Option<String>,
    color: Option<String>,
    width: Option<u32>,
    numbered_start: Option<u32>,
    children: Vec<PBlock>,
}

impl PBlock {
    fn new(kind: BlockKind, raw: &str) -> PBlock {
        PBlock {
            kind,
            raw: raw.to_string(),
            literal: false,
            checked: false,
            target: None,
            color: None,
            width: None,
            numbered_start: None,
            children: Vec::new(),
        }
    }
    fn textual(&self) -> bool {
        !self.literal
            && matches!(
                self.kind,
                BlockKind::Paragraph
                    | BlockKind::Heading1
                    | BlockKind::Heading2
                    | BlockKind::Heading3
                    | BlockKind::Bulleted
                    | BlockKind::Numbered
                    | BlockKind::Todo
                    | BlockKind::Toggle
                    | BlockKind::ToggleHeading1
                    | BlockKind::ToggleHeading2
                    | BlockKind::ToggleHeading3
            )
    }
}

/// A block's metadata comment (see `markdown`): `<!-- toggle color=red -->`.
#[derive(Default)]
struct Meta {
    block: Option<AnnotatedBlock>,
    kind: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    width: Option<u32>,
}

/// Splits a trailing metadata comment off a block's first line. Comments
/// with other words are left alone (they are ordinary text).
fn split_meta(s: &str) -> (&str, Option<Meta>) {
    let t = s.trim_end_matches(['\r', ' ']);
    let Some(body) = t.strip_suffix("-->") else {
        return (s, None);
    };
    let Some(start) = body.rfind("<!--") else {
        return (s, None);
    };
    let before = &t[..start];
    if !before.is_empty() && !before.ends_with(' ') {
        return (s, None);
    }
    let mut meta = Meta::default();
    let words: Vec<&str> = body[start + 4..].split_whitespace().collect();
    if words.is_empty() {
        return (s, None);
    }
    for w in words {
        match w.split_once('=') {
            Some(("block", v)) if meta.block.is_none() => {
                let Some(block) = decode_annotation(v) else {
                    return (s, None);
                };
                meta.block = Some(block);
            }
            Some(("icon", v)) if !v.is_empty() => meta.icon = Some(v.to_string()),
            Some(("color", v)) if !v.is_empty() => meta.color = Some(v.to_string()),
            Some(("width", v)) if v.parse::<u32>().is_ok_and(|w| w > 0 && w < 100) => {
                meta.width = v.parse().ok()
            }
            None if matches!(w, "toggle" | "callout" | "columns" | "column" | "toc")
                && meta.kind.is_none() =>
            {
                meta.kind = Some(w.to_string())
            }
            _ => return (s, None),
        }
    }
    let before = if meta.block.is_some() {
        before.strip_suffix(' ').unwrap_or(before)
    } else {
        before.trim_end()
    };
    (before, Some(meta))
}

fn is_divider(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

fn heading(s: &str) -> Option<(usize, &str)> {
    let n = s.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&n) {
        let rest = &s[n..];
        if rest.is_empty() {
            return Some((n, ""));
        }
        if let Some(r) = rest.strip_prefix(' ') {
            return Some((n, r.trim_end_matches(['#', ' '])));
        }
    }
    None
}

/// (checked or not a todo, rest) for list markers.
fn bullet(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if matches!(b.first(), Some(b'-' | b'*' | b'+')) {
        if b.len() == 1 {
            return Some("");
        }
        if b[1] == b' ' {
            return Some(&s[2..]);
        }
    }
    None
}

fn todo(rest: &str) -> Option<(bool, &str)> {
    for (pre, checked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(r) = rest.strip_prefix(pre) {
            if r.is_empty() {
                return Some((checked, ""));
            }
            if let Some(r) = r.strip_prefix(' ') {
                return Some((checked, r));
            }
        }
    }
    None
}

fn numbered(s: &str) -> Option<(u32, &str)> {
    let n = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if (1..=9).contains(&n) && matches!(s.as_bytes().get(n), Some(b'.' | b')')) {
        let rest = &s[n + 1..];
        let number = s[..n].parse().ok()?;
        let text = if rest.is_empty() { "" } else { rest.strip_prefix(' ')? };
        return Some((number, text));
    }
    None
}

fn fence(s: &str) -> Option<&str> {
    if s.starts_with("```") {
        Some("```")
    } else if s.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// The cells of a table row and how many `|` separate them. A pipe that is
/// escaped (`\|`) or inside a code span is cell content.
fn table_cells(row: &str) -> (Vec<&str>, usize) {
    let t = row.trim();
    let b = t.as_bytes();
    let mut seps = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'`' => {
                let run = b[i..].iter().take_while(|&&c| c == b'`').count();
                let mut j = i + run;
                let mut close = None;
                while j < b.len() {
                    if b[j] == b'`' {
                        let n = b[j..].iter().take_while(|&&c| c == b'`').count();
                        if n == run {
                            close = Some(j + n);
                            break;
                        }
                        j += n;
                    } else {
                        j += 1;
                    }
                }
                i = close.unwrap_or(i + run);
            }
            b'|' => {
                seps.push(i);
                i += 1;
            }
            _ => i += 1,
        }
    }
    let mut cells = Vec::new();
    let mut start = 0;
    for &s in &seps {
        cells.push(&t[start..s]);
        start = s + 1;
    }
    cells.push(&t[start..]);
    if seps.first() == Some(&0) {
        cells.remove(0);
    }
    if seps.last().is_some_and(|&s| s + 1 == b.len()) {
        cells.pop();
    }
    (cells.into_iter().map(str::trim).collect(), seps.len())
}

/// A delimiter cell: `---`, `:--`, `--:` or `:-:`.
fn is_delim_cell(c: &str) -> bool {
    let c = c.strip_prefix(':').unwrap_or(c);
    let c = c.strip_suffix(':').unwrap_or(c);
    !c.is_empty() && c.bytes().all(|b| b == b'-')
}

/// Whether a GFM table starts at `lines[i]`: a header row with a pipe, then
/// a delimiter row with as many cells. The header may follow a paragraph line
/// directly (no blank line needed).
fn table_start(lines: &[&str], i: usize, ind: usize) -> bool {
    let (Some(&head), Some(&delim)) = (lines.get(i), lines.get(i + 1)) else {
        return false;
    };
    if indent_of(head) < ind || indent_of(delim) < ind {
        return false;
    }
    let head = strip_indent(head, ind).trim_end_matches('\r');
    let delim = strip_indent(delim, ind).trim_end_matches('\r');
    // Lines that begin another kind of block are not table headers.
    if heading(head).is_some()
        || bullet(head).is_some()
        || numbered(head).is_some()
        || fence(head).is_some()
        || head.starts_with('>')
    {
        return false;
    }
    let (head_cells, head_seps) = table_cells(head);
    let (delim_cells, delim_seps) = table_cells(delim);
    head_seps > 0
        && !head_cells.is_empty()
        && delim_seps > 0
        && delim_cells.len() == head_cells.len()
        && delim_cells.iter().all(|c| is_delim_cell(c))
}

/// A `.md` note link target (`Notes/Home.md#Intro`, `page.MD`). The extension
/// is checked before any `#fragment`.
pub(crate) fn md_target(target: &str) -> bool {
    let path = decode_path(target.split('#').next().unwrap_or(""));
    path.to_ascii_lowercase().ends_with(".md")
}

/// Percent-decode a local path without changing its original link spelling.
pub(crate) fn decode_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                decoded.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| path.to_string())
}

/// Whether a line (without indentation) starts a new block.
fn starts_block(s: &str) -> bool {
    split_meta(s).1.is_some_and(|m| m.block.is_some())
        || heading(s).is_some()
        || bullet(s).is_some()
        || numbered(s).is_some()
        || fence(s).is_some()
        || s.starts_with('>')
        || is_divider(s)
        || whole_line_special(s)
}

fn whole_line_special(s: &str) -> bool {
    let t = s.trim_end();
    t == "<br>"
        || (t.starts_with("![[") && t.ends_with("]]"))
        || (t.starts_with("![") && t.ends_with(')') && t.contains("]("))
        || (t.starts_with('<') && t.contains('>') && !t.starts_with("<br"))
}

/// `[name](path)` filling the whole line.
fn whole_link(s: &str) -> Option<(&str, &str)> {
    let t = s.trim_end();
    let inner = t.strip_prefix('[')?.strip_suffix(')')?;
    let mid = inner.find("](")?;
    let (name, path) = (&inner[..mid], &inner[mid + 2..]);
    if name.contains(']') || path.contains(' ') && !path.contains("%20") && path.contains('(') {
        return None;
    }
    Some((name, path))
}

fn decode_url(u: &str) -> String {
    u.replace("%20", " ").replace("%29", ")")
}

pub(crate) fn is_web(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:")
}

struct Parser<'a, 'r> {
    lines: Vec<&'a str>,
    /// The same lines split on `\n` alone: a table keeps the `\r` of its rows.
    raw: Vec<&'a str>,
    i: usize,
    r: &'r mut dyn Resolver,
}

impl Parser<'_, '_> {
    /// Blocks whose first line is indented at least `min` columns.
    fn level(&mut self, min: usize) -> Vec<PBlock> {
        let mut out: Vec<PBlock> = Vec::new();
        let mut level: Option<usize> = None;
        let mut blank = false;
        while self.i < self.lines.len() {
            let line = self.lines[self.i];
            if line.trim().is_empty() {
                blank = true;
                self.i += 1;
                continue;
            }
            let ind = indent_of(line);
            if ind < min {
                break;
            }
            let li = *level.get_or_insert(ind);
            if ind < li {
                break;
            }
            let content = strip_indent(line, ind).trim_end_matches('\r');
            if let Some(last) = out.last_mut() {
                let cont_ok = !blank
                    && last.textual()
                    && !starts_block(content)
                    && !table_start(&self.lines, self.i, ind)
                    && (ind > li
                        || matches!(
                            last.kind,
                            BlockKind::Paragraph
                                | BlockKind::Bulleted
                                | BlockKind::Numbered
                                | BlockKind::Todo
                        ));
                if cont_ok && last.children.is_empty() {
                    last.raw.push('\n');
                    last.raw.push_str(content);
                    self.i += 1;
                    continue;
                }
                if ind > li {
                    let kids = self.level(ind);
                    if out.last().is_some_and(|last| matches!(last.kind, BlockKind::Query | BlockKind::View)) {
                        // Query and view cards are leaves; following indented blocks
                        // remain separate blocks rather than becoming children.
                        out.extend(kids);
                    } else {
                        out.last_mut().expect("last").children.extend(kids);
                    }
                    blank = false;
                    continue;
                }
            }
            blank = false;
            let mut b = self.block(content, ind);
            // Only the first marker determines a numbered run's start.
            // Subsequent authored numbers are normalized by the renderer.
            if out.last().is_some_and(|last| last.kind == BlockKind::Numbered) {
                b.numbered_start = None;
            }
            out.push(b);
        }
        out
    }

    /// A GFM table at the current line, kept verbatim: its rows run until a
    /// blank line, a line that starts another block or
    /// the next table. Rows are joined with their own newline bytes (`\r\n` in
    /// a CRLF file); the last row has none of its own.
    fn table(&mut self, ind: usize) -> PBlock {
        let mut rows: Vec<&str> = Vec::new();
        while self.i < self.lines.len() {
            let line = self.raw[self.i];
            // The header and delimiter rows were checked by `table_start`.
            if rows.len() >= 2 {
                let content = strip_indent(line, ind).trim_end_matches('\r');
                if line.trim().is_empty()
                    || indent_of(line) < ind
                    || starts_block(content)
                    || table_start(&self.lines, self.i, ind)
                {
                    break;
                }
            }
            rows.push(line);
            self.i += 1;
        }
        let mut text = rows.join("\n");
        if text.ends_with('\r') {
            text.pop();
        }
        let mut b = PBlock::new(BlockKind::Table, &text);
        b.literal = true;
        b
    }

    /// Parses the block starting at the current line (consumes its lines).
    fn block(&mut self, s: &str, ind: usize) -> PBlock {
        if table_start(&self.lines, self.i, ind) {
            return self.table(ind);
        }
        let (s, meta) = split_meta(s);
        let Some(meta) = meta else {
            return self.plain_block(s, ind);
        };
        if let Some(block) = meta.block {
            self.i += 1;
            let fallback = |line: &str| {
                line.strip_prefix('>')
                    .map(|s| s.strip_prefix(' ').unwrap_or(s))
                    .unwrap_or(line)
                    .to_owned()
            };
            let mut b = PBlock::new(BlockKind::parse(&block.kind), &fallback(s));
            // The line count preserves blank lines and prevents fallback text
            // from absorbing neighbouring blocks or being read as headings.
            for _ in 1..block.lines {
                let Some(line) = self.lines.get(self.i) else {
                    break;
                };
                b.raw.push('\n');
                b.raw
                    .push_str(&fallback(strip_indent(line, ind).trim_end_matches('\r')));
                self.i += 1;
            }
            // An embed's title is looked up when shown, so its text is not kept.
            if b.kind == BlockKind::Embed {
                b.raw.clear();
            }
            b.target = block.target;
            b.checked = block.checked;
            b.color = meta.color;
            b.width = meta.width;
            return b;
        }
        let lone = match meta.kind.as_deref() {
            Some("columns") => Some(BlockKind::ColumnList),
            Some("column") => Some(BlockKind::Column),
            Some("toc") => Some(BlockKind::TableOfContents),
            _ => None,
        };
        let mut b = match lone {
            Some(kind) if s.trim().is_empty() => {
                self.i += 1;
                let mut b = PBlock::new(kind, "");
                b.literal = true;
                b
            }
            _ => self.plain_block(s, ind),
        };
        match (meta.kind.as_deref(), &b.kind) {
            (Some("toggle"), BlockKind::Bulleted) => b.kind = BlockKind::Toggle,
            (Some("toggle"), BlockKind::Heading1) => b.kind = BlockKind::ToggleHeading1,
            (Some("toggle"), BlockKind::Heading2) => b.kind = BlockKind::ToggleHeading2,
            (Some("toggle"), BlockKind::Heading3) => b.kind = BlockKind::ToggleHeading3,
            (Some("callout"), BlockKind::Quote) => {
                b.kind = BlockKind::Callout;
                b.target = meta.icon.clone();
            }
            _ => {}
        }
        b.color = meta.color;
        b.width = meta.width;
        b
    }

    /// A block without a metadata comment.
    fn plain_block(&mut self, s: &str, ind: usize) -> PBlock {
        self.i += 1;
        if let Some(f) = fence(s) {
            // The info string after the fence (`rust`) is the block's target.
            let run = s.bytes().take_while(|&c| c == f.as_bytes()[0]).count();
            let info = s[run..].trim();
            let query = info == "query";
            let view = info == "view";
            let mut text = Vec::new();
            while self.i < self.lines.len() {
                let l = self.lines[self.i];
                self.i += 1;
                let closing = strip_indent(l, usize::MAX);
                let close_run = closing.bytes()
                    .take_while(|&c| c == f.as_bytes()[0])
                    .count();
                if closing.starts_with(f)
                    && indent_of(l) <= ind + 3
                    && (!(query || view) || (close_run >= run && closing[close_run..].trim().is_empty()))
                {
                    break;
                }
                text.push(strip_indent(l, ind).trim_end_matches('\r'));
            }
            let kind = if query {
                BlockKind::Query
            } else if view {
                BlockKind::View
            } else {
                BlockKind::Code
            };
            let mut b = PBlock::new(kind, &text.join("\n"));
            b.literal = true;
            b.target = (!(query || view) && !info.is_empty()).then(|| info.to_string());
            if view {
                // `page` belongs to the fence transport, while the stored text
                // contains the view settings and source. Leave invalid JSON
                // intact so it can be corrected in the editor.
                if let Ok(serde_json::Value::Object(mut object)) = serde_json::from_str(&b.raw) {
                    if let Some(page) = object.get("page").and_then(|v| v.as_str()).map(str::to_owned) {
                        object.remove("page");
                        b.target = Some(page);
                        b.raw = serde_json::to_string(&object).expect("view JSON object");
                    }
                }
            }
            return b;
        }
        if let Some(q) = s.strip_prefix('>') {
            let mut raw = vec![q.strip_prefix(' ').unwrap_or(q).to_string()];
            while self.i < self.lines.len() {
                let l = self.lines[self.i];
                let c = strip_indent(l, ind);
                if indent_of(l) != ind || !c.starts_with('>') {
                    break;
                }
                let q = &c[1..];
                raw.push(
                    q.strip_prefix(' ')
                        .unwrap_or(q)
                        .trim_end_matches('\r')
                        .to_string(),
                );
                self.i += 1;
            }
            return PBlock::new(BlockKind::Quote, &raw.join("\n"));
        }
        if let Some((n, rest)) = heading(s) {
            let kind = match n {
                1 => BlockKind::Heading1,
                2 => BlockKind::Heading2,
                _ => BlockKind::Heading3,
            };
            return PBlock::new(kind, rest);
        }
        if is_divider(s) {
            return PBlock::new(BlockKind::Divider, "");
        }
        if let Some(rest) = bullet(s) {
            if let Some((checked, r)) = todo(rest) {
                let mut b = PBlock::new(BlockKind::Todo, r);
                b.checked = checked;
                return b;
            }
            return PBlock::new(BlockKind::Bulleted, rest);
        }
        if let Some((number, rest)) = numbered(s) {
            let mut b = PBlock::new(BlockKind::Numbered, rest);
            b.numbered_start = (number != 1).then_some(number);
            return b;
        }
        let t = s.trim_end();
        if t == "<br>" {
            return PBlock::new(BlockKind::Paragraph, "");
        }
        if let Some(link) = t.strip_suffix(DATABASE_SUFFIX).map(str::trim_end) {
            if let Some(inner) = link.strip_prefix("![[").and_then(|x| x.strip_suffix("]]")) {
                let name = inner.split('|').next().unwrap_or("");
                let name = name.split('#').next().unwrap_or("").trim();
                if let Some(id) = self.r.page(name) {
                    let mut b = PBlock::new(BlockKind::Database, "");
                    b.literal = true;
                    b.target = Some(id);
                    return b;
                }
            }
        }
        if let Some(inner) = t.strip_prefix("![[").and_then(|x| x.strip_suffix("]]")) {
            let target = inner.split('|').next().unwrap_or("");
            let name = target.split('#').next().unwrap_or("").trim();
            let looks_file = name.rsplit_once('.').is_some_and(|(_, e)| {
                !e.eq_ignore_ascii_case("md") && e.len() <= 5 && !e.contains(' ')
            });
            if looks_file {
                if let Some(id) = self.r.file(name) {
                    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
                    let kind = if crate::attach::is_image(name) {
                        BlockKind::Image
                    } else {
                        BlockKind::File
                    };
                    let caption = if kind == BlockKind::Image {
                        ""
                    } else {
                        file_name
                    };
                    let mut b = PBlock::new(kind, caption);
                    b.literal = true;
                    b.target = Some(id);
                    return b;
                }
            } else if let Some(id) = self.r.page(name) {
                let mut b = PBlock::new(BlockKind::LinkToPage, "");
                b.literal = true;
                b.target = Some(id);
                return b;
            } else {
                self.r.unresolved(target.trim());
            }
            let mut b = PBlock::new(BlockKind::Paragraph, t);
            b.literal = true;
            return b;
        }
        if let Some(rest) = t.strip_prefix("![") {
            if let Some((alt, path)) = whole_link(&format!("[{rest}")) {
                let path = decode_url(path);
                if !is_web(&path) {
                    if let Some(id) = self.r.file(&path) {
                        // Image syntax: an image unless the name says otherwise
                        // (blob links have no extension).
                        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
                        let kind = if !file_name.contains('.') || crate::attach::is_image(&path) {
                            BlockKind::Image
                        } else {
                            BlockKind::File
                        };
                        let mut b = PBlock::new(kind, &unescape(alt));
                        b.literal = true;
                        b.target = Some(id);
                        return b;
                    }
                }
            }
        }
        if let Some((name, path)) = whole_link(t) {
            let path = decode_url(path);
            let is_md = md_target(&path);
            if !is_web(&path) && !is_md {
                if let Some(id) = self.r.file(&path) {
                    let mut b = PBlock::new(BlockKind::File, &unescape(name));
                    b.literal = true;
                    b.target = Some(id);
                    return b;
                }
            }
        }
        if let Some(rest) = t.strip_prefix('<') {
            if let Some(end) = rest.find('>') {
                let url = &rest[..end];
                if url.is_empty() || (is_web(url) && !url.contains(' ')) {
                    let mut b = PBlock::new(BlockKind::Bookmark, &unescape(rest[end + 1..].trim()));
                    b.literal = true;
                    b.target = (!url.is_empty()).then(|| url.to_string());
                    return b;
                }
            }
        }
        PBlock::new(BlockKind::Paragraph, s)
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(&n) = it.peek() {
                if n.is_ascii_punctuation() {
                    out.push(n);
                    it.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

fn finish(b: PBlock, r: &mut dyn Resolver) -> Block {
    let (text, marks) = if b.literal {
        let marks = if matches!(b.kind, BlockKind::Table) {
            table_marks(&b.raw, r)
        } else {
            Vec::new()
        };
        (b.raw, marks)
    } else {
        parse_inline(&b.raw, r)
    };
    Block {
        id: String::new(),
        kind: b.kind,
        text,
        marks,
        checked: b.checked,
        children: b.children.into_iter().map(|c| finish(c, r)).collect(),
        target: b.target,
        color: b.color,
        width: b.width,
        numbered_start: b.numbered_start,
    }
}

/// Parses a markdown body into blocks (with empty ids).
pub(crate) fn parse_blocks(body: &str, r: &mut dyn Resolver) -> Vec<Block> {
    let lines: Vec<&str> = body.lines().collect();
    // Same indexes as `lines`, with each line's `\r` left in place.
    let raw: Vec<&str> = body.split('\n').collect();
    let mut p = Parser {
        lines,
        raw,
        i: 0,
        r,
    };
    let mut out = Vec::new();
    while p.i < p.lines.len() {
        let before = p.i;
        out.extend(p.level(0));
        if p.i == before {
            p.i += 1;
        }
    }
    out.into_iter().map(|b| finish(b, r)).collect()
}

// ---------------------------------------------------------------- inline

struct Open {
    kind: MarkKind,
    delim: &'static str,
    /// Char position in the output where the mark starts.
    pos: u32,
}

/// Inline markdown -> (text, marks).
pub(crate) fn parse_inline(raw: &str, r: &mut dyn Resolver) -> (String, Vec<Mark>) {
    let chars: Vec<char> = raw.chars().collect();
    let mut text = String::new();
    let mut marks = Vec::new();
    inline_into(&chars, r, &mut text, &mut marks);
    (text, crate::doc::normalize_marks(&marks))
}

fn find_seq(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    if pat.is_empty() || chars.len() < pat.len() {
        return None;
    }
    (from..=chars.len() - pat.len()).find(|&i| chars[i..i + pat.len()] == *pat)
}

/// Index of the `]` matching the `[` at `open`, skipping escapes and nesting.
fn matching_bracket(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Marks for the `.md` links and wikilinks in a table's text. The text is kept
/// as it is: each mark covers the raw label or alias, so it stays valid on the
/// literal source. Rows are scanned one at a time, so a link never runs into
/// the next row.
fn table_marks(text: &str, r: &mut dyn Resolver) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut base = 0u32;
    for row in text.split('\n') {
        let chars: Vec<char> = row.chars().collect();
        scan_table_links(&chars, base, r, &mut marks);
        base += chars.len() as u32 + 1;
    }
    crate::doc::normalize_marks(&marks)
}

/// Reads the links in `chars` the way `inline_into` does, but only pushes
/// marks for `.md` links and wikilinks. `base` is the position of `chars[0]`
/// in the table text. Code spans and escaped brackets are skipped.
fn scan_table_links(chars: &[char], base: u32, r: &mut dyn Resolver, marks: &mut Vec<Mark>) {
    let at = |k: usize| base + k as u32;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => i += 2,
            '`' => {
                let run = chars[i..].iter().take_while(|&&x| x == '`').count();
                let delim = vec!['`'; run];
                i = match find_seq(chars, i + run, &delim) {
                    Some(end) if end > i + run => end + run,
                    _ => i + run,
                };
            }
            '[' if chars.get(i + 1) == Some(&'[') => match find_seq(chars, i + 2, &[']', ']']) {
                Some(end) => {
                    wiki_in_table(chars, i + 2, end, base, r, marks);
                    i = end + 2;
                }
                None => i += 1,
            },
            '[' => {
                let link = matching_bracket(chars, i).and_then(|close| {
                    if chars.get(close + 1) != Some(&'(') {
                        return None;
                    }
                    let end = (close + 2..chars.len()).find(|&k| chars[k] == ')')?;
                    Some((close, end))
                });
                match link {
                    Some((close, end)) => {
                        // Links inside the label count too, as they do in paragraphs.
                        scan_table_links(&chars[i + 1..close], at(i + 1), r, marks);
                        let raw: String = chars[close + 2..end].iter().collect();
                        let raw = raw.trim().to_string();
                        let url = decode_url(&raw);
                        if !is_web(&url) && md_target(&url) {
                            let found = r.markdown_page(&raw);
                            if found.is_none() {
                                r.unresolved(&raw);
                            }
                            // A missing note keeps its target; only the page link is absent.
                            if close > i + 1 {
                                if let Some(id) = found {
                                    marks.push(Mark {
                                        start: at(i + 1),
                                        end: at(close),
                                        kind: MarkKind::PageLink(id),
                                    });
                                }
                                marks.push(Mark {
                                    start: at(i + 1),
                                    end: at(close),
                                    kind: MarkKind::Link(raw),
                                });
                            }
                        }
                        i = end + 1;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
}

/// A wikilink whose inner text is `chars[lo..hi]` (between `[[` and `]]`). A
/// resolved link is marked over its alias, or over its target when it has
/// none. A missing one stays literal and is reported. In a table, `\|` is the
/// alias separator, so the backslash is not part of the target.
fn wiki_in_table(
    chars: &[char],
    lo: usize,
    hi: usize,
    base: u32,
    r: &mut dyn Resolver,
    marks: &mut Vec<Mark>,
) {
    let at = |k: usize| base + k as u32;
    let bar = chars[lo..hi].iter().position(|&c| c == '|').map(|p| lo + p);
    let target_end = match bar {
        Some(b) if b > lo && chars[b - 1] == '\\' => b - 1,
        Some(b) => b,
        None => hi,
    };
    let target: String = chars[lo..target_end].iter().collect();
    let t = target.trim();
    let name = t.split('#').next().unwrap_or("").trim();
    let found = r.page(t).or_else(|| r.page(name));
    match found {
        Some(id) => {
            let (from, to) = match bar {
                Some(b) => (b + 1, hi),
                None => (lo, hi),
            };
            if to > from {
                if r.preserve_wikilink_target(t) {
                    marks.push(Mark {
                        start: at(from),
                        end: at(to),
                        kind: MarkKind::Link(t.to_string()),
                    });
                }
                marks.push(Mark {
                    start: at(from),
                    end: at(to),
                    kind: MarkKind::PageLink(id),
                });
            }
            if bar.is_some() {
                scan_table_links(&chars[from..to], at(from), r, marks);
            }
        }
        None => r.unresolved(t),
    }
}

fn inline_into(chars: &[char], r: &mut dyn Resolver, text: &mut String, marks: &mut Vec<Mark>) {
    let mut len = text.chars().count() as u32;
    let mut stack: Vec<Open> = Vec::new();
    let push = |text: &mut String, len: &mut u32, c: char| {
        text.push(c);
        *len += 1;
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => {
                push(text, &mut len, chars[i + 1]);
                i += 2;
            }
            '`' => {
                let run = chars[i..].iter().take_while(|&&x| x == '`').count();
                let delim: Vec<char> = vec!['`'; run];
                match find_seq(chars, i + run, &delim) {
                    Some(end) if end > i + run => {
                        let start = len;
                        let mut content = &chars[i + run..end];
                        // CommonMark removes one padding space at each end,
                        // except when the entire code span consists of spaces.
                        if content.first() == Some(&' ')
                            && content.last() == Some(&' ')
                            && content.iter().any(|&x| x != ' ')
                        {
                            content = &content[1..content.len() - 1];
                        }
                        for &x in content {
                            push(text, &mut len, x);
                        }
                        marks.push(Mark {
                            start,
                            end: len,
                            kind: MarkKind::Code,
                        });
                        i = end + run;
                    }
                    _ => {
                        for _ in 0..run {
                            push(text, &mut len, '`');
                        }
                        i += run;
                    }
                }
            }
            '[' if chars.get(i + 1) == Some(&'[') => match find_seq(chars, i + 2, &[']', ']']) {
                Some(end) => {
                    let inner: String = chars[i + 2..end].iter().collect();
                    let (target, alias) = match inner.split_once('|') {
                        Some((t, a)) => (t.to_string(), Some(a.to_string())),
                        None => (inner.clone(), None),
                    };
                    let name = target.split('#').next().unwrap_or("").trim().to_string();
                    let found = r.page(target.trim()).or_else(|| r.page(&name));
                    match found {
                        // A missing page is reported with its anchor (`Home#Intro`).
                        Some(id) => {
                            let start = len;
                            match alias {
                                Some(a) => {
                                    let a: Vec<char> = a.chars().collect();
                                    inline_into(&a, r, text, marks);
                                }
                                None => {
                                    text.push_str(&target);
                                }
                            }
                            len = text.chars().count() as u32;
                            if len > start {
                                // The target as written (`Home#Intro`) rides
                                // along with the page, so the link is written
                                // back the way the user wrote it.
                                if r.preserve_wikilink_target(target.trim()) {
                                    marks.push(Mark {
                                        start,
                                        end: len,
                                        kind: MarkKind::Link(target.trim().to_string()),
                                    });
                                }
                                marks.push(Mark {
                                    start,
                                    end: len,
                                    kind: MarkKind::PageLink(id),
                                });
                            }
                        }
                        None => {
                            r.unresolved(target.trim());
                            for &x in &chars[i..end + 2] {
                                push(text, &mut len, x);
                            }
                        }
                    }
                    i = end + 2;
                }
                None => {
                    push(text, &mut len, '[');
                    i += 1;
                }
            },
            '[' => {
                let link = matching_bracket(chars, i).and_then(|close| {
                    if chars.get(close + 1) != Some(&'(') {
                        return None;
                    }
                    let end = (close + 2..chars.len()).find(|&k| chars[k] == ')')?;
                    Some((close, end))
                });
                match link {
                    Some((close, end)) => {
                        let raw: String = chars[close + 2..end].iter().collect();
                        let raw = raw.trim().to_string();
                        let url = decode_url(&raw);
                        let start = len;
                        inline_into(&chars[i + 1..close], r, text, marks);
                        len = text.chars().count() as u32;
                        if !is_web(&url) && md_target(&url) {
                            // The target as written (path and fragment included)
                            // goes to the resolver's `markdown_page`; the mark
                            // keeps the same target.
                            let found = r.markdown_page(&raw);
                            if found.is_none() {
                                r.unresolved(&raw);
                            }
                            if len > start {
                                if let Some(id) = found {
                                    marks.push(Mark {
                                        start,
                                        end: len,
                                        kind: MarkKind::PageLink(id),
                                    });
                                }
                                marks.push(Mark {
                                    start,
                                    end: len,
                                    kind: MarkKind::Link(raw),
                                });
                            }
                        } else if len > start {
                            marks.push(Mark {
                                start,
                                end: len,
                                kind: MarkKind::Link(url),
                            });
                        }
                        i = end + 1;
                    }
                    None => {
                        push(text, &mut len, '[');
                        i += 1;
                    }
                }
            }
            '<' if inline_tag(chars, i).is_some() => {
                let (kind, from, to, next) = inline_tag(chars, i).expect("tag");
                let start = len;
                inline_into(&chars[from..to], r, text, marks);
                len = text.chars().count() as u32;
                if len > start {
                    marks.push(Mark {
                        start,
                        end: len,
                        kind,
                    });
                }
                i = next;
            }
            '<' => {
                let end = (i + 1..chars.len()).find(|&k| chars[k] == '>' || chars[k] == ' ');
                let url: String = match end {
                    Some(e) if chars[e] == '>' => chars[i + 1..e].iter().collect(),
                    _ => String::new(),
                };
                if is_web(&url) {
                    let start = len;
                    for ch in url.chars() {
                        push(text, &mut len, ch);
                    }
                    marks.push(Mark {
                        start,
                        end: len,
                        kind: MarkKind::Link(url.clone()),
                    });
                    i = end.unwrap_or(i) + 1;
                } else {
                    push(text, &mut len, '<');
                    i += 1;
                }
            }
            '*' | '_' | '~' => {
                let run = chars[i..].iter().take_while(|&&x| x == c).count();
                let prev = if i > 0 { Some(chars[i - 1]) } else { None };
                let next = chars.get(i + run).copied();
                let mut can_open = next.is_some_and(|n| !n.is_whitespace());
                let mut can_close = prev.is_some_and(|p| !p.is_whitespace());
                if c == '_' {
                    can_open &= !prev.is_some_and(|p| p.is_alphanumeric());
                    can_close &= !next.is_some_and(|n| n.is_alphanumeric());
                }
                let mut n = run;
                if c == '~' {
                    if run != 2 {
                        for _ in 0..run {
                            push(text, &mut len, '~');
                        }
                        i += run;
                        continue;
                    }
                    if can_close && stack.last().is_some_and(|o| o.delim == "~~") {
                        let o = stack.pop().unwrap();
                        if len > o.pos {
                            marks.push(Mark {
                                start: o.pos,
                                end: len,
                                kind: o.kind,
                            });
                        }
                    } else if can_open {
                        stack.push(Open {
                            kind: MarkKind::Strike,
                            delim: "~~",
                            pos: len,
                        });
                    } else {
                        push(text, &mut len, '~');
                        push(text, &mut len, '~');
                    }
                    i += run;
                    continue;
                }
                let (d1, d2) = if c == '*' { ("*", "**") } else { ("_", "__") };
                while n > 0 {
                    let top = stack.last().map(|o| o.delim);
                    if can_close && top == Some(d2) && n >= 2 {
                        close_top(&mut stack, marks, len);
                        n -= 2;
                    } else if can_close && top == Some(d1) {
                        close_top(&mut stack, marks, len);
                        n -= 1;
                    } else if can_open {
                        let (kind, delim, used) = if n >= 2 {
                            (MarkKind::Bold, d2, 2)
                        } else {
                            (MarkKind::Italic, d1, 1)
                        };
                        stack.push(Open {
                            kind,
                            delim,
                            pos: len,
                        });
                        n -= used;
                        // Once opened, the rest of the run only opens.
                        can_close = false;
                    } else {
                        for _ in 0..n {
                            push(text, &mut len, c);
                        }
                        n = 0;
                    }
                }
                i += run;
            }
            _ => {
                push(text, &mut len, c);
                i += 1;
            }
        }
    }
    // Unclosed openers become literal text again (last first, so earlier
    // positions stay valid).
    while let Some(o) = stack.pop() {
        let byte = text
            .char_indices()
            .nth(o.pos as usize)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        text.insert_str(byte, o.delim);
        let n = o.delim.chars().count() as u32;
        for m in marks.iter_mut() {
            if m.start >= o.pos {
                m.start += n;
            }
            if m.end > o.pos {
                m.end += n;
            }
        }
        for s in stack.iter_mut() {
            if s.pos > o.pos {
                s.pos += n;
            }
        }
    }
}

fn starts_at(chars: &[char], i: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(j, p)| chars.get(i + j) == Some(&p))
}

/// Underline, colour, query or date tags.
/// at `i`: (mark, inner start, inner end, index after the closing tag).
fn inline_tag(chars: &[char], i: usize) -> Option<(MarkKind, usize, usize, usize)> {
    let valued = |open: &str, close: &str| -> Option<(String, usize, usize, usize)> {
        if !starts_at(chars, i, open) {
            return None;
        }
        let vs = i + open.chars().count();
        let ve = (vs..chars.len()).find(|&k| chars[k] == '"')?;
        if chars.get(ve + 1) != Some(&'>') {
            return None;
        }
        let close: Vec<char> = close.chars().collect();
        // Query and colour marks share <span>; pair nested tags so both
        // marks survive instead of consuming an inner mark's closing tag.
        let mut end = ve + 2;
        let mut depth = 1;
        loop {
            if end >= chars.len() {
                return None;
            }
            if chars[end] == '\\' {
                end += 2;
                continue;
            }
            if chars.get(end..end + close.len()) == Some(close.as_slice()) {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                end += close.len();
            } else if starts_at(chars, end, "<span")
                && close.as_slice() == ['<', '/', 's', 'p', 'a', 'n', '>']
                && chars.get(end + 5).is_some_and(|c| c.is_whitespace() || *c == '>')
            {
                depth += 1;
                end += 5;
            } else {
                end += 1;
            }
        }
        Some((
            chars[vs..ve].iter().collect(),
            ve + 2,
            end,
            end + close.len(),
        ))
    };
    if starts_at(chars, i, "<u>") {
        let end = find_seq(chars, i + 3, &['<', '/', 'u', '>'])?;
        return Some((MarkKind::Underline, i + 3, end, end + 4));
    }
    if let Some((v, a, b, n)) = valued("<span data-color=\"", "</span>") {
        return Some((MarkKind::Color(v), a, b, n));
    }
    if let Some((v, a, b, n)) = valued("<span data-query=\"", "</span>") {
        // Decode ampersands last, keeping literal entity-looking query text.
        let spec = v.replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#10;", "\n")
            .replace("&#13;", "\r")
            .replace("&#9;", "\t")
            .replace("&amp;", "&");
        return Some((MarkKind::Query(spec), a, b, n));
    }
    if let Some((v, a, b, n)) = valued("<time datetime=\"", "</time>") {
        return Some((MarkKind::Date(v), a, b, n));
    }
    None
}

fn close_top(stack: &mut Vec<Open>, marks: &mut Vec<Mark>, len: u32) {
    let o = stack.pop().expect("open mark");
    if len > o.pos {
        marks.push(Mark {
            start: o.pos,
            end: len,
            kind: o.kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoLinks;
    impl Resolver for NoLinks {
        fn preserve_wikilink_target(&self, _target: &str) -> bool {
            true
        }
        fn page(&mut self, t: &str) -> Option<String> {
            (t == "Home").then(|| "P1".to_string())
        }
        fn file(&mut self, p: &str) -> Option<String> {
            p.ends_with(".png").then(|| "F1".to_string())
        }
    }

    fn m(start: u32, end: u32, kind: MarkKind) -> Mark {
        Mark { start, end, kind }
    }

    #[test]
    fn inline_marks() {
        let r = &mut NoLinks;
        assert_eq!(
            parse_inline("**ab *cd***", r),
            (
                "ab cd".into(),
                vec![m(0, 5, MarkKind::Bold), m(3, 5, MarkKind::Italic)]
            )
        );
        assert_eq!(
            parse_inline("***ab* cd**", r),
            (
                "ab cd".into(),
                vec![m(0, 5, MarkKind::Bold), m(0, 2, MarkKind::Italic)]
            )
        );
        assert_eq!(
            parse_inline("go [here](https://x.y) a\\*b `c*d`", r),
            (
                "go here a*b c*d".into(),
                vec![
                    m(3, 7, MarkKind::Link("https://x.y".into())),
                    m(12, 15, MarkKind::Code)
                ]
            )
        );
        assert_eq!(
            parse_inline("see [[Home|**ho**me]] and [[Nope]]", r),
            (
                "see home and [[Nope]]".into(),
                vec![
                    m(4, 6, MarkKind::Bold),
                    m(4, 8, MarkKind::Link("Home".into())),
                    m(4, 8, MarkKind::PageLink("P1".into()))
                ]
            )
        );
        assert_eq!(
            parse_inline("snake_case_name 2 * 3", r).0,
            "snake_case_name 2 * 3"
        );
        assert_eq!(parse_inline("*open only", r), ("*open only".into(), vec![]));
        assert_eq!(
            parse_inline("~~gone~~ <https://a.b>", r),
            (
                "gone https://a.b".into(),
                vec![
                    m(0, 4, MarkKind::Strike),
                    m(5, 16, MarkKind::Link("https://a.b".into()))
                ]
            )
        );
    }

    /// Resolves `Home` (wikilinks) and `Notes/Home.md#Intro` (`.md` links, with
    /// the path and fragment as written) to P1, and records every `.md`
    /// target and every miss.
    #[derive(Default)]
    struct Recorder {
        md: Vec<String>,
        missing: Vec<String>,
    }
    impl Resolver for Recorder {
        fn preserve_wikilink_target(&self, _target: &str) -> bool {
            true
        }
        fn page(&mut self, t: &str) -> Option<String> {
            (t == "Home").then(|| "P1".to_string())
        }
        fn markdown_page(&mut self, t: &str) -> Option<String> {
            self.md.push(t.to_string());
            (t == "Notes/Home.md#Intro").then(|| "P1".to_string())
        }
        fn file(&mut self, _p: &str) -> Option<String> {
            None
        }
        fn unresolved(&mut self, t: &str) {
            self.missing.push(t.to_string());
        }
    }

    #[test]
    fn md_links_pair_page_and_target() {
        let mut r = Recorder::default();
        let (text, marks) = parse_inline(
            "[a](Notes/Home.md#Intro) [b](Gone.MD) [c](https://x.y/a.md)",
            &mut r,
        );
        assert_eq!(text, "a b c");
        assert_eq!(marks.len(), 4);
        assert!(marks.contains(&m(0, 1, MarkKind::PageLink("P1".into()))));
        assert!(marks.contains(&m(0, 1, MarkKind::Link("Notes/Home.md#Intro".into()))));
        assert!(marks.contains(&m(2, 3, MarkKind::Link("Gone.MD".into()))));
        assert!(marks.contains(&m(4, 5, MarkKind::Link("https://x.y/a.md".into()))));
        // The resolver gets the target as written, path and fragment included.
        assert_eq!(r.md, vec!["Notes/Home.md#Intro", "Gone.MD"]);
        assert_eq!(r.missing, vec!["Gone.MD"]);
    }

    /// Only `page` is written: `.md` targets reach it by their file name.
    #[derive(Default)]
    struct Titles {
        seen: Vec<String>,
    }
    impl Resolver for Titles {
        fn page(&mut self, t: &str) -> Option<String> {
            self.seen.push(t.to_string());
            matches!(t, "My Page" | "Other").then(|| format!("id-{t}"))
        }
        fn file(&mut self, _p: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn md_targets_default_to_their_file_name() {
        let mut r = Titles::default();
        let (text, marks) = parse_inline(
            "[a](Notes/My%20Page.md#Intro) [b](dir\\Other.MD) [c](Gone.md)",
            &mut r,
        );
        assert_eq!(text, "a b c");
        assert_eq!(r.seen, ["My Page", "Other", "Gone"]);
        assert!(marks.contains(&m(0, 1, MarkKind::PageLink("id-My Page".into()))));
        assert!(marks.contains(&m(0, 1, MarkKind::Link("Notes/My%20Page.md#Intro".into()))));
        assert!(marks.contains(&m(2, 3, MarkKind::PageLink("id-Other".into()))));
        assert!(marks.contains(&m(2, 3, MarkKind::Link("dir\\Other.MD".into()))));
        assert!(marks.contains(&m(4, 5, MarkKind::Link("Gone.md".into()))));
        assert_eq!(marks.len(), 5);
    }

    #[test]
    fn missing_wikilink_reports_its_anchor() {
        let mut r = Recorder::default();
        let (text, marks) = parse_inline("[[Home#Intro|here]] [[Nope#Sec|shown]]", &mut r);
        assert_eq!(text, "here [[Nope#Sec|shown]]");
        assert_eq!(
            marks,
            vec![
                m(0, 4, MarkKind::Link("Home#Intro".into())),
                m(0, 4, MarkKind::PageLink("P1".into()))
            ]
        );
        assert_eq!(r.missing, vec!["Nope#Sec"]);
    }

    #[test]
    fn tables_interrupt_paragraphs_and_stay_distinct() {
        let src = "Intro line\n| a | b |\n|:-|-:|\n| `x|y` | c \\| d |\n\nafter\n| p |\n|---|\n| q |\n| r |\n|---|\n| s |\n";
        let blocks = parse_blocks(src, &mut Recorder::default());
        let kinds: Vec<BlockKind> = blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                BlockKind::Paragraph,
                BlockKind::Table,
                BlockKind::Paragraph,
                BlockKind::Table,
                BlockKind::Table,
            ]
        );
        assert_eq!(blocks[1].text, "| a | b |\n|:-|-:|\n| `x|y` | c \\| d |");
        assert!(blocks[1].marks.is_empty());
        assert_eq!(blocks[3].text, "| p |\n|---|\n| q |");
        assert_eq!(blocks[4].text, "| r |\n|---|\n| s |");
    }

    #[test]
    fn table_rows_can_omit_pipes_and_end_at_a_blank_line() {
        // GFM pads missing cells, including a row with no pipes (example 202).
        let src = "| a | b |\n|---|---|\n| 1 |\n| 2 | 3 | 4 |\nNot a row, just prose\n";
        let blocks = parse_blocks(src, &mut Recorder::default());
        let kinds: Vec<BlockKind> = blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(kinds, [BlockKind::Table]);
        assert_eq!(blocks[0].text, src.trim_end_matches('\n'));
    }

    #[test]
    fn crlf_table_keeps_its_row_endings() {
        let src = "Before\r\n| a | b |\r\n|---|---|\r\n| 1 | 2 |\r\n\r\nAfter\r\n";
        let blocks = parse_blocks(src, &mut Recorder::default());
        let kinds: Vec<BlockKind> = blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            [BlockKind::Paragraph, BlockKind::Table, BlockKind::Paragraph]
        );
        // Inner row endings stay; the last row has none of its own.
        assert_eq!(blocks[1].text, "| a | b |\r\n|---|---|\r\n| 1 | 2 |");
        assert_eq!(blocks[2].text, "After");
    }

    #[test]
    fn fence_info_string_is_the_target() {
        let src = "```rust\nlet x = 1;\n```\n\n```\nplain\n```\n\n~~~ py\nx = 1\n~~~\n";
        let blocks = parse_blocks(src, &mut Recorder::default());
        assert_eq!(blocks[0].kind, BlockKind::Code);
        assert_eq!(blocks[0].target.as_deref(), Some("rust"));
        assert_eq!(blocks[0].text, "let x = 1;");
        assert_eq!(blocks[1].target, None);
        assert_eq!(blocks[1].text, "plain");
        assert_eq!(blocks[2].target.as_deref(), Some("py"));
    }

    #[test]
    fn view_fences_round_trip_settings_source_and_optional_page() {
        let definition = serde_json::json!({
            "id": "view-1", "name": "Rows", "kind": "Table",
            "filter": null, "sorts": [], "group_by": null,
            "date_prop": null, "visible_props": ["status"],
            "card_cover": null, "config": "{}",
            "source": "is:todo\n```literal```\n[[Home]] *raw*"
        });
        for page in [None, Some("page-1")] {
            let mut input = definition.clone();
            if let Some(page) = page {
                input["page"] = page.into();
            }
            let source = format!("````view\n{}\n````\n", input);
            let blocks = parse_blocks(&source, &mut Recorder::default());
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].kind, BlockKind::View);
            assert_eq!(blocks[0].target.as_deref(), page);
            assert_eq!(serde_json::from_str::<serde_json::Value>(&blocks[0].text).unwrap(), definition);
            assert!(blocks[0].marks.is_empty());

            let mut rendered = String::new();
            crate::markdown::render_blocks(
                &blocks, "", &std::collections::HashMap::new(), &|_| None, &mut rendered,
            );
            assert!(rendered.contains("````view\n"));
            let restored = parse_blocks(&rendered, &mut Recorder::default());
            assert_eq!(restored, blocks);
        }
    }

    #[test]
    fn view_fences_keep_unbound_text_and_do_not_absorb_children() {
        let source = "````view\n{\n  \"source\": \"```literal```\"\n}\n\n````\n  after\n";
        let blocks = parse_blocks(source, &mut Recorder::default());
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "{\n  \"source\": \"```literal```\"\n}\n");
        assert!(blocks[0].children.is_empty());
        assert_eq!(blocks[1].text, "after");
        let mut rendered = String::new();
        crate::markdown::render_blocks(
            &blocks, "", &std::collections::HashMap::new(), &|_| None, &mut rendered,
        );
        assert_eq!(parse_blocks(&rendered, &mut Recorder::default()), blocks);
    }

    #[test]
    fn view_fences_keep_invalid_json_for_editing() {
        let blocks = parse_blocks("~~~view\n{ unfinished *raw*\n~~~\n", &mut Recorder::default());
        assert_eq!(blocks[0].kind, BlockKind::View);
        assert_eq!(blocks[0].text, "{ unfinished *raw*");
        assert!(blocks[0].marks.is_empty());
    }

    #[test]
    fn yaml_subset() {
        let (fm, body) = split_frontmatter(
            "---\nid: X1\ntitle: \"A \\\"q\\\"\"\ntags: [a, \"b c\"]\nn: 3.5\nok: true\nlist:\n  - one\n  - two\nlinks:\n  \"Home\": P1\n---\nbody\n",
        );
        assert_eq!(body, "body\n");
        let get = |k: &str| fm.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
        assert_eq!(get("id"), Some(Yaml::Str("X1".into())));
        assert_eq!(get("title"), Some(Yaml::Str("A \"q\"".into())));
        assert_eq!(
            get("tags"),
            Some(Yaml::List(vec![
                Yaml::Str("a".into()),
                Yaml::Str("b c".into())
            ]))
        );
        assert_eq!(get("n"), Some(Yaml::Num(3.5)));
        assert_eq!(get("ok"), Some(Yaml::Bool(true)));
        assert_eq!(
            get("list"),
            Some(Yaml::List(vec![
                Yaml::Str("one".into()),
                Yaml::Str("two".into())
            ]))
        );
        assert_eq!(
            get("links"),
            Some(Yaml::Map(vec![("Home".into(), Yaml::Str("P1".into()))]))
        );
        assert_eq!(parse_iso("2026-10-01"), Some((1_790_812_800_000, false)));
        assert_eq!(
            parse_iso("2026-10-01T10:30:00.000Z"),
            Some((1_790_812_800_000 + 37_800_000, true))
        );
    }
}
