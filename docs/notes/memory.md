# Memory pages

Memories are short notes an agent keeps for later sessions: facts about the user, how to work with them, and general reference. They live in the vault under one top-level `Memory` database, not as files. Use the `memory_*` tools; they are the only way to read or write them.

## Term rule

A memory's identifier is its `path`. Use `path` everywhere: when writing, reading, updating, deleting and searching. `name` is only a display label. It defaults to the last segment of the path, and changing it does not move the memory.

## Addresses and layout

- A memory is `project/memory`, or `project/subproject/memory`. The path is the address.
- A project is a database under `Memory`. A subproject is a database under a project. A memory is a row.
- Each segment is 1 to 80 characters: a letter or digit first, then letters, digits, `.`, `_` or `-`.
- `memory_write` creates any missing project or subproject.

## Fields

Each memory has these fields:

- `name`: the display label, as above.
- `description`: one line saying what the memory is for.
- `type`: one of `user`, `feedback`, `project` or `reference`.
- `source`: text, where the memory came from.

A memory can also carry extra fields that its database defines. `memory_read` shows them, and `memory_update` can set them; `null` clears one.

## Writing a memory

`memory_write` creates or replaces a memory. It needs `path`, `description`, `type` and `body`. `name` is optional.

```json
{
  "path": "global/workflow/short-replies",
  "description": "The user wants short replies without a summary at the end",
  "type": "feedback",
  "body": "Keep replies short.\n\n**Why:** stated 2026-10-19.\n\n**How to apply:** no closing recap."
}
```

- `body` replaces the whole body.
- Fields not given are replaced as well: any extra field is set to `null`.
- The memory is created if the path is new, and replaced if it exists.

## Updating a memory

`memory_update` changes only what it is given. `path` is required.

- `body` replaces the body. `append` adds a paragraph at the end instead.
- `description`, `type` and `name` change those fields.
- `fields` sets existing fields of the schema. `null` clears one.
- `new_path` moves the memory to another address. Its project and subproject must already exist. The move is refused if the destination is taken or the schema does not fit. A missing memory is an error.

## Reading and searching

- `memory_read` takes `path` and returns the path, name, description, type, extra fields, last modified time and body. It returns no page ids.
- Search memories with `search_notes` and `in:memory`, followed by the words to find, such as `in:memory deploy key`. Results give each memory's path and the matching text.
- `in:memory` with no words lists the memories. Add `in:[[project]]` to list one project's memories; its subprojects are inside its page and are included.

## Deleting

`memory_delete` takes `path` and moves the memory to the trash. The app can restore it. See `system-pages` for the trash.

## Tips

- Check for an existing memory with `search_notes` and `in:memory` before writing a new one. Edit the match with `memory_update`.
- Write absolute dates, such as `2026-10-19`, not "yesterday".
- Read a memory with `memory_read` before relying on it. Recalled memories are background, and the named files or functions in them can be stale.
- Link related memories in the body as [[project/memory]].

