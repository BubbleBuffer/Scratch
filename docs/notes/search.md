# Search and queries

`search_notes` takes one query string. The same language drives the app's search box and its live query cards; the app ignores the output-size operators, and a saved card keeps them in its query.

## Writing a query

- Words are ANDed, and the last word also matches as a prefix. `"exact phrase"` matches a phrase. `-word` excludes a word.
- `key:value` is an operator. Prefix it with `-` to negate it (`-tag:work`). Commas mean any of (`tag:work,home`); repeating a key means all of (`tag:work tag:urgent`).
- Keys ignore case, spaces, `_` and `-`. Quote keys or values that contain spaces: `"due date":today`, `in:"Project plan"`.
- A word with a colon whose key is not an operator or property is plain text. A known operator with an unreadable value is left out, with a warning in the result.
- A property with the same name as one of the newer operators (`is`, `done`, `assignee`, `calendar`, `context`, `full`, `limit`) keeps its meaning; quote the key to make that explicit. In a query with `is:`, `done:`, `assignee:` and `calendar:` always filter items.
- There is no `OR` keyword and there are no parentheses.
- Trash and templates are never searched.

## Dates

- Absolute: `2026`, `2026-10`, `2026-10-10`. A partial date covers the whole period.
- Named: `today`, `yesterday`, `tomorrow`, `week`, `lastweek`, `month`, `lastmonth`, `year`.
- Relative: `7d`, `2w`, `3m`, `1y` mean the last N; `+7d` means from now through the next N.
- Ranges and comparisons: `2026-09..2026-10`, `today..+7d`, `>2026-09`, `<today`, `>=`, `<=`.
- `now` is the current minute, for comparisons: `date:<now` matches timed dates before now and all-day dates before today.
- Day boundaries use the vault host's local time zone.

## Operators

The table is generated from the operator table in the code.

<!-- generated: operators -->
| Operator | Values | Meaning | Example | Kind |
|---|---|---|---|---|
| `created:` | `DATE` | page created in that period | `created:week` | filter |
| `updated:` | `DATE` | page edited in that period | `updated:7d` | filter |
| `date:` | `DATE` | created, updated or any date property; with item operators, the item's date | `date:today..+7d` | filter |
| `tag:` | `name` | tagged (tag:work also matches work/meetings); #name is the same | `tag:work,home` | filter |
| `type:` | `document\|database\|chat` | page type | `type:database` | filter |
| `in:` | `[[Page]]\|"Title"\|memory\|path` | that page and its subtree; a path glob (* one level, ** any depth) matches only the pages it names; memory is the agent memory pages | `in:Work/*` | filter |
| `title:` | `word` | the title contains it | `title:plan` | filter |
| `links:` | `[[Page]]` | links to that page | `links:[[Roadmap]]` | filter |
| `from:` | `[[Page]]` | linked from that page | `from:[[Inbox]]` | filter |
| `has:` | `link\|attachment\|<property>` | has a link, an attachment or a non-empty property | `has:attachment` | filter |
| `sort:` | `updated\|created\|title\|<property>` | result order | `sort:title` | filter |
| `is:` | `todo\|task\|row\|mention` | item results: to-do blocks, task notes, dated database rows, date mentions | `is:todo` | filter |
| `done:` | `yes\|no` | items that are (not) done; implies item results | `done:no` | filter |
| `assignee:` | `me\|agent` | task notes assigned to you or the agent; implies item results | `assignee:agent` | filter |
| `calendar:` | `name` | task notes in that calendar category; implies item results | `calendar:Work` | filter |
| `undated` | (none) | with item operators: only items without a date (otherwise a plain word) | `is:todo undated` | filter |
| `context:` | `N` | blocks shown around each match (default 2, max 20) | `context:0` | output size |
| `full:` | `CHARS` | show a page whole when it is at most this long (default 1500, 0 never) | `full:4000` | output size |
| `limit:` | `N` | hits per call (default 20, max 200) | `limit:50` | output size |
| `<property>` | `value` | text contains; select option; number 5, >5 or 1..5; checkbox yes/no; dates use DATE; none means empty | `status:doing` | filter |
<!-- /generated -->

`tag:` matches only tags: the tags property and select options. A text property named Tags is not a tag. Search it by its property name instead, as `Tags:<text>`.

## In: pages and paths

`in:` takes a page, or a path of page titles. A page's path is its ancestor titles joined by `/`.

- `in:Work` and `in:[[Work]]` match Work and everything below it.
- `*` matches within one segment. `**` matches any depth.
- `in:Work/*` matches Work's direct children, without Work.
- `in:Work/**` matches all of Work's descendants, without Work.
- `in:**/Meetings` matches any page titled Meetings, anywhere.
- `in:[[Work]]/*` is the same as `in:Work/*`.
- A path without a glob matches that page and its subtree. `in:Work/Q3` matches Q3 and the pages below it.
- A path with a glob matches exactly the pages it names.
- Matching ignores case. Quote a segment that contains spaces: `in:Work/"Q3 notes"/*`.
- An exact page title wins over the path reading. A page titled `a/b` is still `in:a/b`.

"This page only" is `in:Work -in:Work/**`. It keeps Work and drops everything below it.

- `is:todo done:no in:Work/**` lists open to-dos anywhere under Work.
- `in:**/Meetings sort:updated` lists every Meetings page, newest first.

A live query card with a path pattern updates on every change. A rename above a page can change which pages match.

## Items: to-dos, task notes, rows and mentions

A query with `is:`, `done:`, `assignee:` or `calendar:` returns items instead of pages. Each item line shows its block id, `[ ]` or `[x]`, its date and the reference `update_task` takes. Page operators (`in:`, `tag:`, properties) filter the item's page, words match the item's own text, and `date:` filters the item's date.

- `is:todo,task date:<now done:no` lists overdue to-dos and task notes (timed ones due earlier today included). Past events and mentions are left out, since they can't be ticked off.
- `is:todo,task date:today..+7d` lists what is due this week.
- `is:task assignee:agent done:no` lists open task notes for the agent.
- `is:todo undated in:[[Inbox]]` lists to-dos without a date under Inbox.

## Databases

A database's rows are the pages inside it: `in:[[Reading list]] -type:database status:reading sort:title`. Properties filter by type; `sort:<property>,desc` reverses the order.

The agenda role is a calendar setting only. Search ignores it: every dated row is a dated search result, whatever its database's agenda role.

## Agent memory

`in:memory` searches the agent memory pages; add words to search their text. Memories are read and written by path with the `memory_read`, `memory_write`, `memory_update` and `memory_delete` tools.

## Results

Each page hit gives the title, id and path, then the matching blocks with `context:N` blocks around them; each block line starts with the block id `edit_page` takes. A page of at most `full:CHARS` characters is shown whole. A query with only operators shows the start of each page.

One result stays under a fixed size. When it would be larger, previews shrink first (no context, then short lines), and only then are hits left out; the result says what it trimmed. To see more, narrow the query, page with `offset`, raise `limit:N`, or `read_page` a hit.

`read_page` with an id or title that doesn't exist returns a title search instead, labelled as search results. It never opens a near match by itself.
