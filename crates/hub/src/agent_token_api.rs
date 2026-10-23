//! `/agent-tokens`: agent tokens managed from the app.
//!
//! - `GET /agent-tokens` lists them (never the secret or its hash).
//! - `POST /agent-tokens` `{name, scope, expires_days}` creates one and
//!   returns its secret once. `expires_days` is one of
//!   [`EXPIRY_CHOICES_DAYS`]; `null` never expires, and a missing field gets
//!   [`DEFAULT_EXPIRY_DAYS`].
//! - `DELETE /agent-tokens/{name}` revokes one.
//!
//! Every call needs a device token and `X-Device`, checked like `/sync`, so
//! any signed-in device may manage tokens. Agent tokens and OAuth JWTs are
//! not device tokens and are refused (decision 2026-10-21, integration spec
//! phase 4). Responses are `no-store`; nothing here logs a token.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use vault_sync::DEVICE_HEADER;

use crate::agent_tokens::{
    AgentScope, AgentToken, AgentTokenError, DEFAULT_EXPIRY_DAYS, EXPIRY_CHOICES_DAYS,
};
use crate::AppState;

fn no_store(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    r
}

fn error(status: StatusCode, error: &str) -> Response {
    no_store((status, Json(json!({ "error": error }))).into_response())
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
}

/// The device check of `/sync`: `Ok` for a known device token used by its
/// device, else the response to send.
async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    if token.is_empty() {
        return Err(error(StatusCode::UNAUTHORIZED, "missing token"));
    }
    let device = headers
        .get(DEVICE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    crate::agent::check(&state.hub, &token, &device)
        .await
        .map_err(|(status, msg)| {
            let msg = msg.trim();
            let msg = if msg.is_empty() { "unauthorized" } else { msg };
            error(status, msg)
        })
}

fn token_json(t: &AgentToken, now: i64) -> Value {
    json!({
        "name": t.name,
        "scope": t.scope.as_str(),
        "created": t.created,
        "expires": t.expires,
        "last_used": t.last_used,
        "expired": t.expired_at(now),
    })
}

fn store_error(e: &AgentTokenError) -> Response {
    match e {
        AgentTokenError::InvalidName(_) => error(StatusCode::BAD_REQUEST, "invalid_name"),
        AgentTokenError::Duplicate(_) => error(StatusCode::CONFLICT, "duplicate_name"),
        AgentTokenError::Malformed(_) | AgentTokenError::Io(_) => {
            // The message may name the file; keep it in the log only.
            tracing::error!("agent tokens: {e}");
            error(StatusCode::SERVICE_UNAVAILABLE, "tokens_unavailable")
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `GET /agent-tokens`.
pub(crate) async fn list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let st = state.clone();
    match tokio::task::spawn_blocking(move || st.agent_tokens.list()).await {
        Ok(Ok(tokens)) => {
            let now = now_ms();
            let tokens: Vec<Value> = tokens.iter().map(|t| token_json(t, now)).collect();
            no_store(
                Json(json!({
                    "tokens": tokens,
                    "expiry_choices_days": EXPIRY_CHOICES_DAYS,
                    "default_expiry_days": DEFAULT_EXPIRY_DAYS,
                }))
                .into_response(),
            )
        }
        Ok(Err(e)) => store_error(&e),
        Err(_) => internal(),
    }
}

fn default_expiry() -> Option<u32> {
    DEFAULT_EXPIRY_DAYS
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBody {
    name: String,
    scope: String,
    /// Missing: the default lifetime. `null`: never expires.
    #[serde(default = "default_expiry")]
    expires_days: Option<u32>,
}

/// `POST /agent-tokens`.
pub(crate) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<CreateBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let Ok(Json(b)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_body");
    };
    let Ok(scope) = b.scope.parse::<AgentScope>() else {
        return error(StatusCode::BAD_REQUEST, "invalid_scope");
    };
    if !EXPIRY_CHOICES_DAYS.contains(&b.expires_days) {
        return error(StatusCode::BAD_REQUEST, "invalid_expiry");
    }
    let st = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        st.agent_tokens.add_expiring(&b.name, scope, b.expires_days)
    })
    .await;
    match result {
        Ok(Ok((secret, t))) => {
            tracing::info!(token = %t.name, scope = %t.scope, "agent token added from a device");
            let mut v = token_json(&t, now_ms());
            v["token"] = Value::String(secret);
            no_store((StatusCode::CREATED, Json(v)).into_response())
        }
        Ok(Err(e)) => store_error(&e),
        Err(_) => internal(),
    }
}

/// `DELETE /agent-tokens/{name}`.
pub(crate) async fn revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let st = state.clone();
    let n = name.clone();
    match tokio::task::spawn_blocking(move || st.agent_tokens.revoke(&n)).await {
        Ok(Ok(true)) => {
            tracing::info!(token = %name, "agent token revoked from a device");
            no_store(StatusCode::NO_CONTENT.into_response())
        }
        Ok(Ok(false)) => error(StatusCode::NOT_FOUND, "unknown_token"),
        Ok(Err(e)) => store_error(&e),
        Err(_) => internal(),
    }
}
