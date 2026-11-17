---
type: topic
layer: state
summary: "A vault is pages with a string type, blocks and properties; pages of type database hold rows with a schema and views; markdown files are the on-disk format."
parent: "[[core/index]]"
status: current
covers: ["crates/core/src/types.rs", "crates/core/src/type_data.rs", "crates/core/src/view_blocks.rs", "crates/core/src/markdown.rs"]
---

# Data model

A vault is a tree of pages. Each page has an id, a title, a string type, a tree of blocks and a set of properties; it is stored as a Loro document and rendered to a markdown file ([Vault core](vault-core.md)). Today a page of type `database` groups rows under a shared schema. The `database` type is being replaced by typed pages: see "Target" in [Architecture](../architecture.md).

## Pages

- Every page has at most one parent page. Nesting is logical and creates no folders.
- The type is a string. `document` is the default; `database` marks a page whose children are rows (the old `collection` flag, still read from old documents and `collection: true` frontmatter). Unknown type strings survive storage, the index and API reads. Rust exposes `page_type` and `Vault::set_page_type`; `create_page_with_type` installs the type before the page is published; conversion keeps blocks, properties and type data. Dart has `pageType` and an `isDatabase` helper.
- The compiled-in add-on type registry chooses how each installed type is drawn, embedded and edited ([Note types](../app/note-types.md)). Agent chat bodies stay in their own store.

## Blocks

A block is a typed node that can contain blocks. The kinds are the `BlockKind` variants in `types.rs`: paragraph, three heading levels, bulleted, numbered, to-do, quote, code, table, divider, link to page, image, file, bookmark, toggle and toggle headings, callout, column list and column, table of contents, `chat_ref`, `embed`, `query` (a search card), `view` and `database` (the older inline database). A kind from a newer version is kept as `Unknown(name)`.

- A table keeps its GFM source in `text`; a code block keeps its fence info string in `target`.
- A numbered list that starts at a number other than 1 keeps it in the first item's `numbered_start`.
- An embed stores only its target page id; the target's title is looked up when it is drawn, exported or searched. New inline databases are embeds.
- Embed storage has no depth limit; drawing caps recursion at four (`maxEmbedDepth`) and shows link chips for cycles.

## Properties

Property kinds are in `PropKind`: text, number, date (or range), checkbox, select, multi-select, status, relation, URL, email, phone, created time, updated time, files, formula and rollup. Formula and rollup values are computed into the index on each device and are never stored in a page. Pages outside a database take any key not starting with `$`; rows of a database are checked against its schema. Tasks, events and ideas are ordinary pages with the right properties.

## Databases and views

- A database page's children are its rows. Its document holds the schema (`schema`, `schema_order`) and, optionally, a default template.
- A saved view (`ViewDef`) is a filter, sort and group plus a layout: table, list, board, calendar, gallery or timeline.
- A `view` block displays a page's children with the settings of a `ViewDef` in its JSON text. The block id is the view id. `source` is `"contents"`; the block target is the source page id, and no target means the page containing the block. In markdown it is a four-backtick `view` fence holding that JSON, with an optional `page` field for the target.
- `Vault::load_page` migrates a database once, gated by the synced page marker `views_migrated`: saved views become `view` blocks in `view_order`, with ids derived from the page and the legacy view id. A database with saved views and no schema properties besides the title becomes a document with one default List view. The legacy `views` and `view_order` remain for existing APIs.
- Concurrent migrations can leave two nodes with one `view` block id. Each page open keeps the first in document order, deletes a later identical node, and copies a differing one under a fresh id.
- View settings contribute no searchable text; descendant content stays searchable.

## Type data

Each page's Loro document has a root map `type_data` keyed by add-on id. Each add-on owns a nested map: JSON objects are Loro maps, arrays are Loro lists and leaves are JSON scalars. Add-on ids cannot start with `$`. Edits merge through the page's CRDT and use the same write, sync and notification path as property edits. A list move is a delete followed by an insert, so concurrent edits to the removed map do not reach the copy.

- Paths start at the add-on's map. A map segment names a key; a list segment selects a map element by its string `id`. Set creates missing intermediate maps; other missing or invalid paths return errors. Reads return JSON, and an absent add-on reads as `{}`. The API is `type_data`, `type_data_all`, `type_data_set`, `type_data_delete` and the `type_data_list_*` methods in `type_data.rs`.
- The reserved root key `$markdown` holds an optional readable markdown body for any type, so exporters need not know an add-on's data shape. Dart exposes `setTypeMarkdownBody`, `typeMarkdownBody` and the string in `typeDataAll`.
- On markdown folder import, every string in type data that exactly matches a unique source page's frontmatter `id` is rewritten to that page's new id, using the same map as embeds. Keys and the `$markdown` body are not rewritten; unknown or ambiguous ids stay. Only pages in that import are covered, and rescans do not rewrite.

## Links

Links point at page ids, not paths, so renaming or moving a page never breaks them. The index computes backlinks.

## On disk

- Each vault page is a markdown file in the flat `pages/` directory, with YAML frontmatter for its id, parent and properties. Nesting is expressed by the parent id. Mounted folders keep their external layout and no id frontmatter ([Folder mounts](mounts.md)). Wiki links use `[[...]]`, so a vault reads in Obsidian or any editor.
- `type:` is reserved and written for every type except `document`. The legacy `collection:` key is still read. A property named `type` is written as `prop:type`; the same escape applies to other reserved names (`role`, `type-data`).
- Non-empty type data is written as `type-data:` followed by a single-line JSON object `{addonId: data}`. A stored `$markdown` string becomes `"$markdown": true` there and is written verbatim as the body when the page has no blocks (or only empty paragraphs). Importing a file with that marker stores its body verbatim as `$markdown` and parses no blocks.
- Inline code uses a backtick delimiter one longer than the longest backtick run in the text, padded with a space when the text starts or ends with a backtick.
- A block without native markdown syntax (`embed`, unknown kinds, `chat_ref`) is a readable blockquote followed by `<!-- block=<hex-json> -->` holding its raw kind, target and checked state ([generic fallback format](../app/note-types.md#generic-markdown-fallback-built-2026-10-12)). Toggles, callouts, colours and columns use short HTML comments.
- The markdown file is written from the page's CRDT; edits made outside the app are diffed back in by `rescan`.
- The SQLite index holds titles, page types, properties, links and full-text search. It has no embeddings and can be rebuilt from the documents.
