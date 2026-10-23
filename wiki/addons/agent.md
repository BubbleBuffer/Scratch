---
type: topic
layer: implementation
summary: "Desktop and Hub agent chat, its note type and range embeds, shell surfaces, settings and registered search source."
parent: "[[addons/index]]"
status: current
---

# Agent

`AgentAddon` in [api.dart](../../app/lib/addons/agent/api.dart) is always registered and has no add-on dependencies. "Use the agent" (`agentEnabled`) is the agent's on/off switch. Registration opens no vault, agent, transport or timer.

It registers:

- The `chat` note type with chip, card, embed and full-page builders. `bodySource: 'agent'` records that its transcript stays in the agent store, outside vault pages. A full-page presentation selects its session by ID. Its embed/card builders accept a bare session ID (or the primary indexed identity `chat:<session-id>`) and draw a folding card with the chat title and chapters. Range-JSON references retain their original card. Registration does not mirror chats into the vault.
- The existing `chat_ref` block with explicit Markdown, clipboard and plain-text fallback codecs. Its block-backed Chat slash item keeps the same menu position and insertion path. A populated block calls its context's registered chat embed builder with the session ID and opaque message-range target. Cache updates still refresh the target and searchable summary; the existing folded card is unchanged.
- `agent.open` (Ctrl+J), and the existing dev replay command when enabled. The generic `shellPanels` entry supplies the desktop dock wrapper, header toggle and full-screen/mobile builder.
- The desktop and Android Agent settings sections at their existing positions on the AI page.
- The indexed `agent` search source: Chats group, chat type chip, existing row icon and labels, and source-owned navigation. Core ranking and persisted title/chapter/section summary rows remain unchanged. Grouped Ctrl+K and mobile search treat targets as opaque; this add-on decodes session/message anchors.

Files: `agent_store.dart` owns the local/Hub client, sessions, stream, lifecycle and index reconciliation; screen/panel/chat/composer/turn/transcript/markdown/cell files draw the chat; outline files draw chapters, sections and Contents; `chat_reference*`, `add_to_note.dart` and `blocks/chat_ref.dart` adapt references; `chat_type.dart` provides type builders; `chat_search.dart` owns search target parsing and navigation; `agent_search_source.dart` and `agent_search_vault.dart` maintain indexed rows; `commands.dart`, `navigation.dart`, settings files and `dev_replay.dart` own the remaining agent behavior. Other code imports only `api.dart`.

The host contract is [addon_services.dart](../../app/lib/services/addon_services.dart): settings and AI-task notification, navigation/page stack, sidebar, sync/login settings, mention UI and marked text styling are explicit transitional surfaces. The add-on imports no old feature, editor or shell implementations directly. Shared UI cards/chips stay in `ui/`. Rust tools and their protocol are unchanged; `AddonContext.agentTools` remains an interface.

The add-on cannot be disabled per device. Generic unknown-block and unregistered-source fallbacks remain available to the kit and isolated registration tests.

`app/test/addons/agent/` checks isolated registration and actual page-to-type embed dispatch. `app/test/addons/search_sources_test.dart` checks fake sources and unregistered cached rows independently of Agent. Generic unknown-type fallbacks remain covered by the kit tests. See [Add-ons](../architecture/addons.md) for acceptance results and limits.
