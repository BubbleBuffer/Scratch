---
type: topic
layer: implementation
summary: "The database note type, inline block, property implementations and five layouts."
parent: "[[addons/index]]"
status: current
---

# Database

`DatabaseAddon` in `app/lib/addons/database/api.dart` is required and has no add-on dependencies. It registers:

- The `database` note type: chip, card, inline embed and full-page builders. Full pages draw through `NoteTypeRegistry`; `madeDatabases` notifies optimistic conversions immediately.
- The existing `database` inline block with its explicit clipboard/plain-text fallback, in its original slash-menu position. New inserts use the kit's `embed` kind and create a database target. Old `database` blocks keep their original renderer and are not rewritten on read; both presentations use the same inline `DatabaseView`.
- The existing property implementations and Table, Timeline, Calendar, List and Gallery layouts. Board registers separately.
- `database.make`, the existing turn-into-database action exposed as a type command, enabled when the host has a selected page.

Files: `database_view.dart` is the frame; `controller.dart`, `create.dart`, `page_properties.dart`, filter/property/view settings and `editors/` provide its behavior; `properties/` owns property implementations; `views/` owns the five layouts and shared card helpers; `blocks/database.dart` is the inline block. The public `api.dart` exposes only the pieces used by Board and transitional app consumers.

Shared config, value helpers, icons, theme and widgets are kit in `app/lib/ui/database/`. `CollectionStore` remains in `state/`; `ViewController` is the registry interface. Navigation, templates and attachment picking use the temporary host facade in `services/legacy_host.dart`.

Existing database tests and golden assertions stay in place. `app/test/addons/addon_boot_test.dart` checks standalone boot, type registrations, optional Board absence, original layout order and the codec contract.

## Performance

Opening an empty database page keeps the editor's first paragraph local until the first block edit (2026-10-12). It therefore does not write to the vault, wake the change feed or restart the document scan. The insert is persisted together with the first edit, using the existing editor row and undo behavior. The 200-row regression in `app/test/empty_page_open_test.dart` checks desktop and mobile opening, first-line persistence and undo/redo. The `prepareTasks` workaround is removed from `db_perf_test.dart`.

The 10k-row gate (`app/test/db_perf_test.dart`, debug mode, p95 of 480 frames < 60 ms) is timed on linux01 (Intel i5-1135G7, 4 cores, 8 GB, Flutter 3.47.7), not on the shared Windows PC. At 7b9c19c it passes 3 of 3 runs (2026-10-13):

| Run | Table p50 / p95 / max (ms) | Board p50 / p95 / max (ms) |
|---|---|---|
| 1 | 16.5 / 30.1 / 128.2 | 7.7 / 15.6 / 47.3 |
| 2 | 14.8 / 28.0 / 34.9 | 6.5 / 10.5 / 21.1 |
| 3 | 23.6 / 31.9 / 65.4 | 7.9 / 11.5 / 18.4 |

No table-renderer change was needed to pass. linux01 had other users logged in, so the spread is noisy. Windows PC timings under heavy load (p95 382 ms on 2026-10-12) are not meaningful for this gate.

The previous open-time insert caused `PagesStore` to reload and scan `listPages` in 200-row chunks, filtering out database rows until it found enough documents. In the 10k-row investigation that scan rebuilt 5,512 widgets per opening frame; native list calls took 9–23 ms. This was the page-list scan, not the search indexer. Real edits used to trigger the same `PagesStore` reload and scan. Content edits now patch the store in place (see below).

### Edit cost on the 10k-row vault

A content edit patches `PagesStore` in place instead of reloading and re-scanning the vault; structural changes still reload (see [app state](../architecture/app-state.md)). `app/test/edit_perf_test.dart` types one character into an ordinary page with the 10k-row Tasks collection present, 20 times, and waits until idle. Command: `flutter test --concurrency=1 test/edit_perf_test.dart`, debug build with the release bridge, on linux01 (2026-10-13). Each cell is the median of three runs; each run's p50 and p95 are over the 20 edits.

| Metric | Before (7cb34fb) p50 / p95 | After (b0e309b) p50 / p95 |
|---|---|---|
| Wall time until idle (ms) | 6618 / 9351 | 396 / 1065 |
| PagesStore notifications per edit | 54 / 54 | 1 / 1 |
| Bridge calls per edit | 73 / 73 | 23 / 23 |
| Debug frame time (ms) | 111.7 / 194.8 | 66.5 / 194.7 |

Frames per edit fall from 54 to 4. Frame p95 is unchanged: the fix has about 63 frames per run against about 950 before, so its p95 rests on few samples, and in two of three paired runs it was higher. Machine load (1-minute average) ranged from 4.5 to 12.4 across the runs, interleaved base and fix.

Local Windows validation: 295 tests across 26 files pass, including the database, editor and page goldens; no golden baselines are updated.
