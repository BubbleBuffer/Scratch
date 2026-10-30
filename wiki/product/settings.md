---
type: topic
layer: state
summary: "The Settings pages in order, what each one holds, and where options moved in the 2026-10-30 redesign."
parent: "[[product/index]]"
status: current
---

# Settings

Settings opens as a centred panel on desktop, with the pages in a left list grouped as below. On phones it is a full-screen list of the same pages in the same order. The approved layout is in [design/settings-redesign](../../design/settings-redesign/README.md).

## Pages

| Page | Group | What it holds |
| --- | --- | --- |
| General | none | Appearance and feedback. |
| Reminders | none | Reminders on or off, the lead time and the morning time. |
| Sync and devices | Your vault | Hub sign-in, "Share this vault with your devices" (addresses, pairing, paired devices), Download all attachments, and "Run schedules on this device" (desktop). |
| Conflicts | Your vault | Pending conflicts and "Resolve changes on their own". |
| Storage | Your vault | The vault folder and Mounted folders (desktop). |
| Shared links | Your vault | The share links this device's hub holds. Listed only while signed in to a hub, after Storage. |
| Agent | AI | Use the agent, Run AI on (Hub or This computer), Model, What it may do, On this computer (desktop), Advanced (Claude Code's bun program and extension folder), Computer names, and Recent tasks. |
| Connectors | AI | Three collapsed panels: Tools for the agent (MCP servers, desktop), Apps that use your vault (the `notes setup` commands and agent tokens), and This computer for hub apps (letting the hub use this computer, its addresses and its name). |
| About | none | The version, the device ID and, on Windows, an Updates section. |

## Where the old names went

- Preferences: General and Reminders.
- Sync and Devices: Sync and devices.
- Vault: Storage.
- AI: Agent and Connectors.
- AI apps: Connectors, under Apps that use your vault.
- Runner names: Computer names, on the Agent page.
- Updates: a section of About on Windows.
- Add-ons: the page and its switches were removed 2026-10-30. Every compiled-in add-on is always on.
- Serve the vault to AI apps: the local MCP switch was removed 2026-10-23. AI apps reach the vault through the hub, or through `notes-mcp` over stdio.

## Hub-dependent behaviour

- "Share this vault with your devices" reads Off if the hub fails to start.
- The pairing code never offers a loopback (127.0.0.1) address.
- Changing the runner and "Run schedules on this device" save only after the change succeeds.

## Where the rest lives

- Agent and computers: [Agent](../integrations/agent.md) and [AI assistants](../integrations/ai-assistants.md).
- Hub, pairing and agent tokens: [Hub](../sync/hub.md) and [Device keys](../sync/device-keys.md).
- Sharing: [Note sharing](../integrations/sharing.md).
- Conflicts: [Conflict format](../core/conflict-format.md).
- Folder mounts: [Folder mounts](../core/mounts.md).
- Schedules: [Scheduled tasks](scheduled-tasks.md).
- Reminders: [Today](today.md).
- Updates: [Windows install](../releases/windows-install.md).
- Feedback: [Feedback](../app/feedback.md).
- Add-ons: [Add-ons](../app/addons.md).
