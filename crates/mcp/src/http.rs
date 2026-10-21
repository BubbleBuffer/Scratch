//! The streamable HTTP transport (MCP 2025-03-26 and later), in its
//! simplest form: `POST /mcp` with one JSON-RPC message (or a batch)
//! answers with `application/json`; notifications get `202 Accepted`. The
//! server never opens an SSE stream (`GET /mcp` is 405) and keeps no
//! sessions.
//!
//! Security:
//! - Every `/mcp` request needs `Authorization: Bearer <token>` (compared in
//!   constant time); others get 401.
//! - Requests with an `Origin` header that is not a localhost origin get
//!   403 (DNS rebinding protection for browsers), unless listed in
//!   [`HttpOptions::allowed_origins`].
//! - Bind 127.0.0.1 (the default). Remote assistants reach it through a
//!   tunnel that adds TLS (see the wiki).
//! - Nothing is logged; the token never leaves this module.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use tiny_http::{Header, Method, Request, Response, Server};

use crate::{McpError, McpServer, Result};

/// Largest request body (markdown for a big page fits easily).
const MAX_BODY: u64 = 8 << 20;

#[derive(Debug, Clone, Default)]
pub struct HttpOptions {
    /// Extra `Origin` values to accept (exact match), for browser-based
    /// clients behind a tunnel.
    pub allowed_origins: Vec<String>,
}

/// A running HTTP endpoint; stops when dropped.
pub struct McpHttp {
    addr: SocketAddr,
    server: Arc<Server>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    mcp: Arc<McpServer>,
    token: String,
    options: HttpOptions,
}

impl McpHttp {
    /// Serves `mcp` on `addr` (for example `127.0.0.1:47101`, or port 0 for
    /// a free one) with bearer `token`.
    pub fn serve(
        addr: &str,
        mcp: Arc<McpServer>,
        token: String,
        options: HttpOptions,
    ) -> Result<McpHttp> {
        if token.len() < 32 {
            return Err(McpError("the token is too short".into()));
        }
        let server = Server::http(addr).map_err(|e| McpError(format!("listen {addr}: {e}")))?;
        let local = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| McpError("not an IP address".into()))?;
        let server = Arc::new(server);
        let stopping = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            mcp,
            token,
            options,
        });
        let (srv, stop) = (server.clone(), stopping.clone());
        let thread = std::thread::Builder::new()
            .name("notes-mcp-http".into())
            .spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let req = match srv.recv() {
                        Ok(r) => r,
                        Err(_) => break,
                    };
                    let sh = shared.clone();
                    let _ = std::thread::Builder::new()
                        .name("notes-mcp-req".into())
                        .spawn(move || respond(&sh, req));
                }
            })?;
        Ok(McpHttp {
            addr: local,
            server,
            stopping,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The URL clients use (`http://127.0.0.1:<port>/mcp`).
    pub fn url(&self) -> String {
        crate::endpoint::url_for(self.addr)
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for McpHttp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn local_origin(o: &str) -> bool {
    let rest = o
        .strip_prefix("http://")
        .or_else(|| o.strip_prefix("https://"))
        .unwrap_or("");
    let host = if rest.starts_with('[') {
        rest.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        rest.split(':').next().unwrap_or("").to_string()
    };
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

fn reply(req: Request, status: u16, body: String, extra: &[(&str, &str)]) {
    let ct = if body.starts_with('{') || body.starts_with('[') {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    let mut resp = Response::from_data(body.into_bytes()).with_status_code(status);
    for (k, v) in [("Content-Type", ct)].iter().chain(extra) {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp = resp.with_header(h);
        }
    }
    let _ = req.respond(resp);
}

fn respond(sh: &Shared, mut req: Request) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("");
    let method = req.method().clone();
    if path == "/health" && method == Method::Get {
        return reply(req, 200, "notes-mcp".into(), &[]);
    }
    if path != "/mcp" {
        return reply(req, 404, "not found".into(), &[]);
    }
    if let Some(o) = header(&req, "Origin") {
        if !local_origin(o) && !sh.options.allowed_origins.iter().any(|a| a == o) {
            return reply(req, 403, "origin not allowed".into(), &[]);
        }
    }
    let authorized = header(&req, "Authorization")
        .and_then(|h| {
            h.strip_prefix("Bearer ")
                .or_else(|| h.strip_prefix("bearer "))
        })
        .is_some_and(|t| ct_eq(t.trim().as_bytes(), sh.token.as_bytes()));
    if !authorized {
        return reply(
            req,
            401,
            "unauthorized".into(),
            &[("WWW-Authenticate", "Bearer")],
        );
    }
    if method != Method::Post {
        return reply(req, 405, "use POST".into(), &[("Allow", "POST")]);
    }
    if req.body_length().is_some_and(|n| n as u64 > MAX_BODY) {
        return reply(req, 413, "request too large".into(), &[]);
    }
    let mut body = Vec::new();
    if req
        .as_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut body)
        .is_err()
        || body.len() as u64 > MAX_BODY
    {
        return reply(req, 413, "request too large".into(), &[]);
    }
    let r = sh.mcp.handle_body(&body);
    reply(req, r.status, r.body, &[])
}
