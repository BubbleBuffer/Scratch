//! The web viewer and shares.
//!
//! - Web sessions: after a browser login (`/auth/web`, see `login.rs`) the
//!   hub sets an opaque `scratch_session` cookie (HttpOnly, Secure,
//!   SameSite=Lax). The store keeps only its SHA-256, the account and a
//!   30-day expiry, in `.vault/hub/web-sessions.json`.
//!   `POST /auth/web/logout` ends it.
//! - `GET /open/<id>` shows the rendered page to a web session, and the
//!   bounce page (plus "Sign in to view in the browser") to anyone else.
//! - Shares, in `.vault/hub/shares.json`: `https://<hub>/s/<token>`, public
//!   or private (a web session needed), a snapshot (HTML and attachments
//!   frozen in `.vault/hub/shares/<id>/`) or live (rendered per request).
//!   Revoked links answer 410.
//! - `GET|POST /shares`, `DELETE /shares/<id>`: the share API for a device
//!   token (with `X-Device`, checked like `/sync`) or a web session (with a
//!   same-origin `Origin` on changes). Agent tokens are refused.
//!
//! Wiki: `wiki/integrations/sharing.md`.

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::rejection::{FormRejection, JsonRejection};
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vault_core::{is_page_id, sha256_hex, Page, Vault};
use vault_sync::DEVICE_HEADER;

use crate::agent_tokens::write_atomic;
use crate::html::{self, Card, Chrome};
use crate::render::{self, is_attachment_id, RenderOpts};
use crate::AppState;

pub(crate) const COOKIE: &str = "scratch_session";
const SESSION_MS: i64 = 30 * 24 * 3600 * 1000;
const MAX_SESSIONS: usize = 1000;
const ICON: &[u8] = include_bytes!("../assets/icon-512.png");

const PAGE_CSP: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; \
                        base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
const ASSET_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub(crate) fn random_hex(bytes: usize) -> Result<String, String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| format!("random: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &FsPath) -> T {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::error!(path = %path.display(), "unreadable, starting empty: {e}");
            T::default()
        }),
        Err(_) => T::default(),
    }
}

fn save_json<T: Serialize>(path: &FsPath, value: &T) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    write_atomic(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

// ---- web sessions ----

#[derive(Clone, Serialize, Deserialize)]
struct SessionEntry {
    /// SHA-256 of the cookie value.
    hash: String,
    account: String,
    created: i64,
    expires: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct Session {
    pub account: String,
}

pub(crate) struct Sessions {
    path: PathBuf,
    entries: Mutex<Vec<SessionEntry>>,
}

impl Sessions {
    fn open(hub_dir: &FsPath) -> Sessions {
        let path = hub_dir.join("web-sessions.json");
        let entries: Vec<SessionEntry> = load_json(&path);
        Sessions {
            path,
            entries: Mutex::new(entries),
        }
    }

    /// A new session for `account`; returns the cookie value.
    pub fn create(&self, account: &str) -> Result<String, String> {
        let secret = random_hex(32)?;
        let now = now_ms();
        let mut entries = lock(&self.entries);
        entries.retain(|e| e.expires > now);
        if entries.len() >= MAX_SESSIONS {
            entries.sort_by_key(|e| e.created);
            let drop = entries.len() + 1 - MAX_SESSIONS;
            entries.drain(..drop);
        }
        entries.push(SessionEntry {
            hash: sha256_hex(secret.as_bytes()),
            account: account.to_string(),
            created: now,
            expires: now + SESSION_MS,
        });
        save_json(&self.path, &*entries)?;
        Ok(secret)
    }

    pub fn check(&self, secret: &str) -> Option<Session> {
        let hash = sha256_hex(secret.as_bytes());
        let now = now_ms();
        lock(&self.entries)
            .iter()
            .find(|e| e.expires > now && vault_sync::ct_eq(e.hash.as_bytes(), hash.as_bytes()))
            .map(|e| Session {
                account: e.account.clone(),
            })
    }

    pub fn remove(&self, secret: &str) {
        let hash = sha256_hex(secret.as_bytes());
        let mut entries = lock(&self.entries);
        let before = entries.len();
        entries.retain(|e| e.hash != hash);
        if entries.len() != before {
            if let Err(e) = save_json(&self.path, &*entries) {
                tracing::error!("web sessions: {e}");
            }
        }
    }
}

/// The `Set-Cookie` value for a new session.
pub(crate) fn session_cookie(secret: &str) -> String {
    format!(
        "{COOKIE}={secret}; Path=/; Max-Age={}; HttpOnly; Secure; SameSite=Lax",
        SESSION_MS / 1000
    )
}

fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax")
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == COOKIE && !v.is_empty()).then(|| v.to_string())
        })
}

pub(crate) fn session(state: &AppState, headers: &HeaderMap) -> Option<Session> {
    let secret = cookie_value(headers)?;
    state.web.sessions.check(&secret)
}

/// Where a browser login may return to: `/open/<page id>`, `/s/<token>`
/// or `/shares`.
pub(crate) fn valid_return(path: &str) -> bool {
    if path == "/shares" {
        return true;
    }
    if let Some(id) = path.strip_prefix("/open/") {
        return is_page_id(id);
    }
    if let Some(t) = path.strip_prefix("/s/") {
        return !t.is_empty() && t.len() <= 128 && t.bytes().all(|b| b.is_ascii_hexdigit());
    }
    false
}

// ---- shares ----

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Visibility {
    Public,
    Private,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Snapshot,
    Live,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Share {
    id: String,
    token: String,
    page: String,
    visibility: Visibility,
    kind: Kind,
    created: i64,
    created_by: String,
    #[serde(default)]
    revoked: Option<i64>,
    /// A snapshot's content version ([`content_version`]); shares made
    /// before 2026-10-23 have none and are never reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

impl Share {
    /// Whether creating `(page, visibility, kind, content)` should return
    /// this share instead of a new one.
    fn reusable_for(
        &self,
        page: &str,
        visibility: Visibility,
        kind: Kind,
        content: Option<&str>,
    ) -> bool {
        self.revoked.is_none()
            && self.page == page
            && self.visibility == visibility
            && self.kind == kind
            && (kind == Kind::Live || (content.is_some() && self.content.as_deref() == content))
    }
}

/// A snapshot's attachments: id → (mime, name).
type SnapshotAssets = BTreeMap<String, (String, String)>;

pub(crate) struct Shares {
    path: PathBuf,
    snapshots: PathBuf,
    entries: Mutex<Vec<Share>>,
}

impl Shares {
    fn open(hub_dir: &FsPath) -> Shares {
        let path = hub_dir.join("shares.json");
        let entries: Vec<Share> = load_json(&path);
        Shares {
            path,
            snapshots: hub_dir.join("shares"),
            entries: Mutex::new(entries),
        }
    }

    fn by_token(&self, token: &str) -> Option<Share> {
        lock(&self.entries)
            .iter()
            .find(|s| vault_sync::ct_eq(s.token.as_bytes(), token.as_bytes()))
            .cloned()
    }

    fn for_page(&self, page: &str) -> Vec<Share> {
        let mut v: Vec<Share> = lock(&self.entries)
            .iter()
            .filter(|s| s.page == page && s.revoked.is_none())
            .cloned()
            .collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.created));
        v
    }

    /// Every share that is not revoked, newest first.
    fn all(&self) -> Vec<Share> {
        let mut v: Vec<Share> = lock(&self.entries)
            .iter()
            .filter(|s| s.revoked.is_none())
            .cloned()
            .collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.created));
        v
    }

    /// The newest share that creating `share` again should return.
    fn reusable(&self, share: &Share) -> Option<Share> {
        Self::newest_match(&lock(&self.entries), share)
    }

    fn newest_match(entries: &[Share], share: &Share) -> Option<Share> {
        entries
            .iter()
            .filter(|s| {
                s.reusable_for(
                    &share.page,
                    share.visibility,
                    share.kind,
                    share.content.as_deref(),
                )
            })
            .max_by_key(|s| s.created)
            .cloned()
    }

    /// Adds `share`, unless a matching one was added meanwhile: then that
    /// one is returned and `share` is not stored.
    fn add(&self, share: Share) -> Result<Option<Share>, String> {
        let mut entries = lock(&self.entries);
        if let Some(existing) = Self::newest_match(&entries, &share) {
            return Ok(Some(existing));
        }
        entries.push(share);
        save_json(&self.path, &*entries)?;
        Ok(None)
    }

    /// Marks a share revoked and deletes its snapshot; false if unknown or
    /// already revoked.
    fn revoke(&self, id: &str) -> Result<bool, String> {
        let mut entries = lock(&self.entries);
        let Some(s) = entries
            .iter_mut()
            .find(|s| s.id == id && s.revoked.is_none())
        else {
            return Ok(false);
        };
        s.revoked = Some(now_ms());
        save_json(&self.path, &*entries)?;
        let _ = std::fs::remove_dir_all(self.snapshots.join(id));
        Ok(true)
    }

    fn snapshot_dir(&self, id: &str) -> PathBuf {
        self.snapshots.join(id)
    }
}

/// The viewer and share state in [`AppState`].
pub(crate) struct Web {
    pub sessions: Sessions,
    shares: Shares,
    /// `public_url` (or the loopback address), without a trailing slash.
    base: String,
    /// Browser login is configured.
    login: bool,
}

impl Web {
    pub fn open(vault_root: &FsPath, base: String, login: bool) -> Web {
        let dir = vault_root.join(".vault").join("hub");
        Web {
            sessions: Sessions::open(&dir),
            shares: Shares::open(&dir),
            base,
            login,
        }
    }

    fn share_url(&self, token: &str) -> String {
        format!("{}/s/{token}", self.base)
    }

    fn origin(&self) -> String {
        match url::Url::parse(&self.base) {
            Ok(u) => u.origin().ascii_serialization(),
            Err(_) => String::new(),
        }
    }
}

// ---- responses ----

fn with_headers(mut r: Response, csp: &str, cache: &str) -> Response {
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(csp) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    if let Ok(v) = HeaderValue::from_str(cache) {
        h.insert(header::CACHE_CONTROL, v);
    }
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    r
}

fn html_response(status: StatusCode, body: String, csp: &str, cache: &str) -> Response {
    with_headers(
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
        csp,
        cache,
    )
}

fn message(status: StatusCode, title: &str, text: &str) -> Response {
    html_response(
        status,
        html::message_document(title, text, None, None),
        PAGE_CSP,
        "no-store",
    )
}

fn json_error(status: StatusCode, error: &str) -> Response {
    let mut r = (status, Json(json!({ "error": error }))).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn asset_response(bytes: Vec<u8>, mime: &str, name: &str, cache: &str) -> Response {
    let inline = mime.starts_with("image/");
    let ctype = if inline {
        mime
    } else {
        "application/octet-stream"
    };
    let mut r = (StatusCode::OK, bytes).into_response();
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(ctype) {
        h.insert(header::CONTENT_TYPE, v);
    }
    if !inline {
        let safe: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
            .take(100)
            .collect();
        let safe = if safe.trim().is_empty() {
            "file".into()
        } else {
            safe
        };
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{safe}\"")) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
    }
    with_headers(r, ASSET_CSP, cache)
}

fn not_found() -> Response {
    message(
        StatusCode::NOT_FOUND,
        "Not found",
        "This note doesn't exist or is no longer available.",
    )
}

pub(crate) async fn icon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        ICON,
    )
        .into_response()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::error!("web: {e}");
            None
        }
    }
}

fn live_page(v: &Vault, id: &str) -> Option<Page> {
    v.load_page(id.to_string()).ok().filter(|p| !p.in_trash)
}

// ---- the viewer ----

#[derive(Deserialize)]
pub(crate) struct OpenQuery {
    block: Option<String>,
}

/// `GET /open/<page id>[?block=<block id>]`.
pub(crate) async fn open_page(
    State(state): State<Arc<AppState>>,
    Path(page): Path<String>,
    Query(q): Query<OpenQuery>,
    headers: HeaderMap,
) -> Response {
    if !is_page_id(&page) {
        return (StatusCode::BAD_REQUEST, "invalid page id\n").into_response();
    }
    if session(&state, &headers).is_none() {
        let extra = if state.web.login {
            format!("<a class=\"btn\" href=\"/auth/web?return=/open/{page}\">View in browser</a>")
        } else {
            String::new()
        };
        return match vault_sync::bounce_page_with(&page, q.block.as_deref(), &extra) {
            Ok(html) => (
                [
                    (header::CONTENT_TYPE, vault_sync::open::CONTENT_TYPE),
                    (header::CACHE_CONTROL, vault_sync::open::CACHE_CONTROL),
                ],
                html,
            )
                .into_response(),
            Err(msg) => (StatusCode::BAD_REQUEST, format!("{msg}\n")).into_response(),
        };
    }
    let Ok(nonce) = random_hex(16) else {
        return message(StatusCode::INTERNAL_SERVER_ERROR, "Error", "Try again.");
    };
    let vault = state.hub.vault().clone();
    let base = state.web.base.clone();
    let id = page.clone();
    let n = nonce.clone();
    let doc = blocking(move || {
        let p = live_page(&vault, &id)?;
        let asset_base = format!("/open/{id}/a");
        let r = render::render_page(
            &vault,
            &p,
            &RenderOpts {
                asset_base: &asset_base,
                link_pages: true,
            },
        );
        let image = match &r.first_image {
            Some(a) => format!("{base}{asset_base}/{a}"),
            None => format!("{base}/icon.png"),
        };
        let url = format!("{base}/open/{id}");
        let card = Card {
            title: &r.title,
            description: &r.description,
            image_url: &image,
            url: &url,
        };
        Some(html::page_document(
            &r,
            &card,
            Chrome::Viewer {
                page_id: &id,
                nonce: &n,
            },
        ))
    })
    .await
    .flatten();
    match doc {
        Some(doc) => html_response(
            StatusCode::OK,
            doc,
            &format!("{PAGE_CSP}; script-src 'nonce-{nonce}'; connect-src 'self'"),
            "no-store",
        ),
        None => not_found(),
    }
}

/// `GET /open/<page id>/a/<attachment id>`: an attachment the page shows,
/// for a web session.
pub(crate) async fn open_asset(
    State(state): State<Arc<AppState>>,
    Path((page, asset)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !is_page_id(&page) || !is_attachment_id(&asset) {
        return not_found();
    }
    if session(&state, &headers).is_none() {
        return message(
            StatusCode::UNAUTHORIZED,
            "Sign in",
            "Sign in to view this file.",
        );
    }
    live_asset(&state, page, asset, "private, no-store").await
}

async fn live_asset(state: &AppState, page: String, asset: String, cache: &str) -> Response {
    let vault = state.hub.vault().clone();
    let found = blocking(move || {
        let p = live_page(&vault, &page)?;
        if !render::page_assets(&p).contains(&asset) {
            return None;
        }
        let info = vault.attachment_info(asset.clone()).ok()?;
        let path = vault.attachment_path(asset).ok()?;
        let bytes = std::fs::read(path).ok()?;
        Some((bytes, info.mime, info.name))
    })
    .await
    .flatten();
    match found {
        Some((bytes, mime, name)) => asset_response(bytes, &mime, &name, cache),
        None => not_found(),
    }
}

#[derive(Deserialize)]
pub(crate) struct LogoutForm {
    #[serde(rename = "return")]
    return_to: Option<String>,
}

/// `POST /auth/web/logout`: ends the web session and clears the cookie.
pub(crate) async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    form: Result<Form<LogoutForm>, FormRejection>,
) -> Response {
    if let Some(secret) = cookie_value(&headers) {
        state.web.sessions.remove(&secret);
    }
    let back = form
        .ok()
        .and_then(|f| f.0.return_to)
        .filter(|r| valid_return(r));
    let mut r = match back {
        Some(to) => (StatusCode::SEE_OTHER, [(header::LOCATION, to)]).into_response(),
        None => message(
            StatusCode::OK,
            "Signed out",
            "You're signed out of this hub.",
        ),
    };
    if let Ok(v) = HeaderValue::from_str(&clear_cookie()) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// `GET /shares` for a browser: every link you can revoke, across pages.
fn shares_page(state: &AppState, headers: &HeaderMap) -> Response {
    if session(state, headers).is_none() {
        if state.web.login {
            return (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, "/auth/web?return=/shares"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
            )
                .into_response();
        }
        return message(
            StatusCode::UNAUTHORIZED,
            "Sign in",
            "Sign in to the hub to see your shared links.",
        );
    }
    let Ok(nonce) = random_hex(16) else {
        return message(StatusCode::INTERNAL_SERVER_ERROR, "Error", "Try again.");
    };
    html_response(
        StatusCode::OK,
        html::shares_document(&nonce),
        &format!("{PAGE_CSP}; script-src 'nonce-{nonce}'; connect-src 'self'"),
        "no-store",
    )
}

// ---- share pages ----

enum Access {
    Ok(Share),
    Response(Response),
}

/// Resolves a share token for a reader. `asset` is set for an attachment
/// request: a private share without a session then gets a bare 401, not
/// the card page.
fn share_access(state: &AppState, token: &str, headers: &HeaderMap, asset: bool) -> Access {
    let valid =
        !token.is_empty() && token.len() <= 128 && token.bytes().all(|b| b.is_ascii_hexdigit());
    let Some(share) = valid.then(|| state.web.shares.by_token(token)).flatten() else {
        return Access::Response(message(
            StatusCode::NOT_FOUND,
            "Link not found",
            "This share link doesn't exist.",
        ));
    };
    if share.revoked.is_some() {
        return Access::Response(message(
            StatusCode::GONE,
            "Link turned off",
            "The owner turned off this share link.",
        ));
    }
    if share.visibility == Visibility::Private && session(state, headers).is_none() {
        if asset {
            return Access::Response(message(
                StatusCode::UNAUTHORIZED,
                "Sign in",
                "Sign in to view this file.",
            ));
        }
        let url = state.web.share_url(&share.token);
        let icon = format!("{}/icon.png", state.web.base);
        let card = Card {
            title: "Private note",
            description: "",
            image_url: &icon,
            url: &url,
        };
        let sign_in = format!("/auth/web?return=/s/{}", share.token);
        let action = state
            .web
            .login
            .then_some((sign_in.as_str(), "Sign in to view"));
        let doc = html::message_document(
            "Private note",
            "Sign in to the hub to open this note.",
            action,
            Some(&card),
        );
        return Access::Response(html_response(StatusCode::OK, doc, PAGE_CSP, "no-store"));
    }
    Access::Ok(share)
}

fn share_cache(share: &Share) -> &'static str {
    match share.visibility {
        Visibility::Public => "no-cache",
        Visibility::Private => "private, no-store",
    }
}

/// A share's document: the page rendered for an anonymous reader (page
/// links as text), with the share's card.
fn share_document(v: &Vault, page: &Page, base: &str, token: &str) -> (String, render::Rendered) {
    let asset_base = format!("/s/{token}/a");
    let r = render::render_page(
        v,
        page,
        &RenderOpts {
            asset_base: &asset_base,
            link_pages: false,
        },
    );
    let image = match &r.first_image {
        Some(a) => format!("{base}{asset_base}/{a}"),
        None => format!("{base}/icon.png"),
    };
    let url = format!("{base}/s/{token}");
    let card = Card {
        title: &r.title,
        description: &r.description,
        image_url: &image,
        url: &url,
    };
    let doc = html::page_document(&r, &card, Chrome::Share);
    (doc, r)
}

/// `GET /s/<token>`.
pub(crate) async fn share_page(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    let share = match share_access(&state, &token, &headers, false) {
        Access::Ok(s) => s,
        Access::Response(r) => return r,
    };
    let cache = share_cache(&share);
    let doc = match share.kind {
        Kind::Snapshot => {
            let path = state.web.shares.snapshot_dir(&share.id).join("index.html");
            tokio::fs::read_to_string(path).await.ok()
        }
        Kind::Live => {
            let vault = state.hub.vault().clone();
            let base = state.web.base.clone();
            let (page, token) = (share.page.clone(), share.token.clone());
            blocking(move || {
                let p = live_page(&vault, &page)?;
                Some(share_document(&vault, &p, &base, &token).0)
            })
            .await
            .flatten()
        }
    };
    match doc {
        Some(doc) => html_response(StatusCode::OK, doc, PAGE_CSP, cache),
        None => not_found(),
    }
}

/// `GET /s/<token>/a/<attachment id>`.
pub(crate) async fn share_asset(
    State(state): State<Arc<AppState>>,
    Path((token, asset)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !is_attachment_id(&asset) {
        return not_found();
    }
    let share = match share_access(&state, &token, &headers, true) {
        Access::Ok(s) => s,
        Access::Response(r) => return r,
    };
    let cache = share_cache(&share);
    match share.kind {
        Kind::Live => live_asset(&state, share.page, asset, cache).await,
        Kind::Snapshot => {
            let dir = state.web.shares.snapshot_dir(&share.id);
            let assets: SnapshotAssets = match tokio::fs::read(dir.join("assets.json")).await {
                Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
                Err(_) => SnapshotAssets::new(),
            };
            let Some((mime, name)) = assets.get(&asset) else {
                return not_found();
            };
            match tokio::fs::read(dir.join(&asset)).await {
                Ok(bytes) => asset_response(bytes, mime, name, cache),
                Err(_) => not_found(),
            }
        }
    }
}

// ---- the share API ----

/// Who is calling the share API: a device (bearer token and `X-Device`,
/// checked like `/sync`) or a web session. `changes` also needs a
/// same-origin `Origin` from a session.
async fn principal(
    state: &AppState,
    headers: &HeaderMap,
    changes: bool,
) -> Result<String, Response> {
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        let token = auth
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        if token.is_empty() {
            return Err(json_error(StatusCode::UNAUTHORIZED, "missing token"));
        }
        let device = headers
            .get(DEVICE_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        return match crate::agent::check(&state.hub, &token, &device).await {
            Ok(()) => Ok(format!("device:{device}")),
            Err((status, msg)) => {
                let msg = msg.trim();
                Err(json_error(
                    status,
                    if msg.is_empty() { "unauthorized" } else { msg },
                ))
            }
        };
    }
    let Some(s) = session(state, headers) else {
        return Err(json_error(StatusCode::UNAUTHORIZED, "sign in"));
    };
    if changes {
        let origin = headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if origin.is_empty() || origin != state.web.origin() {
            return Err(json_error(StatusCode::FORBIDDEN, "bad_origin"));
        }
    }
    Ok(s.account)
}

/// A share for the API. `title` is the page's title, or `None` when the
/// page is deleted or in the trash.
fn share_json(web: &Web, s: &Share, title: Option<&str>) -> Value {
    json!({
        "id": s.id,
        "url": web.share_url(&s.token),
        "page": s.page,
        "title": title,
        "visibility": s.visibility,
        "kind": s.kind,
        "created": s.created,
        "created_by": s.created_by,
    })
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    page: Option<String>,
    all: Option<String>,
}

/// The shares with their pages' titles (`None` for a deleted or trashed
/// page), looked up once per page.
async fn with_titles(state: &AppState, shares: Vec<Share>) -> Vec<(Share, Option<String>)> {
    let vault = state.hub.vault().clone();
    blocking(move || {
        let mut titles: BTreeMap<String, Option<String>> = BTreeMap::new();
        shares
            .into_iter()
            .map(|s| {
                let t = titles
                    .entry(s.page.clone())
                    .or_insert_with(|| {
                        live_page(&vault, &s.page).map(|p| render::display_title(&p))
                    })
                    .clone();
                (s, t)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// `GET /shares?page=<id>`: the page's links that are not revoked, newest
/// first. `GET /shares?all=1`: every such link, deleted pages included.
/// `GET /shares` with neither: the signed-in Shared links page.
pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    if q.page.is_none() && q.all.is_none() {
        return shares_page(&state, &headers);
    }
    if let Err(r) = principal(&state, &headers, false).await {
        return r;
    }
    let found = match q.page {
        Some(page) if is_page_id(&page) => state.web.shares.for_page(&page),
        Some(_) => return json_error(StatusCode::BAD_REQUEST, "invalid_page"),
        None => state.web.shares.all(),
    };
    let shares: Vec<Value> = with_titles(&state, found)
        .await
        .iter()
        .map(|(s, t)| share_json(&state.web, s, t.as_deref()))
        .collect();
    let mut r = Json(json!({ "shares": shares })).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

#[derive(Deserialize)]
pub(crate) struct CreateBody {
    page: String,
    visibility: Visibility,
    kind: Kind,
}

/// Renders a snapshot into `dir`: `index.html`, each attachment by id, and
/// `assets.json` (id → mime and name).
fn write_snapshot(
    v: &Vault,
    page: &Page,
    base: &str,
    token: &str,
    dir: &FsPath,
) -> Result<(), String> {
    let (doc, _) = share_document(v, page, base, token);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut assets = SnapshotAssets::new();
    for a in render::page_assets(page) {
        let (Ok(info), Ok(path)) = (v.attachment_info(a.clone()), v.attachment_path(a.clone()))
        else {
            continue;
        };
        if std::fs::copy(&path, dir.join(&a)).is_ok() {
            assets.insert(a, (info.mime, info.name));
        }
    }
    save_json(&dir.join("assets.json"), &assets)?;
    write_atomic(&dir.join("index.html"), doc.as_bytes()).map_err(|e| e.to_string())
}

/// A page's content version for snapshot reuse: a hash of what a snapshot
/// shows (title, icon, properties, body). Attachments count only through
/// their references in the body.
fn content_version(v: &Vault, page: &Page) -> String {
    let r = render::render_page(
        v,
        page,
        &RenderOpts {
            asset_base: "/a",
            link_pages: false,
        },
    );
    let mut s = String::new();
    for part in [
        r.title.as_str(),
        r.icon.as_deref().unwrap_or(""),
        r.props_html.as_str(),
        r.body_html.as_str(),
    ] {
        s.push_str(part);
        s.push('\0');
    }
    sha256_hex(s.as_bytes())
}

/// `POST /shares` `{page, visibility, kind}`: 201 with a new link, or 200
/// with `existing: true` when the page already has one for this choice
/// (live: per visibility; snapshot: per visibility and content version).
/// Revoked links are never returned.
pub(crate) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    let who = match principal(&state, &headers, true).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let Ok(Json(b)) = body else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if !is_page_id(&b.page) {
        return json_error(StatusCode::BAD_REQUEST, "invalid_page");
    }
    let (Ok(id), Ok(token)) = (random_hex(8), random_hex(24)) else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error");
    };
    let st = state.clone();
    let made = blocking(
        move || -> Result<(Share, bool, String), (StatusCode, &'static str)> {
            let vault = st.hub.vault();
            let p = live_page(vault, &b.page).ok_or((StatusCode::NOT_FOUND, "no_page"))?;
            if p.is_database() {
                return Err((StatusCode::BAD_REQUEST, "databases_not_shareable"));
            }
            let title = render::display_title(&p);
            let share = Share {
                id: id.clone(),
                token: token.clone(),
                page: b.page.clone(),
                visibility: b.visibility,
                kind: b.kind,
                created: now_ms(),
                created_by: who,
                revoked: None,
                content: (b.kind == Kind::Snapshot).then(|| content_version(vault, &p)),
            };
            let shares = &st.web.shares;
            if let Some(existing) = shares.reusable(&share) {
                return Ok((existing, true, title));
            }
            let dir = shares.snapshot_dir(&id);
            let server_error = |e: String| {
                tracing::error!("shares: {e}");
                let _ = std::fs::remove_dir_all(&dir);
                (StatusCode::INTERNAL_SERVER_ERROR, "server_error")
            };
            if b.kind == Kind::Snapshot {
                write_snapshot(vault, &p, &st.web.base, &token, &dir).map_err(server_error)?;
            }
            match shares.add(share.clone()).map_err(server_error)? {
                Some(existing) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    Ok((existing, true, title))
                }
                None => Ok((share, false, title)),
            }
        },
    )
    .await
    .unwrap_or(Err((StatusCode::INTERNAL_SERVER_ERROR, "server_error")));
    let (share, existing, title) = match made {
        Ok(m) => m,
        Err((status, e)) => return json_error(status, e),
    };
    if !existing {
        tracing::info!(share = %share.id, "share created");
    }
    let mut body = share_json(&state.web, &share, Some(&title));
    body["existing"] = json!(existing);
    let status = if existing {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let mut r = (status, Json(body)).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// `DELETE /shares/<id>`.
pub(crate) async fn revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = principal(&state, &headers, true).await {
        return r;
    }
    match state.web.shares.revoke(&id) {
        Ok(true) => {
            tracing::info!(share = %id, "share revoked");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => json_error(StatusCode::NOT_FOUND, "no_share"),
        Err(e) => {
            tracing::error!("shares: {e}");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_are_only_viewer_and_share_paths() {
        assert!(valid_return("/open/01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(valid_return("/s/abcdef0123"));
        assert!(valid_return("/shares"));
        for bad in [
            "/",
            "//evil.example.com",
            "https://evil.example.com",
            "/open/x",
            "/s/",
            "/s/a/b",
            "/auth/web",
            "/shares/x",
            "/shares?all=1",
        ] {
            assert!(!valid_return(bad), "{bad}");
        }
    }

    #[test]
    fn the_cookie_is_read_among_others() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            "a=1; scratch_session=abc; b=2".parse().unwrap(),
        );
        assert_eq!(cookie_value(&h).as_deref(), Some("abc"));
        let cookie = session_cookie("abc");
        for part in [
            "HttpOnly",
            "Secure",
            "SameSite=Lax",
            "Path=/",
            "Max-Age=2592000",
        ] {
            assert!(cookie.contains(part), "{cookie}");
        }
    }
}
