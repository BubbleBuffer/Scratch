---
type: topic
layer: implementation
summary: "The optional schedule note type, which makes a note from a template on a runner."
parent: "[[addons/index]]"
status: current
---

# Schedule

The schedule add-on is a note type that makes a note from a template on a set rhythm.
Its definition and status live in the page's `schedule` and `schedule_status` type data.
The engine owns the rules: the preview, titles, next runs and acceptance hashes all come from it.
The preview always goes through the vault (`schedulePreview`), so it works on Android and offline, with no hub.
Routing by runner ([engine.dart](../../app/lib/addons/schedule/engine.dart)):
- This device's own runner (the node record that the vault's `localNodeId` names) is called through the vault's local schedule calls. They work without a hub.
- A hub node is called through the hub's agent connection, with the sync settings' hub URL and device token.
- Another desktop's runner is never called; the page shows its synced status and asks the user to open the app there.
Accept sends the hash of the definition on screen, so the runner accepts exactly what was reviewed.
The status band sits under the title and shows the state, the action that applies, the next run and the last run.
Its states read "Active", "Needs acceptance", "Paused", "Runner turned off" and the like.
The actions are compact and side by side on desktop (Accept filled; Pause or Resume and Run now secondary). A phone keeps touch-sized buttons and may wrap them.
While the runner is off, the schedule is paused or it needs acceptance, the next run is labelled "Next run (when active)", because nothing runs yet.
When, Runs on and Step follow it, and Step shows the template and parent page titles from the vault.
Schedules with more than one step, or a step other than a note from a template, are refused.
The hub link rule is copied from the agent add-on, not shared with it.

## "Run schedules on this device" setting

- Registered by the add-on in Settings > Add-ons under "Schedules" ([settings.dart](../../app/lib/addons/schedule/settings.dart)). Desktop only: Android never registers it.
- Stored device-locally in the settings file (`scheduleRunsHere` in [settings_store.dart](../../app/lib/features/settings/settings_store.dart)), never in the vault. Off by default.
- Turning it on or off calls `localRunnerSetAvailable` on the vault. The switch shows the error when the runner does not change.
- The runner starts with the stored value once per vault, when the schedule add-on first builds its backend for that vault ([local_runner.dart](../../app/lib/addons/schedule/local_runner.dart)).
- Gap: the add-on has no hook at app start, so the runner starts only when a schedule or the Add-ons settings is first shown. A desktop with the setting on runs nothing until then. Fix needs an add-on startup hook in the kit, in [boot.dart](../../app/lib/shell/boot.dart).
- The Add-ons panel ignores a section's `isVisible`, so the add-on checks the platform when it registers the setting.

## Tests

Tests run against a fake engine ([fake.dart](../../app/lib/addons/schedule/fake.dart)).
Goldens cover five states at two sizes and two themes: needs acceptance, active with a last run, runner turned off, this computer as the runner, and the embed card.
