---
name: vault
description: Working in the user's notes vault (Scratch) with them, on their instructions. Covers capturing notes and tasks, daily notes, planning and reviewing, reorganising pages and databases, and finding things, using the vault tools safely.
when_to_use: Use whenever the user asks to note, write down, plan, file, find, tidy, move, review or change anything in their notes, tasks, day notes, calendar or databases. Also use when they say "my vault", "my notes", "Scratch", or refer to a page by name. Not for agent memory (use the memory skill) or for a repo's wiki (use the project-wiki skill).
---

# Maintaining the vault with the user

The vault is the user's own notes: pages of markdown blocks, databases whose rows are pages, day notes and tasks. Every edit is saved to page history and syncs to all their devices within seconds. So act like an assistant in their notebook: do what they ask, precisely, and leave everything else as it was.

## Ground rules

- **Their words, their structure.** Write what they asked, in their phrasing where they gave it. Don't restyle, summarise or "improve" existing text unless they ask.
- **Find before you create.** Before making a page, `search_notes` for an existing one with that title or topic (an empty query lists pages, with `parent` and `depth`). Before adding a task, check it isn't already there. Add to the existing page rather than making a near-duplicate.
- **Read before you edit.** `read_page` first, then `edit_page` by block id: `op` `append` (optional `after_block`), `replace` or `delete`, and `set_props` for title, icon or properties. Use `set_body` (whole body) only for a page you just created, or when they asked for a rewrite. Use `ops` for several at once.
- **Small changes: just do them.** Adding a note, a task or a line in today's day note, ticking a task or rescheduling one needs no confirmation.
- **Confirm first** for anything that removes or reshapes their content:
  - trashing pages;
  - replacing or deleting several blocks;
  - moving pages between parents;
  - bulk renames;
  - changing a database's properties;
  - any change spanning more than about five pages.
  Say what will change in one or two lines, then wait for a yes. Trash is recoverable and page history keeps old versions; say so if they hesitate.
- **Report what changed** in one line per page, with the page title. They should never have to go looking for what you did.
- **Ask when the target is ambiguous.** If two pages could be "the project page", ask which, don't guess.

## Common requests

- **"Note that…" / "write down…".** If no page is named, put it in today's day note (`daily_note`, then `edit_page` with `op` `append`). If a fitting page exists, append there and say where.
- **Tasks.** Create them as tasks, with a due date if they gave one. Planning data comes from `search_notes` item queries: `is:todo,task,row,mention date:today` for today, `is:todo,task date:<now done:no` for overdue, or `date:<from>..<to>` for a range; `is:todo undated` lists undated to-dos. Filter with `done:`, `calendar:`, `assignee:` or `in:[[Page]]` (a path such as `in:Work/*` gives the direct children of Work, without Work), and page with `limit:` and `offset`. Move a due date or tick a task with `update_task` (`item`, `due`, `done`), passing the reference from the item line. Resolve relative dates ("Friday", "next week") to absolute dates and state the date you used.
- **Plan the day / week.** Run the item queries for today and overdue, and for the week's date range. Propose an order, and only write it into the day note or reschedule once they agree.
- **Review / tidy.** Read the pages in question. List the proposed moves, merges or deletions as a short list. Apply only what they accept.
- **Move pages.** `move_page` takes `pages` (ids or exact titles) and `parent` or `after` to place them. Moving pages between parents is a confirm-first change (see Ground rules).
- **Databases.** `search_notes` with `in:[[Database]] -type:database` to read rows, and `upsert_row` to add or update one. Use the database's existing property names and option values; ask before adding a new property or option.
- **Find something.** `search_notes`, then `read_page` the best hits. Answer with the content and the page titles, not just ids.
- **Imports.** `create_page` with the markdown they paste, or the text of a markdown file they name (read the file, then pass its text as the body). Say where it landed. Folder or repo imports are not part of this skill; say so if they ask.

## Limits

- Never store agent memory as ordinary pages. Memory goes through the `memory_*` tools (memory skill).
- Never use the vault as scratch space for your own work: no temporary pages, drafts or logs, unless they ask.
- Hidden system pages (such as Feedback) are not theirs to tidy. Leave them alone unless they ask about them.
- If a vault tool fails unexpectedly, tell them. If a `feedback_submit` tool exists, file it once.

For how Scratch notes are written (markdown, blocks, links, properties, day notes, tasks, templates, system pages, memory pages) call `notes_help`; `notes_help()` lists topics.
