---
type: topic
layer: state
summary: "The built Rust core (vault_core): Loro page documents, properties, collections and queries, trash, links, attachments, history, import, index, bench."
parent: "[[core/index]]"
status: current
covers: ["crates/core/**"]
---


Current build source (2026-10-29): Scratch resolves BubblePaw through the public
`BubbleBuffer/BubblePaw-Core` Cargo git source at tag `v0.3.1`, pinned in
`Cargo.lock`. Current builds need no `BubblePaw-base-agent` sibling. Commands
below containing `--with ...BubblePaw-base-agent` record historical runs only;
retain the matching dependency checkout when reproducing those old revisions.
# Core (state)

**As of 2026-09-30.** `crates/core` (package and library `vault_core`) implements the first core API (internal wiki) and Part A (data features) of the MVP core (internal wiki). Part B (sync) is not built. The core API page (internal wiki) was updated to match the deviations listed at the end. Track B (data features, 2026-10-01) added formula, rollup and files properties, templates for the children of any page, and folder mounts.

As of 2026-10-16, `feedback_page()` lazily creates the ordinary role page "Feedback" at fixed id `000000000000000000000000FB`, using the existing synced `role` metadata. The Feedback page and all descendants are hidden from sidebar roots, global page lists/counts and search (including MCP), while reads by id and explicit parent listings still reach reports. Markdown exports preserve page roles in reserved frontmatter key `role`; imports recognize `journal`, `calendars`, `nodes` and `feedback`, retaining unknown role values as ordinary properties.

## Modules

Every public function is a method of `Vault`, split over several `impl Vault` blocks:

- `vault.rs`: `Vault`, open, pages, blocks, flush, reindex, the page cache, update logs and the background writer thread.
- `tree.rs`: sibling order, moves, duplicates, trash, favorites, icon and cover.
- `collections.rs` + `query.rs`: properties, schemas, views, default templates; queries, board groups and date ranges compiled to SQL.
- `agenda.rs`: agenda roles and rows, task completion, checklist and collection progress, Journal daily notes and space colours (2026-10-06). `AgendaItem` also projects optional status label, group (`todo`/`progress`/`complete`) and colour (2026-10-07), using indexed row properties and a per-query collection schema cache. Checkbox tasks and dated block to-dos have null status fields. No index columns or schema-version change are needed. Foreground metadata/property updates and direct agenda refreshes reuse or acquire a write transaction, preventing the background writer from interleaving derived-row replacement; a concurrent-write regression checks document/projection and reindex equality.
- `formula.rs` + `computed.rs`: the formula language, and formula and rollup values computed into the index.
- `templates.rs`: templates for new children of any page.
- `mounts.rs`: folders of this device mounted as pages (see [Folder mounts](mounts.md)).
- `media.rs`: backlinks, attachments, page history and the change feed (backed by `attach.rs`, `history.rs`, `feed.rs`).
- `import.rs` + `mdparse.rs` + `blockdiff.rs`: markdown import, rescan of edited `.md` files, and turning one block tree into another with `BlockOp`s.
- `doc.rs`: one page's Loro document (`PageDoc`).
- `index.rs`: the SQLite index, including the generic local `meta(key TEXT PRIMARY KEY, value TEXT)` table for one-off migration markers. Metadata is not synced and is cleared when the index is rebuilt; losing a marker safely repeats its migration.
- `markdown.rs`: markdown + YAML frontmatter rendering and file names.
- `order.rs`: sibling order keys. `ids.rs`: ULID-style ids. `types.rs`, `error.rs`: public data types and the one error type.
- `sync.rs`: the sync hooks (versions, export and import of updates, conflicts, purge tombstones); see [Sync (state)](../sync/sync-state.md).
- `conflict_detect.rs`, `conflict_restore.rs`, `conflict_resolution.rs`, `conflict_diff.rs`, `conflict_policy.rs`, `conflict_settings.rs` and `conflict_archive.rs`: the implemented v2 scoped-conflict backend with passing core, sync and Rust bridge checks; native/Flutter acceptance is tracked below.
- `mdio.rs` (2026-10-01, for the MCP server): `is_owner()` (whether this instance holds `.vault/lock`), `page_markdown(id)`, `blocks_markdown(blocks)`, `parse_markdown(markdown)` (new block ids; `[[Title]]` links resolve by id or title), `insert_markdown(id, parent, after, at_end, markdown)` and `replace_markdown(id, blocks, markdown)` (an empty `blocks` list replaces the whole page and keeps the ids of unchanged blocks). Tests in `tests/mdio.rs`.
- `seed.rs`: `seed::generate` and `seed::generate_tasks`, deterministic test vaults.
- `examples/bench.rs`: timings for the targets, including a week's agenda, capped overdue and overdue-count queries on the Tasks seed. `tests/api.rs` (first API), `tests/mvp.rs` (MVP features), `tests/sync.rs` (sync hooks), `tests/computed.rs` (formulas, rollups, files), `tests/templates.rs`, `tests/mounts.rs`, `tests/prefs.rs` (page display options) and `tests/agenda.rs` (22 organizer cases, including status projection, concurrent property/writer refreshes, daily notes, import colours, reindex and sync): integration tests.

## Public API

All functions are synchronous and take owned arguments. `Vault` is `Send + Sync` (checked at compile time) and is shared behind an `Arc`. Counts are `u32`, errors are `VaultError`, and every type is an owned serde struct or enum without lifetimes or generics.

```rust
// First API (vault.rs)
Vault::open(root: String) -> Result<Vault>
vault.root() -> String
vault.device_id() -> String
vault.page_count() -> Result<u32>                      // pages not in the trash, not the hidden calendars page
vault.list_pages(parent: Option<PageId>, offset: u32, limit: u32) -> Result<Vec<PageSummary>>
vault.search(query: String, limit: u32) -> Result<Vec<SearchHit>>
vault.search_adaptive(query: String, limit: u32) -> Result<SearchResults> // hits + narrowed
vault.search_full(query: String, limit: u32) -> Result<Vec<SearchHit>> // agent/API recall
vault.create_page(parent: Option<PageId>, title: String) -> Result<PageId>
vault.rename_page(id: PageId, title: String) -> Result<()>
vault.delete_page(id: PageId) -> Result<()>            // moves to the trash
vault.load_page(id: PageId) -> Result<Page>
vault.apply(id: PageId, ops: Vec<BlockOp>) -> Result<()>
vault.flush() -> Result<()>
vault.reindex() -> Result<u32>

// Page tree (tree.rs)
vault.list_children(parent: Option<PageId>, offset: u32, limit: u32) -> Result<Vec<PageSummary>>
vault.move_page(id: PageId, new_parent: Option<PageId>, after: Option<PageId>) -> Result<()>
vault.duplicate_page(id: PageId) -> Result<PageId>
vault.restore_page(id: PageId) -> Result<()>
vault.list_trash() -> Result<Vec<TrashedPage>>
vault.purge_page(id: PageId) -> Result<()>
vault.empty_trash() -> Result<u32>
vault.set_favorite(id: PageId, favorite: bool) -> Result<()>
vault.list_favorites() -> Result<Vec<PageSummary>>
vault.set_icon(id: PageId, icon: Option<String>) -> Result<()>
vault.set_cover(id: PageId, cover: Option<AttachmentId>) -> Result<()>
vault.set_page_prefs(id: PageId, prefs: PagePrefs) -> Result<()>  // small text, full width, lock, cover position; `Page.prefs` reads them

// Properties, collections, views, queries (collections.rs)
vault.set_property(page: PageId, key: String, value: PropValue) -> Result<()>
vault.remove_property(page: PageId, key: String) -> Result<()>
vault.make_collection(page: PageId) -> Result<()>
vault.get_schema(collection: PageId) -> Result<Vec<PropDef>>
vault.set_schema(collection: PageId, schema: Vec<PropDef>) -> Result<Vec<PropDef>>
vault.list_views(collection: PageId) -> Result<Vec<ViewDef>>
vault.add_view(collection: PageId, view: ViewDef) -> Result<String>
vault.update_view(collection: PageId, view: ViewDef) -> Result<()>
vault.delete_view(collection: PageId, view_id: String) -> Result<()>
vault.move_view(collection: PageId, view_id: String, index: u32) -> Result<()>
vault.set_default_template(collection: PageId, template: Option<PageId>) -> Result<()>
vault.default_template(collection: PageId) -> Result<Option<PageId>>
vault.query(scope: Option<PageId>, filter: Option<Filter>, sorts: Vec<Sort>, offset: u32, limit: u32) -> Result<QueryResult>
vault.query_groups(scope: Option<PageId>, filter: Option<Filter>, group_by: String) -> Result<Vec<Group>>
vault.query_range(scope: Option<PageId>, date_prop: String, from_ms: i64, to_ms: i64, filter: Option<Filter>) -> Result<Vec<RangeEntry>>

// Organizer (agenda.rs, 2026-10-06)
vault.set_page_color(page: PageId, color: Option<String>) -> Result<()>
vault.set_page_role(page: PageId, role: Option<String>) -> Result<()>
vault.ensure_space_colors() -> Result<u32>
vault.find_by_role(role: String) -> Result<Option<PageId>>
vault.set_agenda_settings(collection: PageId, settings: AgendaSettings) -> Result<()>
vault.agenda_settings(collection: PageId) -> Result<AgendaSettings>
vault.agenda(timed_from_ms: i64, timed_to_ms: i64, day_from_ms: i64, day_to_ms: i64) -> Result<Vec<AgendaItem>>
vault.overdue(timed_before_ms: i64, day_before_ms: i64, limit: u32) -> Result<Vec<AgendaItem>>
vault.overdue_count(timed_before_ms: i64, day_before_ms: i64) -> Result<u32>
vault.set_agenda_done(page: PageId, block: Option<BlockId>, done: bool) -> Result<()>
vault.in_motion(limit: u32) -> Result<Vec<Progress>>
vault.daily_note(day_ms: i64, title: String, create: bool) -> Result<Option<PageId>>
vault.link_task_to_day(day_ms: i64, task_page_id: PageId) -> Result<bool>
vault.append_log(day_ms: i64, text: String) -> Result<PageId>
vault.calendar_items(timed_from_ms: i64, timed_to_ms: i64, day_from_ms: i64, day_to_ms: i64) -> Result<Vec<AgendaItem>>
vault.day_notes_in_range(from_ms: i64, to_ms: i64) -> Result<Vec<i64>>

// Links, attachments, history, change feed (media.rs)
vault.backlinks(id: PageId) -> Result<Vec<PageSummary>>
vault.add_attachment_from_path(path: String) -> Result<AttachmentId>
vault.add_attachment_bytes(name: String, bytes: Vec<u8>) -> Result<AttachmentId>
vault.attachment_path(id: AttachmentId) -> Result<String>
vault.attachment_info(id: AttachmentId) -> Result<Attachment>
vault.gc_attachments() -> Result<u32>
vault.list_versions(page: PageId) -> Result<Vec<Version>>
vault.load_page_at(page: PageId, version: String) -> Result<Page>
vault.restore_version(page: PageId, version: String) -> Result<()>
vault.poll_changes(since_seq: u32) -> ChangeBatch

// Sync hooks (sync.rs): listed in Sync (state)
vault.page_versions(), sync_page_versions, export_updates, import_updates, import_updates_batch, purged_pages,
import_purged, wait_changes, list_conflicts, conflict_log, resolve_conflict,
undo_conflict, conflict_diff, conflict_auto_resolve, set_conflict_auto_resolve, attachment_ids,
missing_attachments, store_attachment; version_includes(a, b), sha256_hex(bytes)

// Markdown in (import.rs)
vault.import_markdown(dir: String, parent: Option<PageId>) -> Result<ImportReport>
vault.rescan() -> Result<u32>

// Seeds
seed::generate(root: String, pages: u32, big_page_blocks: u32) -> Result<()>
seed::generate_tasks(root: String, rows: u32) -> Result<PageId>
new_id() -> String
```

- `PageId`, `BlockId` and `AttachmentId` are `String` aliases. Page ids are 26-character Crockford base32 strings: ordinary pages use ULID-style random ids; new Journals and day notes use the stable identities described below. Block ids are any non-empty string the caller chooses; attachment ids are SHA-256 hex strings.
- `VaultError` is one enum (`NotFound`, `Invalid`, `Io`, `Index`, `Crdt`, each with a message). It implements `Display`, `std::error::Error`, `Serialize` and `Into<String>`.
- `PageSummary` gained `created_ms`, `icon`, `is_collection`, `favorite` and `properties`; `Page` gained `parent`, `icon`, `cover`, `properties`, `is_collection`, `favorite`, `in_trash`, `created_ms` and `updated_ms`; `Block` gained `target`. The new fields have `#[serde(default)]`, so JSON from the first API still deserializes.
- New block kinds `LinkToPage`, `Image`, `File`, `Bookmark` point at a page, attachment or URL through `Block::target`, set with the new op `BlockOp::SetTarget { id, target }`. New mark kind `MarkKind::PageLink(PageId)`.
- Editor kinds (2026-10-01, Pages and editor track): `Toggle`, `ToggleHeading1..3`, `Callout` (its `target` is the icon emoji), `ColumnList` (children are `Column` blocks), `Column` and `TableOfContents`. Every block has an optional `Block::color` (names: `red`, `red_background`, ...), set with `BlockOp::SetColor { id, color }`; `default` or None clears it. New marks `MarkKind::Underline`, `Color(String)` (one colour per range) and `Date(String)` (an ISO date; the marked text shows it).
- Every block also has an optional `Block::width` (an image's width in percent of the text column; None is full width), set with `BlockOp::SetWidth { id, width }`. `Insert` carries no target, colour or width; the editor sends `SetTarget`, `SetColor` and `SetWidth` after it.
- `BlockOp` is internally tagged: `{"op": "SetText", "id": "b1", "text": "Hi", "marks": []}`. `MarkKind` is externally tagged: `"Bold"`, `{"Link": "https://…"}`. `BlockKind` serializes as its variant name (`"Heading1"`).
- `Insert` and `Move` take `parent: None` for the top level and `after: None` for "first child of `parent`". A non-empty `after` must currently be a child of `parent`.
- A new page has no blocks, unless its parent has a default template.
- Journal day notes created through `daily_note` start with empty top-level `Tasks` and `Log` toggles, in that order, bypassing the Journal's default template. Existing notes and templates stay unchanged; new Journals have no default template. `link_task_to_day` validates the page's `tasks` type data and adds a `LinkToPage` child under `Tasks` only once per task per day, returning whether it added a link. It creates the note through `daily_note` if needed and inserts a missing Tasks toggle before Log in older notes. This uses the editor's existing toggle container and page-link blocks; no editor support or storage format is added. Creation and rescheduling callers are not wired to it yet, and it never removes old-day links. `append_log` creates or reuses the day note and appends a paragraph `HH:MM  text` using the local core clock. It creates a top-level `Log` toggle at the end only when absent, and leaves an existing Log in place. Toggle open state belongs to the device's editor, not the core block.
- `calendar_items` excludes Journal database rows as dated items; dated blocks in those notes remain eligible. `day_notes_in_range` returns sorted, distinct all-day Date stamps in `[from_ms, to_ms)`, excluding trash and templates. Day notes remain ordinary pages in search and the page tree.
- Dated to-dos in `calendar_items` and `agenda` expose a transient `date_mark` reference (Unicode scalar range and original ISO); `move_date_mark` validates it under the document lock and changes only its day through one normal block edit, preserving time, text and other marks, with stale references rejected and no storage/index format change.
- `daily_note(create: true)` holds the separate `.vault/daily-notes.lock` across Journal/day lookup and creation, so independent handles on one vault directory recheck the shared index and create one note; lock acquisition precedes document/flush locks (never flush → daily), and `append_log`/`link_task_to_day` inherit it. The lock covers one directory; stable page identities handle independent offline creation as described below.

### Offline Journal identity and repair (2026-10-13)

- A newly initialized Journal uses `000000000000000000000000JR`, separate from the hidden Calendars key. There is no vault id; `.vault/device` remains a random device identity. These page ids need only be unique inside their vault.
- `ids::journal_day_id(journal, iso_day)` derives a day-note key from the **actual Journal id** and ISO date. It hashes the UTF-8 string `notes-app:journal-day:v1\0{journal}\0{iso_day}` with SHA-256 (`\0` denotes a NUL byte), takes the first 128 bits, and encodes them as 26 Crockford base32 characters. For the fixed Journal and `2026-10-06`, the key is `307ZX565BYK7TKY0P3XHQ07H11`. The function takes the Journal key explicitly so a future vault-specific key can be supplied without building vault identity now. `daily_note` validates a UTC calendar-midnight stamp with a four-digit ISO year before creation.
- Existing role and Date lookup still wins. Older random-id Journals and day notes keep working without changing ids or content. New notes in an existing random-id Journal derive their ids from that Journal's key, so devices sharing it agree on new dates. Independently created old Journals are not merged by this change.
- On creation, a purged preferred Journal/day id gets a random replacement because its tombstone permanently suppresses sync imports. A matching trashed preferred page is restored and reused with its content, title and Date retained. Read-only lookup continues to skip trash. An occupied id with a different role, parent, template flag or day identity is refused rather than overwritten. A matching document without an index row is indexed and reused; unrelated files are preserved and refused. New day properties and random seed blocks are initialized before installation. No public API, binding or storage-format change is required.
- The independent-creation experiment merges the same page id into one Loro document, with both devices' content retained. Separately inserted Tasks/Log nodes remain separate at the CRDT level. The production repair moves the extras' children into the smallest block-id keeper with `BlockOp::Move` before deleting the extra section. Reusing block ids does not deduplicate nodes and makes one node unaddressable through the id map, so seeds retain random block ids. See `independent_same_id_pages_*` in [sync tests](../../crates/core/tests/sync.rs), and the actual daily-note identity regression in [day-note tests](../../crates/core/tests/day_notes.rs).

**Edit beats delete.** If one device deletes a day-note section or trashes a whole day note while another continues editing it offline, the editing device's full version is restored, including items the deleting device removed. The user clearly still used that section or note. Restoring loses nothing; at worst the user deletes it again. Known children moved by seed repair and late edits remain preserved. When restoration duplicates a Tasks or Log heading, the smallest block-id heading survives. The visible, non-restored section's existing items remain first, followed by the restored section's items in their original order, whichever heading survives. An item with the same block id on both sides remains once, at its visible position; the empty extra heading is deleted after its items are moved. Ordinary pages and templates keep their existing behavior, and permanent purge tombstones still win.

Restoration uses causal history to find the editing view and emits existing Loro operations under the repairing device's peer. The original physical tree-node identities and logical block ids are retained, including when both devices restore concurrently. No per-section restoration marker or new document field is added. A deletion with no concurrent content edit stays deleted.

Repair runs after sync import. Hot imports and shared-directory disk refresh call `normalize` through `merge_cached_import`; the completed import batch calls `repair_imported_day_notes` for cold pages and day notes whose Journal identity just arrived. `load_page` and `daily_note` lookup do not run repair.

`Vault::open` runs the legacy repair pass only when the local `meta` key `day_notes_repaired_version` is missing or older than the current repair version (`1`). It rechecks under the existing day-note lock, repairs and flushes documents, then writes the marker. A failed flush leaves the marker missing or old so the next open retries. A current marker skips the pass entirely, with no document writes or version bumps. Raising the repair version or rebuilding the index repeats the pass once. The table is added through the index's existing additive table initialization, without changing the synced document format.

Ordinary text/text conflicts remain pending when both devices actually edited a block differently from their common version. V2 records the actual day-section restoration as one log-only `auto(journal)` entry, with full descendants and historical deletion/edit frames. The repair's actual footprint suppresses a duplicate delete/edit conflict for the same restoration; it does not suppress unrelated text/text events. See [Conflict format v2](conflict-format.md).

Historical deleted-block lookup uses a lazy identity index and at most eight cached predecessor views per conflict side. The ignored release benchmark `bench_before_block_deletion_5000_blocks_300_restored_ids` measures 300 ids on a 5,000-block page. On the Linux machine, the original mean was 161.570 ms/call; a direct-node read reduced it to 67.773 ms/call. Sharing predecessor views reduced the mean to 0.551–1.100 ms/call across two runs (median 0.014–0.015 ms, p95 0.029 ms). The cold first call was 159.877–324.318 ms and is included in those means. These noisy manual measurements include lazy preparation and are excluded from the normal test suite.

**Core backend built and checked (2026-10-14):** the [approved v2 contract](conflict-format.md) replaces single-block records with stable IDs and `Block`, `Subtree` and `Page` scopes. Sides preserve scoped trees with every descendant, frontiers, lifecycle state and placement anchors. Delete/edit selects the editing version at detection; the switch controls whether its already-applied entry needs OK or goes straight to the log. Text/text sets the block to the later side's content at detection (device id breaks ties), recorded as `auto(later-text-wins)` and still pending until a pick. `Vault::list_conflicts` is the pending API; the backend accepts only `Keep` and `UseSide`, identified by record ID. The bridge adds the `conflicts`/`list_conflicts` aliases.

Purge tombstones retain the purging device id when available, so the purged conflict side reads as Local on that device; missing ids fall back to `"unknown"`. Sync carries the optional field through the `X-Purge-Device: 1` capability. Explicit purge choices retain their origin and the tombstone coverage they supersede in the synced lifecycle record.

Applications retain the chosen `Keep` or `UseSide(i)` (unknown for old records), `before` frontiers/non-live state, `by`, ancestor IDs, fallback metadata and `undone_at`. Scoped undo restores that before-state while preserving unrelated page edits. Choosing or undoing a historical scope rebuilds the required missing ancestor chain in order; undo removes ancestors introduced by the chosen application. If history cannot rebuild an ancestor, restoration appends at the page root and records a fallback. The user settled undo status on 2026-10-14: ordinary conflicts reopen pending review and retain an undone application in the log/history; journal repairs stay log-only. A consumed application cannot be undone again.

The synced vault policy lives in `sync.conflict_auto_resolve` type-data on the hidden Calendar settings page. Resolved log records expire 30 days after their latest application or undo. Consumed application metadata on reopened pending records expires 30 days after undo, while the pending record survives. Page history retains every version. A purged conflict-bearing page retains its Loro history, records and lifecycle metadata under `.vault/conflict-history/<page>.loro`; transport's `sync_page_versions` includes those archives, while ordinary `page_versions` keeps its loadable-page meaning. Review/history and scoped undo must remain available without silently recreating an archived page. Old records have block scope, no frontiers and no conflict undo.

`list_versions` and `load_page_at` read retained conflict history after purge without making the page active. `restore_version` can explicitly restore an archived page as a new historical/lifecycle edit. Active-page history merges any retained archive so earlier side and undo frontiers remain readable after restoration.

**Accepted focused checks:** core unit checks report 199 passed and 2 ignored; 5 scoped detection tests, 5 public resolution/history tests and 13 unchanged import-recovery tests also pass. All 54 sync tests pass; details are in [Sync state](../sync/sync-state.md#conflicts). FRB 2.13.0 regeneration and native bridge/fake-agent builds are complete; 21 Rust bridge tests, 12 MCP tests and 12 Flutter tests pass, with clean analysis. After master fixes `16e6080` and `11c2c0b`, 8 journal, 13 unchanged import-recovery and 3 HTTP conflict-history tests pass again, along with the 12 Flutter tests and analysis. The full workspace gate passed on the Linux machine on 2026-10-14 after detector fix `3093be8`: 775 passed, 5 ignored, 0 failed. It ran `cargo test --workspace --locked --exclude notes-tauri` through `<linux-build-helper> --with ../BubblePaw-base-agent`. No deployment is claimed. The [approved merge-screen mockup](../../design/merge/index.html) remains a later UI build; the current block controls only adapt old manual/AI acceptance into a normal `SetText` edit followed by `Keep`.

**Next deferred item: legacy merge of old random-id Journals/day notes.** Preserve their current lookup and identities for now. Define how to merge legacy parents, date rows and sections while retaining all content and references; do not silently choose one old Journal as a migration target.

### Track B additions (2026-10-01)

```rust
// Properties: PropKind::{Files, Formula, Rollup}; PropValue::{Files(Vec<AttachmentId>), Error(String)}
vault.list_templates(parent: PageId) -> Result<Vec<PageSummary>>
vault.create_template(parent: PageId, title: String) -> Result<PageId>
vault.create_page_from(parent: PageId, title: String, template: Option<PageId>) -> Result<PageId>  // None: empty
vault.set_template(page: PageId, on: bool) -> Result<()>
vault.apply_template(page: PageId, template: PageId) -> Result<()>
vault.list_mounts() -> Vec<Mount>                    // Mount { page, folder }
vault.add_mount(folder: String, parent: Option<PageId>) -> Result<Mount>
vault.remove_mount(page: PageId) -> Result<()>
vault.rescan_mounts() -> Result<u32>
```

- `set_default_template(collection, template)` now works for any page, and marks the template as one. Clearing the default keeps the template.
- `rescan` also rescans the mounts.

### Behaviour changes to the first API

- `delete_page` moves the page and its subtree to the trash instead of deleting them. Their markdown files are removed; `restore_page` writes them again, `purge_page` or `empty_trash` delete the documents for good. Deleting a page that is already in the trash (itself or through an ancestor) does nothing.
- `list_pages`, `search`, `page_count`, `list_children`, `backlinks` and `query` leave out pages in the trash; `page_count` and the lists also leave out the hidden calendars page. `load_page` still loads them, with `in_trash` set.
- `create_page` appends the page after its siblings (sibling order) and copies a collection's default template into new rows.
- Markdown files now carry properties, links and new block kinds, and `rescan` reads outside edits back (see below). Frontmatter keys are written plain when they are simple YAML keys and quoted otherwise.
- Emphasis whitespace: text like `**ab **` is written as `**ab** ` (the space moves outside the delimiters) so it parses back to the same marks.

## On disk

```
<vault>/
  pages/<slug>-<id>.md                 # markdown + YAML frontmatter, written by the writer
  .vault/device                        # this copy's device id (random, created on first open)
  .vault/lock                          # held by the Vault that uses the device's Loro peer
  .vault/purged                        # purge tombstones, one page id per line
  .vault/hub/tokens.json               # hub only: paired device tokens (hashed)
  .vault/docs/<id>.loro                # Loro snapshot (source of truth together with the log)
  .vault/docs/<id>.blocks.json         # block cache, pages with 256+ blocks only
  .vault/logs/<id>.log[.old]           # Loro updates not yet in the snapshot
  .vault/cold/<id>.cold               # compressed original history prefix
  .vault/cold/dictionaries/<id>.dict  # immutable device-local compression dictionary
  .vault/cold-policy.json             # optional local cutoff/size/idle-pass parameters
  .vault/blobs/<2 hex>/<sha256>        # attachments, content-addressed
  .vault/index.sqlite (+ -wal, -shm)   # index, rebuildable
  .vault/mounts.json                   # this device's mounted folders (not synced)
  .vault/mount-trash/<id>.md           # files of mounted pages moved to the trash
```

- The slug is the lowercased title with runs of non-alphanumeric characters turned into `-`, at most 50 characters, or `untitled`.
- Frontmatter holds `id`, `title`, `parent`, `created`, `updated`, then `icon`, `cover`, one key per property (by display name for collection rows) and a `links` map from wikilink title to page id.
- Page links are Obsidian wikilinks (`[[Title]]`, `[[Title|text]]`); titles that are empty, ambiguous or unusable in a wikilink are written as the page id. `LinkToPage` blocks are `![[Title]]`, images `![caption](../.vault)`, files `[name](../.vault)`, bookmarks `<url> caption`, empty paragraphs `<br>`. Everything written parses back to the same blocks.
- What markdown has no syntax for goes in an HTML comment at the end of the block's first line: `- text <!-- toggle -->`, `## text <!-- toggle -->`, `> text <!-- callout icon=💡 -->`, `<!-- color=red -->` on any block. Column lists, columns and tables of contents are lines holding only `<!-- columns -->`, `<!-- column -->` or `<!-- toc -->`, with the columns' blocks indented below. Comments with other words stay ordinary text. Underline is `<u>`, text colour `<span data-color="red">`, a date mention `<time datetime="2026-10-01">`.

## Inside a page's Loro document

- Map `page`: `id`, `title`, `parent` (string or null), `created`, `updated` (ms since the epoch); also `icon`, `cover` (attachment id), `favorite` (ms when favorited), `trashed` (ms when moved to the trash), `order` (sibling order key), `collection` (bool), `template` (page id: the default template for new children) and `istemplate` (bool: the page is a template for new children of its parent), each absent or null when unset. Display options (2026-10-01): `small_text`, `full_width`, `locked` (bool) and `cover_position` (per mille, 0 = top of the image), absent or null for the defaults.
- Organizer metadata (2026-10-06) adds optional `color` and `role`, plus collection settings `agenda_role` and `agenda_date`. These remain document data, so sync preserves them and rebuilding the index does not assign or overwrite colours.
- Tree `blocks`: a `LoroTree` (movable tree, fractional index on, jitter 0) with one node per block. Each node's meta map holds `id` (the BlockId), `kind` (lowercase string such as `heading1`; unknown kinds read as paragraphs), `checked` (bool), `target` (optional string), `color` (optional string), `width` (optional integer) and `text` (a `LoroText`).
- Marks are Loro text styles `bold`, `italic`, `strike`, `underline`, `color` (expand after; `color` holds the colour name), `code`, `link`, `pagelink` and `date` (expand none; the value is the URL, page id or date). Offsets are Unicode scalar values, as in the API.
- Map `props`: property key to the `PropValue` as JSON.
- Collections only: map `schema` (key to `PropDef` JSON) with list `schema_order`, and map `views` (id to `ViewDef` JSON) with list `view_order`. Views live in the collection's document, so they will sync with it.
- Map `peers`: Loro peer id to device id, written by each session on its first change, for history.
- All state that must sync (tree position, trash, favorites, icon, cover, display options, properties, schema, views, template) is in the page documents. The index holds only copies.
- `SetText` runs `LoroText::update` (Loro's diff) when the text changed, then re-marks only the style keys that differ.
- Map `conflicts`: stable record id to conflict JSON, including pending and resolved v2 records, their historical sides and applied-resolution history. Format-1 records use their block id as the record id. The same page Loro document carries resolution changes to other devices; a retained purge archive keeps that history outside active page storage.
- A vault's documents use one Loro peer id derived from the device id; a second `Vault` open on the same folder uses a random one (see [Sync (state)](../sync/sync-state.md#stable-peer-ids)). History forks use random peers. Loro records change timestamps for history.

## Edits and durability

- `apply` validates the whole batch first against a simulation of the tree (ids exist, `after` belongs to `parent`, no move into the block's own subtree, no duplicate ids, marks within the text), then applies it as one Loro commit. A failing batch changes nothing.
- Every committed change is exported as a Loro update and appended to `.vault/logs/<id>.log` (4-byte little-endian length, then the bytes) on the caller's thread. There is no fsync.
- Loading a page imports the snapshot, then `<id>.log.old` and `<id>.log`. A torn last frame is ignored.
- `open` lists `.vault/logs` and queues every page found there for the writer, which finishes interrupted flushes.

## Background writer

- One thread (`vault-writer`) per vault with its own SQLite connection. It flushes 300 ms after the last change, or at the latest 2 s after the first change of a burst.
- Before writing a page, check snapshot/log identities and sizes plus the vault-wide sequence under the OS advisory `.vault/flush.lock`; import changed disk content into the cached Loro document first. Persist the merged snapshot durably before truncating its merged logs in place, so retained append handles remain valid and interrupted cleanup safely replays included ops.
- Along with the markdown hash, the index stores the document version the markdown was rendered from (`md_version`). `rescan` uses it as the merge base.
- `flush()` waits for everything queued and returns the last background error. Dropping the `Vault` flushes and stops the thread. Loaded documents stay in a cache of 32 pages; dirty pages are never evicted.

## Index

`user_version` 7; an index with another version is recreated and rebuilt from the Loro files on `open`. `reindex()` does the same on demand.

- `pages`: id, title, folded title, parent, created, updated, markdown path, hash and version, `sort_key`, icon, cover, favorite, `trashed`, `in_trash` (the page or an ancestor is trashed), `is_collection`, `is_template`, schema JSON, template. Indexes: `(in_trash, updated_ms DESC, id DESC)` for the recent list, `(parent, in_trash, updated_ms DESC, id DESC)`, `(parent, in_trash, sort_key, created_ms, id, title_fold, updated_ms)` for sibling order and covering collection scans, plus favorite, trashed and title.
- `props`: one row per page and property, `(page, key)` primary key, typed columns `t` (text or option ids joined by `\x1f`), `tf` (folded text), `n` (number, checkbox 0/1), `d0`/`d1` (date start/end), and the JSON value. Indexes on `(key, t)`, `(key, n)`, `(key, d0)`.
- Schema 8 adds `pages.color`, `role`, `agenda_role`, `agenda_date`, `todos_total`, `todos_done` and nullable `task_done`, plus `agenda(page, block, kind, d0, d1, has_time, done, title)` with indexes on `d0` and `(done, d0)`. A schema-version change rebuilds the index from documents; agenda rows are derived by the same function on incremental writes, sync imports and full reindex.
- `links(src, dst)` with an index on `dst`; `refs(page, blob)`; `attachments(id, name, mime, size, created)`.
- `pages.vv`: the page's encoded version vector, for loadable `page_versions`. The `conflicts(page, id, anchor, json, scope)` mirror records stable ID, anchor block and scope; JSON includes status/history, and pending/log queries filter it. Archive history is additionally advertised by `sync_page_versions`, not installed as a loadable page row.
- Write transactions start with `BEGIN IMMEDIATE`. A deferred one that reads first failed at once with "database is locked" under WAL when the other connection had committed in between.
- `fts(title, body)`, tokenizer `unicode61 remove_diacritics 2`, prefix indexes of 1, 2 and 3 characters.
- The connection uses a 16 MB page cache (SQLite's default is 2 MB). With the default, queries that read every row of a 10,000-row collection were about 3 times slower.
- Tree, trash, property, schema and favorite changes update the index on the caller's thread, so queries see them at once. Block edits (body, links, attachment references) reach it through the writer, so `search` and `backlinks` see them up to 2 s later, or right after `flush()`.

## Properties, collections and queries

- A property value is checked against the collection's schema for rows (known key, matching kind, existing option ids). Pages outside collections take any key not starting with `$`. `Empty`, empty text and empty lists remove the property. `CreatedTime` and `UpdatedTime` are computed from the page.
- `set_schema` assigns missing keys and option ids. A known key with a new name is a rename. Removing a property or option removes the values from every row; a kind change converts values where it can (for example select to multi-select, text to options by name) and drops the rest. Views lose references to removed properties; filter groups that become empty are dropped, but groups that were empty stay.
- **Templates.** Any page can have templates: children marked `istemplate`. One can be the parent's default (`template`, which also marks it). `list_children`, `list_pages`, child counts, search and queries leave templates out (and a default template from before the flag). `create_page` copies the default template; `create_page_from` copies the one given, or none. Copying takes the blocks (new ids), and properties, icon and cover only where the page has none. Blocks replace a blank body and are appended otherwise (`apply_template` on a page with content). Title and sub-pages are not copied. A collection's default template is this same mechanism.
- **Files** properties hold attachment ids from the blob store; `gc_attachments` keeps them.
- **Formula** properties evaluate an expression (the def's config key `formula`) over the row: literals, `prop("Name")`, arithmetic, comparison, logic, `? :`, and functions for text, numbers, dates and lists (see `formula.rs`). **Rollup** properties aggregate a property of the pages a relation points at (config keys `rollupRelation`, `rollupProperty`, `rollupFunction`: `show_original`, the counts, percents, `checked`/`unchecked`, `sum`, `average`, `median`, `min`, `max`, `range`, earliest and latest date, date range).
- Computed values are never stored in documents. They are written to the index's `props` table when a row's index entry is written, and for the rows whose relations point at it (up to 4 levels). So filters, sorts and groups work on them like stored values. An error (unknown property, wrong type, division by zero, a cycle) is the value `PropValue::Error(message)`, shown in the cell.
- `query` compiles the filter tree to SQL: each condition is an `EXISTS` over `props` (or a comparison on `pages` for `$title`, `$created`, `$updated`). Negative ops (`IsNot`, `NotContains`, `IsEmpty`, `Unchecked`) are `NOT` of the positive one, so pages without the property match them. The query fetches every matching row in the scope's order (sibling order for a collection, last update for the vault, both given by indexes) together with its sort keys, then sorts in Rust. That avoids an SQL sort and a window count, and the total comes for free. Empty values sort last in both directions.
- `query_groups` counts rows per option (select, status, multi-select: an empty column first, then options in schema order), per checkbox value, or per distinct value.
- `query_range` returns one entry per page (and per property for `*`) whose date overlaps `[from, to)`, ordered by start. A date without an end counts as the instant it starts.

## Agenda, progress, daily notes and spaces (2026-10-06)

- **Status groups.** `status_group` reads the Status definition's config JSON `groups`; otherwise the first option is To do, the last Complete and the rest In progress. `first_option_in` picks the first option in schema order in the requested group, used when completing or reopening a row.
- **Collection agenda settings.** `AgendaSettings { role: AgendaRole, date_prop }` stores `agenda_role` and `agenda_date` in the collection's document. Auto uses Tasks for a Date plus Status or Checkbox, Events for a Date alone and Off otherwise; created and edited time do not count, and an absent or removed date override falls back to the first Date property.
- **Agenda entries.** Dated collection rows and to-do blocks with a valid ISO date mention become `AgendaItem`s; the block title removes the date mention text. Tasks use the first Status, otherwise Checkbox, for completion; block tasks use `checked`, and Tasks rows have `task_done` even without a date.
- **Queries.** `agenda` includes done items, ordered whole-day first, then start and title, with overlap `d0 < to AND coalesce(d1,d0) >= from`; `overdue` returns undone tasks whose end (or start without an end) precedes the bound, oldest first, capped by `limit` in SQL before decoding and resolving spaces. `overdue_count` returns the uncapped total with the same date, trash and template exclusions, without loading items or spaces. Both use separate instant bounds for timed values and UTC day bounds for whole-day values, exclude trash and templates, and include the parent collection and top-level space with its colour, resolving shared ancestors through a cached parent walk.
- **Completed overdue tasks.** `done_since(timed_before_ms, day_before_ms, edited_since_ms, limit)` uses the same before-bound rule for done tasks and keeps only pages whose `updated_ms` is at or after the edit threshold. It is capped in SQL before decoding, ordered newest page edit first, and uses the existing `agenda_done_date` index without a schema migration. The page edit stamp approximates completion time: editing a previously completed page can include its overdue tasks. Today requests 50 items edited since local midnight and merges them into today's done group only.
- **Recompute and progress.** Content, property, title and parent writes refresh a page's agenda; schema or agenda-setting changes refresh the collection's rows and report the collection to the change feed. `in_motion` combines unfinished page checklists and Tasks collections with open rows, including undated rows, newest first; a collection's timestamp is its newest live row's update, and block-derived agenda and progress follow the existing writer delay (or `flush`).
- **Daily notes.** `daily_note` finds the live Journal by role, so renaming it is safe, and returns the existing whole-day Date row before creating anything. On first creation it makes the Journal with agenda Off and a Date property. New day notes start with Tasks then Log toggles, bypassing default templates without changing existing data.
- **Spaces.** `Page.color` and `Page.role` are optional document metadata, carried through the bridge; the current page role is `journal`. New top-level pages, imports without a supplied colour and top-level copies store the explicit colour value `none`, so they start uncoloured and remain so across boot backfill and reopening. Existing colours and supplied import colours are retained. `ensure_space_colors` still fills legacy unset or empty colours in sidebar order, cycling through apricot, paw, moss, sun, sky and duvet, and leaves explicit `none` and existing palette colours untouched. The page menu lets the user choose a colour; setters accept `none` alongside the six palette names and reject other names.

## Page tree and trash

- Sibling order is a base-62 fractional key (`order` in the page document, `sort_key` in the index). Appending uses 4-digit prefixes; inserting between two keys takes a midpoint. `move_page` rejects moves into the page's own subtree.
- `duplicate_page` copies the page and its subtree (blocks with new ids, properties, icon, cover, schema, views; not the favorite flag) right after the original, with " (copy)" appended to the title.
- The trash is the `trashed` time in the trashed page's document; the index derives `in_trash` for the subtree. `restore_page` clears it. Pages inside the subtree that were trashed on their own stay in the trash. If the parent is gone or still in the trash, the page comes back at the top level.
- Favorites keep the order they were favorited in (the favorite time, kept strictly increasing).

## Links and attachments

- Inline `PageLink` marks and `LinkToPage` blocks are both links. The writer stores them in `links`; `backlinks` lists live pages linking to a page, most recently updated first. Renaming rewrites ordinary mentions in markdown; imported links keep their authored targets, with the export's `links` map retaining their page IDs.
- Attachments are stored once per content (`sha256` hex is the id), with name, MIME type (from the extension) and size in the index. `gc_attachments` flushes, then removes blobs that no page, in the trash or not, uses as image, file or cover.
- SHA-256 is implemented in the crate (FIPS 180-4) to avoid a dependency.

## History and the change feed

- Historical storage-lane item 3 validation (2026-10-13) used the same cross-instance flush protocol: prepare compression outside page locks, then reread/merge under `.vault/flush.lock`; cancel preparation if newer disk operations arrived. Write durable cold before shallow hot, bump the durable sequence before mutation, and truncate included logs in place. History, older exports and outdated-dependency import recovery read the newest hot/cold/log state under that lock; recent operations do not read cold payloads. 38 focused cold tests and 12 retained two-writer tests pass on the Linux machine, including a second instance's flush and its unflushed append followed by process crash. The aged fixture moved all 10,001 pages, including dictionary compression. That historical timing and acceptance run was blocked by a separate master new-page import defect: a failed parallel batch leaves snapshot files without index rows, and identical retry/reopen does not repair visibility. That run retained a focused failing regression; the later master recovery fix and subsequent integration checks are documented in the linked storage page. That full Linux machine acceptance run reported 482 passed, one failed (that regression), four ignored; all 11 shallow-limit tests pass. See [implementation, checks and measurements](../sync/sync-and-storage.md#cold-history-implementation-and-validation-2026-10-13). No type-data implementation or bridge API was changed.

- `list_versions` reads the document's change log sorted causally (Lamport time) and groups it: a new version starts after 5 idle minutes or when the next change comes from another device (through the `peers` map). A version id is the version's frontiers (`peer@counter,…`). The summary names what changed (content, title, properties, and so on).
- `load_page_at` forks the document at the version. `restore_version` computes the ops that turn the current blocks into the old ones (unchanged blocks keep their ids) and restores title, icon, cover and properties, as one new change. Tree position, trash, favorite, schema and views are kept.
- The change feed is an in-memory window of the last 10,000 changes with a sequence number starting at 0 at each `open`. `poll_changes(since)` returns the changes after `since`, each page once, or `reset: true` when `since` is outside the window. Every mutating call and the writer (content edits) push to it.

## Import and rescan

- `import_markdown(dir, parent)` imports markdown and frontmatter under a parent. Folder bodies use a same-named adjacent `.md`, then `index.md`, then `README.md`; a body file is not imported again as a child. GFM tables keep their verbatim markdown in `table` blocks; code fences keep their info string in the existing block `target` attribute.
- Relative `.md` links resolve from the source file's directory, with normalized paths, URL decoding and fragments; basename fallback requires an unambiguous match. Resolved links retain their original target alongside the page link, including links in tables (marks cover their raw labels; table text stays verbatim). Missing markdown links remain URL marks, while missing wikilinks remain literal text; both are reported with source file and target.
- Re-import uses reserved source-path and import-parent properties to reuse pages and update their content and tree position. An identical run adds no operations or timestamp changes. Missing sources are reported as orphaned and kept; pages without import keys are left alone. Referenced images and files reuse content-addressed blobs.
- The import report includes created, updated, unchanged and orphaned pages, tables, resolved and unresolved links, images, attachments and skipped files with reasons. Dot folders and secret filenames are excluded; `import_markdown_with_excludes` also accepts relative paths or globs. This importer is separate from the flat folder-mount rescan below.
- `rescan` flushes, then reads every `pages/*.md` whose hash differs from the one last written. It is a three-way merge: the base is the document forked at the version the file was rendered from, and only the differences between base and file are applied. App edits made after the file was written are kept, and unchanged blocks keep their ids. The changes are made on the fork and imported into the live document as a Loro update. A new `.md` without a known id becomes a new page; a missing file is written again.

## Folder mounts

- A mount is a page whose children are the top-level `.md` files of a folder on this device. Subfolders are ignored. The table is `.vault/mounts.json`, per device. Decisions and limits: [Folder mounts](mounts.md#built-2026-10-01).
- A mounted page's markdown path in the index is the file's absolute path. The writer writes `<folder>/<title>.md` (a taken name gets ` (2)`) without the vault's own frontmatter keys. It writes only when the file's hash is the one it last read or wrote.
- `rescan_mounts` reads files whose size or time changed, merges them as `rescan` does (three-way, on a fork), imports new files as pages and trashes pages whose file is gone. The bridge calls it every 2 s on desktop.

## Block cache

- For pages with at least 256 blocks the writer (and the seed) writes `<id>.blocks.json`: the blocks plus the document's frontiers. A cold `load_page` uses it when the version matches, and decodes the Loro state on the writer thread in the background.

## Seed

- `seed::generate(root, pages, big)` creates `pages` pages plus one titled "Big page" (`seed::BIG_PAGE_TITLE`), so 10,000 gives 10,001 pages. It fails if the vault already has pages. Titles and paragraphs come from about 130 words (a few with umlauts and accents) and a deterministic generator. 5% of pages are top-level; of the rest, 60% get an earlier page as parent. Pages now also get sibling order keys.
- `seed::generate_tasks(root, rows)` adds a top-level collection titled "Tasks" (`seed::TASKS_TITLE`) with `rows` rows. The schema has status (to do / in progress / done), due date (whole days within 90 days of today, 10% empty, 10% ranges of up to a week), tags (multi-select of 6), estimate (number), urgent (checkbox) and created time. Five views: all, open by due date (status is not done and due is set, sorted by due), board by status, calendar and timeline on due. It fails if a top-level "Tasks" page exists.
- Pages are built on up to 8 threads; the index rows are inserted in one transaction.

## Organizer measurements (2026-10-06)

The capped API was rerun on 2026-10-07 with `cargo run --release -p vault_core --example bench -- <fresh scratch vault> 10000 2000 --agenda-only`: a fresh 10,000-row Tasks seed (10,001 pages), 30 calls per query. The final independent run returned 353 week `agenda` items (p50 3.332 ms, p95 3.790 ms), 50 `overdue` items with limit 50 (p50 0.114 ms, p95 0.388 ms), and an `overdue_count` of 2,982 (p50 4.610 ms, p95 6.509 ms). All three measured p95 values were below the 20 ms target. This smaller Tasks-only vault differs from the earlier mixed 20,003-page fixture below; these are measured wall times rather than an idle-machine guarantee.

Before overdue was capped, release `bench -- <vault> 10000 2000 --agenda-only`, 30 calls per query, on the 10,000-row Tasks seed in a 20,003-page vault measured a week's `agenda` of 353 items (p50 7.145 ms, p95 20.153 ms, max 22.458 ms) and unlimited `overdue` of 2,982 items (p50 16.223 ms, p95 62.869 ms, max 75.068 ms); the strict 20 ms wall-time target was not met in that older run.

The queries start with the date-indexed agenda and join pages by rowid; shared space ancestors are cached per query. Wall times varied between runs, so the measured tails remain an acceptance gap rather than an established cause or an idle-machine guarantee.

## Search operators and filters (2026-10-10)

Step 2 of [Search queries](search-queries.md) is built: byte-spanned parsing, every approved operator and date form, stable page ids, canonical printing, completion, bounded totals and matched filter values. `Vault::parse_search`, `complete_search` and `search_spec` reach Dart through regenerated bindings; string search shares the executor. Agent `search_notes` returns its parsed reading and warnings while preserving body recall. Filters constrain every title, body, ranked and typo path. Executions share one SQLite snapshot; candidate sets cache against index-state guards and subtrees against a tree-only epoch. Parsing borrows cached metadata per keystroke.

The created-time index and indexed `(key, value, page)` multi-select table are added. The table was justified by 100 uncached tag searches: SEP lookup measured **41.937/51.343 ms p50/p95**, versus **4.742/5.401 ms** finally, including parsing and annotations. Atomic backfill and triggers maintain existing vaults. Small sets use `rarray`; unary `+` on FTS rowid avoids repeated MATCH scans. Large sets test membership after FTS. Sequential checks with `CARGO_INCREMENTAL=0` passed core/tools tests, bridge build/tests, binding generation, Flutter analysis and every release perf assertion. The 20 ms / 30 ms ceilings remain unchanged; these are warmed core wall times.

| Release case, exactly 10,000 pages, limit 50 | p50 ms | p95 ms | Samples |
| --- | ---: | ---: | ---: |
| Existing warm search workload | 0.749 | 3.521 | 950 |
| Existing typo workload | 3.476 | 11.097 | 650 |
| Uncached tag candidate construction, 100 distinct filters | 4.742 | 5.401 | 100 |
| Filter only: `tag:perf/selective` | 2.472 | 2.771 | 50 |
| Selective filter + text: `project tag:perf/selective` | 13.177 | 14.283 | 50 |
| Broad filter + broad text: `p tag:perf/broad` | 4.884 | 5.694 | 50 |
| Negatives only: `-tag:perf/excluded` | 2.148 | 2.448 | 50 |
| Deep `in:` subtree (32-level chain) | 3.475 | 4.045 | 50 |
| Tag on 10k: `tag:perf/all` | 1.854 | 2.232 | 50 |
| Typed `p` under constant `tag:perf/broad` | 4.304 | 5.445 | 50 |
| Typed `pr` under constant `tag:perf/broad` | 3.820 | 4.707 | 50 |
| Typed `pro` under constant `tag:perf/broad` | 3.632 | 4.694 | 50 |
| Typed `proj` under constant `tag:perf/broad` | 4.483 | 5.699 | 50 |
| Typed `project` under constant `tag:perf/broad` | 4.561 | 5.827 | 50 |
| Filtered typo: `gardne tag:perf/broad` (30 ms ceiling) | 9.211 | 11.271 | 50 |

**Search latency repair (2026-10-13):** historical Linux machine release core checks pass (324 tests, two ignored). The single-call coffee fix removes the unproven whole-result cache. Earlier Linux machine A/B verdicts are superseded for acceptance; qualification now uses the self-hosted CI runner job below. See [measurements](search-queries.md#interleaved-ab-acceptance-2026-10-13). The [Linux profile check](#linux-profile-search-measurement-2026-10-13) is complete; its UI timings are a relative baseline only.

## Search performance gate (2026-10-09)

**Latest relative gate (2026-10-13):** title-loop cost from `8712f52` is repaired in `f3d4b6d`; self-hosted CI runner `4f978b9` versus fixed head `c2da667` (run 38418744539 in the former private repo) passes **47/47**, with no new filtered FAIL, and the Linux machine release core checks pass **378 tests, two ignored**. Final unfiltered bands are 45-67%, so baseline parity is unproven; the isolated repair improves warm p50 13.5% with 5% bands ([cause, bisects and full table](search-queries.md#fixed-cost-investigation-2026-10-13)).

The search target is **p95 < 20 ms**, for warmed core queries and for UI input to visible results. `crates/core/tests/search_perf.rs` reuses `seed::generate` with 9,999 ordinary pages and one 2,000-block page (exactly 10,000 pages). It warms 19 single-word, prefix and multi-word queries, then interleaves 50 samples per query at limit 50. The release-only ignored test prints p50/p95 and retains its default absolute assertions (20 ms for exact/filtered cases, 30 ms for fuzzy cases).

**Acceptance rule (2026-10-13):** Run through [`.github/workflows/search-ab.yml`](../../.github/workflows/search-ab.yml), a `workflow_dispatch` job on **the self-hosted CI runner** (`runs-on: [self-hosted, <ci-runner>]`). Each run has at least five alternating AB/BA real pairs and an interleaved baseline/baseline control in the same session. Both control sides use the identical retained baseline executable, verified by SHA-256. For each case, the control spread is `abs(control median-round p50 ratio - 1)` and the noise band is `max(5%, control spread)`. A case fails only when its candidate/baseline median-round p50 ratio exceeds `1 + band`; p95 ratios are informational. The job prints the band and verdict per case and exits nonzero on FAIL. **Control-spread cap (2026-10-13), per case:** a case whose control spread is 10% or more is rerun once; if it is still 10% or more, it is reported **inconclusive** and neither passes nor fails. A run passes only if every conclusive case is within its noise band and at most 2 cases are inconclusive. (A noisy control otherwise widens the band until any candidate passes.) The harness does not enforce the cap yet; that change comes with the next rerun. The self-hosted CI runner is not isolated: it shares the Proxmox host with the Linux machine (both report an i5-1135G7), so other containers' load reaches its runs. Under the cap, the 2026-10-13 final gate does not pass: every unfiltered case had control spread of 45-67%, far more than 2 inconclusive cases; a confirmation rerun waits for GitHub billing to be fixed. **The Linux machine A/B verdicts are superseded for acceptance**; local and Linux machine harness runs remain diagnostics. Absolute hosted-runner and UI latency qualification are separate.

Dispatch after the workflow reaches master (omit `candidate` to use the dispatched ref's HEAD):

```bash
gh workflow run search-ab.yml --ref lane/search-perf \
  -f baseline=4f978b9 -f candidate=f6e411c -f pairs=5
```

### The self-hosted CI runner result: pre-lane baseline versus master (2026-10-13)

**`verdict: FAIL (21 cases beyond band)`**. Baseline `4f978b9` versus candidate `f6e411c`; five real pairs and five same-binary control pairs, 20 interleaved rounds total. All 20 rounds exited zero with all 47 manifest cases and unchanged sample counts/correctness assertions. The gate exited 1 because 21 p50 ratios exceed their case's band. Work stopped at this initial result; the later title-loop repair and final gate are recorded above.

Machine: **The self-hosted CI runner**, Intel Core i5-1135G7 @ 2.40 GHz, **four physical cores / four available CPUs** (`nproc` and Python report four; `lscpu` lists eight logical IDs with four online), Linux 6.17.13-21-pve, Python 3.13.5, Cargo 1.99.0. Both sequential builds used `--release --locked -j 2` and the persistent target cache. Only the baseline's temporary test file needed the report-only patch. The same baseline executable served both control sides; SHA-256 `6e7dcd6a7dd4d7d787cd7ecd447238bf01fe70233841d685519b572c32296f5e` was checked before every round and after the session. No artificial load was added.

Completed run and job summary (run 38399960905 in the former private repo); raw evidence artifact (run 38399960905 in the former private repo) (`search-ab-38399960905-1`, 30-day retention). Dispatch was refused with HTTP 404 because master lacked the workflow. The run used throwaway branch `bench/search-ab-run`, with one extra commit `904e352` adding the temporary push trigger and pinning candidate `f6e411c`. **Do not merge that commit.** The lane's workflow remains dispatch-only; the central chat merges it to master later. A preliminary workflow-context validation failure happened before any runner job; commit `7a51387` corrected it before this measurement session.

Harness selftests passed through `<linux-build-helper>` with `--repo /home/dev/work/search-ab` (the helper ships no git history), then passed on the self-hosted CI runner. Both builds succeeded, both detached revision worktrees were removed, and the result records no cleanup errors. Independently parsing the uploaded raw outputs reproduces all 940 case records and all medians, ratios, bands and verdicts. This run does not establish absolute hosted-runner latency, Flutter/UI latency or physical-device qualification.

Times below are median-of-round milliseconds; ratios are candidate/baseline. Spread and band are percentages. Displayed values are rounded; verdicts use full precision. Coffee is just beyond its band (p50 ratio 1.119804, band 11.964239%).

| Case | Baseline p50 / p95 ms | Candidate p50 / p95 ms | p50 ratio | p95 ratio (info) | Control spread | Band | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| `warm search, 10,000 pages, limit 50` | 0.781 / 3.401 | 0.962 / 3.719 | 1.232 | 1.094 | 16.92% | 16.92% | FAIL |
| `query "p"` | 1.079 / 1.526 | 1.328 / 1.553 | 1.231 | 1.018 | 13.80% | 13.80% | FAIL |
| `query "pr"` | 0.694 / 0.999 | 0.878 / 1.099 | 1.265 | 1.100 | 14.75% | 14.75% | FAIL |
| `query "pro"` | 0.609 / 0.861 | 0.748 / 0.915 | 1.228 | 1.063 | 16.19% | 16.19% | FAIL |
| `query "proj"` | 0.672 / 0.916 | 0.792 / 1.117 | 1.179 | 1.219 | 15.13% | 15.13% | FAIL |
| `query "project"` | 0.641 / 0.913 | 0.759 / 0.958 | 1.184 | 1.049 | 15.79% | 15.79% | FAIL |
| `query "g"` | 1.111 / 1.352 | 1.366 / 1.629 | 1.230 | 1.205 | 15.10% | 15.10% | FAIL |
| `query "ga"` | 0.723 / 1.037 | 0.899 / 1.016 | 1.243 | 0.980 | 17.01% | 17.01% | FAIL |
| `query "gar"` | 0.648 / 0.735 | 0.806 / 0.909 | 1.244 | 1.237 | 14.94% | 14.94% | FAIL |
| `query "gard"` | 0.751 / 1.045 | 0.903 / 1.129 | 1.202 | 1.080 | 15.28% | 15.28% | FAIL |
| `query "garden"` | 0.702 / 0.853 | 0.855 / 0.999 | 1.218 | 1.171 | 16.09% | 16.09% | FAIL |
| `query "a"` | 1.948 / 2.472 | 2.400 / 2.858 | 1.232 | 1.156 | 12.53% | 12.53% | FAIL |
| `query "t"` | 1.862 / 2.502 | 2.305 / 2.591 | 1.238 | 1.036 | 14.58% | 14.58% | FAIL |
| `query "tomato"` | 0.791 / 1.137 | 0.995 / 1.331 | 1.258 | 1.171 | 17.07% | 17.07% | FAIL |
| `query "weekly review"` | 1.096 / 1.398 | 1.061 / 1.324 | 0.968 | 0.947 | 13.22% | 13.22% | PASS |
| `query "coffee mor"` | 3.464 / 4.576 | 3.879 / 4.468 | 1.120 | 0.976 | 11.96% | 11.96% | FAIL |
| `query "zür"` | 0.552 / 0.845 | 0.691 / 0.913 | 1.252 | 1.080 | 14.77% | 14.77% | FAIL |
| `query "café"` | 0.804 / 1.205 | 1.007 / 1.234 | 1.252 | 1.024 | 14.11% | 14.11% | FAIL |
| `query "big page"` | 1.994 / 2.553 | 2.432 / 2.798 | 1.220 | 1.096 | 10.82% | 10.82% | FAIL |
| `query "xylophone"` | 0.701 / 0.873 | 0.685 / 0.993 | 0.977 | 1.137 | 15.36% | 15.36% | PASS |
| `fuzzy search, 10,000 pages, limit 50` | 3.079 / 7.051 | 2.605 / 7.882 | 0.846 | 1.118 | 5.32% | 5.32% | PASS |
| `typo query "tomatoe"` | 3.444 / 3.689 | 2.894 / 3.171 | 0.840 | 0.860 | 27.79% | 27.79% | PASS |
| `typo query "gardne"` | 3.480 / 3.712 | 2.697 / 3.046 | 0.775 | 0.821 | 26.81% | 26.81% | PASS |
| `typo query "weekyl review"` | 3.019 / 3.316 | 2.707 / 3.155 | 0.897 | 0.951 | 31.90% | 31.90% | PASS |
| `typo query "cofee"` | 3.302 / 3.562 | 2.612 / 2.965 | 0.791 | 0.832 | 28.06% | 28.06% | PASS |
| `typo query "cofee mor"` | 6.758 / 7.188 | 6.676 / 8.123 | 0.988 | 1.130 | 29.14% | 29.14% | PASS |
| `typo query "projcet"` | 1.819 / 2.038 | 1.819 / 2.342 | 1.000 | 1.149 | 29.32% | 29.32% | PASS |
| `typo query "recipies"` | 7.064 / 7.716 | 7.922 / 9.379 | 1.121 | 1.216 | 30.72% | 30.72% | PASS |
| `typo query "gadr"` | 4.097 / 4.345 | 3.178 / 3.677 | 0.776 | 0.846 | 27.17% | 27.17% | PASS |
| `typo query "meeitng"` | 2.165 / 2.333 | 2.037 / 2.741 | 0.941 | 1.175 | 29.27% | 29.27% | PASS |
| `typo query "buget"` | 1.994 / 2.198 | 1.776 / 2.259 | 0.891 | 1.028 | 27.58% | 27.58% | PASS |
| `typo query "desgin"` | 2.856 / 3.097 | 2.249 / 2.824 | 0.787 | 0.912 | 27.51% | 27.51% | PASS |
| `typo query "journla"` | 1.676 / 1.870 | 1.706 / 2.028 | 1.018 | 1.084 | 30.48% | 30.48% | PASS |
| `typo query "databsae"` | 2.230 / 2.440 | 2.090 / 2.408 | 0.937 | 0.987 | 28.59% | 28.59% | PASS |
| `uncached tag candidate construction, 100 distinct filters, 10,000 pages` | 5.569 / 6.773 | 2.167 / 2.998 | 0.389 | 0.443 | 14.55% | 14.55% | PASS |
| `filtered filter only, "tag:perf/selective", 10,000 pages` | 3.294 / 3.841 | 1.379 / 1.466 | 0.419 | 0.382 | 15.01% | 15.01% | PASS |
| `filtered selective filter plus text, "project tag:perf/selective", 10,000 pages` | 19.945 / 21.765 | 13.953 / 14.875 | 0.700 | 0.683 | 6.16% | 6.16% | PASS |
| `filtered broad filter plus broad text, "p tag:perf/broad", 10,000 pages` | 5.603 / 6.536 | 4.345 / 4.703 | 0.775 | 0.720 | 9.97% | 9.97% | PASS |
| `filtered negatives only, "-tag:perf/excluded", 10,000 pages` | 2.905 / 3.341 | 1.465 / 1.537 | 0.504 | 0.460 | 8.65% | 8.65% | PASS |
| `filtered deep in, "in:[[<ID>]]", 10,000 pages` | 3.977 / 4.693 | 2.929 / 3.079 | 0.736 | 0.656 | 13.08% | 13.08% | PASS |
| `filtered tag on 10k, "tag:perf/all", 10,000 pages` | 2.579 / 3.040 | 1.188 / 1.288 | 0.461 | 0.424 | 18.75% | 18.75% | PASS |
| `typed prefix "p", constant tag:perf/broad, 10,000 pages` | 3.951 / 4.175 | 4.213 / 4.734 | 1.066 | 1.134 | 2.25% | 5.00% | FAIL |
| `typed prefix "pr", constant tag:perf/broad, 10,000 pages` | 3.711 / 3.922 | 3.817 / 4.298 | 1.029 | 1.096 | 3.02% | 5.00% | PASS |
| `typed prefix "pro", constant tag:perf/broad, 10,000 pages` | 3.608 / 3.880 | 3.707 / 4.179 | 1.027 | 1.077 | 0.85% | 5.00% | PASS |
| `typed prefix "proj", constant tag:perf/broad, 10,000 pages` | 4.456 / 4.703 | 4.856 / 5.227 | 1.090 | 1.111 | 3.44% | 5.00% | FAIL |
| `typed prefix "project", constant tag:perf/broad, 10,000 pages` | 4.425 / 4.713 | 4.906 / 5.252 | 1.109 | 1.114 | 1.07% | 5.00% | FAIL |
| `filtered typo "gardne tag:perf/broad", 10,000 pages` | 8.245 / 10.239 | 5.671 / 5.888 | 0.688 | 0.575 | 6.81% | 6.81% | PASS |


`SEARCH_PERF_REPORT_ONLY=1` disables only the six timing assertions so all 47 cases can be compared. Correctness assertions, corpus, queries, limits and samples are unchanged. The harness adds this switch only to a temporary test-file copy when the baseline lacks it. See [search measurements](search-queries.md#interleaved-ab-acceptance-2026-10-13) for revision pairs, per-case ratios and per-round load.

`app/test/search_perf_test.dart` types `apple` one character at a time through the real Ctrl+K shortcut and the agent composer's `@` menu, using an exactly 10,000-page native seed. Each route has 100 measured keystrokes after warmup. Wall time includes text input, the bridge and the frame displaying completed results; it excludes GPU rasterisation. An immediate cached-search control measures the debug harness with the same widgets and results. `app/tool/test.ps1 perf` runs the Rust gate before the tagged Flutter tests, with one Flutter worker.

**Earlier core result, Windows desktop (hostname not recorded), 2026-10-09:** p50 **16.327 ms**, p95 **28.226 ms** over 950 searches. The release gate **failed** the 20 ms target; 16 of 19 queries also failed their individual p95 gate. The table below records that earlier implementation, before adaptive narrowing.

| Query | p50 (ms) | p95 (ms) |
| --- | --- | --- |
| `p` | 26.493 | 33.601 |
| `pr` | 15.003 | 21.536 |
| `pro` | 15.817 | 22.776 |
| `proj` | 15.641 | 21.516 |
| `project` | 15.987 | 21.642 |
| `g` | 23.630 | 36.509 |
| `ga` | 22.315 | 27.605 |
| `gar` | 21.693 | 29.719 |
| `gard` | 15.470 | 21.400 |
| `garden` | 15.644 | 22.374 |
| `a` | 23.672 | 31.659 |
| `t` | 23.960 | 29.833 |
| `tomato` | 16.261 | 21.582 |
| `weekly review` | 14.843 | 20.806 |
| `coffee mor` | 13.605 | 17.326 |
| `zür` | 14.909 | 21.424 |
| `café` | 15.932 | 20.127 |
| `big page` | 3.583 | 6.151 |
| `xylophone` | 0.352 | 1.261 |

### Adaptive narrowing

`Vault::search` and `search_adaptive` use this vault's document breadth, with no minimum query length. A query is broad when the estimate exceeds `max(200, live_page_count / 20)` (200 documents or 5% of pages; the constant pair is `SEARCH_BREADTH = (200, 20)`). Vaults with at most 200 live pages always use full search. A bounded FTS posting count stops at threshold + 1, without ranking or loading body text. The live-page count is cached against SQLite `total_changes` and `PRAGMA data_version`, refreshing after foreground or background writes.

Existing one-to-three-character FTS prefix indexes are reused. For longer prefixes, a connection-local [FTS5 vocabulary](https://www.sqlite.org/fts5.html#the_fts5vocab_virtual_table_module) (`instance` type) resolves at most 16 distinct indexed terms, using inclusive dictionary seeks to skip duplicate postings. An equivalent exact-term OR avoids eagerly rebuilding prefix position unions before counting documents. A temporary `unicode61` token view supplies the same case/diacritic normalization as the index. This counts the conjunction of the existing word-prefix terms, so a selective earlier term or combination enables full search. When the vocabulary cap is reached, that prefix is omitted from the breadth-only conjunction, giving a conservative upper bound; no remaining terms means broad. Trashed/template postings and capped vocabulary can therefore over-narrow. The title/full result queries still use the original prefixes and exclude trash/templates. The persistent index schema and version remain unchanged; these virtual tables are temporary and require no migration.

Broad input prioritizes titles from the in-memory title list. When titles do not fill the caller's limit, FTS matches fill the remaining rows, excluding returned title IDs. Fill orders the joined page rows by `updated_ms DESC, id ASC` before LIMIT, without bm25 or snippets. Those keys survive reindexing, unlike FTS rowids. Uncorrected narrowed rows have empty snippets and the result remains `narrowed: true`. Selective input uses full ranked body/title search with `narrowed: false`, merged behind the ordered title matches. Title IDs are excluded before body ranking and snippets, avoiding duplicate work on large pages; literal title scans safely reject titles lacking required ASCII characters. Exact titles come first in both paths. Ranked rows are materialized and limited before snippets are calculated; page metadata uses one join. `search_notes` explicitly calls `search_full`, preserving agent/MCP body recall. Ctrl+K displays “Showing title matches — keep typing to search page text”; `@` menus retain their existing presentation.

**Typo tolerance (2026-10-09).** Titles rank by exact title, title prefix, word-start prefixes, contiguous substring, then fzf-style subsequence score with at most one substitution or adjacent transposition per query word of four or more letters. Ties prefer shorter titles, then newer pages, then stable IDs. The foreground title list is patched on create, rename and trash; a separate `data_version`/`total_changes` guard refreshes it after writer commits and subtree changes. `grdn pln` finds “Garden plan”. Correction eligibility is checked per query word against the live `fts5vocab(row)` vocabulary, independently of result count: a full word needs zero exact vocabulary entries; the last word needs zero entries starting with its prefix. Existing words remain unchanged, including beside a missing word. When every word exists, neither fuzzy title scoring nor body correction runs. Each word costs one vocabulary seek; correctly typed queries do not rebuild the deletion index. Its word index is built at vault open from distinct `fts5vocab(row)` terms, then rebuilt lazily after database changes on the next correction pass. Whole words use hashed one/two-deletion forms with verified edit distances; the last word uses a pruned edit-distance traversal of the sorted vocabulary to correct word starts. Length bands allow zero edits for 1–3 letters, one for 4–7, two for 8+. At most eight candidates per word are kept, most frequent first. One OR-expanded FTS query uses native FTS5 phrase matches for each row's edit count and actual correction; on the ranked path, body rank is `bm25 + 4 × edits` (lower is better). Every uncorrected hit precedes every fuzzy hit. `search_full` and agent `search_notes` use the same fallback. Hits carry `fuzzy` and `corrected`; Ctrl+K adds one muted “Showing results for *word*” line only when all hits are fuzzy.

The existing 19-query release gate retains **p95 < 20 ms**, overall and per query, with the same 10,000-page seed and assertions. A separate 13-query typo workload asserts **p95 < 30 ms**, overall and per query, and verifies that each query returns fuzzy hits. `search_index_stats` reports retained word-index allocation capacities (word strings, character arrays and deletion pairs), excluding titles, SQLite and allocator overhead, plus vocabulary-read/build wall time. The UI perf test adds `applr`, `gadr` and `tomatoe` keystrokes for Ctrl+K and agent `@`, using the unchanged fixed debug ceilings; it does not establish profile-mode frame timings.

**Profile before corrected-query narrowing (2026-10-10).** Release build, the unchanged 10,000-page seed, limit 50, three warm-up rounds and 40 interleaved samples per query. Whole-search measurements run with hooks disabled; a separate phase records paired SQL variants. `cofee mor` has one corrected candidate (`coffee`), keeps the existing prefix `mor`, and matches 2,595 documents. `recipies` has one candidate (`recipe`), matches 4,905 documents, and returns 24 fuzzy titles plus 26 body hits. Reducing the candidate cap cannot help these two cases.

| Stage/cost, p50 / p95 ms | `cofee mor` | `recipies` |
| --- | ---: | ---: |
| Whole search, hooks off | 13.394 / 19.655 | 15.981 / 20.956 |
| Vocabulary eligibility | 0.279 / 0.387 | 0.234 / 0.352 |
| Fuzzy title scorer | 1.702 / 2.305 | 0.702 / 0.916 |
| Candidate generation | 0.021 / 0.032 | 0.054 / 0.063 |
| OR matching, count only | 0.697 / 0.947 | 0.456 / 1.132 |
| OR match + page join/filter, cumulative | 2.864 / 4.406 | 4.608 / 7.898 |
| + bm25/edit ranking and limit, cumulative | 7.833 / 12.549 | 12.746 / 15.887 |
| + final FTS join, no snippets, cumulative | 8.702 / 12.869 | 13.530 / 19.264 |
| Production SQL including snippets/correction | 9.908 / 12.834 | 13.605 / 26.182 |
| Page join/filter, paired increment | 2.186 / 3.656 | 4.134 / 7.031 |
| bm25/edit ranking + limit, paired increment | 4.654 / 9.106 | 7.915 / 10.621 |
| Final FTS join, paired increment | 1.021 / 4.360 | 0.836 / 4.942 |
| Snippets/correction, paired increment | 1.177 / 4.775 | -0.399 / 11.063 |

Both queries already use a constant edit penalty; ranking therefore spends time on bm25/sorting rather than per-hit edit decoding. The negative median snippet increment is noise from separately executed variants. Stage p95s are not additive. The final join costs about 1 ms at the median, while ranking and page filtering dominate; the measurements do not support reducing candidates or treating the final join as the primary bottleneck. This warmed profiling run does not replace the unchanged release gate: the previous gate tails were 31.021/47.066 ms. Both corrected queries exceed the existing 500-document narrowing threshold, supporting adaptive unranked body fill for these broad, uniform-edit-cost queries. Full search retains bm25 and edit ranking. Profiling instrumentation is temporary and removed before verification.

**Typo-tolerance verification: ACCEPTED (2026-10-10).** Checks ran serially with `CARGO_INCREMENTAL=0`; each check has a recorded exit code. The original quality tests and both performance workloads/ceilings are unchanged. release-gate: pass; core-tools: pass; clippy: pass; bridge: pass (`cargo build --release -p notes_bridge` exit 0; `flutter_rust_bridge_codegen generate` exit 0, generated files unchanged); flutter-analyze: pass (`flutter analyze` exit 0, no issues); flutter-typo: pass (`flutter test test/search_typo_test.dart` exit 0); flutter-perf: pass (`flutter test -t perf --concurrency=1 test/search_perf_test.dart` exit 0; highest typo-query p95 Ctrl+K 134.74 ms against the 174.624 ms ceiling, Agent @ 72.72 ms against 87.852 ms). Existing quality/performance test hashes are unchanged. CI/deploy changed concurrently outside this search work and were left untouched: .github/workflows/android-apk.yml, the private APK dispatch script.

The profile above supported narrowing broad corrected queries rather than reducing candidates: each failing query has only one candidate, while ranking processes thousands of matches. Adaptive correction directly counts the expanded MATCH postings up to threshold + 1, using the existing max(200, live pages / 20) threshold. Uniform-edit-cost broad queries use title hits followed by unranked body fill ordered by update time/id, with empty snippets, actual matched corrections and `narrowed: true`. Every uncorrected result stays first. Mixed-edit-cost corrections and `search_full`/agent `search_notes` retain bm25 plus edit penalties. Candidate cap eight, frequency ordering and vocabulary eligibility stay unchanged. Temporary profiling hooks/test are removed before the gate.

Latest 10k word index: **196 words**, **3206 deletion keys**, **66,933 estimated retained bytes** (65.36 KiB), **37.047 ms** vocabulary-read/build time. Accounting excludes titles, SQLite, allocator overhead and peak RSS. Cold/stale fuzzy-query latency remains unverified.

| Release query/class | p50 ms | p95 ms | Samples |
| --- | ---: | ---: | ---: |
| `warm search, 10,000 pages, limit 50` | 0.738 | 4.106 | 950 |
| `query "p"` | 1.585 | 2.958 | 50 |
| `query "pr"` | 0.634 | 1.443 | 50 |
| `query "pro"` | 0.413 | 0.622 | 50 |
| `query "proj"` | 0.348 | 0.908 | 50 |
| `query "project"` | 0.305 | 0.731 | 50 |
| `query "g"` | 1.547 | 3.960 | 50 |
| `query "ga"` | 0.730 | 2.174 | 50 |
| `query "gar"` | 0.476 | 0.964 | 50 |
| `query "gard"` | 0.485 | 1.046 | 50 |
| `query "garden"` | 0.364 | 0.721 | 50 |
| `query "a"` | 3.009 | 6.637 | 50 |
| `query "t"` | 2.686 | 7.282 | 50 |
| `query "tomato"` | 0.688 | 1.735 | 50 |
| `query "weekly review"` | 0.867 | 1.409 | 50 |
| `query "coffee mor"` | 3.980 | 5.958 | 50 |
| `query "zür"` | 0.431 | 0.937 | 50 |
| `query "café"` | 0.660 | 1.362 | 50 |
| `query "big page"` | 0.774 | 1.873 | 50 |
| `query "xylophone"` | 0.931 | 1.699 | 50 |
| `fuzzy search, 10,000 pages, limit 50` | 2.946 | 11.662 | 650 |
| `typo query "tomatoe"` | 2.888 | 5.285 | 50 |
| `typo query "gardne"` | 3.024 | 7.395 | 50 |
| `typo query "weekyl review"` | 3.383 | 7.254 | 50 |
| `typo query "cofee"` | 3.257 | 6.185 | 50 |
| `typo query "cofee mor"` | 8.038 | 17.508 | 50 |
| `typo query "projcet"` | 1.778 | 5.154 | 50 |
| `typo query "recipies"` | 7.873 | 21.805 | 50 |
| `typo query "gadr"` | 4.557 | 10.725 | 50 |
| `typo query "meeitng"` | 2.152 | 6.636 | 50 |
| `typo query "buget"` | 1.928 | 4.559 | 50 |
| `typo query "desgin"` | 2.596 | 7.329 | 50 |
| `typo query "journla"` | 1.422 | 6.162 | 50 |
| `typo query "databsae"` | 1.661 | 6.965 | 50 |

Flutter debug ceilings remain Ctrl+K 174.624 ms and agent @ 87.852 ms. These widget-test timings do not establish the 20 ms profile-mode UI target.

This search pass made no changes to `.github/workflows/android-apk.yml` or the private APK dispatch script; external concurrent work advanced master to `7dd3477`. No commits or goldens were made by this pass. The fixture lock fix from the previous pass remains: seed includes root colors in its initial snapshots/indexing transaction, eliminating competing foreground color-backfill writes and queued background flushes. Per-run fixture directories remain unique; vocabulary rebuild is read-only.

The reproducible stage/recall diagnostic is separate from the unchanged gate. Since title tiers intentionally change top-result order, its current recall check verifies that full search retains the baseline body hits rather than requiring the old order:

```powershell
$env:CARGO_INCREMENTAL = '0'
cargo test --release -p vault_core --test search_diagnostics -- --ignored --nocapture
```

**Original SQL profile, 10,000 pages, 50 samples per stage:** stages are cumulative operations, not independent component timings. FTS count p50/p95 was 0.985/1.345 ms for `g` and 0.808/2.168 ms for `garden`; adding bm25 and LIMIT gave 12.085/15.533 and 6.951/11.246 ms; adding page metadata/filtering/recency gave 18.954/26.032 and 10.787/14.119 ms; adding snippets and DTO decoding gave 22.096/46.942 and 14.311/18.087 ms. JSON serialization of the 50 DTOs was 0.006/0.015 and 0.012/0.033 ms. Ranking dominates, page joins add roughly 4–7 ms at the median, and snippets add roughly 3 ms. The existing search already used one page join, not per-result page lookups.

**Final diagnostic:** all 19 breadth estimates passed p95 < 2 ms; the worst was `weekly review` at **0.772 ms** (including live-count cache validation, token normalization, vocabulary seeks and bounded document counting). `g` measured p50/p95 **0.142/0.284 ms**, and `garden` **0.225/0.653 ms**. Adaptive searches measured **1.937/5.308 ms** and **0.981/2.731 ms**, respectively. The original SQL and optimized full API returned identical IDs/order for `weekly review`, `coffee mor`, `xylophone`, `big page`, `project`, `garden`, and `tomato` (six longest queries, plus both queries tied at six characters).

**Earlier blocker (2026-10-09):** the title-only implementation failed the unchanged release gate during warm-up at `corpus must match "coffee mor"`, before collecting timings. This query matches 2,595 live non-template pages across title/body but zero titles on the retained corpus, above the 500-page threshold. Work stopped under the requested decision-conflict rule. The owner then approved filling narrowed results with unranked FTS matches, keeping the non-empty assertion and 20 ms thresholds unchanged. The immediate pre-change gate measured p50/p95 16.774/31.416 ms (profiling compilation overlapped that baseline).

**Body-fill gate, first verification:** overall p50/p95 **1.752/9.556 ms**, 950 calls. Every warm-up non-empty check passed, including `coffee mor`. The gate still failed because `t` measured p95 **23.028 ms**; the other 18 queries stayed below 20 ms. The corpus, non-empty requirement and timing thresholds were unchanged. `CARGO_INCREMENTAL=0`; builds and checks ran serially.

| Query | p50 (ms) | p95 (ms) |
| --- | --- | --- |
| `p` | 4.387 | 15.251 |
| `pr` | 1.204 | 2.654 |
| `pro` | 1.041 | 3.198 |
| `proj` | 1.117 | 7.114 |
| `project` | 1.085 | 6.134 |
| `g` | 2.432 | 5.922 |
| `ga` | 1.821 | 11.391 |
| `gar` | 1.598 | 3.750 |
| `gard` | 1.144 | 6.553 |
| `garden` | 1.046 | 1.901 |
| `a` | 4.672 | 16.839 |
| `t` | 5.107 | 23.028 |
| `tomato` | 1.284 | 2.446 |
| `weekly review` | 4.049 | 19.888 |
| `coffee mor` | 2.473 | 9.516 |
| `zür` | 0.994 | 3.243 |
| `café` | 1.186 | 6.085 |
| `big page` | 3.733 | 10.435 |
| `xylophone` | 0.532 | 8.199 |

**Earlier body-fill integration checks:** all seven focused core search tests and strict core clippy passed; the lead independently reran those seven tests successfully. The earlier full core/tools suite passed, but the post-fill full suite failed `seed_and_reindex_rebuild_the_same_results` at `tests/api.rs:799`: narrowed fill IDs/order differed after rebuilding SQLite row IDs. The first difference was in empty-snippet fill rows; metadata and preceding title hits matched. The assertion expects stable search results across reindexing, while the first unranked FTS index-order implementation could change after reindexing. This remained unresolved at the 30-minute deadline; the assertion was not weakened. The release bridge build passed; Flutter analysis found no issues. The existing Ctrl+K/Ctrl+P navigation widget test passed alone, and `search_narrowing_test.dart` passed alone against the rebuilt bridge. The latter verifies the muted hint for broad queries, empty narrowed snippets, and clearing the hint after selective or empty input. No goldens were changed. Builds/tests ran one at a time with `CARGO_INCREMENTAL=0`.

**Independent repeat gate before stable fill: PASS**, without code, corpus, assertion or threshold changes. A fresh 10,000-page corpus and 950 interleaved calls measured overall p50/p95 **1.172/3.926 ms**. All 19 queries passed individually; the highest p95 was `a` at **4.929 ms**. `t` measured **3.636/4.912 ms**. The earlier `t` failure remains recorded above rather than discarded: wall-time tails varied between runs, so this pass is measured acceptance, not a guarantee under arbitrary host load. The repeat took 129 seconds to rebuild and 103.39 seconds to run.

| Query | Repeat p50 (ms) | Repeat p95 (ms) |
| --- | --- | --- |
| `p` | 3.224 | 4.901 |
| `pr` | 0.942 | 1.458 |
| `pro` | 0.843 | 1.538 |
| `proj` | 0.952 | 1.222 |
| `project` | 0.885 | 1.682 |
| `g` | 1.939 | 2.835 |
| `ga` | 1.295 | 2.015 |
| `gar` | 1.199 | 2.052 |
| `gard` | 0.850 | 1.464 |
| `garden` | 0.916 | 1.286 |
| `a` | 3.395 | 4.929 |
| `t` | 3.636 | 4.912 |
| `tomato` | 1.074 | 1.805 |
| `weekly review` | 3.006 | 4.074 |
| `coffee mor` | 1.938 | 2.622 |
| `zür` | 0.777 | 1.148 |
| `café` | 0.840 | 1.680 |
| `big page` | 2.813 | 3.891 |
| `xylophone` | 0.374 | 0.691 |

**Body-fill UI performance:** `search_perf_test.dart` passed alone at its unchanged fixed debug ceilings. Across 100 keystrokes per route, Ctrl+K real p50/p95 was **37.37/84.78 ms**, with immediate control **41.28/102.50 ms**; agent `@` real was **28.00/44.39 ms**, with control **22.05/42.43 ms**. Neither real route met 20 ms in this debug harness. The profile-mode target, including rasterisation, remains unverified. The fixture generated 10,000 pages in 192,679 ms; the test completed in approximately 238 seconds.

| Prefix | Ctrl+K real p50/p95 | Ctrl+K control p50/p95 | Agent `@` real p50/p95 | Agent `@` control p50/p95 |
| --- | --- | --- | --- | --- |
| `a` | 69.17/109.60 | 84.03/111.62 | 30.88/39.91 | 32.28/48.51 |
| `ap` | 55.85/69.41 | 54.70/82.94 | 31.36/44.39 | 29.00/42.43 |
| `app` | 30.43/60.23 | 26.89/94.51 | 22.87/40.89 | 16.46/28.36 |
| `appl` | 29.61/53.31 | 31.52/50.03 | 22.00/40.66 | 16.95/37.29 |
| `apple` | 27.38/39.60 | 30.29/53.74 | 25.24/48.70 | 14.65/33.64 |

**Stable-fill follow-up: PASS.** Fill uses `ORDER BY p.updated_ms DESC, p.id` before LIMIT, retaining title priority, deduplication and empty snippets. The unchanged `seed_and_reindex_rebuild_the_same_results` assertion passes, including both explicit reindex and deleted-index rebuilding on open; the lead independently reran it after the final whitespace-only rustfmt pass. All eight focused search tests pass, including a new newest-first/equal-timestamp ID-tie test. The full `cargo test -p vault_core -p vault_tools` suite passes and core all-target clippy with `-D warnings` is clean. The gate and full suite ran before the final formatting-only pass; focused search tests and the lead's reindex check ran after it.

The stable-fill release gate passed overall and every query, using the unchanged corpus, non-empty assertion and strict 20 ms thresholds. Overall p50/p95 was **1.600/7.008 ms** over 950 interleaved calls; the largest query p95 was `p` at **13.201 ms**. Sorting the fill added cost for `coffee mor` but it still measured **6.101/10.017 ms**. These are measured wall times on this host; previous runs demonstrate host/run variability. Checks ran serially with `CARGO_INCREMENTAL=0`. The local release bridge was refreshed successfully (3m 53s), and the unchanged narrowing hint widget test passed alone against the bridge. The rebuilt `target/release/notes_bridge.dll` was written at 22:54:51 local time on 2026-10-09. No schema migration, FRB regeneration, goldens or commits were needed for the stable-order fix. The profile-mode UI target remains unverified.

| Query | Stable-fill p50 (ms) | Stable-fill p95 (ms) |
| --- | --- | --- |
| `p` | 3.908 | 13.201 |
| `pr` | 1.081 | 4.083 |
| `pro` | 0.979 | 1.523 |
| `proj` | 1.064 | 1.870 |
| `project` | 1.037 | 1.806 |
| `g` | 2.318 | 4.498 |
| `ga` | 1.688 | 3.134 |
| `gar` | 1.660 | 3.374 |
| `gard` | 1.163 | 4.141 |
| `garden` | 1.027 | 2.622 |
| `a` | 4.631 | 11.123 |
| `t` | 4.942 | 12.655 |
| `tomato` | 1.332 | 6.554 |
| `weekly review` | 3.795 | 6.784 |
| `coffee mor` | 6.101 | 10.017 |
| `zür` | 1.135 | 3.099 |
| `café` | 1.159 | 1.625 |
| `big page` | 3.372 | 9.771 |
| `xylophone` | 0.504 | 1.775 |

The full API intentionally does not narrow broad queries. In this diagnostic `g` remained above 20 ms: original SQL p50/p95 **22.612/32.051 ms**, optimized full API **24.961/35.522 ms**. For `garden`, original SQL was **15.128/23.542 ms**, and the optimized full API **14.438/22.839 ms**. Broad API search preserves recall rather than taking the typing fast path. DTO copying and JSON serialization were approximately 0.01 ms; this is not a measurement of bridge transport or GPU rendering.

**UI calibration result, desktop debug widget harness, 2026-10-09:** each real/control route measured 100 keystrokes (20 per prefix). Both real routes missed 20 ms. Ctrl+K's immediate-search control also missed it. The agent `@` control initially measured p95 16.88 ms, then 28.40 ms on the verification run (real p95 29.284 ms). That unstable debug baseline prevents this harness from reliably checking 20 ms for either route. Tests use **fixed debug ceilings of 174.624 ms for Ctrl+K** (3 × calibrated real p95 58.208 ms) and **87.852 ms for agent `@`** (3 × calibrated real p95 29.284 ms); neither is recalculated from later runs. These debug timings did not establish 20 ms; the later [Linux profile check](#linux-profile-search-measurement-2026-10-13) measures the frame and raster path.

| Route | Real p50 / p95 (ms) | Immediate control p50 / p95 (ms) | Debug gate |
| --- | --- | --- | --- |
| Ctrl+K | 30.60 / 58.21 | 20.40 / 53.32 | p95 < 174.624 ms |
| Agent `@` (first calibration) | 16.99 / 27.72 | 9.72 / 16.88 | subsequently calibrated to p95 < 87.852 ms |

**Earlier UI verification, before adaptive narrowing:** both fixed debug gates passed. Ctrl+K measured p50/p95 **26.44/45.55 ms**, with immediate control **23.35/52.23 ms**; agent `@` measured **14.44/25.75 ms**, with immediate control **9.01/17.48 ms**. Each route again measured 100 keystrokes, 20 per prefix. Both real p95 values exceeded 20 ms; the profile-mode target remains unverified. The per-keystroke results below are from that earlier run:

| Typed prefix | Ctrl+K real p50 / p95 (ms) | Ctrl+K control p50 / p95 (ms) | Agent `@` real p50 / p95 (ms) | Agent `@` control p50 / p95 (ms) |
| --- | --- | --- | --- | --- |
| `a` | 36.98 / 46.09 | 34.09 / 67.50 | 21.38 / 27.23 | 13.47 / 19.93 |
| `ap` | 34.05 / 47.83 | 35.16 / 53.15 | 15.84 / 24.92 | 12.11 / 19.03 |
| `app` | 23.39 / 28.57 | 15.71 / 25.71 | 11.73 / 19.06 | 7.49 / 9.20 |
| `appl` | 22.19 / 26.56 | 16.95 / 25.77 | 12.60 / 19.06 | 7.34 / 12.87 |
| `apple` | 22.37 / 31.06 | 16.50 / 26.08 | 13.60 / 17.49 | 6.44 / 14.01 |

## Windows profile search measurement (2026-10-13)

**Superseded by the accepted [Linux profile measurement](#linux-profile-search-measurement-2026-10-13).** The historical Windows attempt below collected no native profile timings.

The test-only [profile harness](../../app/integration_test/search_profile_test.dart) and [driver](../../app/test_driver/search_profile_driver.dart) are implemented. **Native execution was blocked by the selected Windows toolchain's ATL libraries.** No profile latency samples were collected, and the roadmap check remained open at that point.

The worktree release bridge build passed (`cargo build --release -p notes_bridge`, 8m 22s, four Cargo jobs). `target/release/notes_bridge.dll` is 31,755,776 bytes, SHA-256 `36f6efc673872d034ea6bbb8c2455c5069e195203f3b411c4f65e3acac8fea0b`. Focused `flutter analyze integration_test test_driver` is clean, and the root `dart scripts/check-addon-imports.dart` check passes. Native profile execution remains unverified.

`flutter build windows --profile --target=integration_test/search_profile_test.dart` exited 1 after 903.8 seconds: `LINK : fatal error LNK1104: ... atls.lib` in `flutter_local_notifications_windows.vcxproj`. The generated CMake cache selects `C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools`, x64, Visual Studio 17 2022. Read-only filesystem checks confirm that its MSVC `14.44.35207/atlmfc/lib/x64/atls.lib` is absent, while `atlmfc/lib/spectre/x64/atls.lib` exists. The ordinary x64 library exists in the separate VS 2022 Community installation. This identifies an installation/library-selection blocker, rather than evidence that Flutter generally lacks Windows profile support. The task stopped under its native-profile failure rule; no toolchain installation or reconfiguration was performed.

The native app never started. The three planned `flutter drive` timing runs were not executed, no driver response data was produced, and there are no during-profile load samples. These rows are missing measurements, not successful runs or estimates:

| Run | Ctrl+K p50 / p95 (ms) | Agent `@` p50 / p95 (ms) | Status |
| --- | --- | --- | --- |
| Historical debug reference | 37.37 / 84.78 | 28.00 / 44.39 | Earlier body-fill measurement |
| Profile 1 | — | — | Not run: native link blocker |
| Profile 2 | — | — | Not run: native link blocker |
| Profile 3 | — | — | Not run: native link blocker |

The unchanged `flutter test --concurrency=1 test/search_perf_test.dart -t perf` check ran against the rebuilt release bridge (expanded reporter added for the log), but exited 1 after approximately 3m 02s at the first Ctrl+K typo gate (`applr`): real p50/p95 **38.19/987.33 ms**, immediate control **34.86/1644.97 ms**, 100 samples each, versus the unchanged p95 ceiling **174.624 ms**. Fixture generation took 51,411 ms. The test reached and measured the ordinary Ctrl+K route but failed before printing its aggregate and before the agent route. Therefore no fresh two-route debug comparison is available. The immediate control also exhibited large stalls; this does not establish an app-search regression or isolate the cause. The source and thresholds were not changed. The local log is `app/build/search-profile-debug-check.log` (ignored build artifact).

The corpus matches `test/search_perf_test.dart`: `NotesVault.seed` with 9,999 ordinary pages and zero big-page blocks, producing exactly 10,000 pages. Each route types `a`, `ap`, `app`, `appl`, `apple`, with one warm-up round followed by 20 measured rounds (100 samples per route). Ctrl+K uses the registered shortcut; agent `@` uses `AgentComposer`. An explicit `NOTES_BRIDGE_LIB` loads the worktree's release DLL, and every run creates and removes its own temporary vault. No user vault is opened.

The stopwatch starts immediately before synthetic `enterText` and stops after the current bridge result and one results frame. The live binding uses `onlyPumps`; bridge completion is awaited on the real event loop before pumping that frame. This preserves the debug test's endpoint without adding a vsync wait on every bridge poll. The frame waits for display vsync; refresh rate is recorded. GPU completion, physical keyboard input and display presentation are not measured. Fixed clocks and immediate debounces match the debug fixture. The 20 ms reference is printed without a timing assertion. Raw samples, per-prefix statistics and route p50/p95 are returned as driver response data and saved to `app/build/search_perf/search_perf.json`.

PowerShell, starting at the worktree root:

```powershell
cargo build --release -p notes_bridge
$env:NOTES_BRIDGE_LIB = (Resolve-Path ./target/release/notes_bridge.dll).Path
Set-Location app
flutter pub get
flutter drive --profile -d windows --driver=test_driver/search_profile_driver.dart --target=integration_test/search_profile_test.dart
```

Before each timing run, inspect foreign Cargo/rustc/Flutter/MSBuild activity and wait for 30 quiet seconds. Reusable MSBuild nodes count as idle only while CPU time remains unchanged and they have no build subprocesses. Preserve the response JSON after each run; the driver otherwise overwrites it. Keep the test window visible, and close it after the driver finishes. This harness is an opt-in measurement, outside CI and the fast test tier.

Machine: Windows 11 Home 10.0.26200, AMD Ryzen 7 5800X (8 cores / 16 logical processors), approximately 32 GB RAM, NVIDIA GeForce GTX 1660 SUPER (also a Parsec virtual display adapter); Flutter 3.47.5, Dart 3.13.4. Windows CPU counters read 100% in the preflight and after the debug check. Foreign Cargo/rustc/Flutter/MSBuild jobs were observed throughout preparation. These are spot samples, not a continuous load record for the debug test. No timing quiet window or native-profile measurement is claimed. No user vault, app source, appearance, existing test thresholds, CI configuration or fast test tier was changed.

## Linux profile search measurement (2026-10-13)

**Verdict: same hotspots at the agreed bridge-versus-Flutter split. The profile-mode check is complete.** Xvfb software rendering on a loaded 4-core host does not measure UI frame time meaningfully, so these UI timings are a relative baseline only; the absolute 20 ms UI target is judged later on real hardware. Five sequential Linux desktop profile runs exited 0. Two extra runs were collected because the spread was wide. Each run used a fresh generated 10,000-page vault, one warm-up and 20 measured rounds of `apple` prefixes, 100 keystrokes per route. No timing assertion, CI or fast-tier change was added.

All new numbers use **Flutter 3.47.7 / Dart 3.13.5**, the accepted difference from the repo/reference SDK 3.47.5. Machine for every row: the Linux machine, Debian 13, Intel Core i5-1135G7 @ 2.40 GHz, four available CPUs, 8 GiB RAM; Xvfb 1600×1000×24 with forced Mesa software rendering and `LP_NUM_THREADS=2`. An EGL probe under the same Xvfb/environment reports **llvmpipe (LLVM 19.1.7, 256 bits), Mesa 25.0.7-2+deb13u1**. The native app surface is 1280×720 at DPR 1; the binding preserves the 1440×900 logical fixture. The display API reports refresh rate 0 (unknown). Software raster timings do not establish GPU or physical display performance.

Milliseconds below are p50/p95 except the explicitly labelled contributor medians. Load is the 1/5/15-minute average immediately before → after each route, not continuous CPU utilisation. Other lanes share the host; no quiet window or artificial load was used.

| Run / Flutter | Route | Input → pump | Search bridge | Outside all bridge calls | Wait / build / raster p50 | Input → raster finish | Load before → after |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 / 3.47.7 | Ctrl+K | 119.77/221.00 | 7.21/33.87 | 110.84/201.54 | 99.87 / 5.13 / 120.52 | 318.38/389.87 | 9.89/12.30/11.33 → 10.69/12.29/11.36 |
| 1 / 3.47.7 | Agent `@` | 69.44/113.43 | 7.79/46.46 | 58.02/105.78 | 52.69 / 1.91 / 64.69 | 159.09/234.32 | 10.39/12.20/11.33 → 10.50/12.11/11.32 |
| 2 / 3.47.7 | Ctrl+K | 74.39/184.25 | 6.64/101.40 | 54.02/169.88 | 50.23 / 2.87 / 87.92 | 204.09/307.79 | 4.04/9.14/10.33 → 4.70/8.89/10.21 |
| 2 / 3.47.7 | Agent `@` | 56.41/123.40 | 7.27/58.68 | 40.15/100.57 | 37.59 / 1.44 / 51.55 | 131.66/180.12 | 4.70/8.89/10.21 → 5.56/8.81/10.16 |
| 3 / 3.47.7 | Ctrl+K | 121.81/246.19 | 7.94/117.47 | 105.65/223.96 | 95.51 / 4.77 / 125.03 | 316.01/408.38 | 7.32/8.90/10.13 → 8.68/9.06/10.14 |
| 3 / 3.47.7 | Agent `@` | 74.50/233.09 | 6.93/54.40 | 62.02/145.22 | 58.10 / 2.39 / 75.88 | 192.63/420.45 | 8.68/9.06/10.14 → 10.22/9.40/10.23 |
| 4 / 3.47.7 | Ctrl+K | 128.32/282.60 | 7.47/112.08 | 108.30/251.88 | 97.70 / 5.29 / 131.86 | 330.30/553.72 | 11.06/10.15/10.39 → 11.15/10.30/10.43 |
| 4 / 3.47.7 | Agent `@` | 74.26/144.15 | 5.96/41.49 | 66.12/131.77 | 59.15 / 2.44 / 70.34 | 179.80/236.16 | 11.15/10.30/10.43 → 10.81/10.28/10.42 |
| 5 / 3.47.7 | Ctrl+K | 114.41/218.85 | 7.51/108.32 | 97.42/201.13 | 88.09 / 4.59 / 113.01 | 287.62/355.37 | 12.02/10.63/10.53 → 11.45/10.62/10.54 |
| 5 / 3.47.7 | Agent `@` | 62.03/185.21 | 5.79/89.41 | 52.28/155.36 | 48.69 / 2.00 / 60.58 | 157.18/304.61 | 11.45/10.62/10.54 → 10.86/10.54/10.51 |

Across-run medians and min–max spread (five run percentiles, not pooled samples):

| Route / endpoint | Flutter | Median p50 [min–max] ms | Median p95 [min–max] ms |
| --- | --- | --- | --- |
| Ctrl+K / pump | 3.47.7 | 119.77 [74.39–128.32] | 221.00 [184.25–282.60] |
| Ctrl+K / raster finish | 3.47.7 | 316.01 [204.09–330.30] | 389.87 [307.79–553.72] |
| Agent `@` / pump | 3.47.7 | 69.44 [56.41–74.50] | 144.15 [113.43–233.09] |
| Agent `@` / raster finish | 3.47.7 | 159.09 [131.66–192.63] | 236.16 [180.12–420.45] |

**Top contributors, medians across runs, Flutter 3.47.7:** Ctrl+K software raster p50/p95 **120.52/152.29 ms**, search-completion → build-start wait **95.51/196.00 ms**, and build/layout **4.77/13.39 ms**; agent `@` respectively **64.69/94.96**, **52.69/120.91**, and **2.00/5.95 ms**. The derived build-finish → raster-start queue gap is **79.59/114.06 ms** for Ctrl+K and **36.02/64.03 ms** for `@`. Frame waiting includes event-loop/vsync scheduling; software rendering/backpressure is the dominant measured path. The main search bridge median p50 is **7.47/6.93 ms** for Ctrl+K/`@`, versus **105.65/58.02 ms** with no captured bridge call pending inside the pump window. This matches the Windows debug/control evidence's broad Flutter/frame-dominant split. Absolute times and the large software raster cost are not a Windows GPU comparison; the prior Windows profile attempt is superseded under the central acceptance decision.

The bridge stage is the awaited FRB round trip, including Rust execution and delivery of its Dart continuation, not isolated Rust CPU time. Its across-run median p95 is **108.32/54.40 ms** (Ctrl+K/`@`, Flutter 3.47.7); the long tails cannot be assigned to Rust alone. `outsideBridge` subtracts the clipped union of search, parser and metadata call intervals, so overlapping calls count once. Calls can overlap Flutter work. Build/raster come from the one matched engine frame, and raster completion is reported separately from the original pump endpoint. Component percentiles are not additive. The queue gap is derived per sample from these matched endpoints/durations.

The release bridge was built through `<linux-build-helper> --with /v/projects/BubblePaw-base-agent /v/worktrees/notes-app/lane-search-perf-profile -- cargo build --release -p notes_bridge -j 2` (4m 25s). In the helper's remote shell, from the worktree, run each measurement with a different `SEARCH_PROFILE_RUN`:

```sh
export PATH=/home/dev/flutter/bin:$PATH
export CARGO_BUILD_JOBS=2 CARGOKIT_CARGO_TARGET_DIR="$CARGO_TARGET_DIR"
export NOTES_BRIDGE_LIB="$CARGO_TARGET_DIR/release/libnotes_bridge.so"
cd app
SEARCH_PROFILE_RUN=1 SEARCH_PROFILE_SDK=3.47.7 \
LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=2 LIBGL_DEBUG=verbose \
xvfb-run -a -s '-screen 0 1600x1000x24' \
  flutter drive --profile -d linux \
  --driver=test_driver/search_profile_driver.dart \
  --target=integration_test/search_profile_test.dart
```

**Checks:** all five commands exited 0; all correctness checks passed; 1,000/1,000 samples matched exactly one frame with nonzero raster duration, and raw stage partitions are consistent. Focused `flutter analyze --no-pub integration_test test_driver` is clean; final `dart format` made no changes. `xvfb-run` cleaned its display after each command, and final checks found no app or Xvfb process. The harmless Linux native integration-test plugin warning did not prevent the driver receiving full passed response data.

Release library: `/home/dev/work/.target/lane-search-perf-profile/release/libnotes_bridge.so`, SHA-256 `f9f978c4be1e61adb82449ea2e1fd5865f2d093bb79a8c5a105222f8cb038319`. Raw samples/logs are retained on the Linux machine at `app/build/search_perf/run-{1..5}.{json,log}`, alongside `summary.json` and `renderer.txt`. Initial missing packages were installed by the central chat. The profile-key preflight failure and a pass with discarded zero-raster frames are preserved separately and excluded from every table. The harness now supplies physical shortcut keys and uses `binding.setSurfaceSize` rather than faking native physical size. No `app/lib`, crates, bindings, application behavior, appearance, dependencies, CI or test thresholds changed; no builds ran on the PC.

## Measured (2026-09-30)

Windows 11 desktop, release build, `cargo run -p vault_core --release --example bench -- <vault>`, fresh vaults, three runs each; the ranges cover the runs. Another process kept the machine at about 70% CPU during these runs, so all numbers are inflated and noisy. To compare with the first core, the old bench (commit `9c7c506`) and the new one (`--pages-only`) ran in turns on two 10,001-page vaults.

| Operation | Target | First core | Now, 10,001 pages | Now, with Tasks (20,002 pages) |
| --- | --- | --- | --- | --- |
| `open` | < 200 ms | 7.6 to 7.9 ms | 7.9 to 9 ms | 9 to 11.4 ms |
| `list_pages`, 100 rows | < 5 ms | p50 0.24 ms, p95 0.43 to 0.56 ms | p50 0.42 to 0.45 ms, p95 0.74 to 0.83 ms | p50 0.9 to 1.2 ms, p95 1.9 to 2.7 ms |
| `search`, 19 queries | < 20 ms | p50 8.6 to 10 ms, p95 13 to 16 ms | p50 7.2 to 7.6 ms, p95 11.5 to 14 ms | p50 9.3 to 11 ms, p95 18 to 20 ms |
| `load_page`, 2,000 blocks, cold | < 30 ms | 8.3 to 9.3 ms | 8.6 to 10 ms | 9 to 11.3 ms |
| `apply`, one `SetText` | p95 < 2 ms | p95 0.04 ms | p95 0.04 ms | p95 0.04 to 0.06 ms |

The Tasks collection (10,000 rows), 50 rows per query, 30 calls each:

| Operation | Target | p50 | p95 |
| --- | --- | --- | --- |
| `query`, no filter, sibling order | < 20 ms | 1.3 to 2.2 ms | 2.8 to 3.8 ms |
| `query`, no filter, by title | < 20 ms | 3.6 to 14 ms | 6 to 42 ms |
| `query`, open view (status not done, due set; 5,998 rows), by due | < 20 ms | 9.6 to 22 ms | 12 to 30 ms |
| same, at offset 3,000 | < 20 ms | 10 to 16 ms | 13 to 35 ms |
| `query`, or-tree over tags, estimate, urgent and title (2,766 rows), by estimate then title | < 20 ms | 12.7 to 19.5 ms | 15 to 52 ms |
| `query_groups` by status, tags or urgent | | 8.5 to 22 ms | 12 to 56 ms |
| `query_range`, 42-day calendar grid (2,123 entries) | | 22 to 26 ms | 30 to 37 ms |
| `query_range`, 180-day timeline (9,008 entries) | | 86 to 93 ms | 98 to 115 ms |
| `backlinks`, 499 linking pages | | 4.6 to 5.5 ms | 7 to 8.2 ms |

- In the quietest run every filtered query stayed under 16 ms at p95; in the busiest the tails reached 30 to 52 ms. The target is met on an idle machine but has little headroom. Most of the cost is one props lookup per condition per row and per property sort key.
- `list_pages` is slower than in the first core because summaries now carry properties and the trash filter. It is still well within target.
- `query_range` returns full summaries (with properties) for every entry, so a 180-day range over 10,000 rows takes about 90 ms. Interfaces should request the visible window.
- Seeding 10,000 + 2,000 took 19 to 22 s; seeding the Tasks collection of 10,000 rows took 16 to 21 s (Windows Defender scanning is part of it). The vault with Tasks uses about 177 MB.
- Flushing after 1,000 keystrokes on the big page took 40 to 60 ms, off the caller's path.
- The seed's small vocabulary makes every word appear on thousands of pages, so search is a worst case. With Tasks, twice as many pages match.

## Build

- Dependencies: `loro` 1.16, `rusqlite` 0.40 (`bundled`), `serde`, `serde_json` (already a dependency of `loro`). No async runtime, no new dependencies for the MVP features.
- `cargo test -p vault_core` runs 12 unit tests and 45 integration tests (10 in `tests/api.rs`, 4 in `tests/blocks.rs`, 20 in `tests/mvp.rs`, 3 in `tests/prefs.rs`, 8 in `tests/sync.rs`; 2026-10-01). `cargo clippy -p vault_core --all-targets` is clean.
- Android: `cargo check -p vault_core --target x86_64-linux-android` and `--target aarch64-linux-android` pass with NDK 27.3. The bundled SQLite needs the NDK's clang. Bash environment used:

```sh
B="$LOCALAPPDATA/Android/Sdk/ndk/27.3.13750724/toolchains/llvm/prebuilt/windows-x86_64/bin"
export CC_x86_64_linux_android="$B/x86_64-linux-android24-clang.cmd"
export AR_x86_64_linux_android="$B/llvm-ar.exe"
export CC_aarch64_linux_android="$B/aarch64-linux-android24-clang.cmd"
export AR_aarch64_linux_android="$B/llvm-ar.exe"
```

- Linux was not checked (no Linux target installed on this machine).
- `target/` was 1.4 GB after the tests, clippy, release bench and both Android checks.

## Known limits

- Multiple writable `Vault` instances merge edits when flushing. Appends briefly take the same OS lock after the edit and reopen cached log handles when file identity changes; the second instance uses a random Loro peer, so its version vectors grow per session.
- Paths longer than 260 characters fail on Windows (SQLite cannot open the index).
- Outside edits to `.md` files are read only by `rescan` (mounted folders: `rescan_mounts`, polled by the bridge). The writer does not overwrite a file whose hash changed; it waits for a rescan.
- Computed values are refreshed when the row or a related row is written. A formula using `now()` is not re-evaluated as time passes, and a change more than 4 relation levels away is not propagated until the row is written or the index is rebuilt.
- Search and backlinks see block edits only after the writer flush (up to 2 s).
- Update logs are not fsynced.
- The change feed is in memory: its sequence restarts at every `open`, and a caller with an older `seq` gets `reset`.
- Page history retains every version. V2 retention trims resolved log records and consumed undo metadata after 30 days; pending records and historical page versions remain.
- Markdown rendering is readable but not exact CommonMark in edge cases.

## Deviations from the proposals

Written into core-api (internal wiki) and core-mvp (internal wiki):

- Owned parameters (`String`, `Option<PageId>`) and `Result` on every fallible call, for flutter_rust_bridge and Tauri.
- `page_count` returns `u32` (a `u64` becomes a `BigInt` in Dart).
- Added `flush()`, `reindex() -> u32`, `root()`, `new_id()` and `seed::BIG_PAGE_TITLE`.
- Added `.vault/logs/` and the block cache file to the layout.
- Block edits update the index through the background writer, not on every change synchronously.
- MVP: `delete_page` moves to the trash (as proposed), which changes the first API's behaviour.
- MVP: `query_range` also takes an optional filter. `poll_changes` returns a `ChangeBatch { seq, changes, reset }` instead of a tuple. `Version` also carries a change count. `import_markdown` takes a parent and returns an `ImportReport`.
- MVP additions beyond the proposal: `list_children` (sibling order), `get_schema`, view editing (`list_views`, `add_view`, `update_view`, `delete_view`, `move_view`), `default_template`, `attachment_info`, `device_id`, `BlockOp::SetTarget` and `Block::target` (the proposal's `Bookmark(url)` is a `Bookmark` block whose target is the URL), `seed::generate_tasks` and `seed::TASKS_TITLE`.

Budget stop (40-minute continuation): release gates, full core/tools tests and strict core clippy passed. The owned serial checker was stopped during the release bridge build to finish reporting within the budget; Flutter analyze and the two individual Flutter test files were not reached. No fresh UI performance numbers are available for this pass. Shared Cargo package-cache waits and concurrent builds consumed verification time.
