---
type: topic
layer: state
summary: "Current state of the Flutter bake-off prototype: what it does, how it is built and tested, and its known gaps."
parent: "[[prototypes/index]]"
status: outdated
covers: []
---

# Flutter prototype

**Status:** outdated, 2026-09-30. The prototype became the real app in [app/](../../app); its current state is [The Flutter app (state)](../architecture/app-state.md). This page describes the prototype as it was at the end of the bake-off. Spec: Bake-off (internal wiki).

## What it does

- The home screen lists every page in the vault (10,002 after seeding) with search and create. Desktop has a sidebar, "Recently visited" cards and an "All pages" list. Mobile has a "Recents" strip, the list, and a bottom bar with a search pill and a compose button.
- The editor supports paragraph, three heading levels, bulleted, numbered, to-do, quote, code and divider blocks, with nesting.
- Editing keys: Enter splits a block, Backspace at the start merges into the previous one, and Tab / Shift+Tab indent and outdent. The slash menu works, and so do markdown shortcuts (`# `, `- `, `1. `, `[] `, `> `, ```` ``` ````, `---`). Ctrl+B/I/E/K and Ctrl+Shift+S apply marks, and Ctrl+Shift+Up/Down move blocks.
- Inline formatting uses a floating selection toolbar on desktop. On mobile a keyboard toolbar is pinned above the soft keyboard; it scrolls sideways and has a format mode.
- Block actions: on desktop, a hover `+` and a drag handle (drag to move, click for the block menu). On mobile, long press opens a bottom sheet.
- Edits are saved per operation through `vault_core`, and back navigation flushes the page.
- The first launch against a vault without a "Bake-off showcase" page seeds the vault if it is empty, then creates the showcase page.

## How it is built

- The UI is `WidgetsApp` with no Material widgets. Icons are drawn with `CustomPainter`, and the tokens come from [design/tokens.json](../../design/tokens.json).
- Fonts are system fonts: Segoe UI and Consolas on Windows, the platform default (Roboto) on Android. No fonts are bundled.
- Each block is its own `EditableText`. The list is a lazily built `SliverList`, and controllers are created per visible block.
- `lib/backend.dart` hides the core behind a `Backend` interface. `RustBackend` goes through flutter_rust_bridge, and `MemoryBackend` stands in for it in tests.

## Testing

- `flutter test` runs the editor widget tests and the golden tests on a `MemoryBackend` seeded like the real vault.
- Golden screenshots are written to `prototypes/flutter/screenshots` (since moved to [app/screenshots](../../app/screenshots) and regenerated) at 1440×900 and 375×812, light and dark, with real fonts. The Android screencaps there come from a headless emulator.
- `measure.ps1` opens "Big page" using the `big-index` and `row-h` fields of the `BAKEOFF layout dpr` line, because the showcase is now the first row.

## Known gaps

- There is no Ask AI, underline, text colour, comments, page icons or covers.
- Undo is per block (Flutter's `EditableText` undo), not page-wide.
- Inline code backgrounds are square: Flutter text spans cannot draw rounded backgrounds.
- Code blocks wrap instead of scrolling sideways.
- The home screen is a simplified version of the reference app's.
