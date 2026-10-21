# Properties and databases

A database is a page whose child pages are rows. Each row has properties, which are typed values set by name. Tools set them through `properties` on `create_page` (inside a database), `upsert_row`, and `edit_page` with op `set_props`.

## Writing values

Pass properties as an object keyed by the property's name, not its internal key:

```json
{ "Status": "Doing", "Due": "2026-10-20", "Tags": ["home", "errands"] }
```

- A name that matches no property is an error that lists the properties. `Title` (or `name`) sets the title; `Created` and `Updated` are read-only.
- Set a value to `null` or `""` to clear it.
- `select` and `multi_select` create an option when the name is new. That is the only schema change a property write makes.
- On a plain page with no database schema, only text, numbers, booleans and `null` are accepted.

## Property types

Each property type and how a tool sets it. Read-only types cannot be set.

<!-- check: property-types -->
- `text`: a string. A number or boolean is written as text.
- `number`: a number, or a numeric string.
- `date`: an ISO date string such as `2026-10-20`, or `{start, end}` for a range. Read as UTC.
- `checkbox`: a boolean. The strings `true`, `yes`, `1` and `x` are true, other strings are false. A nonzero number is true.
- `select`: an option name. A new name creates the option.
- `multi_select`: an array of names, or a comma-separated string. New names create options.
- `status`: an existing option only. An unknown name is an error that lists the options.
- `relation`: page ids or page titles. Titles are resolved to pages.
- `url`: a string.
- `email`: a string.
- `phone`: a string.
- `created_time`: computed by the app. Read-only; a tool cannot set it.
- `updated_time`: computed by the app. Read-only; a tool cannot set it.
- `files`: not settable over MCP yet.
- `formula`: not settable over MCP yet.
- `rollup`: not settable over MCP yet.
<!-- /check -->

Setting `files`, `formula` or `rollup` returns the error "this property type cannot be set over MCP yet".

## Querying a database

A database's rows are the pages inside it, so a query for rows is a `search_notes` query with `in:` set to the database. `-type:database` leaves out the database page itself:

```
in:[[Projects]] -type:database Status:doing sort:Due
```

- A property filter is `<property>:<value>`, such as `Status:doing`. Commas mean any of (`Status:doing,review`), a `-` in front excludes (`-Status:Done`), and a value with spaces is quoted (`Status:"In progress"`).
- `Status:none` finds rows where the property is empty.
- Numbers and dates take comparisons, such as `>5` or `<today`.
- `sort:<property>,desc` reverses the order.
- `limit:` sets the number of rows (default 20, at most 200). Page with the `offset` argument.
- Search has no `OR` keyword and no parentheses. See the Search and queries help topic for the full query language.

## Adding and updating rows

`upsert_row` creates a row when `row` is not given, and updates the row when it is. Its arguments are `database`, `row`, `title`, `properties` and `markdown`. `title` sets the row's title. `markdown` becomes the row's content, or replaces it when the row is updated.

## Moving pages and rows

`move_page` moves pages, with their sub-pages, under a new `parent` (omit it for the top level) or right after a sibling named by `after`. Moving a row into or out of a database is a plain move, as in the app. A page cannot move into its own subtree.

## Database blocks on a page

A page shows a database inside itself with a database block: `![[Title]] <!-- database -->` on its own line. See `markdown` for block syntax and `links` for the link form.

## Dates and tasks

Date properties are plain dates. A date in running text is only a date when it is written as a `time` element; see `markdown`. Dated to-dos and rows are listed by item queries such as `is:todo,row date:today`, and `update_task` moves a row's date. See the Tasks and dates help topic for the views and the time rules.
