---
type: topic
layer: state
summary: "Measured speed of the built app: search and UI timings, and the profile checks behind them."
parent: "[[product/index]]"
status: current
---

# Performance

## Speed targets
Targets are in the internal wiki; the search gate is p95 under 20 ms.

The 2026-10-10 typo implementation after profiling and corrected-query narrowing is **not accepted**. See [the current verification and profile](../architecture/core-state.md#adaptive-narrowing) for the measured breakdown, every query percentile, check results, memory accounting and separate limits.

Search is enforced by a warmed, release-only 10,000-page Rust test and a `perf`-tagged Flutter test for Ctrl+K and the agent composer's `@` menu. The earlier 2026-10-09 core run measured p50 16.327 ms / p95 28.226 ms and failed the 20 ms gate. See [core measurements](../architecture/core-state.md#search-performance-gate-2026-10-09) for the corpus, command and per-query results.

Typing search adapts to this vault's data: when estimated document matches exceed the larger of 200 pages or 5% of live pages, it prioritizes title matches and fills the remaining result slots with page-text matches without bm25. Narrowed body fill, including broad corrections with a constant edit cost, has no snippets. Titles rank by exact title, title prefix, word-start prefixes, contiguous substring, then fuzzy score; ties prefer shorter titles and newer pages. Every uncorrected result stays ahead of corrected results. Page-text fill orders by update time descending and ID ascending before the limit, so results remain stable across reindexing. As the query becomes selective, full ranked page-text search resumes; vaults with at most 200 live pages search fully from the first letter. Ctrl+K explains broad results with “Showing title matches — keep typing to search page text”, and shows one muted correction line only when all results are fuzzy. The agent/MCP `search_notes` tool uses full body/title search and the same typo fallback for recall.

The first title-only implementation failed warm-up because `coffee mor` is broad in page text (2,595 matches) and has zero title matches. The agreed unranked body fill resolves that mismatch without changing the gate's non-empty assertion or 20 ms thresholds. The first body-fill gate measured overall p50/p95 1.752/9.556 ms, but failed the per-query requirement for `t` (5.107/23.028 ms). An independent repeat on the same code and a fresh corpus passed overall and every query: **1.172/3.926 ms** overall, with highest query p95 **4.929 ms**. The earlier failure shows host/run variability and remains documented. The release bridge build, Flutter analysis, Ctrl+K navigation and narrowing hint widget tests passed. The UI performance test passed its unchanged debug ceilings: Ctrl+K real p50/p95 37.37/84.78 ms and agent `@` 28.00/44.39 ms, with immediate controls 41.28/102.50 and 22.05/42.43 ms. These do not verify the 20 ms profile-mode UI target. The threshold at 10,000 pages remains 500. See the core measurements for diagnostics and verification results.

The UI debug calibration measured Ctrl+K p50/p95 30.60/58.21 ms and agent `@` 16.99/27.72 ms. Immediate search measured Ctrl+K p95 53.32 ms; the `@` immediate control varied from p95 16.88 to 28.40 ms in the verification run (real p95 29.284 ms). This debug harness cannot reliably enforce 20 ms. Its regression tests use fixed ceilings of 174.624 ms for Ctrl+K and 87.852 ms for `@` (3 × each calibrated real p95). The **[Linux profile-mode check](#linux-profile-check-2026-10-13) is complete**; these debug ceilings do not replace the 20 ms target.

The earlier verification, before adaptive narrowing, passed both debug gates: Ctrl+K measured p50/p95 **26.44/45.55 ms** and agent `@` **14.44/25.75 ms**, over 100 keystrokes per route. Immediate controls measured **23.35/52.23 ms** and **9.01/17.48 ms**, respectively. Neither real route met the 20 ms target in that debug run.

## Windows profile check (2026-10-13)

The [profile harness and command](../architecture/core-state.md#windows-profile-search-measurement-2026-10-13) are implemented for Windows desktop. They retain the debug fixture's 10,000-page corpus, 20 rounds of `apple` prefixes per route, real release Rust bridge, and synthetic input-to-results-frame endpoint. The driver is set up to print and save p50/p95 and raw samples without a 20 ms assertion. Analysis, the add-on import check and the release bridge build pass.

**This Windows attempt is superseded by the accepted [Linux profile check](#linux-profile-check-2026-10-13).** The Windows profile build failed with LNK1104 for `atls.lib` in `flutter_local_notifications_windows`: Flutter selected VS 2022 BuildTools, whose ordinary x64 ATL library is absent (the Spectre variant exists there; Community has the ordinary library). No native profile app or timing run started. Work stopped under the requested failure rule. No app behavior, timing gate or CI tier changes are included.

| Run | Ctrl+K p50 / p95 (ms) | Agent `@` p50 / p95 (ms) |
| --- | --- | --- |
| Historical debug reference | 37.37 / 84.78 | 28.00 / 44.39 |
| Profile 1 | Not run | Not run |
| Profile 2 | Not run | Not run |
| Profile 3 | Not run | Not run |

Machine: Windows 11 Home 10.0.26200, Ryzen 7 5800X (8 cores / 16 threads), approximately 32 GB RAM, GTX 1660 SUPER plus a Parsec virtual display; Flutter 3.47.5 / Dart 3.13.4. Preparation and post-debug CPU spot samples were 100%, with foreign build/test activity observed. There are no during-profile load measurements. The unchanged debug check ran but failed its first Ctrl+K typo timing gate: `applr` real p50/p95 **38.19/987.33 ms**, immediate control **34.86/1644.97 ms**, against the fixed 174.624 ms ceiling. It stopped before the agent route; the thresholds remain unchanged. These typo/debug figures do not qualify the 20 ms profile target.

## Linux profile check (2026-10-13)

**Complete: same hotspots at the agreed bridge-versus-Flutter split.** Xvfb software rendering on a loaded 4-core host does not measure UI frame time meaningfully: these UI numbers are a relative baseline only. The absolute 20 ms UI target needs real hardware to judge. Five Linux desktop profile runs exited 0 with 100 measured keystrokes per route, following one warm-up round on each fresh 10,000-page vault. The extra runs cover the wide spread. See [every run, stage timings, load and command](../architecture/core-state.md#linux-profile-search-measurement-2026-10-13).

Every result below uses **Flutter 3.47.7 / Dart 3.13.5** (the accepted SDK difference from 3.47.5), on the Linux machine: Debian 13, i5-1135G7, four available CPUs, 8 GiB RAM, Xvfb/Mesa llvmpipe software rendering with two worker threads. Logical fixture 1440×900, native surface 1280×720, DPR 1. These are medians of the five run percentiles, with min–max spread in brackets; timings are milliseconds.

| Route / endpoint | Flutter | Median p50 [min–max] | Median p95 [min–max] |
| --- | --- | --- | --- |
| Ctrl+K / pump | 3.47.7 | 119.77 [74.39–128.32] | 221.00 [184.25–282.60] |
| Ctrl+K / raster finish | 3.47.7 | 316.01 [204.09–330.30] | 389.87 [307.79–553.72] |
| Agent `@` / pump | 3.47.7 | 69.44 [56.41–74.50] | 144.15 [113.43–233.09] |
| Agent `@` / raster finish | 3.47.7 | 159.09 [131.66–192.63] | 236.16 [180.12–420.45] |

In Flutter 3.47.7, main search bridge median p50 is **7.47/6.93 ms** for Ctrl+K/`@`, versus **105.65/58.02 ms** with no captured bridge call pending before the results pump. The largest contributors are software raster (**120.52/64.69 ms** median p50) and waiting for the results build (**95.51/52.69 ms**); build/layout is **4.77/2.00 ms**. The bridge's long tails include FRB/Dart scheduling and are not isolated Rust CPU time. Component percentiles are not additive.

This reproduces the Windows picture's dominant Flutter/frame path, so the roadmap measurement item is closed and a Windows profile run is not required by the agreed rule. Absolute timings are inflated by software rasterisation and shared-host load; they do not establish a GPU result or physical presentation latency. All 1,000 samples matched a real raster frame. The harness corrected profile key inference and discarded-frame sizing; earlier invalid preflights are excluded. No app source, behavior, appearance, timing assertion, CI or fast tier changed. The Windows record is retained above.

## What this implies

Stable update-time and page-ID ordering fixes the earlier reindex-stability failure without changing its assertion. The targeted assertion passes, including an independent lead rerun, all eight focused search tests pass, the full core/tools suite passes, and strict core clippy is clean. The stable-fill release gate also passes overall and all 19 queries: p50/p95 **1.600/7.008 ms** overall, with highest per-query p95 **13.201 ms**. `coffee mor`, which needs body fill, measures **6.101/10.017 ms**. All checks ran serially with `CARGO_INCREMENTAL=0`, preserving the original corpus and 20 ms thresholds. The local release bridge was rebuilt and the unchanged hint widget test passed alone. See the core gate's stable-fill table for all query timings. The earlier tables describe index-order fill. The Linux profile check above completes the measurement; its software-rendered UI numbers are a relative baseline only, and the 20 ms UI target is judged later on real hardware.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Product > Performance).
