---
type: topic
layer: implementation
summary: "The Search card add-on: the `query` block that shows live results of a query."
parent: "[[addons/index]]"
status: current
---

# Search card

`SearchCardAddon` in `app/lib/addons/search_card/api.dart` registers the `query` block kind and its `/` menu entry ("Search query", group Advanced). It has no dependencies and is always enabled. The card reads the Tasks registration only through `AddonServices().addonEnabled('tasks')`, retaining support for isolated kit tests.

Design and behaviour follow [Dynamic search card UI](../architecture/search-queries.md#dynamic-search-card-ui-approved-2026-10-13) (Form A, block card).

- **Block text.** The block holds the query string only. Markdown writes it as a fenced `query` block; the fence is longer than any backtick run in the query. The Dart codec mirrors the core's fence because `MarkdownCodec.rust` cannot be used from Dart.
- **Live results.** `QueryCard` subscribes through `NotesVault.watchQuery` with a limit of 50 and applies the core's snapshot and diffs in `query_hits.dart`. It releases its handle on dispose and when the query text changes.
- **Rows.** With Tasks on, a task page shows a checkbox. Other pages show their own icon, or the generic one when they have none. A board row whose page has a select or status property named "Status" (any case, spaces, `_` or `-` ignored) shows that value as a pill (`query_status.dart`). Five rows show first, then "Show N more" expands in place.
- **Tick.** Ticking a task hides its row at once and sets its status to `done`. When the live result arrives, a row the query still matches comes back checked with no message. Only a row that leaves the list shows "Marked done, so it left this list." with Undo. Ticking the checkbox again sets the status back to `todo`.
- **Edit.** Clicking the header, or Enter on a focused header, opens the query field. Enter applies it to the block text; Esc cancels.
- **Count and wording.** The header shows the core total minus rows hidden by pending ticks, capped at 200+. The chip count and the empty line come from one pure function, `queryWording` (`query_wording.dart`), read from the parsed tokens. Task lists read "N open #work tasks" and "No open #work tasks." when the query says `done:no` or `-status:done`, and "N #work tasks" otherwise; status queries read "N in Doing" and "Nothing in Doing."; `updated:week` reads "N pages this week" and "Nothing edited this week."; anything else reads "N results · <query>" and "No results for <query>.".

## Known limits

- Open work is `#work type:task -status:done`. The core indexes a task's status as the `status` property, so a ticked task leaves this list when the core's live result arrives. Queries that do not name the status still list a done task, checked.
- The core returns a property operator's key as the property id, so the status wording reads the typed key from the token span (`_typedKey`).

- **Shared watch.** `QueryWatch` (`query_watch.dart`) holds one query's core watch, hits, row facts and pending ticks. The card owns one. The chip owns one per mark: `QueryInlineMark` (`query_inline.dart`) makes it, and the editor releases it when the mark or its block goes away (see [Add-ons](../architecture/addons.md)).
- **Results panel.** `showQueryPanel` opens a 420 px popover on desktop, closed by Escape, and a bottom sheet on phone. The phone sheet shows the Undo message above its footer. Its footer offers "Open in Search" and "Show as a block".
- **Inline chip (Form B).** A paragraph with a `query` mark (`BMarkKind.query`, payload = query string) draws "N … #work tasks" from the chip's watch, or its plain text until the first result is in. A tap on the chip opens the results panel. "Open in Search" opens the search overlay with the query.
- **Show as a block.** If the chip is the whole paragraph, the paragraph becomes the query block (`replaceWithBlock`). Otherwise the query block goes after the paragraph and the chip's text is cut from it (`insertBlockAfter` with `removeText`), with one adjacent space cut too (the space after the chip, else the one before), so "chip and more" leaves "and more". Each is one undo step.
- **Show inline.** The card's overflow menu offers "Show inline" (`showInlineMenuItem`), which turns the block into a paragraph holding its chip over the current label (`replaceWithParagraph`). It needs the first result, so it does nothing before then. One undo step.
- **Keyboard.** A chip has no keyboard action. Enter does not open painted mentions or dates in the editor, so there is no precedent to follow; the chip opens on tap only.

## Not built yet

- Nothing. The keyboard decision above is a choice, not a missing feature; revisit it if a keyboard way to open chips is wanted.

Tests are in `app/test/addons/search_card/`: the unit tests `query_wording_test.dart`, `query_status_test.dart` and `query_ticks_test.dart`, the panel tests in `query_panel_test.dart`, and the chip and conversion tests in `query_inline_test.dart`. Golden shots `search-card-*`, `chip-*` and `unsupported-*` are in `app/test/golden_test.dart`. The committed baselines in `app/screenshots/` are not the current renders: the `chip-*` and `unsupported-*` shots were rendered on the Linux machine and are not committed, and the Windows baselines still need regenerating.
