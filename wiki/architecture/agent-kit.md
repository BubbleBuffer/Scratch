---
type: topic
layer: state
summary: "Scratch's UI tokens and notes-free primitives live in the public agent-kit repo (bb_tokens, bb_ui); Scratch pins a tag through git, its ui/ files re-export them, and how to bump the pin."
parent: "[[architecture/index]]"
status: current
---

# agent-kit dependency

Since 2026-10-26, Scratch's design tokens and its notes-free UI primitives live in [BubbleBuffer/agent-kit](https://github.com/BubbleBuffer/agent-kit), a public Dart pub workspace shared with BubblePaw Desktop. Scratch's look is the kit's default.

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

`app/pubspec.yaml` declares `bb_ui` as a git dependency pinned to the commit of kit tag `v0.4.0` (`f1663dd`), because pub cannot take a tag ref for these path-based git packages:

```yaml
  bb_ui:
    git:
      url: https://github.com/BubbleBuffer/agent-kit.git
      ref: f1663dd9c6e8c9383543652e6fffde9165500a16
      path: packages/bb_ui
```

`bb_tokens` comes with it: `bb_ui` depends on it by a relative path inside the kit repo, which pub resolves at the tag's commit. Scratch reaches the tokens through `package:bb_ui/tokens.dart` (that is what `ui/tokens.dart` re-exports), so it does not declare `bb_tokens` itself. Declaring it with `ref: v0.1.0` fails version solving: pub pins the transitive copy to the commit and the direct one to the tag name, and treats them as different sources. `pubspec.lock` records the tag's commit as `resolved-ref`.

## Bumping it

1. Release the kit first: merge the change there with CI green and push an annotated tag `vX.Y.Z` (see the kit's `docs/package-layout.md`).
2. In Scratch, change `ref:` to the new tag, run `flutter pub get` in `app/`, and commit `pubspec.yaml` and `pubspec.lock` together.
3. If the kit changed anything visible, the pull request shows before/after screenshots and updates the goldens on purpose.

To try an unreleased kit change, point `ref:` at a branch or commit for a draft pull request, or use `dependency_overrides` with local `path:` entries; never merge either.

## The dev pane's packages

The Agent dev pane (`app/lib/addons/agent/kit_*.dart`, see [Agent](../addons/agent.md)) uses `agent_kit` and `agent_kit_core`, which `app/pubspec.yaml` declares directly. All three kit packages (`bb_ui`, `agent_kit`, `agent_kit_core`) are pinned to the commit of kit tag `v0.4.0` (`f1663dd`) and move together at each bump: move them to the next tag's commit. `agent_kit` depends on `bb_ui` by path, so pub resolves it as git at that commit and a tag ref on the direct dependency fails version solving.
