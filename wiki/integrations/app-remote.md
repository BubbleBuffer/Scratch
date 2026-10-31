---
type: topic
summary: "The desktop app's local remote-control endpoint and the scratch-cli app commands that drive it."
parent: "[[integrations/index]]"
status: current
---

# App remote control

`scratch-cli app ...` drives a running desktop app (Windows and Linux): status, open a page, navigate, screenshot, a semantic dump of the visible UI, tap, type and key presses. It exists so an agent or script can use the app without a screen. The endpoint lives in the Dart app, because only the Flutter side can render frames and act on widgets. It is separate from the vault MCP server (`crates/mcp`), which is Rust-only, off by default and has no path to the UI.

## Endpoint

- The app binds `127.0.0.1` on a random port at start (desktop only; off when `SCRATCH_REMOTE=0`). Nothing listens on another interface.
- It writes `<data dir>/remote/endpoint.json`: `{"port": N, "token": "<64 hex>", "pid": N, "version": "x.y.z"}`. The token is random per start. On Linux the file is mode 0600 and the folder 0700; on Windows the per-user data folder's ACL applies. The app deletes the file when it stops. A file left by a crashed app is detected by a refused connection.
- The data dir is the app's own: `NOTES_DATA_DIR`, else `%LOCALAPPDATA%\Notes` on Windows, else `~/.local/share/notes_app`. `scratch-cli app --data-dir <DIR>` overrides it.

## Protocol (v1)

`POST http://127.0.0.1:<port>/v1/<command>`, header `Authorization: Bearer <token>`, JSON request body (`{}` when empty), JSON response with `Content-Length` and `Connection: close`.

Success: HTTP 200 with `{"ok": true, ...}`. Failure: `{"ok": false, "error": "<message>"}` with 401 (missing or wrong token; the token is compared in constant time), 400 (bad request body or arguments), 404 (unknown command, page or element id) or 500. Non-POST requests get 405.

| Command | Request | Success fields |
|---|---|---|
| `status` | `{}` | `version`, `pid`, `vault` (`{"open": bool, "name": str or null}`), `page` (`{"id","title"}` or null) |
| `open` | `{"page": "<id or exact title>"}` | `page` (`{"id","title"}`); 404 if no page matches |
| `navigate` | `{"route": "<route>"}` | `route`; routes: `home`, `settings`, `search`, `back`, `page/<id>`; other values are 400 and the error lists the routes |
| `screenshot` | `{"pixel_ratio": 1.0}` (optional) | `width`, `height`, `png_base64` (the app's own frame, rendered from its root RepaintBoundary) |
| `tree` | `{}` | `nodes`: list in reading order of `{"id","role","label","value","hint","actions":[...],"enabled","focused","rect":{"x","y","w","h"}}` for each visible semantics node with a label, value or action. `role` is one of `button`, `textfield`, `link`, `checkbox`, `switch`, `slider`, `image`, `header`, `text`, `other` |
| `tap` | `{"id": "<id>"}` | `{}`; moves focus to the node (when it can take focus), then performs its tap action; 404 if the id is not in the current tree |
| `type` | `{"id": "<id>", "text": "...", "replace": true}` | `{}`; focuses the field and sets or inserts text (`replace` defaults to true) |
| `key` | `{"combo": "ctrl+k"}` | `{}`; Enter on a focused single-line text field also sends the field its submit action (a key event alone never does); modifiers `ctrl`, `shift`, `alt`, `meta` joined with `+`, then a key name (`enter`, `escape`, `tab`, `up`, `down`, `left`, `right`, `backspace`, `delete`, `space`, a letter, a digit, `f1`..`f12`) |

Element ids: a node's `Semantics.identifier` when the widget sets one, else `n<semantics node id>`. Ids from `tree` are valid until the UI changes; take a fresh `tree` after navigating.

## CLI

All commands take `--json` (print the response body as is) and `--data-dir <DIR>`.

- `scratch-cli app status`
- `scratch-cli app open <PAGE-ID-or-TITLE>`
- `scratch-cli app navigate <ROUTE>`
- `scratch-cli app screenshot <OUT.png>`
- `scratch-cli app tree`
- `scratch-cli app tap <ID>`
- `scratch-cli app type <ID> <TEXT>`
- `scratch-cli app key <COMBO>`

Without `--json` each prints short text lines. If the app is not running (no endpoint file, or the connection is refused) the command exits non-zero with `Scratch is not running (no endpoint at <path>); start the app first`. A rejected token or any `ok: false` response exits non-zero with the app's error message. `scratch-cli app start` is not provided: the app is a full Flutter desktop window, and starting it hidden is not a cheap, portable change.
