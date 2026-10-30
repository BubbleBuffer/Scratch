---
type: index
summary: "The Flutter app: app state, note types, the add-on architecture, the UI and agent kit, and feedback capture."
parent: "[[index]]"
status: current
---

# App

These pages describe the Flutter app: how it is built on the core, how note types and add-ons plug in, which shared kits it uses, and how feedback is captured. The add-ons themselves are in the [add-on reference](../addons/index.md).

<!-- wiki:children -->
- [The Flutter app (state)](app-state.md): The built Flutter app: bridge, layers, change feed, stores, registries, UI kit, shells, sync and settings, databases, tests on the real core, CI in the former private repo on its self-hosted runner (until 2026-10-22), known gaps. (partial)
- [Note types](note-types.md): Everything is a note; notes have types (document, database, kanban, chat, task), and a type registry selects how each type is drawn, embedded and edited.
- [Add-ons](addons.md): The app as a thin shell plus a shared kit, every feature an isolated add-on registering types, blocks, views, commands, codecs, search sources and agent tools; enforced in CI.
- [UI and agent-kit dependencies](agent-kit.md): bb-ui supplies tokens and primitives; agent-kit supplies the optional chat pane; repository pins and how to update them.
- [Feedback capture (state)](feedback.md): Shake, Ctrl+Shift+F or a Send feedback button opens a sheet with a screenshot and diagnostics; reports are hidden child pages of the Feedback system page.
<!-- /wiki:children -->
