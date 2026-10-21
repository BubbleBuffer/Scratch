---
type: topic
layer: state
summary: "notes-hub: sync and OIDC device login, BubblePaw at /agent/ws, vault MCP at /mcp, and an opt-in OAuth agent gateway at /mcp/agent; config, routes and security."
parent: "[[architecture/index]]"
status: current
covers: ["crates/hub/**", "crates/agent_wire/**"]
realizes: ["[[integrations/agent]]"]
---

# Hub server (state)

Build step 4a of the [agent plan](../integrations/agent.md), built 2026-10-08 in [crates/hub](../../crates/hub) (package `notes_hub`, binary `notes-hub`). It runs the same hub as the desktop ([Sync (state)](sync-state.md)) on a server: axum on tokio, plain HTTP inside the container, with Caddy in front terminating TLS. Devices get their token from an OIDC login instead of `--pair`. Since step 4b it also runs the BubblePaw agent ([Agent](../integrations/agent.md)) over its vault replica and serves it at `GET /agent/ws`.

## Running it

For the local amd64 image and the lead's VPS procedure, see [Hub deployment](hub-deploy.md).

- `notes-hub [--config <file>]` serves. The vault replica is `<data_dir>/vault`, created on first start.
- `--devices`, `--unpair <label>`: as `vault-hub`.
- `--pair <label>`: an emergency token that skips the login. It prints a warning to stderr.
- `--agent-token add <name> --scope read|write`, `--agent-token list`, `--agent-token revoke <name>` manage agent access. Each accepts the same `--config <file>` as the server and works while it runs; these commands open the token store without opening the vault.
- Logs go to stdout through `tracing` (`RUST_LOG`, default `info`). One line per request: client address, method, path, status, time. Query strings are never logged, nor are tokens or codes.
- SIGTERM or Ctrl+C stops it gracefully; open `/changes` long-polls end at once.

## Config

A TOML file. Each key can be overridden by `NOTES_HUB_<KEY>` (`NOTES_HUB_OIDC_<KEY>` for `[oidc]`, `NOTES_HUB_AGENT_<KEY>` for `[agent]`). Lists in variables are separated by commas or spaces. Unknown keys are an error.

| Key | Default | Meaning |
|---|---|---|
| `listen` | `0.0.0.0:8080` | Plain-HTTP listen address. |
| `data_dir` | (required) | Holds `vault/` and `agent/` (`agent.data/` for BubblePaw chats, `agent.sqlite` for Notes activity/edit records, `workspace/`). |
| `public_url` | (required with login) | Where browsers reach the hub, e.g. `https://notes.example.com`. The redirect URI is `<public_url>/auth/callback`. Also the base of the page links the tools return (`<public_url>/open/<id>`); unset, they use `http://127.0.0.1:<bound port>`. |
| `android_package`, `android_sha256_certs` | empty | Android App Links. The package (`de.bubblebuffer.notes`) and the SHA-256 fingerprints (`AA:BB:...`, 32 bytes, either case; several for key rotation) of its signing certificates. Set both or neither; with both, `GET /.well-known/assetlinks.json` serves the Digital Asset Links statement for `delegate_permission/common.handle_all_urls`, and with neither that route does not exist. Environment: `NOTES_HUB_ANDROID_PACKAGE`, `NOTES_HUB_ANDROID_SHA256_CERTS` (comma or space separated). The app's manifest verifies `https://notes.breadcat.cc/open/*`. |
| `trust_proxy` | `true` | Use the first `X-Forwarded-For` hop as the client address in logs. Set it to `false` if the hub is reachable without Caddy. It affects nothing else: the public URL is configured, not derived from headers. |
| `import_source_limit_bytes` | `67108864` | Per-source tar archive bytes before base64; override with `NOTES_HUB_IMPORT_SOURCE_LIMIT_BYTES`. |
| `oidc.issuer` | empty | Issuer URL (Authentik: `https://<authentik>/application/o/<slug>/`). Empty turns login off, and only `--pair` tokens work. |
| `oidc.client_id`, `oidc.client_secret` | | The confidential client. |
| `oidc.allowed_users` | `[]` | Emails (any case) or `sub`s allowed to sign in. |
| `oidc.allowed_groups` | `[]` | Groups (the ID token's `groups` claim) whose members may sign in. |
| `oidc.scopes` | `openid email profile`, plus `groups` if `allowed_groups` is set | Scopes asked for. |
| `agent.enabled` | `true` | Run the agent. Off: `/agent/ws` answers 503. |
| `agent.default_model` | empty (the agent's default) | `provider/model` for runs that name none. |
| `agent.execution_mode`, `agent.enable_js` | `native`, `false` | Model-facing execution mode (`native` or `js`) and optional JS composition in native mode. Environment: `NOTES_HUB_AGENT_EXECUTION_MODE`, `NOTES_HUB_AGENT_ENABLE_JS`. |
| `agent.bun`, `agent.claude_code_extension` | empty | The `bun` executable and the Claude Code extension folder (holding `src/main.ts`), for the `claude-code` provider. |
| `agent.approvals` | `ask` | `ask` or `allow` for tools that need confirmation. Another value is an error. |
| `mcp_oauth.issuer`, `mcp_oauth.client_id` | empty | Optional MCP OAuth issuer and public client ID; required together, with `public_url`. Environment: `NOTES_HUB_MCP_OAUTH_ISSUER`, `NOTES_HUB_MCP_OAUTH_CLIENT_ID`. |
| `mcp_agent_enabled` | `false` | Enable `/mcp/agent`; requires both MCP OAuth settings. Environment: `NOTES_HUB_MCP_AGENT_ENABLED`. |
| `mcp_agent_default_host` | `hub` | Global fallback host for gateway tool calls. Only `hub` is implemented. Environment: `NOTES_HUB_MCP_AGENT_DEFAULT_HOST`. |

With both allow lists empty every login is refused, and the hub logs a warning at start.

Example (secrets better come from `NOTES_HUB_OIDC_CLIENT_SECRET`):

```toml
data_dir = "/data"
public_url = "https://notes.example.com"

[oidc]
issuer = "https://auth.example.com/application/o/notes/"
client_id = "notes-hub"
allowed_groups = ["notes"]
```

## Routes

| Route | Purpose |
|---|---|
| `GET /health` | `notes-hub` |
| `GET /.well-known/assetlinks.json` | Public. Only with `android_package` and `android_sha256_certs` set (config above): the Digital Asset Links statement, `application/json`. Otherwise no such route. |
| `GET /open/<page id>[?block=<block id>]` | Public (no auth). A 200 HTML page that sends the browser to `notes://page/<page id>[?block=<block id>]`, with an "Open in Scratch" link and a meta refresh fallback. Ids must have the vault's id shape (26 base32 characters, first at most `7`), or the route answers 400. This is the link form agents get from the tools ([links](../../docs/notes/links.md)). The desktop's embedded hub serves the same route from `vault_sync` ([crates/sync/src/open.rs](../../crates/sync/src/open.rs)), and sets the tools' link base to its loopback port when it starts. |
| `/sync`, `/push`, `/changes`, `/blobs/<id>` | The sync protocol, unchanged ([Sync (state)](sync-state.md#protocol)). Paths outside the explicit hub routes fall through to `Hub::handle` on a blocking thread, with the body streamed through a sync reader only after the token check. |
| `GET /auth/start?device=<id>&name=<label>&redirect=<uri>[&state=<s>]` | Starts a login (below). |
| `GET /auth/callback?code&state` | The issuer's redirect back. |
| `POST /auth/redeem` `{code, device}` | Trades a one-time code for `{token, label, account}`. `account` is the signed-in user's email from the verified ID token, or its `sub` when there is none (added 2026-10-11; the deployed hub does not send it yet, so the app shows its fallback). A bad, used, expired or other-device code gets 400 `{"error": "invalid_code"}`. |
| `POST /auth/signout` | With `Authorization: Bearer <token>`: drops that token (204), or 401 if unknown. It also closes that token's agent sockets. |
| `GET /agent/ws` | The agent over a WebSocket (below). |
| `POST /mcp` | The vault MCP tools over Streamable HTTP (17 listed; read scope 7), authenticated by agent tokens, or by Authentik OAuth tokens when the Claude connector is on (below). `GET` and other methods return 405. |
| `GET /.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp` | Protected-resource metadata (RFC 9728) for `/mcp`. Served only when the Claude connector is on. |
| `POST /mcp/agent`, `DELETE /mcp/agent` | Opt-in OAuth-only agent gateway: vault tools, advertised host tools and `hosts`; DELETE closes a gateway session. Other methods return 405. Disabled: 404. |
| `GET /.well-known/oauth-protected-resource/mcp/agent` | Metadata advertising the separate `/mcp/agent` resource and the existing MCP OAuth issuer; served only when the gateway is enabled. |

## Agent tokens and MCP (`/mcp`)

The repo import is the hidden tool `import_markdown` (write scope, MCP only: never in `tools/list`, not offered to the in-app agent, and sent by the `notes wiki` CLI). It accepts `name`, `roots` `{name, archive_base64, exclude?}` containing plain tar archives, `repo_path`, `git_remote`, `last_activity`, `exclude` and `dry_run`. `create_page` has no import arguments any more; passing `roots` to it returns an error that names `import_markdown`. It finds or creates the Projects database by its `ai.projects` checkbox marker, upserts a row by name and imports under that row (multiple roots have named child containers). Rows hold repo path, git remote, status and last activity; the name is their title. Existing `cut over` status is preserved. Repeated imports reuse page ids and write nothing for unchanged content. Replies contain the row id, row-created/updated flags and an `ImportReport` for each root. `dry_run` diagnoses against a fresh empty temporary vault and labels counts accordingly; it never accesses the real vault's files or export state.

Each registry tool lists the surfaces it is served on (`vault_tools::Surface`: `Mcp`, `InAppAgent`). `feedback_submit` is served on both surfaces; the in-app agent, which the hub's agent host also runs, files its reports with transport `in-app agent`. `feedback_list` (hidden) and `import_markdown` (hidden) are `Mcp` only, and the in-app agent is refused both.

Archive bytes before base64 are capped per source by `import_source_limit_bytes`; an oversized-source error names the source and limit. Only `import_markdown` calls receive the larger archive allowance. Ordinary calls and non-archive JSON keep the 8 MiB limit. Archive paths must be relative; links and special entries are refused.

Agent tokens are separate from device tokens. `<data_dir>/vault/.vault/hub/agent-tokens.json` stores `{name, scope, sha256, created}` entries; only `add` prints the plaintext token, once. Names are unique and match `[a-z0-9-]{1,40}`. `list` shows name, scope and creation time, never hashes. Writes use a temporary file and atomic rename, with mode 0600 on Unix. A separate `agent-tokens.lock` serializes mutations across processes, including their reload before saving. Authentication checks the file's modification time and size per request and reloads changes, so revocation takes effect without a server restart. An unreadable or malformed file disables only `/mcp` (503), logs one error per file change without token material, and recovers when repaired; the hub still starts and other routes keep working.

`Authorization: Bearer <agent-token>` is required, unless the Claude connector option below is on. Missing, invalid and device tokens return plain 401 responses. Read scope lists the 4 read-only tools (`search_notes`, `read_page`, `memory_read`, `notes_help`) and also permits the read-only hidden and alias names (`feedback_list`, `list_pages`, `backlinks`, `agenda`, `overdue`, `query_database`, `memory_search`, `memory_list`); other calls return a JSON-RPC error. Write scope lists all 15 tools and permits every name, including `import_markdown`. Every request carrying `Origin` is refused with 403, including localhost origins, unless the Claude connector option is on; this route sends no CORS headers.

**Claude connector option (off by default; `[mcp_oauth]`).** A bearer token that is not an agent token is checked as an Authentik access token: an offline JWT check against the claude-mcp issuer, then the same allow-list as app login. Allowed accounts get the full write-scope tool set, and their edits are recorded as `oauth:<email>` (or `oauth:<sub>`). Without a valid token, `/mcp` answers 401 with `WWW-Authenticate` pointing at the protected-resource metadata; a token outside the allow-list gets 403. Setup and limits: [Claude connector](../integrations/ai-assistants.md#claude-connector).

The hub and desktop use the same transport-independent JSON-RPC handler in [vault_mcp](../../crates/mcp/src/server.rs): initialize, tool lists and calls, errors and notifications. Replies are plain `application/json`; notifications return an empty 202. There are no sessions or SSE streams; ordinary bodies are limited to 8 MiB, with the per-source archive allowance described above for repo imports. Calls use the hub's existing shared vault on a blocking task. Writes record `agent:<name>` in the existing page peer map and history's `Version.device`, with a scoped identity per synchronous call and no sync or history format change. Device writes retain their device identity. The Flutter history panel still displays nonlocal identities as "Another device". The endpoint is available independently of the BubblePaw `[agent].enabled` setting.

## Agent MCP gateway (`/mcp/agent`)

**Implemented; no deployment or real connector sign-in is established here.** `https://notes.breadcat.cc/mcp/agent` is a separate, opt-in endpoint backed by the vault registry and the embedded agent's BubblePaw runtime. `/mcp` retains its vault tools, static agent tokens and stateless transport. Source: [mcp_agent.rs](../../crates/hub/src/mcp_agent.rs), [gateway.rs](../../crates/agent/src/gateway.rs).

- **Enable and select a host.** Set `NOTES_HUB_MCP_AGENT_ENABLED=true`; this requires the existing MCP OAuth issuer/client configuration. `NOTES_HUB_MCP_AGENT_DEFAULT_HOST` defaults to `hub`. Vault tools match `/mcp`; host tools and `hosts` are added. Every host tool accepts optional `host`, while vault tools always act on the vault. Per-call selection is tool argument `host`, request URL `?host=name`, global `NOTES_HUB_MCP_AGENT_DEFAULT_HOST`, then `hub`. Only `hub` is implemented. Unknown hosts in the URL or configuration produce a call error listing available hosts, without fallback. An unavailable embedded agent produces an error rather than falling back to another host. The hub registers no desktop tool pack and has no shell tools.
- **Authentication.** OAuth bearer tokens only; static agent and device tokens are not accepted. It reuses `/mcp`'s issuer, audience validation and app-login allow-list (`oidc.allowed_users` or `oidc.allowed_groups`, in practice `notes-users`). The JWT `aud` must contain the configured client ID. Additional resource audiences `<public_url>/mcp`, `<public_url>/mcp/agent` and `<public_url>/mcp/agent?<query>` are accepted on both endpoints; a resource audience alone is refused. The public `claude-mcp` client ID is `d1QDPw9lEM6jOT2WFfcJkggcMqLuA6BrEIvvWF4b`, issuer `https://auth.breadcat.cc/application/o/claude-mcp/`; keep the connector client secret blank. A 401 challenge points to `/.well-known/oauth-protected-resource/mcp/agent`, whose resource is `<public_url>/mcp/agent`. URL host selection is preserved in discovery: `?host=hub` appears on both the challenge metadata URL and its advertised resource URL, and that query-bearing resource audience is accepted across both endpoints while the configured client ID remains required in `aud`. Accounts outside the allow-list get 403. Requests without an Origin header pass; a single Origin of `https://claude.ai`, `https://claude.com` or `https://chatgpt.com` passes; others or multiple Origins get 403. Existing OAuth callbacks still govern connector login. See [connector setup](../integrations/ai-assistants.md#agent-gateway-connector).
- **Tools.** Vault tools use the exact `/mcp` registry, schemas and HTTP dispatcher, including imports, aliases, limits and actor attribution. The agent's own vault adapters remain unchanged but are omitted from the additional host list using registry-derived names. Other name collisions fail discovery. Host definitions come from BubblePaw's `Harness::effective_tool_definitions`, with gateway exclusions and current permission policy applied; calls use `Harness::call_tool` without a model turn. The hub defaults to native mode without JS. `[agent] enable_js = true` additionally offers `js` and `js_help`; `execution_mode = "js"` selects legacy mode. The local in-app agent retains JS. `hosts` returns `[{name, online, default}]`, marking the selected host; an unknown selection returns an error listing available hosts. When JS is offered, `js_help` discovers its capabilities and runtime guidance, and `tools.hosts()` remains available inside JS. Tools requiring interactive approval are absent and refused, including nested JS calls. `history_*`, web tools, `ui_*`, `todo`, `question` and `reload` are excluded.
- **Sessions.** Initialize returns a new `Mcp-Session-Id` bound to the OAuth subject. Each HTTP session owns a distinct agent session and persistent JavaScript environments; client-supplied chat IDs cannot select them. Host tool execution requires that header; vault calls, discovery and help do not. A concurrent call in the same session receives a busy error. DELETE can cancel an active host call or JS cell. DELETE with the header closes the session and environments. Sessions expire after 24 hours idle; a later initialize reclaims expired sessions. There are at most 256 HTTP sessions. Restart loses the HTTP session mapping, so clients must initialize again. Persistent JavaScript state does not imply restart recovery.
- **Transport.** POST and DELETE only, JSON replies, empty 202 notifications and no SSE stream; ordinary request bodies are limited to 8 MiB; vault imports use the same per-source archive allowance as `/mcp`. Vault calls and `hosts` remain usable when the embedded agent is unavailable; initialization and host execution require it.

To switch an existing OAuth connector, change `https://notes.breadcat.cc/mcp` to `https://notes.breadcat.cc/mcp/agent?host=hub`, or back, retaining the same client ID and login. Both routes share issuer, accepted audiences, allow-list, scopes and registered redirect URIs. The same valid OAuth token is accepted across both routes; the server does not bind it to a metadata resource URL. Static agent tokens remain specific to `/mcp`. See [switching instructions](../integrations/ai-assistants.md#switch-an-existing-connector).

Desktop outbound relay is not implemented. Later integration needs an authenticated outbound desktop connection, host registration and availability reporting, and per-host routing that preserves the host's real tools, permission policy and session ownership. A `desktop` host argument is not desktop access today.

## Agent (`/agent/ws`)

The hub opens one `vault_agent::Agent` on its shared `Arc<Vault>`, at start when enabled, and serves it to every device. The DTOs, method names and envelopes are in [crates/agent_wire](../../crates/agent_wire) (`notes_agent_wire`), shared with the app's bridge. Transport-free method dispatch, event subscriptions and the 1024-event gap buffer are in [`vault_agent::rpc::Dispatcher`](../../crates/agent/src/rpc.rs); this module has no axum or authentication dependency. The hub retains lazy opening, authentication, socket frames, pings and sign-out closing. Both hosts use `Agent::open`; the hub sets `host_kind = Hub` and `packs = []`, without a desktop pack setting.

- **Opening.** If the agent fails to open, the failure is logged, sync keeps working, `/agent/ws` answers 503 with the reason, and the next connection tries again.
- **Auth.** `Authorization: Bearer <token>` and `X-Device: <id>`, checked by `Hub::handle` exactly as sync checks them, before the upgrade. No token gets 401; a bad token or wrong device gets sync's status (401 or 403). Tokens are never read from the query string and never logged. The tokens are checked again at every ping, so a socket whose token was replaced closes within 30 s. Sign-out closes that token's sockets at once. `--pair` and `--unpair` from another process take effect at the next token check, as for sync; token file saves reload under a stable `tokens.lock` and use atomic rename with mode 0600 on Unix.
- **Frames.** JSON-RPC 2.0, one text frame per message. Requests `{"jsonrpc":"2.0","id":n,"method":…,"params":…}` are answered in any order, each on its own blocking task.
- **Methods.** `host_info`, `models`, `sessions`, `create_session {title}`, `rename_session {session_id,title}`, `delete_session`, `session_state`, `send {session_id,text,model?}`, `steer {session_id,text}`, `cancel`, `decide_approval {approval_id,decision,reason?}`, `events_after {after,session_id?}`, `run_script {session_id,label,code}`, `subscribe`, `unsubscribe` (the bare session methods take `{session_id}`). Also: `projects_list`, `project_add {name,path}`, `project_remove {id}`, `project_rename {id,name}`, `session_project_set {session_id,project_id}`, and `permission_mode_set {mode}`. Results are the bridge's DTOs in snake_case. Project add/rename return a project `{id,name,path,added_at}`, remove returns null, and session project set returns the updated session (including optional `project_id`/`project_name`). These project methods return "not supported on this host" on the hub, which has no desktop pack. User-initiated project management does not request tool approval. `permission_mode_set` returns and persists the new mode (`manual`, `auto` or `bypass`); it applies from the next message/run without reopening the agent, while an active run retains its original policy.
- **Host capabilities.** `host_info` returns `{kind, packs, tools, providers, can_run_tasks, version, permission_mode, projects}`: on this hub `kind` is `hub`, `packs` is empty, `exec` is absent, and both `can_run_tasks` and `projects` are false. Desktop hosts add the optional desktop pack and report `projects: true`. Clients query this method to decide task and project availability. These new methods and capability fields require redeploying the hub; the desktop relay (4d) is not implemented. Permission mode is agent-wide; with no Confirm tools it has no effect on ordinary hub tools.
- **Origin.** The server sets the run's client to the device id, named by the token's label. Events carry it as `origin_id` / `origin_name`.
- **Events.** After `subscribe`, the socket gets `{"jsonrpc":"2.0","method":"event","params":<AgentEvent>}` for every event, live `paragraph` deltas included. Each socket buffers 1024; when full, events are dropped and one `gap` event (`skipped`, `sequence` = the last durable seq delivered) follows when there is room, so the client catches up with `events_after`.
- **Errors.** `{code, message, data:{kind, resets_at?}}`, with the bridge's error strings as `message`. Codes: -32700 parse, -32600 invalid request, -32601 unknown method, -32602 bad params, -32000 agent error. `kind` is `session_busy`, `not_found`, `provider_auth` or `rate_limit` (with `resets_at`); `not_found` is detected from the agent's error text.
- **Liveness.** The server pings every 30 s and drops a socket after 90 s without a frame. A device may hold several sockets. Shutdown closes every socket, then the agent.

### Agent tooling API (Part A, 2026-10-09)

The hub and desktop share the additional tooling contracts in `notes_agent_wire`:

- `session_state` includes `activity` and `edits`. Activity entries carry run/call IDs, order, tool name, summary, error state and an optional edit ID. Edit records expose page identity, kind, added/removed counts, line-diff hunks, changed block IDs and undo state; full before content is not sent to clients.
- `edit_undo {edit_id}` restores an agent change as a new vault change and returns the updated edit record. It refuses an intervening page change with `page changed since the agent's edit; use page history`.
- `send {session_id,text,model?,context?}` accepts `context: {pages:[{id,title}],selection?:{page_id,page_title,block_ids,text}}`. User transcript entries expose `text` and parsed `context` separately; selection text is capped at 4,000 characters before the model sees it.
- `subscribe {session_id?}` retains the normal event stream and optionally marks the client as viewing one session. Only clients explicitly viewing the session count as a UI recipient; unsubscribe, replacing the stream or disconnecting clears that presence.
- Page tools notify viewers with `{"jsonrpc":"2.0","method":"ui_action","params":{"session_id":"…","run_id":"…","action":"open_page","page_id":"…","block_ids":[]}}`. The other action is `highlight`. These notifications are transient; tool activity and edit records persist in the agent database.
- `host_info.tools` lists the task, daily-note and page UI tools on both hosts. The hub still has no desktop filesystem or process tools and starts in Bypass.

Deploying these methods, snapshot fields and notifications requires a hub redeploy. Part B will add the Flutter UI that consumes them; no deployment is performed in Part A.

## Login sequence

1. The app opens `/auth/start` in a browser. `redirect` must be exactly `http://127.0.0.1:<port>/done` (port 1024–65535; the desktop's loopback listener) or `notes://auth` (Android). Queries, fragments, other paths, `localhost`, and alternate spellings that URL parsing could normalize are refused. Anything else, or a device id that is not 1–128 letters, digits, `-`, `_` or `.`, gets a 400 page. `name` is cut to 64 characters.
2. The hub makes a state, a nonce and a PKCE verifier (S256), keeps them in memory for 10 minutes, and redirects to the issuer's authorize endpoint, found through discovery (cached for 10 minutes).
3. `/auth/callback` takes the started login (single use). An unknown or expired state shows a small HTML error page. Otherwise every failure redirects to the app with `error=` and the app's `state`:
   - `<the issuer's error>` when the issuer sent one
   - `login_failed`: the code exchange (client secret plus verifier, HTTP Basic) or the ID token check failed. The `openidconnect` crate checks the signature through the JWKS, `iss`, `aud`, `exp` and `nonce`.
   - `access_denied`: the user is in neither allow list
   - `device_refused`: the device id is the hub's own
4. On success the hub creates a device token with `Hub::login_device` (label `<name>`, bound to the device; re-login replaces the device's old token; a clashing label gets ` (2)`) in the usual `tokens.json`, and redirects to `redirect?code=<one-time code>&state=<app state>`. The code is 256 random bits, single use, valid for 2 minutes.
5. The app posts the code with its device id to `/auth/redeem` and stores the token. It then syncs as any paired device, over `https://`.

## Security notes

- Started logins and codes are kept in memory under the SHA-256 of their secret, so a restart cancels logins in progress. At most 10,000 started logins are kept.
- Token hashes and the redeem device are compared in constant time.
- Auth responses carry `Cache-Control: no-store` and `Referrer-Policy: no-referrer`. No cookies are used.
- The OIDC HTTP client follows no redirects (as `openidconnect` requires) and times out after 15 s.
- The token for a login is written at the callback. If its code is never redeemed, the entry stays until the device logs in again or is unpaired.
- An email in `allowed_users` matches whether or not the issuer marks it verified (Authentik sends `email_verified: false` by default). Prefer `allowed_groups` or a `sub`: an email match is only safe while nobody else can set that email in Authentik (no self-enrollment or editable emails).

## Tests

`tests/open.rs` covers `/open`: a valid page or block id gets the bounce page with the `notes://` target and no auth, and malformed ids get 400. `tests/assetlinks.rs` covers the statement: served with the configured package and fingerprints, and no route when unset; `config.rs` unit tests cover the environment, validation and the statement's body. `tests/mcp.rs` covers initialization, the read (7) and write (17) tool lists, reads, refused read-scope writes, agent write attribution, device identity after agent writes, live token creation and revocation, plain authentication failures, Origin rejection, GET 405, notifications, parse errors and batches. Token-store unit tests cover hashing, names, atomic replacement, Unix permissions and reloads.

`cargo test -p notes_hub`: config parsing (with `[agent]` and its variables) and the redirect check (unit); `tests/agent.rs` (scripted provider): no or bad token refused before the upgrade; sessions, a `run_script` page that a `SyncClient` peer pulls, typed `not_found` and `session_busy` errors, subscribed events with rising seq and the device as origin; an approval round trip through `tools.test_confirm`; sign-out closing the socket; an agent that cannot open giving 503 while `/sync` works, then opening once fixed. And `tests/login.rs` against an in-test mock issuer (axum: discovery, JWKS, authorize, token, RS256 ID tokens signed with a test key generated per run). Covered: allowed users by email and by group get tokens with which two `SyncClient`s converge through the hub; a token is bound to its device; disallowed users, a wrong nonce, a wrong audience and an expired ID token are refused and make no token; an unknown state shows the error page; bad redirects get 400; codes are single use, bound to the device and expire; re-login replaces the token; label clashes get ` (2)`; sign-out drops the token. `tests/local_workflow.rs` (2026-10-11) runs the local mobile-Hub workflow: a device signs in through the mock issuer, `/auth/redeem` gives the token and `account`, the device's note syncs to the hub, and a chat over `/agent/ws` sends one message to a scripted model. The model's `js` calls search, read and append to the note inside `js`; the events show the nested tool calls in order, the transcript holds the results, the hub has the edit, and the device pulls it back to the same markdown. It stops the hub and the mock issuer before it ends.

## Limits

- The deployed hub at `https://notes.breadcat.cc` answered a read-only `/auth/start` probe with 302 to Authentik on 2026-10-08. The app's complete real Authentik login has not been exercised here.
- The app browser login is implemented in [hub_auth.dart](../../app/lib/features/sync/hub_auth.dart), with a cancellable five-minute desktop loopback or Android `notes://auth` wait and `/auth/redeem`; [sync settings](../../app/lib/features/sync/sync_settings.dart) store the URL/token through the existing pairing connection and retain the hub's device label. Sign-out calls `/auth/signout` and clears local credentials and the Hub agent choice even if the server cannot be reached. The Dart pairing codec also accepts HTTPS and custom ports. Fake-hub tests cover these flows; a complete real Authentik app login remains unverified. The tightened redirect validation requires a hub redeployment.
- The user reports that the deployed hub's providers are signed in and its agent runs. Automated hub tests use the scripted provider; this app change has not exercised a real provider session on the deployed hub.
