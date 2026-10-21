# notes plugin for Claude Code

Connects Claude Code to the notes hub. The plugin has no server process of its own.

It does three things:

- **MCP tools.** Adds the hub's `notes` server at `<hub>/mcp`, so Claude Code can use the memory tools and the general vault tools.
- **Memory rules.** At session start, prints a compact block of memory rules (condensed from the `notes:memory` skill), so every session has them without loading the skill.
- **Memory index.** Then prints the global memory index, one line per memory. Memory is one global memory in four categories (`user`, `workflow`, `tools`, `infra`): facts about the user and how to work with them. Project facts live in each project's internal wiki (the vault) and repo docs. The last good index is cached, so an offline session still gets it, marked as cached.

The `notes:memory` skill has the full rules for saving, updating, recalling and deleting memories through the tools.

## Prerequisites

- An agent token from the hub admin. Create one on the hub with `notes-hub --agent-token add`.
- The `notes` CLI on PATH, for the memory index. Without it, the hook prints only the memory rules and the session starts normally.

## Install

Steps for this Windows PC, once the production hub serves `/mcp`.

1. **Create an agent token** on the hub host. The token is shown once, so copy it now:

   ```
   notes-hub --agent-token add <name> --scope write
   ```

   Names use lowercase letters, digits and hyphens. Add `--config <file>` if the hub does not use the default config. `--scope read` gives only the read tools.

2. **Add the marketplace and install the plugin.** In Claude Code, with `<repo>` as the local checkout (for example `V:/projects/notes-app`):

   ```
   /plugin marketplace add <repo>/integrations
   /plugin install notes@notes-app
   ```

   Claude Code then asks for two values:

   - **Hub URL:** your hub's URL, for example `https://notes.example.com`.
   - **Agent token:** paste the token from step 1.

3. **Put `notes.exe` on PATH.** It is built at `F:\build\notes-cli\notes.exe`, so add `F:\build\notes-cli` to the user PATH and open a new terminal. Without it, the session-start hook prints only the memory rules, not the index. Outside Claude Code, the CLI reads `NOTES_HUB_URL` and `NOTES_TOKEN`, or `notes/config.toml`.

4. **Turn off Claude Code's built-in memory**, so the vault is the only memory. Do this once, with either:

   - add `"autoMemoryEnabled": false` to `~/.claude/settings.json`, or
   - set the environment variable `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`.

   The plugin cannot change this setting.

5. **Check that it works.** Start a new session, then:

   - Run `/mcp`. `notes` should be listed as connected.
   - The session context should begin with `# Memory rules (notes vault)`, then `# Memory index (notes vault)` and the index lines.
   - Ask Claude to search the vault for a known note.

   If the index command fails, the hook prints the rules and then either the last cached index, under a header that gives its UTC date and says the hub is unreachable, or, with no cache yet, one short unavailable line. The session still starts.

## Offline cache

After each successful index fetch, the hook saves the global index text, so it survives a hub outage. The file is `%LOCALAPPDATA%\notes\memory-index\global.txt` on Windows, and `${XDG_CACHE_HOME:-~/.cache}/notes/memory-index/global.txt` elsewhere. The cache never holds the token. While the hub is unreachable, memory writes fail, and the session header says so.

## Where the token is stored

`hub_token` is marked sensitive, so Claude Code keeps it in the OS credential store, not in a settings file. The session-start hook uses `CLAUDE_PLUGIN_OPTION_HUB_TOKEN` when supplied and never prints it. Otherwise it leaves token resolution to the CLI's existing environment or `notes/config.toml`. If `CLAUDE_PLUGIN_OPTION_HUB_URL` is absent or empty, the hook uses `https://notes.example.com`, matching the manifest's default.

The staging test explicitly exports both hook options: `--plugin-dir` did not export `userConfig` to the hook in the verified session. The installed-plugin option export path remains unverified.

## Staging test

The private staging end-to-end script runs this flow end to end against a staging hub on a Docker host, not production. Run it from Git Bash in the repo root.
