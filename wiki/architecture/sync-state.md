---
type: topic
layer: state
summary: "The built sync (vault_sync + core hooks): hub server, binary protocol, token pairing, delta sync by version vectors, conflicts, purges, lazy blobs, numbers."
parent: "[[architecture/index]]"
status: current
covers: ["crates/sync/**", "crates/core/src/sync.rs"]
realizes: ["[[architecture/sync-and-storage]]"]
---

# Sync (state)

Part B of Core: MVP scope (internal wiki), built 2026-09-30. The topology is in [Sync and storage](sync-and-storage.md): one desktop runs the hub; other devices sync with it over Tailscale. Since 2026-10-08 the same hub logic also runs as `notes-hub` on a server behind TLS, with device tokens from an OIDC login ([Hub server](hub.md), decision 0007 (internal wiki)).

## Crates and modules

Android's Hub sign-in starts this same authenticated sync client with the browser-issued device token. The agent reads the Hub replica; mobile turns wait for a successful sync before sending, and completed runs request another sync to bring edits back. See [Android agent through the Hub](mobile-agent.md) for current wiring, local end-to-end verification and phone setup.

- `crates/core/src/sync.rs`: the core hooks (below), with conflict detection, restoration, resolution, policy, diff and archive helpers in `conflict_*.rs`. The index mirrors conflict records; the page's Loro history is authoritative.
- `crates/sync` (package `vault_sync`):
  - `hub.rs`: `Hub`. `Hub::handle(HubRequest) -> HubResponse` is the transport-free entry point: method, path, query, headers and a body reader in; status, content type, extra headers and body out. It does the routing, the token and device check and the sync handlers, and reads the body only after the token check. `Hub::serve` is a thin tiny_http adapter over it; [`notes-hub`](hub.md) is an axum one.
  - `client.rs`: `SyncClient` and the background loop.
  - `wire.rs`: the message encoding.
  - `auth.rs`: device tokens (pairing, and since 2026-10-08 login tokens and sign-out).
  - `bin/vault-hub.rs`: the command-line hub.
- Tests: `crates/core/tests/sync.rs` (8, core hooks) and `crates/sync/tests/hub.rs` (5, two vaults and a hub in one process over localhost HTTP). The bench is `crates/sync/examples/sync_bench.rs`.

## Core hooks

```
vault.page_versions() -> Result<Vec<PageVersion { page, version }>>   // trash included
vault.sync_page_versions() -> Result<Vec<PageVersion>>               // transport also advertises retained purge history
vault.export_updates(page, since: Vec<u8>) -> Result<Vec<u8>>          // since empty: a snapshot
vault.import_updates(page, bytes) -> Result<SyncImport { created, changed, conflicts, skipped }>
vault.import_updates_batch(Vec<PageUpdate { page, bytes }>) -> Result<SyncImport>
vault.purged_pages() -> Vec<PageId>
vault.import_purged(pages: Vec<PageId>) -> Result<u32>
vault.wait_changes(since_seq, timeout_ms) -> ChangeBatch               // blocking poll_changes
vault.list_conflicts(page: Option<PageId>) -> Result<Vec<Conflict>>    // pending; bridge exposes conflicts/list_conflicts aliases
vault.conflict_log(page: Option<PageId>) -> Result<Vec<Conflict>>      // resolutions and undone history, including reopened entries
vault.resolve_conflict(page, id, Keep | UseSide(u32)) -> Result<()>
vault.undo_conflict(page, id) -> Result<()>
vault.conflict_diff(page, id) -> Result<ConflictDiff>                  // display overlays only
vault.conflict_auto_resolve() -> Result<bool>
vault.set_conflict_auto_resolve(enabled: bool) -> Result<()>
vault.attachment_ids() / missing_attachments() -> Result<Vec<AttachmentId>>
vault.store_attachment(id, name, bytes) -> Result<()>                  // checks the hash
version_includes(a, b) -> Result<bool>     // version a has everything in b
sha256_hex(bytes) -> String
```

- A transport version is an encoded Loro version vector. The index keeps each page's version (`pages.vv`, schema 6), so `page_versions` does not load documents; loaded pages use their live version. Conflict sides and undo references instead carry encoded Loro **frontiers**. `page_versions` continues to list loadable pages, including Trash; `sync_page_versions` additionally advertises retained conflict-history archives without recreating their purged pages.
- `import_updates` writes the Loro file, index rows, markdown and change feed like a local edit. It works on pages that exist here ("hot": imported into the loaded document, new ops appended to the update log) and on new pages ("cold": the document is built from the bytes, a snapshot is stored as received, and the index rows are written in one transaction per batch, markdown in chunks of 64).
- Imports that change the title rerender pages linking to it. Imports that trash a page remove the markdown of its subtree; imports that restore it write it again.
- If Loro rejects a hot import after applying valid ops, those ops are kept, logged and marked dirty with the same conflict, index and notification handling as a successful import before the original error is returned.
- A cold import whose parent was purged here moves the page to the top level.

Vault writers in one process queue on a shared mutex keyed by the canonical `.vault/flush.lock` path, without a deadline or polling. After acquiring that guard, they take the OS advisory lock; contention with another process has a fresh 10-second deadline and emits one `tracing` warning after two seconds. Both locks protect each stat/merge/write/truncate operation and each append, preserving two-writer exclusion and change detection by file identity, size and the durable vault sequence. Cold imports decode pages in parallel, then install the batch's snapshots and index rows under one acquisition, with one durable sequence bump before the first snapshot write. If an import fails after writing snapshots, opening the vault or retrying the import recovers snapshots without index rows as before.

## Stable peer ids

- Each vault's Loro peer id is derived from its device id (`.vault/device`): the first 63 bits of its SHA-256. Every session of a device writes as the same peer, so version vectors do not grow per session. Restored pages derive a fresh per-device peer from their persisted recovery generation, stable across restarts, so removing purge-side history cannot make later edits reuse old operation counters.
- Peer reuse is safe only if no op counter is ever issued twice. The first `Vault` on a folder takes `.vault/lock` (`File::try_lock`) and uses the device peer. A second instance in the same or another process gets a random peer, as before.
- Update logs are not fsynced (see [Core (state)](core-state.md#edits-and-durability)). An op that was exported to another device but lost here in a crash would make this device reuse its counter. To prevent that, `export_updates` fsyncs the page's snapshot and logs if one was written in the last 120 s, and the next flush of an exported page writes its snapshot with fsync before the rename.

## Protocol

Plain HTTP on the tiny_http hub (Tailscale provides transport security); HTTPS through Caddy on `notes-hub`. The routes and bodies are the same on both. Bodies are a small binary encoding (`wire.rs`: a protocol byte, LEB128 varints, length-prefixed bytes). Every route except `GET /health` needs `Authorization: Bearer <token>` and `X-Device: <device id>`.

| Route | Purpose |
|---|---|
| `POST /sync` | Compare versions. Request: `full`, the base `(epoch, seq)`, `(page, version)` pairs, new local purges, how many hub purges the client has, attachment ids, a byte budget. Response: `updates` and the hub `versions` they bring the client to, `need` (pages the hub lacks, with its version), `changed` (hub pages changed since the base that the client did not send), hub purges, `need_blobs`, `more`. |
| `POST /push` | The client's updates for `need`, in budget-sized requests. |
| `GET /changes?since=<epoch>:<seq>&wait=<ms>` | Long-poll: returns when the hub's feed moves past `seq`, or after `wait` (at most 60 s). |
| `GET /blobs/<id>`, `PUT /blobs/<id>` | Attachments by SHA-256; the name travels in `X-Name`. The hub checks the hash. |

- The hub's `epoch` is random per run. A client whose base has another epoch, or a `seq` outside the hub's change window (10,000 changes), gets `reset` and redoes the sync in full.
- The byte budget (8 MB default) bounds each response and push. `more` asks the client for another round. Bodies over 512 MB are refused.
- `X-Purge-Device: 1` is a boolean capability advertised by both hub adapters on `/health` before mutations and selected by the client on v2 `/sync`. When selected, each purge record adds a presence byte and, when present, a length-prefixed UTF-8 device id after its optional version. Capability-off connections retain the original v1/v2 bytes. The hub persists and forwards the original purging id, leaves missing ids absent, and conflict detection falls back to `"unknown"` without it. Zstd negotiation and `/push` are unchanged.
- The `/sync` version exchange advertises and confirms `X-Sync-Capability: zstd-v1` per sync session; missing or unknown capabilities keep the original binary protocol-v1 bodies uncompressed, while confirmed support allows zstd level 3 on `/sync` and `/push` bodies (`Content-Encoding: zstd`). Bodies below 1 KiB or without a size saving stay plain, and decoding caps output at 512 MiB and the zstd window at 8 MiB; attachments remain unchanged.

## Client

- `SyncClient::new(vault, url, token)`; `sync_once()` runs rounds until neither side has anything the other lacks, then uploads attachments the hub lacks.
- **First sync:** full; sends every page's version.
- **Later syncs:** the client remembers, per page, a version the hub is known to have (what it sent and what it received). It sends only pages whose local version that does not cover, plus the pages the hub lists in `changed`. This needs no local change feed, so importing more than 10,000 pages does not force the next sync to be full.
- The client's delta state is in memory: a new `SyncClient` (an app restart) starts with a full compare, which sends every page's version but no page that is already equal.
- **Attachments are lazy.** Imports bring pages that may reference attachments this device lacks. `fetch_attachment(id)` or `fetch_missing_attachments()` download them; the background loop does the latter when `prefetch_attachments` is set.
- **Background loop:** `start(SyncOptions) -> BackgroundSync { status(), sync_now(), stop() }`. It syncs at start, after local changes (debounce 500 ms, at most 3 s after the first), when the hub's long-poll reports a change, every 60 s, and after failures with backoff up to 60 s. It uses three threads: a local feed watcher, the long-poll and the loop.
- The HTTP client does not reuse connections. With pooling, a request after a hub restart could go to a dead keep-alive connection and fail.

## Pairing and auth

- `vault-hub <dir> --pair <label>` (or `Hub::pair`) prints a random 256-bit token. The hub stores only its SHA-256, with the label, in `.vault/hub/tokens.json`. `--unpair <label>` revokes it; `--devices` lists them.
- A token binds to the first device id that uses it. Other devices get 403. A device already bound under another label gets 409, and so does a request with the hub's own device id.
- **Login tokens** (`notes-hub`, 2026-10-08): `Hub::login_device(device, name)` creates an entry of the same kind, already bound to the device. It first drops that device's earlier entries (a re-login replaces its token) and suffixes the label ` (2)`, ` (3)`, ... if another entry has it. `Hub::sign_out(token)` drops the entry of a token.
- Token hashes are compared in constant time over every entry (`vault_sync::ct_eq`).
- The hub runs one thread per request (tiny_http). `vault-hub <dir> --listen <addr>` serves until killed; `Hub::serve(addr)` embeds it and returns a `HubServer` that stops when dropped.

## Trusted certificates (2026-10-19)

- `crates/sync/src/tls.rs` (`vault_sync::tls`) has the one trust type and verifier shared by every hub client (2026-10-26):
  - sync (`SyncClient`, over ureq);
  - the remote agent's WebSocket (`/agent/ws`, `app/rust/src/agent_remote.rs`, used by the agent add-on and the schedule engine);
  - the host relay (`/host/ws`, `app/rust/src/host_relay.rs`).
  The MCP proxy (`crates/mcp/src/proxy.rs`) only talks to the app on 127.0.0.1 without TLS, so it needs none.
- The verifier first runs the normal webpki check with the bundled roots, so publicly trusted hubs behave as before.
- If that check fails with an unknown issuer (self-signed, a private CA or a TLS-inspecting proxy), the certificate is accepted when either holds:
  - **CA anchor, scoped per origin.** The trusted entry for the URL's origin (`scheme://host:port`, ws mapped to http and wss to https) carries the DER of the certificate that issued the leaf. That certificate is an extra trust anchor for this origin only, with full webpki verification: chain, hostname and validity. So the trust survives a proxy re-minting the leaf. `Trust::for_hub(url, entries)` drops the anchors of every other origin, and the verifier also checks the server name against the origin's host. A stored anchor never vouches for another host or port.
  - **Leaf pin.** The SHA-256 of the leaf matches a pinned fingerprint (the fallback, and all that entries saved before 2026-10-26 have).
- Any other failure (expired, wrong name under a known issuer) is never overridable. Handshake signatures are always verified.
- A leaf-only pin is exact, so a changed certificate fails again and the app asks again.
- Without a matching pin the request fails with `SyncError::UntrustedCertificate { sha256 }`, shown as `untrusted certificate: AB:CD:…`. `SyncClient::untrusted_certificate()` returns the presented chain (leaf first) as `CertificateInfo` (subject, issuer, validity, SHA-256, serial, self-signed), parsed with `x509-parser`.
- `SyncClient::with_trusted(vault, url, token, fingerprints)` takes leaf pins only; `SyncClient::with_trusted_certificates(vault, url, token, Vec<TrustedCertificate>)` takes the full trust (`origin`, `sha256`, `issuer_der`). `untrusted_certificate_der()` returns the presented chain as DER, so the app can keep the issuer. The bridge mirrors them as `NotesSync.withTrustedCertificates`, `untrustedCertificateDer()` and the FRB struct `TrustedHubCertificate`. `NotesAgent.connect`, `hostRelayStart` and `hostRelaySetTrust` take the same list. The Rust side stores nothing: the app keeps the trust per server origin in its device settings.
- **A running host relay** keeps its trust list in shared state and builds the verifier anew on each connection attempt. `hostRelaySetTrust` (called by `HostRelayLink` when the trust for the hub changes) replaces the list and wakes a relay waiting to retry, which then tries again at once with the new trust and a reset backoff; no app restart. A socket that is already open stays open after a revoke until it drops. The remote agent and the schedule engine include the trust in their connection keys, so they reconnect with it.
- ureq 3.4.2 has no hook for a custom verifier, so `tls.rs` carries a minimal copy of ureq's rustls connector (MIT/Apache-2.0) and `crates/sync/Cargo.toml` pins `ureq = "=3.4.2"`. HTTP CONNECT proxies still work; SOCKS proxies are not supported by the sync client.
- **App:** `app/lib/features/sync/cert_trust.dart` keeps the pins in `settings.json` (`trustedCertificates`, per device, not synced), keyed by server origin (`scheme://host:port`, lowercased, default port filled in), with the certificate details and the date trusted. When the hub sent an issuer, the pin also keeps `issuerDer` (base64) and `issuerSha256`; both fields are optional, so older settings load unchanged and give a leaf pin only. `TrustedCertificates.trustFor(url)` gives every hub client (`NotesSync`, the agent, the host relay, the schedule engine) the entry for its URL's origin.
- An `untrusted certificate:` error is shown as a sync error, not offline. The sync indicator popover, the connect status and the first-connect error say the certificate can't be verified (or that it changed since it was trusted) and offer "View certificate" and a smaller "Trust certificate". Trusting asks for confirmation with the host and fingerprint, saves the pin and reconnects.
- `certificate_view.dart` shows subject, issuer, validity (marked "Expired" in text), SHA-256 with copy, serial, self-signed, and the chain leaf first, with one action: trust, or revoke for a pinned one. Sync settings list the trusted certificates with revoke; revoking removes the pin and reconnects.
- The list names each certificate by its issuing CA: the issuer's CN as the title, `O · OU` below, then "for <host> · <fingerprint> · valid until …". An issuer without a CN keeps the host as the title, as before. The trust prompt names the CA the same way ("accept certificates for <host> issued by <CN> (<O · OU>)").
- `crates/sync/tests/tls.rs` runs the hub behind a rustls front with rcgen certificates and covers: untrusted self-signed hub, pinned hub syncing between two devices, changed certificate, pin format, and a private CA chain.
- The `hub_trust_*` unit tests in `crates/sync/src/tls.rs` and `app/rust/src/agent_remote.rs` run in-memory rustls handshakes (no network) with an rcgen private CA: the anchor accepts a re-minted leaf for the same host (also under a root), rejects a leaf for another host or another origin and an expired leaf, a leaf-only pin still works, and the WebSocket connector for `/agent/ws` and `/host/ws` uses the same trust.

## Conflicts

**Core and sync backend built and checked, 2026-10-14.** The contract is [Conflict format v2](conflict-format.md), approved 2026-10-13 with missing-parent and undo-status decisions on 2026-10-14. Types, scoped detection, resolution/undo, diff, synced policy and archive/history hooks are implemented. Core checks pass: 199 unit tests (2 ignored), 5 detection tests, 5 public resolution/history tests and 13 unchanged import-recovery tests. All 54 sync tests pass: 18 unit, 3 new real-HTTP, 5 hub, 9 purge and 19 zstd tests. These cover fresh/warm replica purge/undo, tombstone replays, restarts and background startup. FRB 2.13.0 bindings are regenerated; 21 Rust bridge and 12 MCP tests pass. Native bridge and fake-agent builds are complete; all 12 targeted Flutter tests pass and analysis reports no issues. No packaged app or deployment is claimed.

- Detection compares common, local and incoming historical versions. `Block` covers a block's own text/properties; `Subtree` covers a root and every descendant; `Page` covers a whole trashed or purged note opposed by new edits. Broader events cover their children rather than creating a separate conflict for each descendant.
- Each stable record ID keys JSON in the same page's Loro `conflicts` map. A side carries its device, timestamp, encoded frontiers, full scoped tree for display, lifecycle state (`Present`, `Deleted`, `Trashed { at }`, `Purged { tombstone_version }`) and, for block/subtree scope, its parent and previous-sibling anchor. Frontiers remain authoritative. The index mirrors record ID, anchor block, scope and JSON; it is not the source of truth.
- Delete versus edit applies the editing side's **entire** version at detection, including descendants or unchanged content the deleting device removed. With auto-resolve off, that already-applied entry stays pending until OK. With it on, the entry goes straight to the resolved log. Text edited on both sides takes one side's own content at detection (later `time_ms`, then the greater device id; `auto(later-text-wins)`, undoable) and always waits for a user's pick; see [Conflict format v2](conflict-format.md).
- `Keep` confirms the current scope; `UseSide(i)` makes it match the recorded side at index `i`. There is no `Replace` or AI resolution in the v2 backend. Side order stays stable for `UseSide`; display diffs identify Local by this vault's device ID and retain both foreign flags as false when neither side is Local. Diffs are overlays and never drive application of a choice.
- Every application records `resolution: Keep | UseSide(i)` (the applied side for automatic rules; unknown for old records), the historical `before` frontiers, a non-live before-state when needed, `by = user` or `auto(rule)`, restored ancestor IDs and any root fallback. Undo restores only the conflict scope to that before-state and preserves unrelated edits. The user settled status on 2026-10-14: ordinary undo returns the record to pending review and marks the consumed application with `undone_at` in history and the log; journal undo remains log-only. Restoration rebuilds only the missing ancestor chain required by the saved anchor, in its original order and position; undo removes ancestors introduced by that application. If no version can rebuild an ancestor, scoped content appends at the page root and the log records the fallback. A missing previous sibling otherwise appends within the saved parent.
- Actual Journal day-section repairs create one `auto(journal)` entry for the restoration, using its historical deletion/edit frames. It is log-only regardless of the switch, and the same repair footprint suppresses duplicate delete/edit conflict entries.
- The single vault-wide switch is stored as `sync.conflict_auto_resolve` type-data on the hidden Calendar settings page, so it syncs as ordinary page state. Its default is off. Enabling it dismisses pending delete/edit entries whose rule was already applied; it does not apply them twice or dismiss text/text conflicts.
- Resolutions and log changes sync through the page's Loro document. Resolved records expire 30 days after their latest application or undo. Consumed application metadata on reopened pending records expires 30 days after undo; pending records themselves are never age-trimmed. The underlying page history keeps every version, so after log expiry the user still has page-history restoration.
- Purging a conflict-bearing page retains its document/history in `.vault/conflict-history/<page>.loro`, together with the records and lifecycle metadata needed for review and scoped undo. Transport advertises those archives through `sync_page_versions` and synchronizes their metadata without making the deleted note active. A warm offline replica can receive the tombstone before the archive: protocol v2 retains the explicit page comparison after deleting its old baseline and requests a full archive with an empty version. `list_versions` and `load_page_at` read retained history without restoring the note; explicit `restore_version` records a new restoration decision. Active-page history includes retained archive branches. Format-1 records still read as block scope with present/deleted sides, no historical frontiers, and no conflict undo.

**Post-merge focused checks:** after master fixes `16e6080` and `11c2c0b`, all 3 HTTP conflict-history tests pass again, together with 8 journal and 13 unchanged import-recovery tests. The full workspace gate passed on the Linux machine on 2026-10-14 after detector fix `3093be8`: 775 passed, 5 ignored, 0 failed. It ran `cargo test --workspace --locked --exclude notes-tauri` through `<linux-build-helper> --with ../BubblePaw-base-agent`. The settled undo contract is implemented; the HTTP tests prove ordinary pending review plus undone log/history on the hub and both devices. The new [merge screen](../../design/merge/index.html) is a separate later build, with Local/Cloud labels and no AI/manual-merge control. Existing Flutter block controls are interim compatibility that perform an ordinary text edit followed by `Keep` and pass all 12 targeted Flutter tests with clean analysis, including the post-merge rerun.

## Deletion

- Trash is a property of the page document, so it syncs like any edit.
- Permanent deletion stores purge-time version vectors and restoration acknowledgements in `.vault/purge-state.json`, alongside the compatible ID list `.vault/purged`. Unseen user edits restore the editor's whole page and history, live and out of Trash, including content the deleting device removed. Restored and purge-candidate pages use full snapshots (empty `since`) in either sync direction; ordinary pulls and pushes stay deltas. Purge-only branch operations are excluded, covered stale copies stay gone, and durable acknowledgements clear tombstones and suppress replays. Replacement reports an explicit error when independent edits cannot be preserved.
- Legacy ID-only tombstones recover any incoming copy, edited or unedited, into Trash with the persisted page marker `purge_restore_warning = "came back after permanent delete"` and clear the tombstone. A device already holding the page keeps and marks its copy. Re-purge writes a versioned tombstone that blocks stale copies. The merge screen should later list these legacy recoveries; `restored_after_purge_pages` exposes their IDs.
- Leftover snapshots and logs covered by an active purge's version are deleted content: index recovery leaves them unindexed, and they do not block an incoming restoration. Uncovered user edits in leftover files restore the page live through the same full-history restoration path. Legacy leftovers are treated as covered and never restore a page on their own; only an incoming copy triggers legacy recovery into marked Trash. The replacement guard continues to reject discarding independent edits on live pages.
- After a page has been restored from any purge, further ID-only purge records are ignored, including v1 full-sync replays after an old client restarts. A deliberate new purge from an old client is also ignored because it cannot be distinguished from a replay. Versioned purges retain their coverage rule and can delete a restored page when their version covers it. Restoration acknowledgements are not trimmed.
- Protocol v2 is negotiated through `X-Purge-Protocol: 2` on `/health` before mutations. Mixed old/new client and hub connections otherwise retain v1 delete-wins behavior. A new client logs a `tracing` warning once per hub connection when it falls back to v1: purge recovery is off against that hub and edits to permanently deleted pages may be dropped.

## Measured (2026-09-30)

Release build, Windows desktop under load from other jobs, hub and client in one process over localhost. The vault is `seed::generate(10,000 pages + a 2,000-block page)`: 10,001 pages, 58.8 MB on disk.

| Step | Time |
|---|---|
| Core hooks only: export every page (24.2 MB) | 3.1–3.4 s |
| Core hooks only: import into an empty vault, batches of 500 | 28.5–30.2 s |
| Through the hub: full sync into an empty vault (3 rounds, 24.9 MB down, 0.42 MB up) | 30.5–31.6 s |
| Delta sync, nothing changed | p50 22 ms, max 30 ms |
| Delta sync, one page edited on the hub | p50 56 ms, max 147 ms |
| Delta sync, one page edited on the client | p50 35 ms, max 133 ms |

- The client vault is 67.8 MB after the full sync.
- Import time is dominated by creating and renaming about three files per page (Loro file, markdown, temporaries), as in `seed::generate` (22 s for the same vault). Decoding runs on up to 8 threads; the markdown and index pass is serial.
- The first export after a seed or heavy editing is slower (10 s in one run), because recently written pages are fsynced (see stable peer ids).

## Build

- **TLS (2026-10-08):** `ureq` has its `rustls` feature (rustls with ring and the bundled webpki roots), so `SyncClient` talks to `https://` hubs on Windows, Linux and Android alike. The scheme comes from the stored hub URL; nothing in the Rust code assumes `http://`. Since 2026-10-19 the client builds its own TLS connector to support [trusted certificates](#trusted-certificates-2026-10-19).
- Original dependencies (all pure Rust, no async runtime): `tiny_http` 0.12 without default features (server), `ureq` 3 without default features (client, plain HTTP), `getrandom` 0.3 (tokens, epoch). They add `ascii`, `base64`, `chunked_transfer`, `http`, `httparse`, `httpdate`, `percent-encoding`, `ureq-proto` and `utf8-zero` to the lock file.
- `cargo test -p vault_core -p vault_sync` runs 41 core tests (12 unit, 10 `api`, 19 `mvp`), 8 core sync tests, 3 sync unit tests and 5 hub tests. Clippy is clean.
- `cargo check -p vault_core -p vault_sync --all-targets` passes for `x86_64-linux-android` and `aarch64-linux-android` with NDK 27.3 (environment in [Core (state)](core-state.md#build)).

## Known limits

- The tiny_http hub has no TLS: use it only on a trusted network such as Tailscale. `notes-hub` runs behind a TLS proxy.
- Trusted certificates apply to every hub client: sync, the agent's WebSocket and the host relay (since 2026-10-26).
- Client delta state is in memory; each app start does one full version compare.
- The hub's delta relies on its in-memory feed. After a hub restart, or more than 10,000 hub changes since a client's last sync, that client does a full compare.
- The fsync that protects peer-id reuse covers files written in the last 120 s and does not sync directory entries. A crash right after an export could, in theory, still lose ops that another device has, and the device would then reuse their counters.
- Attachments are fetched whole; there is no resume for large files.
- Purge/restoration acknowledgements retain their existing lifetime. V2 policy trims resolved log records and consumed undo metadata after 30 days, keeps pending records and retains all page history; see [Conflicts](#conflicts).
- One thread per hub request; fine for a few devices, not for many.
