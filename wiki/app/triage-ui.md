---
type: topic
layer: state
summary: "The Triage UI: the Triage action on a project, the runner choice, and the Merge proposals list with Merge, Dismiss and Undo."
parent: "[[app/index]]"
status: current
covers: ["app/lib/features/triage/**"]
---

# Triage UI

Built 2026-10-30 on the backend in [Triage](../core/triage.md). Code: `app/lib/features/triage/`. Tests: `app/test/features/triage/triage_ui_test.dart` (fake bridge).

## Start

A project is a top-level page. **Triage** is an item of the sidebar "•••" menu and of the page "•••" menu, shown for top-level pages only. It reads the runners from the AI runner bridge (`AiStore.runnerBridge`), then asks which runner to use: a bottom sheet under 700 px, a centred panel at 700 px and wider. The hub and each PC are listed, the default marked, offline PCs included (the run starts when they are back). One tap on a runner calls `triage_start(projectId, [], runner)` and shows a toast. This is its own small choice, not `runner_picker.dart`, which is tied to a kanban card. Starting on a set of selected pages is not built.

## Proposals

**Merge proposals** is a standalone screen (phone) or panel (desktop), not part of Settings. It opens from the sidebar row of the same name and from the page "•••" menu of a top-level page. The sidebar row appears only while there are pending proposals or merges that can be undone, with the pending count as a badge.

Each entry shows the page kept (the older) and the page merged in (the newer), both as links that open the page, the reason, and what Merge does. **Merge** calls `triage_accept`; the entry then reads "Merged" and offers **Undo** (`triage_undo`, which restores through page history). **Dismiss** calls `triage_dismiss`; dismissed entries leave the list. Empty state: "No proposals. Run Triage on a project to look for duplicates."

## Store

`TriageStore` (one per vault, `TriageStore.of(app)`) holds the list from `triage_all_proposals` and the pending count. It reloads when the change feed reports a change to the proposals page (`triage_page_id`). Reading the list never creates that page but `triage_page_id` can, so the store asks for the id only after a read found proposals; until then it re-reads on each change batch. The widget is `ProposalsPanel` in its own file so it can move into a Review view later.
