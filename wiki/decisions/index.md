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
- 2026-10-16, 0011: Authentik is the authorization server for the Claude connector with a public client `claude-mcp`; the hub only validates its JWTs offline, beside agent tokens.
