# Day notes

A day note is the page for one calendar day. The app keeps them in a Journal database: one row per day, with a Date property. Use `daily_note` to get the note for a day, then write into it with `edit_page`.

## Getting a day's note

`daily_note` takes an optional `date` as an ISO day, `YYYY-MM-DD`. Without it, the day is today in the host's local timezone.

It returns JSON with the note's `id`, its `title` and `created`. `created` is true only on the call that made the page.

- The first call for a day creates the note. Later calls return the same note.
- A new note has two empty toggle blocks, `Tasks` and `Log`.
- The title is the weekday, the day, the month and the year, such as `Monday 19 October 2026`. The day has no leading zero.
- Templates do not apply. A day note is always created blank with the two toggles.

Write into the note with `edit_page`, using `page` set to the returned `id` and op `append`. Use `after_block` to put the text after a given block, for example after the `Log` toggle.

```json
{ "page": "<id from daily_note>", "op": "append", "markdown": "- Called the bank" }
```

## Tasks and the Log

The `Tasks` and `Log` toggles are ordinary toggle blocks. Put to-dos under `Tasks` and notes under `Log` by appending with `after_block` set to the toggle, or by writing the children under it with the markdown indentation rules in `markdown`.

A to-do with a date mention on it appears in the agenda views on the day of that date. See the Tasks and dates help topic.

## Journal and the task views

The Journal is a database page with a role of journal. Each day note is a row whose Date property is that day. A day is found by its Date, not by its title, so a renamed note is still found.

The Journal is left out of the task views. Its rows do not show up as dated items in the agenda.

## A to-do with a date

A to-do is dated by a date mention in its text. In a day note, this puts it on that day in the agenda:

```
- [ ] Call the bank <time datetime="2026-10-20">2026-10-20</time>
```

Move it later with `update_task`, passing `item` (the reference from its `search_notes` item line) and `due`.

## Writing a day note by hand

Call `daily_note` rather than creating a page with the day's title by hand, so the note is the one `daily_note` and the Journal find.

- To find a day's note without creating it, read the Journal's rows with `search_notes`, for example `in:[[Journal]] -type:database date:2026-10-20`.
- To add a dated item for another day, write a to-do with a date mention on any page. The agenda finds it by date, not by where it lives.

## Reading the Journal

To list recent days, search the Journal's rows, sorted by its Date property, newest first:

```
in:[[Journal]] -type:database sort:Date,desc limit:14
```

Find the Journal's id with `search_notes` and the query `Journal`.

Read a day note with `read_page` on its id. Long pages are paginated by top-level block: use `offset` and `limit`.

## Limits

- A day has one day note. `daily_note` returns the existing note when there is one.
- Templates are not applied to day notes.
