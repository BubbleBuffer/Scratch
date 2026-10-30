---
type: page
summary: "Triage: merge proposals for duplicate pages, the merge and its undo (vault_core triage.rs), and the AI run that makes them."
parent: "[[core/index]]"
status: current
---

# Triage

Built 2026-10-30. Code: [crates/core/src/triage.rs](../../crates/core/src/triage.rs), [crates/mcp/src/triage.rs](../../crates/mcp/src/triage.rs), the tools in [crates/agent/src/tasks.rs](../../crates/agent/src/tasks.rs), the bridge [app/rust/src/api/triage.rs](../../app/rust/src/api/triage.rs). Tests: `crates/core/tests/triage.rs`, `crates/agent/tests/triage.rs`.

A user-started AI run reviews the pages of one project and **proposes** merges of duplicate or overlapping pages. Nothing is merged until the user accepts. The model detects duplicates from titles and short summaries; the core has no similarity code.

## Project

There is no project page type, so a *project* is a **top-level page and its subtree** (the same reading as the page tree's roots). A run is scoped to one such page id, or to an explicit list of page ids. Two pages may be merged only if they have the same top-level ancestor.

## Proposals

- Stored in the type data (add-on `triage`) of the hidden Nodes page (fixed id `000000000000000000000000ND`), one map entry per page pair. The Nodes page is already left out of lists, search and counts, so no new hidden role exists. Entries sync like any page data.
- The entry key (the proposal id) is the **sorted page id pair** `<lower>-<higher>`, so a pair has exactly one record on every device, whatever order it was proposed in.
- A record has `keep` (the older page by creation time, tie broken by id; never the model's choice), `dropped`, `reason` (at most 500 characters), `status`, `created_ms`, `task` (the AI task row), `merged_version` and `decided_ms`.
- Status is `pending`, `accepted` or `dismissed`. `triage_propose` on a pair that already has a record (any status, either order) returns that record unchanged, so a dismissed pair is never proposed again.
- A pair is rejected with an error when it is the same page, a trashed or purged page, a system page, template or database, pages of different page types, pages of different projects, or a page with its own sub-page.

## Merge and undo

`Vault::merge_pages(keep, dropped)` is one history commit of `keep` (commit message `triage:merge`): the blocks of `dropped` are appended after a divider and a "Merged from <title>" line, properties, icon and cover that `keep` lacks are copied, and then `dropped` is trashed. It refuses a `dropped` page that has sub-pages (they would be trashed with it). It returns `keep`'s version id from before the merge, which `triage_accept` stores as `merged_version`.

`triage_undo` is `restore_version(keep, merged_version)` plus `restore_page(dropped)`; the kept page returns to its exact blocks and properties as a new change, so history keeps the merge. The proposal then becomes `dismissed`, so the pair stays unproposed. A test covers the exact restore.

**Not done:** links to `dropped` are not repointed to `keep`; they keep pointing at the page in the trash (restorable). Repointing needs link rewriting in every linking page's marks and link blocks, and is a follow-up.

## The run

`vault_mcp::triage::start(root, pages, runner)` creates a normal AI task (runner `agent`, pinned to a PC's device id or `hub`, like `ai_assign_task`) with a fixed prompt. The row's text property `triage` holds the scope JSON and marks it a triage run; its placeholder goes in the project page (or the first listed page). Fewer than two pages in scope is an error.

Only delegated runs whose row has a scope get two extra tools, registered beside `report_progress` and `todo`:

- `triage_pages()` lists the pages in scope: id, title, type, a summary of the first 240 characters of text, creation time (at most 300, oldest first).
- `triage_propose({a, b, reason, keep_id?})` calls `triage_propose` for two listed pages. `keep_id` is ignored. A page outside the scope is an error.

Ordinary tasks, chats and the MCP gateway do not get them.

## Bridge

`triage_start(scope_project_id, page_ids, runner)` returns the task id; `triage_proposals()` lists pending proposals (`triage_all_proposals()` all of them) as `TriageProposalDto`; `triage_accept(id)`, `triage_dismiss(id)` and `triage_undo(id)` return the updated dto; `triage_page_id()` is the Nodes page id, whose change in the change feed means the list changed.

## Links and tags by the run

Built 2026-10-30. A triage run also gets `triage_link({from, to})` and `triage_tag({page, tag})`, for triage runs only. They apply at once (merges are only ever proposed), and both pages must be in the run's scope.

- `triage_link` appends a link-to-page block at the end of `from`. Pages of different types may be linked; pages of different projects may not. A page that already links to the target (block or inline page link) is left alone.
- `triage_tag` adds a tag to the page's `tags` property (a multi-select; the tag is trimmed, a leading `#` dropped, 1 to 64 characters, no commas). A tag the page has already (ignoring case) is left alone.
- Each change is appended to the task row's text property `triage_actions` (the run's outcome), a JSON list of `{kind, page, target, block?, previous?}`: a link records its inserted block, a tag the page's previous `tags` value (absent when it had none).
- `vault_mcp::triage::undo_actions(task)` takes back all of a run's changes newest first and empties the record (`Vault::triage_undo_action` does one). A link's block is deleted if it is still there; tags are restored to the previous list, so tag edits made since are lost. Edits to other properties and blocks stay.

The bridge exposes the record: `triage_applied()` lists the runs (newest first, among the latest 200 tasks) that still hold applied links or tags, with page titles, and `triage_undo_applied(task)` takes a run's changes back. The Merge proposals screen shows them (see [Triage UI](../app/triage-ui.md)).

Link repointing is not built: links to a merged-away page keep pointing at its trashed copy. It means rewriting every linking page and recording each rewrite for undo, which is its own change.
