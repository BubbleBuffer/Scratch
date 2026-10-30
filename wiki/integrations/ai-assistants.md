---
type: topic
layer: vision
summary: "Claude and ChatGPT use the vault over MCP; Ask AI and tasks assigned to Agent run on this computer or on the hub."
parent: "[[integrations/index]]"
status: current
---

# AI assistants

**Status:** a first version is built; see [Current state](#current-state-2026-10-01). Plans, the agent runner proposal and open questions are in the internal wiki (notes-app > AI assistants).

- 2026-09-30: tasks for AI agents run on the user's desktop with their own Claude Code or Codex subscription, or an API such as DeepSeek, not a cloud service. Superseded on 2026-10-22 (next line).
- 2026-10-22: the only AI that runs inside Scratch is the BubblePaw agent, on the hub or on this computer, per "Run AI on". The Claude Code, Codex and DeepSeek Ask AI runners are removed. External agents (Claude Code, Codex, ChatGPT, claude.ai) connect to the vault from outside through `/mcp`, `/mcp/agent` or agent tokens; Scratch never starts them.

## Assistants using the vault (MCP)

- The hub's `POST /mcp` is the primary agent transport. It serves the vault's 17 listed tools on the hub's shared replica, using named, revocable agent tokens, each with a tool set (Read, Write or Everything). [Hub](../sync/hub.md#agent-tokens-and-mcp-mcp) describes token management, scopes and history attribution. The desktop `notes-mcp` (stdio) is the offline fallback.
- The separate, opt-in `/mcp/agent` gateway exposes the vault tools and your computers' tools, in one of two modes (see the connector section below). It uses OAuth only; see [Agent gateway connector](#agent-gateway-connector). `/mcp` retains its existing vault tool interface.
- Page links for the user: `read_page`, `create_page` and `edit_page` results carry a `link:` line (and a `block link:` line for a block), an http address `<base>/open/<id>` (`?block=<id>` for a block). `<base>` is the serving hub's `public_url`, or `http://127.0.0.1:<port>` without one (47100 on the desktop). The Claude app refuses custom-scheme links from chat but opens http ones, so the address bounces to `notes://page/<id>`. The server instructions tell the agent to write `[Title](<link>)` with the `link:` line. See [links](../../docs/notes/links.md) and [app state](../app/app-state.md).
- **Claude:** Claude Code and Claude Desktop can connect locally. Claude on the web and phone uses custom connectors, which need a public HTTPS address.
- **ChatGPT:** supports MCP connectors too, with the same public-address requirement.
- Browser connectors that require OAuth: Claude's connector uses the hub's one Authentik OAuth2 provider through an optional OAuth path on `/mcp`, described in [Claude connector](#claude-connector). ChatGPT, Gemini and CLI MCP clients use the same provider. Otherwise the hub endpoint accepts static agent bearer tokens and rejects every browser `Origin` header.

## Connecting with `notes setup`

`notes setup claude` and `notes setup codex` connect a local agent to the hub without copying a token by hand. Both use the desktop app's sign-in on that computer: the hub address and device token in the app's `settings.json` (`%LOCALAPPDATA%\Notes` on Windows, `~/.local/share/notes_app` elsewhere), and the device id in `<vault>/.vault/device`. Each run that adds a server creates one agent token with the Write tool set, named `<agent>-<host>`, such as `claude-mybox` or `codex-mybox`. The host name is lowercased, characters outside `a-z 0-9 -` become `-`, and the name is cut to 40 characters. A duplicate gets `-2` to `-9`. Setup never revokes a token; revoke it in Settings > Connectors > Apps that use your vault.

- `claude` runs `claude mcp add --transport http --scope user notes <hub>/mcp` with the token in an `Authorization` header. `NOTES_CLAUDE_BIN` names another `claude` binary.
- `codex` adds `[mcp_servers.notes]` to `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`), with the hub's `/mcp` address and `bearer_token_env_var = "NOTES_CODEX_TOKEN"`. The file never holds the secret. Setup prints the token once, with two ways to set `NOTES_CODEX_TOKEN` where Codex runs: `setx` on Windows (then open a new terminal) and an `export` line in a shell profile. It cannot be shown again; if it is lost, revoke the token and run setup again.

Re-running either command does nothing when the agent already has a `notes` server: no token is created and no file changes. To set up again, remove the entry (`claude mcp remove notes -s user`, or delete the `[mcp_servers.notes]` table) and run the command again. If a step fails after the token is created, the message names the token so you can revoke it.

Sign the app in to a hub on this computer first. The hub address must be https, or http on 127.0.0.1 or localhost (for example `https://hub.example`).

## Starting jobs from an outside agent

An agent connected to the hub can hand a task to the hub's agent and collect the result later, instead of doing the work in its own session. Call `jobs` with `action` `create` and a `prompt`; the reply is the job, `queued` at first. Then call `wait` (or `get`) until its `status` is `completed`, and read `result`. The run sees only the prompt, so say which pages to use, what to change and what to report.

- Shell: `notes jobs create [--tools read|write|everything] [PROMPT | < prompt]`, then `notes jobs wait JOB [--timeout-ms N]` (at most 60000), `notes jobs get JOB`, `notes jobs cancel JOB` or `notes jobs list [--limit N]`.
- The token must have the Write or Everything tool set. A Read token cannot call `jobs`. `--tools` can narrow the run's tools, never widen them.
- Jobs run only on the hub. A desktop's local MCP server answers `jobs` with a message that they need the hub's `/mcp` (`vault_tools::tools::JOBS_NEED_A_HUB`).

`create` also takes optional `page` (a page id) and `after` (a block id on that page). With `page`, the job puts a placeholder in the page, and a completed result replaces the placeholder. Failed, cancelled and interrupted runs keep the placeholder and update its status text; failed and interrupted runs also show the error with a red background. `after` without `page` is an error; an unknown page gives "no such page".

The states, who can see a job, the limits and what the hub does on restart are in [Jobs](../sync/hub.md#jobs).

## Hub agent models

The hub's agent offers four models: `claude-code/claude-opus-5-5`, `claude-code/claude-sonnet-5-5` (the default), `claude-code/claude-haiku-5-5` and `opencode/deepseek-v4.1-flash`. In the app, a chat on the hub picks one of them under Settings > Agent > Model, after "Hub default (<model>)". Jobs cannot pick a model: they run on the hub's default. A hub default set in the vault applies only when it is one of the four; otherwise the hub's configured default is used.

The operator sets the list and the default in `[agent]` (`models`, `default_model`, or `NOTES_HUB_AGENT_MODELS` and `NOTES_HUB_AGENT_DEFAULT_MODEL`). The configured default must be listed or startup fails. Set `OPENCODE_API_KEY` for DeepSeek; for Claude, mount the signed-in login directory and set `CLAUDE_CONFIG_DIR`, or provide `CLAUDE_CODE_OAUTH_TOKEN`. Missing credentials fail the run with a variable-naming error. Desktop providers and their default stay unchanged, with OpenCode added when its key is set. See [Agent models and credentials](../sync/hub.md#agent-models-and-credentials) and [Provider sign-in](../sync/hub-deploy.md#provider-sign-in-and-checks).

## Claude connector

**Status:** off by default (the `[mcp_oauth]` settings unset). Not yet verified with a real claude.ai sign-in.

Claude on the web and phone reaches the vault through a custom connector. The hub uses one Authentik OAuth2 provider for everything: device sign-in (`[oidc]`) and MCP OAuth (`[mcp_oauth]`) on `/mcp` and `/mcp/agent`. `[mcp_oauth]` `issuer` and `client_id` (environment `NOTES_HUB_MCP_OAUTH_ISSUER` and `NOTES_HUB_MCP_OAUTH_CLIENT_ID`) are the same issuer and client ID as `[oidc]`.

The provider is a public client, with no client secret (since 2026-10-22; it was confidential before). Connectors need only the client ID: leave the client secret field blank. The hub and these connectors always send PKCE S256. Authentik does not itself require PKCE for public clients, so the protection of the code is PKCE plus the strict redirect URIs listed below.

### How it works

- `/mcp` accepts two kinds of bearer token. First the agent tokens, unchanged. Otherwise an OAuth access token issued by the Authentik provider.
- The hub validates that token offline as a JWT: RS256 signature against the issuer's JWKS, `iss` equal to the configured issuer, `exp` not passed, and `aud` containing the configured client ID. An additional resource audience may be `<public_url>/mcp`, `<public_url>/mcp/agent` or `<public_url>/mcp/agent?<query>` on either endpoint. The configured client ID remains required: a resource audience alone is refused. No resource indicator is required.
- The user must be allowed by `[oidc]`: `oidc.allowed_users` by email or `sub`, or `oidc.allowed_groups`, in practice `notes-users`. If the token lacks the `groups` claim (or the email, when users are listed), the hub asks Authentik's userinfo endpoint once per token and caches the answer until the token expires.
- Discovery and keys are cached for 10 minutes, as app login is. A signing-key rotation in Authentik is picked up within 10 minutes.
- Allowed callers get the same tools as a full-access agent token. Their edits are recorded with actor `oauth:<email>`, or `oauth:<sub>` without an email.
- Without a valid token, `/mcp` answers 401 with `WWW-Authenticate: Bearer resource_metadata="https://hub.example.com/.well-known/oauth-protected-resource/mcp"`. A valid token from an account not on the allow-list gets 403.
- `GET /.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp` serve the protected-resource metadata (RFC 9728). It names resource `https://hub.example.com/mcp`, the issuer as authorization server, the scopes `openid email profile offline_access`, and bearer tokens in the header only.
- Origin: a request without `Origin` passes. With the feature on, a single `Origin` of `https://claude.ai`, `https://claude.com` or `https://chatgpt.com` passes; other or multiple Origin values get 403. Origin logging is bounded and uses debug level.
- With both variables unset, the hub behaves as before: no metadata routes, a bare `Bearer` 401, and every Origin refused.
- Claude Code and Codex set up with `notes setup` use agent tokens on `/mcp` and don't need OAuth.

The two variables are listed in [Hub deployment](../sync/hub-deploy.md#claude-connector).

### Authentik side

Created over root SSH with `ak shell`. Change Authentik infrastructure the same way, never through the admin web UI.

- Provider: one OAuth2/OpenID Connect provider for the hub, serving device sign-in and MCP OAuth. Public client (client ID only, no client secret), PKCE S256.
  - Grant types `authorization_code` and `refresh_token`, set explicitly. An empty list rejects every login; see [the live deployment note](../sync/hub-deploy.md#live-deployment).
  - Strict redirect URIs:
    - `https://hub.example.com/auth/callback`: device sign-in.
    - `https://claude.ai/api/mcp/auth_callback` and `https://claude.com/api/mcp/auth_callback`: Claude.
    - `https://chatgpt.com/connector_platform_oauth_redirect`: ChatGPT.
    - `https://chatgpt.com/connector/oauth/<callback-id>`: ChatGPT's per-connector callback. Copy it from the connector's settings page. ChatGPT uses it because Authentik does not advertise RFC 9207 `iss` in authorization responses.
    - `https://oauth-redirect.googleusercontent.com/r/<id>`: Gemini custom MCP.
    - `^http://127\.0\.0\.1:[0-9]{1,5}/callback(/[A-Za-z0-9_-]{1,64})?$` and `^http://localhost:[0-9]{1,5}/callback$`: loopback regexes for CLI MCP clients doing OAuth.
  - Signing key RSA, so tokens are RS256.
  - Scopes `openid`, `email`, `profile`, `offline_access` (refresh tokens for connectors).
- Application bound to the group `notes-users`.
- Issuer: `https://auth.example.com/application/o/<slug>/`, the same as `[oidc] issuer`. Discovery is public and advertises S256.
- The client ID is not written in this repo. The hub reads it from `[oidc]`. `NOTES_HUB_OIDC_CLIENT_SECRET` stays empty for this public client; set it only for a confidential provider.
- The old separate public client `claude-mcp` is no longer used by the hub. It may still exist in Authentik; remove it once connectors are moved to this provider.

### Connector settings in Claude

In Claude, go to **Settings → Connectors → Add custom connector**:

- Name: `Notes` (any name).
- URL: `https://hub.example.com/mcp`.
- Advanced settings: OAuth Client ID is the hub's `[oidc]` client ID. Leave OAuth Client Secret blank.

Then choose **Connect**. Claude opens Authentik's login. After sign-in, the vault tools appear in chats.

ChatGPT, Gemini and CLI MCP clients use the same client ID, with no secret, and the redirect URIs listed above.

### Limits

- No dynamic client registration: Authentik does not offer it here, so each connector takes the client ID entered by hand. There is no secret to enter.
- Authentik serves its metadata only at `<issuer>.well-known/openid-configuration`, not at the RFC 8414 path. The MCP spec has clients try that URL last, so discovery should still work. This is unverified with Claude until the first real sign-in.
- Streamable HTTP over POST only. `GET /mcp` stays 405 (no SSE stream).
- OAuth sessions get full access. There is no read-only OAuth scope.

## Agent gateway connector

**Implemented, off by default; deployment and real connector sign-in are not established here.** Add a separate custom connector in ChatGPT or claude.ai with one of these addresses. Each one offers the `/mcp` vault tools.

- `https://hub.example.com/mcp/agent` lets the assistant use any of your computers. It calls `hosts`, then names the computer in each call.
- `https://hub.example.com/mcp/agent?host=Work%20PC` uses one computer, with its tools listed directly. The name is the computer's name on the hub (matched ignoring case); after a rename of this computer in Settings > Connectors > This computer for hub apps, the old address stops working, so update the connector.
- `https://hub.example.com/mcp/agent?host=hub` uses the hub's own runtime (`js` when enabled, `chat_read`, environments).

Settings > Connectors > This computer for hub apps shows the per-computer and any-computer addresses, with Copy, while "Let the hub use this computer" is on. The hub has no desktop tool pack or shell; computers are reached through the [PC host relay](agent.md#pc-host-relay-hostcomputer-built-2026-10-22). JS and `js_help` appear only when enabled.

Use the same Authentik provider as the `/mcp` connector: its client ID, with no client secret, and PKCE S256. The issuer is the `[mcp_oauth]` issuer. The redirect URIs listed in [Authentik side](#authentik-side) still apply. Allowing the ChatGPT Origin does not register ChatGPT's per-connector callback; add it on the provider from the connector's settings page before connecting. Even then, no successful ChatGPT connection is established until tested.

Enable `NOTES_HUB_MCP_AGENT_ENABLED=true` alongside `NOTES_HUB_MCP_OAUTH_ISSUER` and `NOTES_HUB_MCP_OAUTH_CLIENT_ID`; enabling the gateway without both OAuth settings is a configuration error. `NOTES_HUB_MCP_AGENT_DEFAULT_HOST` and the TOML key `mcp_agent_default_host` are retired: both are ignored, and startup prints a deprecation line when the value is anything but `hub`. The other TOML keys are `mcp_agent_enabled` and `[mcp_oauth]` `issuer`/`client_id`.

There is no default host, and the vault's "Run AI on" setting does not affect the gateway. A call names its computer: the `host` argument in any-computer mode, or `?host=` in the URL. Unknown or offline names are call errors and never fall back to another computer. A pinned URL lists the available hosts; any-computer mode answers `Unknown computer <name>. Call hosts.`

The gateway accepts OAuth bearer tokens only. It shares `/mcp`'s issuer, RS256/expiry/audience validation and app-login allow-list (`oidc.allowed_users` or `oidc.allowed_groups`, in practice `notes-users`). The JWT `aud` must contain the configured client ID. Additional resource audiences `<public_url>/mcp`, `<public_url>/mcp/agent` and `<public_url>/mcp/agent?<query>` are accepted on both endpoints; a resource audience alone is refused. Requests without Origin pass; single Origins `https://claude.ai`, `https://claude.com` and `https://chatgpt.com` pass, while other or multiple Origins are refused. Missing/invalid credentials return 401 with metadata at `https://hub.example.com/.well-known/oauth-protected-resource/mcp/agent`; an account outside the allow-list gets 403. That metadata advertises resource `https://hub.example.com/mcp/agent` and the same issuer and scopes.

Discovery preserves URL host selection: a connector at `/mcp/agent?host=hub` receives a 401 challenge pointing to `/.well-known/oauth-protected-resource/mcp/agent?host=hub`, whose resource includes `/mcp/agent?host=hub`. The query selects a host; the shared verifier accepts the query-bearing resource audience on either endpoint, while still requiring the configured client ID in `aud`.

`tools/list` returns the same vault definitions and schemas as `/mcp`, plus either the pinned host's model-facing tools or, in any-computer mode, `hosts`, the core computer tools (each with a required `host`), `tool_search` and `call_tool`. Vault calls use the `/mcp` implementation, including imports and write attribution, and never take `host`. The gateway omits the host's duplicate vault adapters using the vault registry; unrelated name collisions fail discovery. Pinned hub tools take no `host` argument and run through BubblePaw's single-tool API without invoking a model; computer tools run on their computer.

The hub defaults to `[agent] execution_mode = "native"`, which offers no `js` tool. `execution_mode = "js"` offers `js` and `js_help`. The environment override is `NOTES_HUB_AGENT_EXECUTION_MODE`. The removed `enable_js` key and `NOTES_HUB_AGENT_ENABLE_JS` are ignored with a deprecation warning. The local in-app agent keeps its JS mode. Only tools the host actually registers are available: the hub has no desktop filesystem or process pack.

In any-computer mode, `hosts` returns `[{name, online}]` for every computer that has connected as a host, kept across hub restarts; an offline computer reports `online: false`. A pinned connection, `?host=hub` included, has no `hosts` tool. When JS is advertised, `js_help` lists its capabilities and full schemas, and `query: "js"` describes the runtime. `tools.hosts()` remains available inside JS. Tools requiring interactive confirmation are omitted and refused, including nested JS calls. `history_*`, web tools, `ui_*`, `todo`, `question` and `reload` remain excluded.

HTTP initialize returns an OAuth-subject-bound `Mcp-Session-Id`. Send it with subsequent host tool calls. When JS is enabled, its environments persist within that session and remain isolated from other sessions. `DELETE` with that header closes the session and environments. Idle sessions expire after 24 hours and are reclaimed on a later initialize; after a hub restart, initialize again. This endpoint accepts POST and DELETE, with JSON replies and no SSE stream. More detail: [Agent gateway](agent.md#agent-mcp-gateway-mcpagent).

### Switch an existing connector

Edit the connector URL from `https://hub.example.com/mcp` to one of the `/mcp/agent` addresses above (`?host=hub`, `?host=<computer>` or none), or back. Keep the same OAuth Client ID and login. Both routes use the same issuer, accepted audiences, allow-list, scopes (`openid email profile offline_access`) and registered redirect URIs. The server accepts the same valid OAuth token on either route, without binding it to the resource URL advertised in metadata. Static agent tokens remain specific to `/mcp`.

## Current state (2026-10-01)

Built in the original implementation: an MCP server for the vault, AI tasks run by desktop agents, and merge proposals in the existing block-conflict review. The v2 core/sync conflict backend is built and checked, and FRB 2.13.0 bindings are regenerated; 12 MCP tests and 21 Rust bridge tests pass; native and targeted Flutter checks also pass. Its approved new screen excludes AI, while agent helpers and interim block controls remain as described below. Code: [crates/mcp](../../crates/mcp) (`vault_mcp`, binary `notes-mcp`), the bridge [app/rust/src/api/ai.rs](../../app/rust/src/api/ai.rs) and [app/lib/features/ai](../../app/lib/features/ai).

### MCP server

- Tool definitions and implementations live in [crates/tools](../../crates/tools) (`vault_tools`), with read-only, destructive and approval metadata shared by MCP and the coming BubblePaw agent. `vault_mcp` adapts them to MCP without changing their schemas or responses. [crates/agent](../../crates/agent) (`vault_agent`, desktop only) hosts them as BubblePaw tools inside `js` and adds the `agent` task runner; see [Agent](agent.md#built-2026-10-08-to-2026-10-09).
- Listed tools (17, in serving order; the source is the registry in [crates/tools/src/tools.rs](../../crates/tools/src/tools.rs)):
  - Pages: `search_notes` (full-text search, rg-style results with block ids; without a query it lists the page tree; `backlinks_of` lists the pages linking to a page; `is:`, `done:`, `assignee:` and `calendar:` return to-do, task-note, dated-row and date-mention items; `in:memory` searches agent memory; `context:`, `full:`, `limit:` and `sort:` size and order the output), `read_page` (markdown; `block_ids`; `include_backlinks`; an unknown id or title returns a labelled list of pages whose titles contain those words, never a near match), `create_page` (a page or database row), `edit_page` (one op, or an `ops` list: `append`, `replace`, `delete`, `set_body`, `set_props`), `attach_file` (see Attachments below), `trash_page`, `move_page` (`pages`, with `parent` or `after`; moves pages with their sub-pages, refuses a move into the page's own subtree, and a move into or out of a database is a plain move).
  - Planning: `daily_note`, `update_task` (due, done or both; item references come from `search_notes` item lines).
  - Databases: `upsert_row`. Rows are read with `search_notes` and `in:[[Database]] -type:database`.
  - Memory: `memory_read`, `memory_write`, `memory_update` and `memory_delete` below; memories are found with `search_notes` and `in:memory`.
  - Feedback: `feedback_submit` (served to MCP and the in-app agent).
  - Help: `notes_help`.
  - Jobs: `jobs` (`create`, `get`, `wait`, `cancel`, `list`). Served on the hub's `/mcp` and `/mcp/agent` only, not to the in-app agent; see [Jobs](../sync/hub.md#jobs).
  - Pages are named by id or exact title. Every write goes through `vault_core`.
- Attachments (`attach_file`, built 2026-10-23; [attach.rs](../../crates/tools/src/attach.rs)): stores a file as a vault blob (content-addressed, as the app does) and inserts an image or file block whose target is the attachment, at the end of the page or after `after_block`; it syncs like an app-made attachment. Arguments: `page`, exactly one of `path`, `url` or `data`, and optional `name`, `caption` and `as` (`image` | `file`; default image when the content type or bytes say png, jpeg, gif, webp or svg). The result names the block id, the `attachment: ../.vault/blobs/xx/<id>` link, `block link:` and `link:`.
  - `path` reads a file on the machine running the tool. Hosts opt in with `vault_tools::allow_local_paths(true)`: the desktop app (its embedded hub and in-app agent) and `notes-mcp`. The hub process never does, so on the hub `path` fails with "path is only available on your own computer; pass url or data".
  - `url` is https only (redirects too), at most 3 redirects, 25 MB and 30 s. Every connection, redirects included, resolves through a resolver that keeps only public addresses (no private, loopback, link-local, CGNAT, multicast or reserved ranges, IPv4-mapped included), so a rebinding name is checked where it connects. Environment proxies are ignored. Only unit tests can allow loopback http (a `cfg(test)` flag).
  - `data` is base64, at most 10 MB decoded. The hub's `/mcp` and `/mcp/agent` let a `data` string past the 8 MiB request limit up to its own limit; the desktop endpoint's request limit includes the same room.
  - As `image`, bytes are sniffed (png, jpeg, gif, webp, svg); an svg with script, foreignObject, event attributes or `javascript:` is refused, and the stored name gets the matching extension so the MIME type is right. As `file`, any type; the name is sanitised (no directories, reserved or control characters, at most 120 characters).
  - Auth is the same as `edit_page`: Write and Everything tokens may attach, Read tokens may not.
  - CLI: `notes attach PAGE FILE [--caption TEXT] [--after BLOCK] [--as image|file]` sends the file as `data` through the same `/mcp` call as page writes, so it takes files up to 10 MB.
  - Decision (2026-10-23): agents attach files through one shared tool (`attach_file`) rather than a new hub upload endpoint; `path` is refused on the hub, `url` fetches are SSRF-guarded on every hop, and listing, deleting or resizing attachments stay out of scope.
- Hidden, MCP only, never listed: `import_markdown`, the repo import (`roots` archives, `repo_path`, `git_remote`, `last_activity`, `exclude`, `dry_run`). `create_page` takes no import arguments; passing `roots` returns an error that names `import_markdown`. The `notes wiki` CLI sends `import_markdown`. `feedback_list` is hidden and MCP only.
- Aliases are callable but never listed: `list_pages` and `backlinks` (`search_notes`), `update_page`, `append_blocks`, `replace_blocks` (`edit_page`), `reschedule` and `set_done` (`update_task`). Their old arguments are rewritten to the new ones. Plugin 0.6.3 adds `agenda`, `overdue`, `query_database` and `memory_search` as aliases of `search_notes` (old arguments become a query: `is:todo,task,row date:today` for the today view, `is:todo,task date:<now done:no` for overdue, `in:[[Database]] -type:database` for rows, `in:memory` for memories); `tasks` is removed. `memory_list` stays a hidden tool with the plain memory index the CLI reads. The 0.6.3 aliases are removed in the following version.
- Alias notes (2026-10-20):
  - `agenda` with `page` and no `subtree` maps to `in:[[P]] -in:[[P]]/**`, which is that page only.
  - Alias results now include rows from databases whose agenda role is off.
  - `overdue` lists only to-dos and task notes.
  - An empty `kinds` or `calendar` list is an error.
  - `tag` no longer matches a text property named Tags.
- **in: path globs (2026-10-20):** `in:` takes path patterns. `in:Work/*` gives Work's direct children, `in:Work/**` gives all its descendants, and `in:**/Meetings` gives any page titled Meetings. `in:Work -in:Work/**` is Work alone. A plain `in:Work` still means Work and its subtree. Rules and examples: [Search and queries](../../docs/notes/search.md#in-pages-and-paths).
- **Item queries (2026-10-20):** `search_notes` with `is:todo,task,row,mention`, `done:`, `assignee:`, `calendar:` and `undated` returns to-dos, task notes, dated database rows and date mentions with done state, date and the `update_task` reference. `date:` filters the item's date, and page operators (`in:`) filter its page. Overdue is `is:todo,task date:<now done:no` (`now` compares timed items to the minute, all-day items to today), so it lists only to-dos and task notes. Ordinary date mentions come back only when `mention` is listed. The local index rebuilds at cache version 13; page documents and sync formats do not change.
- Item results carry the same references as before: `block:<page>/<block>`, `note:<page>` and `row:<database>/<row>` work with `update_task`, including undated task notes; `mention:` references are read-only. `read_page(block_ids: true)` includes nested block IDs accepted by `edit_page` replace/delete. See [Tasks and dates](../../docs/notes/tasks.md).
- Hub call log: the hub writes one structured line per MCP `tools/call` (target `notes_hub::mcp_call`): the tool name as called, sorted argument keys, result size, error kind and duration. It never logs argument values, page content, queries or tokens.
- `notes_help(topic?)` returns the table of contents, or one topic: `markdown`, `links`, `properties`, `day-notes`, `tasks`, `query`, `templates`, `system-pages`, `memory`, `jobs`, or `tools` (every tool with its arguments, generated from the registry). The topic pages are markdown in `docs/notes/`, compiled into the binary. The MCP server instructions point agents to it.
- Memory tools: `memory_read`, `memory_write`, `memory_update` and `memory_delete`, addressed by path (`global/<category>/<memory>`; older `project/[subproject/]memory` paths still resolve) and never by page id. The path is the memory's identifier everywhere; `name` is only a display label. They use ordinary nested databases under `Memory`; see [Agent memory](agent-memory.md#memory-tools-current-2026-10-14) for arguments, schema and path rules.
- Feedback: `feedback_submit` (summary, optional Markdown details, kind, page_id, tool, error). `feedback_list` (limit and optional kind filter) is callable over MCP but not listed. Use them for unexpected vault failures, user complaints about Scratch/sync/the vault, or an explicit request to report; submit once per issue per session. They share the app's hidden Feedback page; see [Feedback](../app/feedback.md#feedback-over-mcp). All 17 listed tools share the same MCP transports.
- Primary transport: the hub's Streamable HTTP `POST /mcp`, with JSON replies, empty 202 notifications and no sessions or SSE. Agent tokens are separate from device tokens, each limited to its tool set: Read (the 4 read-only listed tools `search_notes`, `read_page`, `memory_read` and `notes_help`; `feedback_list` and the read-only aliases are callable too) Write (all 17 listed tools, including `feedback_submit` and `jobs`) or Everything (every tool the surface serves, including later ones; the same as Write on `/mcp` today), and writes appear in history as `agent:<token-name>`. Revocation is visible without restarting the hub. Every `Origin` header gets 403 unless the Claude connector option is on, which also passes `claude.ai`, `claude.com` and `chatgpt.com`; no CORS headers are sent.
- Offline fallback: desktop stdio only (`notes-mcp`). The app no longer serves MCP on this computer (its local endpoint was removed 2026-10-23, see below).
- **Decision (2026-10-01, local endpoint removed 2026-10-23): co-running with the app.** The vault has a single owner (`.vault/lock`), so two programs must not open it at once.
  - Current: `notes-mcp` over stdio opens the vault for each tool call and closes it again, so the lock is held only during a call.
  - Removed 2026-10-23: the app's local MCP endpoint on `127.0.0.1:47101`, its "Serve the vault to AI apps" switch (`mcpEnabled`, `mcpAddress`), the endpoint file that advertised it, the forwarding branch of `notes-mcp`, and `notes-mcp --http`. AI apps reach the vault through the hub, which runs inside the desktop app, or through `notes-mcp` over stdio.
  - Why (2026-10-01): one writer per vault without a second sync peer and no daemon; stdio clients (Claude desktop, Claude Code) start `notes-mcp` themselves.

### Connecting AI apps

**Connect an agent** (Settings > Connectors > Apps that use your vault on desktop, above the agent tokens; hidden on phones since 2026-10-22, its commands run in a desktop terminal) shows one command per agent, each with Copy: `notes setup claude` for Claude Code and `notes setup codex` for Codex. Run it on the computer where that agent runs; what it does is under [Connecting with `notes setup`](#connecting-with-notes-setup). The app writes no config files and makes no network call from this section.

For an agent with HTTP support, create a hub token in the app (Settings > Connectors > Apps that use your vault: name, tool set Read only, Read and write or Everything, expiry Never, 30, 90 or 365 days, default 90; the secret is shown once with Copy) or with `notes-hub --config <file> --agent-token add <name> --scope read|write|everything [--expires 90d]`, and configure `https://<hub>/mcp` with `Authorization: Bearer <token>`. Keep the plaintext in the client's secret configuration. Settings lists each token's creation, last use and expiry; Revoke there, or `--agent-token revoke <name>`, removes access while the server runs. These are local implementation and integration-test guarantees; they do not establish that a live hub has been deployed with this endpoint.

### Shell client (`notes`)

The `notes` binary in `vault_tools` calls the same hub `/mcp` endpoint. It provides `search`, `read`, `query`, `write` and `append`, plus nine `memory` commands (`list`, `search`, `read`, `write`, `update`, `delete`, `index`, `import` and `rules`; `rules` is answered locally, without hub access) `wiki import`, and `jobs` (`create`, `get`, `wait`, `cancel`, `list`; see [Starting jobs](#starting-jobs-from-an-outside-agent)). Write and append bodies come from stdin; `--json` returns the raw tool result. It does not open a local vault.

`NOTES_HUB_URL` overrides the config's `hub`, then defaults to `https://hub.example.com`. `NOTES_TOKEN` overrides the config's `token`. The config is `%APPDATA%\notes\config.toml` on Windows, or `$XDG_CONFIG_HOME/notes/config.toml` (otherwise `~/.config/notes/config.toml`) on Unix. Tokens are never command-line arguments or printed in diagnostics. See [Agent memory](agent-memory.md) for the index and import rules.

### Desktop connection examples

The desktop examples below are the offline fallback, over stdio only.

Build the binary with `cargo build --release -p vault_mcp` (`target/release/notes-mcp.exe`). It finds the app's vault by itself (`NOTES_VAULT`, else `vaultPath` in the app's `settings.json`, else `%LOCALAPPDATA%\Notes\vault` or `~/.local/share/notes_app/vault`); `--vault DIR` overrides.

**Claude Code**, stdio:

```
claude mcp add notes -- C:\path\to\notes-mcp.exe
```

A project `.mcp.json` does the same:

```json
{ "mcpServers": { "notes": { "command": "C:\\path\\to\\notes-mcp.exe", "args": [] } } }
```

**Claude desktop**: `%APPDATA%\Claude\claude_desktop_config.json` (Windows) or `~/.config/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "notes": { "command": "C:\\path\\to\\notes-mcp.exe", "args": ["--vault", "C:\\Users\\you\\AppData\\Local\\Notes\\vault"] }
  }
}
```

**Codex**: `~/.codex/config.toml`:

```toml
[mcp_servers.notes]
command = "C:\\path\\to\\notes-mcp.exe"
args = []
```

**ChatGPT** (and Claude on the web or phone) needs a public HTTPS address, which the hub provides: see the hub connector sections above. The desktop has no endpoint of its own to put a tunnel in front of (removed 2026-10-23). Not verified: ChatGPT's connector form offers OAuth or no authentication and may not send a static bearer token, in which case the hub's `/mcp` answers 401.

### AI tasks

- **Entry points.** "Ask AI" is in the `/` menu, the selection toolbar, the block menu, the phone block sheet and the page menu (next to Move to). From the selection toolbar, the block menu or the sheet, the prompt quotes the selected text (or the block's text) under "About this selected text:", and the answer goes after the last selected block. From the page menu, the answer goes at the page's end. The agent chat (Ask agent) is unchanged.
- **Where it runs.** Ask AI and tasks assigned to Agent run on the synced default runner: the vault's "Run AI on", then the built-in default (this computer, or the Hub on Android), then a reachable fallback. Phones always use the Hub and show "The agent runs on your hub (phones always use it)". Signed out, the entries stay visible and say to sign in to the hub in Settings > Sync and devices.
  - **This computer:** the app writes a row pinned to this device into the AI tasks collection (its `node` is this device's id). This desktop's agent runs it with the synced model and tool set of `this_device` (default Everything). AI task runs never wait for tool approval: tasks without a worktree deny Confirm-level tools, while code-task worktrees allow them. Without a worktree, file writes and shell commands are unavailable; read-only tools, vault tools and `report_progress` still work within the runner's tool set. Assign the task with a project to enable file writes and shell commands in its worktree.
  - **The Hub:** the app flushes its edits and runs one sync round, so the hub has the page, then creates a hub job about the page. If the `after` block is not on the hub, the answer goes at the page's end.
- **Tasks.** Each run is a row of a top-level "AI tasks" collection (marker property `ai.tasks`), with Status (queued, running, done, failed, cancelled, interrupted), Prompt, Page, Runner, Ran on, Error, Progress (`progress`, text), Progress at (`progress_at`, timestamp stored as text Unix epoch milliseconds), Block and Created, so it syncs like any page. Older databases gain the progress properties through the existing additive migration. The page gets a grey ✨ callout placeholder ("AI task queued: …"); an empty paragraph where the user asked becomes it. Each new row is pinned at creation to this device's id or `hub`, after resolving the default runner. Desktop runs save their BubblePaw session id in the row's `session` property before starting. Cancelling marks the row `cancelled`; its desktop runner observes that status, including through sync, and cancels the engine session. A late answer is dropped. Desktop and hub runs share a 1 hour limit: the engine is cancelled and the row becomes `failed` with a limit message.
- **Progress.** Only delegated task agents (desktop and hub) get `report_progress({message: string})`, or `tools.report_progress({message})` inside `js`. The runner supplies the task row; the tool accepts no row id. Messages are trimmed, capped at 500 Unicode characters and rejected if empty. Each report replaces `progress` and `progress_at` on the running row; reports after cancellation or another terminal state return an error without changing the row. The latest report syncs through normal vault paths and remains after done, failed, cancelled or interrupted, alongside the existing result or error. There is no history, relay or new event. Interactive chats do not get the tool.
- **Subtasks.** Delegated desktop and hub agents plan multi-step work with BubblePaw's `todo` host tool, passing the whole list in an object: `{"items":[{"text":"Read the references","status":"in_progress"}]}` (or `await tools.todo({items: [...]})` inside `js`). Pass `{"items":[]}` to clear the list. The tool stores the bare array as `todo` host state, and Scratch mirrors the list into nested to-dos under the assigned source to-do, or under the task's placeholder callout for page and card assignments. The AI tasks schema gains `source_block` and `subtask_blocks` through an additive migration. `done` checks a child; `pending` and `in_progress` leave it unchecked. Text changes, order, additions and removals reconcile only blocks that attempt created. User-written blocks stay intact; a removed mirrored item with user-written children is retained with its subtree and no longer tracked. Deleting a mirrored child suppresses it while its item remains in the list. Checking every child never completes the parent. Writes sync normally; a mirror error is logged without failing the run. A placeholder with children remains, with the final answer after it. Retry removes the previous list only if all surviving mirrored children are unchecked; otherwise it keeps that list and starts a fresh one below. Chats and the MCP gateway do not get `todo`. See [subtask mirroring](agent.md#subtasks-from-the-delegated-task-list).
- **Attempts (2026-10-30).** A task has one active attempt. Each attempt is a compact record in the row's `attempts` property (JSON, newest last, at most 20 attempts and 60 events each): attempt id, parent attempt, session, actor, source revision, the brief the worker was given, model, usage (unknown stays unknown), and ordered events (started, progress checkpoints, steering sent and acknowledged, update requested and sent, cancel requested and acknowledged, ended). `attempt` names the current one. Only the runner pinned to the task writes `attempts`; cancel and update requests use their own properties, so they do not overwrite it. The source revision is a hash of the page's text without the run's own blocks. Code: [crates/mcp/src/tasks/attempts.rs](../../crates/mcp/src/tasks/attempts.rs).
- **Cancel is a request (2026-10-30).** `tasks::cancel` records `cancel_requested` and keeps the status `running`; the card says "cancellation requested". The runner acknowledges by stopping the engine and finishing the attempt, which sets `cancelled`. A queued task cancels at once. Retry is refused while a cancel is pending or the task is running.
- **Late results.** A result from an older attempt, or from a run that was cancelled or retried, goes into that attempt's history. It never replaces a newer attempt, a cancellation, or a later decision on the task.
- **Steering (2026-10-30).** Explicit, two ways. (1) A to-do the user adds under the running task's block (`tasks::add_step`, bridge `ai_add_step`) is put on the worker's `todo` host state and sent to it as a steering message; the history shows it sent and acknowledged, and the subtask mirror adopts the user's to-do instead of adding a copy. To-dos that were under the task before the run started are not steering. (2) Editing the note changes its source revision but sends nothing; the worker keeps its brief and the card says "Running on an earlier version of the note". **Send note update to agent** (`ai_send_update`) asks the runner to send the worker the note's current content.
- **Retention.** The task row, its outcome, reviews and attempt summaries are kept. The raw session (transcript and logs) of an ended hub job is deleted after 30 days and the row's `session` is cleared; the row is no longer trashed.
- **Review and Accept (2026-10-30).** A task assigned from the kanban (`runners::assign` sets the `review` property) ends in the status Review instead of Done. Accept (`tasks::accept`, bridge `ai_accept_task`) sets it to Done. Ask AI, the CLI and hub jobs created through the `jobs` tool have no `review` property and still end in Done; the jobs API reports Review as completed.
- **Heartbeat and stalled runs (2026-10-30).** The runner writes `heartbeat` (Unix ms) on the running row every 60 s. If it is more than 10 minutes old, other devices show "stalled (no contact since HH:MM)" and offer Mark failed (`tasks::mark_failed`, bridge `ai_mark_task_failed`). Nothing fails on its own; the PC that owns a stale running row fails it at startup (`recover`).
- **Desktop log expiry (2026-10-30).** The raw session of an ended desktop task is deleted after 30 days, once per app run when the first task starts. The row, outcome and attempts stay.
- **Asking the user (2026-10-30).** Delegated desktop and hub runs get `ask_user({question})`. `tasks::ask` stores the question in the row's `question` property and in the attempt history (`question` event); the task stays `running`, and the card shows the question with a Reply field (bridge `ai_reply_task`, `tasks::reply`). The reply is stored in `reply`, the question is cleared (the task is plain running again, `reply` event), and the waiting tool returns "The user replied: ..." to the worker. The run's time limit bounds the wait: a run that ends while it waits closes its attempt with the outcome "waiting for input" and the row becomes `interrupted` with the error "waiting for input: no reply within the run limit", which is not a failure. The card still shows the question with its Reply field: a reply to such a row starts a new attempt on the same runner (`tasks::reply` calls `retry`), with the question and the reply added to the task's prompt. A question raises a "Task needs your reply" notice (`runNoticeOf`). The tool waits by polling the row, not through the steering channel.
- **Code tasks on a PC.** Creation and `ai_assign_task` accept an optional `project_id`, stored in the row's text property `project`. The project is an existing agent project on the pinned PC, and its path is the repository. Each PC's hidden Nodes record and the runner list publish only project ids and names, never local paths. A nonempty id must be published by that PC; unknown project ids and any project on `hub` are rejected. Empty means an ordinary notes task. The picker UI does not yet expose this choice.
- **Code worktrees.** Before starting the provider, the runner creates a BubblePaw worktree from the repository's `HEAD`, with branch `bp/<session-id>` at `<app-data>/agent/agent.data/worktrees/<session-id>`, and writes the row's `worktree_branch` and `worktree_path` text properties. Uncommitted source changes are not included. File tools are rooted in the worktree, shell commands default to it, and project switching cannot redirect the session. Confirm-level tools such as write, edit and exec run without approval in this unattended worktree run. Desktop tasks without a worktree deny Confirm-level tools; chat settings are unchanged. Shell access is not a sandbox. Desktop tools must be enabled on that PC, and the project must be a git repository with at least one commit; otherwise the task fails clearly without falling back to the source folder. The agent data directory must be outside that repository. There is no cleanup: worktrees and branches stay for review and merge after the run. Retry keeps the row and creates a fresh session, branch and worktree, preserving earlier attempts. See [code-task execution](agent.md#code-tasks-on-a-pc).
- **Triage (built 2026-10-30).** A user-started task reviews one project's pages (a top-level page and its subtree, or a given page list) and proposes merges of duplicates; it is an ordinary row of this database whose `triage` property holds the scope, run on a PC or the hub like `ai_assign_task`. Only such runs get the `triage_pages` and `triage_propose` tools. Proposals are stored in the hidden Nodes page, accepted (one `triage:merge` history commit, the newer page goes to the trash) or dismissed (never proposed again), and undone through page history. See [Triage](../core/triage.md).
- Jobs that outside agents start through the `jobs` tool are rows of this same database: they add the `node`, `owner`, `tools`, `session` and `finished` properties, and only the hub claims them ([Jobs](../sync/hub.md#jobs)). Hub jobs the app creates are rows too, owned by the creating device.
- A desktop (Windows, Linux) with the agent on claims the queued tasks pinned to it or legacy rows with no device (`node` empty), one at a time, runs it with the agent and writes the answer (markdown) in place of the placeholder. If the agent edited the vault itself and answered nothing, the placeholder is removed. A failure leaves a red callout with the error; Settings > Agent > Recent tasks lists them with their status and Retry. Tasks this device left running are failed at the next start. [scratch-cli](scratch-cli.md) runs the same loop without the app's UI, for a vault the app does not have open.
- **Removed runners (2026-10-22):** Ask AI used to offer Claude Code and Codex (started as programs with the app's MCP endpoint) and the DeepSeek API as runners. They are gone, with their settings (program paths, the DeepSeek key, model and address, the run timeout); their keys in an old `ai_settings.json` are ignored and dropped on the next save. A row that names one of them (Runner `claude`, `codex` or `deepseek`) is answered by the agent like any other.
- The prompt holds the task, the page's markdown (up to 20,000 characters) and its id.
- **The agent answers Ask AI (built 2026-10-08; the only runner since 2026-10-22):** Settings > Agent has one line that follows "Run AI on": "Ask AI runs on the hub." (every Ask AI is a hub job, `AiStore.ask`) or "Ask AI runs on this computer." (a row this computer's agent answers while "Use the agent" is on). There is no "Answered by" choice and no Off: turning the agent off stops this computer from answering. Tasks run through `ai_run_task_with_agent` in their own agent session, which writes the answer or error into the task. When the agent cannot open, the task fails with the reason.
- **Agent settings** (Settings > Agent, the first page of the AI group; reworked 2026-10-22, see [Agent › Settings](agent.md)): "Use the agent" (`agentEnabled`, default on), then "Run AI on: Hub | This computer" (synced). With Hub: the hub's model (listed by the hub, "Hub default (<model>)" first; the last list is kept for when the hub can't be reached) and the hub's tool set. With This computer: this computer's model, its tool set, Computer access and projects, the MCP servers, and Claude Code's bun program and extension folder (`agentBunPath`, `agentClaudeCodeDir`, empty by default; the folder is BubblePaw's `extensions/claude-code`). There is no per-device override and no model visibility list any more; permission modes and `desktopRoot` have no control. A change reopens the agent after 600 ms. On Android the Agent section is the Hub's ([mobile_agent_settings.dart](../../app/lib/addons/agent/mobile_agent_settings.dart)): the Hub address (kept through sign-out), Sign in or Sign out, the account (an older Hub that reports none shows the device name it registered), the connection line, and once signed in the hub's model and tool set. "Run AI on" is at the top; with This computer chosen it says "Phones use the hub". Signed out, the Agent chat shows a Sign in action and no composer; New chat is in its top bar and is off while signed out.

### Tasks assigned to Agent

- Assigning a task to "Agent" starts one run of the agent for that task on the synced default runner: a hub job, or a run on this computer. In the task composer this happens when the task is created; on the task page, when it is saved. The prompt starts "Do this task: <title>" and says that the task's notes are the details. There is no plan step.
- The run id is stored on the task (type data key `agent_run`). The task shows the run's status under its assignee: starting, queued, working, finished (the answer is on the page), failed, cancelled or interrupted.
- Reassigning to Agent later never starts another run. Changing the assignee from Agent to someone else cancels the run if it has not ended.
- Changing the assignee through the MCP `update_task` tool does not start a run; only the app does.

### Agent merge helper and interim block review

- **Approved change, 2026-10-13/14; core/sync backend built and checked 2026-10-14:** the v2 conflict backend only accepts `Keep` and `UseSide(i)` and the new [merge screen](../../design/merge/index.html) has no AI button. Delete versus edit is settled by the entire editing version at detection; AI is not part of that backend rule. Merge-helper and Flutter bridge adaptations are implemented, with regenerated FRB 2.13.0 bindings; Rust checks pass; native and targeted Flutter checks also pass.
- The MCP agent merge helper remains in [merge.rs](../../crates/mcp/src/merge.rs). Its instruction is "The edited version wins; merge only text that both sides edited", and descendants must stay intact. Markdown materialization retains children; `MergeProposal.blocks` preserves the complete parsed tree for agents. The text/marks overlay used by the old Flutter block action edits only the existing block's own text and leaves its children in place.
- The old Flutter block review and its "Resolve with AI" action are gone; the app has no AI merge. The store's last piece of it, a merge proposal from the removed runners, was deleted on 2026-10-22. The approved v2 screen is a separate later build; see [App state](../app/app-state.md#sync-and-settings).

### Limits

- Two desktops with the agent on can both claim a task before they sync; both answers then land in the page.
- If the app starts while `notes-mcp` holds the vault for one call, the app opens it as a second instance; calls are short, but this is not prevented.
