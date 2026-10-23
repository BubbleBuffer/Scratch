---
type: topic
layer: implementation
summary: "A kanban note with manually collected note cards and a compact embed."
parent: "[[addons/index]]"
status: current
---

# Kanban

Kanban is a note type for collecting cards by hand. Each card points to a note of any type; opening the card opens that note. A database's Board view remains a live query. Both use the kit's `BoardRenderer`, while Kanban depends on no other add-on.

`KanbanAddon` is compiled in and always enabled. Registration does not open a vault or start background work.

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

## Limits

The shared page host currently inserts an empty paragraph when it opens a block-less kanban, despite `usesEditor: false`. Blocks take precedence over `$markdown` during export, so an opened board can export the empty paragraph instead of its readable projection. The same-page Markdown round-trip acceptance is blocked until the host stops inserting editor blocks for these types. The add-on still stores its projection; no host workaround is added here.

Columns are the kanban's own list; property-driven columns and agent automation are future work. Type data is preserved by the core's existing `type-data:` front matter. Disabling the optional add-on preserves its pages and data; the kit handles an unavailable type.

The card menu button is hidden on desktop until the pointer is over the card, and it is out of the keyboard focus order while hidden. A keyboard user cannot open the card menu on desktop yet. Mobile long-press remains the kit's drag gesture; the phone menu is reached by the always-visible button.

The readable body is not rewritten for changes from another device. A card renamed elsewhere keeps its old title in the copy until the next local board change. If the copy's write fails, the board change still stands, the copy keeps its previous text, and the error reaches the caller.

Cross-column moves use separate delete and insert calls. An interrupted move can leave a note off the board; it never deletes the note. Column deletion appends references before removing the column.

Source: [`api.dart`](../../app/lib/addons/kanban/api.dart). Tests: [`app/test/addons/kanban/`](../../app/test/addons/kanban/).
