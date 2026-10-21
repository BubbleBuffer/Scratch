//! Streamable HTTP adapter for agent tokens and optional OAuth callers.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde_json::Value;
use vault_mcp::McpServer;

use crate::agent_tokens::AgentScope;
use crate::mcp_log;
use crate::mcp_oauth::AuthError;
use crate::AppState;

const MAX_BODY: usize = 8 << 20;

// Only archive string values receive the larger allowance. Bound all other
// JSON bytes together at the ordinary request limit. Oversized strings are
// discarded while reading; retaining the surrounding JSON lets the error name
// its source even when the name occurs after the archive in the request.
pub(crate) struct ImportBody {
    pub bytes: Vec<u8>,
    pub total: usize,
    metadata: usize,
    in_string: bool,
    escaped: bool,
    token: Vec<u8>,
    archive_key: bool,
    colon: bool,
    archive: Option<(usize, usize)>,
    archive_count: usize,
    oversized: Vec<usize>,
    archive_wire_limit: usize,
}

impl ImportBody {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            total: 0,
            metadata: 0,
            in_string: false,
            escaped: false,
            token: Vec::new(),
            archive_key: false,
            colon: false,
            archive: None,
            archive_count: 0,
            oversized: Vec::new(),
            // JSON can escape each ASCII base64 character as six bytes.
            archive_wire_limit: limit.div_ceil(3) * 24,
        }
    }

    fn extend(&mut self, chunk: &[u8]) -> Result<(), ()> {
        for &b in chunk {
            self.total = self.total.saturating_add(1);
            let mut keep = true;
            if self.in_string {
                let closing = b == b'"' && !self.escaped;
                if let Some((start, len)) = &mut self.archive {
                    if !closing {
                        *len = len.saturating_add(1);
                        if *len > self.archive_wire_limit {
                            self.bytes.truncate(*start);
                            keep = false;
                        }
                    }
                } else {
                    self.metadata += 1;
                    if self.token.len() < 128 {
                        self.token.push(b);
                    }
                }
                if closing {
                    self.in_string = false;
                    if let Some((_, len)) = self.archive.take() {
                        if len > self.archive_wire_limit {
                            self.oversized.push(self.archive_count - 1);
                        }
                        self.archive_key = false;
                    } else {
                        self.token.pop(); // closing quote
                        let mut quoted = Vec::with_capacity(self.token.len() + 2);
                        quoted.push(b'"');
                        quoted.extend_from_slice(&self.token);
                        quoted.push(b'"');
                        self.archive_key = serde_json::from_slice::<String>(&quoted)
                            .is_ok_and(|key| key == "archive_base64");
                    }
                    self.colon = false;
                }
                self.escaped = b == b'\\' && !self.escaped;
            } else {
                self.metadata += 1;
                if b == b'"' {
                    self.in_string = true;
                    self.escaped = false;
                    self.token.clear();
                    if self.archive_key && self.colon {
                        self.archive = Some((self.bytes.len() + 1, 0));
                        self.archive_count += 1;
                    }
                    self.archive_key = false;
                    self.colon = false;
                } else if b == b':' && self.archive_key {
                    self.colon = true;
                } else if !b.is_ascii_whitespace() {
                    self.archive_key = false;
                    self.colon = false;
                }
            }
            if self.metadata > MAX_BODY {
                return Err(());
            }
            if keep {
                self.bytes.push(b);
            }
        }
        Ok(())
    }
}

pub(crate) async fn read_body(body: Body, limit: usize) -> Result<ImportBody, ()> {
    let mut body = body.into_data_stream();
    let mut input = ImportBody::new(limit);
    while let Some(chunk) = body.next().await {
        input.extend(&chunk.map_err(|_| ())?)?;
    }
    Ok(input)
}

pub(crate) async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response {
    // An unusable optional token store disables every request to this route,
    // including requests without credentials or with a different method.
    let st = state.clone();
    match tokio::task::spawn_blocking(move || st.agent_tokens.check_available()).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => return unavailable(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response(),
    }
    if !origins_allowed(req.headers(), state.mcp_oauth.is_some()) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    if req.method() != Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "POST")],
            "use POST",
        )
            .into_response();
    }
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| {
            h.strip_prefix("Bearer ")
                .or_else(|| h.strip_prefix("bearer "))
        })
        .map(|t| t.trim().to_owned());
    let Some(token) = token else {
        return unauthorized(&state, false);
    };
    // File metadata and reloads run off the async executor, as do vault calls.
    let st = state.clone();
    let agent_token = token.clone();
    let identity =
        tokio::task::spawn_blocking(move || st.agent_tokens.authenticate(&agent_token)).await;
    let (actor, read_only) = match identity {
        Ok(Ok(Some(identity))) => (
            format!("agent:{}", identity.name),
            identity.scope == AgentScope::Read,
        ),
        Ok(Ok(None)) => match &state.mcp_oauth {
            Some(oauth) => match oauth.authenticate(&token).await {
                Ok(user) => (
                    format!("oauth:{}", user.email.as_deref().unwrap_or(&user.sub)),
                    false,
                ),
                Err(AuthError::InvalidToken) => return unauthorized(&state, true),
                Err(AuthError::Forbidden) => {
                    return (StatusCode::FORBIDDEN, "forbidden").into_response()
                }
                Err(AuthError::Unavailable) => {
                    return (StatusCode::SERVICE_UNAVAILABLE, "OAuth unavailable").into_response()
                }
            },
            None => return unauthorized(&state, true),
        },
        Ok(Err(_)) => return unavailable(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response(),
    };
    let input = match read_body(req.into_body(), state.import_source_limit_bytes).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response(),
    };
    dispatch_vault(state, input, actor, read_only).await
}

/// Both MCP endpoints use the same vault dispatch, import limits and attribution.
pub(crate) async fn dispatch_vault(
    state: Arc<AppState>,
    input: ImportBody,
    actor: String,
    read_only: bool,
) -> Response {
    let started = Instant::now();
    let request: Option<Value> = serde_json::from_slice(&input.bytes).ok();
    // The calls are kept by name and argument keys only, for the log line.
    let calls = request.as_ref().map(mcp_log::Call::all).unwrap_or_default();
    // A repo import (the hidden import_markdown tool) may exceed MAX_BODY
    // up to the per-source archive limit.
    let import_call = request.as_ref().is_some_and(|r| {
        r["method"] == "tools/call"
            && r["params"]["name"].as_str().is_some_and(|name| {
                vault_tools::import_args(name, &r["params"]["arguments"]).is_some()
            })
    });
    drop(request);
    if input.total > MAX_BODY && !import_call {
        mcp_log::log_vault_failure(&calls, mcp_log::Outcome::InvalidArgs, started);
        return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response();
    }
    let reply = tokio::task::spawn_blocking(move || {
        let vault = state.hub.vault();
        let mcp = McpServer::new(vault.clone());
        mcp.handle_body_with(&input.bytes, read_only, |name, args| {
            vault.with_actor(&actor, || {
                if let Some(import) = vault_tools::import_args(name, args) {
                    let args = &import?;
                    if let Some(&index) = input.oversized.first() {
                        let source = args["roots"][index]["name"]
                            .as_str()
                            .unwrap_or("unnamed source");
                        return Err(vault_mcp::McpError(format!(
                            "source {source:?} exceeds limit {} archive bytes before base64",
                            state.import_source_limit_bytes
                        )));
                    }
                    vault_tools::wiki_import::call(vault, args, state.import_source_limit_bytes)
                        .map_err(Into::into)
                } else {
                    vault_mcp::tools::call(vault, name, args)
                }
            })
        })
    })
    .await;
    match reply {
        Ok(reply) => {
            mcp_log::log_vault(&calls, read_only, &reply.body, started);
            (
                StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                [(header::CONTENT_TYPE, "application/json")],
                reply.body,
            )
                .into_response()
        }
        Err(_) => {
            mcp_log::log_vault_failure(&calls, mcp_log::Outcome::Internal, started);
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}

fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "agent tokens unavailable").into_response()
}

fn unauthorized(state: &AppState, rejected: bool) -> Response {
    let challenge = state.mcp_oauth.as_ref().map_or_else(
        || HeaderValue::from_static("Bearer"),
        |oauth| oauth.challenge(rejected),
    );
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, challenge)],
        "unauthorized",
    )
        .into_response()
}

pub(crate) fn origins_allowed(headers: &axum::http::HeaderMap, oauth_enabled: bool) -> bool {
    let mut count = 0;
    let mut allowed = true;
    for origin in headers.get_all(header::ORIGIN) {
        log_origin(origin);
        count += 1;
        allowed &= oauth_enabled
            && matches!(
                origin.as_bytes(),
                b"https://claude.ai" | b"https://claude.com" | b"https://chatgpt.com"
            );
    }
    allowed && count <= 1
}

fn log_origin(origin: &HeaderValue) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if seen.len() < 32 && seen.insert(vault_core::sha256_hex(origin.as_bytes())) {
        let value: String = String::from_utf8_lossy(origin.as_bytes())
            .chars()
            .take(100)
            .collect();
        tracing::debug!(origin = %value, "MCP request Origin");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_budget_handles_chunk_boundaries_and_keeps_source_name() {
        let json = format!(
            r#"{{"method":"tools/call","params":{{"arguments":{{"roots":[{{"archive_base64":"{}","name":"wiki"}}]}},"name":"create_page"}}}}"#,
            "a".repeat(100)
        );
        for chunk_size in [1, 2, 7, json.len()] {
            let mut body = ImportBody::new(3);
            for chunk in json.as_bytes().chunks(chunk_size) {
                body.extend(chunk).unwrap();
            }
            assert_eq!(body.oversized, vec![0]);
            let value: Value = serde_json::from_slice(&body.bytes).unwrap();
            assert_eq!(value["params"]["arguments"]["roots"][0]["name"], "wiki");
            assert_eq!(
                value["params"]["arguments"]["roots"][0]["archive_base64"],
                ""
            );
        }
    }

    #[test]
    fn archive_budget_keeps_valid_json_and_enforces_metadata_limit() {
        let json = br#"{"name":"create_page","roots":[{"archive_base64":"YQ==","name":"wiki"},{"archive_base64":"Yg==","name":"docs"}],"other":"escaped \" quote"}"#;
        let mut body = ImportBody::new(3);
        for chunk in json.chunks(1) {
            body.extend(chunk).unwrap();
        }
        assert_eq!(body.bytes, json);
        assert!(body.oversized.is_empty());
        assert!(body.extend(&vec![b' '; MAX_BODY]).is_err());
    }

    #[test]
    fn escaped_archive_key_and_value_receive_the_same_budget() {
        let json =
            br#"{"roots":[{"archive_\u0062ase64":"\u0059\u0051\u003d\u003d","name":"wiki"}]}"#;
        let mut body = ImportBody::new(3);
        for chunk in json.chunks(1) {
            body.extend(chunk).unwrap();
        }
        assert_eq!(body.archive_count, 1);
        assert_eq!(body.bytes, json);
        assert!(body.oversized.is_empty());
        assert!(body.metadata < body.total);
    }
}
