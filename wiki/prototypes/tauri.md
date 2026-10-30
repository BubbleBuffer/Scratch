---
type: topic
layer: state
summary: "Tauri 2 + SolidJS bake-off prototype: what is built, how it is tested headlessly, first measurements and known gaps."
parent: "[[prototypes/index]]"
status: current
covers: ["prototypes/tauri/**"]
---

# Tauri prototype

**Status:** current, 2026-09-30. The Tauri side of the Bake-off (internal wiki). Code: [prototypes/tauri](../../prototypes/tauri).

## How it is built

- The UI is SolidJS + Vite + TypeScript in `src/`. The Rust side is in `src-tauri/`.
- [backend.rs](../../prototypes/tauri/src-tauri/src/backend.rs) wraps `vault_core`:
  - It opens the vault and seeds it on first launch (10,000 pages plus the 2,000-block "Big page").
  - It creates the "Bake-off showcase" page once. A marker file stops it from coming back after the user deletes it.
  - It dispatches every command by name.
  - It writes `BAKEOFF` measurement lines: to logcat on Android, and to stdout plus `<app data>/bakeoff.log` on desktop.
- [lib.rs](../../prototypes/tauri/src-tauri/src/lib.rs) is a thin layer of Tauri commands over that backend. Vault calls run on the blocking pool.
  - The window starts hidden and setup shows it.
  - Setting `NOTES_TAURI_HIDDEN=1` runs the full app with no window. The headless start check uses this.
- [api.ts](../../prototypes/tauri/src/api.ts) calls Tauri `invoke` inside the app. In a plain browser it calls `POST /invoke/<cmd>` instead.
- The editor uses one contenteditable per block.
  - Blocks render progressively, 150 per frame, with `content-visibility: auto`.
  - Marks use code-point offsets, as in the core.
  - Edits go to the core as `BlockOp` batches.
- The home list is virtualized. It uses a fixed pool of rows under a header with the recent cards.
- Styling is CSS variables ([styles.css](../../prototypes/tauri/src/styles.css)) taken from the measured tokens in [design](../product/design.md).
  - Light and dark mode follow the system.
  - Fonts are the system stack, with Roboto on Android. No web fonts are bundled.
- Desktop layout:
  - A 240px sidebar with the first 30 pages.
  - A 720px content column.
  - A selection toolbar for formatting.
  - A hover drag handle that opens a block menu.
- Mobile layout:
  - A mobile home: "Notes" title, Recents cards, and a bottom search bar with a compose button.
  - A keyboard toolbar with a formatting mode.
  - Long-press opens a bottom sheet.
  - The system back button closes the sheet, then the page.

## Testing (headless)

- `examples/bridge.rs` is a std-only HTTP server.
  - It serves `dist/` and routes `/invoke/<cmd>` to the same backend.
  - Build it with `npm run bridge`, which runs `cargo build -p notes-tauri --release --example bridge`.
- Playwright runs headless Chromium against two bridges:
  - Port 4480, vault `target/tauri-bridge-data`, for the interaction tests.
  - Port 4481, vault `target/tauri-bridge-shots`, for the screenshots.
  - Run `npm run build && npm test`.
- Projects: desktop at 1440×900, and mobile at 375×812 with DPR 2, touch and an Android user agent.
  - Mobile emulation simulates the on-screen keyboard with `window.__bakeoffKeyboard(px)`.
  - Long-press is sent as touch events over the Chrome DevTools Protocol (CDP).
- [editor.spec.ts](../../prototypes/tauri/tests/editor.spec.ts) covers:
  - Home, virtualization and search.
  - Enter splitting and Backspace merging blocks.
  - Markdown shortcuts, Tab and Shift+Tab, and the slash menu.
  - The selection toolbar and the keyboard toolbar.
  - The to-do toggle.
  - The drag handle and its menu, and the long-press sheet.
  - Persistence across back navigation and reload.
  - The 2,000-block page.
- [screenshots.spec.ts](../../prototypes/tauri/tests/screenshots.spec.ts) writes `screenshots/{desktop,mobile}-{light,dark}-*.png`.
- Android: the release x86_64 APK runs on the `notes-tauri` emulator started with `-no-window`, driven through adb only. `measure.ps1` gathers the bake-off measurements.
- The Playwright Chromium is revision 1243, which is not installed. The config points at the local `chromium_headless_shell-1234`; set `CHROMIUM` to override.

## First numbers (2026-09-30, not the bake-off measurement run)

- Release x86_64 APK: 14.2 MB.
- Windows exe (`target/release/notes-tauri.exe`, not bundled): 15.9 MB.
- Android emulator (SwiftShader GPU):
  - The first-launch seed took 107 s. The bridge on the desktop CPU took 18 s.
  - Cold start `TotalTime` was 1.1–2.3 s. `list_visible` was 0.4–1.0 s after the vault had been seeded.
  - The showcase page opened in 90–110 ms.
- Windows desktop: `list_visible` was 315 ms on an existing vault.

## Known gaps

- Drag-to-move works only on desktop, through the hover handle; the tests cover the handle's menu, not the drag itself. On mobile, blocks move with Move up/down in the sheet.
- There are no page icons, covers, nested pages or sidebar tree. The sidebar is a flat list of the first 30 pages.
- The code block has no syntax highlighting or language picker. Its "Copy" button copies the text.
- There is no multi-block selection. Undo and redo are disabled (Ctrl+Z/Y do nothing).
- The mobile keyboard toolbar is wider than 375px, so its last buttons scroll horizontally.
- The Windows build is not bundled. The NSIS installer would need a tool download, so it was skipped.
