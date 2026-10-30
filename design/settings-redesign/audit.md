# Settings audit (2026-10-23, Scratch main d627fea)

Every settings screen and option as built, read from the code (no app run). "Who" is who plausibly needs it. ⚠ marks the options the user called incomprehensible or that are hard to understand without reading the code. 🐞 marks bugs.

## Shell

- Sections, in order: Preferences, Reminders, Sync, Devices (desktop only), Conflicts, AI, Vault, About. Add-ons is hidden behind About › Manage. (`app/lib/features/settings/settings_screen.dart:34-50`)
- Desktop: a 1150×715 panel with a 240 px left nav. Phone: a list of sections with trailing values, then one screen per section.
- Settings live in `<data dir>/settings.json`, per device, unless noted as synced.

## Preferences

| Option | Control | What it does | Default | Where | Who |
|---|---|---|---|---|---|
| Appearance | System / Light / Dark | Theme on this device | System | All | Everyone |
| Send feedback | "Open" | Opens the feedback sheet; a screenshot and diagnostics go with it | – | All | Everyone |
| Shake to send feedback | Switch | Shaking the phone opens the feedback sheet | On | Phone | Phone users |

## Reminders

| Option | Control | What it does | Default | Where | Who |
|---|---|---|---|---|---|
| Reminders | Switch, no helper text ⚠ | Turns event reminders and the morning summary on or off | On | All | Everyone |
| Remind me before events | At start / 5 / 10 / 30 min | Lead time | 10 min | Disabled when off, with no reason shown | Everyone |
| Morning summary at | 05:00–12:00, 30-min steps | When the summary arrives | 08:00 | Disabled when off | Everyone |
| "Notifications are off for Scratch in Android settings." | Text | Shown when permission is denied; no way to fix it from here | – | Android | Everyone |

## Sync

| Option | Control | What it does | Default | Where | Who |
|---|---|---|---|---|---|
| Hub sign-in URL + Sign in | Field + button | Browser (OIDC) sign-in to a hosted hub; saves on every keystroke, unvalidated | Empty | All | Everyone |
| Signed in to … / Sign out | Text + button | Signs this device out | – | Signed in | Everyone |
| Let the hub use this computer ⚠ | Switch | AI apps connected to the hub can read and edit files and run commands here while the app is open. It is an AI capability, but it sits under Sync | Off | Desktop, signed in | Power users |
| Share this vault with your devices | Switch | This computer runs a hub. 🐞 Start failures are swallowed, so it can show On while the hub is down (`sync_control.dart:76-78`) | **On** | Desktop | Power users |
| Listen address ⚠ | Field + Apply | Hub bind address; unvalidated; restarts the hub. The default exposes every network interface, with no warning | `0.0.0.0:47100` | Desktop | Power users |
| Hub addresses | Read-only list | Where devices can reach this hub | – | Hub running | Power users |
| Add a device | Button | Pairing flow (also in Devices) | – | Desktop | Power users |
| Pairing code or hub address, Token, Connect | Fields + button | Connects to a self-hosted hub; the token is stored in plain text in `settings.json` | – | Not connected | Everyone |
| Connected to … / Sync now / Disconnect | Row + buttons | Status and manual sync. Disconnect only for pairing-code devices | – | Connected | Everyone |
| Download attachments in the background | Switch | Prefetches every file | Off | All | Everyone |
| Download all attachments | Button | One-off full download; no cancel | – | Disabled offline | Everyone |
| Trusted certificates | Rows + Revoke | Pinned hub certificates from unknown issuers | – | Only when pinned | Power users |

## Devices (desktop)

| Option | Control | What it does | Default | Who |
|---|---|---|---|---|
| Paired devices | Rows + Unpair | Revokes a device's token. 🐞 No confirmation | – | Hub owner |
| Add a device | Name + Create pairing code → QR, code, URL select | One-use token, shown once. 🐞 The fallback URL `http://127.0.0.1:47100` can't be reached from a phone; closing loses the token with no warning | Name "Phone" | Hub owner |

## Conflicts

| Option | Control | What it does | Default | Who |
|---|---|---|---|---|
| Changed on two devices | List | Pages with unresolved conflicts; count badge in the nav | – | Everyone |
| Resolve changes on their own ⚠ | Switch, synced to the vault | Auto-dismisses entries that settle themselves. 🐞 Reads Off while loading (`conflicts.dart:851`) | Vault value | Everyone |
| Recent | Log + Undo | Resolved in the last 30 days | – | Everyone |

## AI (`app/lib/features/ai/ai_settings_body.dart`, `app/lib/addons/agent/*`)

Rendered order: intro, Agent, Ask AI, MCP endpoint, Connect an agent, Agent tokens, Recent tasks. Agent and endpoint parts are desktop only (Windows, Linux).

| Option | Control | What it does | Default | Where | Who |
|---|---|---|---|---|---|
| Use the agent | Switch | Shows the Agent chat (`agentEnabled`, per device) | On | Desktop | Everyone |
| Run AI on | Hub / This computer | Where the BubblePaw agent runs; **synced to the vault**, applies to every device | Built-in default | When on | Everyone |
| Model | Select | Vault default model for that runner. 🐞 Shows the first model when the saved one is missing, without saving it | Hub default / first listed | When on | Everyone |
| Tools | Read / Write / Everything ⚠ | Vault tool set for that runner. Computer access, projects and MCP servers only work with Everything, but nothing says so | Everything | When on | Power users |
| Computer access | Switch | Desktop tool pack: projects and unsandboxed commands. Does nothing below Everything | On | This computer | Power users |
| Projects | List, Rename, Remove, Add project ⚠ | Working folders for the agent. 🐞 Home shows as `\\?\C:\Users\…` because `projects.rs:229` stores `std::fs::canonicalize` output (Windows verbatim prefix); fix is to strip it in `make_project`, and existing `projects.json` files keep the old string. "Add project" is a bare centered text button. Remove has no confirmation | Home | Computer access on | Power users |
| MCP servers | List, toggle, Edit, Remove, Add server ⚠ | Extra tool servers (program or URL) for the local agent. The helper is jargon. They are silently skipped below Everything, yet the text says "changes apply at once". Remove has no confirmation | None | This computer | Power users, developers |
| Claude Code: bun program ⚠ | Mono field | Override for `bun.exe`, which runs BubblePaw's **claude-code provider**. Nothing on screen says so. A wrong path hides Claude Code entirely, with no fallback to PATH. Saves on every keystroke | Empty (detected) | This computer | Developers |
| Claude Code: extension folder ⚠ | Mono field | Override for the BubblePaw claude-code extension folder (needs `src/main.ts`). Its placeholder is a relative string, not a real path | Empty (detected) | This computer | Developers |
| Ask AI | Text | "Ask AI runs on the hub / this computer"; repeats Run AI on | – | All | – |
| Serve the vault to AI apps | Switch | Local MCP endpoint inside the app | Off | Desktop | Developers |
| Address | Field | Endpoint bind address; restarts it | `127.0.0.1:47101` | Endpoint on | Developers |
| Copy token / Copy Claude Code command / New token | Buttons | Endpoint token; New token rotates it after a confirmation | – | Endpoint on | Developers |
| Connect an agent | Two copyable commands | `notes setup claude` / `notes setup codex` | – | Desktop | Everyone |
| Agent tokens | List, Revoke, New token form | Hub-issued tokens for outside AI apps (name, tool set, expiry); secret shown once | Write, 90 days | Signed in to a hub | Everyone |
| Recent tasks | List + Retry | The last 20 Ask AI tasks | "No AI tasks yet." | All | Everyone |

## Vault

| Option | Control | What it does | Default | Where | Who |
|---|---|---|---|---|---|
| Location | Copyable path | Vault folder | – | All | Everyone |
| Open another folder / Use the default location | Field + Choose… + Use folder | Switches vault after a restart; 🐞 a nonexistent path is accepted unchecked | Default | Desktop | Power users |
| Mounted folders | Field + Mount, rows + Remove | Markdown folders shown as pages. 🐞 Remove has no confirmation or error handling | None | Desktop | Power users |

## About and Add-ons

| Option | Control | What it does | Default | Who |
|---|---|---|---|---|
| Version | Text | 🐞 Hardcoded `0.1.0`, not read from the package | – | Everyone |
| Device ID | Copyable | Identifies this device to the hub and in conflicts | – | Support, developers |
| Add-ons › Manage | Button | Opens the add-on list (hidden under About) | – | Everyone |
| Optional add-ons (Agent, Board, Kanban, Schedules, Tasks, Search card) | Switches | Disable per device; restart needed. Turning off Tasks also removes Calendar, which the list doesn't say | All on | Everyone |
| Required add-ons (Built-ins, Document, Database) | "Enabled" | – | – | – |
| Run schedules on this device | Switch (Schedules add-on section) | This computer runs schedules while open. 🐞 The flag is saved before the runner changes, so a failure leaves it On (`schedule/settings.dart:35`) | Off | Desktop |

## Settings that live elsewhere (not in this redesign)

- Calendar sheet (gear in the calendar): per-calendar visibility in the calendar and in Today, new, rename and delete.
- Database view settings: layout, properties, filter, sort, group, Show in Today, Open pages in.
- Sync indicator sheet: status, Sync now, a link to Sync settings.
