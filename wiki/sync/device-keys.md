---
type: page
summary: "Each device holds an Ed25519 key (bubblebuffer-device-key v0.1.0) in .vault/device-key.json; the hub binds it to the device token, and sync, /agent/ws, /mcp and the app's hub calls run on 1-hour session tokens bought with a signed challenge. Agent tokens, MCP OAuth and the OIDC login are unchanged."
parent: "[[sync/index]]"
status: current
---

# Device keys

Each device proves possession of its own Ed25519 key instead of only presenting a long-lived token. It is built on [bubblebuffer-device-key v0.1.0](https://github.com/BubbleBuffer/bubblebuffer-identity/releases/tag/v0.1.0) (tag `a539c7e8`), a git dependency of `crates/sync` only.

## Why

Before this, a device was a claimed id plus a bearer token that never expired: 32 random bytes, kept as a SHA-256 in `.vault/hub/tokens.json`, stored in plain text by the app (`settings.json`) and carried in the pairing link itself. A token copied from a settings file, a log, a backup or a pairing QR worked from anywhere until someone unpaired the device.

## How it works

**The key.** `crates/sync/src/device_key.rs` keeps one plain `KeyFile` per vault at `<vault>/.vault/device-key.json` (mode 0600 on Unix), created on first need. Nothing under `.vault/` is synced, exported or sent to the hub. The library's `DeviceId` (the key id, like `K7MQ-2X9F-D4RT-8WHN`) is the bridge's `device_key_id(vault)`. The vault device id (`.vault/device`) stays as it is; the hub binds the key to it.

**The hub entry.** A `tokens.json` entry gains optional `pubkey` (hex) and `key_id`. An entry with them is *key-bound*: its bare token is refused everywhere except `POST /auth/upgrade` and `POST /auth/signout`.

**Routes** (`crates/sync/src/hub.rs`, so the embedded hub and notes-hub both serve them; answers are JSON with `Cache-Control: no-store`):

| Route | Takes | Gives |
|---|---|---|
| `POST /auth/challenge` | nothing | `{nonce}`: 32 random bytes (hex), single use, valid 60 s, in a bounded in-memory table |
| `POST /auth/session` | a proof `{device, key_id, pubkey, origin, nonce, sig}` | `{session, expires_in}` for a key bound to `device` |
| `POST /auth/upgrade` | the legacy token as bearer, plus a proof | binds the key to the token's entry, then a session; 409 if already bound |
| `POST /auth/pair` | `{code, device, pubkey, key_id}` | `{token, label}`: the redeemed pairing code's token, key-bound |

The proof signs `relay_proof_hash("scratch/session/v1", viewer, origin, nonce, key_id, pubkey)`. `origin` is the hub origin the client used (`scheme://host[:port]`, lowercase, no default port, no trailing slash); it must be the public URL's origin when the hub has one configured, else `http://<Host>` or `https://<Host>` of the request. `/health` advertises the routes with `X-Device-Keys: 1`.

**Sessions** are 32 random bytes, kept in memory as SHA-256 → (device, expiry), valid 1 hour. A hub restart drops them; clients prove again on the next 401. A session is accepted as the bearer wherever a device token was (sync, `/agent/ws`, `/host/ws`, `/mcp` with `X-Device`, `/shares`, `/agent-tokens`, `/device/name`, the web principal), with `X-Device` matching its device. Unpair, signout and a new sign-in end the device's sessions; signout also closes its sockets.

**Sign-in and pairing.**
- OIDC: `/auth/redeem` takes optional `pubkey` and `key_id`; with them the new entry is key-bound from the start.
- Add a device: `Hub::pair` (Settings > Sync and devices, `vault-hub --pair`, `notes-hub --pair`) issues a one-time code: random, single use, valid 10 minutes, stored hashed as a pending entry that never works as a bearer. The link is `notes://pair?hub=…&code=…`; the device redeems it with its key (bridge `redeem_pairing_code`). Old `token=` links still parse; the first sync upgrades that token.

**The client** (`crates/sync/src/session.rs`, `DeviceSession`): one provider per (vault, hub origin, token), shared by the sync client, the agent and host sockets, the `notes setup` CLI and the app's HTTP calls (bridge `hub_bearer` / `hub_bearer_invalidate`; Dart retries once on 401). It asks `/health`, upgrades the token if `.vault/hub-client.json` doesn't list it as bound (keyed by the token's SHA-256), proves for a session, and renews it shortly before expiry or after a 401. A hub without device keys gets the bare token, as before. The app keeps storing the token.

**Policy.** `require_device_keys` (notes-hub config, `NOTES_HUB_REQUIRE_DEVICE_KEYS`; embedded hub `NotesHub.set_require_device_keys`; default off) refuses unbound tokens, including their upgrade: such a device signs in or pairs again.

## What stays

- Agent tokens (`.vault/hub/agent-tokens.json`) for MCP clients, and MCP over OAuth (`mcp_oauth.rs`).
- The OIDC login (PKCE, the allow-list, the one-time redeem code), TLS through Caddy and the certificate pins.
- Vault device ids, node records and the sync protocol messages.

## Limits

- **Key storage decides the real gain.** The key file sits in plain text in the vault's `.vault/`, so anyone who copies the whole vault directory gets the key and the token. What it does stop:
  - tokens leaked on their own (logs, pairing QRs, screenshots, backups of `settings.json`);
  - copies of a settings file;
  - session tokens past their hour.

  Against theft of the whole device, the key file needs a passphrase held in OS storage: DPAPI on Windows, a Keystore-wrapped secret on Android. The app has no secure-storage package yet.
- **The vault device id isn't in the signed transcript.** It's bound on the hub side, through the entry that links that vault id to the public key. That's enough, because the hub accepts a key only for the vault id it was registered with.
- **The proof borrows the `viewer` role**, since v0.1.0 knows only `host` and `viewer`. A role-free proof in the library would be cleaner.
- **`bubblebuffer-link` is not used**; the LAN and Tailscale path stays plain HTTP (`sync-state.md`).
