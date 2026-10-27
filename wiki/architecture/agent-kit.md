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

The Agent dev pane (`app/lib/addons/agent/kit_*.dart`, see [Agent](../addons/agent.md)) uses `agent_kit` and `agent_kit_core`, which `app/pubspec.yaml` declares directly. Both move together to the full commit SHA of agent-kit `v0.5.0`. `agent_kit` depends on `agent_kit_core` by path, so pub resolves it as git at that commit and a tag ref on the direct core dependency fails version solving. Its bb_ui dependency is an explicit git dependency on the separate bb-ui repository; Scratch uses the same declaration for its own direct bb_ui import.
