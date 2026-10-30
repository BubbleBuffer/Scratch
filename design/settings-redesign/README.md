# Settings redesign (proposal, 2026-10-23)

**Status:** mockup for review. Nothing here is built in the app.

The user found Settings, and Settings › AI in particular, "horrid ux" and "largely incomprehensible": options are shown with no explanation of what they do or when to change them. This proposal reorganises the options that exist today. It adds no options or features. [audit.md](audit.md) lists every current option, what it does in code, its default, and the bugs found.

- **Open the mockup:** serve `design/` (`python -m http.server 8781 --directory design`) and open `http://localhost:8781/settings-redesign/index.html`. The dark bar at the top switches theme, empty or filled lists, signed in or not, help and the inline editors. It is not part of the design. Narrow the window below 720 px for the phone layout.
- **Screenshots:** [shots/](shots/), made by `python capture.py` (headless Chromium, with the server running).
- **Data is illustrative:** paths, servers, tokens and statuses are made up. Icons are Lucide, which the app already uses.

## Structure

| Group | Page | Holds (today's location) |
|---|---|---|
| – | General | Appearance, Send feedback, Shake to send (Preferences) |
| – | Reminders | Reminders, lead time, morning summary (Reminders) |
| Your vault | Sync and devices | Status and Sync now; hub account or pairing code; attachments; "This computer as a hub" with addresses, **paired devices** and Add a device; Advanced: listen address; trusted certificates; **Run schedules on this device** (desktop only; moved from Add-ons) (Sync + Devices) |
| | Conflicts | List, auto-resolve, 30-day log (Conflicts) |
| | Storage | Vault location; Advanced: another folder; mounted folders (Vault) |
| AI | Agent | Use the agent, where it runs, model, what it may do (tool set), On this computer (computer access, projects; MCP servers link to Connectors), Advanced: Claude Code provider (bun, extension folder), recent tasks (AI › Agent, Ask AI, Recent tasks) |
| | Connectors | One page, three collapsible panels, all collapsed by default. **Tools for the agent**: MCP servers and the tool set status (MCP servers, from AI › Agent). **Apps that use your vault**: quick setup commands, access tokens (AI apps). **This computer for hub apps**: Let the hub use this computer, with the relay switch (AI apps › Advanced; Sync) |
| – | About | Version, Device ID (About) |

## What changes, and why

Two things are removed: the add-on switches (item 7) and the local endpoint (open question 2).

1. **AI splits into two jobs.** *Agent* is Scratch's own AI. *Connectors* is everything outside Scratch that plugs in: the MCP servers the agent uses, and outside apps (Claude Code, Codex) using your vault. Today both are on one page, and "MCP" means two different things: servers the agent uses, and a local endpoint that serves the vault. The local endpoint is dropped (open question 2). Each page now opens with one sentence that defines its job, and MCP is defined once.
2. **Every option says what it does; the ⓘ button says when to change it.** Descriptions are always visible. The "when to change it" note is one hover or click away, so the page stays calm. Hovering ⓘ, or focusing it with the keyboard, shows the note for as long as the pointer or focus stays there, and the note stays open while the pointer is on it. A click pins the note open or closed, and the button shows a pressed state. Touch has no hover, so a tap toggles. The review bar's "Help: all open" pins every note.
3. **Dependencies are visible.** Computer access and projects (on Agent) and MCP servers (on Connectors) do nothing unless the tool set is *Everything*. Today they look active anyway; in the mockup they show a lock line: "Needs “Everything” on the Agent page.", which links back to Agent. Reminder timings show "Turn on Reminders to change this."
4. **Advanced options are folded.** The listen address, another vault folder and the Claude Code paths sit under "Advanced" disclosures, each with a short summary of what is inside. Hub access to this computer is not folded: it is a row in the "This computer for hub apps" panel on Connectors.
5. **The Claude Code fields explain themselves.** They sit under Advanced › Claude Code provider, with a paragraph on what they are for (BubblePaw's claude-code provider), the detected path as the placeholder, and the warning that a wrong path hides Claude Code.
6. **Real lists with empty states.** Projects, MCP servers, paired devices, tokens, mounts, conflicts and recent tasks are bordered lists. Each row has an icon, a name, a mono path or command, a status pill and labelled actions. Add buttons sit in the list footer, not as bare centered text. Empty lists explain what the thing is and offer the first action.
7. **Add-ons is dropped, and its switches with it.** Owner's decision, 2026-10-23: "way to premature, we dont have any addons we want toggleable like that yet". Agent › Use the agent stays the only agent on/off switch. **Run schedules on this device** moves to Sync and devices, in "This computer as a hub", as its own desktop-only row with the ⓘ help.
8. **Sync and Devices merge.** Paired devices are managed where the hub is switched on. "Add a device" appears once, not twice.
9. **"Let the hub use this computer" moves from Sync to Connectors › This computer for hub apps.** It is a connector capability, not a sync one. That panel has no Advanced disclosure.
10. **Look:** the Stacks tokens and fonts; family switches (square, moss when on); fitted corners; pills only for statuses.

## Bugs to fix regardless (from the audit)

- Home project path shows `\\?\C:\Users\…`: strip the verbatim prefix in `make_project` (`crates/agent/src/projects.rs:229`), and also when reading existing `projects.json` files.
- "Share this vault" and the listen address swallow hub start failures, so the switch can read On while the hub is down.
- Run schedules on this device saves the flag before the runner changes.
- Conflicts auto-resolve reads Off while loading.
- Unpair, Remove project, Remove MCP server and Remove mount have no confirmation.
- The pairing fallback URL `127.0.0.1` can't be reached from a phone.
- The bun override is authoritative: a wrong path hides Claude Code with no fallback.

## Code changes this implies

Dropping the local endpoint (open question 2) removes code in these places. The owner's estimate is about 750 lines, with their tests and docs.

- `crates/mcp/src/http.rs`, `crates/mcp/src/proxy.rs` and `crates/mcp/src/endpoint.rs`: the endpoint server.
- `notes-mcp`: its http and forwarding branches.
- `NotesMcp` in `app/rust/src/api/ai.rs`, and the code that starts and stops it in `app/lib/features/ai/ai_store.dart`.
- `_McpSection` in `app/lib/features/ai/ai_settings_body.dart`.
- The `mcpEnabled` and `mcpAddress` settings, in `app/lib/features/ai/ai_settings.dart` (fields, load and save).
- The tests and docs that cover any of the above.

## Open questions for the user

1. **Decided: resolved by dropping Add-ons (2026-10-23).** The add-on switches are gone, so Agent › Use the agent is the only agent on/off switch.
2. **Decided: the local endpoint is dropped (2026-10-23).** An extra local MCP server feels like it conflicts. Outside apps go through the hub, which on a desktop runs inside the app. Advanced keeps only "Let the hub use this computer".
3. **Decided: hover shows the note, click pins it (2026-10-23).** Hover or keyboard focus opens the note while it stays; a click pins it open or closed. Touch has no hover, so a tap toggles. "Help: all open" pins every note.
4. **Decided: keep the hub defaults, no warning (2026-10-23).** "Share this vault" stays on by default and listens on every network (`0.0.0.0`). The mockup does not change this.
5. **Decided: keep "Agent"** (not "Assistant"). The outside-apps question is decided too: see Decisions (2026-10-23).
6. **Decided: adopt the square switch.** The mockup's family switch (square, moss when on) replaces the app's small round toggle.

## Decisions

- **2026-10-23: one Connectors page replaces AI apps.** The owner asked for one Connectors page with collapsible panels, replacing AI apps: Tools for the agent, Apps that use your vault, and This computer for hub apps, all collapsed by default. MCP servers move there from Agent, and the lock line on the Tools panel links back to Agent. The page id is `connectors`, and the URL hash keeps the open panels as `open=tools,apps,hub`. On a phone the three panels stack.
