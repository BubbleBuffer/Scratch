//! `notes-mcp`: the notes vault as an MCP server.
//!
//! ```text
//! notes-mcp [--vault DIR]                 stdio (Claude Code, Claude desktop, Codex)
//! notes-mcp [--vault DIR] --http [ADDR]   streamable HTTP on ADDR (default 127.0.0.1:47101)
//! notes-mcp [--vault DIR] --token         print the bearer token for HTTP clients
//! notes-mcp [--vault DIR] --rotate-token  replace it
//! ```
//!
//! Over stdio, tool calls go to the program that has the vault open (the
//! desktop app serves it at the address in `.vault/mcp/endpoint.json`); when
//! nothing has it open, each call opens the vault, runs and closes it.
//! `--http` holds the vault open: use it only while the app is not running.
//!
//! The vault defaults to the app's: `NOTES_VAULT`, else the `vaultPath` in
//! the app's `settings.json`, else `<data dir>/vault`, where the data dir is
//! `NOTES_DATA_DIR`, `%LOCALAPPDATA%\Notes` (Windows) or
//! `~/.local/share/notes_app`.

use std::io::{stdin, stdout, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use vault_mcp::endpoint::{self, DEFAULT_ADDR};
use vault_mcp::http::{HttpOptions, McpHttp};
use vault_mcp::proxy::McpClient;
use vault_mcp::{stdio, McpServer};

fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("NOTES_DATA_DIR").filter(|d| !d.is_empty()) {
        return d.into();
    }
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        return base.join("Notes");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".local/share/notes_app")
}

fn default_vault() -> String {
    if let Some(v) = std::env::var_os("NOTES_VAULT").filter(|v| !v.is_empty()) {
        return v.to_string_lossy().into_owned();
    }
    let data = data_dir();
    let from_settings = std::fs::read(data.join("settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|j| j.get("vaultPath").and_then(Value::as_str).map(String::from));
    from_settings.unwrap_or_else(|| data.join("vault").to_string_lossy().into_owned())
}

fn fail(msg: &str) -> ! {
    eprintln!("notes-mcp: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut vault = None;
    let mut http: Option<String> = None;
    let mut print_token = false;
    let mut rotate = false;
    let mut origins = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--vault" => {
                vault = Some(
                    args.next()
                        .unwrap_or_else(|| fail("--vault needs a folder")),
                )
            }
            "--http" => {
                let addr = match args.peek() {
                    Some(n) if !n.starts_with("--") => args.next().unwrap(),
                    _ => DEFAULT_ADDR.to_string(),
                };
                http = Some(addr);
            }
            "--allow-origin" => origins.push(
                args.next()
                    .unwrap_or_else(|| fail("--allow-origin needs a value")),
            ),
            "--token" => print_token = true,
            "--rotate-token" => rotate = true,
            "-h" | "--help" => {
                println!(
                    "notes-mcp [--vault DIR] [--http [ADDR]] [--allow-origin ORIGIN] [--token] [--rotate-token]"
                );
                return;
            }
            other => fail(&format!("unknown argument {other} (see --help)")),
        }
    }
    let root = vault.unwrap_or_else(default_vault);
    if rotate {
        let t = endpoint::rotate_token(&root).unwrap_or_else(|e| fail(&e.0));
        println!("{t}");
        return;
    }
    if print_token {
        println!("{}", endpoint::token(&root).unwrap_or_else(|e| fail(&e.0)));
        return;
    }
    match http {
        Some(addr) => serve_http(&root, &addr, origins),
        None => serve_stdio(&root),
    }
}

fn serve_http(root: &str, addr: &str, allowed_origins: Vec<String>) {
    let v = vault_core::Vault::open(root.to_string()).unwrap_or_else(|e| fail(&e.to_string()));
    if !v.is_owner() {
        let at = endpoint::read_endpoint(root)
            .map(|e| format!(" (it serves MCP at {})", e.url))
            .unwrap_or_default();
        fail(&format!(
            "the vault is open in another program, probably the notes app{at}"
        ));
    }
    let token = endpoint::token(root).unwrap_or_else(|e| fail(&e.0));
    let server = Arc::new(McpServer::new(Arc::new(v)));
    let h = McpHttp::serve(addr, server, token, HttpOptions { allowed_origins })
        .unwrap_or_else(|e| fail(&e.0));
    let url = h.url();
    let _ = endpoint::write_endpoint(root, &url);
    eprintln!("notes-mcp: serving {root} at {url} (token: notes-mcp --token)");
    loop {
        std::thread::park();
    }
}

fn serve_stdio(root: &str) {
    let local = McpServer::per_call(root.to_string());
    let handle = |msg: Value| -> Option<Value> {
        if msg.get("method").and_then(Value::as_str) == Some("tools/call") {
            if let Some(ep) = endpoint::read_endpoint(root) {
                let token = endpoint::token(root).unwrap_or_default();
                match McpClient::new(&ep.url, &token).forward(&msg) {
                    Ok(r) => return r,
                    // The app answered but refused: report it.
                    Err(e) if !e.0.starts_with("cannot reach") => {
                        let id = msg.get("id").cloned().unwrap_or(Value::Null);
                        return Some(serde_json::json!({ "jsonrpc": "2.0", "id": id,
                            "error": { "code": -32000, "message": e.0 } }));
                    }
                    // A stale endpoint file: nobody is serving; open the vault here.
                    Err(_) => {}
                }
            }
        }
        local.handle(msg)
    };
    let r = stdio::serve(BufReader::new(stdin().lock()), stdout().lock(), handle);
    if let Err(e) = r {
        fail(&e.to_string());
    }
}
