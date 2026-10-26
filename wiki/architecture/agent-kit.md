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

`app/pubspec.yaml` declares both packages as git dependencies on one tag:

```yaml
  bb_tokens:
    git:
      url: https://github.com/BubbleBuffer/agent-kit.git
      ref: v0.1.0
      path: packages/bb_tokens
  bb_ui:
    git:
      url: https://github.com/BubbleBuffer/agent-kit.git
      ref: v0.1.0
      path: packages/bb_ui
```

`bb_ui` depends on `bb_tokens` by a relative path inside the kit repo; pub resolves that to the same git package, so both entries agree. `pubspec.lock` records the tag's commit as `resolved-ref`.

## Bumping it

1. Release the kit first: merge the change there with CI green and push an annotated tag `vX.Y.Z` (see the kit's `docs/package-layout.md`).
2. In Scratch, change `ref:` in both entries to the new tag, run `flutter pub get` in `app/`, and commit `pubspec.yaml` and `pubspec.lock` together.
3. If the kit changed anything visible, the pull request shows before/after screenshots and updates the goldens on purpose.

To try an unreleased kit change, point both entries at a branch or commit for a draft pull request, or use `dependency_overrides` with local `path:` entries; never merge either.
