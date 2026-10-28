---
type: page
summary: "Quick feedback capture: shake, Ctrl+Shift+F or a Send feedback button (sidebar, page menu, Pages screen) opens a sheet with a screenshot and diagnostics; app and MCP reports are hidden child pages of the Feedback system page."
parent: "[[architecture/index]]"
status: current
---

# Feedback capture (state)

Built 2026-10-16 on `lane/feedback`.

## How it works

- **Triggers.** On Android and iOS, shaking the phone opens the sheet (accelerometer via `sensors_plus`, sampled every 20 ms, threshold 2.7 g, two peaks within 500 ms, 2 s cooldown, ignored while the sheet is open). Settings has a toggle "Shake to send feedback", on by default, shown on phones only. On desktop, Ctrl+Shift+F (command `feedback.send`) opens it. Settings has a "Send feedback" row on every platform.
- **Entry points.** Besides the shortcut, shake and the Settings row, these run `feedback.send` through `commandRegistry.run` (`sendFeedback` in `app/lib/shell/sidebar.dart`). All use the feedback icon (`Ix.feedback`, Lucide `message-square-warning`); "Add icon" keeps the smile. Their semantic label stays "Send feedback (Ctrl+Shift+F)"; the `AppTooltip` shows "Send feedback" with the shortcut taken from the `feedback.send` command.
  - Sidebar footer: an icon button beside Settings (key `sidebar-feedback`).
  - Page "•••" menu: a last row "Send feedback" with Ctrl+Shift+F shown on desktop (key `page-menu-feedback`, action `PageMenuAction.feedback`).
  - Sidebar collapsed (desktop): `CollapsedFeedbackButton` (key `topbar-feedback`) sits beside the reopen button in the home, page and agent top bars.
  - Phone: a "Send feedback" row below Settings at the bottom of the Pages screen (key `pages-feedback`).
- **Sheet.** A bottom sheet on phones and a dialog on desktop. It has a "What happened?" text field, a thumbnail of the screen with a remove button, and a collapsed Diagnostics section that shows the exact text that will be sent. Send with an empty text field is allowed. The screenshot is captured from a `RepaintBoundary` at the app root before the sheet opens, at the screen's pixel ratio capped at 3x.
- **Diagnostics.** App version, build, git commit (`--dart-define=GIT_COMMIT`, empty when absent), platform, OS, device, locale, screen size and pixel ratio, theme, current route and open page id/title (never contents), sync state, the schedule-runner setting, page count, the last 200 log lines and recent uncaught errors. The log ring buffer (`app/lib/feedback/log_buffer.dart`) is fed by `AppLog`, `FlutterError.onError` and `PlatformDispatcher.onError`, chaining to any previous handler. The hub token and AI keys are replaced by `[redacted]`, as are `Bearer` values and token/key/secret query parameters.
- **Where reports land.** Each report is a child page of the system page "Feedback" (role `feedback`, fixed id `000000000000000000000000FB`, created on first use). The title is "Feedback yyyy-MM-dd HH:mm Platform". The body is the text, the screenshot as an image block (a normal attachment, so it syncs through the hub as a blob), then the diagnostics as a `text` code block. A toast "Feedback saved â€” thanks" confirms; it works offline and syncs later.
- **Hidden.** The Feedback page and its descendants are left out of the sidebar roots, page lists and counts, and search, including MCP `search_notes`. They are reachable by id, by link, and over MCP with `read_page` or `list_pages` with `parent` set to the Feedback id. The `role` round-trips through Markdown frontmatter ([core state](core-state.md)).

## Feedback over MCP

Agents can submit reports through the hub's HTTP `/mcp`, desktop HTTP endpoint and `notes-mcp` stdio. The in-app agent files them with the same `feedback_submit` tool (transport `in-app agent`). `feedback_list` is MCP only. Use these tools when a vault tool fails unexpectedly, the user complains about Scratch, sync or the vault, or the user asks to report something. File each issue only once per session; use the list to check recent reports.

| Tool | Inputs | Result |
|---|---|---|
| `feedback_submit` | `summary` (required, nonempty single line); `details` (optional Markdown); `kind` (`bug`, `complaint`, `idea`, `other`, default `bug`); optional strings `page_id`, `tool`, `error` | JSON with the new report's `id` and `title`. Requires write scope. |
| `feedback_list` | `limit` (default 20, max 100); optional `kind` (`bug`, `complaint`, `idea`, `other`, or `app`) | Newest first: report id, title, creation time, kind, source (`app` or `mcp`) and first line of text. Available in read scope. App reports without a kind show `app`. |

Submission reuses `Vault::feedback_page()` and creates a child titled `Feedback yyyy-MM-dd HH:mm agent` (UTC). Its body is the summary as a literal paragraph, then the details, then a `text` code block containing kind, source `mcp`, server name/version, transport (`stdio` or `http`) and the optional page/tool/error values. App reports keep their existing format. Listing uses the same hidden parent's children and never creates a Feedback page itself. It recognizes the diagnostics block even after follow-up text or code is appended.

Submitted text is scrubbed for Bearer values, token/key/secret/password URL parameters and bare 64-character hexadecimal tokens (the hub and desktop token format). The adapter collects no credentials or authentication configuration. The server does not retain initialize `clientInfo`, so client name/version are omitted. The hub knows token names for history attribution, but does not pass them into the shared tool adapter; the diagnostics omit the token label rather than adding authentication plumbing. Hub report edits retain the usual `agent:<token-name>` history attribution.

## Code

`app/lib/feedback/` (controller, sheet, host, report composition, diagnostics collector, shake detector, log buffer); `crates/core/src/feedback.rs`, `crates/tools/src/feedback.rs`, the MCP adapter and the role filters in `crates/core/src/index.rs`.

## Known limits

- Real shake on a device and hub sync of a real report image are not yet verified on hardware.
- If saving fails after the screenshot attachment is stored, the orphan blob stays until attachment cleanup.
- No macOS Cmd+Shift+F binding.
