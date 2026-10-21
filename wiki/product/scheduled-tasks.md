---
type: topic
layer: state
summary: "Schedule notes whose frontmatter holds a trigger, one runner node, allowed tools and a list of steps; phase 1 ships the weekly note."
parent: "[[product/index]]"
status: "current (phase 1 built)"
---

# Scheduled tasks

## Format

**A schedule is a note of type `schedule`.** Its definition is the note's frontmatter, stored as `type_data.schedule` ([data model](../architecture/data-model.md)). Reason: only pages carry properties and frontmatter; blocks do not. It sits in normal notes as a child page or an embed. Frontmatter is the stored format; users edit it in the schedule view.

```yaml
type: schedule
schedule:
  version: 1
  trigger:
    rrule: "FREQ=WEEKLY;BYDAY=MO"   # omit for a one-off
    at: "2026-10-19T08:00"
    tz: Europe/Berlin
    missed: latest                   # latest | skip
  runner: hub                        # a node record id
  tools: write                       # read | write | everything, or a list of tool names
  steps:
    - id: note
      do: note.from_template
      template: "[[Weekly note]]"
      parent: "[[Journal]]"
      title: "Week {{week}} {{year}}"
    - id: recap                      # phase 3
      do: agent.prompt
      prompt: "Add last week's done tasks to {{steps.note.page}}."
      if: "{{steps.note.created}}"   # phase 4
```

Rules that let the format grow without breaking:

- `version` starts at 1. A runner refuses a schedule with a higher version and says why on the card.
- Unknown keys are kept on edit and ignored by older clients.
- Every step has an `id` and a `do`. A runner that does not know a step's `do` refuses the whole schedule. It never runs part of it.
- Step outputs are read as `{{steps.<id>.<field>}}`. Each step kind documents its outputs.
- `if` holds one expression over step outputs and date fields. v1 of conditions has only "is set", "equals" and "not".
- `tools` limits every step. It is a tool set, the same as agent tokens use: `read`, `write` or `everything` (every tool the runner serves, including later ones). A list of tool names, as schedules were first written, still works; old names count as the tool they run. The note step counts as the tool `create_page`, so it needs `write`, `everything` or a list naming it; the runner refuses the schedule otherwise, and checks again before each run. The schedule page's Tools section offers Write and Everything (Read cannot create the note) and shows an older list as "Listed tools". New schedules get `write`. A runner from before tool sets refuses a `tools` string as invalid until it is updated.
- The note body is free text for the user. The runner does not read it.

Run results:

- The runner writes `last_run` (time, result, link to the output) into `type_data.schedule_status`. That key syncs, so every device's card shows it. Reason: the older sketch already wrote status back to the page.
- The detailed run log stays on the runner, next to `agent.sqlite`.

## Trigger

- v1: a start time plus an optional recurrence rule. No rule means a one-off.
- With a rule, `at` is only the anchor: the earliest date and the time of day. `at` counts as a run only if it matches the rule (decided 2026-10-15). Reason: changing the weekday in the view must not leave a stray first run.
- The rule is a small subset of iCalendar RRULE: daily, weekly with weekdays, monthly by day of month, an interval, and an optional end. Reason: Google Calendar import brings RRULE anyway (see Google in the internal wiki), and recurring tasks can reuse it later ([calendar](calendar.md)).
- No raw cron in the UI.
- `missed: latest` runs only the latest missed occurrence once when the runner comes back. `missed: skip` runs nothing. The default is `latest`. Reason: one catch-up keeps the result current without a pile of copies.
- Event triggers (tag added, task done, event starting) are not built.

## Current state (phase 1 engine)

`crates/agent/src/schedules/format.rs` validates v1 definitions and calculates the supported recurrences with bundled jiff time zones.
`crates/agent/src/schedules/mod.rs` runs accepted schedules and persists node-local acceptance, pause state and bounded run logs beside `agent.sqlite`.
The runner reads `schedule` and writes changed `schedule_status` fields without rewriting the definition.
`crates/core/src/templates.rs` creates template pages with caller-chosen deterministic ids and reports existing pages without changing them.
`crates/core/src/nodes.rs` maintains deterministic node records in the hidden Nodes collection.
`crates/hub/src/lib.rs` owns the hub scheduler independently of model providers.
`crates/hub/src/config.rs` exposes the default-on schedules runner setting and its environment override.
`crates/agent/src/rpc.rs` exposes state, acceptance, pause, manual execution, preview and desktop availability methods.
`app/rust/src/api/vault.rs` exposes vault-owned local scheduling and portable preview through the `app/lib/core/vault.dart` wrappers. `app/rust/src/api/agent.rs` retains hub routing.
`app/rust/src/api/clock.rs` exposes the device's IANA time-zone name through jiff's system-zone lookup.
`app/lib/addons/schedule/` shows the schedule page with a status band under the title; hub runners are called through the hub's agent connection, and other desktops are shown from the vault and never called ([schedule add-on](../addons/schedule.md)).
The vault-open wrappers receive the app-data directory from `app/lib/shell/boot.dart`, preserving the existing scheduler state location. The "Run schedules on this device" setting is registered by the schedule add-on under Settings > Add-ons on desktop; boot starts the local runner when it is on.
A desktop with the runner off publishes no node record; an existing record is marked `available: false`.
The live-queries lane's generic unsupported card is not yet on this branch; older clients show the existing "Unsupported note type" placeholder.

## Decisions

- 2026-10-13: synced content never runs anything on its own; the runner must accept a schedule first.
- 2026-10-15: each schedule names one runner device, and the hub is the default.
- 2026-10-15: the runner accepts a schedule once, pinned to a hash of its runner, tools and steps; a synced edit to any of them pauses it until accepted again.
- 2026-10-15: no Bypass; each schedule lists the tools it may use, and unattended runs use exactly those and never prompt.
- 2026-10-21 (integration spec phase 5): `tools` picks one of the built-in tool sets (Read, Write, Everything); Everything is always offered.
- 2026-10-15: for a recurring schedule, `at` is only the anchor and counts as a run only if it matches the rule.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Product > Scheduled tasks).
