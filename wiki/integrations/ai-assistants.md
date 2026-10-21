---
type: topic
layer: vision
summary: "Claude and ChatGPT use the vault over MCP; tasks can be assigned to AI agents that run on the desktop via Claude Code, Codex or an API."
parent: "[[integrations/index]]"
status: current
---

# AI assistants

**Status:** a first version is built; see [Current state](#current-state-2026-10-01). Plans, the agent runner proposal and open questions are in the internal wiki (notes-app > AI assistants).

- 2026-09-30: tasks for AI agents run on the user's desktop with their own Claude Code or Codex subscription, or an API such as DeepSeek, not a cloud service.

## Assistants using the vault (MCP)

- The hub's `POST /mcp` is the primary agent transport. It serves the vault's 15 listed tools on the hub's shared replica, using named, revocable read or write agent tokens. [Hub](../architecture/hub.md#agent-tokens-and-mcp-mcp) describes token management, scopes and history attribution. The desktop endpoint is the offline fallback.
- The separate, opt-in `/mcp/agent` gateway exposes the vault tools, the embedded agent's advertised host tools and `hosts`. It uses OAuth only; see [Agent gateway connector](#agent-gateway-connector). `/mcp` retains its existing vault tool interface.
- Page links for the user: `read_page`, `create_page` and `edit_page` results carry a `link:` line (and a `block link:` line for a block), an http address `<base>/open/<id>` (`?block=<id>` for a block). `<base>` is the serving hub's `public_url`, or `http://127.0.0.1:<port>` without one (47100 on the desktop). The Claude app refuses custom-scheme links from chat but opens http ones, so the address bounces to `notes://page/<id>`. The server instructions tell the agent to write `[Title](<link>)` with the `link:` line. See [links](../../docs/notes/links.md) and [app state](../architecture/app-state.md).
- **Claude:** Claude Code and Claude Desktop can connect locally. Claude on the web and phone uses custom connectors, which need a public HTTPS address.
- **ChatGPT:** supports MCP connectors too, with the same public-address requirement.
- Browser connectors that require OAuth: Claude's connector uses Authentik through an optional OAuth path on the hub's `/mcp`, described in [Claude connector](#claude-connector). Otherwise the hub endpoint accepts static agent bearer tokens and rejects every browser `Origin` header.

## Claude connector

**Status:** off by default (the `[mcp_oauth]` settings unset). Not yet verified with a real claude.ai sign-in. Decision 2026-10-16 (0011, internal wiki): Authentik is the authorization server for the connector, with a public client `claude-mcp`; the hub only validates its JWTs offline.

Claude on the web and phone reaches the vault through a custom connector. The hub's `/mcp` accepts an OAuth token from Authentik for this, next to the agent tokens.

### How it works

- `/mcp` accepts two kinds of bearer token. First the agent tokens, unchanged. Otherwise an OAuth access token from the Authentik provider `claude-mcp`.
- The hub validates that token offline as a JWT: RS256 signature against the issuer's JWKS, `iss` equal to the configured issuer, `exp` not passed, and `aud` containing the configured client ID. An additional resource audience may be `<public_url>/mcp`, `<public_url>/mcp/agent` or `<public_url>/mcp/agent?<query>` on either endpoint. The configured client ID remains required: a resource audience alone is refused. No resource indicator is required.
- Discovery and keys are cached for 10 minutes, as app login is. A signing-key rotation in Authentik is picked up within 10 minutes.
- The allow-list is the same as app login: `oidc.allowed_users` by email or `sub`, and `oidc.allowed_groups`, in practice `notes-users`. If the token lacks the `groups` claim (or the email, when users are listed), the hub asks Authentik's userinfo endpoint once per token and caches the answer until the token expires.
- Allowed callers get the same tools as a full-access agent token. Their edits are recorded with actor `oauth:<email>`, or `oauth:<sub>` without an email.
- Without a valid token, `/mcp` answers 401 with `WWW-Authenticate: Bearer resource_metadata="https://notes.example.com/.well-known/oauth-protected-resource/mcp"`. A valid token from an account not on the allow-list gets 403.
- `GET /.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp` serve the protected-resource metadata (RFC 9728). It names resource `https://notes.example.com/mcp`, the claude-mcp issuer as authorization server, the scopes `openid email profile offline_access`, and bearer tokens in the header only.
- Origin: a request without `Origin` passes. With the feature on, a single `Origin` of `https://claude.ai`, `https://claude.com` or `https://chatgpt.com` passes; other or multiple Origin values get 403. Origin logging is bounded and uses debug level.
- With both variables unset, the hub behaves as before: no metadata routes, a bare `Bearer` 401, and every Origin refused.

The two variables are listed in [Hub deployment](../architecture/hub-deploy.md#claude-connector).

### Authentik side (exists since 2026-10-16)

Created over root SSH with `ak shell`. Change Authentik infrastructure the same way, never through the admin web UI.

- Provider `claude-mcp`: OAuth2/OpenID Connect, public client (no secret), PKCE S256.
  - Grant types `authorization_code` and `refresh_token`, set explicitly. An empty list rejects every login; see [the live deployment note](../architecture/hub-deploy.md#live-deployment).
  - Strict redirect URIs `https://claude.ai/api/mcp/auth_callback` and `https://claude.com/api/mcp/auth_callback`.
  - Signing key (RSA, so tokens are RS256) and flows copied from `notes-hub`.
  - Scopes `openid`, `email`, `profile`, `offline_access`.
- Application `claude-mcp`, bound to the group `notes-users`.
- Client ID: `d1QDPw9lEM6jOT2WFfcJkggcMqLuA6BrEIvvWF4b` (public, not a secret).
- Issuer: `https://auth.example.com/application/o/claude-mcp/`. JWKS: `https://auth.example.com/application/o/claude-mcp/jwks/`. Discovery is public and advertises S256.

### Connector settings in Claude

In Claude, go to **Settings → Connectors → Add custom connector**:

- Name: `Notes` (any name).
- URL: `https://notes.example.com/mcp`.
- Advanced settings: OAuth Client ID is the `claude-mcp` Client ID. Leave the OAuth Client Secret blank; the client is public.

Then choose **Connect**. Claude opens Authentik's login. After sign-in, the vault tools appear in chats.

### Limits

- No dynamic client registration. Authentik gains it in 2026.8, so until then the client ID is entered by hand.
- Authentik serves its metadata only at `<issuer>.well-known/openid-configuration`, not at the RFC 8414 path. The MCP spec has clients try that URL last, so discovery should still work. This is unverified with Claude until the first real sign-in (internal wiki: AI assistants).
- Only the claude.ai and claude.com callbacks are registered. Claude Code's local loopback callback is not, so Claude Code keeps using agent tokens.
- Streamable HTTP over POST only. `GET /mcp` stays 405 (no SSE stream).
- OAuth sessions get full access. There is no read-only OAuth scope.

## Agent gateway connector

**Implemented, off by default; deployment and real connector sign-in are not established here.** Add a separate connector for `https://notes.example.com/mcp/agent` to use the agent runtime. It offers all `/mcp` vault tools, the host's advertised tools and `hosts`. Host tools accept optional `host`; JS and `js_help` appear only when enabled. A connector URL can select a host, for example `https://notes.example.com/mcp/agent?host=hub`. Only the hub host is available; desktop outbound relay is not implemented, and the hub has no desktop tool pack or shell.

Use the same public Authentik `claude-mcp` client as the `/mcp` connector: OAuth Client ID `d1QDPw9lEM6jOT2WFfcJkggcMqLuA6BrEIvvWF4b`, with the client secret blank. The issuer is `https://auth.example.com/application/o/claude-mcp/`. The existing registered callbacks above still apply; allowing the ChatGPT Origin does not register a ChatGPT OAuth callback or establish a successful ChatGPT connection.

Enable `NOTES_HUB_MCP_AGENT_ENABLED=true` alongside `NOTES_HUB_MCP_OAUTH_ISSUER` and `NOTES_HUB_MCP_OAUTH_CLIENT_ID`; enabling the gateway without both OAuth settings is a configuration error. `NOTES_HUB_MCP_AGENT_DEFAULT_HOST` defaults to `hub`. These also have TOML keys `mcp_agent_enabled`, `mcp_agent_default_host` and `[mcp_oauth]` `issuer`/`client_id`.

Host selection is per call: tool argument `host` wins, followed by the request URL's `?host=name`, the global `NOTES_HUB_MCP_AGENT_DEFAULT_HOST`, then `hub`. Unknown hosts from URL or configuration cause a call error listing available hosts; they do not fall back silently.

The gateway accepts OAuth bearer tokens only. It shares `/mcp`'s issuer, RS256/expiry/audience validation and app-login allow-list (`oidc.allowed_users` or `oidc.allowed_groups`, in practice `notes-users`). The JWT `aud` must contain the configured client ID. Additional resource audiences `<public_url>/mcp`, `<public_url>/mcp/agent` and `<public_url>/mcp/agent?<query>` are accepted on both endpoints; a resource audience alone is refused. Requests without Origin pass; single Origins `https://claude.ai`, `https://claude.com` and `https://chatgpt.com` pass, while other or multiple Origins are refused. Missing/invalid credentials return 401 with metadata at `https://notes.example.com/.well-known/oauth-protected-resource/mcp/agent`; an account outside the allow-list gets 403. That metadata advertises resource `https://notes.example.com/mcp/agent` and the same issuer and scopes.

Discovery preserves URL host selection: a connector at `/mcp/agent?host=hub` receives a 401 challenge pointing to `/.well-known/oauth-protected-resource/mcp/agent?host=hub`, whose resource includes `/mcp/agent?host=hub`. The query selects a host; the shared verifier accepts the query-bearing resource audience on either endpoint, while still requiring the configured client ID in `aud`.

`tools/list` returns the same vault definitions and schemas as `/mcp`, plus `hosts` and the selected host's model-facing tools. Vault calls use the `/mcp` implementation, including imports and write attribution, and never take `host`. The gateway omits the host's duplicate vault adapters using the vault registry; unrelated name collisions fail discovery. Native host tools receive an optional `host` argument and execute through BubblePaw's single-tool API without invoking a model.

The hub defaults to `[agent] execution_mode = "native"` and `enable_js = false`. Set `enable_js = true` to also offer `js` and `js_help`; `execution_mode = "js"` selects the legacy mode. Environment overrides are `NOTES_HUB_AGENT_EXECUTION_MODE` and `NOTES_HUB_AGENT_ENABLE_JS`. The local in-app agent keeps its JS mode. Only tools the host actually registers are available: the hub has no desktop filesystem or process pack.

Call `hosts` to get `[{name, online, default}]`, where `default` marks the host selected by this connection or the optional argument. Only `hub` is currently registered; unavailable hosts report `online: false`. When JS is advertised, `js_help` lists its capabilities and full schemas, and `query: "js"` describes the runtime. `tools.hosts()` remains available inside JS. Tools requiring interactive confirmation are omitted and refused, including nested JS calls. `history_*`, web tools, `ui_*`, `todo`, `question` and `reload` remain excluded.

HTTP initialize returns an OAuth-subject-bound `Mcp-Session-Id`. Send it with subsequent host tool calls. When JS is enabled, its environments persist within that session and remain isolated from other sessions. `DELETE` with that header closes the session and environments. Idle sessions expire after 24 hours and are reclaimed on a later initialize; after a hub restart, initialize again. This endpoint accepts POST and DELETE, with JSON replies and no SSE stream. More detail: [Agent gateway](agent.md#agent-mcp-gateway-mcpagent).

### Switch an existing connector

Edit the connector URL from `https://notes.example.com/mcp` to `https://notes.example.com/mcp/agent?host=hub`, or back. Keep the same OAuth Client ID and login. Both routes use the same issuer, accepted audiences, allow-list, scopes (`openid email profile offline_access`) and registered redirect URIs. The server accepts the same valid OAuth token on either route, without binding it to the resource URL advertised in metadata. Static agent tokens remain specific to `/mcp`.

## Current state (2026-10-01)

Built in the original implementation: an MCP server for the vault, AI tasks run by desktop agents, and merge proposals in the existing block-conflict review. The v2 core/sync conflict backend is built and checked, and FRB 2.13.0 bindings are regenerated; 12 MCP tests and 21 Rust bridge tests pass; native and targeted Flutter checks also pass. Its approved new screen excludes AI, while agent helpers and interim block controls remain as described below. Code: [crates/mcp](../../crates/mcp) (`vault_mcp`, binary `notes-mcp`), the bridge [app/rust/src/api/ai.rs](../../app/rust/src/api/ai.rs) and [app/lib/features/ai](../../app/lib/features/ai).

### MCP server

- Tool definitions and implementations live in [crates/tools](../../crates/tools) (`vault_tools`), with read-only, destructive and approval metadata shared by MCP and the coming BubblePaw agent. `vault_mcp` adapts them to MCP without changing their schemas or responses. [crates/agent](../../crates/agent) (`vault_agent`, desktop only) hosts them as BubblePaw tools inside `js` and adds the `agent` task runner; see [Agent](agent.md#built-2026-10-08-to-2026-10-09).
- Listed tools (15, in serving order; the source is the registry in [crates/tools/src/tools.rs](../../crates/tools/src/tools.rs)):
  - Pages: `search_notes` (full-text search, rg-style results with block ids; without a query it lists the page tree; `backlinks_of` lists the pages linking to a page; `is:`, `done:`, `assignee:` and `calendar:` return to-do, task-note, dated-row and date-mention items; `in:memory` searches agent memory; `context:`, `full:`, `limit:` and `sort:` size and order the output), `read_page` (markdown; `block_ids`; `include_backlinks`; an unknown id or title returns a labelled list of pages whose titles contain those words, never a near match), `create_page` (a page or database row), `edit_page` (one op, or an `ops` list: `append`, `replace`, `delete`, `set_body`, `set_props`), `trash_page`, `move_page` (`pages`, with `parent` or `after`; moves pages with their sub-pages, refuses a move into the page's own subtree, and a move into or out of a database is a plain move).
  - Planning: `daily_note`, `update_task` (due, done or both; item references come from `search_notes` item lines).
  - Databases: `upsert_row`. Rows are read with `search_notes` and `in:[[Database]] -type:database`.
  - Memory: `memory_read`, `memory_write`, `memory_update` and `memory_delete` below; memories are found with `search_notes` and `in:memory`.
  - Feedback: `feedback_submit` (served to MCP and the in-app agent).
  - Help: `notes_help`.
  - Pages are named by id or exact title. Every write goes through `vault_core`.
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
- `notes_help(topic?)` returns the table of contents, or one topic: `markdown`, `links`, `properties`, `day-notes`, `tasks`, `query`, `templates`, `system-pages`, `memory`, or `tools` (every tool with its arguments, generated from the registry). The topic pages are markdown in `docs/notes/`, compiled into the binary. The MCP server instructions point agents to it.
- Memory tools: `memory_read`, `memory_write`, `memory_update` and `memory_delete`, addressed by path (`global/<category>/<memory>`; older `project/[subproject/]memory` paths still resolve) and never by page id. The path is the memory's identifier everywhere; `name` is only a display label. They use ordinary nested databases under `Memory`; see [Agent memory](agent-memory.md#memory-tools-current-2026-10-14) for arguments, schema and path rules.
- Feedback: `feedback_submit` (summary, optional Markdown details, kind, page_id, tool, error). `feedback_list` (limit and optional kind filter) is callable over MCP but not listed. Use them for unexpected vault failures, user complaints about Scratch/sync/the vault, or an explicit request to report; submit once per issue per session. They share the app's hidden Feedback page; see [Feedback](../architecture/feedback.md#feedback-over-mcp). All 15 listed tools share the same MCP transports.
- Primary transport: the hub's Streamable HTTP `POST /mcp`, with JSON replies, empty 202 notifications and no sessions or SSE. Agent tokens are separate from device tokens, scoped to read (the 4 read-only listed tools `search_notes`, `read_page`, `memory_read` and `notes_help`; `feedback_list` and the read-only aliases are callable too) or write (all 15 listed tools, including `feedback_submit`), and writes appear in history as `agent:<token-name>`. Revocation is visible without restarting the hub. Every `Origin` header gets 403 unless the Claude connector option is on, which also passes `claude.ai`, `claude.com` and `chatgpt.com`; no CORS headers are sent.
- Offline fallback: desktop stdio and Streamable HTTP. Desktop HTTP needs `Authorization: Bearer <token>`, binds 127.0.0.1 by default and rejects browser `Origin`s other than localhost unless explicitly allowed. Its token remains `<vault>/.vault/mcp/token` (created on first use; it does not sync).
- **Decision (2026-10-01): co-running with the app.** The vault has a single owner (`.vault/lock`), so two programs must not open it at once.
  - While the app runs with "Serve the vault to AI apps" on (Settings > AI), the app serves MCP itself on `127.0.0.1:47101` and writes the address to `.vault/mcp/endpoint.json`. `notes-mcp` over stdio forwards every tool call there.
  - When the app is not running, `notes-mcp` (stdio) opens the vault for each tool call and closes it again, so the lock is held only during a call. If the app holds the vault but does not serve MCP, the call fails with "turn on Settings > AI > MCP endpoint".
  - `notes-mcp --http` holds the vault open; it refuses to start while the app has it.
  - A stale `endpoint.json` (app crashed) is ignored when the address does not answer.
  - Why: one writer per vault without a second sync peer, no daemon, and stdio clients (Claude desktop, Claude Code) work whether or not the app is open.

### Connecting AI apps

For an agent with HTTP support, create a hub token with `notes-hub --config <file> --agent-token add <name> --scope read|write` and configure `https://<hub>/mcp` with `Authorization: Bearer <token>`. Keep the plaintext in the client's secret configuration. The hub binary's `--agent-token revoke <name>` removes access while the server runs. These are local implementation and integration-test guarantees; they do not establish that a live hub has been deployed with this endpoint.

### Shell client (`notes`)

The `notes` binary in `vault_tools` calls the same hub `/mcp` endpoint. It provides `search`, `read`, `query`, `write` and `append`, plus nine `memory` commands (`list`, `search`, `read`, `write`, `update`, `delete`, `index`, `import` and `rules`; `rules` is answered locally, without hub access) and `wiki import`. Write and append bodies come from stdin; `--json` returns the raw tool result. It does not open a local vault.

`NOTES_HUB_URL` overrides the config's `hub`, then defaults to `https://notes.example.com`. `NOTES_TOKEN` overrides the config's `token`. The config is `%APPDATA%\notes\config.toml` on Windows, or `$XDG_CONFIG_HOME/notes/config.toml` (otherwise `~/.config/notes/config.toml`) on Unix. Tokens are never command-line arguments or printed in diagnostics. See [Agent memory](agent-memory.md) for the index and import rules.

### Desktop connection examples

The desktop examples below are the offline fallback.

Build the binary with `cargo build --release -p vault_mcp` (`target/release/notes-mcp.exe`). It finds the app's vault by itself (`NOTES_VAULT`, else `vaultPath` in the app's `settings.json`, else `%LOCALAPPDATA%\Notes\vault` or `~/.local/share/notes_app/vault`); `--vault DIR` overrides. `notes-mcp --token` prints the HTTP token.

**Claude Code**, stdio (works with or without the app):

```
claude mcp add notes -- C:\path\to\notes-mcp.exe
```

or HTTP to the running app (Settings > AI > "Copy Claude Code command" fills in the token):

```
claude mcp add --transport http notes http://127.0.0.1:47101/mcp --header "Authorization: Bearer <token>"
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

**ChatGPT** (and Claude on the web or phone) needs a public HTTPS address: put a tunnel in front of the app's endpoint, for example `tailscale funnel 47101` or `cloudflared tunnel --url http://127.0.0.1:47101`, and add `https://<tunnel>/mcp` as a connector (ChatGPT: Settings > Connectors, developer mode). Not verified: ChatGPT's connector form offers OAuth or no authentication and may not send a static bearer token, in which case the endpoint answers 401. A tunnel with its own access control (Cloudflare Access) or an OAuth front end is then needed; neither is built.

### AI tasks

- "Ask AI" in the `/` menu (any device) asks for a prompt and queues a task. Tasks are rows of a top-level "AI tasks" collection (marker property `ai.tasks`), with Status (queued, running, done, failed), Prompt, Page, Runner, Ran on, Error, Block and Created, so they sync like any page. The page gets a grey ✨ callout placeholder ("AI task queued: …"); an empty paragraph where the user asked becomes it.
- A desktop (Windows, Linux) whose Settings > AI names a runner claims queued tasks one at a time, runs the agent and writes the answer (markdown) in place of the placeholder. If the agent edited the vault itself and answered nothing, the placeholder is removed. A failure leaves a red callout with the error; Settings > AI lists recent tasks with their status and Retry. Tasks this device left running are failed at the next start.
- Runners: **Claude Code** (`claude -p --mcp-config <file> --strict-mcp-config --allowedTools mcp__notes --output-format text`) and **Codex** (`codex exec --skip-git-repo-check -c mcp_servers.notes.url=… -c mcp_servers.notes.bearer_token_env_var=NOTES_MCP_TOKEN --output-last-message <file> -`). Each runs from an argument list with the prompt on stdin (never a shell line), without a console window, with a 10-minute timeout, and the MCP config file is deleted afterwards. They get the app's MCP endpoint (started on a free local port when "Serve the vault" is off). **DeepSeek** is called over HTTPS from Dart (`/chat/completions`; the key stays in `ai_settings.json` on the device) and sees only the page's text.
- The prompt holds the task, the page's markdown (up to 20,000 characters) and its id.
- **Agent runner (built 2026-10-08):** Settings > AI > Runner has "Agent", the in-app BubblePaw agent ([Agent](agent.md)). It is the default when the agent is on and no runner was chosen before; a runner chosen earlier is kept (`runner` is written to `ai_settings.json` only once chosen). Its tasks run through `ai_run_task_with_agent` in their own agent session, which writes the answer or error into the task. When the agent cannot open, the task fails with the reason. Claude Code, Codex and DeepSeek are unchanged.
- **Agent settings** (Settings > AI > Agent, Windows and Linux, in `ai_settings.json`): "Use the agent" (`agentEnabled`, default on), the default model (`agentModel`; a select from the models of the default host, "Agent runs on", read from that host even while the AI tab is on the other one; when that host can't list them, the stored value and a short hint, and the setting is left as it is; empty for the built-in default), Claude Code's bun program and extension folder (`agentBunPath`, `agentClaudeCodeDir`, empty by default; the folder is BubblePaw's `extensions/claude-code`), and approvals Ask or Allow (`agentApprovals`). A change reopens the agent after 600 ms. On Android the Agent section is the Hub's ([mobile_agent_settings.dart](../../app/lib/addons/agent/mobile_agent_settings.dart)): the Hub address (kept through sign-out), Sign in or Sign out, the account (an older Hub that reports none shows the device name it registered), the connection line and the default model when the Hub lists models. Signed out, the Agent chat shows a Sign in action and no composer; New chat is in its top bar and is off while signed out.

### Agent merge helper and interim block review

- **Approved change, 2026-10-13/14; core/sync backend built and checked 2026-10-14:** the v2 conflict backend only accepts `Keep` and `UseSide(i)` and the new [merge screen](../../design/merge/index.html) has no AI button. Delete versus edit is settled by the entire editing version at detection; AI is not part of that backend rule. Merge-helper and Flutter bridge adaptations are implemented, with regenerated FRB 2.13.0 bindings; Rust checks pass; native and targeted Flutter checks also pass.
- The MCP agent merge helper remains in [merge.rs](../../crates/mcp/src/merge.rs). Its instruction is "The edited version wins; merge only text that both sides edited", and descendants must stay intact. Markdown materialization retains children; `MergeProposal.blocks` preserves the complete parsed tree for agents. The text/marks overlay used by the old Flutter block action edits only the existing block's own text and leaves its children in place.
- The existing Flutter block review temporarily retains "Resolve with AI" and its Accept/Reject keys for compatibility. It sends the block versions to a configured runner, hides the action with the Agent runner (which has no one-shot answer), and changes nothing on Reject. Accept makes a normal `SetText` page edit followed by `Keep`, not a `Replace` conflict resolution. New subtree/page review does not use this action. The approved v2 screen is a separate later build; see [App state](../architecture/app-state.md#sync-and-settings).

### Limits

- Real Claude Code and Codex were not run (tests use a fake agent); their flags come from their documentation and may need adjusting. On Windows a `.cmd` shim (npm installs) gets its arguments through cmd.exe quoting, which was not tested; a timeout kills the shim but maybe not its child.
- Two desktops with runners can both claim a task before they sync; both answers then land in the page.
- The app stops its MCP endpoint only at exit, not when the vault closes.
- If the app starts while `notes-mcp` holds the vault for one call, the app opens it as a second instance; calls are short, but this is not prevented.
- The merge proposal preview shows plain text; its marks are applied but not shown.
