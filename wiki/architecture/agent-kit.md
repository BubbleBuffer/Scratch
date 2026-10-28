---
type: topic
layer: state
summary: "Scratch consumes UI tokens and primitives from bb-ui and the optional chat pane from agent-kit; repository pins, compatibility and release updates."
parent: "[[architecture/index]]"
status: current
---

# agent-kit dependency

Scratch's design tokens and notes-free UI primitives come from [BubbleBuffer/bb-ui](https://github.com/BubbleBuffer/bb-ui), shared with BubblePaw Desktop. They were originally extracted into agent-kit on 2026-10-26. The separate [BubbleBuffer/agent-kit](https://github.com/BubbleBuffer/agent-kit) repository owns the agent-chat packages. Scratch's look is the UI packages' default.

## What comes from the kit

- `bb_tokens`: `tokens.dart` (`AppColors`, `TypeScale`, `Layout`, `Radii`, `Motion`, `FormFactor`, `AppTheme`, the font family names).
- `bb_ui`: buttons, fields, hover, menus, `AppTooltip`, `Ico` and the Lucide glyphs, lines, marked list, modal, sheet, routes, picker sheet, reference chip, selection handles, `FoldableStackCard`, and the settings controls.

The old files stay in place as one-line re-exports, so imports and call sites are unchanged:

- `app/lib/ui/`: `buttons`, `fields`, `hover`, `icons`, `lines`, `lucide`, `marked_list`, `menu`, `modal`, `picker_sheet`, `reference_chip`, `routes`, `selection`, `sheet`, `stack_card`, `tokens`, `tooltip`.
- `app/lib/features/settings/controls.dart` (from `package:bb_ui/settings_controls.dart`).

Two pieces stay in Scratch:

- `ui/tokens.dart` also re-exports the app clock (`clockNow` from `core/clock.dart`).
- The tooltip's command shortcuts: the kit's `AppTooltip.commandShortcuts` is a lookup the app sets. `commandRegistry` (`registry/command_registry.dart`) points it at itself when it is created, so tooltips show the same shortcuts as before. `test/ui/app_tooltip_test.dart` covers this through the registry.

The fonts stay bundled in Scratch (`app/pubspec.yaml`); the kit names the same families. Everything else in `ui/` is Scratch's own (note previews, embeds, page icons, boards, databases, emoji picker, toasts and the rest).

## The pin

`app/pubspec.yaml` declares `bb_ui` as a git dependency pinned to bb-ui's `v0.6.0` tag. Its URL, ref and package path must match agent-kit's explicit bb_ui dependency:

```yaml
  bb_ui:
    git:
      url: https://github.com/BubbleBuffer/bb-ui
      ref: v0.6.0
      path: packages/bb_ui
```

`bb_tokens` comes with it: `bb_ui` depends on it by a relative path inside bb-ui, which pub resolves at the tag's commit. Scratch reaches the tokens through `package:bb_ui/tokens.dart` (that is what `ui/tokens.dart` re-exports), so it does not declare `bb_tokens` itself. Declaring it directly with a tag ref conflicts with the transitive copy's commit ref: pub treats them as different sources. The generated `pubspec.lock` records each resolved commit as `resolved-ref`.

## Bumping it

1. Release bb-ui first when its packages change: merge with CI green and push an annotated tag. Update agent-kit's bb_ui dependency to that exact URL, ref and path, then release agent-kit with an annotated tag (see its `docs/package-layout.md`).
2. In Scratch, match bb_ui's declaration to agent-kit's, and pin both `agent_kit` and `agent_kit_core` to the full commit SHA of the new agent-kit tag. Run `flutter pub get` in `app/` on the build host, then commit `pubspec.yaml` and the generated `pubspec.lock` together. Run analyze, add-on boundaries and the kit backend tests there; require CI green.
3. If the kit changed anything visible, the pull request shows before/after screenshots and updates the goldens on purpose.

To try an unreleased kit change, point `ref:` at a branch or commit for a draft pull request, or use `dependency_overrides` with local `path:` entries; never merge either.

## The dev pane's packages

The agent chat's outline and the Agent dev pane (`app/lib/addons/agent/kit_*.dart`, see [Agent](../addons/agent.md)) use `agent_kit` and `agent_kit_core`, which `app/pubspec.yaml` declares directly. Both move together to the full commit SHA of agent-kit `v0.8.0`. `agent_kit` depends on `agent_kit_core` by path, so pub resolves it as git at that commit and a tag ref on the direct core dependency fails version solving. Its bb_ui dependency is an explicit git dependency on the separate bb-ui repository; Scratch uses the same declaration for its own direct bb_ui import.

## The chat outline

Chapter cards and headings, section dividers, the where-am-I strip, Contents and their menus, the fold and jump state, the indexed list and its row layout come from the kit (`OutlineController`, `OutlineList`, `OutlineWhereStrip` and the rest of `agent_kit`'s outline area). Scratch's side is `addons/agent/agent_outline.dart` (the store's `ChatOutline` becomes the kit's `Outline`), `ChatReaderController` (composes the kit's `OutlineController`; keeps following the end of a live reply, pending references, the plain list and `keepMessagePosition`) and `AgentChat`, which passes the host actions as `OutlineAction`s. `ItemRow` carries an item id, so the chat looks the item up when a row is built. The chat itself, `AgentChat(store, panel)`, is unchanged.

The kit reads its tokens through `BbThemeScope`. `NotesShell` wraps the app in one, above the navigator, because the kit's sheets and menus are routes and read the scope too.

## Search widgets

The search presentation is bb_ui's (`package:bb_ui/search.dart`, imported as `bb`): `SearchPanel` (field, filter slot, arrow and Enter selection, notes), `SearchResultsList`, `SearchResultRow`, `SearchGroupHeader`, `HighlightedText` and `SearchTypeChips` over neutral `SearchResult`, `SearchResultSection` and `SearchTypeChip` models, with `termHighlights` for the marked ranges. Scratch keeps the mapping and the data. It is shared by the Ctrl+K overlay, the mobile search screen and home's inline results through `app/lib/shell/search_widgets.dart`, which re-exports three files: `search_result_widgets.dart` (`searchResultOf` maps a `Hit` to a `bb.SearchResult`: title tone, page icon and path or the source's icon, highlights; plus the Hit-typed `SearchResultRow` and `SearchResultsList` adapters that map a picked result back to its `Hit`), `search_type_chips.dart` (which `type:` chips a query shows, drawn by bb) and `search_panel_body.dart` (a thin wrapper around `bb.SearchPanel` that keeps the `type:` rewrite). The shared list builds all result models up front instead of one per visible row. Rows from registered sources are opaque to them: a hit's `target` is read only by the source that wrote it, and `openChatHit` (`addons/agent/chat_search.dart`) is the Agent source's open. `ChatSearchTarget` (`chat_search_target.dart`) reads and writes that JSON and imports nothing from navigation. `test/search_open_contract_test.dart` pins the open contract: a malformed target says why, an anchored hit asks for its range with the hit's labels, and a chat the agent does not have is reported.

## Command palette

The command palette is bb_ui's (`package:bb_ui/palette.dart`, bb-ui 0.6.0): `showCommandPalette` opens a 560-wide panel near the top of the window (the full screen with a Cancel button on a phone) over a list of neutral `PaletteCommand`s (id, title, subtitle, group, shortcut label, enabled with a reason, icon from `Ico`). Typing fuzzy-filters (case-insensitive, characters in order, a contiguous or word-start match ranks higher, matches marked); Up and Down move the selection and stop at the ends, Enter runs it, Escape closes, Tab stays in the field. A disabled command is listed with its reason and never runs. Focus goes back to what had it, and the future completes only after that, with the picked command.

Scratch's side is `app/lib/shell/command_palette.dart`: `paletteCommands` maps `commandRegistry` onto the neutral model, sorted by label (enabled checks run when the palette opens; `Command.disabledReason` is the text shown for a disabled one, "Not available now" when a command gave none; the shortcut label is the command's first shortcut), and `openCommandPalette` runs the picked command through `CommandRegistry.run`, so its `BuildContext` action, shortcut binding and enabled check stay in the registry. The palette lists every registered command except `palette.open`. The command `palette.open` ("Command palette") is registered with the shell commands and opens with **Ctrl+Shift+P**, because Ctrl+K and Ctrl+P already open search. `test/command_palette_test.dart` covers opening by the shortcut, filtering, running, focus restore, a disabled command and the phone layout without the native core.

## Chat preferences

Each chat's host ("Runs on") and picked model stay in Scratch; no kit widget reads them, so agent-kit has no preferences API. `AgentChatPrefs` (`addons/agent/agent_chat_prefs.dart`) holds the choices and goes through a small `AgentChatPrefsStorage` interface (`read`, `write`). `AgentChatPrefsFile` (`agent_chat_prefs_file.dart`) is the only implementation: the device's `<data dir>/agent_chats.json`, never synced, in the same format as before (an object keyed by chat id, `{"host": ..., "model": ...}`, two-space indent). `AgentStore` builds it with that file, or with no storage (memory only) when it has neither a data dir nor a settings file. A missing or unreadable file, and entries that are not a valid choice, give no choices. A save writes `agent_chats.json.tmp` and renames it over the file; a failed save is dropped, so the choice stays in memory for the run, the old file is left as it was and the next save that works writes everything. `test/agent_runner_select_test.dart` covers the round trip, a junk file, a file saved by earlier versions (loaded and re-saved byte for byte), a failed save and a throwing storage.

## MCP and runner settings

The MCP servers and the runner pickers in Settings > AI > Agent stay in Scratch. Their rows, fields, toggles and select buttons already come from `bb_ui` (the settings controls); nothing else in them is used twice, and no kit consumer needs agent-specific settings, so neither bb-ui nor agent-kit has a settings API for them. Each is split into logic without widgets and a layout file:

- `agent_mcp_draft.dart`: a server as editor text (`McpServerDraft`), `validateMcpServer`, `mcpServerJson` and the list edits (save in place or at the end, turn on or off, remove). The format is BubblePaw's `McpServerSettings`. A save starts from the server as saved, so keys the editor doesn't show (`cwd`, `env_from`, timeouts, anything newer) stay as they are; a change of type drops only the other type's keys (a URL server has no `command`, `args`, `env`, `env_from` or `cwd`; a command server no `url` or `headers_from`).
- `agent_mcp_controller.dart`: `McpServersController`, the section's state (polled server states, the open editor, its error) over `read`, `write` and `status` functions. `McpServersSettings` (`agent_mcp_settings.dart`) wires them to this device's AI settings (`agentMcpServers`, not synced) and `AgentStore.mcpStatus`, and lays the section out.
- `agent_runner_choices.dart`: the chosen runner, the help lines and the model pickers' options; `agent_runner_settings.dart` lays them out and saves through `AgentStore` (vault defaults, synced).

`test/agent_mcp_server_json_test.dart` has the table tests (every validation message, duplicate names, both transports and a change between them, unknown keys through load, edit and save, byte for byte). `test/agent_mcp_controller_test.dart` covers the controller and the runner choices; `test/agent_mcp_settings_test.dart` and `test/agent_runner_select_test.dart` cover the screens.

## Transcript and composer port (plan)

Port plan step 7: the chat's transcript and composer move onto the kit's surfaces (`agent_kit`: transcript, activity, approval, composer) fed by Scratch adapters, in increments that each land as one pull request. Scratch's current look is the target. A pull request either leaves the agent goldens byte-identical or lists every changed image with the reason; a change the kit cannot make without altering what the user sees or does is a design question and waits for an answer.

What stays in Scratch, through the kit's hooks: reference chips and the vault's page mentions (`MarkdownRenderer`, `MentionSource`), page-edit rows with their undo that finishes later (`onUndo`), a draft that is kept when a send fails, the page, selection and chat context a message carries, and the chat store itself. The kit never reads `AgentStore`.

### Protective tests (done first)

These run on the code before any increment and stay green through all of them.

- `test/agent_composer_input_test.dart`: an `@` answer for an older query that arrives after a newer one is dropped (arriving after, and arriving while the newer one is pending); an answer after the `@` is gone shows no menu; a send that fails (a model no provider serves) keeps the draft, the picked mention, the page chip and the quoted selection, and the same draft then goes out whole; a chat that cannot be created keeps the draft. `AgentComposer` takes an optional `mentionSource` (default `mentionPages`) so the test controls when each answer arrives; this is the seam the kit's `MentionSource` replaces.
- Already in place: `test/agent_transcript_order_test.dart` (item order and tool ownership), `test/agent_chat_partb_test.dart` (live turn, edits, undo, context chips, composer layout), `test/agent_test.dart` (approvals, steer and stop), `test/agent_golden_test.dart`, `test/agent_panel_golden_test.dart`, `test/chat_reference_golden_test.dart`, `test/mobile_agent_golden_test.dart`.

### Increments

| # | Increment | Moves to the kit | Stays in Scratch | Protected by |
|---|---|---|---|---|
| 0 | Protective tests, the `mentionSource` seam, this plan | nothing | everything | the tests above |
| 1 | Transcript adapter: `AgentSessionState` and the open chat's live state to the kit's `TranscriptItem`s, with no widget change | nothing visible | the mapping (`agent_transcript.dart` keeps ordering, ownership and outline segments) | order test; new mapping tests; goldens identical by construction |
| 2 | Approval (done) | `ApprovalBar` (keys 1, 2, 3, "Needs approval", waiting time, Details) replaces `ApprovalCard` | the mapping from `AgentApproval` (`kitApproval`, `kitToolTarget`) | approval tests in `agent_test.dart` and `agent_chat_partb_test.dart`; approval goldens |
| 3 | Tool and code cells | `ActivityGroupView`, exec output rows replace `AgentWorkRow` and `CodeCellRow` | code disclosure setting, the work summary counts | partb activity tests; agent goldens |
| 4 | Edit rows | `FileEditRow`, `DiffView` replace `AgentEditRow` | page-edit rows (opening the page at the changed blocks), undo that finishes later, edits of untitled pages | partb edit and undo tests; goldens |
| 5 | Turn view | `AssistantProse`, notice lines for stopped and failed runs, the working line | `AgentMarkdown` as the `MarkdownRenderer` (reference chips, page links), `Add to note` | partb live-turn test; reference goldens |
| 6 | Transcript list | `TranscriptList` under the outline rows (follow the end, jump to latest, anchor restore) | `ChatReaderController` (pending references, keep message position) | outline widget, split and live tests; order test |
| 7 | Composer | `AgentComposer` with a `MentionSource`; chips from `KitRef`s | the mention source over pages and chats, the page, selection and chat context, host and model pickers, the draft kept on a failed send | `agent_composer_input_test.dart`; partb composer tests; composer goldens |

Each increment from 2 on first checks whether the kit can draw Scratch's current look. Where it cannot, the kit change is released first (agent-kit v0.7.x, bb-ui v0.6.x, pinned by commit SHA as under "Bumping it") and the Scratch pull request follows. The pull request runs a temporary workflow that renders the agent goldens on main and on the branch with the same bridge and compares the images; it is removed before the merge.

### Known differences to settle before increments 2 to 7

These are visible or behavioural, so the swap cannot be byte-identical until each is decided or the kit grows an option:

- Approval (settled, done): the kit's bar replaces the card. It sits above the composer and reads Approve, For session and Deny with the keys 1, 2 and 3. It names the tool and its key argument (a command, path, URL, title, query or page) on one line, cut with a tooltip; Details opens the full arguments as indented JSON. The old `agent-approve-*` keys are gone and the tests find the buttons by their text.
- Edit rows (settled): the kit's `onUndo` returns a future and the row reads "Undoing…" until it completes, "Undo failed" if it throws (agent-kit 0.8.0).
- Edit rows: the kit's undo is a void callback and its row cannot show that an undo is running or failed; Scratch's finishes later and reports.
- Composer (settled): the kit's `onSubmit` returns `FutureOr<bool>` (agent-kit 0.8.0) and the composer clears only on `true`, so a failed send keeps the text, the mentions, the chips and the quote. `agent_composer_input_test.dart` proves it.
- Transcript: Scratch groups a turn's messages, activity, edits and cells into one `AgentTurn` item per outline segment; the kit's items are flatter. The mapping in increment 1 has to carry segment ids so the outline and "keep message position" still find their rows.

## Approval bar

A request the agent waits on is the kit's `ApprovalBar` (agent-kit 0.8.0), not a card in the transcript. `AgentChat` draws one bar per pending approval above the composer (`_approvalBars`), each from `kitApproval(AgentApproval)` (`kit_backend.dart`): tool name, a target taken from the call's `path`, `command`, `url`, `title`, `query`, `page`, `name` or `id` argument (first present), the request time for "waiting 12m" (read against the app clock), and the arguments for Details. Approve, For session and Deny (with an optional reason) call `AgentStore.decide` as `approve`, `approve_session` and `deny`. The first bar answers to 1, 2 and 3 while no text field has focus, focus is inside the bar, or the composer is empty (`AgentComposer` takes the draft's controller for that). The transcript no longer has approval items (`ApprovalItem` is gone from `agentTurns`). `test/agent_test.dart` covers approve, deny with a reason, the keys, and the `project_open` request.
