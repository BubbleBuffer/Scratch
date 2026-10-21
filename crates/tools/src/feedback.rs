//! Feedback reports sent by MCP agents. They are children of the hidden
//! Feedback page, laid out like the app's reports (see `app/lib/feedback/report.dart`):
//! the summary paragraph, the details markdown, then a `text` code block of
//! diagnostics as its last top-level block. Metadata is read from that block only.
use serde_json::{json, Value};
use vault_core::{new_id, Block, BlockKind, BlockOp, NewBlock, PageSummary, Vault};

use crate::{err, Result, ToolError};

const KINDS: [&str; 4] = ["bug", "complaint", "idea", "other"];
const SERVER_NAME: &str = "notes";
const PAGE_SIZE: u32 = 100;
const REDACTED: &str = "[redacted]";
const PARAMS: [&str; 4] = ["token", "key", "secret", "password"];

/// Stores a report from an MCP call. `transport` names how the server was
/// reached (`stdio` or `http`) and goes into the diagnostics.
pub fn submit(v: &Vault, a: &Value, transport: &str) -> Result<String> {
    let summary = string_arg(a, "summary")?.unwrap_or("").trim();
    if summary.is_empty() {
        return err("summary is required");
    }
    if summary.contains(|c: char| c == '\n' || c == '\r') {
        return err("summary must be one line");
    }
    let summary = redact(summary);
    let kind = string_arg(a, "kind")?.unwrap_or("bug");
    if !KINDS.contains(&kind) {
        return err("kind must be bug, complaint, idea or other");
    }
    let details = redact(string_arg(a, "details")?.unwrap_or("").trim());
    let diagnostics = format!(
        "kind: {kind}\nsource: mcp\nserver: {SERVER_NAME}\nversion: {}\ntransport: {}\npage_id: {}\ntool: {}\nerror: {}",
        env!("CARGO_PKG_VERSION"),
        single_line(transport),
        optional_line(a, "page_id")?,
        optional_line(a, "tool")?,
        optional_line(a, "error")?,
    );
    let title = format!(
        "Feedback {} agent",
        chrono::Utc::now().format("%Y-%m-%d %H:%M")
    );

    let feedback = v.feedback_page()?;
    let report = v.create_page(Some(feedback), title.clone())?;
    let summary_id = new_id();
    v.apply(
        report.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: summary_id.clone(),
                kind: BlockKind::Paragraph,
                text: summary,
                marks: vec![],
                checked: false,
            },
        }],
    )?;
    let mut last = summary_id;
    if !details.is_empty() {
        if let Some(id) = v
            .insert_markdown(report.clone(), None, Some(last.clone()), false, details)?
            .pop()
        {
            last = id;
        }
    }
    let code_id = new_id();
    v.apply(
        report.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: Some(last),
                block: NewBlock {
                    id: code_id.clone(),
                    kind: BlockKind::Code,
                    text: diagnostics,
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: code_id,
                target: Some("text".into()),
            },
        ],
    )?;
    Ok(json!({ "id": report, "title": title }).to_string())
}

/// Lists reports under the Feedback page, newest first, without creating it.
pub fn list(v: &Vault, a: &Value) -> Result<String> {
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .min(100) as usize;
    let kind = string_arg(a, "kind")?;
    if let Some(k) = kind {
        if k != "app" && !KINDS.contains(&k) {
            return err("kind must be bug, complaint, idea, other or app");
        }
    }
    let Some(feedback) = v.find_by_role("feedback".into())? else {
        return Ok(json!({ "reports": [] }).to_string());
    };
    // Sort every report before applying the limit, so the newest matches win.
    let mut children: Vec<PageSummary> = Vec::new();
    let mut offset = 0;
    loop {
        let batch = v.list_children(Some(feedback.clone()), offset, PAGE_SIZE)?;
        let n = batch.len() as u32;
        children.extend(batch);
        if n < PAGE_SIZE {
            break;
        }
        offset += n;
    }
    children.sort_by(|x, y| (y.created_ms, &y.id).cmp(&(x.created_ms, &x.id)));

    let mut reports = Vec::new();
    for child in &children {
        if reports.len() == limit {
            break;
        }
        let page = v.load_page(child.id.clone())?;
        let (report_kind, source) = metadata(&page.blocks);
        if let Some(k) = kind {
            if k != report_kind {
                continue;
            }
        }
        reports.push(json!({
            "id": child.id,
            "title": child.title,
            "created": created(child.created_ms),
            "kind": report_kind,
            "source": source,
            "first_line": first_line(&page.blocks),
        }));
    }
    Ok(json!({ "reports": reports }).to_string())
}

/// Read the newest recognizable diagnostics block. Follow-up text or code may
/// be appended after filing; it must not change the report's kind or source.
fn metadata(blocks: &[Block]) -> (String, String) {
    for block in blocks.iter().rev() {
        if !matches!(block.kind, BlockKind::Code) || block.target.as_deref() != Some("text") {
            continue;
        }
        let header = |key: &str| {
            block
                .text
                .lines()
                .take_while(|line| !line.trim().is_empty())
                .find_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
        };
        let kind = header("kind").filter(|k| KINDS.contains(k));
        if header("source") == Some("mcp") && header("server") == Some(SERVER_NAME) {
            return (kind.unwrap_or("app").into(), "mcp".into());
        }
        if header("app").is_some() {
            return (kind.unwrap_or("app").into(), "app".into());
        }
    }
    ("app".into(), "app".into())
}

/// The whole first line of the first top-level paragraph: the summary, or the
/// first line of an app report's text.
fn first_line(blocks: &[Block]) -> String {
    blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Paragraph))
        .and_then(|b| b.text.lines().next())
        .unwrap_or("")
        .to_string()
}

fn created(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn string_arg<'a>(a: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(ToolError(format!("{key} must be a string"))),
    }
}

/// A redacted, single-line value, or `none` when the argument is absent.
fn optional_line(a: &Value, key: &str) -> Result<String> {
    Ok(match string_arg(a, key)?.map(str::trim) {
        Some(s) if !s.is_empty() => single_line(&redact(s)),
        _ => "none".into(),
    })
}

/// Control characters become spaces, so a value cannot start a new diagnostics line.
fn single_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Removes secrets from text: the value after `Bearer`, the value of a
/// `token`, `key`, `secret` or `password` parameter, and any run of 64 or more
/// hex digits (the shape of hub and desktop tokens).
pub(crate) fn redact(text: &str) -> String {
    redact_hex(&redact_params(&redact_bearer(text)))
}

/// Where a query-parameter value ends.
fn param_end(s: &str) -> usize {
    s.find(|c: char| c.is_whitespace() || matches!(c, '&' | '#' | '"' | '<' | '>'))
        .unwrap_or(s.len())
}

/// Where a Bearer credential ends. `&` and `#` can be part of one, so only
/// whitespace and quote or angle brackets end it.
fn bearer_end(s: &str) -> usize {
    s.find(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>'))
        .unwrap_or(s.len())
}

fn redact_bearer(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in lower.match_indices("bearer") {
        let boundary = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
        if start < last || !boundary {
            continue;
        }
        let after = start + "bearer".len();
        let value = after + text[after..].len() - text[after..].trim_start().len();
        let end = value + bearer_end(&text[value..]);
        if value == after || end == value {
            continue;
        }
        out.push_str(&text[last..value]);
        out.push_str(REDACTED);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn redact_params(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        let boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let name = PARAMS.iter().find(|p| {
            bytes[i..].starts_with(p.as_bytes()) && bytes.get(i + p.len()) == Some(&b'=')
        });
        match name {
            Some(p) if boundary => {
                let value = i + p.len() + 1;
                let end = value + param_end(&text[value..]);
                out.push_str(&text[last..value]);
                out.push_str(REDACTED);
                last = end;
                i = end;
            }
            _ => i += 1,
        }
    }
    out.push_str(&text[last..]);
    out
}

fn redact_hex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut run = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (run, c.is_ascii_hexdigit()) {
            (None, true) => run = Some(i),
            (Some(start), false) => {
                if i - start >= 64 {
                    out.push_str(&text[last..start]);
                    out.push_str(REDACTED);
                    last = i;
                }
                run = None;
            }
            _ => {}
        }
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn redacts_each_secret_shape_and_keeps_the_rest() {
        assert_eq!(
            redact("Authorization: Bearer abc.def-ghi"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(redact("BEARER   xyz"), "BEARER   [redacted]");
        assert_eq!(
            redact("https://h/mcp?x=1&token=SECRET&y=2"),
            "https://h/mcp?x=1&token=[redacted]&y=2"
        );
        assert_eq!(redact("?key=A#frag"), "?key=[redacted]#frag");
        assert_eq!(redact("access_token=S"), "access_token=[redacted]");
        assert_eq!(redact("password=p<b>"), "password=[redacted]<b>");
        assert_eq!(redact(&format!("tok {HEX} end")), "tok [redacted] end");
        assert_eq!(redact(&format!("x{HEX}y")), "x[redacted]y");
        // Shorter hex and ordinary words stay.
        assert_eq!(redact("deadbeef facade"), "deadbeef facade");
        assert_eq!(redact("keyboard token tokens"), "keyboard token tokens");
        assert_eq!(
            redact("hÃƒÂ©llo bearer ÃƒÂ¼nÃƒÂ¯"),
            "hÃƒÂ©llo bearer [redacted]"
        );
    }

    #[test]
    fn bearer_runs_through_ampersand_and_hash_but_not_quotes() {
        assert_eq!(redact("Bearer abc&def#g end"), "Bearer [redacted] end");
        assert_eq!(redact("Bearer abc\"q"), "Bearer [redacted]\"q");
        assert_eq!(redact("x?token=abc&y"), "x?token=[redacted]&y");
        assert_eq!(redact("x?token=abc#frag"), "x?token=[redacted]#frag");
    }

    #[test]
    fn redaction_handles_multibyte_text() {
        assert_eq!(redact("ÃƒÂ©?token=ÃƒÂ¼"), "ÃƒÂ©?token=[redacted]");
    }
}
