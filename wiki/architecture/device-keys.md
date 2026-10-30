---
type: page
summary: "Proposal: each device gets an Ed25519 key from bubblebuffer-identity; pairing and OIDC sign-in register it, and sync and /agent/ws prove possession for a short-lived session token. Agent tokens, MCP OAuth and the OIDC login stay."
parent: "[[architecture/index]]"
status: proposed
---

# Device keys (proposal)

**Status:** proposed 2026-10-30, not decided, not built. It depends on [bubblebuffer-identity](https://github.com/BubbleBuffer/bubblebuffer-identity) v0.1.0, which isn't tagged yet. Once it's decided, the decision goes in the internal wiki and this page becomes the description of how it works.

## Why

Today a device is a claimed id plus a bearer token:

- **The device id isn't a secret.** It's a random 64-bit id in `.vault/device` (`crates/core/src/vault.rs:697-712`). The `X-Device` header only has to match the id the token was bound to on first use (`crates/sync/src/auth.rs`, `check`/`judge`).
- **The device token never expires.** It's 32 random bytes, stored on the hub as a SHA-256 in `.vault/hub/tokens.json`, with no expiry. It's replaced only when the device signs in again or is unpaired.
- **Clients keep it in plain text.** The app stores the token in `settings.json` in its data dir (`app/lib/features/settings/settings_store.dart`), and the `notes` CLI in `notes/config.toml`.
- **The pairing link carries the long-lived token itself:** `notes://pair?hub=…&token=…`.

So a token copied from a settings file, a backup, a log or a pairing QR works from anywhere until someone unpairs that device.

## What changes

1. **A key per device.** On first launch the app creates an Ed25519 device key with `bubblebuffer-device-key`. Its fingerprint is shown in Settings › Devices. The vault device id stays as it is: node records and history are keyed by it (`crates/core/src/ids.rs:108-112`). The key is bound to that id; it doesn't replace it.
2. **Pairing and sign-in register the public key.**
   - **OIDC:** `/auth/redeem` takes the device's public key, alongside the one-time code it already takes. The hub stores the key in the device's `tokens.json` entry, next to the token hash.
   - **Add a device:** the pairing link and QR carry a one-time pairing code that expires after about 10 minutes, instead of a long-lived token. The new device redeems it with its public key, as with OIDC.
3. **Sessions instead of a bare long-lived token.**
   - The device asks the hub for a challenge (`POST /auth/challenge`).
   - It signs the challenge, its device id and the hub URL, and sends the signature back (`POST /auth/session`).
   - It gets a session token that lasts about 1 hour and is bound to that device id.
   - Sync requests and `/agent/ws` send the session token as today's bearer does, so the sync protocol and `Hub::handle` keep their shape.
   - `/agent/ws` checks the session token when the connection opens.

   This is one signature per hour, not one per request, which keeps sync's cost the same.
4. **The device token becomes the key binding.** The hub entry holds the public key, label, account and last-seen time. Unpair removes the entry, which ends that device's sessions too. Signing in again replaces the key, as it replaces the token today.

## What migrates

- **Paired devices** keep working. The hub accepts both kinds for a while:
  - On its next connect, a device holding a legacy token creates its key and calls `POST /auth/upgrade`. That call sends the legacy token once, plus its public key and a signature over a fresh challenge. The hub marks the entry key-bound and stops accepting that bare token.
  - A hub setting `require_device_keys` (off at first) later rejects any legacy token that hasn't been upgraded. A device that missed the window signs in or pairs again.
- **Agent tokens** (`.vault/hub/agent-tokens.json`) stay as they are. They're already named, scoped, optionally expiring and accepted only on `/mcp`. They're meant for MCP clients such as the Claude plugin, which can only send a static bearer or do OAuth.
- **MCP over OAuth** (Authentik JWTs, `mcp_oauth.rs`) stays as it is.
- **The device-token path on `/mcp`** (`X-Device`, jobs tool only) switches to session tokens with everything else.
- **The `notes` CLI** keeps working with an agent token. Giving it its own device key later is possible, but it's a separate choice.

## What stays

- The OIDC login against Authentik: PKCE, the allow-list, and the one-time redeem code. Only what it binds changes.
- TLS through Caddy for the public hub, and the client certificate pins.
- Vault device ids and node records.
- The sync protocol itself.

## Limits

- **Key storage decides the real gain.** If the key file sits in plain text next to `settings.json`, anyone who copies the whole data dir gets both. What it does stop:
  - tokens leaked on their own (logs, pairing QRs, screenshots, backups of `settings.json`);
  - copies of a settings file;
  - session tokens past their hour.

  Against theft of the whole device, the key needs OS-backed storage: DPAPI on Windows, a Keystore-wrapped secret on Android. The library protects the key file with a passphrase, so that passphrase would live in OS storage. The app has no secure-storage package yet.
- **`bubblebuffer-link` (Noise pairing and tunnel) doesn't fit yet.** Scratch has no device-to-device sync: other devices connect to the hub, or to the hub built into the desktop app. Over LAN or Tailscale that connection is plain HTTP (`sync-state.md`, "trusted network only"). A Noise tunnel could secure that path, but it changes the transport for every client, so it's a separate decision. Tailscale already encrypts most of those connections.

## Size

- **Core and hub:**
  - a new key-store module in `crates/sync`;
  - three hub routes (`challenge`, `session`, `upgrade`) and the key field in `tokens.json`;
  - session checks in `Hub::handle` and `/agent/ws`.
- **App:** key creation and storage, the upgrade at connect, the fingerprint in Settings › Devices, and the pairing-code change in `pairing_codec.dart`.
- **Tests:**
  - an upgrade from a legacy token;
  - rejection of a replayed signature or a wrong key;
  - expiry of a session token;
  - unpair ending live sessions.

## Open questions

1. Is plain-file key storage enough for the first version, or should OS-backed storage come in the same change?
2. Should sessions last 1 hour, or longer for phones that sleep?
3. When should `require_device_keys` default to on: after one release, or never automatically?
4. Should the `notes` CLI get a device key, or stay on agent tokens?

Found while reading the code, and not part of this proposal: connecting to a hub doesn't check for `https` on the client side (`app/lib/features/sync/sync_control.dart:94-108`). Yet `mobile-agent.md` says release Android builds require HTTPS. This needs checking.
