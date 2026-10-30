---
type: topic
layer: implementation
summary: "An optional kanban note with manually collected note cards and a compact embed."
parent: "[[addons/index]]"
status: current
---

# Kanban

Kanban is a note type for collecting cards by hand. Each card points to a note of any type; opening the card opens that note. A database's Board view remains a live query. Both use the kit's `BoardRenderer`, while Kanban depends on no other add-on.

`KanbanAddon` is compiled in, optional and enabled by default. Settings → About → Add-ons → Manage can disable it for the next start. Registration does not open a vault or start background work.

## Data

The kanban page has type `kanban`. Its type-data area under add-on id `kanban` holds:

```json
{
  "columns": [
    {"id": "short-id", "title": "To do", "color": "blue"}
  ],
  "cards": {"short-id": ["page-id", "another-page-id"]}
}
```

Column ids are short random ids. Column and card order follows the lists. Rename sets the column's title by id; reordering uses type-data list moves. Across-column moves delete the source list entry and insert into the destination list. Type data uses the page's existing CRDT, durable writes and sync change feed.

Cards retain their columns and order when the board and its referenced notes are exported as Markdown and imported together into another vault.

## Entry points and editing

- **New kanban** creates a top-level kanban note and opens it.
- **/Kanban** in a document creates a child kanban and inserts a kit `embed` pointing to it.
- A new board starts with **To do**, **Doing** and **Done**.
- **New card** in a column accepts a title and creates a document note as a child of the kanban. **Add existing note…** uses the existing page picker. The kanban itself is excluded, and a note already on the board is ignored.
- Each card has a menu button at its top right, in its own column so it never covers the note's text. On desktop the button appears while the pointer is over the card; on a phone it is always shown. The menu has **Open** and **Remove from board**; removing only drops the reference, and the note remains. Right-click opens the same menu on desktop.
- On a phone the menu also has **Move to column…**, which lists the other columns and appends the card to the end of the chosen one. The entry is hidden when the board has one column. Desktop moves cards by dragging.
- Deleting a column appends its cards to the first remaining column. The final column cannot be deleted.

## Readable body

Each board keeps a readable Markdown copy of itself in the page's `$markdown` type-data string, written with `Vault.setTypeMarkdownBody`. It is written when a board is created and after every local board change: adding, removing or moving a card, and adding, renaming, reordering or deleting a column. Writes run one at a time, and each copy is built from the board as the previous write left it.

The copy has one `##` heading per column, in stored order. Under each heading is a bullet per card, in stored order. Missing and trashed cards are left out.

A card's bullet uses the core's page-link syntax, for example `- [[page-id|Garden plan]]`. The target is the stable page id and the label is the current title, escaped as core escapes text. Line breaks become spaces; blank titles read `Untitled`. Column headings use the same escaping. Cards with duplicate titles retain distinct link targets.

Changes from sync reload the board but never rewrite the copy, so a synced copy cannot start a write loop. A card renamed in its own note shows the new title in the copy at the next local board change.

Cards use the note type registry's card builder, with title and first-line fallback for unavailable types. Missing note ids stay in storage but are hidden. The full page and compact embed share a source and the kit's drag, keyboard and mobile behavior. Generic embeds supply the cycle and depth guards.

The source listens to the host change feed for type-data and card changes. It uses the same feed for local writes and incoming changes, and refreshes card metadata and first-line previews. Column colors, when present, use the kit's existing color tokens.

## Runner picker

A card's ellipsis menu offers **Assign to Agent** when the board has a `RunnerScope`. With no scope, the board uses the agent's runners (`aiRunnerAssignments`), or with `--dart-define=SEED_DEMO=true` the fake runners (`demoRunnerAssignments`). The entry opens **Run on**: a popover at 700 px and wider, always opened below the card menu button (it moves up only as far as the window needs, and never flips above the card), and a bottom sheet below that. The 700 px switch is `runnerSheetBelow` in `runner_picker.dart`; it is separate from the app's 768 px panel breakpoint. The picker lists the backend's runners, starts on the default, and assigns on **Assign**. Under a PC that has projects, a **Project** row lists **None** (the default) and the PC's projects by name; switching runners resets it to None, the chosen project id goes to `ai_assign_task`, and a task in a project shows "<runner> · <project> · <state>" on its chip. A card with an assignment shows a chip, "<runner> · <state>", under its body. A `waiting` assignment shows its reason under the chip.

The UI talks only to `RunnerBackend` (`runner_backend.dart`): `listRunners()`, `assign(taskId, runnerId)`, and two hooks for state it already holds, `current(taskId)` and `changes`.

**The real backend** (`RealRunnerBackend`, `runner_real.dart`) is what the app uses outside `SEED_DEMO`. It reads the agent through `AiTaskSource` (the services barrel; `AiStore` implements it):
- **Runners:** `ai_list_runners`, hub first, then the PCs. The hub counts as usable on phones and when a hub link is signed in (`AiStore.hubUsable`). The default runner is marked, and a PC's last-seen time comes from the synced timestamp.
- **Assign:** `ai_assign_task` with the fixed prompt `runnerCardPrompt`, the card's page id and the chosen project id (none when None is chosen). The row is pinned to the runner, even an offline one. The store then refreshes.
- **Chip state:** read from the newest task row whose page is the card. `queued` on an online runner shows queued. `queued` on an offline runner shows `waiting`, with "<name> is offline (last seen …)". `running`, `done` and `cancelled` map directly. `failed` shows the row's error as the reason. `interrupted` shows as failed with "The run was interrupted before it finished." A row with no runner is not an assignment and shows no chip.
- **Progress line:** the row's `progress` (trimmed; blank means none) shows under the chip as one line with an ellipsis, and the full text is in a tooltip. It shows while the run is `running`, and after a `failed` or `cancelled` run it stays in the tertiary text colour. A `done` run shows none. Queued and waiting rows show none.
- **Notices:** after an assign, the backend calls `AiTaskSource.onDelegated`; on Android that asks for the notification permission once. Phone notices for ended runs are described in [app state](../app/app-state.md#run-notices-android).
- **Updates:** the backend listens to the AI store, so a row that syncs redraws its chip. The runner list refreshes on each store change, and nothing polls.

The bridge has no `cancelling` status, so a stopping run shows as running until it ends. Rows come from the store's latest 50 tasks, so a card whose row is older than that shows no chip. The bridge's `AiTask` carries `progress` and `progress_at` (empty until reported); `progress_at` is not shown.

**The fake** (`runner_fake.dart`) backs `--dart-define=SEED_DEMO=true` and the tests. It has four demo runners: Hub, Studio-PC (this device and the default, with the projects notes-app and docs-site), Office-PC (online, no projects) and Laptop (offline, last seen 2 h ago). Assigning to an online runner returns `queued`, and nothing moves it on; a test records the next state with `RunnerAssignments.record`. The SEED_DEMO build (`demoRunnerAssignments`) makes its first online assignment run with sample progress text, since demo cards have no fixed ids. Assignments live in memory only.

The real backend's assignments are kept by the AI store, and the card's chip reads them through `current`, so a `record` made by the picker only shows where the store has no row yet.

**Rename:** Settings > AI > "Runner names" lists the PCs by their vault names. Each row has an edit button; saving a trimmed, non-empty name calls `ai_rename_device`. The hub has no name to change. The picker and chips show the new name after the next runner listing.

Source: [`runner_picker.dart`](../../app/lib/addons/kanban/runner_picker.dart), [`runner_chip.dart`](../../app/lib/addons/kanban/runner_chip.dart), [`runner_backend.dart`](../../app/lib/addons/kanban/runner_backend.dart), [`runner_real.dart`](../../app/lib/addons/kanban/runner_real.dart), [`runner_fake.dart`](../../app/lib/addons/kanban/runner_fake.dart), [`ai_runners.dart`](../../app/lib/features/ai/ai_runners.dart), [`ai_settings_body.dart`](../../app/lib/features/ai/ai_settings_body.dart). Tests: [`kanban_runner_picker_test.dart`](../../app/test/addons/kanban/kanban_runner_picker_test.dart), [`kanban_runner_backend_test.dart`](../../app/test/addons/kanban/kanban_runner_backend_test.dart), [`kanban_runner_real_test.dart`](../../app/test/addons/kanban/kanban_runner_real_test.dart), [`ai_runner_names_test.dart`](../../app/test/ai_runner_names_test.dart).

## Limits

The shared page host currently inserts an empty paragraph when it opens a block-less kanban, despite `usesEditor: false`. Blocks take precedence over `$markdown` during export, so an opened board can export the empty paragraph instead of its readable projection. The same-page Markdown round-trip acceptance is blocked until the host stops inserting editor blocks for these types. The add-on still stores its projection; no host workaround is added here.

Columns are the kanban's own list; property-driven columns and agent automation are future work. Type data is preserved by the core's existing `type-data:` front matter. Disabling the optional add-on preserves its pages and data; the kit handles an unavailable type.

The card menu on desktop is reachable by keyboard: each card is one focus stop (the board's, which Enter opens), the menu button shows while the card has focus, and the context-menu key or Shift+F10 on the card opens the menu under it (the button itself is for the pointer and touch). Focus starts on the first item, Up and Down move between items, Enter runs one, and Esc closes the menu and returns focus to the card (`keyboard_menu.dart`). The Run on picker is not keyboard-driven yet. Mobile long-press remains the kit's drag gesture; the phone menu is reached by the always-visible button.

Pending: an Assign to Agent button in the opened card. The design is being settled; today Assign is only in the card menu.

The readable body is not rewritten for changes from another device. A card renamed elsewhere keeps its old title in the copy until the next local board change. If the copy's write fails, the board change still stands, the copy keeps its previous text, and the error reaches the caller.

Cross-column moves use separate delete and insert calls. An interrupted move can leave a note off the board; it never deletes the note. Column deletion appends references before removing the column.

Source: [`api.dart`](../../app/lib/addons/kanban/api.dart). Tests: [`app/test/addons/kanban/`](../../app/test/addons/kanban).
