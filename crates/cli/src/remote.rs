//! scratch-cli app ...: drives a running desktop app through its local
//! remote-control endpoint (protocol in wiki/integrations/app-remote.md).
//!
//! The endpoint is a loopback HTTP/1.1 server owned by the Flutter app, so this
//! module carries a small client for it (POST, Connection: close, read to EOF)
//! rather than a general HTTP library.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::Flags;

/// Connecting to a loopback port that nothing listens on is immediate; five
/// seconds only covers a hung app.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Replies are small except screenshot, which base64-encodes a frame.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs one app command line (without the leading word app).
pub fn run(flags: &Flags, out: &mut dyn Write) -> Result<()> {
    let words: Vec<&str> = flags.positional.iter().map(String::as_str).collect();
    let mut screenshot: Option<PathBuf> = None;
    let (command, body) = match words.as_slice() {
        ["status"] => ("status", json!({})),
        ["open", page] => ("open", json!({ "page": *page })),
        ["navigate", route] => ("navigate", json!({ "route": *route })),
        ["screenshot", path] => {
            screenshot = Some(PathBuf::from(*path));
            ("screenshot", json!({ "pixel_ratio": 1.0 }))
        }
        ["tree"] => ("tree", json!({})),
        ["tap", id] => ("tap", json!({ "id": *id })),
        ["focus", id] => ("focus", json!({ "id": *id })),
        ["type", id, text @ ..] if !text.is_empty() => (
            "type",
            json!({ "id": *id, "text": text.join(" "), "replace": true }),
        ),
        ["key", combo] => ("key", json!({ "combo": *combo })),
        [] => bail!("usage:
{}", crate::USAGE),
        _ => bail!(
            "unknown app command {:?}
{}",
            words.join(" "),
            crate::USAGE
        ),
    };
    let data_dir = data_dir(flags)?;
    let endpoint = endpoint(&data_dir)?;
    let value = post(&endpoint, command, &body)?;
    if let Some(path) = &screenshot {
        write_png(&value, path)?;
    }
    if flags.json {
        writeln!(out, "{}", serde_json::to_string(&value)?)?;
        return Ok(());
    }
    match words.as_slice() {
        ["status"] => print_status(&value, out)?,
        ["tree"] => print_tree(&value, out)?,
        ["focus", _] => {
            let focused = value.get("focused").and_then(Value::as_bool).unwrap_or(false);
            writeln!(out, "{}", if focused { "focused" } else { "not focused" })?;
        }
        ["screenshot", path] => {
            let width = number(value.get("width"));
            let height = number(value.get("height"));
            writeln!(out, "wrote {path} ({width}x{height})")?;
        }
        _ => writeln!(out, "ok")?,
    }
    Ok(())
}

/// The app's data folder: --data-dir, else the app's own (NOTES_DATA_DIR,
/// %LOCALAPPDATA%\Notes, ~/.local/share/notes_app).
fn data_dir(flags: &Flags) -> Result<PathBuf> {
    if let Some(dir) = flags.get("--data-dir") {
        let dir = PathBuf::from(dir);
        return std::path::absolute(&dir).with_context(|| format!("resolve {}", dir.display()));
    }
    crate::app_data_dir().ok_or_else(|| {
        anyhow!("cannot find the app's data folder: set NOTES_DATA_DIR or pass --data-dir")
    })
}

/// Where the running app says where it listens: <data dir>/remote/endpoint.json.
struct Endpoint {
    path: PathBuf,
    port: u16,
    token: String,
}

fn endpoint(data_dir: &Path) -> Result<Endpoint> {
    let path = data_dir.join("remote").join("endpoint.json");
    let text = std::fs::read_to_string(&path).map_err(|_| not_running(&path))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| anyhow!("malformed endpoint file {}: {e}", path.display()))?;
    let port = value
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| anyhow!("malformed endpoint file {}: no port", path.display()))?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("malformed endpoint file {}: no token", path.display()))?
        .to_string();
    Ok(Endpoint { path, port, token })
}

/// The message for an app that is not there: no endpoint file, or a refused
/// connection (a file left by a crashed app).
fn not_running(path: &Path) -> anyhow::Error {
    anyhow!(
        "Scratch is not running (no endpoint at {}); start the app first",
        path.display()
    )
}

/// One command over the endpoint. A rejected token or any other ok: false
/// reply is an error carrying the app's own message; success returns the whole
/// reply object.
fn post(endpoint: &Endpoint, command: &str, body: &Value) -> Result<Value> {
    let payload = body.to_string();
    let request = format!(
        "POST /v1/{command} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{payload}",
        port = endpoint.port,
        token = endpoint.token,
        length = payload.len(),
    );
    let addr = SocketAddr::from(([127, 0, 0, 1], endpoint.port));
    let mut stream =
        TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|_| not_running(&endpoint.path))?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .with_context(|| format!("set the read timeout for {command}"))?;
    stream
        .write_all(request.as_bytes())
        .with_context(|| format!("send {command} to Scratch"))?;
    stream
        .flush()
        .with_context(|| format!("send {command} to Scratch"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .with_context(|| format!("read the {command} reply"))?;
    let reply = parse_reply(&raw)?;
    let value: Value = serde_json::from_slice(&reply.body).map_err(|e| {
        anyhow!(
            "the app's {command} reply was not JSON (HTTP {}): {e}",
            reply.status
        )
    })?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(value);
    }
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        bail!("{error}");
    }
    bail!(
        "the app's {command} reply failed (HTTP {}): {}",
        reply.status,
        String::from_utf8_lossy(&reply.body).trim()
    )
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

/// A bare-bones HTTP/1.1 reply: status line, headers, then either a
/// Content-Length body or a chunked one (both Connection: close).
fn parse_reply(raw: &[u8]) -> Result<Reply> {
    let split =
        find(raw, b"\r\n\r\n").ok_or_else(|| anyhow!("the app's reply ended before its headers"))?;
    let head =
        std::str::from_utf8(&raw[..split]).context("the app's reply headers are not UTF-8")?;
    let rest = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| anyhow!("the app's reply had no status line: {status_line:?}"))?;
    let mut chunked = false;
    let mut length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        } else if name == "content-length" {
            length = value.parse::<usize>().ok();
        }
    }
    let body = if chunked {
        dechunk(rest)?
    } else {
        match length {
            Some(length) => rest[..rest.len().min(length)].to_vec(),
            None => rest.to_vec(),
        }
    };
    Ok(Reply { status, body })
}

fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let end =
            find(rest, b"\r\n").ok_or_else(|| anyhow!("the app's chunked reply ended early"))?;
        let line =
            std::str::from_utf8(&rest[..end]).context("the app's reply headers are not UTF-8")?;
        let size = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size, 16)
            .map_err(|_| anyhow!("the app's chunked reply had a bad chunk size: {size:?}"))?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok(body);
        }
        if rest.len() < size {
            bail!("the app's chunked reply ended early");
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size..];
        if let Some(tail) = rest.strip_prefix(b"\r\n".as_slice()) {
            rest = tail;
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|window| window == needle)
}

/// status as key: value lines.
fn print_status(value: &Value, out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "version: {}",
        value.get("version").and_then(Value::as_str).unwrap_or("-")
    )?;
    let pid = value
        .get("pid")
        .and_then(Value::as_i64)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    writeln!(out, "pid: {pid}")?;
    let vault = value.get("vault");
    let open = vault
        .and_then(|vault| vault.get("open"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if open {
        let name = vault
            .and_then(|vault| vault.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        writeln!(out, "vault: {name}")?;
    } else {
        writeln!(out, "vault: none")?;
    }
    match value.get("page").filter(|page| !page.is_null()) {
        Some(page) => writeln!(
            out,
            "page: {} {:?}",
            page.get("id").and_then(Value::as_str).unwrap_or("-"),
            page.get("title").and_then(Value::as_str).unwrap_or("")
        )?,
        None => writeln!(out, "page: none")?,
    }
    Ok(())
}

/// tree as one line per node: <id> <role> "label" [value] (actions).
fn print_tree(value: &Value, out: &mut dyn Write) -> Result<()> {
    let nodes = value.get("nodes").and_then(Value::as_array);
    for node in nodes.into_iter().flatten() {
        let id = node.get("id").and_then(Value::as_str).unwrap_or("");
        let role = node.get("role").and_then(Value::as_str).unwrap_or("other");
        let label = node.get("label").and_then(Value::as_str).unwrap_or("");
        let mut line = format!("{id} {role} {label:?}");
        if let Some(value) = node.get("value").and_then(Value::as_str) {
            if !value.is_empty() {
                line.push_str(&format!(" [{value}]"));
            }
        }
        let actions: Vec<&str> = node
            .get("actions")
            .and_then(Value::as_array)
            .map(|actions| actions.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if !actions.is_empty() {
            line.push_str(&format!(" ({})", actions.join(", ")));
        }
        if node.get("focused").and_then(Value::as_bool) == Some(true) {
            line.push_str(" focused");
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

fn write_png(value: &Value, path: &Path) -> Result<()> {
    let encoded = value
        .get("png_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the app's screenshot reply has no png_base64"))?;
    let png = base64_decode(encoded)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
    }
    std::fs::write(path, &png).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// A JSON number as it should read in text: whole numbers without a .0.
fn number(value: Option<&Value>) -> String {
    match value.and_then(Value::as_f64) {
        Some(n) if n.fract() == 0.0 => format!("{}", n as i64),
        Some(n) => format!("{n}"),
        None => "?".to_string(),
    }
}

fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => bail!("the app's screenshot reply had invalid base64 ({byte:#04x})"),
        } as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((acc >> bits) as u8);
        }
    }
    Ok(bytes)
}
