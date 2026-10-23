---
type: index
summary: "Decisions that shape what's built, one line each with its date."
parent: "[[index]]"
status: current
---

# Decisions

Decisions that shape what's built, one line each with its date. The full records (context, reasons, consequences) are in the internal wiki in the notes vault (notes-app > Decisions).

- 2026-09-30, 0001: First platforms are Android, Windows and Linux; iOS and macOS are not planned for now.
- 2026-09-30, 0002: The app is built on blocks, properties, views; markdown stays underneath but hidden.
- 2026-09-30, 0003: Single user and private for now; runs on the user's own machines with no paid cloud or subscription.
- 2026-09-30, 0004: The MVP is a notes app with pages, blocks and databases, focused on moddability and the basics.
- 2026-09-30, 0006: Flutter over the shared Rust core for Android, Windows and Linux, chosen after the bake-off; replaces the Tauri shell.
- 2026-10-07, 0007: The hub moves to the user's VPS: a Rust server that runs sync and the BubblePaw agent, behind Authentik login.
- 2026-10-15, 0008: The app's user-visible name is Scratch (was Notes); package id, binaries, repo and hub domain stay.
- 2026-10-15, 0009: Schedule notes with a frontmatter format, one runner node, listed tools instead of Bypass, and a dedicated schedule view.
- 2026-10-16, 0010: Hidden system pages reuse the page role; a feedback role hides its page and descendants, and roles round-trip through Markdown frontmatter.
- 2026-10-16, 0011: Authentik is the authorization server for the Claude connector; the hub only validates its JWTs offline, beside agent tokens. Superseded in part on 2026-10-22: connectors use the one shared provider below.
- 2026-10-21, integration spec phase 4: any signed-in device manages the hub's agent tokens with its device token (agent tokens and OAuth JWTs are refused); a token's expiry is picked at creation (never, 30, 90 or 365 days, default 90), existing tokens keep none, and last use is recorded at most once a minute. Scope stays read or write until tool sets (phase 5).
- 2026-10-21, integration spec phase 5: permissions are tool sets. The built-in sets are Read, Write and Everything (every tool a surface serves, including later ones), and Everything is always offered; there are no user-defined sets yet. Agent tokens keep the `scope` field (`read` and `write` map to Read and Write, `everything` is new); `notes setup` asks for Write. Schedules and the in-app agent's runners pick from the same sets. Each runner's set is a vault setting next to its default model and is read by that runner when its engine starts (default Everything); the conversation tools come with every set, the desktop and project tools only with Everything.
- 2026-10-22: One Authentik provider for hub sign-in and every MCP OAuth connector (claude.ai, ChatGPT, Gemini, CLI clients). It was a confidential client at first (see the 2026-10-22 line below); the old public client `claude-mcp` is no longer used by the hub and is removed once connectors move.
  - 2026-10-22: The provider is now a public client: PKCE S256, no client secret. Connectors take only the client ID. The hub's `[oidc] client_secret` is optional; set it only for a confidential provider. Security: PKCE and strict redirect URIs carry the code's protection, since Authentik does not itself require PKCE for public clients.
- 2026-10-22: PC host relay. A desktop with "Let the hub use this computer" on (off by default, per device) keeps one outbound socket to `/host/ws` with its device token and is the host `<device name>` on `/mcp/agent`. It exposes the same tools as the in-app agent on that computer (desktop pack included), runs them under Bypass, and accepts calls from OAuth logins only. One connection per device, a 10-minute call timeout, and no host state across hub restarts. Phones are never hosts. See [agent.md](../integrations/agent.md#pc-host-relay-hostcomputer-built-2026-10-22).
- 2026-10-22: Desktop MCP servers. The desktop agent starts the MCP servers set in Settings > AI > Agent > MCP servers (per device, in `ai_settings.json`, never the vault; changes apply live to chats and the relay) when it uses the Everything tool set. An existing `mcp.toml` (BubblePaw's `[mcp.servers.<name>]` format) is imported once and renamed to `mcp.toml.imported`. In-app chats and the PC host relay both get their tools and run them under Bypass, so `approval = "confirm"` does not prompt; this is the approved Everything and Bypass behaviour. A server that fails to start is logged and skipped. The hub never starts MCP servers. See [agent.md](../integrations/agent.md#mcp-servers-on-the-desktop-built-2026-10-22).
- 2026-10-23: Note sharing. The hub renders pages as small HTML: `/open/<id>` shows the page to a signed-in web session (same OIDC provider and callback, 30-day `HttpOnly; Secure; SameSite=Lax` cookie) and otherwise bounces to the app as before; share links `/s/<token>` are public or private (web session), this version (frozen snapshot) or live, revocable (410), `noindex`, without subpages, with preview cards that say only "Private note" for private links. Devices and web sessions create shares; agent tokens can't. Not shared: subpages, whole databases, comments or editing, per-person access, expiry. See [sharing.md](../integrations/sharing.md).
- 2026-10-23: Stable share links. Creating a link is idempotent: one live link per page and visibility, and one snapshot per page, visibility and content version. This holds from any device or the web. Revoked tokens are never reused. Links can be revoked from the popup, the viewer, the web `/shares` page and Settings › Shared links. See [sharing.md](../integrations/sharing.md).
