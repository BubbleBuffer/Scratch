# Tasks and dates

`search_notes` reads planning data from the same local index as the app's calendar. It includes to-do blocks, task notes, dated database rows and date mentions in ordinary text, plus undated to-dos and task notes. `update_task` changes a to-do, task note or database row by its returned reference.

## Listing items with search_notes

A query with an item operator (`is:`, `done:`, `assignee:` or `calendar:`) returns items instead of pages. The same query language applies as for page search (see the Search and queries help topic): `date:` filters the item's date, and page operators such as `in:` filter the page the item is on.

| Query | Lists |
| --- | --- |
| `is:todo,task,row,mention date:today` | everything dated today |
| `is:todo,task date:<now done:no` | open to-dos and task notes due before now (all-day ones before today): the overdue ones. Past events and mentions are left out, since they can't be ticked off |
| `is:todo,task,row date:2026-10-01..2026-10-07` | items dated in that range |
| `is:todo undated` | to-dos without a date |
| `is:todo done:no in:[[My wiki]]` | open to-dos on that page and below it |
| `is:task assignee:agent done:no` | open task notes assigned to the agent |
| `is:task calendar:Work date:today..+7d` | task notes in the Work category for the next week |

Commas mean any of, so `is:todo,task` takes both. Ordinary date mentions are returned only when `mention` is listed in `is:`.

## Results and dates

Each item line shows its block id, `[ ]` or `[x]`, its date and the reference `update_task` takes. An undated item has no date. Results are capped like other search results: narrow the query, page with `offset`, or raise `limit:`.

References are stable:

- To-do: `block:<page id>/<block id>`.
- Task note: `note:<page id>`.
- Database row: `row:<database id>/<row id>`.
- Ordinary date mention: `mention:<page id>/<block id>`; read-only, not accepted for task mutation.

Pass the reference from an item line as `update_task`'s `item` argument. Use `read_page` with `block_ids: true` for surrounding text and editable IDs, including nested blocks.

Dates with a time use the host's local timezone. All-day dates keep their calendar day. **Done items in a range use the item's scheduled date**, not its completion time. To-dos have no separate completion timestamp: the app's `done_since` view uses the page's last-update time as a proxy, which can change on unrelated edits.

The app's Today and overdue views keep their old task-view eligibility. An open timed item is overdue after its time (or duration) has passed; an open all-day item is overdue after its date (or end date). Done items, ordinary mentions and undated items are not overdue. Visibility preferences do not limit the tool's queries.

## Dating a to-do in markdown

A to-do's first valid date mention supplies its date:

```markdown
- [ ] Renew passport <time datetime="2026-11-02">2026-11-02</time>
- [ ] Dentist <time datetime="2026-10-22T09:30">2026-10-22 09:30</time>
```

A date without a time is all day. A time in a markdown `datetime` is read as UTC; `update_task` reads times in the host's local timezone. Prefer `update_task` for a time you want in local time. A checkbox written `- [x]` is done. A to-do without a date is still returned by `is:todo undated`.

Database rows get their dates from the database's configured date property. The agenda role is a calendar setting only: search returns every dated row, whatever its database's agenda role. Ordinary text with a valid date mention is returned with kind `mention`. Task notes use `type_data.tasks`, including their status, duration, calendar and assignment.

## Changing an item with update_task

Supply `item` and at least one of `due` or `done`:

```json
{ "item": "note:<page id>", "due": "2026-10-23 14:00", "done": false }
```

- `due`: `YYYY-MM-DD` (all day) or `YYYY-MM-DD HH:MM` (host local time).
- `done`: `true` or `false`.

Both may be supplied together. A to-do changes its date mention and checkbox. A task note changes its date or status (`done` when completed, `todo` when reopened), preserving its other task fields. A row changes its date or completion property. Ordinary date mentions cannot be changed with `update_task`; edit their blocks with `edit_page`. `update_task` never starts an agent run: a task assigned to `agent` runs only when the app assigns it.

The app's existing Today, calendar, overdue and completion views keep their behavior. Undated rows are held only in the rebuildable local index; this adds no page-document or sync format.
