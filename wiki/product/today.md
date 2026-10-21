---
type: topic
layer: state
summary: "Today, the week, daily notes, quick capture, progress, space colours and reminders: the organizer features the Stacks direction is built on."
parent: "[[product/index]]"
status: current
---

# Today and the organizer layer

**Status (2026-10-13):** The horizontal date strip and selected-day short cards replace the master Today body. The day note uses the existing page editor in a generic embedding mode. Viewing or focusing an empty day creates nothing; the first typing creates its Journal document. The paged agenda is withdrawn.

The Journal core and bridge are merged, including Tasks + Log sections and idempotent task links.

## What counts as a task or an event

Tasks and events stay ordinary pages and blocks ([data model](../architecture/data-model.md)). The *agenda* is everything with a date that belongs on a day:

- **Task notes.** The `tasks` add-on stores status, date, optional time/duration, optional checkbox presentation, calendar and assignee in core type data. Status controls completion. Category switches control Calendar and Today independently.
- **Database rows.** Each database has an agenda role:
  - **Tasks:** the row has a date. Done means a status in the Complete group, or a ticked checkbox.
  - **Events:** the row has a date. A timed event goes in the timeline; an all-day event goes at the top of the day.
  - **Off:** not in the agenda.
  - **Auto (default):** a database with a date property *and* a status or checkbox property is Tasks. One with a date but neither is Events. Otherwise Off.
  - The agenda date is the first date property that isn't created or edited time; the database settings can choose another. Both overrides live in the database's "Show in Today" settings.
- **To-do blocks with a date.** A to-do whose text contains a date mention ("Set date" in the block menu; typed mentions like `@tomorrow` are not parsed yet) is a task on that day. Done means ticked. To-dos without a date stay plain checkboxes.
- **Status groups move into the core.** Complete is known in the core, with the app's existing rule:
  - each option's configured group is used;
  - otherwise the first option is To do, the last is Complete and the rest are In progress.

## Today (home)

Today stays a top-level shell destination. Its date strip scrolls continuously in both directions, shows weekday and date, marks days with Today-visible agenda items and Journal notes, and shares the selected date with Calendar. Today returns to the current day; the month label opens the jump picker. Left/right select adjacent days while the strip has focus; T returns to today and G opens the picker.

The selected day uses the tasks add-on's compact short cards for timed items, tasks and dated to-dos. Plain `@date` mentions remain excluded by the shared agenda query. Overdue appears only on today. The mockup's initial five-card limit has Show more; overdue and larger done groups have disclosures, while one or two done items are shown directly. Completion and source-page opening use the existing agenda actions.

The day note embeds the ordinary page editor, editable on past and future dates and created only on first edit. Open as page opens the same document; returning restores inline editing. Embedded mobile editing keeps the existing keyboard toolbar and block menus above the keyboard, including on long notes. Quick capture remains available and writes Task/Note blocks into today's Journal note above Log, leaving Log last. The searchable All pages disclosure and shell navigation remain available; the old greeting, timeline, In motion and recent-card body is removed.

## Daily notes

- **Journal:** the first daily note creates a top-level database "Journal" with a Date property and agenda role Off. New day notes start with an empty Tasks section and a collapsible Log last, both using the editor's existing toggle blocks; existing notes and Journal templates stay unchanged.
- **Task links:** the core and bridge expose `link_task_to_day` to add a task page link under Tasks once per day, creating the note or missing Tasks section when needed. The tasks add-on links dated task saves and Calendar rescheduling; undated tasks do not link. Link failures are logged and do not fail a successful task save. Agent tools and database-row date edits are not wired in this round. Linking on a new day keeps the old day's link. `append_log` appends timestamped lines under Log.
- **One page per day:** each note is titled in the app's long date format and dated with that day.
- **Finding the note:** the Journal is found by a page role `journal`, not by its title. If the user renames it, it still works.

## Spaces and colours

- Top-level pages are the spaces.
- **Colours:** a top-level page can have a colour from the family set: apricot, paw, moss, duvet, sun (attention) and sky (information).
- **Assignment:** new top-level pages, imports and copies have no colour by default. The user sets one from the page menu when they want it; existing pages keep their colours across restarts. Uncoloured spaces render without a space dot and use the existing neutral or fallback styles in Today.
- **Where it shows, quietly:**
  - a small dot before the space in the sidebar;
  - the dot on agenda items and In motion entries from that space;
  - the progress bar of In motion entries.

## Reminders

- **Timed events:** a notification at the start, or earlier by a lead time: 0, 5, 10 or 30 minutes (default 10).
- **Timed tasks:** a notification at their time.
- **Morning summary:** one notification at a set time (default 08:00), such as "5 tasks due today, 2 overdue". None when nothing is due.
- **Settings > Reminders:** on/off (on by default), the lead time and the morning time.
- **Android:**
  - Notifications are scheduled with the system, so they arrive when the app is closed. They are rescheduled after a reboot.
  - Notifications use inexact delivery, which can arrive a few minutes late, because exact alarms need a permission that Android 14 denies by default.
  - The app asks for notification permission the first time reminders are needed.
- **Desktop (Windows, Linux, macOS):** notifications are shown by the running app only. Running in the background or tray is not in this round.
- **Scheduling window:** the next 7 days, rescheduled when the agenda changes and at start.

## First run and demo

- **A fresh vault** starts with:
  - Getting started, which explains Today, capture and dates, linking pages (@ or [[, backlinks, the page stack) and databases (views, properties, /database);
  - a Tasks database (status To do / Doing / Done, a Due date);
  - a Calendar database (a Date with time, which makes it Events);
  - the Journal on first use.
- **A demo vault** (`--dart-define=SEED_DEMO=true`) holds the [direction brief's](../../design/directions/BRIEF.md#same-content-in-every-direction) content with dates relative to today, so every feature has something real on it.


## Build

1. **Core and bridge — built 2026-10-06:** status groups, the agenda index and queries, bounded overdue and overdue count, to-do progress, daily notes, space colours and roles, set done. ([Core state](../architecture/core-state.md).)
2. **App, Today — built 2026-10-06:** home sections, week strip, capture, In motion, to-do dates in the editor, "Show in Today" in database settings, colours in the sidebar and page menu, first-run and demo seeds. ([App state](../architecture/app-state.md).)
3. **App, reminders — built 2026-10-07:** notifications on Android and desktop, plus the settings. Android schedules absolute UTC instants with inexact delivery; Windows, Linux and macOS deliver only while the app runs. Settings hold the switch, event lead and morning time on this device. ([App state](../architecture/app-state.md#reminders).)

Build deviations: a focused to-do shows its canonical date mention while resting to-dos show the compact chip, preserving editing offsets and IME behaviour. New top-level pages and Colour > None store explicit `none`, which the boot backfill leaves uncoloured across restarts. The demo leaves Lisbon's editable Dates/Status/Budget/Tags out: ordinary pages need a parent database schema for these, and making Trips a database would change the settled hierarchy. No such hierarchy change was made. The assignment's capture → week → day order and dots after sidebar titles take precedence over the earlier ordering above. Reminders keep the agreed behaviour: because agenda items carry source ids only, the app loads source titles before pure planning; no core changes were needed. The seven-day upper edge is inclusive. Validation is headless; notification delivery on a real Android device and on Windows remains unverified.
## Decisions

- Agreed with the user: features come before the new look; to-dos can carry a date; daily notes and reminders are in; space colours are used quietly.
- Not in this round: Google Calendar sync, recurring tasks and events, notifications from a closed desktop app, snooze (done or open only), and time zones beyond the device's own.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Product > Today and the organizer layer).
