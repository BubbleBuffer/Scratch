//! The vault's MCP token and the address of the endpoint that is serving
//! it, both in `<vault>/.vault/mcp/` (local files; they never sync).
//!
//! - `token`: 64 hex characters, created on first use. HTTP clients send it
//!   as `Authorization: Bearer <token>`.
//! - `endpoint.json`: `{"url": "http://127.0.0.1:47101/mcp", "pid": 123}`,
//!   written by whoever serves the vault over HTTP (the app, or
//!   `notes-mcp --http`) and removed when it stops. `notes-mcp` over stdio
//!   forwards to it, because only one program may hold the vault open.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{random_hex, Result};

/// The default address of the app's MCP endpoint.
pub const DEFAULT_ADDR: &str = "127.0.0.1:47101";

pub fn mcp_dir(root: &str) -> PathBuf {
    Path::new(root).join(".vault").join("mcp")
}

fn token_path(root: &str) -> PathBuf {
    mcp_dir(root).join("token")
}

fn endpoint_path(root: &str) -> PathBuf {
    mcp_dir(root).join("endpoint.json")
}

/// The vault's MCP token, created on first use.
pub fn token(root: &str) -> Result<String> {
    if let Ok(t) = fs::read_to_string(token_path(root)) {
        let t = t.trim();
        if t.len() >= 32 {
            return Ok(t.to_string());
        }
    }
    rotate_token(root)
}

/// Replaces the token (connected clients must be given the new one).
pub fn rotate_token(root: &str) -> Result<String> {
    let t = random_hex()?;
    fs::create_dir_all(mcp_dir(root))?;
    let tmp = mcp_dir(root).join("token.tmp");
    fs::write(&tmp, &t)?;
    fs::rename(&tmp, token_path(root))?;
    Ok(t)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub url: String,
    pub pid: u32,
}

pub fn write_endpoint(root: &str, url: &str) -> Result<()> {
    fs::create_dir_all(mcp_dir(root))?;
    let e = Endpoint {
        url: url.to_string(),
        pid: std::process::id(),
    };
    let tmp = mcp_dir(root).join("endpoint.json.tmp");
    fs::write(&tmp, serde_json::to_vec(&e)?)?;
    fs::rename(&tmp, endpoint_path(root))?;
    Ok(())
}

pub fn read_endpoint(root: &str) -> Option<Endpoint> {
    let b = fs::read(endpoint_path(root)).ok()?;
    serde_json::from_slice(&b).ok()
}

/// Removes `endpoint.json` if it still names `url` from this process.
pub fn clear_endpoint(root: &str, url: &str) {
    if let Some(e) = read_endpoint(root) {
        if e.url == url && e.pid == std::process::id() {
            let _ = fs::remove_file(endpoint_path(root));
        }
    }
}

/// The URL clients use for a bound address (`0.0.0.0` becomes 127.0.0.1).
pub fn url_for(addr: std::net::SocketAddr) -> String {
    let host = if addr.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else if addr.is_ipv6() {
        format!("[{}]", addr.ip())
    } else {
        addr.ip().to_string()
    };
    format!("http://{host}:{}/mcp", addr.port())
}
