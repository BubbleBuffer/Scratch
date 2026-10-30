---
type: index
summary: "How the app works now: data model, folder mounts, sync and storage, add-ons, and the built app, core, sync and hub."
parent: "[[index]]"
status: current
---

# Architecture

These pages describe how the app works now. Plans, proposals and decisions are in the internal wiki (notes vault).

<!-- wiki:children -->
- [Add-ons](addons.md): The app as a thin shell plus a shared kit, every feature an isolated add-on registering types, blocks, views, commands, codecs, search sources and agent tools; enforced in CI.
- [Add-on reference](../addons/index.md): Board and database registrations, files and dependencies.
- [UI and agent-kit dependencies](agent-kit.md): bb-ui supplies tokens and primitives; agent-kit supplies the optional chat pane; repository pins and how to update them.
- [Android agent through the Hub](mobile-agent.md): Hub-only mobile agent, authenticated vault sync, local acceptance tests and the phone sign-in steps.
- [Android release](android-release.md): How Android builds are numbered, signed and published: a Scratch release is built on GitHub, release-signed on the home Linux box, and published to the owner's private F-Droid repo.
- [The Flutter app (state)](app-state.md): The built Flutter app: bridge, layers, change feed, stores, registries, UI kit, shells, sync and settings, databases, tests on the real core, CI in the former private repo on its self-hosted runner (until 2026-10-22), known gaps. (partial)
- [Conflict format v2](conflict-format.md): Approved Block, Subtree and Page conflicts, edit-beats-delete, synced resolutions and undo, journal repair log and vault-wide auto-resolve.
- [Core (state)](core-state.md): The built Rust core (vault_core): Loro page documents, properties, collections and queries, trash, links, attachments, history, import, index, bench.
- [Data model](data-model.md): Everything is a page with blocks and properties; collections group pages; views are queries plus renderers; markdown is the file format.
- [Device keys](device-keys.md): each device holds an Ed25519 key (bubblebuffer-device-key); the hub binds it to the device token, and sync, sockets and hub calls run on 1-hour sessions bought with a signed challenge; pairing links carry one-time codes. Agent tokens and MCP OAuth stay.
- [Feedback capture (state)](feedback.md): Shake, Ctrl+Shift+F or a Send feedback button opens a sheet with a screenshot and diagnostics; reports are hidden child pages of the Feedback system page.
- [Folder mounts](mounts.md): Any PC folder can be mounted at a vault path; devices see one logical vault, with no symlinks.
- [Hub server (state)](hub.md): notes-hub, the server hub: sync over axum behind Caddy, device tokens from an Authentik (OIDC) login, the agent at /agent/ws (JSON-RPC over WebSocket); config, routes, protocol, security.
- [Hub deployment](hub-deploy.md): Local amd64 container build and deploy kit; Authentik, the existing Caddy network, provider sign-ins, backup and rollback.
- [Note types](note-types.md): Everything is a note; notes have types (document, database, kanban, chat, task), and a type registry selects how each type is drawn, embedded and edited.
- [Search queries and live queries](search-queries.md): Search operators (date:, tag:, in:, property filters) parsed into one query spec, filtered search inside the 20 ms budget, and live queries pushed after index commits.
- [Sync and storage](sync-and-storage.md): Local-first sync with CRDTs, attachments by hash, page history and cold history storage.
- [Sync (state)](sync-state.md): The built sync (vault_sync + core hooks): hub server, binary protocol, token pairing, delta sync by version vectors, conflicts, purges, lazy blobs, numbers.
- [Repository and releases](repository-and-releases.md): Where code, CI, releases and private operations live (public Scratch repo, private scratch-ops), the CI workflows, the private-strings check, and how to cut a release.
- [Windows installer](windows-install.md): Per-user Inno Setup installer built by the windows-build and release workflows: where Scratch installs, shortcuts, upgrade, uninstall and how to build it.
<!-- /wiki:children -->
