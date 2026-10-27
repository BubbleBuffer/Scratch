---
type: topic
layer: implementation
summary: "Optional desktop and Hub agent chat, its note type and range embeds, shell surfaces, settings and registered search source."
parent: "[[addons/index]]"
status: current
---

# Agent

`AgentAddon` in [api.dart](../../app/lib/addons/agent/api.dart) is optional, enabled by default, and has no add-on dependencies. Settings → About → Add-ons → Manage can disable it for the next start. Registration opens no vault, agent, transport or timer.

It registers:

- The `chat` note type with chip, card, embed and full-page builders. `bodySource: 'agent'` records that its transcript stays in the agent store, outside vault pages. A full-page presentation selects its session by ID. Its embed/card builders accept a bare session ID (or the primary indexed identity `chat:<session-id>`) and draw a folding card with the chat title and chapters. Range-JSON references retain their original card. Registration does not mirror chats into the vault.
- The existing `chat_ref` block with explicit Markdown, clipboard and plain-text fallback codecs. Its block-backed Chat slash item keeps the same menu position and insertion path. A populated block calls its context's registered chat embed builder with the session ID and opaque message-range target. Cache updates still refresh the target and searchable summary; the existing folded card is unchanged.
- `agent.open` (Ctrl+J), and the existing dev replay command when enabled. The generic `shellPanels` entry supplies the desktop dock wrapper, header toggle and full-screen/mobile builder.
- The desktop and Android Agent settings sections at their existing positions on the AI page.
- The indexed `agent` search source: Chats group, chat type chip, existing row icon and labels, and source-owned navigation. Core ranking and persisted title/chapter/section summary rows remain unchanged. Grouped Ctrl+K and mobile search treat targets as opaque; this add-on decodes session/message anchors.

Files: `agent_store.dart` owns the local/Hub client, sessions, stream, lifecycle and index reconciliation; screen/panel/chat/composer/turn/transcript/markdown/cell files draw the chat; outline files draw chapters, sections and Contents; `chat_reference*`, `add_to_note.dart` and `blocks/chat_ref.dart` adapt references; `chat_type.dart` provides type builders; `chat_search.dart` owns search target parsing and navigation; `agent_search_source.dart` and `agent_search_vault.dart` maintain indexed rows; `commands.dart`, `navigation.dart`, settings files and `dev_replay.dart` own the remaining agent behavior. Other code imports only `api.dart`.

The host contract is [addon_services.dart](../../app/lib/services/addon_services.dart): settings and AI-task notification, navigation/page stack, sidebar, sync/login settings, mention UI and marked text styling are explicit transitional surfaces. The add-on imports no old feature, editor or shell implementations directly. Shared UI cards/chips stay in `ui/`. Rust tools and their protocol are unchanged; `AddonContext.agentTools` remains an interface.

When disabled, its type, block, slash item, commands, settings, panel and search source are absent. The agent store refuses to open. Existing `chat_ref` data uses the unknown-block placeholder and the containing page still opens. Grouped search hides unregistered indexed rows. With no registered indexed sources it uses page-only search before ranking and the limit, so cached chat rows cannot crowd out pages.

`app/test/addons/agent/` checks isolated registration, actual page-to-type embed dispatch, disabled placeholders and refused agent opening. `app/test/addons/search_sources_test.dart` checks a fake source independently of Agent. Existing agent/mobile/outline/reference and grouped-search assertions and golden files remain unchanged. See [Add-ons](../architecture/addons.md) for acceptance results and limits.

## Kit dev pane

A second, read-only chat view built from the shared [agent kit](https://github.com/BubbleBuffer/agent-kit) (`AgentChatPane`, pinned in `app/pubspec.yaml` at a commit on its main). It is off by default: the old Agent screen stays the default and is unchanged. Building with `--dart-define=AGENT_KIT_PANE=true` makes the Agent shell panel (`kit_pane.dart`, the panel the sidebar and Ctrl+J open) show `AgentKitScreen` instead; it draws the store's open chat inside the kit's `BbThemeScope`, fed from Scratch's light/dark and form factor. The full-page chat type and the mobile route still use the old screen.

The data path is unchanged (Dart, flutter_rust_bridge, `NotesAgent`). `kit_backend.dart` implements the kit's `AgentBackend` over `sessions`, `sessionState`, `eventsAfter` and `subscribe`: `listChats`, `loadChat` and `events` work, capabilities are empty, and every command (send, cancel, approve, rename, undo, models and the rest) fails with `KitUnsupported`. Scratch sends no message or tool-row events, so `events` forwards run start and finish, approvals and streamed paragraphs, and asks the kit to reload the chat (a debounced `gap`) after tool, queue and settle events.

The pane follows a store-owned feed for its connection, so view-scope subscription replacement keeps it live and requests a reload across the replacement window. Connection-wide transport gaps also trigger a reload even when they have no session ID. Startup reconciles snapshot and buffered paragraph overlap through a reload, and cancellation stops pending startup work from publishing events.

Mapping gaps (Scratch concept dropped or reduced):

- The chat list has no run status (every row is idle), no cursor and a client-side title filter.
- A user message keeps its attached pages and chats as chips; the attached selection text and blocks are dropped.
- Chapters and sections (outline), page actions, schedule and replay records have no kit counterpart and are dropped.
- Only pending approvals are in a snapshot (no resolved history, no diff). Queued inputs, plan, children, jobs, questions, reasoning text and the context meter do not exist in Scratch; usage holds the last run's token counts only.
- Tool rows are grouped per assistant message, with output cut at 4000 characters; the target is guessed from common argument names. Page edits become file-edit items; an undone edit shows only after a reload.
- The workspace is the project name; branch and worktree do not exist.
- The port has no capability for sending, cancelling or approving, so those commands report `steer` as the missing capability.

Tests: `app/test/addons/agent/kit_backend_test.dart` (synthetic sessions: mapping, events, unsupported commands, and the kit pane drawing a chat).
