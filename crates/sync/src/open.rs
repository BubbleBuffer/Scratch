//! The `/open/<page id>[?block=<block id>]` route. It answers a small HTML
//! page that sends the browser to the `notes://page/...` link, which opens
//! the Scratch client. Two hubs serve it, the standalone `notes-hub` (axum,
//! `crates/hub`) and the desktop's embedded hub ([`crate::Hub`]); both use
//! [`bounce_page`], so the ids are checked and the page is built in one place.

use vault_core::is_page_id;

/// Content type of the bounce page.
pub const CONTENT_TYPE: &str = "text/html; charset=utf-8";
/// Cache-Control of the bounce page: it is never cached.
pub const CACHE_CONTROL: &str = "no-store";

/// The bounce page for a page, or for one block of it. The error names the
/// malformed id (`invalid page id` or `invalid block id`); the route answers
/// 400 with it. Ids must be vault ids ([`vault_core::is_page_id`]).
pub fn bounce_page(page: &str, block: Option<&str>) -> Result<String, &'static str> {
    bounce_page_with(page, block, "")
}

/// [`bounce_page`] with `extra`, trusted HTML the caller built, shown after
/// the "Open in Scratch" button (the standalone hub adds its "View in
/// browser" button there).
pub fn bounce_page_with(
    page: &str,
    block: Option<&str>,
    extra: &str,
) -> Result<String, &'static str> {
    if !is_page_id(page) {
        return Err("invalid page id");
    }
    let mut target = format!("notes://page/{page}");
    if let Some(block) = block {
        if !is_page_id(block) {
            return Err("invalid block id");
        }
        target.push_str(&format!("?block={block}"));
    }
    Ok(open_html(&target, extra))
}

/// The value of `block` in a query string (without `?`), percent-decoded.
/// `block=` gives `Some("")`, which [`bounce_page`] refuses; no `block` key
/// gives `None`.
pub fn block_query(query: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == "block").then(|| crate::pct_decode(value))
    })
}

/// The bounce page. `target` is a `notes://` link built from checked ids; it
/// is escaped for the attribute and the script anyway.
fn open_html(target: &str, extra: &str) -> String {
    let attr = html_escape(target);
    let js = serde_json::to_string(target)
        .unwrap_or_else(|_| "\"\"".to_string())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    format!(
        "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<meta name=\"referrer\" content=\"no-referrer\">
<meta http-equiv=\"refresh\" content=\"0; url={attr}\">
<title>Opening in Scratch</title>
<style>{BOUNCE_STYLE}</style>
</head>
<body>
<main>
<h1>Open this note</h1>
<p class=\"actions\"><a class=\"btn primary\" href=\"{attr}\">Open in Scratch</a>{extra}</p>
</main>
<script>location.replace({js});</script>
</body>
</html>
"
    )
}

/// The bounce page's look: the app's colours, light and dark.
const BOUNCE_STYLE: &str = "body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;font:16px/1.5 system-ui,sans-serif;text-align:center;background:#FFFAF1;color:#282A2F}main{padding:16px}h1{font-size:24px;font-weight:640;margin:0 0 20px}.actions{display:flex;flex-wrap:wrap;gap:10px;justify-content:center;margin:0}.btn{display:inline-block;padding:9px 18px;border-radius:7px 11px 6px 9px;border:1px solid #E4DCCF;color:#282A2F;background:#F3EFE7;font-weight:600;text-decoration:none}.btn.primary{background:#DE945F;border-color:#DE945F;font-weight:700}@media (prefers-color-scheme:dark){body{background:#30333A;color:#F3EFE7}.btn{background:#2A2D33;border-color:#41444C;color:#F3EFE7}.btn.primary{background:#F0A064;border-color:#F0A064;color:#282A2F}}";

fn html_escape(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    #[test]
    fn bounce_page_escapes_its_target() {
        assert_eq!(
            html_escape("<a href='x'>&\""),
            "&lt;a href=&#39;x&#39;&gt;&amp;&quot;"
        );
        let html = open_html(
            "notes://page/01ARZ3NDEKTSV4RRFFQ69G5FAV?block=</script>",
            "",
        );
        // The script literal is JSON, with `<` and `>` written as \u escapes.
        let escaped = "notes://page/01ARZ3NDEKTSV4RRFFQ69G5FAV?block=\\u003c/script\\u003e";
        assert!(html.contains(&format!("location.replace(\"{escaped}\")")));
        assert!(!html.contains("block=</script>"));
    }

    #[test]
    fn bounce_page_checks_both_ids() {
        assert!(bounce_page(PAGE, None)
            .unwrap()
            .contains("notes://page/01ARZ3NDEKTSV4RRFFQ69G5FAV\""));
        assert_eq!(bounce_page("bad", None), Err("invalid page id"));
        assert_eq!(bounce_page(PAGE, Some("")), Err("invalid block id"));
        assert_eq!(
            bounce_page(PAGE, Some("01ARZ3NDEKTSV4RRFFQ69G5FAI")),
            Err("invalid block id")
        );
    }

    #[test]
    fn block_query_reads_the_block_key() {
        assert_eq!(block_query(""), None);
        assert_eq!(block_query("x=1"), None);
        assert_eq!(block_query("block=ABC"), Some("ABC".into()));
        assert_eq!(block_query("x=1&block=A%2DB"), Some("A-B".into()));
        assert_eq!(block_query("block"), Some(String::new()));
        assert_eq!(block_query("block="), Some(String::new()));
    }
}
