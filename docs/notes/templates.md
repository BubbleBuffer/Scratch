# Templates

A template is a page that new pages are copied from. The app sets templates up; the tools here only use them. No tool creates a template or chooses a default template, so set those in the app.

## Terms

- A **template** is a page kept as a source for copies. It is not an ordinary page in lists or search.
- A **default template** is the template a parent hands to new children.
- A **child** is a page under a page, or a row under a database. Templates copy into children and rows only.

## What a template is

- A template is a child page of a page or a database, marked as a template in the app. A top-level page cannot be a template.
- A page can have a default template. A new child of that page starts as a copy of the template.
- A database can have a default template too. A new row starts as a copy of it, the same way a child page does.

Templates are kept out of normal listings. They do not show up in child lists, page lists, search results or queries.

## What a new page gets

When a page is created under a parent that has a default template, the new page copies these from the template:

- its blocks, which replace the blank body the page would otherwise have,
- its properties, but only the ones the new page has not set,
- its icon and cover, if the new page has none.

The title is not copied, and the template's sub-pages are not copied.

## Order of creation with create_page

`create_page` does its steps in this order:

1. Create the page under `parent`. The parent's default template is applied here.
2. Set `properties`, which win over the template's values.
3. Append `markdown` after whatever the template put in the body.

So a page created with `markdown` keeps the template's blocks and adds the text after them.

```json
{ "title": "Meeting 2026-10-19", "parent": "<parent id>", "markdown": "- Next step" }
```

## Checking what was copied

Use `read_page` on the new page to see the blocks and properties it got. With `block_ids` set, `read_page` shows each top-level block's id, which `edit_page` needs for `replace` and `delete`.

## Writing under a template

When the parent has a default template, put only the new text in `markdown`. Do not repeat headings or blocks the template already has, or the page ends up with two copies.

## Other places

- Each new page is its own copy, so editing the new page does not change the template.
- Day notes ignore templates. A day note is always created blank with its `Tasks` and `Log` toggles. See `day-notes`.
- Templates apply only when a page or row is created. `edit_page` does not apply one.

## Summary

- Templates are set in the app. Tools use them when a page or row is created.
- A new page gets the body, properties and icon of its parent's default template. Properties given in the call win over the template's.
- Check the result with `read_page`.

## Hidden system pages

Calendars, the Nodes collection and the Feedback page are hidden system pages, not templates. See `system-pages`.
