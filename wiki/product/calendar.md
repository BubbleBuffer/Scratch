---
type: topic
layer: state
summary: "The calendar: one task note type for everything dated, calendars as coloured categories, one vault-wide calendar view, and what is built so far."
parent: "[[product/index]]"
status: current
---

# Calendar

## Current state (2026-10-13)

- **Tasks add-on:** `app/lib/addons/tasks/` registers one `task` note type and New task. Chip, card and embed share the task presentation; the full page puts editable scheduling fields above the ordinary note editor. Fields persist in `tasks` type data. The composer accepts title, date, optional time and duration, calendar and assignee. Assigning `Agent` starts one agent run for the task; see [Tasks assigned to Agent](../integrations/ai-assistants.md#tasks-assigned-to-agent).
- **Short card:** calendar day lists use the tasks add-on's short card, with a calendar colour, title, time or overdue date, calendar/source and an Agent badge only for agent-assigned items. Done titles are muted and struck through. Note cards and embeds retain dates. Today uses the same short cards for its selected-day agenda.
- **Vault calendar:** Calendar registers a top-level sidebar entry and opts into the phone dock (Pages, Today, Calendar, Search), retaining `calendar.open` / Ctrl+Shift+C. The phone floating + creates a page in Pages and opens the task composer in Today and Calendar. Month scrolls vertically across months; desktop week/day show a time grid with a now marker in the gutter. Phone week/day swipe between periods. Jump, period/day keys, per-day creation and day lists are available. Timed items clip at midnight.
- **Categories:** the panel creates, renames, recolours and deletes categories, edits their independent Show in calendar / Show in Today switches and toggles database overlays. Deleting a category keeps its tasks and dates. Hiding a database keeps its rows and dates.
- **Rescheduling boundary:** task notes and database rows drag between calendar days, retaining time and duration. Database rows use the configured agenda date property and shift both ends of a date range through the existing property write. Dated to-dos remain non-draggable: the bridge offers only full-block `SetText`, which cannot safely preserve concurrent text/mark edits between loading and applying. They need an atomic date edit such as the core's unexposed `reschedule_agenda`. Mentions are read-only calendar references.
- **Today:** the continuous date strip and compact selected-day agenda replace the master body. Calendar and Today share the existing selected date. The selected day's Journal note now embeds the existing full page editor, with lazy first-edit creation, past/future editing and Open as page. No paged-agenda dependency is added.
- **Note drops:** sidebar note rows publish the kit's `NoteDragPayload`. Month days and week/day slots accept it. A task updates its date (and the chosen slot time); another note gains a final paragraph with a date mention. Existing mentions of that day produce a duplicate toast. Undo restores the previous task date or removes only the inserted paragraph after checking for later changes, including added children. Same-client drops are serialized; cross-client duplicate and Undo check/write races remain because the bridge has no conditional insert/delete or compare-and-set. Today does not accept these drops.
- **Cards and title dates:** Board and Kanban use the task type's optional board-card builder, showing the shared checkbox card with its date. Ordinary Board rows retain `RowCard`. Title-chip providers show a task's date and optional time; other dated notes show the next upcoming distinct day, plus "+N" opening the other dates, or the earliest day when all are past. Tapping any date opens Calendar on that day.
- **Shell boundaries:** navigation, floating actions, drag payloads, optional board cards and title chips are generic kit contracts. The tasks add-on owns Calendar behavior and data writes. Tasks is optional, default on; disabling it removes its registrations at the next start and preserves its notes and data. Today's body uses the approved date strip and short cards; shell contracts stay generic.
- **Golden sources:** calendar month/week/day, task page and title-date chips cover light/dark and desktop/phone. The paged-Today golden source is removed. Baseline PNGs are pending and are not generated in this lane. Existing Home/Pages and sidebar-containing golden baselines also need review after the navigation changes.
- **Dart verification (the Linux machine, 2026-10-13):** analysis and the import checker pass. New feature checks and the existing sidebar/Home/Board/Kanban/agent-panel/Today regressions pass. One whole fast-tier run records 874 passed, 8 skipped and one sync conflict-edit timeout with a native call pending. The unchanged sync file then passes all six cases on both this lane and `b5333a9` with the same bridge and toolchain; the full-tier timeout remains unexplained. No golden images are rendered.
- **Core:** `crates/core/src/agenda.rs` indexes:
  - task notes, from type data `tasks` (status, date as ISO, `duration_min`, assignee, calendar; the checkbox flag controls presentation);
  - the first date mark of each non-to-do block, as kind `Mention`;
  - database rows and dated to-dos, as before.
- **Queries:**
  - `calendar_items` returns everything for the calendar;
  - `agenda`, `overdue`, `overdue_count` and `done_since` exclude mentions.
- **Model:** `AgendaItem` has `calendar` and `assignee`.
- **Tests:** `crates/core/tests/calendar_source.rs`.
- **Dart:** `Vault.calendarItems` in `app/lib/core/vault.dart`.
- **Database add-on:** its own calendar view (`app/lib/addons/database/views/calendar/`) is unchanged.

## Decisions

- 2026-10-12: one `task` note type holds everything dated; there is no separate event type.
- 2026-10-13: calendars are categories, stored in a hidden system page (role `calendars`).
- 2026-10-13: a plain `@date` mention shows in the calendar but not in Today; a dated to-do stays in Today.
- 2026-10-13: calendar and tasks use the short item card; the paged Today portion is withdrawn.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Product > Calendar).
