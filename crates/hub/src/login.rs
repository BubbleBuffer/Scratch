//! The login routes: `/auth/start`, `/auth/callback`, `POST /auth/redeem`
//! and `POST /auth/signout`, and `/auth/web`, the browser login of the web
//! viewer (same provider and callback; it ends in a web session cookie,
//! see `web.rs`). Started logins (10 min) and one-time codes
//! (2 min) live in memory, keyed by the SHA-256 of their secret.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use openidconnect::{Nonce, PkceCodeVerifier};
use serde::Deserialize;
use serde_json::json;
use url::Url;
use vault_core::sha256_hex;
use vault_sync::SyncError;

use crate::oidc::{Oidc, User};
use crate::{AppState, Config};

const PENDING_TTL: Duration = Duration::from_secs(600);
const CODE_TTL: Duration = Duration::from_secs(120);
/// Started logins kept at most (a flood of `/auth/start` cannot grow
/// memory without bound).
const MAX_PENDING: usize = 10_000;

struct Pending {
    nonce: Nonce,
    verifier: PkceCodeVerifier,
    flow: Flow,
    expires: Instant,
}

/// Where a login goes once the provider sends the browser back.
enum Flow {
    /// The app: a one-time code for a device token, sent to `redirect`.
    App {
        device: String,
        name: String,
        redirect: Url,
        app_state: Option<String>,
    },
    /// The web viewer: a session cookie, then back to `return_to` (a path
    /// checked by [`crate::web::valid_return`]).
    Web { return_to: String },
}

struct Issued {
    token: String,
    label: String,
    account: String,
    device: String,
    expires: Instant,
}

/// The account a login shows: the ID token's email, or its `sub` when the
/// token has no email.
fn account_of(user: &User) -> String {
    user.email
        .as_deref()
        .filter(|e| !e.is_empty())
        .unwrap_or(&user.sub)
        .to_string()
}

pub(crate) struct Login {
    oidc: Oidc,
    pending: Mutex<HashMap<String, Pending>>,
    codes: Mutex<HashMap<String, Issued>>,
    code_ttl_ms: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn random_hex() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("random: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

impl Login {
    pub fn new(config: &Config) -> Result<Login, String> {
        Ok(Login {
            oidc: Oidc::new(config.oidc.clone(), &config.public_url)?,
            pending: Mutex::new(HashMap::new()),
            codes: Mutex::new(HashMap::new()),
            code_ttl_ms: AtomicU64::new(CODE_TTL.as_millis() as u64),
        })
    }

    pub fn set_code_ttl(&self, ttl: Duration) {
        self.code_ttl_ms
            .store(ttl.as_millis() as u64, Ordering::SeqCst);
    }

    fn take_pending(&self, state: &str) -> Option<Pending> {
        let p = lock(&self.pending).remove(&sha256_hex(state.as_bytes()))?;
        (p.expires > Instant::now()).then_some(p)
    }

    fn take_code(&self, code: &str) -> Option<Issued> {
        let mut codes = lock(&self.codes);
        let now = Instant::now();
        codes.retain(|_, c| c.expires > now);
        codes.remove(&sha256_hex(code.as_bytes()))
    }
}

/// Whether `redirect` is an app redirect the hub may send a code to:
/// Exactly `http://127.0.0.1:<port>/done` (ports 1024–65535, the desktop's
/// loopback listener) or `notes://auth` (Android).
pub(crate) fn app_redirect(redirect: &str) -> Option<Url> {
    // Check the supplied spelling before URL parsing can normalize whitespace,
    // dot segments, backslashes, or alternative loopback address spellings.
    let desktop_port = if redirect == "notes://auth" {
        None
    } else {
        let port = redirect
            .strip_prefix("http://127.0.0.1:")?
            .strip_suffix("/done")?;
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let port = port.parse::<u16>().ok()?;
        if port < 1024 {
            return None;
        }
        Some(port)
    };
    let u = Url::parse(redirect).ok()?;
    if u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return None;
    }
    let ok = match desktop_port {
        Some(port) => {
            u.scheme() == "http"
                && u.host_str() == Some("127.0.0.1")
                && u.port() == Some(port)
                && u.path() == "/done"
        }
        None => u.scheme() == "notes" && u.host_str() == Some("auth") && u.path().is_empty(),
    };
    ok.then_some(u)
}

fn valid_device(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 128
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn no_store(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    r
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A small plain HTML error page.
fn page(status: StatusCode, msg: &str) -> Response {
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
         <title>Sign-in failed</title>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:3rem auto;padding:0 1rem\">\
         <h1 style=\"font-size:1.25rem\">Sign-in failed</h1><p>{}</p></body>",
        escape(msg)
    );
    no_store(
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response(),
    )
}

fn found(location: &str) -> Response {
    no_store(
        (
            StatusCode::FOUND,
            [(header::LOCATION, location.to_string())],
        )
            .into_response(),
    )
}

fn json_error(status: StatusCode, error: &str) -> Response {
    no_store((status, Json(json!({ "error": error }))).into_response())
}

/// `redirect` with `params` added to its query.
fn to_app(redirect: &Url, params: &[(&str, &str)], app_state: Option<&str>) -> Response {
    let mut u = redirect.clone();
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
        if let Some(s) = app_state {
            q.append_pair("state", s);
        }
    }
    found(u.as_str())
}

fn not_configured() -> Response {
    page(
        StatusCode::SERVICE_UNAVAILABLE,
        "Login is not configured on this hub.",
    )
}

#[derive(Deserialize)]
pub(crate) struct StartParams {
    device: Option<String>,
    name: Option<String>,
    redirect: Option<String>,
    state: Option<String>,
}

pub(crate) async fn start(
    State(state): State<Arc<AppState>>,
    Query(p): Query<StartParams>,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    let Some(redirect) = p.redirect.as_deref().and_then(app_redirect) else {
        return page(
            StatusCode::BAD_REQUEST,
            "The app's redirect address is not allowed.",
        );
    };
    let device = p.device.unwrap_or_default();
    if !valid_device(&device) {
        return page(StatusCode::BAD_REQUEST, "Missing or invalid device id.");
    }
    let name: String = p
        .name
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    if p.state.as_ref().is_some_and(|s| s.len() > 512) {
        return page(StatusCode::BAD_REQUEST, "The app's state is too long.");
    }
    begin(
        login,
        Flow::App {
            device,
            name,
            redirect,
            app_state: p.state,
        },
    )
    .await
}

#[derive(Deserialize)]
pub(crate) struct WebStartParams {
    #[serde(rename = "return")]
    return_to: Option<String>,
}

/// `GET /auth/web?return=/open/<id>|/s/<token>`: the browser login of the
/// web viewer.
pub(crate) async fn web_start(
    State(state): State<Arc<AppState>>,
    Query(p): Query<WebStartParams>,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    let Some(return_to) = p.return_to.filter(|r| crate::web::valid_return(r)) else {
        return page(
            StatusCode::BAD_REQUEST,
            "The return address is not allowed.",
        );
    };
    begin(login, Flow::Web { return_to }).await
}

/// Starts a login at the provider and remembers it until the callback.
async fn begin(login: &Login, flow: Flow) -> Response {
    let started = match login.oidc.start().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("login start: {e}");
            return page(
                StatusCode::BAD_GATEWAY,
                "The sign-in provider could not be reached. Try again later.",
            );
        }
    };
    {
        let mut pending = lock(&login.pending);
        let now = Instant::now();
        pending.retain(|_, p| p.expires > now);
        if pending.len() >= MAX_PENDING {
            return page(
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many sign-ins in progress. Try again later.",
            );
        }
        pending.insert(
            sha256_hex(started.state.as_bytes()),
            Pending {
                nonce: started.nonce,
                verifier: started.verifier,
                flow,
                expires: now + PENDING_TTL,
            },
        );
    }
    found(&started.authorize_url)
}

#[derive(Deserialize)]
pub(crate) struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub(crate) async fn callback(
    State(state): State<Arc<AppState>>,
    Query(p): Query<CallbackParams>,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    let Some(pending) = p.state.as_deref().and_then(|s| login.take_pending(s)) else {
        return page(
            StatusCode::BAD_REQUEST,
            "This sign-in has expired or was already used. Start again from the app.",
        );
    };
    let (device, name, redirect, app_state) = match pending.flow {
        Flow::App {
            device,
            name,
            redirect,
            app_state,
        } => (device, name, redirect, app_state),
        Flow::Web { return_to } => {
            return web_callback(
                &state,
                login,
                p,
                pending.verifier,
                &pending.nonce,
                &return_to,
            )
            .await
        }
    };
    let app_state = app_state.as_deref();
    let fail = |error: &str| to_app(&redirect, &[("error", error)], app_state);
    if let Some(e) = p.error {
        let e: String = e
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(64)
            .collect();
        tracing::warn!("login: the provider answered {e}");
        return fail(if e.is_empty() { "login_failed" } else { &e });
    }
    let Some(code) = p.code else {
        return fail("invalid_request");
    };
    let user = match login
        .oidc
        .finish(code, pending.verifier, &pending.nonce)
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("login failed: {e}");
            return to_app(&redirect, &[("error", "login_failed")], app_state);
        }
    };
    if !login.oidc.allowed(&user) {
        tracing::warn!(
            sub = %user.sub,
            email = user.email.as_deref().unwrap_or(""),
            "login refused: not in oidc.allowed_users or oidc.allowed_groups"
        );
        return to_app(&redirect, &[("error", "access_denied")], app_state);
    }
    let account = account_of(&user);
    let (hub, dev) = (state.hub.clone(), device.clone());
    let made = tokio::task::spawn_blocking(move || hub.login_device(&dev, &name)).await;
    let (token, label) = match made {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            // SyncError stores formatted details that can quote token-file
            // values. Log only its category and the known credential path.
            let error_kind = match e {
                SyncError::Io(_) => "io",
                SyncError::Status(_, _) => "status",
                SyncError::Http(_) => "http",
                SyncError::UntrustedCertificate { .. } => "untrusted_certificate",
                SyncError::Protocol(_) => "protocol",
                SyncError::Vault(_) => "vault",
            };
            let path =
                std::path::PathBuf::from(state.hub.vault().root()).join(".vault/hub/tokens.json");
            tracing::warn!(path = %path.display(), error_kind, "login: device token refused");
            return to_app(&redirect, &[("error", "device_refused")], app_state);
        }
        Err(e) => {
            tracing::error!("login: {e}");
            return to_app(&redirect, &[("error", "server_error")], app_state);
        }
    };
    let code = match random_hex() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("login: {e}");
            return to_app(&redirect, &[("error", "server_error")], app_state);
        }
    };
    let ttl = Duration::from_millis(login.code_ttl_ms.load(Ordering::SeqCst));
    lock(&login.codes).insert(
        sha256_hex(code.as_bytes()),
        Issued {
            token,
            label: label.clone(),
            account,
            device,
            expires: Instant::now() + ttl,
        },
    );
    tracing::info!(sub = %user.sub, label = %label, "login: device signed in");
    to_app(&redirect, &[("code", &code)], app_state)
}

/// The callback of a web viewer login: a session cookie for an allowed
/// account, then back to the page.
async fn web_callback(
    state: &AppState,
    login: &Login,
    p: CallbackParams,
    verifier: PkceCodeVerifier,
    nonce: &Nonce,
    return_to: &str,
) -> Response {
    if p.error.is_some() {
        return page(
            StatusCode::FORBIDDEN,
            "The sign-in provider refused the sign-in.",
        );
    }
    let Some(code) = p.code else {
        return page(
            StatusCode::BAD_REQUEST,
            "The sign-in provider sent no code.",
        );
    };
    let user = match login.oidc.finish(code, verifier, nonce).await {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("web login failed: {e}");
            return page(
                StatusCode::FORBIDDEN,
                "The sign-in could not be checked. Try again.",
            );
        }
    };
    if !login.oidc.allowed(&user) {
        tracing::warn!(
            sub = %user.sub,
            email = user.email.as_deref().unwrap_or(""),
            "web login refused: not in oidc.allowed_users or oidc.allowed_groups"
        );
        return page(
            StatusCode::FORBIDDEN,
            "This account may not view notes on this hub.",
        );
    }
    let secret = match state.web.sessions.create(&account_of(&user)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("web login: {e}");
            return page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The hub could not sign you in.",
            );
        }
    };
    tracing::info!(sub = %user.sub, "web login: signed in");
    let mut r = found(return_to);
    if let Ok(v) = crate::web::session_cookie(&secret).parse() {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

#[derive(Deserialize)]
pub(crate) struct RedeemBody {
    code: String,
    device: String,
}

pub(crate) async fn redeem(
    State(state): State<Arc<AppState>>,
    Json(b): Json<RedeemBody>,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    // Taken on any attempt: a code is single use.
    match login.take_code(&b.code) {
        Some(i) if vault_sync::ct_eq(i.device.as_bytes(), b.device.as_bytes()) => no_store(
            Json(json!({ "token": i.token, "label": i.label, "account": i.account }))
                .into_response(),
        ),
        _ => json_error(StatusCode::BAD_REQUEST, "invalid_code"),
    }
}

pub(crate) async fn signout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
    else {
        return json_error(StatusCode::UNAUTHORIZED, "missing token");
    };
    let hub = state.hub.clone();
    let t = token.clone();
    match tokio::task::spawn_blocking(move || hub.sign_out(&t)).await {
        Ok(Ok(true)) => {
            // The device's agent sockets end with its token.
            state.agent.close_token(&token);
            state.hosts.close_token(&token);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Ok(false)) => json_error(StatusCode::UNAUTHORIZED, "unknown token"),
        Ok(Err(e)) => {
            tracing::error!("signout: {e}");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
        }
        Err(e) => {
            tracing::error!("signout: {e}");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{account_of, app_redirect};
    use crate::oidc::User;

    #[test]
    fn account_is_the_email_or_the_sub() {
        let user = |email: Option<&str>| User {
            sub: "ann-sub".into(),
            email: email.map(String::from),
            groups: vec![],
        };
        assert_eq!(
            account_of(&user(Some("ann@example.com"))),
            "ann@example.com"
        );
        assert_eq!(account_of(&user(None)), "ann-sub");
        assert_eq!(account_of(&user(Some(""))), "ann-sub");
    }

    #[test]
    fn app_redirects() {
        for ok in [
            "http://127.0.0.1:53111/done",
            "http://127.0.0.1:1024/done",
            "http://127.0.0.1:65535/done",
            "notes://auth",
        ] {
            assert!(app_redirect(ok).is_some(), "{ok}");
        }
        for bad in [
            "https://evil.example/cb",
            "http://127.0.0.1/done",
            "http://localhost:8000/done",
            "http://127.0.0.1:1023/done",
            "http://127.0.0.1:65536/done",
            "http://127.0.0.1:8000/other",
            "http://127.0.0.1:8000/done/",
            "http://127.0.0.1:8000/done?x=1",
            "http://127.0.0.1:8000/done#x",
            "http://127.0.0.1:8000/foo/../done",
            "http://127.0.0.1:8000/%64one",
            "http://127.0.0.1:8000\\done",
            "http://127.1:8000/done",
            "http://127.0.0.1:8000/done\n",
            "http://evil.example:80/done",
            "http://127.0.0.1.evil.example:8000/",
            "http://user@127.0.0.1:8000/",
            "http://127.0.0.1:8000/#x",
            "notes://other",
            "notes://authx",
            "notes://auth/",
            "notes://auth/done",
            "notes://auth?x=1",
            "notes://auth#x",
            "notes://auth:8000",
            "javascript:alert(1)",
            "",
        ] {
            assert!(app_redirect(bad).is_none(), "{bad}");
        }
    }
}
