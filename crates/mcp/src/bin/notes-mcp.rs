//! `notes-mcp`: the notes vault as an MCP server over stdio.
//!
//! ```text
//! notes-mcp [--vault DIR]   stdio (Claude Code, Claude desktop, Codex)
//! ```
//!
//! Each tool call opens the vault, runs and closes it, so the vault is held
//! only during the call. While another program holds the vault open (the
//! desktop app does while it runs), a call fails and says so: close that
//! program and try again.
//!
//! The vault defaults to the app's: `NOTES_VAULT`, else the `vaultPath` in
//! the app's `settings.json`, else `<data dir>/vault`, where the data dir is
//! `NOTES_DATA_DIR`, `%LOCALAPPDATA%\Notes` (Windows) or
//! `~/.local/share/notes_app`.

use std::io::{stdin, stdout, BufReader};
use std::path::PathBuf;

use serde_json::Value;
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
    let mut args = std::env::args().skip(1);
    let mut vault = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--vault" => {
                vault = Some(
                    args.next()
                        .unwrap_or_else(|| fail("--vault needs a folder")),
                )
            }
            "-h" | "--help" => {
                println!("notes-mcp [--vault DIR]");
                return;
            }
            other => fail(&format!("unknown argument {other} (see --help)")),
        }
    }
    let root = vault.unwrap_or_else(default_vault);
    serve_stdio(&root);
}

fn serve_stdio(root: &str) {
    let server = McpServer::per_call(root.to_string());
    let r = stdio::serve(BufReader::new(stdin().lock()), stdout().lock(), |msg| {
        server.handle(msg)
    });
    if let Err(e) = r {
        fail(&e.to_string());
    }
}
