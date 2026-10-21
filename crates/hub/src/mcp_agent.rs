//! OAuth-only MCP gateway for vault tools and the embedded host's advertised tools.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use vault_agent::bubblepaw_core::{ClientInfo, ToolDefinition};

use crate::agent::Shared;
use crate::mcp_log;
use crate::mcp_oauth::AuthError;
use crate::AppState;

const SESSION_HEADER: &str = "mcp-session-id";
const MAX_BODY: usize = 8 << 20;
const MAX_SESSIONS: usize = 256;
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

struct Session {
    owner: String,
    agent_session: String,
    touched: Mutex<Instant>,
    gate: tokio::sync::Mutex<()>,
    closed: AtomicBool,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) struct Gateway {
    pub enabled: bool,
    default_host: String,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

impl Gateway {
    pub fn new(enabled: bool, default_host: String) -> Self {
        Self {
            enabled,
            default_host,
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

    async fn reap(&self, agent: &Shared) {
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

fn unavailable_host(host: &str, why: &str) -> String {
    format!("Host {host:?} is unavailable: {why}. Available hosts: hub")
}

fn selected_host(state: &AppState, args: &Value, query: Option<&str>) -> Result<String, String> {
    let url_host = query.and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "host")
            .map(|(_, value)| value.into_owned())
    });
    let selected = match args.get("host") {
        Some(Value::String(host)) => host.as_str(),
        Some(_) => return Err("host must be a string. Available hosts: hub".into()),
        None => url_host.as_deref().unwrap_or_else(|| {
            if state.mcp_agent.default_host.is_empty() {
                "hub"
            } else {
                &state.mcp_agent.default_host
            }
        }),
    };
    Ok(selected.to_owned())
}

async fn host(state: &AppState, args: &Value, query: Option<&str>) -> Result<Shared, String> {
    let selected = selected_host(state, args, query)?;
    if selected != "hub" {
        return Err(unavailable_host(&selected, "unknown host"));
    }
    state
        .agent
        .get()
        .await
        .map_err(|why| unavailable_host(&selected, &why))
}

async fn host_definitions(
    agent: Shared,
    session: Option<&Arc<Session>>,
) -> Result<Vec<ToolDefinition>, String> {
    let session = session.map(|session| session.agent_session.clone());
    tokio::task::spawn_blocking(move || match session {
        Some(session) => agent.gateway_tool_definitions(&session),
        None => agent.gateway_tool_definitions_for_discovery(),
    })
    .await
    .map_err(|_| "Host discovery failed".to_owned())?
    .map_err(|error| error.to_string())
}

fn list_tools(definitions: Vec<ToolDefinition>) -> Result<Vec<Value>, String> {
    let mut tools = vault_mcp::tools::definitions();
    let has_js = definitions.iter().any(|tool| tool.name == "js");
    for tool in definitions {
        if vault_mcp::tools::exists(&tool.name) || matches!(tool.name.as_str(), "hosts" | "js_help")
        {
            return Err(format!("Tool name collision: {}", tool.name));
        }
        let mut schema = tool.input_schema;
        if schema["type"] != "object" || schema["properties"].get("host").is_some() {
            return Err(format!(
                "Cannot inject host argument into tool {}",
                tool.name
            ));
        }
        if schema.get("properties").is_none() {
            schema["properties"] = json!({});
        }
        schema["properties"]["host"] = json!({"type":"string","description":"Host name; omitted uses the connection's default."});
        tools.push(json!({"name":tool.name,"description":tool.description,"inputSchema":schema}));
    }
    tools.push(json!({"name":"hosts","description":"List reachable execution hosts, their availability and which host this connection selects.",
        "inputSchema":{"type":"object","properties":{"host":{"type":"string"}},"additionalProperties":false}}));
    if has_js {
        tools.push(json!({"name":"js_help","description":"Discover this host's JavaScript capabilities; query js for runtime guidance.",
            "inputSchema":{"type":"object","properties":{"query":{"type":"string"},"host":{"type":"string"}},"additionalProperties":false}}));
    }
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
        return crate::mcp::dispatch_vault(state, input, actor, false).await;
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
                    Json(rpc_error(id, -32000, &unavailable_host("hub", &why))),
                )
                    .into_response()
            }
        };
        state.mcp_agent.reap(&agent).await;
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
            "instructions":format!("{} Call hosts for execution hosts. Host tools accept an optional host argument; vault tools always act on the vault. When js is listed, js_help describes its runtime and capabilities.", vault_tools::help::SERVER_INSTRUCTIONS)
        })))).into_response();
    }
    // Host and builtin tool calls below are logged once, whatever their outcome.
    let started = Instant::now();
    let call = mcp_log::Call::from_request(&msg);
    let reply = match method {
        "ping" => result(id, json!({})),
        "tools/list" => match host(&state, &json!({}), query.as_deref()).await {
            Err(why) => rpc_error(id, -32000, &why),
            Ok(agent) => match host_definitions(agent, session.as_ref())
                .await
                .and_then(list_tools)
            {
                Ok(tools) => result(id, json!({"tools":tools})),
                Err(error) => rpc_error(id, -32000, &error),
            },
        },
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !args.is_object() {
                rpc_error(id, -32602, "arguments must be an object")
            } else if name == "hosts" {
                if args.as_object().unwrap().keys().any(|key| key != "host") {
                    rpc_error(id, -32602, "hosts accepts only an optional host")
                } else {
                    match selected_host(&state, &args, query.as_deref()) {
                        Err(error) => tool_result(id, Err(error)),
                        Ok(selected) if selected != "hub" => {
                            tool_result(id, Err(unavailable_host(&selected, "unknown host")))
                        }
                        Ok(selected) => tool_result(id, Ok(json!([{
                            "name":"hub", "online":state.agent.get().await.is_ok(), "default":selected == "hub"
                        }]).to_string())),
                    }
                }
            } else {
                match host(&state, &args, query.as_deref()).await {
                    Err(error) => tool_result(id, Err(error)),
                    Ok(agent) => {
                        let definitions = host_definitions(agent.clone(), session.as_ref()).await;
                        let tools = match definitions.and_then(list_tools) {
                            Ok(tools) => tools,
                            Err(error) => {
                                if let Some(call) = &call {
                                    call.log(mcp_log::Outcome::Internal, error.len(), started);
                                }
                                return Json(rpc_error(id, -32000, &error)).into_response();
                            }
                        };
                        if !tools.iter().any(|tool| tool["name"] == name) {
                            rpc_error(
                                id,
                                -32602,
                                &format!("unknown or unavailable host tool: {name}"),
                            )
                        } else if name == "js_help" {
                            if args
                                .as_object()
                                .unwrap()
                                .keys()
                                .any(|key| !matches!(key.as_str(), "host" | "query"))
                                || args.get("query").is_some_and(|query| !query.is_string())
                            {
                                rpc_error(
                                    id,
                                    -32602,
                                    "js_help accepts optional string query and host",
                                )
                            } else {
                                tool_result(id, Ok(agent.gateway_js_help(args["query"].as_str())))
                            }
                        } else {
                            let Some(session) = session else {
                                if let Some(call) = &call {
                                    call.log(mcp_log::Outcome::InvalidArgs, 0, started);
                                }
                                return (
                                    StatusCode::BAD_REQUEST,
                                    "Mcp-Session-Id required; initialize first",
                                )
                                    .into_response();
                            };
                            let Ok(_gate) = session.gate.try_lock() else {
                                let busy = tool_result(id, Err("MCP session is busy; wait for the active tool call or close the session".into()));
                                if let Some(call) = &call {
                                    call.log(
                                        mcp_log::Outcome::Tool,
                                        mcp_log::result_bytes(&busy),
                                        started,
                                    );
                                }
                                return Json(busy).into_response();
                            };
                            if session.closed.load(Ordering::SeqCst) {
                                if let Some(call) = &call {
                                    call.log(mcp_log::Outcome::InvalidArgs, 0, started);
                                }
                                return StatusCode::NOT_FOUND.into_response();
                            }
                            let agent_session = session.agent_session.clone();
                            let mut arguments = args;
                            arguments.as_object_mut().unwrap().remove("host");
                            let name = name.to_owned();
                            let client = ClientInfo {
                                id: format!("oauth:{}", user.sub),
                                name: Some(user.email.unwrap_or(user.sub)),
                            };
                            match tokio::task::spawn_blocking(move || {
                                agent.gateway_call_as(&agent_session, &name, arguments, client)
                            })
                            .await
                            {
                                Ok(Ok(outcome)) => result(
                                    id,
                                    json!({
                                        "content":[{"type":"text","text":outcome.content}],
                                        "isError":outcome.is_error,
                                        "_meta":{"notes/pending":outcome.pending}
                                    }),
                                ),
                                Ok(Err(error)) => tool_result(id, Err(error.to_string())),
                                Err(_) => tool_result(id, Err("Host tool execution failed".into())),
                            }
                        }
                    }
                }
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
