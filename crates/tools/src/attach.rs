//! `attach_file`: stores a file as a vault blob and adds an image or file
//! block for it, the same way the app does (a content-addressed blob, then
//! an `Image` or `File` block whose target is the attachment id). Blobs sync
//! like any app-made attachment: devices fetch them once a synced page
//! references them.
//!
//! Sources: `path` (a file on this computer; only where the host enabled it
//! with [`allow_local_paths`], never on the hub), `url` (https only, fetched
//! with private-address, redirect, size and time limits) or `data` (base64).

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};
use vault_core::{BlockKind, BlockOp, NewBlock, Vault};

use crate::{err, Result, ToolError};

/// The largest file attach_file stores, from any source.
pub const MAX_BYTES: usize = 25 << 20;
/// The largest `data` argument, after base64 decoding.
pub const MAX_DATA_BYTES: usize = 10 << 20;
/// The longest a `url` fetch may take, redirects included.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Redirects a `url` fetch follows.
pub const MAX_REDIRECTS: u32 = 3;

/// The error for `path` where the tool does not run on the user's computer.
pub const PATH_NOT_HERE: &str = "path is only available on your own computer; pass url or data";

/// The longest base64 text, in bytes, a host should accept for `data`: the
/// encoding of [`MAX_DATA_BYTES`] plus room for line breaks.
pub const fn data_wire_limit() -> usize {
    let encoded = MAX_DATA_BYTES.div_ceil(3) * 4;
    encoded + encoded / 32 + 64
}

static LOCAL_PATHS: AtomicBool = AtomicBool::new(false);

/// Lets `path` read files on this computer. Hosts running on the user's own
/// machine (the desktop app, the local notes-mcp plugin) turn this on at
/// startup; the hub never does, so there `path` is refused.
pub fn allow_local_paths(allow: bool) {
    LOCAL_PATHS.store(allow, Ordering::Relaxed);
}

/// Whether `path` is served in this process.
pub fn local_paths_allowed() -> bool {
    LOCAL_PATHS.load(Ordering::Relaxed)
}

#[cfg(test)]
thread_local! {
    /// Test-only: lets this thread fetch plain http from loopback, so tests
    /// can serve files from a local server. Never compiled outside tests.
    static TEST_LOOPBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn test_loopback() -> bool {
    #[cfg(test)]
    {
        TEST_LOOPBACK.with(|f| f.get())
    }
    #[cfg(not(test))]
    {
        false
    }
}

/// The tool's schema.
pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page": { "type": "string", "description": "Page id (or exact title)" },
            "path": { "type": "string", "description": "A file on this computer (local plugin, CLI and desktop app only; not on the hub)" },
            "url": { "type": "string", "description": "An https URL to download (at most 25 MB, 3 redirects, 30 s)" },
            "data": { "type": "string", "description": "The file's bytes, base64 encoded (at most 10 MB decoded)" },
            "name": { "type": "string", "description": "File name to keep (default: from the path or URL)" },
            "caption": { "type": "string", "description": "Image caption, or a file block's label (default: the name)" },
            "after_block": { "type": "string", "description": "Insert after this block (default: at the end of the page)" },
            "as": { "type": "string", "enum": ["image", "file"], "description": "Block kind (default: image for png, jpeg, gif, webp or svg content, else file)" }
        },
        "required": ["page"]
    })
}

fn opt<'a>(a: &'a Value, k: &str) -> Result<Option<&'a str>> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => err(format!("{k} must be a string")),
    }
}

/// The source's bytes plus a name and declared content type, if known.
struct Fetched {
    bytes: Vec<u8>,
    name: Option<String>,
    content_type: Option<String>,
}

pub(crate) fn attach_file(v: &Vault, a: &Value) -> Result<String> {
    let page_ref = opt(a, "page")?.ok_or_else(|| ToolError("missing argument: page".into()))?;
    let path = opt(a, "path")?.filter(|s| !s.is_empty());
    let url = opt(a, "url")?.filter(|s| !s.is_empty());
    let data = opt(a, "data")?;
    let caption = opt(a, "caption")?;
    let after = opt(a, "after_block")?.filter(|s| !s.is_empty());
    let kind = match opt(a, "as")? {
        None => None,
        Some("image") => Some(BlockKind::Image),
        Some("file") => Some(BlockKind::File),
        Some(_) => return err("as must be image or file"),
    };
    let given = [path.is_some(), url.is_some(), data.is_some()]
        .iter()
        .filter(|x| **x)
        .count();
    if given != 1 {
        return err("give exactly one of path, url or data");
    }
    // Check the page and position before reading or downloading anything.
    let page = crate::tools::resolve_page(v, page_ref)?;
    let (parent, after) = position(v, &page, after)?;

    let fetched = if let Some(p) = path {
        read_path(p)?
    } else if let Some(u) = url {
        fetch_url(u)?
    } else {
        decode_data(data.unwrap_or_default())?
    };
    let name_arg = opt(a, "name")?.filter(|s| !s.trim().is_empty());
    let sniffed = sniff_image(&fetched.bytes);
    let declared_image = fetched
        .content_type
        .as_deref()
        .map(|t| t.trim().to_ascii_lowercase().starts_with("image/"))
        .unwrap_or(false)
        || name_arg
            .or(fetched.name.as_deref())
            .is_some_and(|n| ext_is_image(n));
    let kind = kind.unwrap_or(if declared_image || sniffed.is_some() {
        BlockKind::Image
    } else {
        BlockKind::File
    });
    let mut name = sanitize_name(name_arg.or(fetched.name.as_deref()).unwrap_or("file"));
    if kind == BlockKind::Image {
        let Some(image) = sniffed else {
            return err("not an image: as image takes png, jpeg, gif, webp or svg");
        };
        if image == ImageType::Svg {
            check_svg(&fetched.bytes)?;
        }
        // The stored name sets the attachment's MIME type: make it match.
        if !image.exts().iter().any(|e| has_ext(&name, e)) {
            name = format!("{name}.{}", image.exts()[0]);
        }
    }
    let size = fetched.bytes.len();
    let id = v.add_attachment_bytes(name.clone(), fetched.bytes)?;
    let text = match kind {
        BlockKind::Image => caption.unwrap_or("").replace('\n', " "),
        _ => caption.unwrap_or(&name).replace('\n', " "),
    };
    let block = vault_core::new_id();
    v.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent,
                after,
                block: NewBlock {
                    id: block.clone(),
                    kind: kind.clone(),
                    text,
                    marks: Vec::new(),
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: block.clone(),
                target: Some(id.clone()),
            },
        ],
    )?;
    let what = if kind == BlockKind::Image {
        "an image"
    } else {
        "a file"
    };
    Ok(format!(
        "Attached {name} ({size} bytes) as {what} block: {block}\nattachment: ../.vault/blobs/{}/{id}\nblock link: {}\nlink: {}",
        &id[..2],
        crate::tools::page_link(&page, Some(&block)),
        crate::tools::page_link(&page, None),
    ))
}

/// Where the new block goes: after `after` under its parent, or after the
/// page's last top-level block.
fn position(
    v: &Vault,
    page: &str,
    after: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    let loaded = v.load_page(page.to_string())?;
    match after {
        Some(b) => {
            let parent = crate::tools::parent_in(&loaded.blocks, None, b)
                .ok_or_else(|| ToolError(format!("no block {b} in the page")))?;
            Ok((parent, Some(b.to_string())))
        }
        None => Ok((None, loaded.blocks.last().map(|b| b.id.clone()))),
    }
}

fn too_big(limit: usize) -> ToolError {
    ToolError(format!("file is over the {} MB cap", limit >> 20))
}

fn read_path(p: &str) -> Result<Fetched> {
    if !local_paths_allowed() {
        return err(PATH_NOT_HERE);
    }
    let path = std::path::Path::new(p);
    let meta = std::fs::metadata(path).map_err(|e| ToolError(format!("cannot read {p}: {e}")))?;
    if !meta.is_file() {
        return err(format!("{p} is not a file"));
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err(too_big(MAX_BYTES));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| ToolError(format!("cannot read {p}: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(too_big(MAX_BYTES));
    }
    Ok(Fetched {
        bytes,
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
        content_type: None,
    })
}

fn decode_data(data: &str) -> Result<Fetched> {
    if data.len() > data_wire_limit() {
        return Err(too_big(MAX_DATA_BYTES));
    }
    let clean: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .map_err(|e| ToolError(format!("data is not valid base64: {e}")))?;
    if bytes.len() > MAX_DATA_BYTES {
        return Err(too_big(MAX_DATA_BYTES));
    }
    if bytes.is_empty() {
        return err("data is empty");
    }
    Ok(Fetched {
        bytes,
        name: None,
        content_type: None,
    })
}

// ------------------------------------------------------------------ url

/// Whether `ip` is a public address a fetch may connect to.
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || o[0] == 0 // "this network"
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // shared address space (CGNAT)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking
        || o[0] >= 240) // reserved
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let s = ip.segments();
    // IPv4-compatible (::a.b.c.d) and NAT64 (64:ff9b::/96) embed an IPv4 address.
    if s[..6] == [0, 0, 0, 0, 0, 0] || s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        if !(ip.is_unspecified() || ip.is_loopback()) {
            return is_public_v4(v4);
        }
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00 // unique local
        || (s[0] & 0xffc0) == 0xfe80 // link-local
        || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated)
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        || (s[0] == 0x0100 && s[1..4] == [0, 0, 0])) // discard prefix
}

fn allowed_ip(ip: IpAddr) -> bool {
    is_public(ip) || (test_loopback() && ip.is_loopback())
}

/// Resolves names with the system resolver, then drops every address that
/// is not public. ureq resolves each connection, redirects included, and
/// connects only to the addresses returned here, so a name that later
/// resolves elsewhere (DNS rebinding) is checked again where it is used.
#[derive(Debug, Default)]
struct PublicOnly(ureq::unversioned::resolver::DefaultResolver);

impl ureq::unversioned::resolver::Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let all = self.0.resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        for addr in all.iter() {
            if allowed_ip(addr.ip()) {
                kept.push(*addr);
            }
        }
        if kept.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{} resolves to a private, loopback or link-local address",
                    uri.host().unwrap_or("the host")
                ),
            )));
        }
        Ok(kept)
    }
}

fn fetch_url(u: &str) -> Result<Fetched> {
    let parsed = url::Url::parse(u).map_err(|_| ToolError("url is not a valid URL".into()))?;
    let loopback_http = test_loopback() && parsed.scheme() == "http";
    if parsed.scheme() != "https" && !loopback_http {
        return err("url must be https");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return err("url must not contain a user name or password");
    }
    let config = ureq::Agent::config_builder()
        .https_only(!loopback_http)
        .max_redirects(MAX_REDIRECTS)
        .timeout_global(Some(FETCH_TIMEOUT))
        // An environment proxy would connect on our behalf, past the resolver.
        .proxy(None)
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::default(),
        PublicOnly::default(),
    );
    let mut resp = agent.get(u).call().map_err(fetch_error)?;
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .map(String::from);
    let declared = resp
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_BYTES as u64) {
        return Err(too_big(MAX_BYTES));
    }
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_BYTES as u64)
        .read_to_vec()
        .map_err(fetch_error)?;
    if bytes.is_empty() {
        return err("the url returned no content");
    }
    // The name comes from the final URL, after redirects.
    let name = {
        use ureq::ResponseExt as _;
        resp.get_uri()
            .path()
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.replace("%20", " "))
    };
    Ok(Fetched {
        bytes,
        name,
        content_type,
    })
}

fn fetch_error(e: ureq::Error) -> ToolError {
    match e {
        ureq::Error::BodyExceedsLimit(_) => too_big(MAX_BYTES),
        ureq::Error::TooManyRedirects => {
            ToolError(format!("url redirected more than {MAX_REDIRECTS} times"))
        }
        ureq::Error::RequireHttpsOnly(_) => {
            ToolError("url must be https (also after redirects)".into())
        }
        ureq::Error::Timeout(_) => ToolError(format!(
            "url took longer than {} s",
            FETCH_TIMEOUT.as_secs()
        )),
        ureq::Error::StatusCode(code) => ToolError(format!("url returned HTTP {code}")),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            ToolError(format!("url refused: {io}"))
        }
        other => ToolError(format!("cannot fetch url: {other}")),
    }
}

// ------------------------------------------------------------ validation

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageType {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
}

impl ImageType {
    fn exts(self) -> &'static [&'static str] {
        match self {
            ImageType::Png => &["png"],
            ImageType::Jpeg => &["jpg", "jpeg"],
            ImageType::Gif => &["gif"],
            ImageType::Webp => &["webp"],
            ImageType::Svg => &["svg"],
        }
    }
}

/// The image type the bytes start with; svg is recognised by its root tag.
fn sniff_image(b: &[u8]) -> Option<ImageType> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageType::Png)
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageType::Jpeg)
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some(ImageType::Gif)
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(ImageType::Webp)
    } else if looks_svg(b) {
        Some(ImageType::Svg)
    } else {
        None
    }
}

/// Text whose first element, after an optional BOM, XML declaration,
/// comments and doctype, is `<svg`.
fn looks_svg(b: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(b) else {
        return false;
    };
    let mut t = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if let Some(rest) = t.strip_prefix("<?") {
            match rest.find("?>") {
                Some(i) => t = rest[i + 2..].trim_start(),
                None => return false,
            }
        } else if let Some(rest) = t.strip_prefix("<!--") {
            match rest.find("-->") {
                Some(i) => t = rest[i + 3..].trim_start(),
                None => return false,
            }
        } else if t.len() >= 9 && t[..9].eq_ignore_ascii_case("<!doctype") {
            match t.find('>') {
                Some(i) => t = t[i + 1..].trim_start(),
                None => return false,
            }
        } else {
            break;
        }
    }
    t.len() >= 4
        && t[..4].eq_ignore_ascii_case("<svg")
        && t[4..].starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
}

/// Refuses an svg that could run script: script elements, foreignObject,
/// event-handler attributes or javascript: URLs.
fn check_svg(b: &[u8]) -> Result<()> {
    let lower = String::from_utf8_lossy(b).to_ascii_lowercase();
    let bad = lower.contains("<script")
        || lower.contains(":script")
        || lower.contains("foreignobject")
        || lower.contains("javascript:")
        || lower.contains("<!entity")
        || has_event_attr(&lower);
    if bad {
        return err("svg refused: it contains script or foreignObject");
    }
    Ok(())
}

/// An attribute named on… (onload=, onclick = …) inside a tag.
fn has_event_attr(lower: &str) -> bool {
    let b = lower.as_bytes();
    let mut i = 0;
    while let Some(off) = lower[i..].find("on") {
        let at = i + off;
        let before_ok = at > 0
            && (b[at - 1].is_ascii_whitespace()
                || b[at - 1] == b'/'
                || b[at - 1] == b'"'
                || b[at - 1] == b'\'');
        if before_ok {
            let mut j = at + 2;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'_') {
                j += 1;
            }
            let name_len = j - at - 2;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if name_len > 0 && j < b.len() && b[j] == b'=' {
                return true;
            }
        }
        i = at + 2;
    }
    false
}

fn has_ext(name: &str, ext: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
}

fn ext_is_image(name: &str) -> bool {
    ["png", "jpg", "jpeg", "gif", "webp", "svg"]
        .iter()
        .any(|e| has_ext(name, e))
}

/// A safe file name: no directories, control or reserved characters, no
/// leading dots, at most 120 characters.
pub(crate) fn sanitize_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' ']);
    let mut out: String = trimmed.chars().take(120).collect();
    if out.trim().is_empty() {
        out = "file".into();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    const PNG: &[u8] =
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

    struct Loopback;
    impl Loopback {
        fn on() -> Self {
            TEST_LOOPBACK.with(|f| f.set(true));
            Loopback
        }
    }
    impl Drop for Loopback {
        fn drop(&mut self) {
            TEST_LOOPBACK.with(|f| f.set(false));
        }
    }

    /// Serves `count` requests: each answer is picked by request path.
    fn serve(count: usize, answer: impl Fn(&str, u16) -> Vec<u8> + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(count) {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let _ = stream.write_all(&answer(&path, port));
            }
        });
        port
    }

    fn ok(body: &[u8], ctype: &str) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn redirect(to: &str) -> Vec<u8> {
        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .into_bytes()
    }

    #[test]
    fn fetches_from_a_local_server_under_the_test_flag() {
        let _flag = Loopback::on();
        let port = serve(2, |path, port| match path {
            "/start" => redirect(&format!("http://127.0.0.1:{port}/cat.png")),
            _ => ok(PNG, "image/png"),
        });
        let f = fetch_url(&format!("http://127.0.0.1:{port}/start")).unwrap();
        assert_eq!(f.bytes, PNG);
        assert_eq!(f.name.as_deref(), Some("cat.png"));
        assert_eq!(f.content_type.as_deref(), Some("image/png"));
    }

    #[test]
    fn refuses_http_and_private_addresses_without_the_flag() {
        let e = fetch_url("http://example.com/a.png").err().unwrap();
        assert!(e.0.contains("https"), "{e}");
        for host in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.200.10",
            "169.254.169.254",
            "[::1]",
            "[fd00::1]",
        ] {
            let e = fetch_url(&format!("https://{host}/a.png")).err().unwrap();
            assert!(e.0.contains("private"), "{host}: {e}");
        }
    }

    #[test]
    fn refuses_a_redirect_into_a_private_address() {
        // The first hop is allowed (loopback under the flag); the redirect
        // goes to a private address, which the resolver refuses.
        let _flag = Loopback::on();
        let port = serve(1, |_, _| redirect("http://10.255.255.1:9/secret"));
        let e = fetch_url(&format!("http://127.0.0.1:{port}/start"))
            .err()
            .unwrap();
        assert!(e.0.contains("private"), "{e}");
    }

    #[test]
    fn follows_at_most_three_redirects() {
        let _flag = Loopback::on();
        let port = serve(4, |path, port| {
            let n: u32 = path.trim_start_matches('/').parse().unwrap_or(0);
            redirect(&format!("http://127.0.0.1:{port}/{}", n + 1))
        });
        let e = fetch_url(&format!("http://127.0.0.1:{port}/0"))
            .err()
            .unwrap();
        assert!(e.0.contains("redirected"), "{e}");
    }

    #[test]
    fn caps_the_download_size() {
        let _flag = Loopback::on();
        let port = serve(1, |_, _| {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_BYTES + 1
            )
            .into_bytes()
        });
        let e = fetch_url(&format!("http://127.0.0.1:{port}/big"))
            .err()
            .unwrap();
        assert!(e.0.contains("25 MB"), "{e}");
    }

    #[test]
    fn caps_a_download_without_content_length() {
        let _flag = Loopback::on();
        let port = serve(1, |_, _| {
            let mut r = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
            r.extend(std::iter::repeat_n(b'a', MAX_BYTES + 10));
            r
        });
        let e = fetch_url(&format!("http://127.0.0.1:{port}/big"))
            .err()
            .unwrap();
        assert!(e.0.contains("25 MB"), "{e}");
    }

    #[test]
    fn public_address_checks() {
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.100.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "224.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn sniffs_images_and_refuses_scripted_svg() {
        assert_eq!(sniff_image(PNG), Some(ImageType::Png));
        assert_eq!(sniff_image(b"\xff\xd8\xff\xe0rest"), Some(ImageType::Jpeg));
        assert_eq!(sniff_image(b"GIF89a...."), Some(ImageType::Gif));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageType::Webp));
        let svg = b"<?xml version=\"1.0\"?>\n<!-- c --><svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        assert_eq!(sniff_image(svg), Some(ImageType::Svg));
        assert_eq!(sniff_image(b"hello world"), None);
        assert_eq!(sniff_image(b"<html><svg></svg></html>"), None);
        assert!(check_svg(svg).is_ok());
        for bad in [
            &b"<svg><script>alert(1)</script></svg>"[..],
            b"<svg><foreignObject><div/></foreignObject></svg>",
            b"<svg onload=\"alert(1)\"/>",
            b"<svg><a href=\"javascript:alert(1)\">x</a></svg>",
        ] {
            assert!(check_svg(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        // "on" inside words or values is not an event handler.
        assert!(check_svg(b"<svg><text font-family=\"mono\">online</text></svg>").is_ok());
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("C:\\x\\a:b*?.txt"), "a_b__.txt");
        assert_eq!(sanitize_name("..."), "file");
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name("a\nb[1].pdf"), "a_b_1_.pdf");
        assert_eq!(sanitize_name(&"x".repeat(300)).len(), 120);
    }
}
