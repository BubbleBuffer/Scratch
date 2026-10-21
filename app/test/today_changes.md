# Today test changes

## Current checkpoint and validation status

The reviewed merged checkpoint is `854783c` (merge of `origin/master`, currently `a973780`). Inline Journal editing and the 62 Linux golden baselines were committed in `cfb375e`; the desktop overlay corrections and additional menu tests are in `1da97e3`. The already-pushed WIP checkpoint `1859c26` is retained in history. Its only code delta captures `PageStore` before navigation and awaits `lastSave` after the full-page route returns, before remounting the inline editor. Further corrections and this status update belong in normal follow-up commits; the pushed checkpoint is not being rewritten or squashed.

All 62 PNGs in the inventory below are tracked at this checkpoint, including all four `day-with-note` variants. On linux01, the rebuilt merged checkpoint passed `flutter analyze`, the add-on import boundary check, all 62 requested golden comparisons, and one whole fast-tier invocation: 135 files, concurrency 1, 903 passed, 8 skipped, 0 failures in 13:59. All 14 inline-note cases and all six sync cases passed within that invocation. No builds, rendering or tests ran on the PC. CI results belong to the final pushed-commit handoff.

The original sync flake remains unexplained. It timed out after Replace at `app/test/sync_test.dart:182` with one pending Vault future, but the original log did not identify the native call. The test and resolution path (`app/lib/state/sync_store.dart:240-242`) match merged master. Forty earlier isolated file repetitions, an earlier full-tier run, and this merged full-tier run passed. The existing opt-in `NOTES_TRACE_BRIDGE_CALLS=1` trace was enabled for this run; without a reproduction it cannot identify the old call. No sync behavior, assertions or timeout was changed, and no speculative external cause or fix is claimed.

The approved date strip and compact day cards replace the greeting, week controls, timeline, In motion and Recent-card body. Capture, agenda persistence, source opening, shell navigation and editor behavior remain covered.

Each entry includes its desktop/phone or light/dark variants where present.

| Changed or removed test | Reason and retained behavior |
| --- | --- |
| Today ticks a database task and dated to-do | Short-card checkbox finders replace old agenda-row keys; persisted database and block completion remain asserted. |
| Select a day, move a week, and return to today | Strip selection and Today replace withdrawn week controls; selected-day items and overdue-on-today remain covered. |
| Capture appends dated task then paragraph to one daily note | Title, UTF-16 date marks, clearing, note reuse and opening remain asserted; captured content now precedes Log. |
| Today’s note opens and reuses the daily note | Retained with scroll reveal before helper taps; opening twice must reuse the page. |
| Dated to-do opens its page at the block with a brief highlight | New card finder; source block, viewport and temporary highlight assertions retained. |
| Completing an overdue task moves it into done and unticking returns it to Overdue | Opens the new overdue disclosure; completion, reopening and overdue counts remain asserted. |
| In motion reflects checklist ticks made in a page | Retired progress-card presentation removed; unchanged store and seed tests cover checklist progress. |
| Mobile capture sits at the top of Today, above the dock, and the dock clears the keyboard | Retains capture/dock/keyboard geometry without requiring the withdrawn capture-first order. |
| Narrow desktop stacks capture actions without overflow | Retained against the new body. |
| Now marker advances without a vault edit | Removed with the retired timeline; clock/midnight store tests remain unchanged. |
| Due tasks render fifty at a time and reset after changing days | Five-card increments match the mockup; expansion and selected-day reset remain checked. |
| Overdue and expanded done groups show fifty before loading more | Disclosure and card expansion replace old layout; loading beyond the existing fifty-item overdue store bound remains checked. |
| Today uses real status groups, keeps overdue chips and reverses completion | Real status-group completion/reopening retained through short cards; removed status-chip styling is no longer asserted. |
| First-run Home says each thing once | Selected strip, day agenda and capture replace greeting/summary/Recent assertions. |
| Another day reads its date and counts | New day heading/items replace the old week peek. |
| The done pile appears once a task is done | Persisted completion and the new done presentation replace the old All done pile. |
| Recent cards sit in an even grid without overlap | Removed with the retired Recent-card grid. |
| Greeting and recently visited cards, newest visit first | Newest-first visited state and sidebar reopening replace retired cards. |
| Mobile dock: tree, search with paths, trash, new page | New selected-day heading replaces greeting assertions; navigation/search/trash/create remain checked. |
| Mobile tree closes with a scrim tap | New heading replaces greeting; panel dismissal remains checked. |
| Untitled page not open goes by its first line | All pages replaces Recent; first-line label, italic and tertiary styling remain checked alongside sidebar/breadcrumb/pickers. |
| Fling right/left opens/closes tree and agent panels | Selected-day heading replaces the removed greeting drag target; gesture behavior remains checked. |
| Tree follows finger; short release closes; drag past halfway opens | New body drag target; thresholds and panel geometry remain checked. |
| Horizontal drag on Recent scrolls Recent and moves no panel | Date-strip scrolling now guards nested gesture arbitration, unchanged selected day and closed panels. |
| Vertical scroll of Home moves no panel | Explicit vertical body scroll also verifies the date strip stays pinned. |
| Today last due tick celebrates exactly once under normal/reduced motion | Short-card completion, core checked state, source colour and reload replace retired Today confetti; page-editor celebration tests remain untouched. |
| Today home/another-day/empty golden sources | Updated controls and compact-card setup; desktop/phone and light/dark baselines are tracked in `cfb375e`. |
| Today day-with-note golden sources | Enabled all four previously skipped variants after inline editor authorization. Seeds a dated item and daily-note paragraph before Log, waits for the mounted editable block, and checks that both item and paragraph are visible with the paragraph below the item before capture. |
| Today status golden sources | New disclosure/card setup replaces removed status chips; baselines are tracked in `cfb375e`. |
| First-run home golden sources | Sources target the new selected heading; baselines are tracked in `cfb375e`. |
| Shell Favorites/recents golden sources | Favorites uses Pages; the former Recent source uses Today’s page disclosure. Baselines are tracked in `cfb375e`. |
| Boot without Tasks removes the Calendar sidebar entry | Also verifies Today’s Add task control follows the same disabled-add-on contract. |
| Phone: connect to a hub with a pairing code; sync runs in the background | Opens Today’s All pages disclosure before checking imported titles; real pairing, background updates and two-way sync assertions remain intact. |
| Task composer temporary link-hook tests | Replaced by real-core Journal-link assertions after the core merge. |
| Existing task-page save and rescheduling tests | Add new-day and historical Journal links while preserving scheduling/body assertions. |
| Calendar task-note drop and Undo | Add old/new Journal-link assertions while retaining date/time and Undo checks. |
| Today day with note desktop-light | Re-enabled after shared PageScreen embedding; requires editable note content visible below a selected-day task. |
| Today day with note desktop-dark | Same real-editor/content/order check in the dark desktop theme. |
| Today day with note mobile-light | Same real-editor/content/order check in the phone viewport. |
| Today day with note mobile-dark | Same real-editor/content/order check in the dark phone viewport. |
| Shared settle diagnostics | Optional named native-call traces improve evidence for the conflict-edit timeout; assertions, timing and wait behavior are unchanged. |

New widget tests cover browsing/selection/Today, jump validation across a year boundary, keyboard focus, read-only day viewing, `dayNotesInRange` marks, separate strip filtering, selected-day visibility, event start-day placement, and category/Agent metadata after completion. New task tests cover dated Today and Calendar creation, deduplication, undated saves, date clearing, retained history and nonfatal link errors. The new capture test verifies insertion above Log and unchanged log children.

The shared database test navigator now targets the page’s sidebar row by ID, rather than a duplicate agenda source label. The shared showcase navigator uses the canonical sidebar/Pages row instead of removed Today Recent cards. All database agenda and editor assertions remain unchanged.

`today_day_note_test.dart` now defines 14 cases: the original 11 inline-note cases and three overlay/bootstrap-menu cases. It uses the ordinary `PageScreen` in its generic `embedded` mode. The exact test names are:

| Test name | Variants and contract |
| --- | --- |
| `view and focus do not create a day note on $form` | Desktop/mobile: view, focus and another empty date remain read-only. |
| `first typing creates and then edits the real inline note on $form` | Desktop/mobile: real editor, focus handoff, `Tasks`/paragraph/`Log` order, continued editing, and full-page edit followed by persisted inline return. |
| `existing ${isoDate(day)} note is editable inline on $form` | `2026-10-04` and `2026-10-09`, desktop/mobile: existing past/future pages are reused, selected-day edits persist, and Log children start hidden. |
| `first edit on a future day stays with that day when switching on $form` | Desktop/mobile: rapid selection change retains the old day's draft and leaves the next day uncreated. |
| `long inline note keeps mobile toolbar and slash menu above keyboard` | One mobile case: forty-line note, toolbar/menu viewport bounds and keyboard dismissal. |
| `short desktop note menus accept real taps beyond the document body` | One desktop case: slash, selection/link, block and mention menus accept actual taps through the screen overlay. |
| `bootstrap slash opens ordinary block menu on ${mobile ? 'mobile' : 'desktop'}` | Desktop/mobile: first slash typing creates the note and hands off to the real slash menu. |

The full-page-return assertions are in the two first-typing cases. They check the edited text in both the remounted inline editor and stored blocks. They do not inject a delayed save to force the race addressed by `1859c26`, so that barrier's regression coverage is behavioral rather than a deterministic save-race reproduction.

The four enabled golden tests are named `Today day with note desktop-light`, `Today day with note desktop-dark`, `Today day with note mobile-light`, and `Today day with note mobile-dark`. Each requires the integrated editor to show an editable paragraph below a real selected-day item. Explicit agenda reload and the mounted-editor wait cover writes made after Home mounts; a date-strip marker alone cannot satisfy capture assertions.

The merged core still creates ordinary Journal rows with `Tasks` and `Log` toggle roots in that order (`crates/core/src/agenda.rs`, `daily_note`). Its existing `daily_note_is_idempotent_starts_with_sections_and_finds_renamed_journal` test covers lazy lookup, reuse and section names; `crates/tools/tests/agenda.rs` includes `daily_note_seeds_tasks_and_log_and_preserves_added_content`. Inline creation inserts a paragraph before the top-level Log and uses the existing page session/save path. The Today delta does not change the master purge/restore implementation or sync protocol. This establishes source compatibility, not a new executed purge/restore acceptance result.

## Golden regeneration inventory

Run rendering and golden verification on linux01. Sources are under `app/test/`; all outputs below are under `app/screenshots/`. Let `tag` be each of `desktop-light`, `desktop-dark`, `mobile-light`, `mobile-dark`.

| Source | Output paths | Count and reason |
| --- | --- | --- |
| `today_golden_test.dart` | `today-{tag}-home.png`, `today-{tag}-other-day.png`, `today-{tag}-empty.png`, `today-{tag}-day-with-note.png` | 16; 12 existing selected-day references plus 4 new inline-note references. |
| `today_status_golden_test.dart` | `today-status-{tag}.png` | 4; status cards, overdue disclosure and expansion. The fixture deliberately returns to the selected-day heading after expanding all cards. |
| `home_first_run_golden_test.dart` | `first-run-{tag}-home.png`, `first-run-{tag}-getting-started.png`, `first-run-mobile-light-pages.png`, `first-run-mobile-dark-pages.png` | 10; 4 Today homes, 4 Getting started pages, 2 mobile Pages views. Only home directly changes with inline Today, but run the full requested first-run source. |
| `shell_golden_test.dart` | `desktop-light-sidebar.png`, `desktop-dark-sidebar.png`, `mobile-light-favorites.png`, `mobile-dark-favorites.png`, plus `{tag}-today-pages.png`, `{tag}-trash.png`, `{tag}-history.png`, `{tag}-import.png`, `{tag}-move-to.png`, `{tag}-search-paths.png` | 28; 4 sidebar/Favorites scenes plus 6 scenes per variant. Today page disclosure and desktop overlays can expose the changed Home behind them; run the full requested shell source. |
| `golden_test.dart`, tests matching `^home ` | `{tag}-home.png` | 4 additional general Home references affected by Today. Its page/menu/editor references are outside this regeneration scope. |

The four requested full files cover 58 references. Including the general Home tests brings the Today-related regeneration set to 62. Regenerate the first four files together with `flutter test --concurrency=1 --update-goldens test/today_golden_test.dart test/today_status_golden_test.dart test/home_first_run_golden_test.dart test/shell_golden_test.dart`, then run `flutter test --concurrency=1 --update-goldens test/golden_test.dart --name '^home '`. Verify the same commands without `--update-goldens` and visually review changed/new PNGs before accepting them.

`today_features_golden_test.dart` has 12 existing page-editor/database-settings references (`date-chips`, `colour-menu`, `show-in-today`, each tag); it does not render Today and needs no automatic baseline change for this integration. No unrelated missing golden baseline was identified in these inspected sources. Other overlay goldens may show Home in their background and should be checked by the broader golden suite rather than updated without a reviewed mismatch. All 62 requested baselines were generated on linux01, reviewed during the original integration, and verified unchanged after the master merge. None of this requested baseline set remains pending. No rendering or tests were run on the PC.
