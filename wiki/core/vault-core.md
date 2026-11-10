---
type: topic
layer: state
summary: "The Rust core (vault_core): a Vault of Loro page documents with a SQLite index, a background writer, markdown files, trash, history, import and the sync hooks."
parent: "[[core/index]]"
status: current
covers: ["crates/core/**"]
---

# Vault core

`vault_core` (`crates/core`) is the Rust library that owns a vault on one device. Each page is a Loro CRDT document; a SQLite index, markdown files and a change feed are derived from those documents. Every public function is a method of `Vault`, which is `Send + Sync` and is shared behind an `Arc`. The Flutter app, the MCP server and the sync crate all go through it ([App state](../app/app-state.md), [Sync](../sync/vault-sync.md)).

## Modules

All sources are in `crates/core/src`. The public methods are in the `impl Vault` blocks of these files:

| File | Holds |
|---|---|
| `vault.rs` | `Vault`: open, pages, blocks, flush, reindex, the page cache, update logs, the background writer |
| `tree.rs` | sibling order, moves, duplicates, trash, favorites, icon, cover, display options |
| `collections.rs`, `query.rs` | properties, schemas, views, default templates; queries, board groups and date ranges compiled to SQL |
| `types.rs`, `type_data.rs` | page types and the add-on `type_data` maps ([Data model](data-model.md)) |
| `view_blocks.rs` | `view` blocks and the migration from saved views |
| `formula.rs`, `computed.rs` | the formula language; formula and rollup values computed into the index |
| `templates.rs` | templates for new children of any page |
| `agenda.rs`, `calendars.rs` | agenda rows, task completion, progress, Journal and daily notes, space colours and roles |
| `mounts.rs` | folders of this device mounted as pages ([Folder mounts](mounts.md)) |
| `media.rs`, `attach.rs`, `history.rs`, `feed.rs` | backlinks, attachments, page history, the change feed |
| `import.rs`, `mdparse.rs`, `markdown.rs`, `mdio.rs`, `blockdiff.rs` | markdown import and rescan, rendering with frontmatter, the markdown API used by MCP, turning one block tree into another |
| `doc.rs`, `index.rs`, `migrate.rs`, `cold.rs` | one page's Loro document, the SQLite index and its migrations, cold history storage |
| `search_*.rs`, `fts_rank.rs`, `fuzzy.rs`, `live_query.rs` | search and live queries ([Search queries](search-queries.md)) |
| `sync.rs`, `conflict_*.rs` | the sync hooks and the scoped-conflict backend ([Sync](../sync/vault-sync.md), [Conflict format](conflict-format.md)) |
| `triage.rs` | merge proposals ([Triage](triage.md)) |
| `feedback.rs` | the hidden Feedback page |
| `seed.rs` | `seed::generate` and `seed::generate_tasks`, deterministic test vaults |

`examples/bench.rs` times the main operations. Integration tests are in `crates/core/tests`.

## API conventions

- Parameters are owned (`String`, `Option<PageId>`) and every fallible call returns `Result<T, VaultError>`. `VaultError` is one enum: `NotFound`, `Invalid`, `Io`, `Index`, `Crdt`, each with a message.
- `PageId`, `BlockId` and `AttachmentId` are `String` aliases. Page ids are 26-character Crockford base32 strings, random ULID-style for ordinary pages. Block ids are any non-empty string the caller chooses. Attachment ids are SHA-256 hex.
- `Vault::apply(page, Vec<BlockOp>)` edits blocks. `BlockOp` is `Insert`, `Delete`, `Move`, `SetText`, `SetKind`, `SetChecked`, `SetTarget`, `SetColor` and `SetWidth`, serialized as `{"op": "SetText", ...}`. `Insert` and `Move` take `parent: None` for the top level and `after: None` for the first child; a non-empty `after` must be a child of `parent`. A batch is validated against a simulation of the tree and then applied as one Loro commit, so a failing batch changes nothing.
- Marks (`Bold`, `Italic`, `Strike`, `Code`, `Link`, `PageLink`, `Underline`, `Color`, `Date`) count Unicode scalar values, not bytes or UTF-16 units.
- The Rust types are in `types.rs`; the signatures are in the `impl Vault` blocks. This page does not repeat them.

## On disk

```
<vault>/
  pages/<slug>-<id>.md                 # markdown + YAML frontmatter, written by the writer
  .vault/device                        # this copy's device id (random, created on first open)
  .vault/lock                          # held by the Vault that uses the device's Loro peer
  .vault/purged, purge-state.json      # purge tombstones and restoration acknowledgements
  .vault/hub/tokens.json               # hub only: paired device tokens (hashed)
  .vault/docs/<id>.loro                # Loro snapshot (source of truth together with the log)
  .vault/docs/<id>.blocks.json         # block cache, pages with 256+ blocks only
  .vault/logs/<id>.log[.old]           # Loro updates not yet in the snapshot
  .vault/cold/<id>.cold                # compressed original history prefix
  .vault/cold/dictionaries/<id>.dict   # device-local compression dictionary
  .vault/cold-policy.json              # optional local cold-history parameters
  .vault/conflict-history/<id>.loro    # history kept for a purged page that had conflicts
  .vault/blobs/<2 hex>/<sha256>        # attachments, content-addressed
  .vault/index.sqlite (+ -wal, -shm)   # index, rebuildable
  .vault/mounts.json                   # this device's mounted folders (not synced)
  .vault/mount-trash/<id>.md           # files of mounted pages moved to the trash
```

- The markdown file name is the lowercased title with runs of non-alphanumeric characters turned into `-`, at most 50 characters, or `untitled`, then `-<id>`.
- The markdown format (frontmatter keys, links, block comments) is in [Data model](data-model.md#on-disk).
- Everything that must sync (tree position, trash, favorites, icon, cover, display options, properties, schema, views, templates, conflicts) is in the page documents. The index holds only copies.

## A page's Loro document

- Map `page`: `id`, `title`, `parent`, `created`, `updated`, and when set `icon`, `cover`, `favorite`, `trashed`, `order`, `collection`, `template`, `istemplate`, the display options (`small_text`, `full_width`, `locked`, `cover_position`), `color`, `role`, the agenda settings `agenda_role` and `agenda_date`, and `views_migrated`.
- Tree `blocks`: a `LoroTree` with one node per block. A node's meta map holds `id`, `kind` (a lowercase string; unknown kinds are kept), `checked`, `target`, `color`, `width` and `text` (a `LoroText`).
- Map `props`: property key to the `PropValue` as JSON. Maps `schema` (with `schema_order`) and `views` (with `view_order`) exist on databases. Map `type_data` holds add-on data. Map `conflicts` holds conflict records. Map `peers` maps Loro peer ids to device ids, for history.
- Every vault session of a device writes as one Loro peer derived from the device id; a second `Vault` on the same folder uses a random peer ([Stable peer ids](../sync/vault-sync.md#stable-peer-ids)).

## Edits and durability

- Every committed change is exported as a Loro update and appended to `.vault/logs/<id>.log` (4-byte little-endian length, then the bytes) on the caller's thread. There is no fsync.
- Loading a page imports the snapshot, then `<id>.log.old` and `<id>.log`. A torn last frame is ignored. `open` queues every page that has a log for the writer, which finishes interrupted flushes.
- One thread (`vault-writer`) per vault flushes 300 ms after the last change, or at the latest 2 s after the first change of a burst. Before writing a page it takes the OS advisory lock `.vault/flush.lock`, imports changed disk content into the cached document, writes the merged snapshot durably, then truncates the merged logs in place.
- Writers in one process queue on a shared mutex for the lock path. Contention with another process times out after 10 s and logs a warning after 2 s.
- `flush()` waits for everything queued and returns the last background error. Dropping the `Vault` flushes and stops the thread. Loaded documents stay in a cache of 32 pages; dirty pages are never evicted.
- For pages with at least 256 blocks the writer also writes `<id>.blocks.json` (blocks plus frontiers). A cold `load_page` uses it when the version matches.
- Old history moves into `.vault/cold` once it is older than 30 days by default (`COLD_AFTER` in `vault.rs`, overridable in `cold-policy.json`); see [Sync and storage](../sync/sync-and-storage.md#hot-and-cold-storage-proposed-2026-10-12).

## Index

The index is `.vault/index.sqlite`, rebuilt from the Loro files when needed.

- Tables: `pages` (titles, folded title, parent, order key, `page_type`, flags, schema JSON, template, version vector `vv`, agenda columns), `props` (typed columns per page and property), `links`, `refs`, `attachments`, `agenda`, `conflicts` (a mirror of the page's records), `meta` (local one-off markers, not synced), and the FTS5 table `fts(title, body)` with the `unicode61 remove_diacritics 2` tokenizer.
- `user_version` is the breaking generation (`SCHEMA_VERSION` in `index.rs`). A change to it recreates the schema and `reindex()` rebuilds from the documents; an interrupted rebuild continues at the next `open`, and `.vault/index-update.json` carries its progress for the boot screen.
- Additive changes are ordered steps in `migrate.rs` (`STEPS`) that run in place on `open`. A step may carry a backfill that runs on the `vault-backfill` thread in batches of 500 rows and keeps its cursor in `meta`. New columns are nullable or defaulted; readers treat an unfilled value as absent. Such changes never bump `SCHEMA_VERSION`.
- Tree, trash, property, schema and favorite changes update the index on the caller's thread. Block edits (body, links, attachment references) reach it through the writer, so `search` and `backlinks` see them up to 2 s later, or right after `flush()`.
- Write transactions start with `BEGIN IMMEDIATE`; a deferred transaction that reads first fails with "database is locked" under WAL when the other connection committed in between.

## Properties, collections and queries

- A property value is checked against the schema of the row's database (known key, matching kind, existing option ids). Pages outside databases take any key not starting with `$`. Empty values remove the property. `CreatedTime` and `UpdatedTime` are computed from the page.
- `set_schema` assigns missing keys and option ids. A known key with a new name is a rename. Removing a property or option removes its values from every row; a kind change converts values where it can and drops the rest. Views lose references to removed properties.
- Property kinds are in `PropKind`: text, number, date, checkbox, select, multi-select, status, relation, URL, email, phone, created time, updated time, files, formula and rollup.
- Formula properties evaluate an expression over the row (`formula.rs`). Rollup properties aggregate a property of the pages a relation points at (config keys `rollupRelation`, `rollupProperty`, `rollupFunction`). Computed values are never stored in documents: they are written to the index's `props` table when a row is indexed, and for rows whose relations point at it, up to 4 levels away. An error is the value `PropValue::Error(message)`.
- `query` compiles the filter tree to SQL: each condition is an `EXISTS` over `props` (or a comparison on `pages` for `$title`, `$created`, `$updated`). Negative operators are the `NOT` of the positive one, so pages without the property match them. Sorting is done in Rust over the fetched rows; empty values sort last in both directions.
- `query_groups` counts rows per option, checkbox value or distinct value. `query_range` returns one entry per page whose date overlaps `[from, to)`.
- **Templates.** Children marked `istemplate` are templates for new children of their parent; the parent's `template` names the default. Templates are left out of lists, search and queries. `create_page` copies the default; `create_page_from` copies the one given, or none. Copying takes blocks with new ids, and properties, icon and cover only where the page has none. A database's default template is the same mechanism.

## Agenda, Journal and spaces

- A database's `AgendaSettings` role is Auto, Tasks, Events or Off. Auto means Tasks for a Date plus Status or Checkbox, Events for a Date alone, Off otherwise. `date_prop` overrides which Date property counts.
- Dated rows and to-do blocks with a valid ISO date mention become `AgendaItem`s. `agenda`, `overdue` (capped in SQL), `overdue_count`, `done_since` and `in_motion` read the `agenda` table.
- `daily_note` finds the live Journal by role, returns the existing whole-day Date row, and otherwise creates the Journal and the day note. New day notes start with top-level `Tasks` and `Log` toggles. Creation holds `.vault/daily-notes.lock`. `append_log` and `link_task_to_day` build on it.
- The Journal's fixed id is `000000000000000000000000JR`; the hidden Calendars page is `...CA` and the Feedback page `...FB`. A day-note id is derived from the Journal id and the ISO date (`ids::journal_day_id`, SHA-256), so two devices creating the same day offline agree on the id. `agenda.rs` and `day_repair.rs` merge the duplicate `Tasks` and `Log` sections after sync, and `Vault::open` runs the repair pass once per repair version.
- If one device deletes a day-note section while another edits it offline, the editing side's version wins ("edit beats delete", [Conflict format](conflict-format.md)).
- `Page.color` and `Page.role` are optional document metadata; the roles in use are `journal`, `calendars` and `feedback`. The Feedback page and its descendants are left out of root lists, counts and search, and are reachable by id ([Feedback](../app/feedback.md)).

## Page tree and trash

- Sibling order is a base-62 fractional key (`order` in the document, `sort_key` in the index). `move_page` rejects a move into the page's own subtree.
- `delete_page` moves the page and its subtree to the trash and removes their markdown files. The trash is the `trashed` time in the page's document; the index derives `in_trash` for the subtree. `restore_page` clears it and writes the files again; a page whose parent is gone or still trashed returns at the top level. `purge_page` and `empty_trash` delete the documents for good.
- `list_pages`, `search`, `page_count`, `list_children`, `backlinks` and `query` leave out pages in the trash; `load_page` still loads them with `in_trash` set.
- `duplicate_page` copies the subtree right after the original with " (copy)" appended to the title. Favorites keep the order in which they were favorited.

## Links and attachments

- Inline `PageLink` marks and `LinkToPage` blocks are links; the writer stores them in `links`. `backlinks` lists live pages linking to a page, most recently updated first.
- An attachment is stored once per content, keyed by its SHA-256. `gc_attachments` flushes, then removes blobs that no page, trashed or not, uses as image, file or cover.

## History and the change feed

- `list_versions` groups the document's change log causally: a new version starts after 5 idle minutes or when the next change comes from another device. A version id is the version's frontiers.
- `load_page_at` forks the document at a version. `restore_version` turns the current blocks into the old ones as one new change and restores title, icon, cover and properties; tree position, trash, favorite, schema and views are kept.
- The change feed is an in-memory window of the last 10,000 changes, numbered from 0 at each `open`. `poll_changes(since)` returns the changes after `since`, each page once, or `reset: true` when `since` is outside the window. Every mutating call and the writer push to it.

## Import and rescan

- `import_markdown(dir, parent)` imports markdown with frontmatter under a parent. A folder's body is the same-named adjacent `.md`, then `index.md`, then `README.md`. Relative `.md` links resolve from the source file's directory; unresolved links are reported.
- Re-import reuses pages through reserved source-path properties, updates their content and position, and adds no operations when nothing changed. Missing sources are reported as orphaned and kept. Dot folders and secret filenames are excluded.
- `rescan` reads every `pages/*.md` whose hash differs from the one last written. It is a three-way merge: the base is the document forked at the version the file was rendered from, and only the differences between base and file are applied, so app edits made after the file was written are kept. A new `.md` without a known id becomes a page; a missing file is written again.
- Mounted folders are rescanned by `rescan_mounts` ([Folder mounts](mounts.md)).

## Limits

- Several writable `Vault` instances on one folder merge edits when flushing; the second uses a random Loro peer, so its version vectors grow per session.
- Paths longer than 260 characters fail on Windows (SQLite cannot open the index).
- Outside edits to `.md` files are read only by `rescan`. The writer does not overwrite a file whose hash changed.
- A formula using `now()` is not re-evaluated as time passes.
- Update logs are not fsynced.
- The change feed restarts at every `open`; a caller with an older sequence gets `reset`.
- Markdown rendering is readable but not exact CommonMark in edge cases.

## See also

[Search performance measurements](search-measurements.md) has the recorded search timings.

## History

Sync hooks and conflicts were added from 2026-10; page types replaced the collection flag on 2026-10-12; index migration steps arrived on 2026-11-02. Earlier states of this page are in git.
