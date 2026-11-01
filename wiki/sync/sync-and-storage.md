---
type: topic
layer: state
summary: "Local-first sync with CRDTs, attachments by hash, page history and cold history storage."
parent: "[[sync/index]]"
status: current
---

# Sync and storage

## Local-first

Every device holds its own copy of the vault (the phone possibly without all attachments) and works fully offline. Devices exchange changes when they can reach each other.

On open or an import retry, durable page snapshots missing an index row are indexed with their pending logs under the flush lock; existing rows are retained and purge tombstones prevent resurrection.

## CRDTs

Each page's content is a CRDT, a data structure that merges concurrent edits without a central authority. The library is [Loro](https://loro.dev): Rust core, JavaScript bindings, rich text and movable trees, and built-in version history.

## History

- Versioning uses the CRDT's own history instead of git. It records every change, restores any earlier version of a page, and works the same on Android.
- Git is line-based, awkward with binary attachments and heavy on Android, and it cannot merge two offline edits of the same paragraph cleanly.

## Attachments

- Files are stored by content hash, so duplicates cost nothing and renames are free.
- The desktop keeps every file. The phone downloads a file when it is opened and caches it; files or folders can be pinned for offline use.
- Storage is the user's own disks: free, and limited only by disk size.

## Hot and cold storage (proposed 2026-10-12)

Measured 2026-10-12 (read-only probe, bench vault of 10,001 pages and the real 12-page vault):
- Nothing is trimmed or compressed today. Every `.loro` file keeps its full history; history is about 40-45% of its bytes (probe total 44%; bench and real vault alike, and it will grow with real editing).
- zstd saves about 41% on page snapshots on top of what Loro does.
- A phone's first sync receives every page's full history (`sync.rs` ~288-317); there is no transport compression.
- The search index is the biggest item: about 2.9x the markdown text, three quarters of it FTS (a full second copy of the text plus the token index).

- **Cold history (built 2026-10-13):** each page keeps a compressed, device-local history prefix and a shallow hot snapshot. History, exports to older or fresh peers, and imports with outdated dependencies automatically reconstruct full history. Publication uses the cross-instance flush protocol, including the two-writer fix merged from master; the original edit-loss regression now passes. Acceptance is blocked by the separate interrupted new-page import recovery defect below.

### Cold-history implementation and validation (2026-10-13)

The lane now contains `cold.rs`: one `.vault/cold/<page>.cold` file, with `VCLD` magic, format version 1, dictionary id or none, original root frontiers/VV, lengths and checksums, then zstd level 3 of the original Loro updates. The prefix is readable independently and preserves operation IDs. Writes sync a temporary file before replacing cold, and cold must be durable before publishing hot. Unix also syncs the directory; Windows uses a write-through replacement (not built or runtime-tested in this run).

The first dictionary is trained after 20 cold files, from at most 64 KiB per page, with an 8 KiB dictionary limit. It lives at `.vault/cold/dictionaries/<id>.dict`, stays immutable and is never removed by page purge. Existing plain files remain readable. There is no automatic retraining after successful training.

The settled defaults are `COLD_AFTER = 30 days` and `COLD_MIN_BYTES = 16 KiB` of uncompressed eligible updates. `.vault/cold-policy.json` can supply device-local age, minimum size, pages per pass and idle timeout without a bridge API. The writer scans at most two directory entries after five idle seconds; dirty or held pages and pages with unmerged foreign logs are skipped. Trash uses a shallow snapshot at the latest frontier: materialized state remains necessary for restore, sync and index rebuilding, while the operation history is fully archived.

Cutoff selection takes a causal prefix and stops before the first recent commit, including when an offline clock produces nonmonotonic timestamps. A verified critical-root workaround exports the shallow prefix at the requested frontier and appends original updates. The actual shallow root is read back and stored; a conservative retreat retains extra history hot. The alternating-peer regression and both measured vaults preserved every requested root: zero operations and zero seconds of retreat. In the aged fixture, root timestamps landed 245,945–646,475 seconds (2.85–7.48 days) before the 30-day cutoff, reflecting available commit spacing. The six real archives are trashed pages and use their latest frontier regardless of age.

The rebuild guard now materializes the loaded hot document's root handles and ignores only empty top-level Map/List/String roots when comparing state. Nested empty values and nonempty roots remain exact, as do oplog VV, frontiers and the full-history requirement. Loro can omit unused roots from snapshots and recreate deleted empty roots from old updates; these differences do not describe edits. No type-data implementation or bridge API changed. Dedicated regressions cover an accessed empty `type_data` root and a deleted root with a meaningful nested empty value.

Before adding the new import-recovery regression, the focused Linux machine run passed **38 tests, zero failed, one ignored**. It covers original history/version identity, pre-root view/restore, fresh and older-VV exports, offline imports including partial valid-op application, unflushed log recovery, trash/subtree/restore/sync/purge, actual writer-idle execution, the 16 KiB default and local edit/acquisition/purge interleavings. The retained two-writer subset separately passed 12 tests with one ignored; shallow-limit and sync integration subsets passed 11 and nine tests.

Cold publication prepares compression outside page/file locks, then checks snapshot/log identities, sizes and the vault sequence under the same OS advisory `.vault/flush.lock` used by flushes and all log appends. New disk operations cancel prepared publication and are merged for ordinary persistence; foreign pending logs remain available for recovery and indexing. A durable sequence bump precedes mutation, cold is durable before hot replacement, and included logs are truncated in place. A crash after cold replacement but before hot replacement leaves the old full snapshot, or an older shallow snapshot covered by the newer cold prefix. The second-instance flush regression and append-without-flush/crash regression pass. Clean stale caches can subsequently go cold; cached full documents also merge operations archived by another instance and retain that instance's cold boundary when flushing.

History and older/full exports refresh the newest hot/cold/log state under the flush lock. Outdated-dependency imports rebuild from cold, newest disk state and locally applied operations, then retry idempotently and log the result. Recent-based opening, editing and exports do not read cold payloads; a reconstructed full document may remain cached, but its flush still writes a shallow hot snapshot.

Release measurements use an aged fixture rebuilt from `seed::generate(10000, 2000)`: 10,000 ordinary pages and one 2,000-block page. The replay preserves the seed's complete current state and original page/block IDs. Creation and old commits span 180 to 32 days before the injected anchor, with two recent commits at 25 and 20 days. Build batches are eight ordinary blocks or 40 big-page blocks, followed by 24 rewrite/append rounds and head restoration. Two peers switch with 20% probability per commit. Artificial text rewrites use explicit Loro delete/insert operations to avoid expensive text diffing in the generator; normal production editing is unchanged. Eligible history targets 20 KiB, starting with 800 characters per ordinary rewrite and 1,500 for the big page, with adaptive retries available. All pages qualified without retries: ordinary eligible prefixes ranged from 31,115 to 59,549 bytes (p50 45,976); the big prefix was 925,847 bytes.

The default 30-day/16-KiB policy moved all 10,001 pages. The harness verified all heads, frontiers, VVs and version lists, plus pre-root views on 241 candidates, with zero mismatches. The cold pass took 82.0 s; archived original updates totalled 460,807,557 uncompressed bytes. The first 20 archives have no dictionary and 9,981 use the trained 8,192-byte dictionary. Recompressing those same prefixes without a dictionary gives 131,899,498 bytes, versus 126,777,760 with dictionary use: 5,121,738 bytes saved before dictionary overhead. Actual roots matched requested roots throughout.

The real fixture is a copy of the original vault, which was only read. It contains 18 documents; six already-trashed pages moved fully cold, with no artificial trash or aging and no dictionary. Every head and version list, and all six sampled pre-root views, matched. Short archives add header and materialized-state overlap, so this small copy grows by 951 bytes overall.

The before/after file lengths below exclude the index, markdown, block caches and attachment blobs. These historical measurements used the Linux machine under `/home/dev/cold-history-measure-20261013-v3` (fixed fixtures and cold-pass checks) and `...-v4` (paired runs). Baseline is master `4200cd0`; branch binaries contain `7ae562e` and the master merge `3a09ad4`. The same public-API `bench --history` source was compiled on both revisions. Side order alternates per pair. All edit and sync work uses additional disposable copies; original fixtures remain unchanged.

| Storage measurement | Aged 10,001 pages before → after | Real copy, 18 documents before → after |
| --- | --- | --- |
| Hot `.loro` bytes | 309,976,276 → 35,882,765 | 29,396 → 26,826 |
| Cold bytes | 0 → 126,777,760 | 0 → 3,521 |
| Dictionary bytes | 0 → 8,192 | 0 → 0 |
| Combined hot + cold + dictionary bytes | 309,976,276 → 162,668,717 (−47.5%) | 29,396 → 30,347 (+3.2%) |
| Cold files / dictionary files after | 10,001 / 1 | 6 / 0 |

The aged timings sample 60 ordinary pages and the 2,000-block page separately, using five repeat calls for each warmed sample. The selected generated historical version is halfway back through the version list and before the cold root. Every successful run checks its pinned version list and old page value. Edits insert one paragraph, first in a fresh Vault and then in the same Vault. `sync_bench` runs core export/import before HTTP sync, checks every received VV and performs its existing delta assertions. HTTP byte figures are rounded by the existing benchmark; serialized full snapshot byte lengths need not be identical across rebuilds. That historical campaign completed four pairs; its fifth was blocked by the product timeout documented below. The current five-pair self-hosted CI runner campaign is recorded in the 2026-10-14 subsection below.
The table below retains the initial two raw successful pairs. Later complete pairs 3 and 4 were recorded on the Linux machine under `cold-history-measure-20261013-v5`, with alternating side order and explicit `/proc/loadavg` start/end readings. Cold means a fresh Vault cache; OS caches were not dropped.

| Measurement | Master 1 | Branch 1 | Master 2 | Branch 2 |
| --- | --- | --- | --- | --- |
| page open cold (fresh vault) (ms) | 1.091 | 1.832 | 0.737 | 0.805 |
| page open cold (fresh vault), big page (ms) | 23.637 | 18.075 | 8.012 | 13.464 |
| page open warm (repeat load) (ms) | 0.025 | 0.023 | 0.015 | 0.023 |
| page open warm (repeat load), big page (ms) | 7.566 | 8.724 | 4.298 | 6.422 |
| history list first (fresh vault) (ms) | 0.883 | 5.778 | 0.553 | 3.310 |
| history list first (fresh vault), big page (ms) | 23.682 | 249.641 | 13.359 | 141.969 |
| history list repeat (ms) | 0.179 | 0.161 | 0.107 | 0.162 |
| history list repeat, big page (ms) | 4.536 | 5.221 | 2.312 | 3.324 |
| old version first (fresh vault) (ms) | 4.033 | 8.168 | 2.564 | 5.607 |
| old version first (fresh vault), big page (ms) | 1862.297 | 1173.092 | 1074.083 | 932.758 |
| old version repeat (ms) | 3.378 | 4.097 | 2.118 | 2.949 |
| old version repeat, big page (ms) | 1762.062 | 1116.899 | 869.264 | 807.263 |
| edit first (fresh vault, cold page) (ms) | 0.633 | 0.618 | 0.424 | 0.603 |
| edit first (fresh vault, cold page), big page (ms) | 15.282 | 15.776 | 7.684 | 13.055 |
| edit second (same vault) (ms) | 0.104 | 0.101 | 0.064 | 0.090 |
| edit second (same vault), big page (ms) | 0.150 | 0.161 | 0.083 | 0.178 |
| Full export (ms) | 7097.198 | 55206.423 | 5275.428 | 37992.917 |
| Full export (bytes) | 309967530 | 309797149 | 309967530 | 309797149 |
| HTTP full sync (s) | 57.57 | 86.95 | 41.71 | 67.39 |
| HTTP down / up (MB, rounded) | 182.7 / 3.69 | 182.7 / 3.69 | 182.7 / 3.68 | 182.7 / 3.69 |

Load is `uptime` 1/5/15 minutes, before → after each successful run:

| Pair / side | Page/history/edit/export run | Sync run |
| --- | --- | --- |
| 1 / master | 13.41/11.24/7.89 → 10.54/10.86/7.96 | 10.49/10.85/7.97 → 10.07/10.44/8.14 |
| 1 / branch | 10.75/10.57/8.21 → 14.23/12.07/9.10 | 14.23/12.07/9.10 → 8.45/10.77/9.18 |
| 2 / master | 5.77/8.52/8.66 → 5.22/8.07/8.50 | 3.69/5.33/7.21 → 4.39/5.16/6.99 |
| 2 / branch | 8.41/10.72/9.17 → 8.57/10.13/9.09 | 8.57/10.13/9.09 → 6.01/8.61/8.69 |

The successful master sync in pair 2 used a fresh destination after a separate failed attempt. Its failed HTTP attempt started at load 5.22/8.07/8.50; the subsequent failed pair-3 core import ran at 7.76/5.87/7.11 → 14.56/8.18/7.83. Both failure logs and partially imported destinations are retained. No real-vault paired timings or five-pair stability result were completed.

**Historical import-recovery defect, fixed by merged master:** the initial master baseline failed twice with `timed out waiting for the vault flush lock`, once in HTTP fresh-device sync and once in the core new-page batch import. Its import path ran parallel snapshot writers under the two-second lock deadline, before the batch index transaction. After the failed core import, the receiver had 7,950 snapshot files but only 7,500 indexed pages; the failed HTTP client had 6,199 snapshots but 5,973 indexed pages. The source history remained intact; no acknowledged-edit loss is established. Master recovery commit `be26b37` is now integrated by `536b819`. It repairs unindexed durable snapshots on open and identical retry, while retaining cold reconstruction and the shared flush protocol. Recovery does not change the lock deadline or eliminate the separate timeout below.

Before the recovery fix, identical retries detected no new operations and skipped index repair. Master's `crates/core/tests/import_recovery.rs` now covers the interrupted batch, identical retry and reopen through the public API, plus missing snapshots with pending logs and purge handling. Its eight tests pass; the cold-lane duplicate repro was removed. The initial partial pair 3 remains excluded, and the original master pair-2 fresh retry remains disclosed.

The focused blocker run was **zero passed, one failed**. The full acceptance command `cargo test -p vault_core -p vault_sync -p notes_hub --no-fail-fast` ran once on the Linux machine: **482 passed, one failed, four ignored**. Package counts are `vault_core` 436 passed / one failed / four ignored, `vault_sync` 30 passed and `notes_hub` 16 passed. The sole failure is the retained interrupted-import regression; all 11 shallow-limit tests and all 20 private/integration two-writer checks passed. Totals exclude two nested filtered child-process result lines already represented by their parent targets. Acceptance load was 3.44/4.91/6.47 → 11.87/7.17/7.09. Master `4200cd0` was merged before this run; the later master `0919026` changes only `wiki/architecture/node.md` and was also merged, without changing tested Rust source. Windows write-through code remains unbuilt and runtime-unverified. No local workspace builds, package/target installs or pushes were performed. Unrelated concurrent design/wiki changes are preserved.

### First-sync cost investigation (12e, 2026-10-13)

**Post-rewrite integration and stop condition:** the lane was verified clean at `515892d` equal to `origin/lane/storage`, then merged fetched master `fe9579e` (which includes `17b98ca` and import recovery). The requested Linux machine command `cargo test -p vault_core -p vault_sync -p notes_hub` passes: **490 passed, zero failed, four ignored** (`vault_core` 444 passed / four ignored, `vault_sync` 30 passed, `notes_hub` 16 passed). All eight recovery tests and 11 shallow-limit tests pass. The compiled test inventory contains 494 tests; nested child-process output is not counted twice. The earlier acceptance failure described above is historical.

The historical pair 5 remained incomplete. This was a **product import timeout on the pinned measured binaries**, with no supported harness-overlap or stale-holder explanation. The harness runs one benchmark at a time using `subprocess.run`, fresh owned paths and separate receiver UUIDs; every failed command exited 101 rather than reaching the harness's 600-second timeout. The saved receiver lock files have distinct inodes on device 104 and link count 1. A surviving lock-file pathname or sequence contents cannot retain an OS advisory lock after its owner exits.

| Failed pair-5 attempt | Stage | Distinct receiver / lock inode | Retained log root |
| --- | --- | --- | --- |
| Master attempt 1 | Core new-page import, pinned bench line 87 | `core-01M4VA6EP98G3NAAPK552KS7AB` / 2446676 | `cold-history-measure-20261013-v5/aged-master-sync-5.failed-1.log` |
| Master attempt 2 | Core new-page import, pinned bench line 87 | `core-01M4VA7MKJ4YAK9HFF8HMN4EA9` / 2220204 | `cold-history-measure-20261013-v5/aged-master-sync-5.failed-2.log` |
| Branch attempt 1 | **HTTP first sync**, pinned bench line 111, after successful core export/import | `client-01M4VAZY11C07E3NR8THPGDJW6` / 2462391 | `cold-history-measure-20261013-v6/aged-branch-sync-5.failed-1.log` |
| Branch attempt 2 | Core new-page import, pinned bench line 87 | `core-01M4VB3NK8E3JEDCCB9E0EEG8E` / 2513563 | `cold-history-measure-20261013-v6/aged-branch-sync-5.failed-2.log` |

Paths above were under `/home/dev/` on the Linux machine. As checked on 2026-10-14, disk cleanup removed the v3, v4 and v5 directories; v6 failure evidence and the original profile directory remain. Copies of the v3 fixtures were retained on the self-hosted CI runner before cleanup and are used by the current campaign below. In the HTTP failure the core phase had already completed: export 46.85 s, import 56.55 s, flush 0.00 s. Thus the failure is not confined to the benchmark's preliminary core phase.

Both revisions pinned for that historical campaign used the same mechanism: [parallel new-page workers](../../crates/core/src/sync.rs#L576) call [read_cold](../../crates/core/src/sync.rs#L383), which calls [write_page_doc](../../crates/core/src/vault.rs#L765). Each durable page install competes for one vault mutex and OS advisory lock, under a [two-second deadline with 10 ms polling](../../crates/core/src/vault.rs#L72). The [held interval](../../crates/core/src/vault.rs#L765) includes sequence fsync, durable snapshot replacement and metadata checks. The exact failed acquisition stage and holder were not recorded: polling starvation, slow durable I/O or another internal holder remain hypotheses, not a proven deadlock or a cold-specific regression. Successful profiles' 56.042/36.571 s aggregate lock waits support contention but do not identify the longest wait or lock owner.

At that stage, the user's stop instruction prevented another fifth-pair retry or lock-policy fix. The five-pair medians/ratios were unfinished; the minimum was not lowered. Existing complete HTTP totals are master/branch 57.57/86.95, 41.71/67.39, 86.32/105.22 and 67.40/74.39 seconds for pairs 1–4. These are noisy relative observations, not final wall-time claims. A later authorized diagnostic should record mutex-versus-OS wait stage, PID/TID, root/device/inode and acquisition/release/hold durations, capturing the holder at timeout. No such diagnostic or design change is implemented here.

The 2026-10-13 investigation itself made no storage or sync design changes. Optional `storage-metrics` features in `vault_core` and `vault_sync` compile all production instrumentation out by default. Counters report calls, bytes, inclusive time and time excluding nested spans on the same thread. These are elapsed times under load, not CPU samples; cross-thread sums and HTTP wait spans overlap and must not be added to wall time. Read spans include file open/read/close, allocation, cached reads and scheduling; OS caches were not dropped, and physical disk service time is not isolated. `perf` is absent on the Linux machine and no tools were installed.

`sync_bench --only core-export` isolates source export; `--only http-first-sync` starts HTTP with a fresh source cache, without the default benchmark's preceding core export/import. Both modes require the feature. The HTTP mode still checks that all 10,001 pages arrive and every receiver VV includes the source VV. The measured master archive was labeled `4200cd0`, the paired candidate binary `7ae562e`, and its telemetry `9f2c861`: these are **historical pre-rewrite artifact labels**, not bases for current work or pushes. Telemetry is now `86a67cb` in the rewritten history. The checked overlay does not copy candidate core behavior into master. Artifacts, feature binaries, commands, hashes, counters and loads are retained under `/home/dev/cold-history-profile-20261013-v1`. All runs use disposable copies of the same v3 fixtures.

The export-only run isolates the source's Loro and cold work:

| Cost / counter | Master | Branch | Scope |
| --- | ---: | ---: | --- |
| Export all pages | 5.235 s | 43.403 s | Parent elapsed span; diagnostic, not a qualified A/B wall ratio |
| Initial hot snapshot reads | 1.796 s / 309,976,276 B | 6.181 s / 35,882,765 B | 10,001 reads each |
| Cold payload reads | 0 | 3.249 s / 126,777,760 B | 10,001 reads |
| Dictionary reads | 0 | 0.195 s / 81,764,352 B | 9,981 reads of the same immutable 8,192 B dictionary |
| zstd dictionary decompression | 0 | 1.492 s / 459,879,741 B output | 9,981 calls |
| zstd plain decompression | 0 | 0.00236 s / 927,816 B output | 20 calls; different pages, not a controlled dictionary speed comparison |
| Cold FNV checksums | 0 | 0.800 s / 669,269,661 B hashed | Header, payload, raw updates and dictionary; 39,984 calls |
| Loro initial blob import | 0.850 s | 6.890 s | 10,001 hot imports vs 10,001 hot plus 10,001 cold-prefix imports |
| Loro hot suffix batch import | 0 | 1.861 s / 41,676,582 B | 10,001 batches; disk hot plus cached hot updates |
| Rebuild from cold + hot | 0 | 7.760 s | Subset parent containing prefix/suffix import and PageDoc setup |
| Verification fork | 0 | 10.072 s | Subset parent; raw Loro fork 9.620 s |
| Verification compare | 0 | 1.184 s | Subset parent; deep values 1.051 s, VV/frontiers and normalization remainder |
| Export final full snapshots | 1.781 s | 6.735 s | 10,001 calls; 309,967,530 vs 309,797,149 B |
| Export cached hot / reread disk hot / probe logs | 0 | 0.579 / 0.231 / 0.059 s | Rebuild subsets, not additional to its parent |
| Stat page files | 0.200 s / 30,003 calls | 0.790 s / 60,006 calls | File identity/size checks |
| Acquire flush lock | 0.011 s / 10,001 calls | 0.055 s / 20,002 calls | Source only; lock acquisition, not the held interval |
| Complete reconstruction | 0 | 26.323 s | `full_copy.total`; includes cold I/O, imports, verification and cleanup |

Loro 1.16.2's `fork_at` internally exports `SnapshotAt` and imports that snapshot into another `LoroDoc` (`loro-internal/src/fork.rs:38–53`). The verification-fork timer therefore contains another snapshot build and import; those internal halves are not separately measured by public-API spans. Final snapshot construction/export is timed separately. The table's import, fork and parent rows overlap deliberately; it is not an additive waterfall.

Before the accepted change below, the guard forked at the cached document's frontiers even when reconstruction had reached exactly the same head. The fixture has no foreign writer advancing those heads, yet every one of the 10,001 reconstructions performs this additional snapshot/import. The advanced-disk case still needs validation at the cached version; this investigation does not select or implement a replacement guard.

The HTTP run confirms that reconstruction is the added server cost:

| HTTP phase / counter | Master | Branch | Interpretation |
| --- | ---: | ---: | --- |
| Hub export of 10,001 pages | 7.203 s | 23.825 s | 16.622 s extra at the source |
| Cold reconstruction (`full_copy.total`) | 0 | 16.024 s | 96.4% of that observed export increase; 67.3% of branch export time |
| Rebuild import / verification fork / compare | 0 | 4.891 / 5.966 / 0.696 s | Reconstruction subsets |
| Cold I/O / dictionary I/O | 0 | 1.769 / 0.131 s | Thread sums; includes 19 receiver archival validation reads |
| Dict / plain zstd decompression | 0 | 0.982 / 0.00511 s | 9,981 dictionary calls; 20 source plain plus 19 receiver validation calls |
| Cold checksums | 0 | 0.731 s | Includes receiver archival checks |
| Loro snapshot export | 2.328 s | 3.903 s | Branch also includes 9 tiny receiver archival composition exports |
| Hub inventory collection / comparison and sorting | 0.237 / 0.403 s | 0.112 / 0.180 s | 37 budget rounds; no increase in rounds |
| Binary request/response encode and decode, both endpoints | 0.125 s | 0.088 s | Sum of four codec spans |
| Wire zstd compression / decompression, both endpoints | 3.856 / 0.691 s | 2.021 / 0.434 s | Separate from cold-file zstd |
| Authentication/SHA-256 | <0.001 s | <0.001 s | 37 request-token hashes, not page-content hashing |
| HTTP send wait / response body read | 11.780 / 0.170 s | 26.199 / 0.238 s | Send wait includes hub work; not pure network transfer time |
| Receiving client batch imports | 39.794 s | 26.179 s | Critical-path parent including SQL, markdown and parallel durable page installation |
| Receiving durable page writes excluding nested lock acquisition | 17.044 s | 11.736 s | Thread sums across 10,001 files |
| All flush-lock acquisition spans | 56.042 s | 36.571 s | Thread sums dominated by competing receiver writers, not wall seconds |
| Receiver idle cold pass | 0 | 0.331 s | 19 newly imported pages archived under the normal policy; 0.013 s archive writes |
| Response bytes / request bytes | 182,670,903 / 3,683,690 | 182,666,248 / 3,687,395 | Wire growth does not explain the regression |
| Instrumented client sync span | 52.841 s | 53.254 s | Diagnostic only: faster branch-run receiver work masks the extra exporter cost |

`sync.read_cold` means *new-page sync installation*, not reading a `.cold` archive; its large parallel totals must not be attributed to old-history decompression. The receiver import path is common to both revisions. File durability, the two-writer lock and normal receiver indexing/rendering remain substantial shared costs. We have elapsed lock-acquisition counters, not a lock-holder trace; no new import-policy diagnosis or fix is claimed.

The Linux machine cannot SSH to the `home-server` alias, so these start/end readings explicitly use the Linux machine's `/proc/loadavg` (1/5/15 minutes), not the self-hosted CI runner's reported load:

| Profile run | Start → end load |
| --- | --- |
| Branch source export | 9.22/12.42/15.31 → 10.49/12.22/15.10 |
| Master source export | 6.55/10.37/14.12 → 6.18/10.23/14.05 |
| Master HTTP only | 6.18/10.23/14.05 → 9.22/10.28/13.84 |
| Branch HTTP only | 9.21/10.26/13.82 → 5.31/9.02/13.19 |

**Which requests need cold?** [Core export](../../crates/core/src/sync.rs#L444) refreshes when `since` is empty or [the peer VV does not include the shallow root](../../crates/core/src/doc.rs#L355). [Refresh](../../crates/core/src/vault.rs#L1065) checks disk sequence/stamps under the flush lock, then [rehydrates](../../crates/core/src/doc.rs#L471) only a shallow page with an attached cold source. [The rebuild](../../crates/core/src/doc.rs#L401) reads cold, imports cold plus hot/logs, verifies the cached version and returns a full document; export then builds a full snapshot for empty `since`.

Yes: every split page sent to the fresh peer reads and decompresses history once. Source-only counters show exactly 10,001 reads, 20 plain and 9,981 dictionary decompressions. HTTP has the same 10,001 reconstructions plus 19 small receiver archival validation reads. First sync's missing page uses an empty VV, so the entire history is needed under the settled contract that every device retains full history. Sending only the shallow state would change that contract; the existing wire protocol has no current-state-only sync request.

All-hot pages already perform zero cold reads. A delta peer with `since >= root VV` already skips cold refresh and exports only its requested updates. A fully covered page is [filtered before export](../../crates/sync/src/hub.rs#L428). `SyncRequest.full` means [complete inventory comparison](../../crates/sync/src/wire.rs#L135), not “force snapshots for every page.” The [budget loop](../../crates/sync/src/hub.rs#L460) charges uncompressed page bytes and the [client repeats inventory rounds](../../crates/sync/src/client.rs#L386); already received versions are excluded on later rounds. Thus 37 rounds do not mean 37 decompressions per page. The [32-page cache](../../crates/core/src/vault.rs#L67) limits reuse across another whole-vault sweep; already-full cached pages can reuse their full document while disk observations stay unchanged.

**Historical savings estimates (before the accepted verification change):** legal root-covered deltas already avoid the reconstruction bucket (16.0 s in this diagnostic full-sync export); that gives **zero additional saving for this fresh-peer fixture**, because none of its missing VVs covers a root. The entire archive read/decode bucket, including dictionaries and integrity checks, is roughly 3.7 s (about 7% of the diagnostic sync); avoiding this work cannot eliminate the required history. The largest measured candidate is duplicate validation: fork plus comparison is 6.66 s, about 12.5% of the diagnostic sync or 28% of hub export. That is a cost ceiling, not permission to remove completeness/state checks. Cached-hot export, duplicate disk-hot read and log probes total 0.476 s (under 1% of sync); any reduction must still observe another writer's newest state. Repeated dictionary reads cost 0.131 s; dictionary hashing is roughly another 0.09 s at the aggregate checksum throughput, so dictionary reuse alone is unlikely to resolve a 50% regression. Inventory work is below 1% here. That investigation implemented no caching, validation, export-mode or durability change; the subsequently accepted verification change is described below.

A fresh peer still receives full history through the existing wire protocol.

### Cheaper verification and five self-hosted CI runner pairs (2026-10-14)

Master `f9fdc81` is integrated by `3462914`. Snapshot decoding takes owned `PageUpdate` values, has no filesystem effects and runs outside the flush lock. One batch acquisition installs its durable files and commits the index; cold-file publication retains the flush protocol. In-process writers queue, while waits for another OS lock holder have a 10-second deadline. Both `tracing` and `zstd`, and optional `storage-metrics`, remain enabled as appropriate.

Verification change `036f4e9` reuses the rebuilt document when it is non-shallow, its operation VV/frontiers equal the cached hot head, and its state frontiers are at that head. It then performs the same exact canonical state comparison used before, ignoring only empty top-level roots. Different versions or state frontiers keep the original fork at the cached frontiers and its state/version check. A mismatch is still rejected with the same error and handling. A regression supplies a valid sealed prefix with matching versions/frontiers but wrong state; both the original and cheaper guards reject it. Missing-version and corrupt-prefix regressions also pass. Full history, cold integrity checks and durability behavior are retained.

The [committed evidence](../../crates/core/benches/evidence/cold-storage-20261014/README.md) and [full A/B report](../../crates/core/benches/evidence/cold-storage-20261014/cold-ab-5pairs.md) pin master `f9fdc81` against candidate `036f4e9`; timing commit is `8e72d14`. All five interleaved pairs ran on the self-hosted CI runner, alternating side order, one benchmark at a time, without an idle wait. Both revisions use byte-identical public-API benchmark sources and fixed default-feature release binaries. Every run uses a disposable copy of the same retained v3 fixtures; fingerprints and binaries remain unchanged. All 20 runs pass on their first attempt, with zero failures, retries or excluded pairs.

Each history run samples 60 ordinary pages and one 2,000-block page, with five repeat calls for warm samples. All version-list and selected pre-root old-view digests match. The existing sync benchmark runs core export/import before HTTP first sync into a separate empty receiver, checks all 10,001 received VVs and retains its delta assertions. Cold means a fresh Vault cache; OS caches were not dropped. The table is the median of five run p50s for page operations, and the median of five HTTP totals. Ratios divide the two side medians. These are noisy relative observations, without a stability or significance verdict.

| Fresh operation | Master median | Branch median | Branch/master |
| --- | ---: | ---: | ---: |
| Ordinary page open | 0.768 ms | 0.714 ms | 0.930 |
| Ordinary edit | 0.562 ms | 0.581 ms | 1.034 |
| Ordinary history list | 0.755 ms | 2.370 ms | 3.139 |
| Ordinary old version | 2.950 ms | 4.702 ms | 1.594 |
| First HTTP sync, 10,001 pages | 45.56 s | 58.13 s | 1.276 |
| Full source export | 7,121.401 ms | 28,704.095 ms | 4.031 |

| Fresh operation, 2,000-block page | Master median (ms) | Branch median (ms) | Branch/master |
| --- | ---: | ---: | ---: |
| Page open | 12.880 | 17.903 | 1.390 |
| Edit | 11.392 | 9.301 | 0.816 |
| History list | 16.297 | 108.122 | 6.634 |
| Old version | 1,449.461 | 634.823 | 0.438 |

Warm-call medians, individual pair values and paired ratios are in the full report. HTTP download medians are 182.7 MB on both sides; uploads are 3.68/3.69 MB (benchmark rounding). Full exports are 309,967,530/309,797,149 bytes; serialized snapshot lengths may differ while version/history assertions still match.

Each boundary reading below is an actual `ssh home-server uptime` call from the PC. Per-phase runner readings are retained separately and explicitly name `/proc/loadavg` as their source.

| Pair / order | Home-server load 1/5/15 minutes, start → end |
| --- | --- |
| 1 / master, branch | 51.09/43.04/30.68 → 13.09/32.57/30.97 |
| 2 / branch, master | 12.11/31.73/30.71 → 9.65/19.70/25.63 |
| 3 / master, branch | 9.12/19.43/25.51 → 19.86/17.77/22.38 |
| 4 / branch, master | 19.86/17.77/22.38 → 25.79/23.73/23.54 |
| 5 / master, branch | 22.54/23.14/23.35 → 18.33/21.33/22.62 |

Separate feature-enabled profiles compare the merged production tree at `3462914` before the change with `036f4e9` after it, using identical fixture fingerprints and fresh source caches. Source-export verification falls from 10.978518 s (fork + compare) to 0.538398 s (head check + compare). HTTP verification falls from 8.197015 s to 0.497673 s. Both after runs perform 10,001 exact state comparisons and zero verification forks. The fork fallback remains available when another writer advances disk history. [Raw counters and build provenance](../../crates/core/benches/evidence/cold-storage-20261014/profile-summary.json) are retained alongside the individual profile records.

| HTTP diagnostic bucket | Before (s) | After (s) |
| --- | ---: | ---: |
| Hub export, 10,001 pages | 30.096212 | 22.405128 |
| Complete cold reconstruction | 19.840824 | 11.906929 |
| Rebuild imports | 6.168184 | 6.385064 |
| Verification head check | 0.000000 | 0.006041 |
| Verification fork | 7.333698 | 0.000000 |
| Exact state comparison | 0.863317 | 0.491632 |
| Cold payload I/O | 2.235747 | 1.880815 |
| Dictionary I/O | 0.172081 | 0.189097 |
| Dictionary zstd decompression | 1.238916 | 1.245363 |
| Cold checksums | 0.768298 | 0.782237 |
| Final snapshot exports | 4.741246 | 5.917500 |
| Receiver batch imports | 28.587906 | 27.164206 |
| Durable batch installation, 37 acquisitions | 19.041992 | 17.275856 |
| Client sync span | 63.204650 | 54.314722 |

These are single diagnostic runs under load, not the five-pair medians. Inclusive spans overlap and must not be added into a waterfall. Some I/O, checksum and snapshot counters include small receiver archival work. HTTP process walls are 64.09/55.40 s; source-export process walls are 38.29/23.41 s. Separate home-server readings surround both phases of each profile; individual phase readings explicitly use the runner. All raw load files are retained in the committed evidence.

Validation at the measured candidate on the Linux machine: 600 passing core/sync/hub tests, six ignored, with only the coordinator-approved inherited journal-purge test filtered out; ten feature-enabled prefix tests pass. All eight two-writer integration tests pass. The explicit aged batch stress passes for 500 and 10,001 pages, with lock holds of 235.62 ms and 4,282.68 ms and total times of 534.28 ms and 10,467.47 ms. These holds are observations, not an upper bound under different load. Owned Linux machine stress directories and the uploaded archive were removed. Windows runtime behavior remains unverified. Current fixtures, binaries and raw phase logs are retained under `/home/builder/work/cold-storage-20261014` on the self-hosted CI runner.

Shallow snapshot limits (Loro 1.16.2, rerun 2026-10-13 in `crates/core/tests/shallow_snapshot_limits.rs`). These describe raw shallow documents; a device with its cold prefix reconstructs full history automatically on the affected paths:

- A shallow doc rejects any update whose causal deps lie before its shallow root (`ImportUpdatesThatDependsOnOutdatedVersion`), including an edit concurrent with the root itself. Retrying fails the same way; nothing is queued. So a phone holding a shallow page cannot take in edits from a device that went offline before the root, whether they come via the Hub or directly.
- When a batch mixes valid and rejected ops, Loro applies the valid ones in memory and then errors. Vault now persists operations applied before an error. With a cold prefix, an outdated-dependency rejection additionally triggers full reconstruction and an idempotent retry, so no error reaches the caller and both sets of operations survive reopening.
- A shallow doc cannot check out or list versions before its root, and exporting for a peer older than the root silently omits the missing history (the peer keeps it pending).
- Edits made after the root converge normally in both directions. A shallow doc can't be upgraded in place: recovery is a fresh full-history doc plus the phone's unsent changes, which the test shows is lossless.

Search speed does not benefit from compressing pages: search reads the index, not page files.

## Decisions

- 2026-10-13: phones keep full history, compressed on the wire and cold-stored locally like the desktop; no device holds a shallow snapshot without its cold part.
- 2026-10-13: an import that depends on history older than the root loads the cold part and retries automatically; no error reaches the user.
- 2026-10-13: cold history starts after 30 days and 16 KiB (`COLD_AFTER`, `COLD_MIN_BYTES`); page history looks the same to the user.
- 2026-10-13: index-size work is not pursued under the current speed and snippet constraints.
- 2026-10-13: sending the stored cold prefix to a fresh peer is deferred; the existing wire protocol keeps sending full history.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Architecture > Sync and storage).
