---
type: topic
layer: state
summary: "Every page has a type, blocks and properties; database pages group rows; views are queries plus renderers; markdown is the file format."
parent: "[[architecture/index]]"
status: current
---

# Data model

**Status:** page types, unknown-kind preservation and generic embeds built 2026-10-12.

## Pages, blocks and properties

- **A page** has an id, a title, a string **type**, a tree of blocks and a set of properties. Pages nest logically: every page has at most one parent page. `document` is the default; `database` replaces the former collection boolean. Unknown type strings survive storage, index and API reads. Rust exposes `page_type`, Dart `pageType` with an `isDatabase` helper.
- The compiled-in add-on type registry and generic embeds are built. Agent chat bodies stay in their own store. The registry selects how each installed type is drawn, embedded and edited. Embed storage has no depth limit; drawing caps recursion at four and uses link chips for cycles. See [Note types](note-types.md).
- **A block** is a typed node: paragraph, heading, list item, to-do, toggle, quote, code, markdown table, image, file, embed, link to a page, or an inline view of a collection. Blocks can contain blocks. A table keeps its GFM source in `text`; a code block keeps its fence info string in the existing `target` attribute.
- **Properties** are typed values: text, number, date or date range, checkbox, select, multi-select, status, relation to other pages, URL, file. Tasks, events, ideas and plans are all ordinary pages with the right properties.

### Type data

Each page's Loro document has a root map named `type_data`, keyed by add-on id. Each add-on owns a nested map: JSON objects are Loro maps, arrays are Loro lists, and leaves are JSON scalars. The reserved root key `$markdown` holds an optional ready-made readable Markdown string for any page type; add-on ids cannot start with `$`. Edits merge through the page's CRDT and use the same durable write, sync and change-notification path as property edits. A list move is a delete followed by an insert; it creates a new copy, so concurrent edits to the removed map do not transfer to that copy.

**Decision B (2026-10-12):** use a generic `$markdown` readable body, so each type can supply a readable projection without exporter code depending on that type's data shape. Dart exposes `setTypeMarkdownBody(String page, String? body)` (null removes it), `typeMarkdownBody(String page)`, and the string in `typeDataAll(page)`.

Paths start at the add-on's map. A map segment names a key; a list segment selects a map element by its string `id`. Set creates missing intermediate maps. Other missing or invalid paths return errors. List operations select items by index or by id; an id matches a string element or a map's `id` field. Reads return JSON and an absent add-on reads as `{}`.

Page creation accepts an optional type and installs it before publishing the page. Conversion sets the existing page's type without removing its blocks, properties or type data.

During a Markdown folder import, every JSON string value anywhere in type data that exactly matches a unique source page's front-matter `id` is rewritten to that page's new imported id, using the same old-to-new map as embeds. Object keys and the reserved root `$markdown` body are never rewritten; unknown or ambiguous source ids stay unchanged. Add-ons storing page references as plain id strings get this remapping automatically. The map covers only files in that import, so importing a referenced page separately later does not repair earlier references; rescans do not apply this rewrite.

## Collections and views

- **A database** (called a collection by existing schema/view APIs) is a page with type `database` whose children share a property schema and, optionally, a default template. This covers both ordinary databases and the wanted "template per parent" (see Features (internal wiki)). Old collection documents and `collection: true` front matter read as databases.
- **A view** is a saved query (filter, sort, group) plus a renderer: list, table, board, calendar, timeline, gallery. A calendar is a view over pages with a date property, so tasks, events and Google Calendar entries all appear there.
- Views can also query across collections, for example "everything with a date this week".

### View blocks

A `view` block displays a page's children using the settings of a `ViewDef` in its JSON text. The block id is the view id; a stored `id` is ignored. `source` is `"contents"` (also the default when missing), and the optional block target is the source page id; no target means the page containing the block. Markdown uses a `view` fence containing that JSON object, with an optional `page` field for the target.

`Vault::load_page`, shared by the app and MCP `read_page`, lazily migrates database pages once. The synced page metadata marker `views_migrated: 1` gates conversion. Saved views become blocks appended in `view_order`, with deterministic ids derived from the page and legacy view ids. A database with saved views and no schema properties besides the implicit title becomes a document with one default List view instead; its child pages are untouched. A database without saved views receives only the marker. The legacy `views` and `view_order` remain for existing APIs; the new blocks own their settings independently.

Loro can retain two physical nodes with the same logical block id after concurrent migration. Each page open checks for duplicate `view` ids after migration, including pages already marked or converted. It keeps the first node in document order. A later node with identical text, target and children is deleted; a differing node is copied in place under a fresh id before the old node is deleted. Copies preserve settings, target, colour and descendants. Replacement ids are derived from the old physical node so simultaneous repairs can themselves be deduplicated. Opening an unchanged page adds no edits.

View settings JSON contributes no searchable text (index version 14); ordinary descendant content remains searchable. Markdown folder import assigns fresh block ids as usual and remaps explicit source targets when their pages are imported in the same batch. Missing or ambiguous source targets are preserved.

## Links

- Links point at page ids, not paths, so renaming or moving a page never breaks them. Backlinks are computed by the index.

## On disk

- **Each vault page is a markdown file** in the flat `pages/` directory, with YAML front matter for its id, parent and properties. The parent id expresses nesting; nesting does not create folders. Mounted folders retain their external layout. Wiki links use Obsidian's `[[...]]` form, so a vault stays readable in Obsidian or any editor.
- `type:` is reserved page metadata and is written for every type except the default `document`. The legacy `collection:` key is still read. A property named type is written as `prop:type`.
- Non-empty type data is written as `type-data:` followed by a single-line JSON object `{addonId: data}` in front matter. A stored `$markdown` string becomes `"$markdown": true` in that object, and is written verbatim as the body when the page has no blocks, or only empty paragraphs with no text or children. Other blocks take precedence when present. Import with the true marker stores the entire body verbatim as `$markdown` and never parses it into blocks; without the marker, normal block parsing applies. The nested add-on data still round-trips, including unknown ids. The reserved name is escaped as `prop:type-data` for a user property. The core vault writer and Export panel share the core renderer, including identical front matter for the same page.
- A numbered list that starts at a number other than 1 keeps it: the first item stores an optional `numbered_start`, and both the app and the Markdown renderer count on from it. Typing `N. ` and pasting numbered Markdown store that number the same way, unless the item continues a numbered run directly above it. Inline code is written with a backtick delimiter one longer than the longest backtick run in the text, padded with a space when the text starts or ends with a backtick; the parser strips one space from each side, as in CommonMark.
- Blocks without native Markdown syntax (`embed`, unknown kinds and `chat_ref`) use readable blockquote text plus `<!-- block=<hex-json> -->` metadata for their raw kind, target, fallback line count and checked state. An embed stores only its target id, with empty block text; its current target title is the readable fallback and search text, or empty while unavailable. New inline databases are embeds; old `database` blocks remain readable without migration. Existing toggle, callout, colour and column annotations remain supported. See the [generic fallback format](note-types.md#generic-markdown-fallback-built-2026-10-12). Schema/view definitions and CRDT block ids live in the page's editing state.
- **The live editing state is a CRDT per page** (see [Sync and storage](sync-and-storage.md)). The markdown file is written from it, and outside edits to the file (in VS Code, or in a [mounted folder](mounts.md)) are diffed back into it.
- **A local index database** (SQLite) holds titles, page types, properties, links and full-text search; it has no embeddings. It can be rebuilt from the vault's CRDT documents. Index schema version 9 replaces `is_collection` with a string `page_type` column.

## Built (2026-10-01, Track B)

- **Type foundation (2026-10-12):** unknown block names, text and targets survive reopen, sync, Markdown and the Rust/Dart bridge; an unavailable kind uses the existing registry placeholder. `chat_ref` targets now round-trip through Markdown. Every page has a preserved string type; absent types default to `document`, and legacy collections read as `database`.

- Property kinds also include **files** (attachment ids), **formula** (an expression over the row) and **rollup** (an aggregate over a relation's pages). Formula and rollup values are computed into the index, never stored in pages, so they are not synced; each device computes them. Person is not built.
- **Templates per parent:** any page's children can be marked `istemplate` (page meta), and the parent's `template` meta names the default. Templates are left out of lists, search and queries. A collection's default template uses the same mechanism. Index schema version 7 (`is_template` column).
- **Mounted folders** keep their pages as plain markdown without the vault's id frontmatter; see [Folder mounts](mounts.md#built-2026-10-01). Details: [Core (state)](core-state.md).
