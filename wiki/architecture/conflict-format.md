# Conflict format v2

Status: core and sync backend built 2026-10-14; merge screen built on `feat/merge-screen`. The text rule below refines v2 (2026-10-14). Reviewed together with the merge-screen mockup in [`design/merge/`](../../design/merge/index.html) (v2).

Implementation evidence (2026-10-14): core/sync backend, Rust bridge and interim Flutter compatibility are built and checked. Core unit checks report 199 passed and 2 ignored; 5 detection tests, 5 public resolution tests and 13 unchanged import-recovery tests also pass. All 54 sync tests pass: 18 unit, 3 new HTTP, 5 hub, 9 purge and 19 zstd tests. FRB 2.13.0 regeneration, native bridge and fake-agent builds are complete; 21 Rust bridge tests, 12 MCP tests (2 unit, 10 integration) and 12 targeted Flutter tests pass, with no analysis issues.

Master fixes `16e6080` and `11c2c0b` are merged. Post-merge checks pass: 8 journal tests, 13 unchanged import-recovery tests, 3 HTTP conflict-history tests and the same 12 Flutter tests with clean analysis. The full workspace gate passed on linux01 on 2026-10-14 after detector fix `3093be8`: 775 passed, 5 ignored, 0 failed. It ran `cargo test --workspace --locked --exclude notes-tauri` through `linux01-run --with ../BubblePaw-base-agent`. The approved [merge-screen mockup](../../design/merge/index.html) (v2) is a later UI build.

Integration with master's cold history store (`e3a497a`, 2026-10-14) preserves conflict side and undo frontiers before the shallow root. Historical reads and scoped edits use the verified complete history; retained conflict archives keep that history before cold files are removed. Ordinary hot snapshots stay shallow, and recent sync does not read the cold prefix unless historical conflict work requires it. Externally supplied shallow snapshots without a local prefix retain Loro's existing history limitations.

Six new regressions cover pre-root conflict sides and undo, retained archives, causal purge recovery, post-root text conflicts and recent sync without prefix reads. All 29 cold tests, 54 sync tests and 11 unchanged shallow-limit tests pass; the storage-metrics build check also passes. The full workspace gate with the base-agent checkout passes with 823 passed, 6 ignored and 0 failed. Native bridge and fake-agent builds pass; Flutter analysis is clean and all 19 tests in `sync_test.dart`, `ai_test.dart` and `today_store_test.dart` pass. The conflict refresh now checks for vault closure between its pending-list and log reads.

## Format

**Scope.** Each conflict names what it covers:
- `Block { block }`: text or properties of one block edited on both sides (today's case).
- `Subtree { root }`: a block with its children. A section is a heading and the blocks under it. Used when one side deleted the subtree and the other edited anything inside it.
- `Page`: the whole note. One side trashed or purged it, and the other edited it.

**Sides.** Each side records:
- `device` and `time_ms`, as today.
- `version`: the page's Loro frontiers on that side when the conflict was recorded. Page history keeps every version (decided 2026-10-13), and cold storage keeps old history readable, so any side can be rebuilt exactly from this reference. That is what makes undo possible.
- `state`: `Present { content }`, `Deleted`, `Trashed { at }` or `Purged { tombstone_version }`. `content` is the scoped blocks *with children*, materialized for display. The frontiers stay the source of truth.
- `anchor` (subtree and block scope): the parent block and previous sibling on that side, so a restore lands where the content was.

**Resolutions.** The rule is applied when the conflict is recorded, so the note already shows the result when the user opens it (merge screen v2, 2026-10-13). The user can only confirm it or switch sides:
- `Keep`: "OK". Leave the note as it is and dismiss the entry.
- `UseSide(i)`: "Use the other version", one tap. Make the scope exactly as it was on side `i`.
  - A subtree comes back whole at its anchor, falling back to the end of the parent if the anchor is gone.
  - A page comes back out of trash, or is recreated after a purge, from that side's full state.
  - For text edited on both sides, this is the inline pick: the note shows the text rule's side (see below), the other side's text sits next to it in a lighter style, and a tap on it applies `UseSide` at block scope.

There is no manual merge, AI result or "keep both" on the conflict screen; after resolving, the user edits the note as usual. (v1 had `Replace { blocks }` and `KeepBoth`; v2 drops them.)

Every applied resolution stores `before`, the page frontiers just before applying it, plus `by`, either `user` or `auto(rule)`. Undo restores the scope to `before`.

**Trash state.** Page-scope conflicts carry the trash state of each side. Restoring the editing side's version clears trash or the purge tombstone. A purge tombstone carries the page's version at purge time, distinguishing new edits from a stale copy. Delete/edit records the page conflict and applies the whole editing version. Choosing a purged side retains the full conflict history and lifecycle decision in `.vault/conflict-history/<page>.loro`; exchanging that archive must not recreate an active note. Historical versions remain readable, and explicit scoped undo or page-history restore records a new restoration decision.

**Auto-resolve** is a single switch.
- The rule always applies: delete versus edit, at any scope, becomes `UseSide(editing side)` as soon as it is detected.
- Switch on: these entries dismiss themselves into the log, and undo stays available from the log and page history. Switch off: they stay in "Changed on two devices" until the user taps OK.
- Text edited on both sides always waits for the user's tap. The note shows one side's text meanwhile, chosen by the text rule.

**AI.** The conflict screen has no AI button. The MCP merge tool (`crates/mcp/src/merge.rs`) stays for agents.

**Compatibility.**
- New fields get serde defaults. Old records read as `Block` scope with `Present` or `Deleted` state and no version, and old records cannot be undone.
- Conflict records and their resolutions sync between devices, so a resolution on one device clears the conflict everywhere.
- The bridge has new types and regenerated FRB 2.13.0 bindings; Rust, native build and targeted Flutter checks pass.

## Implementation and acceptance status (2026-10-14)

The core format, detection, scope restoration and undo, display diff, settings/policy, archive/lifecycle transport hooks and archived page-history reads/restores are implemented on `feat/conflict-format`. Core, sync and Rust bridge/MCP checks listed above pass. `Vault::list_conflicts` returns pending review; `Vault::conflict_log` also exposes recent undone applications on reopened entries. The bridge supplies both `NotesVault::conflicts` and the `list_conflicts` alias. Bridge DTOs, regenerated bindings and interim Flutter adaptation are present; native bridge and fake-agent builds are complete; all 12 targeted Flutter tests pass and analysis reports no issues.

**Workspace gate:** passed on linux01 with 775 passed, 5 ignored and 0 failed after fix `3093be8`. The focused format/bridge and retained Flutter checks above pass. Current notes are in [Sync state](sync-state.md#conflicts), [Core state](core-state.md) and [App state](app-state.md#bridge). No packaged release or deployment is claimed here.

The undo-status decision was settled by the user on 2026-10-14 below and is implemented and checked in core and cross-replica transport. The interim Flutter compatibility surface passes its existing tests.

Current linux01 checks (2026-10-14): the locked workspace gate passes with 959 passed, 6 ignored and 0 failed. FRB 2.13.0 regeneration, the release bridge build and Flutter analysis with the original analyzer config pass.

## Known limits

- **Known limit: journal text.** Text conflicts inside journal-repair-restored sections keep the merged text; journal repairs stay log-only.
- **Known limit: divergence.** Records can diverge when one side keeps editing after the conflict is recorded.

## Decisions

- 2026-10-13: A resolution syncs, so resolving a conflict on one device clears it on every device.
- 2026-10-13: Auto-resolve is one vault-wide setting, and its log keeps entries for 30 days.
- 2026-10-13: Journal repairs are logged only and never enter the pending list.
- 2026-10-13: The conflict screen labels the sides Local and Cloud.
- 2026-10-14: Undo of an ordinary resolution returns the conflict to pending review on every device.
- 2026-10-14: `UseSide` and undo restore missing ancestors in their original order; with no usable version, the content is appended at the page root and logged.
- 2026-10-14: Text edited on both sides shows the side with the later edit time, ties going to the greater device id, and still waits for a tap.
- 2026-10-14: Purge records carry the purging device id behind the `X-Purge-Device` capability.
- 2026-10-14: Settings > Conflicts shows a 30-day "Recent" log below the pending list.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Architecture > Conflict format v2 (approved 2026-10-13)).
