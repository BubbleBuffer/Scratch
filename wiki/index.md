---
type: index
summary: "A private organizer for notes, plans, lists and calendar: mobile-first, AI-integrated, self-hosted."
status: current
---

# Scratch

A personal, one-stop organizer: notes, a wiki, plans, ideas, lists, tasks and a calendar. It is built on blocks, pages, databases and views, keeps plain files underneath as Obsidian does, and leaves out what gets in the way: Obsidian's syntax, keybinds and poor mobile experience. It is built for one person, runs on that person's own machines, and treats AI assistants (Claude, ChatGPT, local agents) as first-class users of the vault.

Plans, open questions, decisions and the current focus are in the internal wiki in the notes vault (notes-app); these pages describe how it works now.

**Status:** Flutter Android/desktop app, Rust vault and sync, and the Hub server are built. Android agent behavior and its [phone verification steps](architecture/mobile-agent.md) are retained. Answers in these pages are the user's; proposals and interpretations are marked.

Start with the [architecture index](architecture/index.md), then the [data model](architecture/data-model.md), [folder mounts](architecture/mounts.md) and [sync state](architecture/sync-state.md).

<!-- wiki:children -->
- [Add-on reference](addons/index.md): The compiled-in add-ons, registrations and public APIs.
- [Kanban](addons/kanban.md): Optional kanban notes with manually collected cards and compact embeds.
- [Architecture](architecture/index.md): How the app works now: data model, folder mounts, sync and storage, add-ons, and the built app, core, sync and hub.
- [Decisions](decisions/index.md): Decisions that shape what's built, one line each with its date.
- [Integrations](integrations/index.md): Connections to AI assistants and the agent.
- [Product](product/index.md): What the app does now, how it looks and feels, and the speed measurements behind it.
- [Prototypes](prototypes/index.md): The Tauri and Flutter bake-off prototypes: what each is, how it is built and tested, and its known gaps.
<!-- /wiki:children -->
