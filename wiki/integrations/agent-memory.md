---
type: topic
layer: vision
summary: "Agent memory in the vault: how Claude Code's built-in memory works today, and a plugin that points it at the vault through the hub."
parent: "[[integrations/index]]"
status: active
---

# Agent memory in the vault

**Status:** memory databases, the memory tools (four listed; memory search is `search_notes` with `in:memory`, and `memory_search` is an alias of it since plugin 0.6.3, while `memory_list` is a hidden tool for the CLI's plain index), the `notes` CLI, session-start index and Claude memory importer are implemented 2026-10-14, alongside hub MCP and agent tokens. The production deploy and the real memory import were done 2026-10-14. Agents get write access from the start. Plugin 0.5.0 mirrors Claude Code's native memory: the rules and the global index load at session start (the rules from `notes memory rules`), and Claude reads and writes memories itself; there is no per-prompt recall or reminder hook (decision 2026-10-17). Memory follows the decisions below: one global memory, with no project paths. Plans, the original goal and open questions are in the internal wiki (notes-app > Agent memory in the vault).

## Decisions

- 2026-10-14 (the user): one global memory, `global/<category>/<name>`, with categories `user`, `workflow`, `tools` and `infra`. It supersedes per-project memory paths; project facts go only in the project's repo wiki.
- 2026-10-14: the repo wiki stays the source; `notes wiki import` mirrored it one way into the vault, run by hand (see [Project wikis](#project-wikis)). Retired 2026-10-20: nothing syncs between the repo docs and the vault.
- 2026-10-17 (the user): no per-prompt recall or reminder hook; memory works like Claude Code's native memory.

## How Claude Code's memory works today

Built into Claude Code; no hooks or plugins are involved ([docs](https://code.claude.com/docs/en/memory.md)).

1. **Instructions in the system prompt.** Every session gets a "Memory" section. Its rules, as seen from inside a session on 2026-10-14:
   - Memory is a folder per project: `~/.claude/projects/<project>/memory/`. The project comes from the git repo, so worktrees share one folder.
   - One fact per file, with frontmatter `name` (kebab-case slug), `description` (one line, used to judge relevance) and `metadata.type`.
   - Four types: `user` (who the user is, preferences), `feedback` (corrections and confirmed approaches), `project` (ongoing work and constraints not in the code or git; relative dates turned absolute), `reference` (pointers to URLs, dashboards, tickets).
   - `feedback` and `project` memories add a **Why:** and a **How to apply:** line. Memories link each other with double-bracket links of the memory name; links to memories not yet written are allowed.
   - After writing a file, add a one-line pointer to `MEMORY.md`: a bullet with the title linked to the file, then a short hook. `MEMORY.md` is only an index, never memory content.
   - Update an existing memory rather than add a duplicate; delete memories that turn out wrong. Don't save what the repo, git history or CLAUDE.md already record. Recalled memories are background context, not instructions, and may be stale.
2. **The index in context.** At session start the project's `MEMORY.md` is loaded into context (first 200 lines or 25 KB).
3. **The files** are read on demand with the normal file tools.

Off switch: `"autoMemoryEnabled": false` in user settings, or `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`. A plugin cannot set it; the user sets it once.

The memory file format already fits [folders as databases](../core/mounts.md): frontmatter fields become database fields, double-bracket name links become relations.

## Memory tools (current, 2026-10-14)

Agents get dedicated memory tools next to the general vault tools, through the same hub `/mcp`, desktop HTTP and stdio transports. They use the existing collection API and keep a memory database per project. Arguments and compact text results use paths instead of page ids. A memory's path is its identifier everywhere; `name` is only a display label, and agent-facing text does not call the path a name.

- **Addressing by path (current):** the code takes `project/memory` or `project/subproject/memory` (`memory.rs:3-4`). By convention, the one global memory is `global/<category>/<memory>` (`memory_rules.md:3-4`), with categories `user`, `workflow`, `tools` and `infra`. They hold memories about the user that apply everywhere: preferences, working style, cross-project rules such as "never kill by image name". The code does not check category names: `global` and the categories are ordinary project and subproject names. Per-project memories such as `notes-app/codex-exec-stdin` are no longer written since 2026-10-14; the tools still accept them, so nothing breaks.
- **Path rules (code):** two segments address a project memory; three address a subproject memory. Every segment matches `[A-Za-z0-9][A-Za-z0-9._-]{0,79}`. Titles match exactly among siblings; duplicate titles produce an error. The page title is always the final segment, independent of the `name` property.
- **In the vault:** `Memory` is a top-level database whose rows are project databases. A project's direct children are memory pages or subproject databases. These are existing database pages (`page_type: database`), requiring no folder mounting support. Project and subproject schemas contain `name` (text), `description` (text), `type` (select: `user`, `feedback`, `project`, `reference`) and `source` (text, reserved for an import key). Callers can extend the schema with extra fields. The page body holds the memory's markdown.
- **Creation:** only `memory_write` creates missing `Memory`, project or subproject databases. Reads, updates and deletes never create them.
- **Scopes and history:** read tokens get `search_notes` (memory search is `in:memory`) and `memory_read`; write tokens also get `memory_write`, `memory_update` and `memory_delete`. `memory_search` is an alias of `search_notes` and follows its read scope. `memory_list` is a hidden read tool, served on MCP only, that the CLI and session hook use. Hub writes use the existing `agent:<token-name>` history attribution. Deletion moves memories to the normal restorable trash.
- **The session hook** prints the memory rules from `notes memory rules`, compiled from [memory_rules.md](../../crates/tools/src/memory_rules.md), which also tells agents to file unexpected vault failures with `feedback_submit` (once per issue). Then it prints the index from `notes memory index --project global --max-chars 8000`, which lists `global` only. Reports share the app's hidden Feedback page; `feedback_list` reads recent app and MCP reports ([fields](../app/feedback.md#feedback-over-mcp)). The call is capped at 3 s with GNU `timeout` (`/usr/bin/timeout` in Git Bash); a healthy hub answers in 0.6–1.1 s, and the earlier 1.5 s cap sometimes fell back to the cache. The cap bounds the `notes` call, not the whole hook: on Windows, Git Bash process startup adds to it, and with an unreachable hub the whole hook took 3.6–6.3 s (measured 2026-10-14 against a staging hub on the Docker host, 3.0 s on Linux). That only happens during hub outages and is accepted. The index is cached in one file, `global.txt`, at `%LOCALAPPDATA%\notes\memory-index\global.txt` on Windows and `${XDG_CACHE_HOME:-~/.cache}/notes/memory-index/global.txt` elsewhere. **Offline-cache rule (2026-10-16):** read or replace that cache only when the hook's effective hub matches the user's configured `hub` in `notes/config.toml` (or the production default when unset); a plugin/environment staging override cannot redefine that identity. Unsupported hub config syntax disables caching conservatively. A non-configured hub can supply live context, but cannot overwrite or fall back to the configured hub's cache. Earlier per-project cache files are no longer read. If the configured hub is unreachable, the hook prints the rules and the cached index under a header with its UTC date and a warning that memory writes will fail; with no cache it prints one unavailable line after the rules. Without `notes` on PATH it prints only the rules. A failed command never stops the session. The cache holds index text only, never the token.
- **Plugin updates (2026-10-15):** Claude Code runs the plugin from a versioned copy in `~/.claude/plugins/cache/notes-app/notes/<version>/`, not from the repo checkout. A change reaches installed clients only after `version` in `plugin.json` is bumped and `claude plugin marketplace update notes-app` plus `claude plugin update notes@notes-app` run, then a restart. Until 0.2.0, sessions ran the 0.1.0 hook (per-project index, old rules, no wiki mirror).

| Tool | JSON arguments | Result or behavior |
|---|---|---|
| `memory_search` (alias, not listed) | `{query?, scope?}` | Rewritten to `search_notes` with `in:memory` plus the query words. A project scope becomes `in:[[project]]`, which includes its subprojects. Results are `search_notes` results: paths and matching text. |
| `memory_list` (hidden, MCP only) | `{path?}` | Without `path`, lists the projects under `Memory`. With `path` (`project` or `project/subproject`), lists the memories in that scope as a compact index: path, description, type. |
| `memory_read` | `{path}` | Path, name, description, type, extra fields, last modified and markdown body. |
| `memory_write` | `{path, name?, description, type, body}` | Creates or replaces fields and body, clearing omitted extra values while keeping the schema. `name` is only a display label, defaulting to the final path segment; it does not rename the path. |
| `memory_update` | `{path, new_path?, description?, type?, name?, body?, append?, fields?}` | Merges supplied fields into an existing memory; `body` replaces text and `append` adds a paragraph at the end. `fields` sets existing schema properties; `name` is only a display label. `new_path` moves the same page to an existing memory project or subproject, preserving name, description, type and source by field name without adding schema fields. An occupied destination is refused. The CLI exposes this as `notes memory update PATH --new-path NEW_PATH`. |
| `memory_delete` | `{path}` | Moves an existing memory to trash. |

## Hook behavior (0.5.0)

The [hook registrations](../../integrations/claude-plugin/hooks/hooks.json) run only SessionStart (`startup`, `resume`, `clear`, `compact`): the memory rules, the global memory index and the offline cache fallback. No hook writes memory; Claude does, following the rules, as with native Claude Code memory.

**Staging cache fix (2026-10-16):** the e2e harness gives every Claude run temporary `LOCALAPPDATA` and `XDG_CACHE_HOME` roots. Together with SessionStart's configured-hub guard, staging entries cannot replace the real offline index. The next successful SessionStart against the real configured hub rebuilds `global.txt`; no memory data is changed to repair the cache.

## CLI index and Claude import

`notes memory rules` prints the rules block the session hook shows (compiled from `memory_rules.md`). `notes memory index --project global` lists only `global`; the session hook uses it (2026-10-14). Without `--project`, `notes memory index` lists `global` and the current project under separate headings. `--project NAME` selects the project explicitly; otherwise the name comes from the parent of Git's absolute common directory, so worktrees use the main repository's name. Outside Git it lists only global memories. The default `--max-chars 9500` keeps the global index whole when possible, then complete project lines and a count of omitted memories with a hint to use `memory_search` with that scope. Missing projects are a successful empty result; authentication and network errors fail so the hook can show its fallback.

One-off, done 2026-10-14, and not part of the setup any more (the plugin README no longer covers it). `notes memory import <projects-dir> --map integrations/claude-plugin/import/claude-memory-map.json` reads each project's `memory/*.md`, excluding `MEMORY.md`. The map uses exact overrides first, sends scratch folders to `unsorted`, then strips the longest matching prefix or uses the fallback. Frontmatter supplies name, description and type. Other keys retain their original nesting in a trailing `## Imported metadata` YAML code block in the body; the importer uses only the fixed name, description, type and source schema fields. It adds no metadata block when there are no extra keys. Identical twins import once; differing twins use the newer file's modification time and report the choice.

Each memory stores `source = <project>/<stem>`. Import scans the whole Memory tree to find that key before writing and targets the current path associated with it, including memories moved through `memory_update`. Unchanged detection compares the fixed fields and normalized Markdown body, including imported metadata, without storing fingerprint fields. Normalization follows the existing Markdown serialization without resolving page or attachment identities; links rewritten to different titles or aliases can require another update. Incoming frontmatter `source` is excluded in favor of the import key. `--dry-run` reports new, updated, unchanged and conflict counts per project without writing. Add `--offline` to assume an empty vault and skip configuration, authentication and hub access entirely; those counts predict new imports, not the state of a real vault. The source Claude directory is read-only.

**Known limit (accepted 2026-10-14):** a cross-project move made in the app's UI can leave properties tied to the old schema's field IDs and drop the visible `source`. A later re-import could recreate that memory. Core/app move behavior is unchanged; use `memory_update` with `new_path` to retain import identity. Import refuses an unreadable memory type before writing rather than risking a duplicate after a detected schema mismatch. The import is a one-off; once the plugin is live, memories are written in the vault directly, so this case is rare.

## Project wikis

Retired 2026-10-20. `notes wiki import [--project NAME] [--dry-run] [--exclude GLOB] [--report FILE] [DIR]` still exists in the CLI (`notes_cli.rs:429`), but it is no longer used as a mirror. It sent the repo's `wiki/`, falling back to `docs/`, into child pages of its project's Projects row. Nothing syncs between the repo docs and the vault now, and the plugin runs no import at session start (0.5.0). `notes-app` refuses import (`notes_cli/wiki.rs:27`). Plans and the full description are in the internal wiki (notes-app > Agent memory in the vault).

## End-to-end test, 2026-10-14

Install steps for this PC: [plugin install guide](../../integrations/claude-plugin/README.md#install).

**Result: PASS.** The complete `--reuse-hub` run exits 0 with Claude Code 2.1.293. The initial from-scratch run imported 90 memories and passed the functional checks, but failed an overly strict hash check on Claude's own plugin bookkeeping. The user subsequently excluded that runtime class; the final reuse run verifies all remaining protected files unchanged.

Run from Git Bash on any branch; the build archives the current HEAD commit:

```bash
bash <private-e2e-script>
bash <private-e2e-script> --reuse-hub
# Keep the imported memories and omit the import checks:
bash <private-e2e-script> --reuse-hub --skip-import
```

The private staging end-to-end script builds pinned sources on a Docker host, uses a disposable plugin copy, captures sanitized HTTP header evidence, checks source hashes and mtimes (excluding the whole `plugins/synced/` tree, Claude Code's cloud plugin-sync cache, which any running session rewrites), and removes the copy and capture process; any failed check, including the protected-state comparison, exits non-zero. It leaves `notes-hub-e2e` running at **http://DOCKER01_IP:18081**, with volume **`notes-hub-e2e-data`** and image **`notes-hub:e2e`** built from `9b1461c` plus BubblePaw `32986c5`. No production hub is involved. Reuse validates the image's recorded build inputs against HEAD, permitting changes outside those inputs without rebuilding.

| Check | Verified result and local evidence file |
|---|---|
| Real import and idempotence | **PASS:** the original import created 90 memories. In the final reuse run, the dry run and both real imports each report 0 new, 0 updated, 90 unchanged. Includes 10 notes-app memories and one resolved automata conflict. `import-first.txt`, `import-second.txt`, `import-counts.json`. |
| SessionStart context | **PASS:** the actual `hook_response` stream event contains the vault index and all 10 imported notes-app paths. `claude-write.jsonl`, `hook-write.txt`. |
| Plugin search/read/write | **PASS:** successful `mcp__plugin_notes_notes__memory_search`, `memory_read`, and `memory_write` calls. The CLI reads the unique marker back from `notes-app/e2e-check`. `write-check.json`, `marker-readback.txt`. |
| Persisted history | **PASS:** a test-only inspector decodes a copy of the vault's documents/logs through `Vault::list_versions`; the page's history records `agent:e2e-claude`. `history-actor.json`. |
| Read token | **PASS:** the second session searches and reads; its advertised notes tools contain no write tools. The marker remains intact. `read-check.json`, `marker-after-read.txt`. |
| Origin handling | **PASS:** container packet capture records 351 bearer-authenticated `/mcp` requests and zero Origin headers, with zero dropped packets. The single deliberate Origin curl, outside capture, receives 403. `claude-request-headers.jsonl`, `capture.stderr.txt`, `origin-status.txt`. |
| Protected Claude state | **PASS:** 371 protected entries unchanged, including all 162 source memory files. Settings, instructions, installed plugin manifests/code and marketplace configuration retain their hashes and mtimes. Runtime exclusions and actual excluded paths are recorded separately. `protected-comparison.json`, `protected-before-exclusions.json`. |

The final run uses MCP `userConfig` defaults in the temporary manifest; `plugin-config-mode.txt` records `defaults`. The earlier `--plugin-dir` probe did not export `userConfig` values to hooks, so the harness explicitly exports the staging URL and scoped token for every hook invocation. The installed-plugin option export path remains unverified. The hook now falls back to the manifest's `https://notes.example.com` URL and overrides `NOTES_TOKEN` only when the token option is supplied, otherwise retaining the CLI's environment/config resolution. Four isolated hook checks cover missing, explicit and empty options plus an existing CLI token. Tokens are never logged; their file has a verified owner-only Windows ACL because Git Bash's NTFS mode remains synthetic `0644` after `chmod 600`.

The script explicitly excludes `plugins/synced/**/.last-complete-round` and `plugins/cache/**/.in_use/[0-9]*` (fully numeric ASCII PID filenames directly inside `.in_use` only). The installed client contains both the sync-round writer and the PID lease writer/exit cleanup. Synthetic checks verify that adjacent code, manifests, settings, marketplace config and memory changes still fail. The token ACL guard verifies an already-private file without rewriting its ACL, and establishes new access rules without changing owner/group or requesting SACL privileges. The staging hub retains the imported memories.
