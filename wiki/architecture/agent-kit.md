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

`app/pubspec.yaml` declares `bb_ui` as a git dependency pinned to bb-ui's `v0.4.0` tag. Its URL, ref and package path must match agent-kit's explicit bb_ui dependency:

```yaml
  bb_ui:
    git:
      url: https://github.com/BubbleBuffer/bb-ui
      ref: v0.4.0
      path: packages/bb_ui
```

`bb_tokens` comes with it: `bb_ui` depends on it by a relative path inside bb-ui, which pub resolves at the tag's commit. Scratch reaches the tokens through `package:bb_ui/tokens.dart` (that is what `ui/tokens.dart` re-exports), so it does not declare `bb_tokens` itself. Declaring it directly with a tag ref conflicts with the transitive copy's commit ref: pub treats them as different sources. The generated `pubspec.lock` records each resolved commit as `resolved-ref`.

## Bumping it

1. Release bb-ui first when its packages change: merge with CI green and push an annotated tag. Update agent-kit's bb_ui dependency to that exact URL, ref and path, then release agent-kit with an annotated tag (see its `docs/package-layout.md`).
2. In Scratch, match bb_ui's declaration to agent-kit's, and pin both `agent_kit` and `agent_kit_core` to the full commit SHA of the new agent-kit tag. Run `flutter pub get` in `app/` on the build host, then commit `pubspec.yaml` and the generated `pubspec.lock` together. Run analyze, add-on boundaries and the kit backend tests there; require CI green.
3. If the kit changed anything visible, the pull request shows before/after screenshots and updates the goldens on purpose.

To try an unreleased kit change, point `ref:` at a branch or commit for a draft pull request, or use `dependency_overrides` with local `path:` entries; never merge either.

## The dev pane's packages

The agent chat's outline and the Agent dev pane (`app/lib/addons/agent/kit_*.dart`, see [Agent](../addons/agent.md)) use `agent_kit` and `agent_kit_core`, which `app/pubspec.yaml` declares directly. Both move together to the full commit SHA of agent-kit `v0.7.0`. `agent_kit` depends on `agent_kit_core` by path, so pub resolves it as git at that commit and a tag ref on the direct core dependency fails version solving. Its bb_ui dependency is an explicit git dependency on the separate bb-ui repository; Scratch uses the same declaration for its own direct bb_ui import.

## The chat outline

Chapter cards and headings, section dividers, the where-am-I strip, Contents and their menus, the fold and jump state, the indexed list and its row layout come from the kit (`OutlineController`, `OutlineList`, `OutlineWhereStrip` and the rest of `agent_kit`'s outline area). Scratch's side is `addons/agent/agent_outline.dart` (the store's `ChatOutline` becomes the kit's `Outline`), `ChatReaderController` (composes the kit's `OutlineController`; keeps following the end of a live reply, pending references, the plain list and `keepMessagePosition`) and `AgentChat`, which passes the host actions as `OutlineAction`s. `ItemRow` carries an item id, so the chat looks the item up when a row is built. The chat itself, `AgentChat(store, panel)`, is unchanged.

The kit reads its tokens through `BbThemeScope`. `NotesShell` wraps the app in one, above the navigator, because the kit's sheets and menus are routes and read the scope too.

## Search widgets

The search UI is shared by the Ctrl+K overlay, the mobile search screen and home's inline results. `app/lib/shell/search_widgets.dart` only re-exports three files: `search_result_widgets.dart` (result rows, highlighted text, group headers and the grouped list), `search_type_chips.dart` (the `type:` chips) and `search_panel_body.dart` (the overlay body and its keyboard selection). Rows from registered sources are opaque to them: a hit's `target` is read only by the source that wrote it, and `openChatHit` (`addons/agent/chat_search.dart`) is the Agent source's open. `ChatSearchTarget` (`chat_search_target.dart`) reads and writes that JSON and imports nothing from navigation. `test/search_open_contract_test.dart` pins the open contract: a malformed target says why, an anchored hit asks for its range with the hit's labels, and a chat the agent does not have is reported.
