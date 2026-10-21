---
type: topic
layer: state
summary: "Everything is a note; notes have types (document, database, kanban, chat, task), and a type registry selects how each type is drawn, embedded and edited."
parent: "[[architecture/index]]"
status: current
---

# Note types

**Status:** preparatory refactors 1–7 built 2026-10-12.

## Kanban current state (2026-10-13)

The optional [Kanban add-on](../addons/kanban.md) registers type `kanban` with a full-page board and a compact kit embed. It depends only on the kit, independently of the database Board add-on. Columns and manually collected card ids live under `type_data.kanban`; new boards start with To do, Doing and Done. `New kanban` creates and opens a top-level note; `/Kanban` creates a child of the current document and inserts a kit embed.

Cards draw through their note type's card builder, with title and first-line fallback for an unavailable type. Missing or trashed cards are hidden without removing their ids. Card and column edits use the type-data API and refresh through the host change feed. Removing a card never deletes its note; deleting a column appends its cards to the first remaining column. The final column cannot be deleted.

Kanban stores the generic readable Markdown body after board edits: ordered column headings and page-link bullets for existing cards. Card titles changed elsewhere refresh in that body on the next board edit. Export acceptance is currently blocked by the shared page host inserting an empty editor paragraph when it opens a block-less kanban; that block takes precedence over the readable body. A card's ellipsis menu appears on desktop hover and stays visible on phones; it opens or removes the card, and on phones also moves it to the end of another column. Desktop secondary-click remains available, and mobile long-press remains drag. The add-on page records the remaining limits.

**Decision B (2026-10-12; built 2026-10-13):** any type can store a readable body in the reserved root `type_data.$markdown` string. This keeps exports readable without coupling exporters to kanban or any other add-on's data shape. Both exporters retain add-on areas in `type-data` front matter and replace the string there with `"$markdown": true`; block-less pages write the string verbatim, while pages with blocks render their blocks. Import of a marked file stores its body verbatim as the string and creates no blocks. Types own when their readable projection is refreshed; the vault does not derive it automatically. See [Type data](data-model.md#type-data).

## Chats as a type (registration built 2026-10-12; broader integration proposed)

- An agent chat is a note with its own card. Title and chapter and section summary search (agreed 2026-10-10) is built through the external search source in refactor 6; the Agent add-on now registers its chip, card, range embed and full-page builders.
- Current indexing includes the title and outline display names/summaries.
- The transcript stays in the agent store, outside the vault. This refines the [2026-10-08 decision](../integrations/agent.md), not reverses it.

## Preparatory refactors

They make room for types without choosing the mod runtime. Refactors 1 and 2 were built on 2026-10-12.

1. **Unknown kinds survive — built.** `BlockKind::Unknown(String)` keeps the raw name, target and text through CRDT reads, reopen, sync and bridge operations. Its plain text feeds search. Dart receives the raw name and the existing block registry draws its read-only placeholder. Built-in variants, wire names and the every-kind test list come from one declaration, so additions cannot escape that test. Unknown kinds and `chat_ref` use the generic Markdown annotation below.
2. **Page type field — built.** Page metadata stores `type`; absent means `document`, `database` is the former collection, and other strings are preserved. Rust's page and summary field is `page_type`, exposed as `pageType` in Dart; `isDatabase` replaces boolean checks. SQLite indexes the string. Old CRDT `collection: true` and Markdown `collection: true` read as `database`. Markdown writes reserved `type:` and omits it for `document`; `prop:type` remains a user property.
3. **Type registry in Flutter — absorbed into [Add-ons](addons.md) (2026-10-12).** A `NoteType` has chip, card, embed and full-page builders plus commands, with `document` and `database` registered as add-ons. Built on `addons-core`: the page screen picks its body through the registry; unknown types keep the existing markdown fallback.
4. **Generic embed block — built 2026-10-12** (a kit feature under [Add-ons](addons.md)). `embed` stores only the target note id; its text and marks stay empty. The kit resolves the current title/type, then calls the target type's embed builder, or its card if no embed is declared. Documents draw read-only block previews, databases use the original inline view, and chats use a whole-session card with their title and chapters. The inherited ancestor chain starts with the containing page; cycles draw a link chip. `maxEmbedDepth = 4` limits inline drawing only, never storage. Unknown or disabled types keep their title and block/Markdown fallback in a placeholder. `/embed` reuses the existing page picker. The Database menu entry now inserts an embed targeting a new database note; stored `database` blocks still use their original renderer and are never rewritten just by opening a page. External notes resolve by their exact indexed identity (for a whole chat, `chat:<session-id>`); their bodies remain in the owning source. Bare session IDs also work directly with the chat type's builders. Existing range-JSON `chat_ref` presentations are unchanged.
5. **Board layout extracted — built 2026-10-12.** The storage-independent `BoardRenderer` and `BoardSource` interface live in `app/lib/ui/board/`; `app/lib/addons/board/database_board_source.dart` adapts the database's live query, with its existing card presentation retained by `BoardView`. Dragging, mobile long press, keyboard card activation and lazy/inline layout are shared; optional column editing is available to other sources. `app/test/board_renderer_test.dart` proves the renderer with an in-memory source and no database. This prepares the shared renderer; it does not add a kanban note type. The current master includes the shared header golden refresh.
6. **External index sources — built 2026-10-12.** Search hits carry `source`, `page_type` and an optional opaque target. Vault rows are `vault`; clear and reindex preserve external rows. Sources upsert or delete by `(source, id)`, or clear their own rows. Agent chats use `agent`: one title row per chat, plus chapter and section display names and summaries, with session/message-range anchors. Raw transcripts stay outside the index. Ctrl+K and mobile search group Pages and Chats, keep ranking within each group and open chat hits through the existing anchor jump. `type:document`, `type:database` and `type:chat` filter search and drive the type chips. The Agent add-on owns this source and the chat presentations in refactor 7. The follow-up restores body/summary previews for title hits and qualifies the closed-chapter anchor fixture in three consecutive full suites. The two requested mobile path goldens and all four grouped-search references are refreshed. Search labels now follow the active agent backend, and grouped fixtures wait for complete expected rows and check their order. Broader timing limits are recorded in [search queries](search-queries.md). See [agent search sources](../integrations/agent.md#chat-search-source).
7. **Chat as a type — built 2026-10-12.** The optional [Agent add-on](../addons/agent.md) registers `chat` with chip, card, embed and full-page builders and `bodySource: agent`. Bodies stay in the agent store. `chat_ref` draws through the registered chat type's embed of its existing chapter/section message range, with the same card and cache updates. Disabling Agent preserves stored references through the unknown-block placeholder; the containing page still opens. No transcript is mirrored into vault pages. Generic recursive embeds are supplied by the kit in refactor 4.

### Generic Markdown fallback (built 2026-10-12)

A kind without native Markdown syntax (`embed`, `chat_ref` or an unknown kind) writes its fallback text as a blockquote. The first line ends with `<!-- block=<hex-json> -->`, alongside the existing optional `color=` and `width=` words. Continuation lines also start with `> `; child blocks retain the normal two-space indentation.

The annotation is lowercase hexadecimal UTF-8 JSON with `kind` (raw string), `target` (string or null), `lines` (positive fallback line count) and `checked` (boolean). Encoding prevents arbitrary targets, including chat JSON, whitespace, Unicode and `-->`, from breaking the comment. The line count preserves blank lines and block boundaries. Text and inline marks are read from the visible fallback, so editing that text edits the block, except for `embed`: its visible title is fallback text only, its id is authoritative, and import/paste never stores the title. Embed export and search resolve the target's current title, or empty text for an unavailable target. Malformed annotations remain ordinary Markdown. Example decoded payload: `{"kind":"mod:diagram","target":"page-id","lines":1,"checked":false}`.

On an existing page, rescanning a file with neither `type:` nor the legacy `collection: true` leaves its current type intact; an explicit `type: document` resets it. New untyped imports are documents. Unknown type strings are preserved exactly, including whitespace and empty strings. Database conversion/schema operations refuse unknown types to avoid overwriting a mod's identity.

For a single folder import, embeds targeting notes included in the import are remapped by source frontmatter id to the new imported page ids. Missing, external and ambiguous source ids stay intact. Importing the host and target in separate operations preserves the original target id; it does not reconnect the embed to a copy given a new id. The importer never chooses an embed target by fallback title. Vault renames and external-index upserts/deletes/source clears refresh the embedding page's derived search text and Markdown; the embed's stored target and empty text are unchanged.

This preservation format introduces no mod runtime. The kit renderer and type dispatch above are separate from the core's annotation codec.

## Decisions

- 2026-10-12: A kanban is a note type whose cards are notes of any type; its columns come from its own column list.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Architecture > Note types).
