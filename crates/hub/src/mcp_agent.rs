//! OAuth-only MCP gateway for vault tools and computer tools.
//!
//! Two modes, chosen by the URL. `/mcp/agent?host=<name>` is pinned to one
//! host: the embedded host (`hub`) or a desktop connected at `/host/ws`, with
//! that host's advertised tools. `/mcp/agent` reaches any of the user's
//! computers: a fixed list of the vault tools, `hosts`, the core computer
//! tools and `tool_search` and `call_tool`, each naming its computer in a
//! required `host` argument. Nothing is remembered per MCP session (clients
//! may reconnect or share a session), except the relay session the desktop
//! keeps its processes in.
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use vault_agent::bubblepaw_core::{ClientInfo, ToolDefinition};
use vault_agent::wire::notes_agent_wire::host as relay;

use crate::agent::Shared;
use crate::hosts::{self, HostConn, Hosts, Lookup};
use crate::mcp_log;
use crate::mcp_oauth::AuthError;
use crate::oidc::User;
use crate::AppState;

const SESSION_HEADER: &str = "mcp-session-id";
const MAX_BODY: usize = 8 << 20;
const MAX_SESSIONS: usize = 256;
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Listing a desktop host's tools or closing its session.
const HOST_QUICK_TIMEOUT: Duration = Duration::from_secs(30);
/// MCP-server tools on a computer are named `mcp__<server>__<tool>`.
const MCP_SERVER_PREFIX: &str = "mcp__";

struct Session {
    owner: String,
    agent_session: String,
    /// The opaque session id desktop hosts see for this session.
    relay_id: String,
    /// Desktop hosts that ran a call in this session; closed with it.
    remote_hosts: Mutex<BTreeSet<String>>,
    touched: Mutex<Instant>,
    gate: tokio::sync::Mutex<()>,
    closed: AtomicBool,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) struct Gateway {
    pub enabled: bool,
    /// Any-computer mode: which computer started each `exec` process, by the
    /// bare uuid of its handle, so poll, wait and cancel on another computer
    /// get a clear refusal. Process uuids are unique; bounded, oldest first.
    processes: Mutex<ProcessIndex>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

impl Gateway {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            processes: Mutex::new(ProcessIndex::default()),
            sessions: Mutex::new(HashMap::new()),
            slots: Arc::new(tokio::sync::Semaphore::new(MAX_SESSIONS)),
        }
    }

    fn session(&self, id: &str, owner: &str) -> Option<Arc<Session>> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(id)?;
        if session.owner != owner
            || session.closed.load(Ordering::SeqCst)
            || session.touched.lock().unwrap().elapsed() >= SESSION_TTL
        {
            return None;
        }
        *session.touched.lock().unwrap() = Instant::now();
        Some(session.clone())
    }

    async fn reap(&self, agent: &Shared, hosts: &Hosts) {
        let expired: Vec<_> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .iter()
                .filter_map(|(id, s)| {
                    (s.closed.load(Ordering::SeqCst)
                        || (s.touched.lock().unwrap().elapsed() >= SESSION_TTL
                            && s.gate.try_lock().is_ok()))
                    .then(|| (id.clone(), s.clone()))
                })
                .collect()
        };
        for (id, session) in expired {
            session.closed.store(true, Ordering::SeqCst);
            close_remote(hosts, &session).await;
            if cleanup(agent.clone(), session).await {
                self.sessions.lock().unwrap().remove(&id);
            }
        }
    }

    pub async fn shutdown(&self, agent: &Shared) {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, s)| s)
            .collect();
        for session in sessions {
            session.closed.store(true, Ordering::SeqCst);
            cleanup(agent.clone(), session).await;
        }
    }
}

const MAX_PROCESSES: usize = 4096;

#[derive(Default)]
struct ProcessIndex {
    by_id: HashMap<String, String>,
    order: std::collections::VecDeque<String>,
}

impl ProcessIndex {
    fn insert(&mut self, id: String, computer: String) {
        if self.by_id.insert(id.clone(), computer).is_none() {
            self.order.push_back(id);
            while self.order.len() > MAX_PROCESSES {
                if let Some(old) = self.order.pop_front() {
                    self.by_id.remove(&old);
                }
            }
        }
    }
}

/// A process handle's bare uuid: `process:<uuid>` or the uuid itself.
fn process_id(handle: &str) -> &str {
    handle.strip_prefix("process:").unwrap_or(handle)
}

async fn cleanup(agent: Shared, session: Arc<Session>) -> bool {
    match tokio::task::spawn_blocking(move || agent.gateway_close_session(&session.agent_session))
        .await
    {
        Ok(Ok(())) => true,
        _ => {
            // Runtime errors can contain script output. Keep diagnostics free
            // of user code, note content and credentials.
            tracing::warn!("MCP agent session cleanup failed");
            false
        }
    }
}

/// Ends the session's counterpart on every desktop host it used. Best effort:
/// a desktop also closes its relay sessions when its socket drops.
async fn close_remote(hosts: &Hosts, session: &Session) {
    let names: Vec<String> = std::mem::take(&mut *session.remote_hosts.lock().unwrap())
        .into_iter()
        .collect();
    for name in names {
        if let Lookup::Online(conn) = hosts.lookup(&name) {
            let params = relay::CloseParams {
                session: session.relay_id.clone(),
            };
            let _ = conn
                .request(
                    relay::method::CLOSE,
                    serde_json::to_value(params).unwrap_or_default(),
                    HOST_QUICK_TIMEOUT,
                )
                .await;
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn result(id: Value, value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":value})
}

fn tool_result(id: Value, outcome: Result<String, String>) -> Value {
    let failed = outcome.is_err();
    let text = outcome.unwrap_or_else(|error| error);
    result(
        id,
        json!({"content":[{"type":"text","text":text}],"isError":failed}),
    )
}

fn unavailable_host(state: &AppState, host: &str, why: &str) -> String {
    let mut available = vec!["hub".to_owned()];
    available.extend(
        state
            .hosts
            .list()
            .into_iter()
            .filter(|(_, online)| *online)
            .map(|(name, _)| name),
    );
    format!(
        "Host {host:?} is unavailable: {why}. Available hosts: {}",
        available.join(", ")
    )
}

/// Where a session's host tools run.
enum Target {
    Hub(Shared),
    Remote(Arc<HostConn>),
}

/// The host a `?host=<name>` URL pins the connection to; `None` is
/// any-computer mode.
fn locked_host(query: Option<&str>) -> Option<String> {
    query.and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "host")
            .map(|(_, value)| value.into_owned())
    })
}

async fn host(state: &AppState, selected: &str) -> Result<Target, String> {
    if selected == "hub" {
        return state
            .agent
            .get()
            .await
            .map(Target::Hub)
            .map_err(|why| unavailable_host(state, selected, &why));
    }
    match state.hosts.lookup(selected) {
        Lookup::Online(conn) => Ok(Target::Remote(conn)),
        Lookup::Offline => Err(hosts::offline(selected)),
        Lookup::Unknown => Err(unavailable_host(state, selected, "unknown host")),
    }
}

async fn host_definitions(
    target: &Target,
    session: Option<&Arc<Session>>,
) -> Result<Vec<ToolDefinition>, String> {
    match target {
        Target::Hub(agent) => {
            let agent = agent.clone();
            let session = session.map(|session| session.agent_session.clone());
            tokio::task::spawn_blocking(move || match session {
                Some(session) => agent.gateway_tool_definitions(&session),
                None => agent.gateway_tool_definitions_for_discovery(),
            })
            .await
            .map_err(|_| "Host discovery failed".to_owned())?
            .map_err(|error| error.to_string())
        }
        Target::Remote(conn) => {
            let params = relay::ToolsParams {
                session: session.map(|session| session.relay_id.clone()),
            };
            let reply = conn
                .request(
                    relay::method::TOOLS,
                    serde_json::to_value(params).unwrap_or_default(),
                    HOST_QUICK_TIMEOUT,
                )
                .await?;
            let listed: relay::ToolsResult = serde_json::from_value(reply)
                .map_err(|_| format!("{} sent an invalid tool list", conn.name))?;
            Ok(listed
                .tools
                .into_iter()
                .map(|tool| ToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    input_schema: tool.input_schema,
                    read_only: tool.read_only,
                    approval: Default::default(),
                })
                .collect())
        }
    }
}

/// Locked mode's list: the vault tools and the pinned host's tools as it
/// advertises them.
fn list_tools(definitions: Vec<ToolDefinition>) -> Result<Vec<Value>, String> {
    let mut tools = vault_mcp::tools::definitions();
    let has_js = definitions.iter().any(|tool| tool.name == "js");
    for tool in definitions {
        // The hub agent's own host directory has no place in a pinned connection.
        if tool.name == "hosts" {
            continue;
        }
        if vault_mcp::tools::exists(&tool.name) || tool.name == "js_help" {
            return Err(format!("Tool name collision: {}", tool.name));
        }
        tools.push(json!({"name":tool.name,"description":tool.description,"inputSchema":tool.input_schema}));
    }
    if has_js {
        tools.push(json!({"name":"js_help","description":"Discover this host's JavaScript capabilities; query js for runtime guidance.",
            "inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"additionalProperties":false}}));
    }
    Ok(tools)
}

/// The core computer tools, as a desktop advertises them (static, from the
/// agent crate; an agent test checks they match a desktop's relay list).
fn core_tools() -> Result<&'static [Value], String> {
    static CORE: OnceLock<Result<Vec<Value>, String>> = OnceLock::new();
    CORE.get_or_init(|| {
        vault_agent::tools::core_computer_tool_definitions()
            .map(|definitions| {
                definitions
                    .into_iter()
                    .map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.input_schema}))
                    .collect()
            })
            .map_err(|error| format!("Computer tool definitions unavailable: {error}"))
    })
    .as_ref()
    .map(Vec::as_slice)
    .map_err(Clone::clone)
}

fn host_property() -> Value {
    json!({"type":"string","description":"The computer to run on, from hosts."})
}

fn is_core_tool(name: &str) -> bool {
    vault_agent::tools::CORE_COMPUTER_TOOLS.contains(&name)
}

/// Any-computer mode's list. Fixed per connection: selecting a computer
/// never changes it.
fn any_tools() -> Result<Vec<Value>, String> {
    let mut tools = vault_mcp::tools::definitions();
    tools.push(json!({"name":"hosts","description":"List your computers, online or offline. Computer tools name one in their host argument.",
        "inputSchema":{"type":"object","properties":{},"additionalProperties":false}}));
    for core in core_tools()? {
        let mut tool = core.clone();
        let schema = &mut tool["inputSchema"];
        if schema["type"] != "object" || schema["properties"].get("host").is_some() {
            return Err(format!(
                "Cannot add a host argument to tool {}",
                tool["name"]
            ));
        }
        if schema.get("properties").is_none() {
            schema["properties"] = json!({});
        }
        schema["properties"]["host"] = host_property();
        let mut required = schema["required"].as_array().cloned().unwrap_or_default();
        required.push(json!("host"));
        schema["required"] = Value::Array(required);
        tools.push(tool);
    }
    tools.push(json!({"name":"tool_search","description":"Search a computer's MCP-server tools by name and description. Returns names, descriptions and input schemas for call_tool.",
        "inputSchema":{"type":"object","properties":{
            "query":{"type":"string","description":"Words to match; empty lists every MCP-server tool."},
            "host":host_property()},
            "required":["query","host"],"additionalProperties":false}}));
    tools.push(json!({"name":"call_tool","description":"Call one MCP-server tool found with tool_search on a computer.",
        "inputSchema":{"type":"object","properties":{
            "name":{"type":"string","description":"Tool name from tool_search."},
            "arguments":{"type":"object","description":"The tool's arguments, per its input schema."},
            "host":host_property()},
            "required":["name","host"],"additionalProperties":false}}));
    Ok(tools)
}

pub(crate) async fn metadata(State(state): State<Arc<AppState>>, req: Request) -> Response {
    if !state.mcp_agent.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    match &state.mcp_oauth {
        Some(oauth) => Json(oauth.agent_metadata(req.uri().query())).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response {
    if !state.mcp_agent.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !crate::mcp::origins_allowed(req.headers(), state.mcp_oauth.is_some()) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    if req.method() != Method::POST && req.method() != Method::DELETE {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "POST, DELETE")],
            "use POST or DELETE",
        )
            .into_response();
    }
    let Some(oauth) = &state.mcp_oauth else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let query = req.uri().query().map(str::to_owned);
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::trim);
    let Some(token) = token else {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                oauth.agent_challenge(false, query.as_deref()),
            )],
        )
            .into_response();
    };
    let user = match oauth.authenticate(token).await {
        Ok(user) => user,
        Err(AuthError::InvalidToken) => {
            return (
                StatusCode::UNAUTHORIZED,
                [(
                    header::WWW_AUTHENTICATE,
                    oauth.agent_challenge(true, query.as_deref()),
                )],
            )
                .into_response()
        }
        Err(AuthError::Forbidden) => return (StatusCode::FORBIDDEN, "forbidden").into_response(),
        Err(AuthError::Unavailable) => {
            return (StatusCode::SERVICE_UNAVAILABLE, "OAuth unavailable").into_response()
        }
    };
    let session_id = match req.headers().get(SESSION_HEADER) {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value.to_owned()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        None => None,
    };
    let session = match &session_id {
        Some(id) => match state.mcp_agent.session(id, &user.sub) {
            Some(session) => Some(session),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "MCP session not found; initialize again",
                )
                    .into_response()
            }
        },
        None => None,
    };
    if req.method() == Method::DELETE {
        let Some(session) = session else {
            return (StatusCode::BAD_REQUEST, "Mcp-Session-Id required").into_response();
        };
        session.closed.store(true, Ordering::SeqCst);
        close_remote(&state.hosts, &session).await;
        if let Ok(agent) = state.agent.get().await {
            if cleanup(agent, session).await {
                state
                    .mcp_agent
                    .sessions
                    .lock()
                    .unwrap()
                    .remove(session_id.as_ref().unwrap());
                return StatusCode::NO_CONTENT.into_response();
            }
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Session closed; runtime cleanup will be retried on initialize",
        )
            .into_response();
    }
    let input = match crate::mcp::read_body(req.into_body(), state.import_source_limit_bytes).await
    {
        Ok(body) => body,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response(),
    };
    let msg: Value = match serde_json::from_slice(&input.bytes) {
        Ok(msg) => msg,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(Value::Null, -32700, "parse error")),
            )
                .into_response()
        }
    };
    if !msg.is_object() || msg["jsonrpc"] != "2.0" || !msg["method"].is_string() {
        return Json(rpc_error(msg["id"].clone(), -32600, "invalid request")).into_response();
    }
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = &msg["params"];
    let method = msg["method"].as_str().unwrap();
    // The vault registry owns these names, aliases and hidden tools. Use the
    // same HTTP dispatcher as /mcp, including imports, limits and actor identity.
    if method == "tools/call"
        && params["name"]
            .as_str()
            .is_some_and(vault_mcp::tools::exists)
    {
        let actor = format!("oauth:{}", user.email.as_deref().unwrap_or(&user.sub));
        // Imports can be much larger than ordinary calls. Release this parsed
        // copy before the shared dispatcher parses the original body.
        drop(msg);
        // OAuth callers get the Everything tool set.
        let set = vault_tools::ToolSet::Everything;
        return crate::mcp::dispatch_vault(state, input, actor, set).await;
    }
    if input.total > MAX_BODY {
        return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response();
    }
    if method == "initialize" {
        let agent = match state.agent.get().await {
            Ok(agent) => agent,
            Err(why) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(rpc_error(
                        id,
                        -32000,
                        &unavailable_host(&state, "hub", &why),
                    )),
                )
                    .into_response()
            }
        };
        state.mcp_agent.reap(&agent, &state.hosts).await;
        let slot = match state.mcp_agent.slots.clone().try_acquire_owned() {
            Ok(slot) => slot,
            Err(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "MCP session capacity reached",
                )
                    .into_response()
            }
        };
        let opened =
            tokio::task::spawn_blocking(move || agent.create_session("MCP agent gateway")).await;
        let agent_session = match opened {
            Ok(Ok(session)) => session.id,
            _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        let sid = vault_core::new_id().to_string();
        state.mcp_agent.sessions.lock().unwrap().insert(
            sid.clone(),
            Arc::new(Session {
                owner: user.sub,
                agent_session,
                relay_id: vault_core::new_id().to_string(),
                remote_hosts: Mutex::new(BTreeSet::new()),
                touched: Mutex::new(Instant::now()),
                gate: tokio::sync::Mutex::new(()),
                closed: AtomicBool::new(false),
                _slot: slot,
            }),
        );
        let requested = params["protocolVersion"].as_str().unwrap_or("");
        let version = vault_mcp::PROTOCOL_VERSIONS
            .iter()
            .copied()
            .find(|v| *v == requested)
            .unwrap_or(vault_mcp::PROTOCOL_VERSIONS[0]);
        return ([(SESSION_HEADER, sid)], Json(result(id, json!({
            "protocolVersion":version,"capabilities":{"tools":{}},
            "serverInfo":{"name":"notes-agent","version":env!("CARGO_PKG_VERSION")},
            "instructions":format!("{} {}", vault_tools::help::SERVER_INSTRUCTIONS, match locked_host(query.as_deref()) {
                Some(host) => format!("Host tools run on {host}; vault tools always act on the vault. When js is listed, js_help describes its runtime and capabilities."),
                None => "Vault tools always act on the vault. Call hosts for your computers. read, write, edit, search, list, exec, poll, wait and cancel run on the computer named in their host argument; poll, wait and cancel a process on the computer that started it. A computer's MCP-server tools are found with tool_search and run with call_tool.".to_owned(),
            })
        })))).into_response();
    }
    // Host and builtin tool calls below are logged once, whatever their outcome.
    let started = Instant::now();
    let call = mcp_log::Call::from_request(&msg);
    let log = Log {
        call: call.as_ref(),
        started,
    };
    let locked = locked_host(query.as_deref());
    let reply = match method {
        "ping" => result(id, json!({})),
        "tools/list" => match &locked {
            None => match any_tools() {
                Ok(tools) => result(id, json!({"tools":tools})),
                Err(error) => rpc_error(id, -32000, &error),
            },
            Some(selected) => match host(&state, selected).await {
                Err(why) => rpc_error(id, -32000, &why),
                Ok(target) => match host_definitions(&target, session.as_ref())
                    .await
                    .and_then(list_tools)
                {
                    Ok(tools) => result(id, json!({"tools":tools})),
                    Err(error) => rpc_error(id, -32000, &error),
                },
            },
        },
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("").to_owned();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let ctx = CallCtx {
                state: &state,
                session: session.as_ref(),
                user: &user,
                log,
            };
            let outcome = if !args.is_object() {
                Ok(rpc_error(id, -32602, "arguments must be an object"))
            } else {
                match &locked {
                    Some(selected) => locked_call(&ctx, selected, id, &name, args).await,
                    None => any_call(&ctx, id, &name, args).await,
                }
            };
            match outcome {
                Ok(reply) => reply,
                Err(early) => return early,
            }
        }
        _ => rpc_error(id, -32601, "method not found"),
    };
    if let Some(call) = &call {
        call.log(
            mcp_log::outcome_of(&reply),
            mcp_log::result_bytes(&reply),
            started,
        );
    }
    Json(reply).into_response()
}

#[derive(Clone, Copy)]
struct Log<'a> {
    call: Option<&'a mcp_log::Call>,
    started: Instant,
}

impl Log<'_> {
    /// Logs a reply that leaves the handler early (not as a JSON-RPC reply).
    fn early(&self, outcome: mcp_log::Outcome, bytes: usize) {
        if let Some(call) = self.call {
            call.log(outcome, bytes, self.started);
        }
    }
}

struct CallCtx<'a> {
    state: &'a AppState,
    session: Option<&'a Arc<Session>>,
    user: &'a User,
    log: Log<'a>,
}

fn unknown_tool(id: Value, name: &str) -> Value {
    rpc_error(
        id,
        -32602,
        &format!("unknown or unavailable host tool: {name}"),
    )
}

/// The session a host call needs; without one the request is refused.
fn need_session<'a>(ctx: &CallCtx<'a>) -> Result<&'a Arc<Session>, Response> {
    ctx.session.ok_or_else(|| {
        ctx.log.early(mcp_log::Outcome::InvalidArgs, 0);
        (
            StatusCode::BAD_REQUEST,
            "Mcp-Session-Id required; initialize first",
        )
            .into_response()
    })
}

/// Runs one tool on a host in the session, one call at a time per session.
/// `Err` is an early response; `Ok(Err)` a tool error for the assistant.
async fn run_on(
    ctx: &CallCtx<'_>,
    session: &Arc<Session>,
    target: Target,
    name: String,
    arguments: Value,
) -> Result<Result<relay::CallResult, String>, Response> {
    let Ok(_gate) = session.gate.try_lock() else {
        return Ok(Err(
            "MCP session is busy; wait for the active tool call or close the session".into(),
        ));
    };
    if session.closed.load(Ordering::SeqCst) {
        ctx.log.early(mcp_log::Outcome::InvalidArgs, 0);
        return Err(StatusCode::NOT_FOUND.into_response());
    }
    let client = ClientInfo {
        id: format!("oauth:{}", ctx.user.sub),
        name: Some(
            ctx.user
                .email
                .clone()
                .unwrap_or_else(|| ctx.user.sub.clone()),
        ),
    };
    Ok(match target {
        Target::Hub(agent) => {
            let agent_session = session.agent_session.clone();
            match tokio::task::spawn_blocking(move || {
                agent.gateway_call_as(&agent_session, &name, arguments, client)
            })
            .await
            {
                Ok(Ok(outcome)) => Ok(relay::CallResult {
                    content: outcome.content,
                    is_error: outcome.is_error,
                    pending: outcome.pending,
                }),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err("Host tool execution failed".into()),
            }
        }
        Target::Remote(conn) => {
            session
                .remote_hosts
                .lock()
                .unwrap()
                .insert(conn.name.clone());
            call_remote(ctx.state, &conn, session, name, arguments, client).await
        }
    })
}

fn call_reply(id: Value, outcome: Result<relay::CallResult, String>) -> Value {
    match outcome {
        Ok(outcome) => result(
            id,
            json!({
                "content":[{"type":"text","text":outcome.content}],
                "isError":outcome.is_error,
                "_meta":{"notes/pending":outcome.pending}
            }),
        ),
        Err(error) => tool_result(id, Err(error)),
    }
}

/// `/mcp/agent?host=<name>`: every host tool runs on that host.
async fn locked_call(
    ctx: &CallCtx<'_>,
    selected: &str,
    id: Value,
    name: &str,
    args: Value,
) -> Result<Value, Response> {
    let target = match host(ctx.state, selected).await {
        Ok(target) => target,
        Err(error) => return Ok(tool_result(id, Err(error))),
    };
    let tools = match host_definitions(&target, ctx.session)
        .await
        .and_then(list_tools)
    {
        Ok(tools) => tools,
        Err(error) => {
            ctx.log.early(mcp_log::Outcome::Internal, error.len());
            return Err(Json(rpc_error(id, -32000, &error)).into_response());
        }
    };
    if !tools.iter().any(|tool| tool["name"] == name) {
        return Ok(unknown_tool(id, name));
    }
    if name == "js_help" {
        if args.as_object().unwrap().keys().any(|key| key != "query")
            || args.get("query").is_some_and(|query| !query.is_string())
        {
            return Ok(rpc_error(
                id,
                -32602,
                "js_help accepts an optional string query",
            ));
        }
        return Ok(match &target {
            Target::Hub(agent) => {
                tool_result(id, Ok(agent.gateway_js_help(args["query"].as_str())))
            }
            Target::Remote(conn) => {
                tool_result(id, Err(format!("js_help is not offered by {}", conn.name)))
            }
        });
    }
    let session = need_session(ctx)?;
    let outcome = run_on(ctx, session, target, name.to_owned(), args).await?;
    Ok(call_reply(id, outcome))
}

/// The computer named by a call's `host` argument, online.
fn pick_computer(state: &AppState, host: Option<&str>) -> Result<(String, Arc<HostConn>), String> {
    let Some(name) = host.filter(|name| !name.is_empty()) else {
        return Err("Unknown computer (no host given). Call hosts.".into());
    };
    match state.hosts.lookup(name) {
        Lookup::Online(conn) => Ok((conn.name.clone(), conn)),
        Lookup::Offline => Err(hosts::offline(name)),
        Lookup::Unknown => Err(format!("Unknown computer {name}. Call hosts.")),
    }
}

/// A computer's tool result, tagged with the computer it ran on: a first line
/// for the assistant and `_meta["notes/host"]`.
fn tagged(id: Value, computer: &str, outcome: Result<relay::CallResult, String>) -> Value {
    let (text, is_error, pending) = match outcome {
        Ok(outcome) => (outcome.content, outcome.is_error, outcome.pending),
        Err(error) => (error, true, false),
    };
    result(
        id,
        json!({
            "content":[{"type":"text","text":format!("[on {computer}]\n{text}")}],
            "isError":is_error,
            "_meta":{"notes/pending":pending,"notes/host":computer}
        }),
    )
}

fn text_outcome(outcome: Result<String, String>) -> Result<relay::CallResult, String> {
    outcome.map(|content| relay::CallResult {
        content,
        is_error: false,
        pending: false,
    })
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ()> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(()),
    }
}

fn only_keys(args: &Value, allowed: &[&str]) -> bool {
    args.as_object()
        .unwrap()
        .keys()
        .all(|key| allowed.contains(&key.as_str()))
}

/// `/mcp/agent`: the fixed tool set; every computer tool names its computer.
async fn any_call(
    ctx: &CallCtx<'_>,
    id: Value,
    name: &str,
    args: Value,
) -> Result<Value, Response> {
    match name {
        "hosts" => {
            if !only_keys(&args, &[]) {
                return Ok(rpc_error(id, -32602, "hosts takes no arguments"));
            }
            let list: Vec<Value> = ctx
                .state
                .hosts
                .list()
                .into_iter()
                .map(|(name, online)| json!({"name":name,"online":online}))
                .collect();
            Ok(tool_result(id, Ok(Value::Array(list).to_string())))
        }
        "tool_search" => {
            let (Ok(Some(query)), Ok(host), true) = (
                string_arg(&args, "query"),
                string_arg(&args, "host"),
                only_keys(&args, &["query", "host"]),
            ) else {
                return Ok(rpc_error(
                    id,
                    -32602,
                    "tool_search takes a string query and a string host",
                ));
            };
            let (computer, conn) = match pick_computer(ctx.state, host) {
                Ok(picked) => picked,
                Err(error) => return Ok(tool_result(id, Err(error))),
            };
            let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
            let listed = host_definitions(&Target::Remote(conn), ctx.session).await;
            let outcome = listed.map(|tools| {
                let found: Vec<Value> = tools
                    .into_iter()
                    .filter(|tool| tool.name.starts_with(MCP_SERVER_PREFIX))
                    .filter(|tool| {
                        let haystack = format!("{} {}", tool.name, tool.description).to_lowercase();
                        terms.iter().all(|term| haystack.contains(term.as_str()))
                    })
                    .map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.input_schema}))
                    .collect();
                Value::Array(found).to_string()
            });
            Ok(tagged(id, &computer, text_outcome(outcome)))
        }
        "call_tool" => {
            let (Ok(Some(tool)), Ok(host), true) = (
                string_arg(&args, "name"),
                string_arg(&args, "host"),
                only_keys(&args, &["name", "arguments", "host"]),
            ) else {
                return Ok(rpc_error(
                    id,
                    -32602,
                    "call_tool takes a string name, an arguments object and a string host",
                ));
            };
            let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return Ok(rpc_error(
                    id,
                    -32602,
                    "call_tool arguments must be an object",
                ));
            }
            if !tool.starts_with(MCP_SERVER_PREFIX) {
                return Ok(tool_result(id, Err(format!("call_tool runs a computer's MCP-server tools (named {MCP_SERVER_PREFIX}...); call {tool} directly with its host argument"))));
            }
            let (computer, conn) = match pick_computer(ctx.state, host) {
                Ok(picked) => picked,
                Err(error) => return Ok(tool_result(id, Err(error))),
            };
            let session = need_session(ctx)?;
            match host_definitions(&Target::Remote(conn.clone()), Some(session)).await {
                Err(error) => return Ok(tagged(id, &computer, Err(error))),
                Ok(tools) if !tools.iter().any(|listed| listed.name == tool) => {
                    return Ok(tagged(
                        id,
                        &computer,
                        Err(format!("{computer} has no tool {tool}; use tool_search")),
                    ));
                }
                Ok(_) => {}
            }
            let outcome = run_on(
                ctx,
                session,
                Target::Remote(conn),
                tool.to_owned(),
                arguments,
            )
            .await?;
            Ok(tagged(id, &computer, outcome))
        }
        name if is_core_tool(name) => {
            let Ok(host) = string_arg(&args, "host") else {
                return Ok(rpc_error(
                    id,
                    -32602,
                    "host must be a string; call hosts for the list",
                ));
            };
            let (computer, conn) = match pick_computer(ctx.state, host) {
                Ok(picked) => picked,
                Err(error) => return Ok(tool_result(id, Err(error))),
            };
            let mut arguments = args;
            arguments.as_object_mut().unwrap().remove("host");
            // A process runs on the computer that started it.
            if let Some(handle) = arguments.get("handle").and_then(Value::as_str) {
                let started_on = ctx
                    .state
                    .mcp_agent
                    .processes
                    .lock()
                    .unwrap()
                    .by_id
                    .get(process_id(handle))
                    .cloned();
                if let Some(owner) = started_on.filter(|owner| *owner != computer) {
                    return Ok(tool_result(id, Err(format!(
                        "Process {handle} was started on {owner}, not {computer}. Call {name} with host \"{owner}\"."
                    ))));
                }
            }
            let session = need_session(ctx)?;
            let outcome = run_on(
                ctx,
                session,
                Target::Remote(conn),
                name.to_owned(),
                arguments,
            )
            .await?;
            if name == "exec" {
                if let Ok(result) = &outcome {
                    let handle = serde_json::from_str::<Value>(&result.content)
                        .ok()
                        .and_then(|value| value["handle"].as_str().map(str::to_owned));
                    if let Some(handle) = handle {
                        ctx.state
                            .mcp_agent
                            .processes
                            .lock()
                            .unwrap()
                            .insert(process_id(&handle).to_owned(), computer.clone());
                    }
                }
            }
            Ok(tagged(id, &computer, outcome))
        }
        _ => Ok(unknown_tool(id, name)),
    }
}

/// Runs one host tool on a desktop, within the hub's call timeout.
async fn call_remote(
    state: &AppState,
    conn: &HostConn,
    session: &Session,
    name: String,
    arguments: Value,
    client: ClientInfo,
) -> Result<relay::CallResult, String> {
    let params = relay::CallParams {
        session: session.relay_id.clone(),
        name,
        arguments,
        caller: relay::Caller {
            id: client.id,
            name: client.name,
        },
    };
    let reply = conn
        .request(
            relay::method::CALL,
            serde_json::to_value(params).unwrap_or_default(),
            state.hosts.call_timeout(),
        )
        .await?;
    serde_json::from_value(reply).map_err(|_| format!("{} sent an invalid result", conn.name))
}
