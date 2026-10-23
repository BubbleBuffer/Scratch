//! Sharing and the web viewer end to end: the mock OIDC issuer of
//! `login.rs`, then the hub's `/open`, `/auth/web`, `/s` and `/shares`.
//!
//! notes-hub end to end in one process: a mock OIDC issuer (discovery,
//! JWKS, authorize, token; RS256 ID tokens signed with a test key), the
//! hub on axum, and `SyncClient`s using the tokens a login gives them.

#![allow(dead_code, unused_imports)]

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use notes_hub::{Config, OidcConfig, Server};
use openidconnect::core::{
    CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreRsaPrivateSigningKey,
};
use openidconnect::{
    AdditionalClaims, Audience, EndUserEmail, IdToken, IdTokenClaims, IssuerUrl, JsonWebKeyId,
    Nonce, PrivateSigningKey, StandardClaims, SubjectIdentifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use vault_core::{
    new_id, sha256_hex, BlockKind, BlockOp, Mark, MarkKind, NewBlock, PropDef, PropKind, PropValue,
    SelectOption, Vault,
};
use vault_sync::{SyncClient, SyncError};

const CLIENT_ID: &str = "notes";
const CLIENT_SECRET: &str = "test-secret";

// ---- temp dirs ----

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("notes_hub_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.0.to_string_lossy().into_owned()).expect("open"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- the mock issuer ----

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Groups {
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<String>>,
}
impl AdditionalClaims for Groups {}

/// Who signs in next, and how the next ID token is spoiled.
#[derive(Clone)]
struct Knobs {
    sub: String,
    email: String,
    groups: Vec<String>,
    wrong_nonce: bool,
    wrong_aud: bool,
    expired: bool,
}

impl Knobs {
    fn user(sub: &str, email: &str, groups: &[&str]) -> Knobs {
        Knobs {
            sub: sub.into(),
            email: email.into(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
            wrong_nonce: false,
            wrong_aud: false,
            expired: false,
        }
    }
}

struct Grant {
    nonce: String,
    challenge: String,
    redirect_uri: String,
    knobs: Knobs,
}

struct Issuer {
    url: String,
    key: CoreRsaPrivateSigningKey,
    knobs: Mutex<Knobs>,
    grants: Mutex<HashMap<String, Grant>>,
    /// Public mode: the token endpoint takes no client secret, only
    /// `client_id` in the body, and refuses any Authorization header.
    public: bool,
    token_requests: Mutex<Vec<TokenRequest>>,
}

/// What the hub sent to the token endpoint.
#[derive(Clone)]
struct TokenRequest {
    authorization: Option<String>,
    form: HashMap<String, String>,
}

fn test_key_pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
        let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string()
    })
}

async fn discovery(State(i): State<Arc<Issuer>>) -> Json<Value> {
    Json(json!({
        "issuer": i.url,
        "authorization_endpoint": format!("{}/authorize", i.url),
        "token_endpoint": format!("{}/token", i.url),
        "jwks_uri": format!("{}/jwks", i.url),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": [if i.public { "none" } else { "client_secret_basic" }],
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn jwks(State(i): State<Arc<Issuer>>) -> Json<Value> {
    Json(json!({ "keys": [i.key.as_verification_key()] }))
}

async fn authorize(
    State(i): State<Arc<Issuer>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["scope"].split(' ').any(|s| s == "openid"));
    let code = new_id();
    i.grants.lock().unwrap().insert(
        code.clone(),
        Grant {
            nonce: q["nonce"].clone(),
            challenge: q["code_challenge"].clone(),
            redirect_uri: q["redirect_uri"].clone(),
            knobs: i.knobs.lock().unwrap().clone(),
        },
    );
    let mut to = Url::parse(&q["redirect_uri"]).unwrap();
    to.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &q["state"]);
    (StatusCode::FOUND, [(header::LOCATION, to.to_string())]).into_response()
}

async fn token(
    State(i): State<Arc<Issuer>>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    i.token_requests.lock().unwrap().push(TokenRequest {
        authorization: authorization.clone(),
        form: f.clone(),
    });
    let accepted = if i.public {
        authorization.is_none()
            && f.get("client_id").map(String::as_str) == Some(CLIENT_ID)
            && !f.contains_key("client_secret")
    } else {
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
        let expected = format!("Basic {basic}");
        authorization.as_deref() == Some(expected.as_str())
    };
    if !accepted {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_client"})),
        )
            .into_response();
    }
    let bad = || {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response()
    };
    let Some(g) = i.grants.lock().unwrap().remove(&f["code"]) else {
        return bad();
    };
    let verifier_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(f["code_verifier"].as_bytes()));
    if f["grant_type"] != "authorization_code"
        || verifier_hash != g.challenge
        || f["redirect_uri"] != g.redirect_uri
    {
        return bad();
    }
    let k = &g.knobs;
    let now = chrono::Utc::now();
    let exp = if k.expired {
        now - chrono::Duration::minutes(5)
    } else {
        now + chrono::Duration::minutes(5)
    };
    let aud = if k.wrong_aud {
        "someone-else"
    } else {
        CLIENT_ID
    };
    let nonce = if k.wrong_nonce {
        "not-the-nonce".to_string()
    } else {
        g.nonce.clone()
    };
    let claims = IdTokenClaims::new(
        IssuerUrl::new(i.url.clone()).unwrap(),
        vec![Audience::new(aud.into())],
        exp,
        now - chrono::Duration::minutes(10),
        StandardClaims::new(SubjectIdentifier::new(k.sub.clone()))
            .set_email(Some(EndUserEmail::new(k.email.clone()))),
        Groups {
            groups: Some(k.groups.clone()),
        },
    )
    .set_nonce(Some(Nonce::new(nonce)));
    let id_token = IdToken::<
        Groups,
        CoreGenderClaim,
        CoreJweContentEncryptionAlgorithm,
        CoreJwsSigningAlgorithm,
    >::new(
        claims,
        &i.key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .unwrap();
    Json(json!({
        "access_token": "at",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token.to_string(),
    }))
    .into_response()
}

async fn start_issuer(public: bool) -> Arc<Issuer> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let issuer = Arc::new(Issuer {
        url: format!("http://{addr}"),
        key: CoreRsaPrivateSigningKey::from_pem(
            test_key_pem(),
            Some(JsonWebKeyId::new("test".into())),
        )
        .unwrap(),
        knobs: Mutex::new(Knobs::user("ann-sub", "ann@example.com", &[])),
        grants: Mutex::new(HashMap::new()),
        public,
        token_requests: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(issuer.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    issuer
}

// ---- the hub ----

const APP_REDIRECT: &str = "http://127.0.0.1:53111/done";

struct Env {
    _dirs: Vec<TempDir>,
    issuer: Arc<Issuer>,
    server: notes_hub::Server,
    hub_url: String,
    http: reqwest::Client,
}

async fn env() -> Env {
    let issuer = start_issuer(false).await;
    let dir = TempDir::new("share");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let hub_url = format!("http://{addr}");
    let config = Config {
        mcp_agent_enabled: false,
        listen: addr.to_string(),
        data_dir: dir.0.clone(),
        public_url: hub_url.clone(),
        trust_proxy: false,
        oidc: OidcConfig {
            issuer: issuer.url.clone(),
            client_id: CLIENT_ID.into(),
            client_secret: CLIENT_SECRET.into(),
            allowed_users: vec!["ann@example.com".into()],
            allowed_groups: vec!["notes-users".into()],
            scopes: None,
        },
        agent: notes_hub::AgentSection {
            enabled: false,
            ..notes_hub::AgentSection::default()
        },
        ..Config::default()
    };
    let server = notes_hub::start_on(listener, config).await.unwrap();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Env {
        _dirs: vec![dir],
        issuer,
        server,
        hub_url,
        http,
    }
}

fn param(u: &Url, k: &str) -> Option<String> {
    u.query_pairs()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.into_owned())
}

/// The value of our session cookie in a `Set-Cookie` header, if any.
fn set_cookie(r: &reqwest::Response) -> Option<String> {
    r.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("scratch_session="))
        .map(str::to_string)
}

impl Env {
    fn sign_in_as(&self, k: Knobs) {
        *self.issuer.knobs.lock().unwrap() = k;
    }

    fn vault(&self) -> &Arc<Vault> {
        self.server.hub().vault()
    }

    async fn location(&self, url: &str) -> Url {
        let r = self.http.get(url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FOUND, "GET {url}");
        let loc = r.headers()[header::LOCATION].to_str().unwrap();
        Url::parse(loc).unwrap()
    }

    /// A browser login of the web viewer: the callback's response.
    async fn web_callback(&self, return_to: &str) -> reqwest::Response {
        let mut start = Url::parse(&format!("{}/auth/web", self.hub_url)).unwrap();
        start.query_pairs_mut().append_pair("return", return_to);
        let authorize = self.location(start.as_str()).await;
        assert!(authorize.as_str().starts_with(&self.issuer.url));
        // The same redirect URI as the app's login: no provider change.
        assert_eq!(
            param(&authorize, "redirect_uri").as_deref(),
            Some(format!("{}/auth/callback", self.hub_url).as_str())
        );
        let callback = self.location(authorize.as_str()).await;
        self.http.get(callback.as_str()).send().await.unwrap()
    }

    /// A signed-in web session: the `Cookie` header value.
    async fn web_session(&self, return_to: &str) -> String {
        let r = self.web_callback(return_to).await;
        assert_eq!(r.status(), StatusCode::FOUND);
        assert_eq!(r.headers()[header::LOCATION].to_str().unwrap(), return_to);
        let c = set_cookie(&r).expect("a session cookie");
        c.split(';').next().unwrap().to_string()
    }

    /// An app login: a device token.
    async fn device_token(&self, device: &str) -> String {
        let mut start = Url::parse(&format!("{}/auth/start", self.hub_url)).unwrap();
        start
            .query_pairs_mut()
            .append_pair("device", device)
            .append_pair("name", "Laptop")
            .append_pair("redirect", APP_REDIRECT);
        let authorize = self.location(start.as_str()).await;
        let callback = self.location(authorize.as_str()).await;
        let back = self.location(callback.as_str()).await;
        let code = param(&back, "code").expect("a code");
        let r = self
            .http
            .post(format!("{}/auth/redeem", self.hub_url))
            .json(&json!({ "code": code, "device": device }))
            .send()
            .await
            .unwrap();
        let v: Value = r.json().await.unwrap();
        v["token"].as_str().unwrap().to_string()
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> (StatusCode, HeaderMap, String) {
        let mut req = self.http.get(format!("{}{path}", self.hub_url));
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let r = req.send().await.unwrap();
        let (status, headers) = (r.status(), r.headers().clone());
        (status, headers, r.text().await.unwrap())
    }

    async fn get_bytes(
        &self,
        path: &str,
        cookie: Option<&str>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = self.http.get(format!("{}{path}", self.hub_url));
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let r = req.send().await.unwrap();
        let (status, headers) = (r.status(), r.headers().clone());
        (status, headers, r.bytes().await.unwrap().to_vec())
    }

    /// `POST /shares` as a device.
    async fn share(&self, token: &str, page: &str, visibility: &str, kind: &str) -> Value {
        let r = self
            .http
            .post(format!("{}/shares", self.hub_url))
            .bearer_auth(token)
            .header("X-Device", DEVICE)
            .json(&json!({ "page": page, "visibility": visibility, "kind": kind }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        r.json().await.unwrap()
    }
}

const DEVICE: &str = "dev-laptop";

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

struct Fixture {
    trip: String,
    other: String,
    image: String,
    other_image: String,
}

fn nb(id: &str, kind: BlockKind, text: &str, checked: bool) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind,
        text: text.into(),
        marks: vec![],
        checked,
    }
}

fn insert(after: Option<&str>, block: NewBlock) -> BlockOp {
    BlockOp::Insert {
        parent: None,
        after: after.map(str::to_string),
        block,
    }
}

/// "Trip": a heading, a list, two todos, a page link to "Other", an image,
/// a property and an inline database "Tasks" with two rows. "Other" has an
/// image of its own.
fn fixture(v: &Vault) -> Fixture {
    let other = v.create_page(None, "Other".into()).unwrap();
    let trip = v.create_page(None, "Trip".into()).unwrap();
    let image = v
        .add_attachment_bytes("dot.png".into(), PNG.to_vec())
        .unwrap();
    let mut png2 = PNG.to_vec();
    png2.extend_from_slice(b"other");
    let other_image = v.add_attachment_bytes("other.png".into(), png2).unwrap();
    v.apply(
        other.clone(),
        vec![
            insert(None, nb("oi", BlockKind::Image, "Other image", false)),
            BlockOp::SetTarget {
                id: "oi".into(),
                target: Some(other_image.clone()),
            },
        ],
    )
    .unwrap();
    let tasks = v.create_page(Some(trip.clone()), "Tasks".into()).unwrap();
    v.make_collection(tasks.clone()).unwrap();
    let schema = v
        .set_schema(
            tasks.clone(),
            vec![PropDef {
                key: String::new(),
                name: "Stage".into(),
                kind: PropKind::Select,
                options: vec![SelectOption {
                    id: String::new(),
                    name: "Booked".into(),
                    color: "green".into(),
                }],
                config: String::new(),
            }],
        )
        .unwrap();
    let stage = &schema[0];
    let row1 = v
        .create_page(Some(tasks.clone()), "Flights".into())
        .unwrap();
    v.create_page(Some(tasks.clone()), "Hotel".into()).unwrap();
    v.set_property(
        row1,
        stage.key.clone(),
        PropValue::Select(stage.options[0].id.clone()),
    )
    .unwrap();
    v.set_property(
        trip.clone(),
        "Where".into(),
        PropValue::Text("Lisbon".into()),
    )
    .unwrap();
    v.apply(
        trip.clone(),
        vec![
            insert(None, nb("h", BlockKind::Heading2, "Plan", false)),
            insert(Some("h"), nb("b1", BlockKind::Bulleted, "Pack bags", false)),
            insert(Some("b1"), nb("t1", BlockKind::Todo, "Book flights", true)),
            insert(Some("t1"), nb("t2", BlockKind::Todo, "Book hotel", false)),
            insert(
                Some("t2"),
                NewBlock {
                    id: "p".into(),
                    kind: BlockKind::Paragraph,
                    text: "Original text, see Other".into(),
                    marks: vec![Mark {
                        start: 19,
                        end: 24,
                        kind: MarkKind::PageLink(other.clone()),
                    }],
                    checked: false,
                },
            ),
            insert(Some("p"), nb("i", BlockKind::Image, "A dot", false)),
            BlockOp::SetTarget {
                id: "i".into(),
                target: Some(image.clone()),
            },
            insert(Some("i"), nb("d", BlockKind::Database, "", false)),
            BlockOp::SetTarget {
                id: "d".into(),
                target: Some(tasks.clone()),
            },
        ],
    )
    .unwrap();
    Fixture {
        trip,
        other,
        image,
        other_image,
    }
}

fn og(html: &str, property: &str) -> Option<String> {
    let key = format!("property=\"{property}\" content=\"");
    let i = html.find(&key)? + key.len();
    Some(html[i..].split('"').next()?.to_string())
}

// ---- tests ----

#[tokio::test]
async fn the_viewer_renders_a_page_for_a_web_session() {
    let e = env().await;
    let f = fixture(e.vault());
    let open = format!("/open/{}", f.trip);
    let cookie = e.web_session(&open).await;
    let (status, headers, html) = e.get(&open, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    for want in [
        "<h1 class=\"title\">Trip</h1>",
        "<h2>Plan</h2>",
        "<li>Pack bags</li>",
        "checked=\"\"",
        "Book hotel",
        "<th>Where</th><td>Lisbon</td>",
        "<th>Stage</th>",
        "Flights",
        "<td>Booked</td>",
        "Hotel",
        &format!("<img src=\"/open/{}/a/{}\" alt=\"A dot\">", f.trip, f.image),
        &format!("<a class=\"pagelink\" href=\"/open/{}\">Other</a>", f.other),
        "<meta name=\"robots\" content=\"noindex\">",
        "id=\"share-open\"",
        "Sign out",
    ] {
        assert!(html.contains(want), "missing {want}\n{html}");
    }
    assert_eq!(og(&html, "og:title").as_deref(), Some("Trip"));
    assert_eq!(og(&html, "og:site_name").as_deref(), Some("Scratch"));
    assert!(og(&html, "og:description")
        .unwrap()
        .starts_with("Plan Pack bags"));
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'nonce-"), "{csp}");
    assert_eq!(headers["cache-control"], "no-store");
    // Raw HTML in a note is text.
    e.vault()
        .apply(
            f.trip.clone(),
            vec![BlockOp::SetText {
                id: "b1".into(),
                text: "<script>alert(1)</script>".into(),
                marks: vec![],
            }],
        )
        .unwrap();
    let (_, _, html) = e.get(&open, Some(&cookie)).await;
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{html}"
    );
    assert_eq!(
        html.matches("<script").count(),
        1,
        "only the share dialog's script"
    );

    // The page's image, only with the session.
    let asset = format!("/open/{}/a/{}", f.trip, f.image);
    let (status, headers, bytes) = e.get_bytes(&asset, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(bytes, PNG);
    assert_eq!(e.get(&asset, None).await.0, StatusCode::UNAUTHORIZED);
    // Not another page's image through this page.
    let other = format!("/open/{}/a/{}", f.trip, f.other_image);
    assert_eq!(e.get(&other, Some(&cookie)).await.0, StatusCode::NOT_FOUND);

    // The app icon for cards.
    let (status, headers, bytes) = e.get_bytes("/icon.png", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert!(bytes.starts_with(b"\x89PNG"));
}

#[tokio::test]
async fn open_without_a_session_bounces_with_a_sign_in_button() {
    let e = env().await;
    let f = fixture(e.vault());
    let (status, _, html) = e.get(&format!("/open/{}", f.trip), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("location.replace(") && html.contains(&format!("notes://page/{}", f.trip)),
        "{html}"
    );
    assert!(
        html.contains(&format!("href=\"/auth/web?return=/open/{}\"", f.trip)),
        "{html}"
    );
    assert!(html.contains("Sign in to view in the browser"));
    assert!(!html.contains("Lisbon") && !html.contains("Plan"));
    // A made-up cookie is no session.
    let (_, _, html) = e
        .get(&format!("/open/{}", f.trip), Some("scratch_session=00ff"))
        .await;
    assert!(html.contains("Sign in to view in the browser"));
}

#[tokio::test]
async fn web_login_sets_a_session_cookie_refuses_others_and_logs_out() {
    let e = env().await;
    let f = fixture(e.vault());
    let open = format!("/open/{}", f.trip);
    // Only viewer and share paths are return addresses.
    for bad in [
        "https://evil.example.com/",
        "//evil.example.com",
        "/auth/web",
    ] {
        let (status, _, _) = e
            .get(&format!("/auth/web?return={}", urlencode(bad)), None)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let r = e.web_callback(&open).await;
    assert_eq!(r.status(), StatusCode::FOUND);
    let c = set_cookie(&r).unwrap();
    for part in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=2592000",
    ] {
        assert!(c.contains(part), "{c}");
    }
    let cookie = c.split(';').next().unwrap().to_string();
    // The store keeps only a hash of the cookie value.
    let secret = cookie.trim_start_matches("scratch_session=");
    let store =
        fs::read_to_string(PathBuf::from(e.vault().root()).join(".vault/hub/web-sessions.json"))
            .unwrap();
    assert!(!store.contains(secret));
    assert!(store.contains(&sha256_hex(secret.as_bytes())));
    assert!(e
        .get(&open, Some(&cookie))
        .await
        .2
        .contains("<h2>Plan</h2>"));

    // A group member may sign in too; someone else may not.
    e.sign_in_as(Knobs::user("gus-sub", "gus@example.com", &["notes-users"]));
    let r = e.web_callback(&open).await;
    assert_eq!(r.status(), StatusCode::FOUND);
    e.sign_in_as(Knobs::user("eve-sub", "eve@example.com", &["other"]));
    let r = e.web_callback(&open).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(set_cookie(&r).is_none());

    // Logout ends the session and clears the cookie.
    let r = e
        .http
        .post(format!("{}/auth/web/logout", e.hub_url))
        .header(header::COOKIE, &cookie)
        .form(&[("return", open.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()[header::LOCATION].to_str().unwrap(), open);
    assert!(set_cookie(&r).unwrap().contains("Max-Age=0"));
    let (_, _, html) = e.get(&open, Some(&cookie)).await;
    assert!(html.contains("Sign in to view in the browser"), "{html}");
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[tokio::test]
async fn shares_public_private_snapshot_live_and_revoke() {
    let e = env().await;
    let f = fixture(e.vault());
    let token = e.device_token(DEVICE).await;
    let pub_snap = e.share(&token, &f.trip, "public", "snapshot").await;
    let pub_live = e.share(&token, &f.trip, "public", "live").await;
    let priv_live = e.share(&token, &f.trip, "private", "live").await;
    let priv_snap = e.share(&token, &f.trip, "private", "snapshot").await;
    let path = |s: &Value| {
        let url = s["url"].as_str().unwrap();
        assert!(url.starts_with(&format!("{}/s/", e.hub_url)), "{url}");
        url.trim_start_matches(&e.hub_url).to_string()
    };
    // At least 128 random bits in the token.
    let tok = path(&pub_snap).trim_start_matches("/s/").to_string();
    assert!(
        tok.len() >= 32 && tok.bytes().all(|b| b.is_ascii_hexdigit()),
        "{tok}"
    );
    assert_eq!(pub_snap["created_by"], format!("device:{DEVICE}"));

    // Public: anyone, with the card and no script.
    for s in [&pub_snap, &pub_live] {
        let (status, headers, html) = e.get(&path(s), None).await;
        assert_eq!(status, StatusCode::OK, "{html}");
        assert!(html.contains("Original text"), "{html}");
        assert!(html.contains("<meta name=\"robots\" content=\"noindex\">"));
        assert_eq!(headers["x-robots-tag"], "noindex");
        assert!(!html.contains("<script"));
        assert_eq!(og(&html, "og:title").as_deref(), Some("Trip"));
        assert_eq!(og(&html, "og:site_name").as_deref(), Some("Scratch"));
        assert!(og(&html, "og:description").unwrap().contains("Pack bags"));
        assert_eq!(
            og(&html, "og:image").unwrap(),
            format!("{}{}/a/{}", e.hub_url, path(s), f.image)
        );
        assert!(html.contains("name=\"twitter:card\""));
        // Page links are text for a signed-out reader.
        assert!(!html.contains(&format!("/open/{}", f.other)), "{html}");
        assert!(
            html.contains("<span class=\"pagelink\">Other</span>"),
            "{html}"
        );
        let (status, _, bytes) = e
            .get_bytes(&format!("{}/a/{}", path(s), f.image), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, PNG);
        // The token reaches only this page's attachments.
        let (status, _, _) = e
            .get(&format!("{}/a/{}", path(s), f.other_image), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // Private: only a title-less card when signed out; the page with a session.
    for s in [&priv_live, &priv_snap] {
        let (status, _, html) = e.get(&path(s), None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.contains("<title>Private note · Scratch</title>"),
            "{html}"
        );
        assert_eq!(og(&html, "og:title").as_deref(), Some("Private note"));
        assert_eq!(og(&html, "og:site_name").as_deref(), Some("Scratch"));
        assert!(og(&html, "og:description").is_none(), "{html}");
        assert!(!html.contains("Trip") && !html.contains("Original") && !html.contains("Lisbon"));
        let (status, _, _) = e.get(&format!("{}/a/{}", path(s), f.image), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let cookie = e.web_session(&path(&priv_live)).await;
    for s in [&priv_live, &priv_snap] {
        let (status, _, html) = e.get(&path(s), Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Original text"), "{html}");
        let (status, _, _) = e
            .get(&format!("{}/a/{}", path(s), f.image), Some(&cookie))
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    // An edit: snapshots keep the old text, live links show the new.
    e.vault()
        .apply(
            f.trip.clone(),
            vec![BlockOp::SetText {
                id: "p".into(),
                text: "Edited text".into(),
                marks: vec![],
            }],
        )
        .unwrap();
    let (_, _, html) = e.get(&path(&pub_snap), None).await;
    assert!(html.contains("Original text") && !html.contains("Edited text"));
    let (_, _, html) = e.get(&path(&pub_live), None).await;
    assert!(html.contains("Edited text") && !html.contains("Original text"));
    let (_, _, html) = e.get(&path(&priv_snap), Some(&cookie)).await;
    assert!(html.contains("Original text"));

    // The page's links, newest first.
    let list = |e: &Env| {
        let req = e
            .http
            .get(format!("{}/shares?page={}", e.hub_url, f.trip))
            .bearer_auth(&token)
            .header("X-Device", DEVICE);
        async move {
            let v: Value = req.send().await.unwrap().json().await.unwrap();
            v["shares"].as_array().unwrap().clone()
        }
    };
    let shares = list(&e).await;
    assert_eq!(shares.len(), 4);
    assert!(shares
        .iter()
        .all(|s| s["url"].as_str().is_some() && s["kind"].as_str().is_some()));

    // Revoked: 410, page and attachments; gone from the list.
    let r = e
        .http
        .delete(format!(
            "{}/shares/{}",
            e.hub_url,
            pub_snap["id"].as_str().unwrap()
        ))
        .bearer_auth(&token)
        .header("X-Device", DEVICE)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(e.get(&path(&pub_snap), None).await.0, StatusCode::GONE);
    assert_eq!(
        e.get(&format!("{}/a/{}", path(&pub_snap), f.image), None)
            .await
            .0,
        StatusCode::GONE
    );
    assert_eq!(list(&e).await.len(), 3);
    // Unknown tokens are 404.
    assert_eq!(
        e.get(&format!("/s/{}", "0".repeat(48)), None).await.0,
        StatusCode::NOT_FOUND
    );

    // A live link of a page in the trash shows nothing.
    e.vault().delete_page(f.trip.clone()).unwrap();
    let (status, _, html) = e.get(&path(&pub_live), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!html.contains("Edited"));
}

#[tokio::test]
async fn only_devices_and_web_sessions_create_shares() {
    let e = env().await;
    let f = fixture(e.vault());
    let token = e.device_token(DEVICE).await;
    let body = json!({ "page": f.trip, "visibility": "public", "kind": "live" });
    let post = |auth: Option<(&str, &str)>, cookie: Option<(&str, Option<&str>)>| {
        let mut req = e.http.post(format!("{}/shares", e.hub_url)).json(&body);
        if let Some((t, d)) = auth {
            req = req.bearer_auth(t).header("X-Device", d);
        }
        if let Some((c, origin)) = cookie {
            req = req.header(header::COOKIE, c);
            if let Some(o) = origin {
                req = req.header(header::ORIGIN, o);
            }
        }
        async move { req.send().await.unwrap().status() }
    };
    assert_eq!(post(None, None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        post(Some(("not-a-token", DEVICE)), None).await,
        StatusCode::UNAUTHORIZED
    );
    // The device token from another device id is refused.
    assert_ne!(
        post(Some((&token, "someone-else")), None).await,
        StatusCode::CREATED
    );

    // An agent token, minted by the device, may not create shares.
    let r = e
        .http
        .post(format!("{}/agent-tokens", e.hub_url))
        .bearer_auth(&token)
        .header("X-Device", DEVICE)
        .json(&json!({ "name": "helper", "scope": "write" }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let agent: Value = r.json().await.unwrap();
    let agent = agent["token"].as_str().unwrap().to_string();
    let status = post(Some((&agent, DEVICE)), None).await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status}"
    );
    let r = e
        .http
        .get(format!("{}/shares?page={}", e.hub_url, f.trip))
        .bearer_auth(&agent)
        .send()
        .await
        .unwrap();
    assert!(!r.status().is_success());

    // A web session: only with this hub's Origin.
    let cookie = e.web_session(&format!("/open/{}", f.trip)).await;
    assert_eq!(
        post(None, Some((&cookie, None))).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(None, Some((&cookie, Some("https://evil.example.com")))).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(None, Some((&cookie, Some(&e.hub_url)))).await,
        StatusCode::CREATED
    );
    assert_eq!(
        post(Some((&token, DEVICE)), None).await,
        StatusCode::CREATED
    );

    // Databases are not shared.
    let db = e.vault().create_page(None, "Board".into()).unwrap();
    e.vault().make_collection(db.clone()).unwrap();
    let r = e
        .http
        .post(format!("{}/shares", e.hub_url))
        .bearer_auth(&token)
        .header("X-Device", DEVICE)
        .json(&json!({ "page": db, "visibility": "public", "kind": "live" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

/// Writes sample pages for the screenshots when `SHARE_SHOTS_DIR` is set:
/// a public share page, its attachment, and the signed-out bounce page.
#[tokio::test]
async fn write_screenshot_pages() {
    let Ok(dir) = std::env::var("SHARE_SHOTS_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let e = env().await;
    let f = fixture(e.vault());
    let token = e.device_token(DEVICE).await;
    let s = e.share(&token, &f.trip, "public", "snapshot").await;
    let path = s["url"]
        .as_str()
        .unwrap()
        .trim_start_matches(&e.hub_url)
        .to_string();
    let (_, _, html) = e.get(&path, None).await;
    fs::create_dir_all(dir.join("a")).unwrap();
    fs::write(dir.join("share.html"), html).unwrap();
    fs::write(dir.join("a").join(&f.image), PNG).unwrap();
    fs::write(dir.join("share-path.txt"), &path).unwrap();
    let (_, _, html) = e.get(&format!("/open/{}", f.trip), None).await;
    fs::write(dir.join("bounce.html"), html).unwrap();
    let cookie = e.web_session(&format!("/open/{}", f.trip)).await;
    let (_, _, html) = e.get(&format!("/open/{}", f.trip), Some(&cookie)).await;
    fs::write(dir.join("viewer.html"), html).unwrap();
    let (_, _, html) = e.get(&format!("/s/{}", "0".repeat(48)), None).await;
    fs::write(dir.join("missing.html"), html).unwrap();
}
