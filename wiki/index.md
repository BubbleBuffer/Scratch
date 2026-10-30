---
type: index
summary: "A private organizer for notes, plans, lists and calendar: mobile-first, AI-integrated, self-hosted."
status: current
---

# Scratch

A personal, one-stop organizer: notes, a wiki, plans, ideas, lists, tasks and a calendar. It is built on blocks, pages, databases and views, keeps plain files underneath as Obsidian does, and leaves out what gets in the way: Obsidian's syntax, keybinds and poor mobile experience. It is built for one person, runs on that person's own machines, and treats AI assistants (Claude, ChatGPT, local agents) as first-class users of the vault.

Plans, open questions, decisions and the current focus are in the internal wiki in the notes vault (notes-app); these pages describe how it works now. Agents get their own help topics, written for them, in [docs/notes](../docs/notes/).

**Status:** Flutter Android/desktop app, Rust vault and sync, and the Hub server are built. Android agent behavior and its [phone verification steps](sync/mobile-agent.md) are retained. Answers in these pages are the user's; proposals and interpretations are marked.

## How the wiki is organised

One section per component, each with its own index. Every page is reachable from here in two clicks.

## Read first

1. [Data model](core/data-model.md): pages, blocks, properties and views.
2. [The Flutter app](app/app-state.md): how the app is built on the core.
3. [Sync and storage](sync/sync-and-storage.md), then [sync state](sync/sync-state.md): how devices share a vault.

<!-- wiki:children -->
- [Product](product/index.md): What the app does now, how it looks and feels, and the speed measurements behind it.
- [Core](core/index.md): The Rust core (vault_core): the data model, state, search queries, the conflict format and folder mounts.
- [App](app/index.md): The Flutter app: app state, note types, the add-on architecture, the UI and agent kit, and feedback capture.
- [Add-on reference](addons/index.md): The compiled-in add-ons, registrations and public APIs.
- [Sync and Hub](sync/index.md): Sync and storage, the sync state, the Hub server, its deployment and the Android agent through the Hub.
- [Integrations](integrations/index.md): Connections to AI assistants and the agent, and note sharing.
- [Releases](releases/index.md): Where code and CI live, and how Android and Windows builds are made and published.
- [Decisions](decisions/index.md): Decisions that shape what's built, one line each with its date.
- [Prototypes](prototypes/index.md): Historical. The Tauri and Flutter bake-off prototypes: what each is, how it is built and tested, and its known gaps.
<!-- /wiki:children -->
