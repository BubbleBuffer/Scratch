---
type: topic
summary: "scratch-cli runs a desktop vault's AI tasks with the in-app agent, without the app's UI."
parent: "[[integrations/index]]"
status: current
---

# scratch-cli

`scratch-cli` (crate [crates/cli](../../crates/cli), package `scratch_cli`) runs the desktop AI task runner with no Flutter UI. Given a vault folder, it claims the queued AI tasks pinned to that vault copy's device and runs them with the in-app agent, the same way the app's claim loop does, code tasks in a project worktree included. The agent writes results, progress and mirrored to-dos into the vault as it does in the app. See [AI assistants](ai-assistants.md#ai-tasks) for how tasks, subtasks and code worktrees behave.

It makes the app's Rust calls and adds no task logic of its own: `tasks::recover` once at start, then `tasks::claim_next` for the vault's device id (`<vault>/.vault/device`), `Agent::run_task`, and `tasks::finish` with the error when a run cannot start. The agent opens with the app's desktop config: desktop tools on (the Home project is the user's home folder on first use), Bypass permissions, and the `this_device` tool set from the vault. Its data folder is the vault's parent folder, the app's own default; sessions, projects and worktrees go in `<vault parent>/agent/`. The app's MCP servers are not applied.

## Commands

- `scratch-cli run-tasks --vault <DIR> [--device <ID>] [--model <provider/model>] [--once]`: recovers tasks this device left running, then claims and runs tasks one at a time. It prints one line per task: `<id> done` or `<id> failed: <error>`. With `--once` it exits 0 when no task is left queued for the device; without it, it polls every 5 seconds until Ctrl-C. `--model` overrides the default model. Otherwise the vault's default for `this_device` applies, then the agent's built-in desktop default. `--device` must match the vault's device id.
- `scratch-cli task create --vault <DIR> --prompt <TEXT> [--page <ID>] [--project <NAME-or-ID>]`: queues a task pinned to this device (runner `agent`) and prints its id. Without `--page` the task gets a new top-level page, titled with the prompt's first line. `--project` names an agent project by id or name; the task then runs as a code task in a worktree of that repository.
- `scratch-cli task show --vault <DIR> <ID>`: prints the status, error, progress, project, `worktree_branch` and `worktree_path`, and the result once the task is done.
- `scratch-cli project add --vault <DIR> --name <NAME> --path <REPO>`: registers a repository as an agent project of this device and prints its id. It is the same project list the app's agent uses.
- `scratch-cli page create --vault <DIR> --title <TITLE>`: creates a top-level page and prints its id, for `task create --page`.

## Close the app or use another vault

Only one process may write a vault as its device. If the app (or another `scratch-cli`) has the vault open, every command exits non-zero with "close Scratch or use another vault". There is no locking or remote control of a running app.

## Credentials

Credentials come from the environment, as for the app's agent, and are never printed: the Claude Code login, the Codex login (`CODEX_HOME`), and `OPENCODE_API_KEY` for OpenCode models.

Claude Code needs Bun and the extension folder, found as the desktop app finds them, with the app's own folder added because `scratch-cli.exe` does not live in it (`vault_agent::claude::app_overrides`, then `claude::detect_installed`). First match wins:

1. Extension folder: `NOTES_CLAUDE_CODE_DIR` when set; else the app's saved Claude Code folder (`agentClaudeCodeDir` in `ai_settings.json`, read beside the vault, then in the app's data folder: `NOTES_DATA_DIR`, else `%LOCALAPPDATA%\Notes` on Windows, else `~/.local/share/notes_app`); else the `claude-code` folder beside `scratch-cli.exe`; else `%LOCALAPPDATA%\Programs\Scratch\claude-code`, the installed app's copy (Windows only); else the extension location compiled into the binary from the BubblePaw dependency, which exists only on the machine that built it.
2. Bun: the app's saved Bun path (`agentBunPath`, same files); else `bun` on `PATH`; then `~/.bun/bin`; then `bun\bun.exe` beside `scratch-cli.exe`, else in `%LOCALAPPDATA%\Programs\Scratch\bun` (Windows only).

The Windows installer ships both, so an installed Scratch needs no setting: the CLI finds the same extension and Bun the app runs. Only a machine without the app (or with a BubblePaw checkout you would rather use) needs `NOTES_CLAUDE_CODE_DIR` or the folder in the app's Settings > AI > Agent.

A model whose provider is missing fails the task before any request, with an error that names the provider and what is missing, for example `claude-code is unavailable: the Claude Code extension folder was not found; set NOTES_CLAUDE_CODE_DIR`. It is never sent to another provider.

## Builds and tests

The Windows CLI job in `.github/workflows/windows-build.yml` builds `scratch-cli.exe` beside `notes.exe` and uploads it as the `scratch-cli-windows` artifact. The tests in `crates/cli/tests` drive the command lines in-process with the agent's scripted provider: a task created with `task create` finishes with its result and nested to-dos, and a project task runs in a worktree of a temporary git repository.
