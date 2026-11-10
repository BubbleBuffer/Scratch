---
type: topic
layer: state
summary: "The built sync (vault_sync plus the core hooks): hub server, binary protocol, device pairing, delta sync by version vectors, trusted certificates, conflicts and purges."
parent: "[[sync/index]]"
status: current
covers: ["crates/sync/**", "crates/core/src/sync.rs"]
realizes: ["[[sync/sync-and-storage]]"]
---

# Vault sync

Devices share a vault by syncing each page's Loro document with a hub over HTTP. One desktop runs the hub, or the server runs `notes-hub` behind TLS ([Hub server](hub.md)); every other device runs a client. The topology is in [Sync and storage](sync-and-storage.md). The code is `crates/sync` (package `vault_sync`) and the hooks in `crates/core/src/sync.rs`.

## Modules

- `crates/core/src/sync.rs`: the core hooks (below). Conflict detection, restoration, resolution, policy, diff and archive helpers are in `conflict_*.rs`; the index mirrors conflict records, and the page's Loro history is authoritative.
- `crates/sync/src`:
  - `hub.rs`: `Hub`. `Hub::handle(HubRequest) -> HubResponse` is the transport-free entry point: it routes, checks the token and device, runs the sync handlers and reads the body only after the token check. `Hub::serve` is a thin `tiny_http` adapter over it, one thread per request; [`notes-hub`](hub.md) is an axum one.
  - `client.rs`: `SyncClient` and the background loop.
  - `wire.rs`, `encoding.rs`: the binary message encoding and zstd compression.
  - `auth.rs`, `keys.rs`, `session.rs`, `device_key.rs`: device tokens, device keys and sessions ([Device keys](device-keys.md)).
  - `tls.rs`: trusted certificates (below).
  - `bin/vault-hub.rs`: the command-line hub.
- Tests are in `crates/sync/tests`; the bench is `crates/sync/examples/sync_bench.rs`.

## Core hooks

The `Vault` methods that sync uses:

- `page_versions` lists each page's version, trash included, from the index (`pages.vv`) without loading documents. `sync_page_versions` also advertises retained purge and conflict history.
- `export_updates(page, since)` returns a Loro update; an empty `since` gives a snapshot. `import_updates` and `import_updates_batch` apply updates and report created, changed and conflicting pages.
- `purged_pages`, `import_purged`, `purge_records` and `import_purge_records` carry permanent deletions.
- `wait_changes`, `poll_sync_changes` and `wait_sync_changes` are the change feeds; the sync feed has its own cursor and window.
- `list_conflicts`, `conflict_log`, `resolve_conflict`, `undo_conflict`, `conflict_diff`, `conflict_auto_resolve` and `set_conflict_auto_resolve` serve the conflict UI.
- `attachment_ids`, `missing_attachments` and `store_attachment` (which checks the hash) serve blob sync. `version_includes(a, b)` and `sha256_hex` are free functions.

A transport version is an encoded Loro version vector. Conflict sides and undo references carry encoded Loro frontiers instead.

An import writes the Loro file, index rows, markdown and change feed like a local edit. It is hot for a page that exists here (imported into the loaded document, new ops appended to its update log) and cold for a new page (the document is built from the bytes, the snapshot is stored as received, and index rows are written in one transaction per batch). Imports that change a title rerender pages linking to it; imports that trash a page remove the markdown of its subtree. A cold import whose parent was purged here puts the page at the top level.

## Stable peer ids

- A vault's Loro peer id is derived from its device id (`.vault/device`), so every session of a device writes as the same peer and version vectors do not grow per session.
- Peer reuse is safe only if no op counter is issued twice. The first `Vault` on a folder takes `.vault/lock` and uses the device peer; a second instance gets a random peer.
- Update logs are not fsynced. To stop a device from reusing counters after a crash, `export_updates` fsyncs the page's snapshot and logs if one was written in the last 120 s, and the next flush of an exported page fsyncs its snapshot before the rename.

## Protocol

Plain HTTP on the `tiny_http` hub (Tailscale provides transport security) and HTTPS through Caddy on `notes-hub`; routes and bodies are the same. Bodies use a small binary encoding (`wire.rs`: a protocol byte, LEB128 varints, length-prefixed bytes). Every route except `GET /health` and the `/auth/*` routes needs `Authorization: Bearer <token>` and `X-Device: <device id>`.

| Route | Purpose |
|---|---|
| `POST /sync` | Compare versions. The request carries `full`, the base `(epoch, seq)`, `(page, version)` pairs, new local purges, how many hub purges the client has, attachment ids and a byte budget. The response carries `updates` and the hub `versions` they bring the client to, `need` (pages the hub lacks), `changed` (hub pages changed since the base that the client did not send), hub purges, `need_blobs` and `more`. |
| `POST /push` | The client's updates for `need`, in budget-sized requests. |
| `GET /changes?since=<epoch>:<seq>&wait=<ms>` | Long-poll: returns when the hub's feed moves past `seq`, or after `wait` (at most 60 s). |
| `GET /blobs/<id>`, `PUT /blobs/<id>` | Attachments by SHA-256; the name travels in `X-Name`. The hub checks the hash. |
| `/auth/challenge`, `/auth/session`, `/auth/pair`, `/auth/upgrade` | Device keys and pairing ([Device keys](device-keys.md)). |

- The hub's `epoch` is random per run. A client whose base has another epoch, or a `seq` outside the hub's change window, gets `reset` and redoes the sync in full.
- The byte budget (8 MB default) bounds each response and push; `more` asks for another round. Bodies over 512 MB are refused.
- Capabilities are negotiated before use. `X-Purge-Protocol: 2` selects purge protocol v2; `X-Purge-Device: 1` adds the purging device id to purge records; `X-Sync-Capability: zstd-v1` allows zstd (level 3) on `/sync` and `/push` bodies, with `Content-Encoding: zstd`. A peer without a capability gets the original uncompressed v1 bytes. Small bodies and bodies that do not shrink stay plain, and decoding caps output at 512 MiB.

## Client

- `SyncClient::new(vault, url, token)`; `sync_once()` runs rounds until neither side has anything the other lacks, then uploads attachments the hub lacks.
- The first sync is full and sends every page's version. Later syncs send only pages whose local version is not covered by the version the client knows the hub has, plus the pages the hub lists in `changed`.
- The client's delta state is in memory, so each app start does one full version compare; it sends every page's version but no page that is already equal.
- Attachments are lazy. `fetch_attachment(id)` and `fetch_missing_attachments()` download them; the background loop does the latter when `prefetch_attachments` is set (default off).
- `SyncClient::fetch_pages(ids) -> FetchOutcome` (`Fetched`, `NotOnHub`, `Offline`, `Unauthorized`, `Error`) pulls pages ahead of the sync, for a link to a page that has not synced yet. It asks `GET /changes` for the hub's position and sends one `POST /sync` with `full = false` and an empty version for each page, so the hub answers with those snapshots only. Missing ancestors are fetched the same way, up to 16 levels; sub-pages and attachments are not. It never reads or writes the client's sync state, so it does not wait for a running `sync_once`. A page that is trashed on the hub, or purged there or here, is `NotOnHub`. The app side is `NotesSync.fetch_pages` and `SyncStore.fetchPages` ([links](../../docs/notes/links.md#links-from-outside-the-vault)).
- The background loop is `start(SyncOptions) -> BackgroundSync { status(), sync_now(), stop() }`. It syncs at start, after local changes (debounce 500 ms, at most 3 s after the first), when the long-poll reports a change, every 60 s, and after failures with backoff up to 60 s.
- The HTTP client does not reuse connections: after a hub restart a pooled request could go to a dead keep-alive connection and fail.

## Pairing and auth

- `vault-hub <dir> --pair <label>` (or `Hub::pair`) prints a one-time pairing code, valid for 10 minutes and redeemed at `/auth/pair` with the device's key. `--unpair <label>` revokes a device and `--devices` lists them. `vault-hub <dir> --listen <addr>` serves until killed; `Hub::serve(addr)` embeds a hub and returns a `HubServer` that stops when dropped.
- The hub stores only token hashes, with the label, in `.vault/hub/tokens.json`, and compares them in constant time (`vault_sync::ct_eq`).
- A token binds to the first device id that uses it. Other devices get 403. A device already bound under another label gets 409, and so does a request with the hub's own device id.
- Each vault holds an Ed25519 key in `.vault/device-key.json`. After pairing the client sends 1-hour sessions (`DeviceSession`); a legacy token is bound to the key on its first sync (`/auth/upgrade`) and the bare token is refused from then on. See [Device keys](device-keys.md).
- `notes-hub` creates tokens at OIDC login: `Hub::login_device(device, name)` replaces that device's earlier entries, and `Hub::sign_out(token)` drops one.

## Trusted certificates

`crates/sync/src/tls.rs` has the one trust type and verifier shared by every hub client: sync (`SyncClient`), the remote agent's WebSocket (`/agent/ws`, `app/rust/src/agent_remote.rs`) and the host relay (`/host/ws`, `app/rust/src/host_relay.rs`). The MCP proxy only talks to the app on 127.0.0.1 without TLS and needs none.

- The verifier first runs the normal webpki check with the bundled roots, so publicly trusted hubs work unchanged.
- If that fails with an unknown issuer (self-signed, a private CA or a TLS-inspecting proxy), the certificate is accepted when either holds:
  - **CA anchor, scoped per origin.** The trusted entry for the URL's origin (`scheme://host:port`, ws mapped to http and wss to https) carries the DER of the certificate that issued the leaf. That certificate is an extra trust anchor for this origin only, with full webpki verification of chain, hostname and validity, so the trust survives a proxy re-minting the leaf. `Trust::for_hub(url, entries)` drops the anchors of every other origin.
  - **Leaf pin.** The SHA-256 of the leaf matches a pinned fingerprint. A leaf pin is exact: a changed certificate fails again and the app asks again.
- Any other failure (expired, wrong name under a known issuer) is never overridable. Handshake signatures are always verified.
- Without a matching pin the request fails with `SyncError::UntrustedCertificate { sha256 }`. `SyncClient::untrusted_certificate()` returns the presented chain (leaf first) as `CertificateInfo`; `untrusted_certificate_der()` returns it as DER so the app can keep the issuer.
- `SyncClient::with_trusted` takes leaf pins; `with_trusted_certificates` takes the full `TrustedCertificate { origin, sha256, issuer_der }` list. The bridge mirrors both, and `NotesAgent.connect`, `hostRelayStart` and `hostRelaySetTrust` take the same list. The Rust side stores nothing.
- A running host relay keeps its trust list in shared state and builds the verifier anew on each connection attempt. `hostRelaySetTrust` replaces the list and wakes a relay that is waiting to retry; an open socket stays open after a revoke until it drops.
- `tls.rs` carries a minimal copy of ureq's rustls connector because ureq has no hook for a custom verifier, and `crates/sync/Cargo.toml` pins `ureq` to an exact version. HTTP CONNECT proxies work; SOCKS proxies are not supported by the sync client.
- **App:** `app/lib/features/sync/cert_trust.dart` keeps the trust in `settings.json` (`trustedCertificates`, per device, not synced), keyed by server origin. Each entry has the certificate details, the date trusted and, when the hub sent an issuer, `issuerDer`. An `untrusted certificate:` error is shown as a sync error, not offline, with "View certificate" and "Trust certificate"; trusting asks for confirmation with the host and fingerprint, saves the pin and reconnects. Sync settings list the trusted certificates with revoke.

## Conflicts

The format is in [Conflict format v2](../core/conflict-format.md). What sync adds:

- Detection compares the common, local and incoming versions on import. Scopes are `Block`, `Subtree` and `Page`.
- Each record lives in the page's Loro `conflicts` map, so resolutions and log changes sync with the page. A side carries its device, timestamp, frontiers, the scoped tree, a lifecycle state and placement anchors.
- Delete versus edit applies the editing side's entire version at detection. Text edited on both sides takes one side's content (later `time_ms`, then the greater device id) and waits for the user's pick.
- `Keep` confirms the current scope and `UseSide(i)` makes it match side `i`. Undo restores only the conflict scope to its before-state and returns the record to pending review.
- The vault-wide auto-resolve switch is `sync.conflict_auto_resolve` type data on the hidden Calendar settings page, default off, so it syncs as page state.
- Resolved records expire 30 days after their latest application or undo; pending records are never trimmed, and page history keeps every version.
- Purging a conflict-bearing page keeps its document, records and lifecycle in `.vault/conflict-history/<page>.loro`. `sync_page_versions` advertises these archives without recreating the page. `list_versions` and `load_page_at` read them; `restore_version` restores explicitly.

## Deletion

- Trash is a property of the page document and syncs like any edit.
- Permanent deletion stores purge-time version vectors and restoration acknowledgements in `.vault/purge-state.json`, next to the ID list `.vault/purged`. Unseen edits restore the editor's whole page and history, live and out of Trash, including content the deleting device removed. Restored and purge-candidate pages travel as full snapshots in either direction.
- A legacy ID-only tombstone sends an incoming copy to Trash with the page marker `purge_restore_warning`. `restored_after_purge_pages` exposes those ids.
- After a page has been restored from any purge, ID-only purge records for it are ignored.
- Purge protocol v2 is negotiated through `X-Purge-Protocol: 2` on `/health`. Against a hub without it the client falls back to v1 delete-wins and logs a warning once per connection: edits to permanently deleted pages may be dropped.

## Limits

- The `tiny_http` hub has no TLS; use it only on a trusted network such as Tailscale. `notes-hub` runs behind a TLS proxy.
- The hub's delta relies on its in-memory feed. After a hub restart, or when more changes than the feed window have happened since a client's last sync, that client does a full compare.
- The fsync that protects peer-id reuse covers files written in the last 120 s and does not sync directory entries.
- Attachments are fetched whole; large files have no resume.
- One thread per hub request: fine for a few devices, not for many.
