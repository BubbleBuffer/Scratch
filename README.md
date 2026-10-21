# Scratch app

> **Pre-alpha. Not ready for real use.** Scratch is unstable and changes often. Vault, sync and hub data formats may change without a migration path, so keep your own backups and expect to start over. There are no support, security or compatibility promises, and issues or pull requests may go unanswered.

A private notes vault with a Rust core, Flutter app and self-hosted hub. See the [project wiki](wiki/index.md) for architecture and current state.

The `notes` CLI uses a scoped hub token from `notes/config.toml` or `NOTES_TOKEN`. From a project repository, `notes wiki import` mirrors `wiki/` (or `docs/`) into that project's row in the vault's Projects database. Worktrees share the main repository's project name. Repeating the import updates pages in place; unchanged pages create no edits, and missing files remain in the vault as reported orphans. The repository stays the source.

```
notes wiki import [--project NAME] [--dry-run] [--exclude GLOB] [--report report.md] [DIR]
```

Repeat `--exclude` as needed. Secret files are never packed. Non-markdown files are stored only when linked from an imported page; other files are counted as not imported, with their relative paths listed in `--report`. Skipped files include reasons. `--dry-run` diagnoses an import against an empty temporary vault; its counts do not predict changes to the real vault. The `notes-app` project is excluded.

The [Claude Code plugin](integrations/claude-plugin/README.md) starts a silent wiki import at most once per project per UTC day, when the CLI is installed and configured. It runs separately with a 120-second deadline. Turn it off with the plugin's `wiki_import` option or `NOTES_WIKI_IMPORT=0`.

## Licence

Scratch is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version (`AGPL-3.0-or-later`); see [LICENSE](LICENSE). Bundled third-party components and their licences are listed in [THIRD_PARTY.md](THIRD_PARTY.md).
