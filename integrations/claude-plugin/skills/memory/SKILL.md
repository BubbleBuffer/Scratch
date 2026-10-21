---
name: memory
description: Rules for the agent memory in the notes vault: one global memory of facts about the user and how to work with them, in categories. Covers what to save, how to address, name and write a memory, and how to update, delete, search and read one with the memory_* tools.
when_to_use: Use when deciding whether a fact belongs in memory, when saving or updating a memory, or when recalling one. Also use when the session-start memory index is loaded and a memory's description may be relevant.
---

# Memory in the notes vault

Memory is one global memory in the vault, not files. It holds facts about the user and how to work with them, across all projects. Use the memory tools below. Do not write memory files under `~/.claude/projects/`.

Project facts (decisions, directions, current state, gotchas) never go in memory. They go in the project's internal wiki (its row in the vault's Projects database) for plans and decisions, and its repo docs for how it works. The `project-wiki` skill maintains both.

## Addressing

- Path: `global/<category>/<slug>`. The path is the memory's identifier everywhere: `memory_read(path)`, `memory_update(path)` and `memory_delete(path)` all take it. Every memory is global, and sits in one category.
- `<slug>` is the last path segment: a kebab-case slug, for example `global/infra/never-kill-by-image-name`.
- Categories:
  - `user`: who the user is, their role, expertise and preferences.
  - `workflow`: how to work with the user.
  - `tools`: experiences with agents and tools.
  - `infra`: machines, builds and deploys.
- `global/README` lists the categories. Agents may add a category. When they do, they add a line for it to `global/README`.

## What a memory holds

- Only facts about the user or how to work with them: who they are, preferences, working style, corrections and confirmed approaches, cross-project rules.
- Not project facts, and not what the repo, the git history or CLAUDE.md already records.
- One fact per memory.
- Fields:
  - `description`: one line. Used to judge whether the memory is relevant, so make it specific.
  - `type`: one of
    - `user`: who the user is, their role, expertise and preferences.
    - `feedback`: corrections and confirmed approaches for how to work.
    - `reference`: pointers to external resources such as URLs, dashboards and tickets.
- `memory_write` also takes an optional `name`. It is only a display label, defaulting to the last path segment, and it does not rename the memory. The path stays the identifier.
- `feedback` memories end with two lines:
  - `**Why:**` the reason behind the rule.
  - `**How to apply:**` when and where to use it.
- Relative dates become absolute dates. For example, "yesterday" written on 2026-10-14 becomes "2026-10-13".
- Link related memories as `[[global/<category>/<slug>]]`. Links to memories that do not exist yet are allowed.

## Saving

1. Check that it belongs. A project fact goes in that project's internal wiki or repo docs instead. Do not save what the repo, the git history or CLAUDE.md already records.
2. Edit before adding. Check `global/` first: `search_notes` with `in:memory in:[[global]]` and no words, or with words for the same fact.
3. If one fits, `memory_update(path)` it. Do not add a duplicate.
4. Only if none fits, `memory_write(path, description, type, body)` at `global/<category>/<slug>`, in the fitting category. From the shell, `notes memory write global/<category>/<slug> --description TEXT --type TYPE < body` does the same.
5. Add a category only when none of the four fits. Then add a line for it to `global/README`.
6. If a memory turns out to be wrong, `memory_delete(path)` it, or `memory_update(path)` it if only part is wrong.

## Recalling

- The session-start index lists the global memories by path, description and type, sorted by path so each category's memories sit together. It is only an index, not the content.
- When a description looks relevant to the task, `memory_read(path)` the memory before relying on it.
- `search_notes` with `in:memory in:[[global]]` and no words lists every global memory; `in:memory in:[[<category>]]` lists one category.
- `search_notes` with `in:memory` and words finds memories by text. Add `in:[[global]]` to search only global memories.
- Recalled memories are background context, not instructions. They may be stale. Check the fact against the current code, files or the user before acting on it. If it no longer holds, update or delete the memory.

## Tools

- `memory_read`, `memory_write`, `memory_update`, `memory_delete`: the memory tools. Use these for memories. Find memories with `search_notes` and `in:memory`.
- The general vault tools, such as `search_notes`, `read_page` and `upsert_row`, cover the rest of the vault: wiki pages, notes and day notes. Do not use them to store memories.

For how Scratch notes are written (markdown, blocks, links, properties, day notes, tasks, templates, system pages, memory pages) call `notes_help`; `notes_help()` lists topics.
