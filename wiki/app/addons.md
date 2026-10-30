---
type: topic
layer: state
summary: "The app as a thin shell plus a shared kit, with every feature an isolated add-on that registers types, blocks, views, commands, codecs, search sources and agent tools; enforced in CI."
parent: "[[app/index]]"
status: current
---

# Add-ons

**Status:** Steps 1 (Board/Database) and 2 ([Agent](../addons/agent.md)) and the kit's generic embeds (refactor 4) are implemented.

## Layers

1. **Kit**: the shared base. Stacks UI widgets (`app/lib/ui/`), the registry interfaces, and a services facade over core (vault, index, sync, agent client, settings). Kit changes are serialized through the lead session because everything depends on them.
2. **Add-ons**: one folder each, `app/lib/addons/<id>/`. A public `api.dart`, private files, tests in `app/test/addons/<id>/`, a page in `wiki/addons/<id>.md`. Each add-on has an `Addon` entry with `id`, `dependsOn` and `register(AddonContext)`.
3. **Shell**: boots the app, holds the list of add-ons, and asks the registries what to draw. No feature special cases (such as `isDatabase` branches) in the shell.

## What an add-on registers

Through `AddonContext`: note types (chip, card, embed and full-page builders), block kinds (renderer plus Markdown, clipboard and plain-text codecs), views, commands, slash items, settings sections, search sources, shell panels and agent tools.

### Shell and note extension points

- **Navigation:** `ctx.shellNavigation` accepts `AddonShellNavigation` entries with an id, label, icon, order and open callback. The sidebar renders them in its top group; `mobileDock: true` opts an entry into the phone dock between Today and Search. `ctx.shellPrimaryActions` supplies the floating create action for named destinations. Registrations belong to the booted context, so disabling an optional add-on removes its navigation and actions at the next start.
- **Note dragging:** `NoteDragPayload` identifies a note by `pageId`, with optional title/type metadata. Sidebar rows publish it while retaining their existing reorder callbacks. Add-on `DragTarget`s interpret the payload and own their writes and Undo feedback.
- **Board cards:** `NoteType.boardCard` is an optional note builder. Board uses it for typed rows and otherwise keeps `RowCard`; Kanban prefers it over the type's ordinary `card`. The shared renderer continues to own the card's drag and selection behavior.
- **Title chips:** `ctx.titleChips` accepts `AddonTitleChip` providers receiving the current `PageData`. `NoteTitleChips` renders non-null widgets after the title in registration order. Providers own their data and navigation; the host owns the title editor.
- **Inline marks:** `ctx.inlineMarks` accepts `AddonInlineMark` providers (`app/lib/registry/addon_registry.dart`), keyed by the mark's `BMarkKind` and the mark's `url` as payload. The editor (`RichController.syncInline`) creates one state per mark through `watch(context, payload)`, reads the painted text through `label(state)`, and disposes the state when the mark or its block goes away. A tap on the mark calls `open(context, anchor, InlineMarkRef)`, which carries the payload, the state, the block and the host. The kit stays generic: the add-on converts the paragraph through `BlockHost.replaceWithBlock`, `insertBlockAfter` and `replaceWithParagraph`, which go through the normal undo path.

## Rules, checked in CI

- An add-on imports only the kit, its own folder, and the `api.dart` of add-ons in its `dependsOn`. An import-check script fails the build otherwise.
- A boot test starts each add-on with only the kit and its dependencies.
- A contract test checks every registered block kind has all three codecs.
- A parallel work lane owns one add-on folder, its tests and its wiki page.

## Current state (2026-10-13)

- **Generic embeds (kit):** `embed` is registered by the kit, independently of the add-on list. It stores only the target id, resolves current metadata from the vault or exact external index identity, and draws through the target type. The optional embed builder falls back to its card. Documents use read-only block previews; cycles and drawing beyond four embeds use chips. Unknown/disabled types retain title and fallback. The existing page picker handles `/embed`, and a page link can turn into Embed without selecting its target again. New Database inserts create an embed target; old `database` blocks retain their original rendering and storage kind. Whole-chat cards draw the session title and chapters; `chat_ref` is unchanged.
- **API:** `app/lib/registry/addon_registry.dart` defines `Addon`, isolated `AddonContext`, the dependency sorter and boot function. Contexts expose note types, block kinds, views, properties, commands, slash items, settings sections, search sources, shell panels and agent tools. Registration does not open a vault or start background work. Registered search sources supply group/type metadata, opaque row targets and navigation. Indexed sources share core ranking and filters; callback sources supply their own rows. Grouped Ctrl+K/mobile search consumes these registrations generically. Agent tools remain an interface; their Rust protocol is unchanged. `shellPanels` supplies optional shell wrappers, header actions and screen builders.
- **Note types:** `note_type_registry.dart` has chip, card, embed and full-page builders plus commands, source-owned embed targets and cache callbacks. `chat` declares its body source as the agent store and its range card as the type's embed. `editor/page_screen.dart` selects the body through `NotePageBody`. Unknown types keep the existing markdown editor fallback. Optimistic conversions notify the registry before page metadata reloads.
- **Fixed list and boot:** `app/lib/shell/addons.dart` lists required `legacy`, `document`, `database`, and optional `board`, `kanban`, `agent`, `tasks` and `search_card`. Boot validates the whole dependency graph before registering anything, then registers dependencies first; duplicate ids, cycles and missing dependencies are errors. `main.dart` initializes Rust; `shell/boot.dart` registers add-ons using device settings before opening the vault. Test harnesses keep the compatibility name `registerBuiltins`, now in the shell.
- **First add-ons:** [Database](../addons/database.md) owns the database note type, inline block, controller, property implementations and its five layouts. [Board](../addons/board.md) owns the Board layout and live-query source, declares `dependsOn: ['database']`, and consumes the database public API. [Agent](../addons/agent.md) owns desktop/Hub chat, outlines/references, settings, commands, shell surfaces and the indexed search source, with no add-on dependencies. `document` is thin; `legacy` wraps the remaining non-agent block and shell-command registrations.
- **Kit:** the storage-independent board renderer stays in `app/lib/ui/board/`. Shared property/view configuration, values, icons and widgets live in `app/lib/ui/database/`; `ViewController` is an interface in the view registry. `app/lib/services/addon_services.dart` resolves the host vault, change feed, settings and selected page, plus the agent's explicit transitional host UI/navigation, AI settings/task notification, sync/login and mention/styling surfaces. `services/legacy_host.dart` explicitly exposes the existing navigation, template and attachment-picker functions while those features await their own add-ons. The collection store remains in `state/`.
- **Codecs:** `BlockType.codecs` carries Markdown, clipboard and plain-text encoders. `registry/block_codecs.dart` holds the existing clipboard conventions as data; the legacy adapter attaches them to registrations. The database block explicitly keeps its old text fallback. Unknown kinds keep the generic fallback, and the contract test rejects an installed kind without codecs. The Dart paste parser stays shared. `MarkdownCodec.rust(id)` can reference the core's writer; Rust codecs and bridge API are unchanged.
- **Optional settings:** Settings → About → Add-ons → Manage opens the toggle list. Required entries cannot be disabled. Board, Kanban, Agent, Tasks and Search card are optional, default on, and changes take effect at the next start. Disabling Agent removes its registrations, prevents the store from opening, preserves chat references as unknown-block placeholders and hides cached chat search rows. Disabled ids live in the device's `settings.json`, never the vault; stale ids and attempts to disable required entries are ignored. An enabled add-on with a disabled dependency fails boot.
- **Enforcement:** `scripts/check-addon-imports.dart` resolves relative/package imports, exports, conditional directives and parts. Add-on-to-add-on edges require both the target `api.dart` and a declared dependency; kit UI, registries and services cannot import any add-on. Core, state and the generated bridge are part of the migration kit. Only the `legacy` adapter may directly reach the remaining old feature folders; migrated add-ons use the host facade. The checker also checks tests under `app/test/addons/<id>/`, graph errors and unresolved local imports. Its `--self-test` plants violations in temporary fixtures. `app/test/addons/` checks dependency boot, each add-on alone with its dependency closure, registry isolation, optional selection, note-body fallback and block codec completeness.

- **Kit page picker:** `services/legacy_host.dart` exports `Future<String?> pickPage(BuildContext context, Rect? anchor, {String? exclude})` for add-ons to open the existing page picker.
- **Kit inline marks (2026-10-16):** the editor paints and opens marks that an add-on's `AddonInlineMark` supplies, and `RichController` no longer has a fixed `queryLabel`. Search card registers `search_card.query` for `BMarkKind.query` (see [Search card](../addons/search-card.md)). `services/legacy_host.dart` exports `showSearchOverlay`, which the chip's "Open in Search" uses.
- **Kit board columns:** `BoardSource.deleteColumn` is an optional `Future<void> Function(String columnId)?` hook (default `null`); the renderer offers confirmed deletion only with multiple columns, and the source keeps cards as notes and moves them to the first remaining column.

### Generic embed acceptance (2026-10-12, Windows)

Branch `embed` starts at master `3c5b8b5`. The core kind, kit renderer/guards, database insertion switch, existing picker hooks and whole-chat builder extension are implemented. Only `chat_type.dart` changes inside Agent; `chat_ref` and its existing range path are unchanged. The database add-on changes only its block insertion declaration; its legacy body and `DatabaseView` remain intact. The other UI lane's title/alignment files and `.github/` are untouched.

| Check | Result |
| --- | --- |
| Full `cargo test -p vault_core -j 4` | Passed on final core source; includes import, sync, mounts and search suites. |
| Independent core regressions | 28 passed: 12 embed, 1 exact external metadata, 5 Markdown API, 6 page-type and 4 unknown-kind cases. |
| Bridge every-kind round trip | Passed, with `embed` included by the one-declaration built-in list. |
| `bash scripts/regen-bindings.sh` | Passed; final regeneration changes zero generated-file hashes. |
| `cargo build --release -p notes_bridge -j 4` | Passed after all Rust changes; tests load this worktree's DLL. Release fake-agent examples also rebuilt. |
| `flutter analyze --no-pub` | No issues on final source and tests. |
| `dart scripts/check-addon-imports.dart` | Passed for 5 add-ons / 379 files / 2074 resolved directives. |
| Full `flutter test --no-pub --concurrency=1 --exclude-tags drive` | 1003 passed, 8 skipped, 1 failed in 30m11s. All 41 new embed/chat checks and every executed golden comparison pass. |

The only full-run failure is the existing `db_perf_test.dart` → `10k rows scroll in a table`: p95 158.3 ms against the unchanged 60 ms limit. Its isolated rerun times out settling the 10k-row fixture with one native call pending, before measurement. The same table test on an exact `3c5b8b5` checkout also fails the 60 ms gate, at p95 406.3 ms. That comparison loads the existing `addon-agent` release DLL; its Rust source trees match the base and its committed bridge ABI hash is identical (`-1371857031`, codegen 2.13.0). No assertions, timeouts or images were changed. This leaves the table timing/fixture gate unqualified on this shared machine; the broader performance repair is outside the embed refactor.

The bridge DLL, release fake-agent fixture and explicitly selected BubblePaw debug fake-extension hashes are unchanged across the full suite. Whole-chat card content uses a fake source in its widget tests; generic type dispatch and disabled-at-boot indexed fallback are tested independently. Packaged/device execution, production Agent/Hub availability and drive-tagged paths are not qualified by these local checks. Separate folder imports preserve unresolved old target ids; only a single import containing both source notes remaps their ids.

### Step 2 acceptance (2026-10-12, Windows)

`addon-agent` starts from master `f29eddf` (after `8f59795`). The Agent extraction, chat type/range embeds and generic search registrations are implemented. The source import checker passes for five add-ons; its 26 planted-edge self-tests pass without changing its rules. Existing test bodies, assertions and timeouts are unchanged apart from import/format edits; the Board-only boot fixture additionally disables Agent so its original assertions still test the original required set. Optional-agent integration tests use the host bootstrap in `app/test/addon_harness.dart`, keeping named add-on tests inside their import boundary.

| Check | Result |
| --- | --- |
| `cargo build --release -p notes_bridge -j 4` | Passed once; tests load this worktree's `target/release/notes_bridge.dll`. |
| `cargo build --release -p vault_mcp --examples -j 4` | Passed; the full suite uses this worktree's release fake-agent fixture. |
| BubblePaw `cargo build -p bubblepaw-runner --bin bubblepaw-fake-extension -j 4` | Passed; agent tests use that debug executable explicitly. No BubblePaw source changes. |
| `flutter analyze --no-pub` | No issues on the final source and tests. Direct `dart analyze` also passes. |
| Chat type/disabled-agent, boot and existing reference checks | 30 passed, including actual registered-type dispatch and a real page opening with a disabled `chat_ref` placeholder. |
| Existing grouped-search/result/golden checks | 37 passed; all four grouped-search images match unchanged. |
| Fake registered sources, existing chat anchors and page-only scope | 10 passed, including Ctrl+K without Agent, opaque-target activation and hiding cached unregistered rows. |
| `flutter test --no-pub --concurrency=1 --exclude-tags drive` | 962 passed, 8 skipped, 1 failed in 32m25s. The only failure is the user-reported master test `home_first_run_test.dart` → `/ menu the menu shows Results while filtering` (two Basic blocks headings rather than one). |

All existing agent, mobile-agent, chat-outline and chat-reference checks pass in the full run. Every executed golden comparison passes; no image baseline was updated. Both Ctrl+K and agent-mention timing gates pass in that run, so no isolated timing rerun is needed. The native bridge and fixture hashes are unchanged across the full run. Rust crates, bridge sources/bindings and `.github/` are unchanged. Packaged/device execution, production Hub/provider operation and drive-tagged tests are not qualified by these local checks.

### Bridge merges and CI

Local acceptance on Windows, from base `9744eab`: `flutter analyze` is clean; the import checker and its 26 planted-edge cases pass; regeneration with codegen 2.13.0 produces no diff. `flutter test --concurrency=1 --exclude-tags drive` completed with 882 passed, 6 skipped and only the two independently reproduced master failures (slash-menu Results headings and search snippets). All golden comparisons passed, including the Stacks page-block images; no images or existing assertions were updated. A separate desktop/mobile test of Settings → About → Manage passed both cases. The first combined focused run ended one board-renderer case as "did not complete"; its independent rerun, master comparison and full-suite run passed it. CI on the self-hosted CI runner and packaged/device execution are not verified by these checks.

Generated bindings stay committed. `.gitattributes` selects the `ours` merge driver for `app/rust/src/frb_generated.rs` and `app/lib/src/rust/frb_generated*.dart`. Once per clone, configure the driver:

```sh
git config merge.ours.driver true
```

After a merge, regenerate with `bash scripts/regen-bindings.sh` (codegen 2.13.0 and Flutter/Dart must be on PATH). The script runs `flutter_rust_bridge_codegen generate` in `app/` from any working directory. `merge=ours` keeps the local side of conflicting generated files; regeneration reconciles the actual API afterward.

### Kit type data

Use `ctx.services.vault(context)` to call `typeData(page, addonId)` or `typeDataAll(page)`; both return JSON strings. Write JSON values with `typeDataSet` and remove keys with `typeDataDelete`. Paths are string segments from the add-on's root map; list segments select map elements by their `id`. `typeDataListInsert` appends or takes an index, and `typeDataListDelete` / `typeDataListMove` take a `TypeDataListItem` with exactly one of `index` or `id`. These Vault methods persist, sync and wake the change feed.

Create a typed page with `vault.createPage(parent: parent, title: title, pageType: typeId)`. Its type is set atomically before the page appears. Use `vault.setPageType(page, typeId)` for conversion. Type data belongs to the add-on id independently of the page type, so conversion and an unavailable add-on retain it. Markdown stores the area under the reserved `type-data` front-matter key.

## How to write an add-on

Create `app/lib/addons/<id>/api.dart`, keep implementation files inside that folder, and add its entry to `shell/addons.dart`. Public imports of another add-on must use its `api.dart`, and its id must appear in `dependsOn`. Use literal getters for ids and dependency lists so the checker can inspect them without booting Flutter.

```dart
import 'package:flutter/widgets.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/registry/note_type_registry.dart';

class ExampleAddon extends Addon {
  const ExampleAddon();
  @override
  String get id => 'example';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    Widget title(BuildContext context, NoteArgs args) => Text(args.title);
    ctx.registerNoteType(NoteType(
      id: 'example', label: 'Example',
      chip: title, card: title, embed: title, fullPage: title,
    ));
  }
}
```

For a block kind, declare `BlockType` with a renderer and explicit `BlockCodecs`, then register it with `ctx.blockKinds`. For a view, register a `ViewType` with `ctx.views`; the optional `before` parameter preserves menu placement independently of boot order. Register note-type commands with `ctx.registerNoteType`; it also adds them to `ctx.commands`. Slash items are consumed by the editor; a block-backed item declares `blockKind` and uses the registered block menu/insertion path. Settings sections default to the Add-ons settings panel and can declare a host `location`, `placement` and visibility callback (Agent uses the existing AI-page slots). Shell panel builders are consumed by the shell and header. Search sources declare group/type and row presentation metadata, whether they use the shared index, a query callback for independent sources and an open callback receiving the opaque target. Unregistered external hits are hidden. Do not put vault work in `register`: use `ctx.services` when handling a UI action.

Put tests in `app/test/addons/<id>/`, add a short page to `wiki/addons/`, and run the import checker, the boot/codec tests, relevant existing tests and `flutter analyze`. The boot test iterates the fixed list, so a new entry gets isolation coverage automatically. Keep existing golden assertions and images unchanged during structural moves.

For a note type, `embed` is optional; omitting it uses `card`, while existing callers still call `type.embed(context, args)`. Use the inherited drawing context when composing child embeds so `EmbedScope` keeps the ancestor chain. Vault document bodies are supplied in `args.editor` as read-only block previews. A `BlockType` can declare `insertKind` and `createTarget` when a compatibility menu entry should create a target but write the kit's embed kind; Database uses these hooks without replacing its legacy body. The host opens empty embeds through `BlockHost.openBlockPicker`.

## Decisions

- 2026-10-12: Everything is an add-on except the kit and shell.
- 2026-10-12: Dart first; an add-on gets its own Rust crate and bridge file only when it needs Rust.
- 2026-10-12: Add-ons are compiled in and listed in one place; optional ones can be switched off in settings.
- 2026-10-12: Bridge files stay committed and are regenerated by a script after a merge.
- 2026-10-12: Refactor 3 became this add-on API; refactor 4 is a kit feature; per-kind codecs are part of block registration.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Architecture > Add-ons).
