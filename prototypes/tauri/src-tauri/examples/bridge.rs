//! Headless test bridge: serves the Vite build (`dist/`) and answers the
//! interface's commands over HTTP (`POST /invoke/<command>` with the JSON
//! arguments), calling the real `vault_core` through `Backend`. Playwright
//! drives headless Chromium against it; no app window is ever opened.
//!
//!   cargo run -p notes-tauri --release --example bridge -- \
//!     --port 4480 --data <folder> --dist ../dist [--pages 10000 --big 2000]
//!
//! Standard library only (no extra dependencies): one thread per connection,
//! `Connection: close` on every response.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notes_tauri_lib::backend::{Backend, SEED_BIG_BLOCKS, SEED_PAGES};

fn main() {
    let mut port = 4480u16;
    let mut data = std::env::temp_dir().join("notes-tauri-bridge");
    let mut dist = PathBuf::from("../dist");
    let mut pages = SEED_PAGES;
    let mut big = SEED_BIG_BLOCKS;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--port" => port = v.parse().expect("port"),
            "--data" => data = PathBuf::from(v),
            "--dist" => dist = PathBuf::from(v),
            "--pages" => pages = v.parse().expect("pages"),
            "--big" => big = v.parse().expect("big"),
            _ => panic!("unknown argument {a}"),
        }
    }
    std::fs::create_dir_all(&data).expect("data dir");
    let backend = Arc::new(Backend::with_seed(data.clone(), pages, big));
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    println!("bridge listening on http://127.0.0.1:{port} data={} dist={}", data.display(), dist.display());
    let dist = Arc::new(dist);
    for stream in listener.incoming().flatten() {
        let b = Arc::clone(&backend);
        let d = Arc::clone(&dist);
        std::thread::spawn(move || {
            if let Err(e) = handle(stream, &b, &d) {
                eprintln!("bridge: {e}");
            }
        });
    }
}

fn handle(stream: TcpStream, b: &Backend, dist: &Path) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    let mut out = stream;

    if method == "POST" {
        if let Some(cmd) = path.strip_prefix("/invoke/") {
            let args: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
            let (status, json) = match b.dispatch(cmd, &args) {
                Ok(v) => ("200 OK", v),
                Err(e) => ("500 Internal Server Error", serde_json::Value::String(e)),
            };
            let bytes = serde_json::to_vec(&json).unwrap_or_default();
            return respond(&mut out, status, "application/json", &bytes);
        }
    }
    if method != "GET" {
        return respond(&mut out, "405 Method Not Allowed", "text/plain", b"method not allowed");
    }
    let clean = path.split('?').next().unwrap_or("/").trim_start_matches('/');
    let mut file = dist.join(if clean.is_empty() { "index.html" } else { clean });
    if clean.contains("..") || !file.is_file() {
        file = dist.join("index.html");
    }
    match std::fs::read(&file) {
        Ok(bytes) => respond(&mut out, "200 OK", mime(&file), &bytes),
        Err(_) => respond(&mut out, "404 Not Found", "text/plain", b"not found (build the frontend first)"),
    }
}

fn mime(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn respond(out: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    out.write_all(body)?;
    out.flush()
}
