//! `notes_hub`: the vault hub for a server. Sync over axum through
//! [`vault_sync::Hub::handle`], and device tokens issued after an OIDC
//! login (Authentik), behind a TLS proxy (Caddy).
//!
//! Routes, config and the login sequence: `wiki/architecture/hub.md`.

pub mod agent;
mod agent_token_api;
pub mod agent_tokens;
pub mod config;
mod hosts;
mod html;
mod jobs;
mod login;
mod mcp;
mod mcp_agent;
mod mcp_log;
mod mcp_oauth;
mod oidc;
mod render;
mod web;

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::TryStreamExt;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::io::{StreamReader, SyncIoBridge};
use vault_core::Vault;
use vault_sync::{Hub, HubRequest};

pub use agent::AgentTestHooks;
pub use config::{AgentSection, Config, McpOAuthConfig, OidcConfig, SchedulesSection};
#[doc(hidden)]
pub use jobs::JobLimits;

/// Shared by every handler.
pub(crate) struct AppState {
    pub hub: Arc<Hub>,
    /// Set at shutdown; ends `/changes` long-polls.
    pub stopping: AtomicBool,
    pub trust_proxy: bool,
    pub import_source_limit_bytes: usize,
    pub login: Option<login::Login>,
    pub agent: agent::AgentHost,
    pub agent_tokens: agent_tokens::AgentTokens,
    pub mcp_oauth: Option<mcp_oauth::McpOAuth>,
    /// The `/.well-known/assetlinks.json` body; None serves no route.
    pub assetlinks: Option<String>,
    pub mcp_agent: mcp_agent::Gateway,
    /// Desktops connected at `/host/ws` as execution hosts.
    pub hosts: hosts::Hosts,
    pub scheduler: Option<Arc<vault_agent::schedules::Scheduler>>,
    schedule_runner: std::sync::Mutex<Option<vault_agent::schedules::RunningScheduler>>,
    /// The jobs API (`jobs` tool) and its worker's state.
    pub jobs: jobs::Jobs,
    /// The web viewer: sessions and shares.
    pub web: web::Web,
}

/// Opens (creating it on first start) the vault replica at
/// `<data_dir>/vault` and its hub.
pub fn open_hub(config: &Config) -> Result<Arc<Hub>, String> {
    let dir = config.vault_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let vault = Vault::open(dir.to_string_lossy().into_owned())
        .map_err(|e| format!("open vault {}: {e}", dir.display()))?;
    Hub::new(Arc::new(vault)).map_err(|e| e.to_string())
}

/// A running `notes-hub`.
pub struct Server {
    addr: SocketAddr,
    state: Arc<AppState>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<io::Result<()>>,
}

impl Server {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.state.hub
    }

    /// Test hook: how long one-time login codes stay valid (2 min).
    #[doc(hidden)]
    pub fn set_code_ttl(&self, ttl: std::time::Duration) {
        if let Some(l) = &self.state.login {
            l.set_code_ttl(ttl);
        }
    }

    /// Open `/agent/ws` sockets (tests).
    #[doc(hidden)]
    pub fn agent_sockets(&self) -> usize {
        self.state.agent.socket_count()
    }

    /// Desktops connected as execution hosts (tests).
    #[doc(hidden)]
    pub fn hosts_online(&self) -> usize {
        self.state.hosts.online()
    }

    /// Test hook: how long a relayed host call may take (10 min).
    #[doc(hidden)]
    pub fn set_host_call_timeout(&self, timeout: std::time::Duration) {
        self.state.hosts.set_call_timeout(timeout);
    }

    /// Stops accepting, ends long-polls and agent sockets, waits for open
    /// requests and shuts the agent down.
    pub async fn stop(mut self) {
        self.state.stopping.store(true, Ordering::SeqCst);
        self.state.jobs.stop();
        let runner = self.state.schedule_runner.lock().unwrap().take();
        let _ = tokio::task::spawn_blocking(move || drop(runner)).await;
        self.state.hosts.shutdown();
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if self.state.mcp_agent.enabled {
            if let Ok(agent) = self.state.agent.get().await {
                self.state.mcp_agent.shutdown(&agent).await;
            }
        }
        self.state.agent.shutdown().await;
        let _ = (&mut self.task).await;
    }

    /// Advances only the scheduler clock, for deterministic integration tests.
    #[doc(hidden)]
    pub fn schedule_tick_at(&self, utc: &str) -> Result<(), String> {
        let scheduler = self
            .state
            .scheduler
            .as_ref()
            .ok_or("schedules are disabled")?;
        scheduler
            .set_test_time(Some(utc))
            .map_err(|e| e.to_string())?;
        scheduler.tick().map_err(|e| e.to_string())
    }

    /// Runs until the server fails or `stop` is called elsewhere.
    pub async fn wait(&mut self) -> io::Result<()> {
        match (&mut self.task).await {
            Ok(r) => r,
            Err(e) => Err(io::Error::other(e)),
        }
    }
}

/// Binds `config.listen` and serves.
pub async fn start(config: Config) -> Result<Server, String> {
    let listener = TcpListener::bind(&config.listen)
        .await
        .map_err(|e| format!("listen {}: {e}", config.listen))?;
    start_on(listener, config).await
}

/// Serves on an already bound listener.
pub async fn start_on(listener: TcpListener, config: Config) -> Result<Server, String> {
    start_with(listener, config, None).await
}

/// [`start_on`] with test hooks for the agent (scripted provider, test
/// tools).
#[doc(hidden)]
pub async fn start_with(
    listener: TcpListener,
    config: Config,
    agent_hooks: Option<AgentTestHooks>,
) -> Result<Server, String> {
    config.validate()?;
    let cfg = config.clone();
    let hub = tokio::task::spawn_blocking(move || open_hub(&cfg))
        .await
        .map_err(|e| e.to_string())??;
    let login = if config.oidc.enabled() {
        if config.oidc.allowed_users.is_empty() && config.oidc.allowed_groups.is_empty() {
            tracing::warn!(
                "oidc.allowed_users and oidc.allowed_groups are both empty: every login will be refused"
            );
        }
        Some(login::Login::new(&config)?)
    } else {
        tracing::warn!("oidc.issuer is not set: login is off (devices need --pair tokens)");
        None
    };
    let (scheduler, schedule_runner, scheduler_error) = if config.agent.enabled {
        let vault = hub.vault().clone();
        let data = config.data_dir.clone();
        let available = config.schedules.runner;
        let opened = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let scheduler =
                vault_agent::schedules::Scheduler::open(vault, &data, "hub", "Hub", available)
                    .map_err(|e| format!("open schedules: {e:#}"))?;
            let runner = vault_agent::schedules::RunningScheduler::start(scheduler.clone())
                .map_err(|e| format!("start schedules: {e:#}"))?;
            Ok((scheduler, runner))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
        match opened {
            Ok((scheduler, runner)) => (Some(scheduler), Some(runner), None),
            Err(error) => {
                let error = format!("schedule runner unavailable: {error}");
                tracing::error!("{error}");
                (None, None, Some(error))
            }
        }
    } else {
        (None, None, None)
    };
    let agent = agent::AgentHost::new(
        &config,
        hub.vault().clone(),
        agent_hooks,
        scheduler.clone(),
        scheduler_error,
    );
    let agent_tokens = agent_tokens::AgentTokens::open_for_server(config.vault_dir());
    let mcp_oauth = if config.mcp_oauth.enabled() {
        if config.oidc.allowed_users.is_empty() && config.oidc.allowed_groups.is_empty() {
            tracing::warn!(
                "oidc.allowed_users and oidc.allowed_groups are both empty: every OAuth MCP request will be refused"
            );
        }
        Some(mcp_oauth::McpOAuth::new(
            config.mcp_oauth.clone(),
            &config.public_url,
            &config.oidc,
        )?)
    } else {
        None
    };
    let hub_vault = hub.vault().clone();
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let base = link_base(&config, addr.port());
    let web = web::Web::open(&config.vault_dir(), base.clone(), login.is_some());
    let state = Arc::new(AppState {
        hub,
        stopping: AtomicBool::new(false),
        trust_proxy: config.trust_proxy,
        import_source_limit_bytes: config.import_source_limit_bytes,
        login,
        agent,
        agent_tokens,
        mcp_oauth,
        assetlinks: config.assetlinks_json(),
        mcp_agent: mcp_agent::Gateway::new(config.mcp_agent_enabled),
        hosts: hosts::Hosts::new(),
        scheduler,
        schedule_runner: std::sync::Mutex::new(schedule_runner),
        jobs: jobs::Jobs::new(hub_vault, config.job_limits.clone(), config.agent.enabled),
        web,
    });
    tokio::spawn(jobs::run_worker(state.clone()));
    if state.agent.enabled() {
        // Open now so a failure shows in the log at start; a failed open is
        // retried by the next `/agent/ws` connection.
        let st = state.clone();
        tokio::spawn(async move {
            let _ = st.agent.get().await;
        });
    }
    vault_tools::set_link_base(Some(base.as_str()));
    let app = router(state.clone());
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = rx.await;
        })
        .await
    });
    tracing::info!("notes-hub listening on {addr}");
    Ok(Server {
        addr,
        state,
        stop: Some(tx),
        task,
    })
}

/// The base of the page links the tools return: `public_url` when it is an
/// http(s) URL, otherwise this hub's loopback address.
fn link_base(config: &Config, port: u16) -> String {
    let public = config.public_url.trim().trim_end_matches('/');
    if !public.is_empty() {
        if let Ok(u) = url::Url::parse(public) {
            if matches!(u.scheme(), "http" | "https") {
                return public.to_string();
            }
        }
        tracing::warn!("public_url is not an http(s) URL; page links use 127.0.0.1");
    }
    format!("http://127.0.0.1:{port}")
}

fn router(state: Arc<AppState>) -> Router {
    let mut app = Router::new()
        .route(
            "/health",
            get(|| async {
                (
                    [
                        (
                            vault_sync::PURGE_CAPABILITY_HEADER,
                            vault_sync::PURGE_CAPABILITY,
                        ),
                        (
                            vault_sync::PURGE_DEVICE_CAPABILITY_HEADER,
                            vault_sync::PURGE_DEVICE_CAPABILITY,
                        ),
                    ],
                    "notes-hub",
                )
            }),
        )
        // `/open/<id>`: the bounce page, or the rendered page for a web
        // session (`web.rs`).
        .route("/open/{id}", get(web::open_page))
        .route("/open/{id}/a/{asset}", get(web::open_asset))
        .route("/s/{token}", get(web::share_page))
        .route("/s/{token}/a/{asset}", get(web::share_asset))
        .route("/shares", get(web::list).post(web::create))
        .route("/shares/{id}", axum::routing::delete(web::revoke))
        .route("/icon.png", get(web::icon))
        .route("/auth/web", get(login::web_start))
        .route("/auth/web/logout", post(web::logout))
        .route("/auth/start", get(login::start))
        .route("/auth/callback", get(login::callback))
        .route("/auth/redeem", post(login::redeem))
        .route("/auth/signout", post(login::signout))
        .route("/agent/ws", get(agent::ws))
        .route("/host/ws", get(hosts::ws))
        .route("/device/name", post(hosts::rename))
        .route(
            "/agent-tokens",
            get(agent_token_api::list).post(agent_token_api::create),
        )
        .route(
            "/agent-tokens/{name}",
            axum::routing::delete(agent_token_api::revoke),
        )
        .route("/mcp", axum::routing::any(mcp::handle))
        .route("/mcp/agent", axum::routing::any(mcp_agent::handle))
        .route(
            "/.well-known/oauth-protected-resource/mcp/agent",
            axum::routing::any(mcp_agent::metadata),
        );
    if let Some(body) = state.assetlinks.clone() {
        app = app.route(
            "/.well-known/assetlinks.json",
            get(move || {
                let body = body.clone();
                async move { ([(header::CONTENT_TYPE, "application/json")], body) }
            }),
        );
    }
    if state.mcp_oauth.is_some() {
        app = app
            .route(
                "/.well-known/oauth-protected-resource",
                get(mcp_oauth::metadata),
            )
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(mcp_oauth::metadata),
            );
    }
    app
        // Everything else is the sync protocol, routed by the hub itself.
        .fallback(sync)
        // `Hub::handle` caps bodies itself (512 MB).
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn_with_state(state.clone(), log_request))
        .with_state(state)
}

/// The client address for logs: the first `X-Forwarded-For` hop when the
/// proxy is trusted, else the peer.
fn client_addr(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if state.trust_proxy {
        if let Some(f) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
        {
            return f.trim().to_string();
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_default()
}

/// One line per request: method, path (never the query, which can carry
/// login codes), status, time.
async fn log_request(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let client = client_addr(&state, req.headers(), peer);
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let start = Instant::now();
    let resp = next.run(req).await;
    tracing::info!(
        client = %client,
        "{method} {path} {} {}ms",
        resp.status().as_u16(),
        start.elapsed().as_millis()
    );
    resp
}

/// The sync routes: [`Hub::handle`] on a blocking thread, reading the
/// body through a sync bridge only once the hub has checked the token.
async fn sync(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let headers = parts
        .headers
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let body_len = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let reader = SyncIoBridge::new(StreamReader::new(
        body.into_data_stream().map_err(io::Error::other),
    ));
    let method = parts.method.as_str().to_string();
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or("").to_string();
    let st = state.clone();
    let reply = tokio::task::spawn_blocking(move || {
        st.hub.handle(HubRequest {
            method,
            path,
            query,
            headers,
            body: Box::new(reader),
            body_len,
            cancel: Some(&st.stopping),
        })
    })
    .await;
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("sync handler failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };
    let mut resp = Response::new(Body::from(reply.body));
    *resp.status_mut() =
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&reply.content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    for (k, v) in reply.headers {
        if let (Ok(k), Ok(v)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(&v),
        ) {
            h.insert(k, v);
        }
    }
    resp
}

/// Sets up logging to stdout (`RUST_LOG` filters; default `info`).
pub fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stdout)
        .try_init();
}

#[cfg(test)]
mod open_tests {
    use super::*;

    #[test]
    fn link_base_is_public_url_or_the_loopback_port() {
        let mut config = Config::default();
        assert_eq!(link_base(&config, 47100), "http://127.0.0.1:47100");
        config.public_url = " https://notes.example.com/ ".into();
        assert_eq!(link_base(&config, 47100), "https://notes.example.com");
        config.public_url = "ftp://notes.example.com".into();
        assert_eq!(link_base(&config, 8000), "http://127.0.0.1:8000");
    }
}
