---
type: topic
layer: state
summary: "The notes CLI's wiki import, retired 2026-10-20 as the repo-to-vault mirror; the command remains."
parent: "[[integrations/index]]"
status: retired
---

# Wiki import

Retired 2026-10-20: repo docs are no longer mirrored into the vault, and nothing syncs between them. The command below still exists in the CLI but is not used, and the Claude Code plugin runs no import.

The `notes` CLI uses a scoped hub token from `notes/config.toml` or `NOTES_TOKEN`. From a project repository, `notes wiki import` mirrors `wiki/` (or `docs/`) into that project's row in the vault's Projects database. Worktrees share the main repository's project name. Repeating the import updates pages in place; unchanged pages create no edits, and missing files remain in the vault as reported orphans. The repository stays the source.

```
notes wiki import [--project NAME] [--dry-run] [--exclude GLOB] [--report report.md] [DIR]
```

Repeat `--exclude` as needed. Secret files are never packed. Non-markdown files are stored only when linked from an imported page; other files are counted as not imported, with their relative paths listed in `--report`. Skipped files include reasons. `--dry-run` diagnoses an import against an empty temporary vault; its counts do not predict changes to the real vault. The `notes-app` project is excluded.

The [Claude Code plugin](../../integrations/claude-plugin/README.md) has no wiki import (it had a daily auto-import before 0.5.0).

See [AI assistants](ai-assistants.md) for the rest of the CLI and its config.
