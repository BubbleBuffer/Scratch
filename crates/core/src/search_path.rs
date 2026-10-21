//! Path patterns for `in:`. A page's path is its ancestor titles joined by
//! `/`. `*` matches within one segment, `**` any number of segments (a
//! trailing `**` at least one, so `Work/**` leaves Work out). A `[[...]]`
//! segment names one page. A literal first segment resolves the way a plain
//! `in:` value does, so `in:Work -in:Work/**` is Work alone.
//!
//! A pattern without a glob (`Work/Meetings`) matches pages and their
//! subtrees, like a plain `in:`; a pattern with a glob matches exactly the
//! pages it describes.

use std::collections::HashMap;

use crate::error::Result;
use crate::index::Index;

/// One path segment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seg {
    /// A page by id: `[[id]]`, or the resolved first segment.
    Id(String),
    /// A literal title, compared without case.
    Title(String),
    /// A title pattern with `*`.
    Glob(String),
    /// `**`.
    Deep,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PathPattern {
    pub segs: Vec<Seg>,
}

/// Splits `value` on `sep` outside quotes and `[[...]]`, keeping quotes and
/// brackets. Pieces are trimmed; empty ones are dropped.
pub(crate) fn split_raw(value: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut bracket = false;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted && c == '\\' {
            current.push(c);
            if let Some(next) = chars.next() {
                current.push(next);
            }
            continue;
        }
        if c == '"' && !bracket {
            quoted = !quoted;
        } else if !quoted && !bracket && c == '[' && chars.peek() == Some(&'[') {
            chars.next();
            bracket = true;
            current.push('[');
        } else if bracket && c == ']' && chars.peek() == Some(&']') {
            chars.next();
            bracket = false;
            current.push(']');
        } else if c == sep && !quoted && !bracket {
            let piece = current.trim();
            if !piece.is_empty() {
                out.push(piece.to_string());
            }
            current.clear();
            continue;
        }
        current.push(c);
    }
    let piece = current.trim();
    if !piece.is_empty() {
        out.push(piece.to_string());
    }
    out
}

/// Whether a raw alternative reads as a path pattern: an unquoted `/` or
/// `*` outside `[[...]]`.
pub(crate) fn is_pattern(raw: &str) -> bool {
    split_raw(raw, '/').len() > 1 || raw.ends_with('/') || raw.starts_with('/') || {
        let mut quoted = false;
        let mut bracket = 0usize;
        let mut prev = '\0';
        let mut found = false;
        for c in raw.chars() {
            match c {
                '"' if bracket == 0 => quoted = !quoted,
                '[' if !quoted && prev == '[' => bracket += 1,
                ']' if !quoted && prev == ']' && bracket > 0 => bracket -= 1,
                '*' if !quoted && bracket == 0 => found = true,
                _ => {}
            }
            prev = c;
        }
        found
    }
}

/// A raw segment's text: quotes removed (with escapes), and whether it was
/// quoted or bracketed.
fn unquote(raw: &str) -> (String, bool, bool) {
    if let Some(inner) = raw.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        return (inner.trim().to_string(), false, true);
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let mut out = String::new();
        let mut chars = raw[1..raw.len() - 1].chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else {
                out.push(c);
            }
        }
        return (out, true, false);
    }
    (raw.to_string(), false, false)
}

/// Parses a raw alternative. `resolve` turns a page reference or title into
/// an id, the way a plain `in:` value resolves; `memory` is the memory root.
pub(crate) fn parse(
    raw: &str,
    resolve: impl Fn(&str) -> Option<String>,
    memory: Option<String>,
) -> std::result::Result<PathPattern, String> {
    let pieces = split_raw(raw, '/');
    if pieces.is_empty() {
        return Err("in: needs a path".into());
    }
    let mut segs = Vec::with_capacity(pieces.len());
    for (i, piece) in pieces.iter().enumerate() {
        let (text, quoted, bracket) = unquote(piece);
        let seg = if bracket {
            Seg::Id(resolve(&text).ok_or_else(|| format!("in: no page named '{text}'"))?)
        } else if !quoted && text == "**" {
            Seg::Deep
        } else if !quoted && text.contains('*') {
            if text.contains("**") {
                return Err(format!("in: '**' must be a whole segment in '{raw}'"));
            }
            Seg::Glob(text)
        } else if i == 0 {
            let id = if !quoted && text.eq_ignore_ascii_case("memory") {
                memory.clone()
            } else {
                resolve(&text)
            };
            Seg::Id(id.ok_or_else(|| format!("in: no page named '{text}'"))?)
        } else {
            Seg::Title(text)
        };
        segs.push(seg);
    }
    Ok(PathPattern { segs })
}

fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text.chars().any(|c| {
            c.is_whitespace() || matches!(c, ',' | '"' | '\\' | '[' | ']' | '/' | '*' | ':')
        })
}

impl PathPattern {
    pub fn has_glob(&self) -> bool {
        self.segs
            .iter()
            .any(|s| matches!(s, Seg::Glob(_) | Seg::Deep))
    }

    /// The canonical text: ids as `[[id]]`, titles quoted when needed.
    pub fn canonical(&self) -> String {
        self.segs
            .iter()
            .map(|s| match s {
                Seg::Id(id) => format!("[[{id}]]"),
                Seg::Title(t) if needs_quotes(t) => {
                    format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
                }
                Seg::Title(t) | Seg::Glob(t) => t.clone(),
                Seg::Deep => "**".into(),
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The page the pattern starts at, when its first segment names one.
    fn anchor(&self) -> Option<&str> {
        match self.segs.first() {
            Some(Seg::Id(id)) => Some(id),
            _ => None,
        }
    }

    /// Whether a chain of (id, lowercased title), from the start page down to
    /// the page itself, matches.
    fn matches(&self, chain: &[(&str, String)]) -> bool {
        if self.has_glob() {
            match_segs(&self.segs, chain)
        } else {
            chain.len() >= self.segs.len() && match_segs(&self.segs, &chain[..self.segs.len()])
        }
    }

    /// Ids of the pages the pattern matches.
    pub(crate) fn page_ids(&self, db: &Index) -> Result<Vec<String>> {
        let rows: Vec<(String, Option<String>, String)> = match self.anchor() {
            Some(anchor) => db
                .conn
                .prepare_cached(
                    "WITH RECURSIVE tree(id) AS (SELECT id FROM pages WHERE id = ?1
                     UNION SELECT p.id FROM pages p JOIN tree t ON p.parent = t.id)
                     SELECT p.id, p.parent, p.title FROM pages p JOIN tree t ON p.id = t.id",
                )?
                .query_map([anchor], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?,
            None => db
                .conn
                .prepare_cached("SELECT id, parent, title FROM pages")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?,
        };
        let by_id: HashMap<&str, (Option<&str>, String)> = rows
            .iter()
            .map(|(id, parent, title)| (id.as_str(), (parent.as_deref(), title.to_lowercase())))
            .collect();
        let anchor = self.anchor();
        let mut out = Vec::new();
        for (id, _, _) in &rows {
            // Walk up to the anchor, or to the top of the tree.
            let mut chain: Vec<(&str, String)> = Vec::new();
            let mut at = Some(id.as_str());
            while let Some(cur) = at {
                let Some((parent, title)) = by_id.get(cur) else {
                    break;
                };
                chain.push((cur, title.clone()));
                if Some(cur) == anchor || chain.len() > 256 {
                    break;
                }
                at = *parent;
            }
            chain.reverse();
            if self.matches(&chain) {
                out.push(id.clone());
            }
        }
        Ok(out)
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

fn match_segs(segs: &[Seg], chain: &[(&str, String)]) -> bool {
    let Some((seg, rest)) = segs.split_first() else {
        return chain.is_empty();
    };
    match seg {
        Seg::Deep => {
            // A trailing ** needs at least one segment.
            let min = usize::from(rest.is_empty());
            (min..=chain.len()).any(|skip| match_segs(rest, &chain[skip..]))
        }
        _ => {
            let Some(((id, title), tail)) = chain.split_first() else {
                return false;
            };
            let ok = match seg {
                Seg::Id(want) => id == want,
                Seg::Title(want) => *title == want.to_lowercase(),
                Seg::Glob(pattern) => glob_match(pattern, title),
                Seg::Deep => unreachable!(),
            };
            ok && match_segs(rest, tail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(raw: &str) -> PathPattern {
        parse(
            raw,
            |t| Some(format!("id-{}", t.to_lowercase())),
            Some("id-mem".into()),
        )
        .unwrap()
    }

    fn chain(items: &[&str]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|t| (format!("id-{}", t.to_lowercase()), t.to_lowercase()))
            .collect()
    }

    fn hit(p: &PathPattern, items: &[&str]) -> bool {
        let c = chain(items);
        let c: Vec<(&str, String)> = c.iter().map(|(i, t)| (i.as_str(), t.clone())).collect();
        p.matches(&c)
    }

    #[test]
    fn splits_outside_quotes_and_brackets() {
        assert_eq!(
            split_raw(r#"[[A/B]]/"x/y"/z"#, '/'),
            vec!["[[A/B]]", "\"x/y\"", "z"]
        );
        assert_eq!(split_raw(r#"a/*,"b,c"/**"#, ','), vec!["a/*", "\"b,c\"/**"]);
        assert!(is_pattern("Work/*"));
        assert!(is_pattern("**/Meetings"));
        assert!(is_pattern("Wo*"));
        assert!(!is_pattern("Work"));
        assert!(!is_pattern("\"a/b*\""));
        assert!(!is_pattern("[[a/b*]]"));
    }

    #[test]
    fn patterns_match_children_descendants_and_titles_anywhere() {
        let children = pat("Work/*");
        assert!(hit(&children, &["Work", "Q3"]));
        assert!(!hit(&children, &["Work"]));
        assert!(!hit(&children, &["Work", "Q3", "Notes"]));
        let all = pat("Work/**");
        assert!(!hit(&all, &["Work"]));
        assert!(hit(&all, &["Work", "Q3"]));
        assert!(hit(&all, &["Work", "Q3", "Notes"]));
        let meetings = pat("**/Meetings");
        assert!(hit(&meetings, &["Meetings"]));
        assert!(hit(&meetings, &["Home", "Club", "MEETINGS"]));
        assert!(!hit(&meetings, &["Meetings", "Notes"]));
        let plain = pat("Work/Q3");
        assert!(!plain.has_glob());
        assert!(hit(&plain, &["Work", "q3"]));
        assert!(
            hit(&plain, &["Work", "Q3", "Notes"]),
            "no glob: the subtree too"
        );
        assert!(!hit(&plain, &["Work"]));
        let partial = pat("Work/Q*");
        assert!(hit(&partial, &["Work", "Q3 plans"]));
        assert!(!hit(&partial, &["Work", "Plans"]));
    }

    #[test]
    fn quoted_and_bracketed_segments_and_canonical_text() {
        let p = pat(r#"[[My work]]/"Q3 notes"/*"#);
        assert_eq!(
            p.segs,
            vec![
                Seg::Id("id-my work".into()),
                Seg::Title("Q3 notes".into()),
                Seg::Glob("*".into())
            ]
        );
        assert_eq!(p.canonical(), r#"[[id-my work]]/"Q3 notes"/*"#);
        assert_eq!(pat(&p.canonical()).segs[1..], p.segs[1..]);
        assert_eq!(pat("memory/*").segs[0], Seg::Id("id-mem".into()));
        assert_eq!(pat(r#""Work*"/x"#).segs[0], Seg::Id("id-work*".into()));
        assert!(parse("a/b**", |t| Some(t.into()), None).is_err());
        assert!(parse("nope/*", |_| None, None).is_err());
    }
}
