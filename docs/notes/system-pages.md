# System pages, feedback and trash

Some pages carry a role. The app and the scheduler use them for their own data. Ordinary notes never need them, and an agent should not add children to them by hand except through the tools named here.

## Roles

Each role is set on one page. The page is found by its role, not its title.

<!-- check: system-roles -->
- `journal`: the Journal database. Holds one row per day note. See `day-notes`.
- `calendars`: a hidden page for calendar settings, stored as ordinary synced data. The app writes it.
- `nodes`: a hidden collection of runner device records, one page of type node per device. The scheduler writes it.
- `feedback`: a hidden page. Each report is a child page. See `feedback_submit` below.
<!-- /check -->

The `calendars`, `nodes` and `feedback` pages are hidden from page lists, search, counts and the task views. Feedback reports are also left out of global search. The Journal is not hidden.

## Feedback

`feedback_submit` files a report for the app's developers on the hidden Feedback page. Use it when:

- a vault tool fails unexpectedly,
- the user complains about Scratch, sync or the vault, or
- the user asks for a report.

Do not file a second report for an issue already filed in the same session.

Arguments:

- `summary` (required): one plain line saying what is wrong or wanted. A line break is an error.
- `details`: optional markdown with more detail.
- `kind`: `bug` (the default), `complaint`, `idea` or `other`.
- `page_id`: optional, the page the report is about.
- `tool`: optional, the tool name the report is about.
- `error`: optional, the error message seen.

```json
{ "summary": "edit_page append fails on a page with a table", "kind": "bug", "tool": "edit_page", "error": "op 1 failed: ..." }
```

The report becomes a child page titled with the date, time and the word `agent`. It has the summary as its first paragraph, the details as markdown, and a code block with diagnostics: the kind, the source, the server and version, the transport, the page id, the tool and the error. Credential values are redacted before storing: Bearer values, `token`, `key`, `secret` and `password` parameters, and 64-digit hex tokens.

The tool returns JSON with the new page's `id` and `title`.

## Trash

`trash_page` moves a page and its sub-pages to the trash. The app can restore them. Trashed pages do not show in search or lists, and do not count as link targets.

Do not use `trash_page` to remove a page the user has not asked to remove.

## Writing to system pages

- Calendar settings are written by the app. Do not edit the `calendars` page.
- Node records are written by the scheduler. Do not edit the `nodes` collection.
- Day notes are written with `daily_note` and `edit_page`. See `day-notes`.
- Reports are written with `feedback_submit`. Do not edit the Feedback page.

## Summary

- A role marks a system page, and the app finds that page by its role.
- Hidden system pages stay out of page lists, search and the task views.
- File a report with `feedback_submit` once per issue, and only when a tool fails or the user asks.
- `trash_page` moves a page to the trash; use it only for a page the user asked to remove.

For templates, see `templates`. For the Memory pages, see `memory`.
