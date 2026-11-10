---
type: topic
layer: state
summary: "Recorded search timings for the Rust core and the Flutter UI: the 20 ms gate, the A/B acceptance rule, adaptive narrowing and the Windows and Linux profile runs."
parent: "[[core/index]]"
status: current
covers: ["crates/core/tests/search_perf.rs", "crates/core/benches/**"]
---

# Search performance measurements

The search target is p95 under 20 ms. This page holds the dated measurement records behind it: the gate, the A/B acceptance rule, adaptive narrowing and the UI profile runs. The summary is in [Performance](../product/performance.md); how search works is in [Search queries](search-queries.md). Each section carries its date; the numbers describe the runs, not the current code.

## Search performance gate (2026-10-09)

**Latest relative gate (2026-10-13):** title-loop cost from `8712f52` is repaired in `f3d4b6d`; self-hosted CI runner `4f978b9` versus fixed head `c2da667` (run 38418744539 in the former private repo) passes **47/47**, with no new filtered FAIL, and the Linux machine release core checks pass **378 tests, two ignored**. Final unfiltered bands are 45-67%, so baseline parity is unproven; the isolated repair improves warm p50 13.5% with 5% bands ([cause, bisects and full table](search-queries.md#fixed-cost-investigation-2026-10-13)).

The search target is **p95 < 20 ms**, for warmed core queries and for UI input to visible results. `crates/core/tests/search_perf.rs` reuses `seed::generate` with 9,999 ordinary pages and one 2,000-block page (exactly 10,000 pages). It warms 19 single-word, prefix and multi-word queries, then interleaves 50 samples per query at limit 50. The release-only ignored test prints p50/p95 and retains its default absolute assertions (20 ms for exact/filtered cases, 30 ms for fuzzy cases).

**Acceptance rule (2026-10-13):** Run through [`.github/workflows/search-ab.yml`](../../.github/workflows/search-ab.yml), a `workflow_dispatch` job on **the self-hosted CI runner** (`runs-on: [self-hosted, <ci-runner>]`). Each run has at least five alternating AB/BA real pairs and an interleaved baseline/baseline control in the same session. Both control sides use the identical retained baseline executable, verified by SHA-256. For each case, the control spread is `abs(control median-round p50 ratio - 1)` and the noise band is `max(5%, control spread)`. A case fails only when its candidate/baseline median-round p50 ratio exceeds `1 + band`; p95 ratios are informational. The job prints the band and verdict per case and exits nonzero on FAIL. **Control-spread cap (2026-10-13), per case:** a case whose control spread is 10% or more is rerun once; if it is still 10% or more, it is reported **inconclusive** and neither passes nor fails. A run passes only if every conclusive case is within its noise band and at most 2 cases are inconclusive. (A noisy control otherwise widens the band until any candidate passes.) The harness does not enforce the cap yet; that change comes with the next rerun. The self-hosted CI runner is not isolated: it shares the Proxmox host with the Linux machine (both report an i5-1135G7), so other containers' load reaches its runs. Under the cap, the 2026-10-13 final gate does not pass: every unfiltered case had control spread of 45-67%, far more than 2 inconclusive cases; a confirmation rerun waits for GitHub billing to be fixed. **The Linux machine A/B verdicts are superseded for acceptance**; local and Linux machine harness runs remain diagnostics. Absolute hosted-runner and UI latency qualification are separate.

Dispatch after the workflow reaches master (omit `candidate` to use the dispatched ref's HEAD):

```bash
gh workflow run search-ab.yml --ref lane/search-perf \
  -f baseline=4f978b9 -f candidate=f6e411c -f pairs=5
```

### The self-hosted CI runner result: pre-lane baseline versus master (2026-10-13)

**`verdict: FAIL (21 cases beyond band)`**. Baseline `4f978b9` versus candidate `f6e411c`; five real pairs and five same-binary control pairs, 20 interleaved rounds total. All 20 rounds exited zero with all 47 manifest cases and unchanged sample counts/correctness assertions. The gate exited 1 because 21 p50 ratios exceed their case's band. Work stopped at this initial result; the later title-loop repair and final gate are recorded above.

Machine: **The self-hosted CI runner**, Intel Core i5-1135G7 @ 2.40 GHz, **four physical cores / four available CPUs** (`nproc` and Python report four; `lscpu` lists eight logical IDs with four online), Linux 6.17.13-21-pve, Python 3.13.5, Cargo 1.99.0. Both sequential builds used `--release --locked -j 2` and the persistent target cache. Only the baseline's temporary test file needed the report-only patch. The same baseline executable served both control sides; SHA-256 `6e7dcd6a7dd4d7d787cd7ecd447238bf01fe70233841d685519b572c32296f5e` was checked before every round and after the session. No artificial load was added.

Completed run and job summary (run 38399960905 in the former private repo); raw evidence artifact (run 38399960905 in the former private repo) (`search-ab-38399960905-1`, 30-day retention). Dispatch was refused with HTTP 404 because master lacked the workflow. The run used throwaway branch `bench/search-ab-run`, with one extra commit `904e352` adding the temporary push trigger and pinning candidate `f6e411c`. **Do not merge that commit.** The lane's workflow remains dispatch-only; the central chat merges it to master later. A preliminary workflow-context validation failure happened before any runner job; commit `7a51387` corrected it before this measurement session.

Harness selftests passed through `<linux-build-helper>` with `--repo /home/dev/work/search-ab` (the helper ships no git history), then passed on the self-hosted CI runner. Both builds succeeded, both detached revision worktrees were removed, and the result records no cleanup errors. Independently parsing the uploaded raw outputs reproduces all 940 case records and all medians, ratios, bands and verdicts. This run does not establish absolute hosted-runner latency, Flutter/UI latency or physical-device qualification.

Times below are median-of-round milliseconds; ratios are candidate/baseline. Spread and band are percentages. Displayed values are rounded; verdicts use full precision. Coffee is just beyond its band (p50 ratio 1.119804, band 11.964239%).

| Case | Baseline p50 / p95 ms | Candidate p50 / p95 ms | p50 ratio | p95 ratio (info) | Control spread | Band | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| `warm search, 10,000 pages, limit 50` | 0.781 / 3.401 | 0.962 / 3.719 | 1.232 | 1.094 | 16.92% | 16.92% | FAIL |
| `query "p"` | 1.079 / 1.526 | 1.328 / 1.553 | 1.231 | 1.018 | 13.80% | 13.80% | FAIL |
| `query "pr"` | 0.694 / 0.999 | 0.878 / 1.099 | 1.265 | 1.100 | 14.75% | 14.75% | FAIL |
| `query "pro"` | 0.609 / 0.861 | 0.748 / 0.915 | 1.228 | 1.063 | 16.19% | 16.19% | FAIL |
| `query "proj"` | 0.672 / 0.916 | 0.792 / 1.117 | 1.179 | 1.219 | 15.13% | 15.13% | FAIL |
| `query "project"` | 0.641 / 0.913 | 0.759 / 0.958 | 1.184 | 1.049 | 15.79% | 15.79% | FAIL |
| `query "g"` | 1.111 / 1.352 | 1.366 / 1.629 | 1.230 | 1.205 | 15.10% | 15.10% | FAIL |
| `query "ga"` | 0.723 / 1.037 | 0.899 / 1.016 | 1.243 | 0.980 | 17.01% | 17.01% | FAIL |
| `query "gar"` | 0.648 / 0.735 | 0.806 / 0.909 | 1.244 | 1.237 | 14.94% | 14.94% | FAIL |
| `query "gard"` | 0.751 / 1.045 | 0.903 / 1.129 | 1.202 | 1.080 | 15.28% | 15.28% | FAIL |
| `query "garden"` | 0.702 / 0.853 | 0.855 / 0.999 | 1.218 | 1.171 | 16.09% | 16.09% | FAIL |
| `query "a"` | 1.948 / 2.472 | 2.400 / 2.858 | 1.232 | 1.156 | 12.53% | 12.53% | FAIL |
| `query "t"` | 1.862 / 2.502 | 2.305 / 2.591 | 1.238 | 1.036 | 14.58% | 14.58% | FAIL |
| `query "tomato"` | 0.791 / 1.137 | 0.995 / 1.331 | 1.258 | 1.171 | 17.07% | 17.07% | FAIL |
| `query "weekly review"` | 1.096 / 1.398 | 1.061 / 1.324 | 0.968 | 0.947 | 13.22% | 13.22% | PASS |
| `query "coffee mor"` | 3.464 / 4.576 | 3.879 / 4.468 | 1.120 | 0.976 | 11.96% | 11.96% | FAIL |
| `query "zür"` | 0.552 / 0.845 | 0.691 / 0.913 | 1.252 | 1.080 | 14.77% | 14.77% | FAIL |
| `query "café"` | 0.804 / 1.205 | 1.007 / 1.234 | 1.252 | 1.024 | 14.11% | 14.11% | FAIL |
| `query "big page"` | 1.994 / 2.553 | 2.432 / 2.798 | 1.220 | 1.096 | 10.82% | 10.82% | FAIL |
| `query "xylophone"` | 0.701 / 0.873 | 0.685 / 0.993 | 0.977 | 1.137 | 15.36% | 15.36% | PASS |
| `fuzzy search, 10,000 pages, limit 50` | 3.079 / 7.051 | 2.605 / 7.882 | 0.846 | 1.118 | 5.32% | 5.32% | PASS |
| `typo query "tomatoe"` | 3.444 / 3.689 | 2.894 / 3.171 | 0.840 | 0.860 | 27.79% | 27.79% | PASS |
| `typo query "gardne"` | 3.480 / 3.712 | 2.697 / 3.046 | 0.775 | 0.821 | 26.81% | 26.81% | PASS |
| `typo query "weekyl review"` | 3.019 / 3.316 | 2.707 / 3.155 | 0.897 | 0.951 | 31.90% | 31.90% | PASS |
| `typo query "cofee"` | 3.302 / 3.562 | 2.612 / 2.965 | 0.791 | 0.832 | 28.06% | 28.06% | PASS |
| `typo query "cofee mor"` | 6.758 / 7.188 | 6.676 / 8.123 | 0.988 | 1.130 | 29.14% | 29.14% | PASS |
| `typo query "projcet"` | 1.819 / 2.038 | 1.819 / 2.342 | 1.000 | 1.149 | 29.32% | 29.32% | PASS |
| `typo query "recipies"` | 7.064 / 7.716 | 7.922 / 9.379 | 1.121 | 1.216 | 30.72% | 30.72% | PASS |
| `typo query "gadr"` | 4.097 / 4.345 | 3.178 / 3.677 | 0.776 | 0.846 | 27.17% | 27.17% | PASS |
| `typo query "meeitng"` | 2.165 / 2.333 | 2.037 / 2.741 | 0.941 | 1.175 | 29.27% | 29.27% | PASS |
| `typo query "buget"` | 1.994 / 2.198 | 1.776 / 2.259 | 0.891 | 1.028 | 27.58% | 27.58% | PASS |
| `typo query "desgin"` | 2.856 / 3.097 | 2.249 / 2.824 | 0.787 | 0.912 | 27.51% | 27.51% | PASS |
| `typo query "journla"` | 1.676 / 1.870 | 1.706 / 2.028 | 1.018 | 1.084 | 30.48% | 30.48% | PASS |
| `typo query "databsae"` | 2.230 / 2.440 | 2.090 / 2.408 | 0.937 | 0.987 | 28.59% | 28.59% | PASS |
| `uncached tag candidate construction, 100 distinct filters, 10,000 pages` | 5.569 / 6.773 | 2.167 / 2.998 | 0.389 | 0.443 | 14.55% | 14.55% | PASS |
| `filtered filter only, "tag:perf/selective", 10,000 pages` | 3.294 / 3.841 | 1.379 / 1.466 | 0.419 | 0.382 | 15.01% | 15.01% | PASS |
| `filtered selective filter plus text, "project tag:perf/selective", 10,000 pages` | 19.945 / 21.765 | 13.953 / 14.875 | 0.700 | 0.683 | 6.16% | 6.16% | PASS |
| `filtered broad filter plus broad text, "p tag:perf/broad", 10,000 pages` | 5.603 / 6.536 | 4.345 / 4.703 | 0.775 | 0.720 | 9.97% | 9.97% | PASS |
| `filtered negatives only, "-tag:perf/excluded", 10,000 pages` | 2.905 / 3.341 | 1.465 / 1.537 | 0.504 | 0.460 | 8.65% | 8.65% | PASS |
| `filtered deep in, "in:[[<ID>]]", 10,000 pages` | 3.977 / 4.693 | 2.929 / 3.079 | 0.736 | 0.656 | 13.08% | 13.08% | PASS |
| `filtered tag on 10k, "tag:perf/all", 10,000 pages` | 2.579 / 3.040 | 1.188 / 1.288 | 0.461 | 0.424 | 18.75% | 18.75% | PASS |
| `typed prefix "p", constant tag:perf/broad, 10,000 pages` | 3.951 / 4.175 | 4.213 / 4.734 | 1.066 | 1.134 | 2.25% | 5.00% | FAIL |
| `typed prefix "pr", constant tag:perf/broad, 10,000 pages` | 3.711 / 3.922 | 3.817 / 4.298 | 1.029 | 1.096 | 3.02% | 5.00% | PASS |
| `typed prefix "pro", constant tag:perf/broad, 10,000 pages` | 3.608 / 3.880 | 3.707 / 4.179 | 1.027 | 1.077 | 0.85% | 5.00% | PASS |
| `typed prefix "proj", constant tag:perf/broad, 10,000 pages` | 4.456 / 4.703 | 4.856 / 5.227 | 1.090 | 1.111 | 3.44% | 5.00% | FAIL |
| `typed prefix "project", constant tag:perf/broad, 10,000 pages` | 4.425 / 4.713 | 4.906 / 5.252 | 1.109 | 1.114 | 1.07% | 5.00% | FAIL |
| `filtered typo "gardne tag:perf/broad", 10,000 pages` | 8.245 / 10.239 | 5.671 / 5.888 | 0.688 | 0.575 | 6.81% | 6.81% | PASS |


`SEARCH_PERF_REPORT_ONLY=1` disables only the six timing assertions so all 47 cases can be compared. Correctness assertions, corpus, queries, limits and samples are unchanged. The harness adds this switch only to a temporary test-file copy when the baseline lacks it. See [search measurements](search-queries.md#interleaved-ab-acceptance-2026-10-13) for revision pairs, per-case ratios and per-round load.

`app/test/search_perf_test.dart` types `apple` one character at a time through the real Ctrl+K shortcut and the agent composer's `@` menu, using an exactly 10,000-page native seed. Each route has 100 measured keystrokes after warmup. Wall time includes text input, the bridge and the frame displaying completed results; it excludes GPU rasterisation. An immediate cached-search control measures the debug harness with the same widgets and results. `app/tool/test.ps1 perf` runs the Rust gate before the tagged Flutter tests, with one Flutter worker.

**Earlier core result, Windows desktop (hostname not recorded), 2026-10-09:** p50 **16.327 ms**, p95 **28.226 ms** over 950 searches. The release gate **failed** the 20 ms target; 16 of 19 queries also failed their individual p95 gate. The table below records that earlier implementation, before adaptive narrowing.

| Query | p50 (ms) | p95 (ms) |
| --- | --- | --- |
| `p` | 26.493 | 33.601 |
| `pr` | 15.003 | 21.536 |
| `pro` | 15.817 | 22.776 |
| `proj` | 15.641 | 21.516 |
| `project` | 15.987 | 21.642 |
| `g` | 23.630 | 36.509 |
| `ga` | 22.315 | 27.605 |
| `gar` | 21.693 | 29.719 |
| `gard` | 15.470 | 21.400 |
| `garden` | 15.644 | 22.374 |
| `a` | 23.672 | 31.659 |
| `t` | 23.960 | 29.833 |
| `tomato` | 16.261 | 21.582 |
| `weekly review` | 14.843 | 20.806 |
| `coffee mor` | 13.605 | 17.326 |
| `zür` | 14.909 | 21.424 |
| `café` | 15.932 | 20.127 |
| `big page` | 3.583 | 6.151 |
| `xylophone` | 0.352 | 1.261 |

### Adaptive narrowing

`Vault::search` and `search_adaptive` use this vault's document breadth, with no minimum query length. A query is broad when the estimate exceeds `max(200, live_page_count / 20)` (200 documents or 5% of pages; the constant pair is `SEARCH_BREADTH = (200, 20)`). Vaults with at most 200 live pages always use full search. A bounded FTS posting count stops at threshold + 1, without ranking or loading body text. The live-page count is cached against SQLite `total_changes` and `PRAGMA data_version`, refreshing after foreground or background writes.

Existing one-to-three-character FTS prefix indexes are reused. For longer prefixes, a connection-local [FTS5 vocabulary](https://www.sqlite.org/fts5.html#the_fts5vocab_virtual_table_module) (`instance` type) resolves at most 16 distinct indexed terms, using inclusive dictionary seeks to skip duplicate postings. An equivalent exact-term OR avoids eagerly rebuilding prefix position unions before counting documents. A temporary `unicode61` token view supplies the same case/diacritic normalization as the index. This counts the conjunction of the existing word-prefix terms, so a selective earlier term or combination enables full search. When the vocabulary cap is reached, that prefix is omitted from the breadth-only conjunction, giving a conservative upper bound; no remaining terms means broad. Trashed/template postings and capped vocabulary can therefore over-narrow. The title/full result queries still use the original prefixes and exclude trash/templates. The persistent index schema and version remain unchanged; these virtual tables are temporary and require no migration.

Broad input prioritizes titles from the in-memory title list. When titles do not fill the caller's limit, FTS matches fill the remaining rows, excluding returned title IDs. Fill orders the joined page rows by `updated_ms DESC, id ASC` before LIMIT, without bm25 or snippets. Those keys survive reindexing, unlike FTS rowids. Uncorrected narrowed rows have empty snippets and the result remains `narrowed: true`. Selective input uses full ranked body/title search with `narrowed: false`, merged behind the ordered title matches. Title IDs are excluded before body ranking and snippets, avoiding duplicate work on large pages; literal title scans safely reject titles lacking required ASCII characters. Exact titles come first in both paths. Ranked rows are materialized and limited before snippets are calculated; page metadata uses one join. `search_notes` explicitly calls `search_full`, preserving agent/MCP body recall. Ctrl+K displays “Showing title matches — keep typing to search page text”; `@` menus retain their existing presentation.

**Typo tolerance (2026-10-09).** Titles rank by exact title, title prefix, word-start prefixes, contiguous substring, then fzf-style subsequence score with at most one substitution or adjacent transposition per query word of four or more letters. Ties prefer shorter titles, then newer pages, then stable IDs. The foreground title list is patched on create, rename and trash; a separate `data_version`/`total_changes` guard refreshes it after writer commits and subtree changes. `grdn pln` finds “Garden plan”. Correction eligibility is checked per query word against the live `fts5vocab(row)` vocabulary, independently of result count: a full word needs zero exact vocabulary entries; the last word needs zero entries starting with its prefix. Existing words remain unchanged, including beside a missing word. When every word exists, neither fuzzy title scoring nor body correction runs. Each word costs one vocabulary seek; correctly typed queries do not rebuild the deletion index. Its word index is built at vault open from distinct `fts5vocab(row)` terms, then rebuilt lazily after database changes on the next correction pass. Whole words use hashed one/two-deletion forms with verified edit distances; the last word uses a pruned edit-distance traversal of the sorted vocabulary to correct word starts. Length bands allow zero edits for 1–3 letters, one for 4–7, two for 8+. At most eight candidates per word are kept, most frequent first. One OR-expanded FTS query uses native FTS5 phrase matches for each row's edit count and actual correction; on the ranked path, body rank is `bm25 + 4 × edits` (lower is better). Every uncorrected hit precedes every fuzzy hit. `search_full` and agent `search_notes` use the same fallback. Hits carry `fuzzy` and `corrected`; Ctrl+K adds one muted “Showing results for *word*” line only when all hits are fuzzy.

The existing 19-query release gate retains **p95 < 20 ms**, overall and per query, with the same 10,000-page seed and assertions. A separate 13-query typo workload asserts **p95 < 30 ms**, overall and per query, and verifies that each query returns fuzzy hits. `search_index_stats` reports retained word-index allocation capacities (word strings, character arrays and deletion pairs), excluding titles, SQLite and allocator overhead, plus vocabulary-read/build wall time. The UI perf test adds `applr`, `gadr` and `tomatoe` keystrokes for Ctrl+K and agent `@`, using the unchanged fixed debug ceilings; it does not establish profile-mode frame timings.

**Profile before corrected-query narrowing (2026-10-10).** Release build, the unchanged 10,000-page seed, limit 50, three warm-up rounds and 40 interleaved samples per query. Whole-search measurements run with hooks disabled; a separate phase records paired SQL variants. `cofee mor` has one corrected candidate (`coffee`), keeps the existing prefix `mor`, and matches 2,595 documents. `recipies` has one candidate (`recipe`), matches 4,905 documents, and returns 24 fuzzy titles plus 26 body hits. Reducing the candidate cap cannot help these two cases.

| Stage/cost, p50 / p95 ms | `cofee mor` | `recipies` |
| --- | ---: | ---: |
| Whole search, hooks off | 13.394 / 19.655 | 15.981 / 20.956 |
| Vocabulary eligibility | 0.279 / 0.387 | 0.234 / 0.352 |
| Fuzzy title scorer | 1.702 / 2.305 | 0.702 / 0.916 |
| Candidate generation | 0.021 / 0.032 | 0.054 / 0.063 |
| OR matching, count only | 0.697 / 0.947 | 0.456 / 1.132 |
| OR match + page join/filter, cumulative | 2.864 / 4.406 | 4.608 / 7.898 |
| + bm25/edit ranking and limit, cumulative | 7.833 / 12.549 | 12.746 / 15.887 |
| + final FTS join, no snippets, cumulative | 8.702 / 12.869 | 13.530 / 19.264 |
| Production SQL including snippets/correction | 9.908 / 12.834 | 13.605 / 26.182 |
| Page join/filter, paired increment | 2.186 / 3.656 | 4.134 / 7.031 |
| bm25/edit ranking + limit, paired increment | 4.654 / 9.106 | 7.915 / 10.621 |
| Final FTS join, paired increment | 1.021 / 4.360 | 0.836 / 4.942 |
| Snippets/correction, paired increment | 1.177 / 4.775 | -0.399 / 11.063 |

Both queries already use a constant edit penalty; ranking therefore spends time on bm25/sorting rather than per-hit edit decoding. The negative median snippet increment is noise from separately executed variants. Stage p95s are not additive. The final join costs about 1 ms at the median, while ranking and page filtering dominate; the measurements do not support reducing candidates or treating the final join as the primary bottleneck. This warmed profiling run does not replace the unchanged release gate: the previous gate tails were 31.021/47.066 ms. Both corrected queries exceed the existing 500-document narrowing threshold, supporting adaptive unranked body fill for these broad, uniform-edit-cost queries. Full search retains bm25 and edit ranking. Profiling instrumentation is temporary and removed before verification.

**Typo-tolerance verification: ACCEPTED (2026-10-10).** Checks ran serially with `CARGO_INCREMENTAL=0`; each check has a recorded exit code. The original quality tests and both performance workloads/ceilings are unchanged. release-gate: pass; core-tools: pass; clippy: pass; bridge: pass (`cargo build --release -p notes_bridge` exit 0; `flutter_rust_bridge_codegen generate` exit 0, generated files unchanged); flutter-analyze: pass (`flutter analyze` exit 0, no issues); flutter-typo: pass (`flutter test test/search_typo_test.dart` exit 0); flutter-perf: pass (`flutter test -t perf --concurrency=1 test/search_perf_test.dart` exit 0; highest typo-query p95 Ctrl+K 134.74 ms against the 174.624 ms ceiling, Agent @ 72.72 ms against 87.852 ms). Existing quality/performance test hashes are unchanged. CI/deploy changed concurrently outside this search work and were left untouched: .github/workflows/android-apk.yml, the private APK dispatch script.

The profile above supported narrowing broad corrected queries rather than reducing candidates: each failing query has only one candidate, while ranking processes thousands of matches. Adaptive correction directly counts the expanded MATCH postings up to threshold + 1, using the existing max(200, live pages / 20) threshold. Uniform-edit-cost broad queries use title hits followed by unranked body fill ordered by update time/id, with empty snippets, actual matched corrections and `narrowed: true`. Every uncorrected result stays first. Mixed-edit-cost corrections and `search_full`/agent `search_notes` retain bm25 plus edit penalties. Candidate cap eight, frequency ordering and vocabulary eligibility stay unchanged. Temporary profiling hooks/test are removed before the gate.

Latest 10k word index: **196 words**, **3206 deletion keys**, **66,933 estimated retained bytes** (65.36 KiB), **37.047 ms** vocabulary-read/build time. Accounting excludes titles, SQLite, allocator overhead and peak RSS. Cold/stale fuzzy-query latency remains unverified.

| Release query/class | p50 ms | p95 ms | Samples |
| --- | ---: | ---: | ---: |
| `warm search, 10,000 pages, limit 50` | 0.738 | 4.106 | 950 |
| `query "p"` | 1.585 | 2.958 | 50 |
| `query "pr"` | 0.634 | 1.443 | 50 |
| `query "pro"` | 0.413 | 0.622 | 50 |
| `query "proj"` | 0.348 | 0.908 | 50 |
| `query "project"` | 0.305 | 0.731 | 50 |
| `query "g"` | 1.547 | 3.960 | 50 |
| `query "ga"` | 0.730 | 2.174 | 50 |
| `query "gar"` | 0.476 | 0.964 | 50 |
| `query "gard"` | 0.485 | 1.046 | 50 |
| `query "garden"` | 0.364 | 0.721 | 50 |
| `query "a"` | 3.009 | 6.637 | 50 |
| `query "t"` | 2.686 | 7.282 | 50 |
| `query "tomato"` | 0.688 | 1.735 | 50 |
| `query "weekly review"` | 0.867 | 1.409 | 50 |
| `query "coffee mor"` | 3.980 | 5.958 | 50 |
| `query "zür"` | 0.431 | 0.937 | 50 |
| `query "café"` | 0.660 | 1.362 | 50 |
| `query "big page"` | 0.774 | 1.873 | 50 |
| `query "xylophone"` | 0.931 | 1.699 | 50 |
| `fuzzy search, 10,000 pages, limit 50` | 2.946 | 11.662 | 650 |
| `typo query "tomatoe"` | 2.888 | 5.285 | 50 |
| `typo query "gardne"` | 3.024 | 7.395 | 50 |
| `typo query "weekyl review"` | 3.383 | 7.254 | 50 |
| `typo query "cofee"` | 3.257 | 6.185 | 50 |
| `typo query "cofee mor"` | 8.038 | 17.508 | 50 |
| `typo query "projcet"` | 1.778 | 5.154 | 50 |
| `typo query "recipies"` | 7.873 | 21.805 | 50 |
| `typo query "gadr"` | 4.557 | 10.725 | 50 |
| `typo query "meeitng"` | 2.152 | 6.636 | 50 |
| `typo query "buget"` | 1.928 | 4.559 | 50 |
| `typo query "desgin"` | 2.596 | 7.329 | 50 |
| `typo query "journla"` | 1.422 | 6.162 | 50 |
| `typo query "databsae"` | 1.661 | 6.965 | 50 |

Flutter debug ceilings remain Ctrl+K 174.624 ms and agent @ 87.852 ms. These widget-test timings do not establish the 20 ms profile-mode UI target.

This search pass made no changes to `.github/workflows/android-apk.yml` or the private APK dispatch script; external concurrent work advanced master to `7dd3477`. No commits or goldens were made by this pass. The fixture lock fix from the previous pass remains: seed includes root colors in its initial snapshots/indexing transaction, eliminating competing foreground color-backfill writes and queued background flushes. Per-run fixture directories remain unique; vocabulary rebuild is read-only.

The reproducible stage/recall diagnostic is separate from the unchanged gate. Since title tiers intentionally change top-result order, its current recall check verifies that full search retains the baseline body hits rather than requiring the old order:

```powershell
$env:CARGO_INCREMENTAL = '0'
cargo test --release -p vault_core --test search_diagnostics -- --ignored --nocapture
```

**Original SQL profile, 10,000 pages, 50 samples per stage:** stages are cumulative operations, not independent component timings. FTS count p50/p95 was 0.985/1.345 ms for `g` and 0.808/2.168 ms for `garden`; adding bm25 and LIMIT gave 12.085/15.533 and 6.951/11.246 ms; adding page metadata/filtering/recency gave 18.954/26.032 and 10.787/14.119 ms; adding snippets and DTO decoding gave 22.096/46.942 and 14.311/18.087 ms. JSON serialization of the 50 DTOs was 0.006/0.015 and 0.012/0.033 ms. Ranking dominates, page joins add roughly 4–7 ms at the median, and snippets add roughly 3 ms. The existing search already used one page join, not per-result page lookups.

**Final diagnostic:** all 19 breadth estimates passed p95 < 2 ms; the worst was `weekly review` at **0.772 ms** (including live-count cache validation, token normalization, vocabulary seeks and bounded document counting). `g` measured p50/p95 **0.142/0.284 ms**, and `garden` **0.225/0.653 ms**. Adaptive searches measured **1.937/5.308 ms** and **0.981/2.731 ms**, respectively. The original SQL and optimized full API returned identical IDs/order for `weekly review`, `coffee mor`, `xylophone`, `big page`, `project`, `garden`, and `tomato` (six longest queries, plus both queries tied at six characters).

**Earlier blocker (2026-10-09):** the title-only implementation failed the unchanged release gate during warm-up at `corpus must match "coffee mor"`, before collecting timings. This query matches 2,595 live non-template pages across title/body but zero titles on the retained corpus, above the 500-page threshold. Work stopped under the requested decision-conflict rule. The owner then approved filling narrowed results with unranked FTS matches, keeping the non-empty assertion and 20 ms thresholds unchanged. The immediate pre-change gate measured p50/p95 16.774/31.416 ms (profiling compilation overlapped that baseline).

**Body-fill gate, first verification:** overall p50/p95 **1.752/9.556 ms**, 950 calls. Every warm-up non-empty check passed, including `coffee mor`. The gate still failed because `t` measured p95 **23.028 ms**; the other 18 queries stayed below 20 ms. The corpus, non-empty requirement and timing thresholds were unchanged. `CARGO_INCREMENTAL=0`; builds and checks ran serially.

| Query | p50 (ms) | p95 (ms) |
| --- | --- | --- |
| `p` | 4.387 | 15.251 |
| `pr` | 1.204 | 2.654 |
| `pro` | 1.041 | 3.198 |
| `proj` | 1.117 | 7.114 |
| `project` | 1.085 | 6.134 |
| `g` | 2.432 | 5.922 |
| `ga` | 1.821 | 11.391 |
| `gar` | 1.598 | 3.750 |
| `gard` | 1.144 | 6.553 |
| `garden` | 1.046 | 1.901 |
| `a` | 4.672 | 16.839 |
| `t` | 5.107 | 23.028 |
| `tomato` | 1.284 | 2.446 |
| `weekly review` | 4.049 | 19.888 |
| `coffee mor` | 2.473 | 9.516 |
| `zür` | 0.994 | 3.243 |
| `café` | 1.186 | 6.085 |
| `big page` | 3.733 | 10.435 |
| `xylophone` | 0.532 | 8.199 |

**Earlier body-fill integration checks:** all seven focused core search tests and strict core clippy passed; the lead independently reran those seven tests successfully. The earlier full core/tools suite passed, but the post-fill full suite failed `seed_and_reindex_rebuild_the_same_results` at `tests/api.rs:799`: narrowed fill IDs/order differed after rebuilding SQLite row IDs. The first difference was in empty-snippet fill rows; metadata and preceding title hits matched. The assertion expects stable search results across reindexing, while the first unranked FTS index-order implementation could change after reindexing. This remained unresolved at the 30-minute deadline; the assertion was not weakened. The release bridge build passed; Flutter analysis found no issues. The existing Ctrl+K/Ctrl+P navigation widget test passed alone, and `search_narrowing_test.dart` passed alone against the rebuilt bridge. The latter verifies the muted hint for broad queries, empty narrowed snippets, and clearing the hint after selective or empty input. No goldens were changed. Builds/tests ran one at a time with `CARGO_INCREMENTAL=0`.

**Independent repeat gate before stable fill: PASS**, without code, corpus, assertion or threshold changes. A fresh 10,000-page corpus and 950 interleaved calls measured overall p50/p95 **1.172/3.926 ms**. All 19 queries passed individually; the highest p95 was `a` at **4.929 ms**. `t` measured **3.636/4.912 ms**. The earlier `t` failure remains recorded above rather than discarded: wall-time tails varied between runs, so this pass is measured acceptance, not a guarantee under arbitrary host load. The repeat took 129 seconds to rebuild and 103.39 seconds to run.

| Query | Repeat p50 (ms) | Repeat p95 (ms) |
| --- | --- | --- |
| `p` | 3.224 | 4.901 |
| `pr` | 0.942 | 1.458 |
| `pro` | 0.843 | 1.538 |
| `proj` | 0.952 | 1.222 |
| `project` | 0.885 | 1.682 |
| `g` | 1.939 | 2.835 |
| `ga` | 1.295 | 2.015 |
| `gar` | 1.199 | 2.052 |
| `gard` | 0.850 | 1.464 |
| `garden` | 0.916 | 1.286 |
| `a` | 3.395 | 4.929 |
| `t` | 3.636 | 4.912 |
| `tomato` | 1.074 | 1.805 |
| `weekly review` | 3.006 | 4.074 |
| `coffee mor` | 1.938 | 2.622 |
| `zür` | 0.777 | 1.148 |
| `café` | 0.840 | 1.680 |
| `big page` | 2.813 | 3.891 |
| `xylophone` | 0.374 | 0.691 |

**Body-fill UI performance:** `search_perf_test.dart` passed alone at its unchanged fixed debug ceilings. Across 100 keystrokes per route, Ctrl+K real p50/p95 was **37.37/84.78 ms**, with immediate control **41.28/102.50 ms**; agent `@` real was **28.00/44.39 ms**, with control **22.05/42.43 ms**. Neither real route met 20 ms in this debug harness. The profile-mode target, including rasterisation, remains unverified. The fixture generated 10,000 pages in 192,679 ms; the test completed in approximately 238 seconds.

| Prefix | Ctrl+K real p50/p95 | Ctrl+K control p50/p95 | Agent `@` real p50/p95 | Agent `@` control p50/p95 |
| --- | --- | --- | --- | --- |
| `a` | 69.17/109.60 | 84.03/111.62 | 30.88/39.91 | 32.28/48.51 |
| `ap` | 55.85/69.41 | 54.70/82.94 | 31.36/44.39 | 29.00/42.43 |
| `app` | 30.43/60.23 | 26.89/94.51 | 22.87/40.89 | 16.46/28.36 |
| `appl` | 29.61/53.31 | 31.52/50.03 | 22.00/40.66 | 16.95/37.29 |
| `apple` | 27.38/39.60 | 30.29/53.74 | 25.24/48.70 | 14.65/33.64 |

**Stable-fill follow-up: PASS.** Fill uses `ORDER BY p.updated_ms DESC, p.id` before LIMIT, retaining title priority, deduplication and empty snippets. The unchanged `seed_and_reindex_rebuild_the_same_results` assertion passes, including both explicit reindex and deleted-index rebuilding on open; the lead independently reran it after the final whitespace-only rustfmt pass. All eight focused search tests pass, including a new newest-first/equal-timestamp ID-tie test. The full `cargo test -p vault_core -p vault_tools` suite passes and core all-target clippy with `-D warnings` is clean. The gate and full suite ran before the final formatting-only pass; focused search tests and the lead's reindex check ran after it.

The stable-fill release gate passed overall and every query, using the unchanged corpus, non-empty assertion and strict 20 ms thresholds. Overall p50/p95 was **1.600/7.008 ms** over 950 interleaved calls; the largest query p95 was `p` at **13.201 ms**. Sorting the fill added cost for `coffee mor` but it still measured **6.101/10.017 ms**. These are measured wall times on this host; previous runs demonstrate host/run variability. Checks ran serially with `CARGO_INCREMENTAL=0`. The local release bridge was refreshed successfully (3m 53s), and the unchanged narrowing hint widget test passed alone against the bridge. The rebuilt `target/release/notes_bridge.dll` was written at 22:54:51 local time on 2026-10-09. No schema migration, FRB regeneration, goldens or commits were needed for the stable-order fix. The profile-mode UI target remains unverified.

| Query | Stable-fill p50 (ms) | Stable-fill p95 (ms) |
| --- | --- | --- |
| `p` | 3.908 | 13.201 |
| `pr` | 1.081 | 4.083 |
| `pro` | 0.979 | 1.523 |
| `proj` | 1.064 | 1.870 |
| `project` | 1.037 | 1.806 |
| `g` | 2.318 | 4.498 |
| `ga` | 1.688 | 3.134 |
| `gar` | 1.660 | 3.374 |
| `gard` | 1.163 | 4.141 |
| `garden` | 1.027 | 2.622 |
| `a` | 4.631 | 11.123 |
| `t` | 4.942 | 12.655 |
| `tomato` | 1.332 | 6.554 |
| `weekly review` | 3.795 | 6.784 |
| `coffee mor` | 6.101 | 10.017 |
| `zür` | 1.135 | 3.099 |
| `café` | 1.159 | 1.625 |
| `big page` | 3.372 | 9.771 |
| `xylophone` | 0.504 | 1.775 |

The full API intentionally does not narrow broad queries. In this diagnostic `g` remained above 20 ms: original SQL p50/p95 **22.612/32.051 ms**, optimized full API **24.961/35.522 ms**. For `garden`, original SQL was **15.128/23.542 ms**, and the optimized full API **14.438/22.839 ms**. Broad API search preserves recall rather than taking the typing fast path. DTO copying and JSON serialization were approximately 0.01 ms; this is not a measurement of bridge transport or GPU rendering.

**UI calibration result, desktop debug widget harness, 2026-10-09:** each real/control route measured 100 keystrokes (20 per prefix). Both real routes missed 20 ms. Ctrl+K's immediate-search control also missed it. The agent `@` control initially measured p95 16.88 ms, then 28.40 ms on the verification run (real p95 29.284 ms). That unstable debug baseline prevents this harness from reliably checking 20 ms for either route. Tests use **fixed debug ceilings of 174.624 ms for Ctrl+K** (3 × calibrated real p95 58.208 ms) and **87.852 ms for agent `@`** (3 × calibrated real p95 29.284 ms); neither is recalculated from later runs. These debug timings did not establish 20 ms; the later [Linux profile check](#linux-profile-search-measurement-2026-10-13) measures the frame and raster path.

| Route | Real p50 / p95 (ms) | Immediate control p50 / p95 (ms) | Debug gate |
| --- | --- | --- | --- |
| Ctrl+K | 30.60 / 58.21 | 20.40 / 53.32 | p95 < 174.624 ms |
| Agent `@` (first calibration) | 16.99 / 27.72 | 9.72 / 16.88 | subsequently calibrated to p95 < 87.852 ms |

**Earlier UI verification, before adaptive narrowing:** both fixed debug gates passed. Ctrl+K measured p50/p95 **26.44/45.55 ms**, with immediate control **23.35/52.23 ms**; agent `@` measured **14.44/25.75 ms**, with immediate control **9.01/17.48 ms**. Each route again measured 100 keystrokes, 20 per prefix. Both real p95 values exceeded 20 ms; the profile-mode target remains unverified. The per-keystroke results below are from that earlier run:

| Typed prefix | Ctrl+K real p50 / p95 (ms) | Ctrl+K control p50 / p95 (ms) | Agent `@` real p50 / p95 (ms) | Agent `@` control p50 / p95 (ms) |
| --- | --- | --- | --- | --- |
| `a` | 36.98 / 46.09 | 34.09 / 67.50 | 21.38 / 27.23 | 13.47 / 19.93 |
| `ap` | 34.05 / 47.83 | 35.16 / 53.15 | 15.84 / 24.92 | 12.11 / 19.03 |
| `app` | 23.39 / 28.57 | 15.71 / 25.71 | 11.73 / 19.06 | 7.49 / 9.20 |
| `appl` | 22.19 / 26.56 | 16.95 / 25.77 | 12.60 / 19.06 | 7.34 / 12.87 |
| `apple` | 22.37 / 31.06 | 16.50 / 26.08 | 13.60 / 17.49 | 6.44 / 14.01 |

## Windows profile search measurement (2026-10-13)

**Superseded by the accepted [Linux profile measurement](#linux-profile-search-measurement-2026-10-13).** The historical Windows attempt below collected no native profile timings.

The test-only [profile harness](../../app/integration_test/search_profile_test.dart) and [driver](../../app/test_driver/search_profile_driver.dart) are implemented. **Native execution was blocked by the selected Windows toolchain's ATL libraries.** No profile latency samples were collected, and the roadmap check remained open at that point.

The worktree release bridge build passed (`cargo build --release -p notes_bridge`, 8m 22s, four Cargo jobs). `target/release/notes_bridge.dll` is 31,755,776 bytes, SHA-256 `36f6efc673872d034ea6bbb8c2455c5069e195203f3b411c4f65e3acac8fea0b`. Focused `flutter analyze integration_test test_driver` is clean, and the root `dart scripts/check-addon-imports.dart` check passes. Native profile execution remains unverified.

`flutter build windows --profile --target=integration_test/search_profile_test.dart` exited 1 after 903.8 seconds: `LINK : fatal error LNK1104: ... atls.lib` in `flutter_local_notifications_windows.vcxproj`. The generated CMake cache selects `C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools`, x64, Visual Studio 17 2022. Read-only filesystem checks confirm that its MSVC `14.44.35207/atlmfc/lib/x64/atls.lib` is absent, while `atlmfc/lib/spectre/x64/atls.lib` exists. The ordinary x64 library exists in the separate VS 2022 Community installation. This identifies an installation/library-selection blocker, rather than evidence that Flutter generally lacks Windows profile support. The task stopped under its native-profile failure rule; no toolchain installation or reconfiguration was performed.

The native app never started. The three planned `flutter drive` timing runs were not executed, no driver response data was produced, and there are no during-profile load samples. These rows are missing measurements, not successful runs or estimates:

| Run | Ctrl+K p50 / p95 (ms) | Agent `@` p50 / p95 (ms) | Status |
| --- | --- | --- | --- |
| Historical debug reference | 37.37 / 84.78 | 28.00 / 44.39 | Earlier body-fill measurement |
| Profile 1 | — | — | Not run: native link blocker |
| Profile 2 | — | — | Not run: native link blocker |
| Profile 3 | — | — | Not run: native link blocker |

The unchanged `flutter test --concurrency=1 test/search_perf_test.dart -t perf` check ran against the rebuilt release bridge (expanded reporter added for the log), but exited 1 after approximately 3m 02s at the first Ctrl+K typo gate (`applr`): real p50/p95 **38.19/987.33 ms**, immediate control **34.86/1644.97 ms**, 100 samples each, versus the unchanged p95 ceiling **174.624 ms**. Fixture generation took 51,411 ms. The test reached and measured the ordinary Ctrl+K route but failed before printing its aggregate and before the agent route. Therefore no fresh two-route debug comparison is available. The immediate control also exhibited large stalls; this does not establish an app-search regression or isolate the cause. The source and thresholds were not changed. The local log is `app/build/search-profile-debug-check.log` (ignored build artifact).

The corpus matches `test/search_perf_test.dart`: `NotesVault.seed` with 9,999 ordinary pages and zero big-page blocks, producing exactly 10,000 pages. Each route types `a`, `ap`, `app`, `appl`, `apple`, with one warm-up round followed by 20 measured rounds (100 samples per route). Ctrl+K uses the registered shortcut; agent `@` uses `AgentComposer`. An explicit `NOTES_BRIDGE_LIB` loads the worktree's release DLL, and every run creates and removes its own temporary vault. No user vault is opened.

The stopwatch starts immediately before synthetic `enterText` and stops after the current bridge result and one results frame. The live binding uses `onlyPumps`; bridge completion is awaited on the real event loop before pumping that frame. This preserves the debug test's endpoint without adding a vsync wait on every bridge poll. The frame waits for display vsync; refresh rate is recorded. GPU completion, physical keyboard input and display presentation are not measured. Fixed clocks and immediate debounces match the debug fixture. The 20 ms reference is printed without a timing assertion. Raw samples, per-prefix statistics and route p50/p95 are returned as driver response data and saved to `app/build/search_perf/search_perf.json`.

PowerShell, starting at the worktree root:

```powershell
cargo build --release -p notes_bridge
$env:NOTES_BRIDGE_LIB = (Resolve-Path ./target/release/notes_bridge.dll).Path
Set-Location app
flutter pub get
flutter drive --profile -d windows --driver=test_driver/search_profile_driver.dart --target=integration_test/search_profile_test.dart
```

Before each timing run, inspect foreign Cargo/rustc/Flutter/MSBuild activity and wait for 30 quiet seconds. Reusable MSBuild nodes count as idle only while CPU time remains unchanged and they have no build subprocesses. Preserve the response JSON after each run; the driver otherwise overwrites it. Keep the test window visible, and close it after the driver finishes. This harness is an opt-in measurement, outside CI and the fast test tier.

Machine: Windows 11 Home 10.0.26200, AMD Ryzen 7 5800X (8 cores / 16 logical processors), approximately 32 GB RAM, NVIDIA GeForce GTX 1660 SUPER (also a Parsec virtual display adapter); Flutter 3.47.5, Dart 3.13.4. Windows CPU counters read 100% in the preflight and after the debug check. Foreign Cargo/rustc/Flutter/MSBuild jobs were observed throughout preparation. These are spot samples, not a continuous load record for the debug test. No timing quiet window or native-profile measurement is claimed. No user vault, app source, appearance, existing test thresholds, CI configuration or fast test tier was changed.

## Linux profile search measurement (2026-10-13)

**Verdict: same hotspots at the agreed bridge-versus-Flutter split. The profile-mode check is complete.** Xvfb software rendering on a loaded 4-core host does not measure UI frame time meaningfully, so these UI timings are a relative baseline only; the absolute 20 ms UI target is judged later on real hardware. Five sequential Linux desktop profile runs exited 0. Two extra runs were collected because the spread was wide. Each run used a fresh generated 10,000-page vault, one warm-up and 20 measured rounds of `apple` prefixes, 100 keystrokes per route. No timing assertion, CI or fast-tier change was added.

All new numbers use **Flutter 3.47.7 / Dart 3.13.5**, the accepted difference from the repo/reference SDK 3.47.5. Machine for every row: the Linux machine, Debian 13, Intel Core i5-1135G7 @ 2.40 GHz, four available CPUs, 8 GiB RAM; Xvfb 1600×1000×24 with forced Mesa software rendering and `LP_NUM_THREADS=2`. An EGL probe under the same Xvfb/environment reports **llvmpipe (LLVM 19.1.7, 256 bits), Mesa 25.0.7-2+deb13u1**. The native app surface is 1280×720 at DPR 1; the binding preserves the 1440×900 logical fixture. The display API reports refresh rate 0 (unknown). Software raster timings do not establish GPU or physical display performance.

Milliseconds below are p50/p95 except the explicitly labelled contributor medians. Load is the 1/5/15-minute average immediately before → after each route, not continuous CPU utilisation. Other lanes share the host; no quiet window or artificial load was used.

| Run / Flutter | Route | Input → pump | Search bridge | Outside all bridge calls | Wait / build / raster p50 | Input → raster finish | Load before → after |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 / 3.47.7 | Ctrl+K | 119.77/221.00 | 7.21/33.87 | 110.84/201.54 | 99.87 / 5.13 / 120.52 | 318.38/389.87 | 9.89/12.30/11.33 → 10.69/12.29/11.36 |
| 1 / 3.47.7 | Agent `@` | 69.44/113.43 | 7.79/46.46 | 58.02/105.78 | 52.69 / 1.91 / 64.69 | 159.09/234.32 | 10.39/12.20/11.33 → 10.50/12.11/11.32 |
| 2 / 3.47.7 | Ctrl+K | 74.39/184.25 | 6.64/101.40 | 54.02/169.88 | 50.23 / 2.87 / 87.92 | 204.09/307.79 | 4.04/9.14/10.33 → 4.70/8.89/10.21 |
| 2 / 3.47.7 | Agent `@` | 56.41/123.40 | 7.27/58.68 | 40.15/100.57 | 37.59 / 1.44 / 51.55 | 131.66/180.12 | 4.70/8.89/10.21 → 5.56/8.81/10.16 |
| 3 / 3.47.7 | Ctrl+K | 121.81/246.19 | 7.94/117.47 | 105.65/223.96 | 95.51 / 4.77 / 125.03 | 316.01/408.38 | 7.32/8.90/10.13 → 8.68/9.06/10.14 |
| 3 / 3.47.7 | Agent `@` | 74.50/233.09 | 6.93/54.40 | 62.02/145.22 | 58.10 / 2.39 / 75.88 | 192.63/420.45 | 8.68/9.06/10.14 → 10.22/9.40/10.23 |
| 4 / 3.47.7 | Ctrl+K | 128.32/282.60 | 7.47/112.08 | 108.30/251.88 | 97.70 / 5.29 / 131.86 | 330.30/553.72 | 11.06/10.15/10.39 → 11.15/10.30/10.43 |
| 4 / 3.47.7 | Agent `@` | 74.26/144.15 | 5.96/41.49 | 66.12/131.77 | 59.15 / 2.44 / 70.34 | 179.80/236.16 | 11.15/10.30/10.43 → 10.81/10.28/10.42 |
| 5 / 3.47.7 | Ctrl+K | 114.41/218.85 | 7.51/108.32 | 97.42/201.13 | 88.09 / 4.59 / 113.01 | 287.62/355.37 | 12.02/10.63/10.53 → 11.45/10.62/10.54 |
| 5 / 3.47.7 | Agent `@` | 62.03/185.21 | 5.79/89.41 | 52.28/155.36 | 48.69 / 2.00 / 60.58 | 157.18/304.61 | 11.45/10.62/10.54 → 10.86/10.54/10.51 |

Across-run medians and min–max spread (five run percentiles, not pooled samples):

| Route / endpoint | Flutter | Median p50 [min–max] ms | Median p95 [min–max] ms |
| --- | --- | --- | --- |
| Ctrl+K / pump | 3.47.7 | 119.77 [74.39–128.32] | 221.00 [184.25–282.60] |
| Ctrl+K / raster finish | 3.47.7 | 316.01 [204.09–330.30] | 389.87 [307.79–553.72] |
| Agent `@` / pump | 3.47.7 | 69.44 [56.41–74.50] | 144.15 [113.43–233.09] |
| Agent `@` / raster finish | 3.47.7 | 159.09 [131.66–192.63] | 236.16 [180.12–420.45] |

**Top contributors, medians across runs, Flutter 3.47.7:** Ctrl+K software raster p50/p95 **120.52/152.29 ms**, search-completion → build-start wait **95.51/196.00 ms**, and build/layout **4.77/13.39 ms**; agent `@` respectively **64.69/94.96**, **52.69/120.91**, and **2.00/5.95 ms**. The derived build-finish → raster-start queue gap is **79.59/114.06 ms** for Ctrl+K and **36.02/64.03 ms** for `@`. Frame waiting includes event-loop/vsync scheduling; software rendering/backpressure is the dominant measured path. The main search bridge median p50 is **7.47/6.93 ms** for Ctrl+K/`@`, versus **105.65/58.02 ms** with no captured bridge call pending inside the pump window. This matches the Windows debug/control evidence's broad Flutter/frame-dominant split. Absolute times and the large software raster cost are not a Windows GPU comparison; the prior Windows profile attempt is superseded under the central acceptance decision.

The bridge stage is the awaited FRB round trip, including Rust execution and delivery of its Dart continuation, not isolated Rust CPU time. Its across-run median p95 is **108.32/54.40 ms** (Ctrl+K/`@`, Flutter 3.47.7); the long tails cannot be assigned to Rust alone. `outsideBridge` subtracts the clipped union of search, parser and metadata call intervals, so overlapping calls count once. Calls can overlap Flutter work. Build/raster come from the one matched engine frame, and raster completion is reported separately from the original pump endpoint. Component percentiles are not additive. The queue gap is derived per sample from these matched endpoints/durations.

The release bridge was built through `<linux-build-helper> --with /v/projects/BubblePaw-base-agent /v/worktrees/notes-app/lane-search-perf-profile -- cargo build --release -p notes_bridge -j 2` (4m 25s). In the helper's remote shell, from the worktree, run each measurement with a different `SEARCH_PROFILE_RUN`:

```sh
export PATH=/home/dev/flutter/bin:$PATH
export CARGO_BUILD_JOBS=2 CARGOKIT_CARGO_TARGET_DIR="$CARGO_TARGET_DIR"
export NOTES_BRIDGE_LIB="$CARGO_TARGET_DIR/release/libnotes_bridge.so"
cd app
SEARCH_PROFILE_RUN=1 SEARCH_PROFILE_SDK=3.47.7 \
LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=2 LIBGL_DEBUG=verbose \
xvfb-run -a -s '-screen 0 1600x1000x24' \
  flutter drive --profile -d linux \
  --driver=test_driver/search_profile_driver.dart \
  --target=integration_test/search_profile_test.dart
```

**Checks:** all five commands exited 0; all correctness checks passed; 1,000/1,000 samples matched exactly one frame with nonzero raster duration, and raw stage partitions are consistent. Focused `flutter analyze --no-pub integration_test test_driver` is clean; final `dart format` made no changes. `xvfb-run` cleaned its display after each command, and final checks found no app or Xvfb process. The harmless Linux native integration-test plugin warning did not prevent the driver receiving full passed response data.

Release library: `/home/dev/work/.target/lane-search-perf-profile/release/libnotes_bridge.so`, SHA-256 `f9f978c4be1e61adb82449ea2e1fd5865f2d093bb79a8c5a105222f8cb038319`. Raw samples/logs are retained on the Linux machine at `app/build/search_perf/run-{1..5}.{json,log}`, alongside `summary.json` and `renderer.txt`. Initial missing packages were installed by the central chat. The profile-key preflight failure and a pass with discarded zero-raster frames are preserved separately and excluded from every table. The harness now supplies physical shortcut keys and uses `binding.setSurfaceSize` rather than faking native physical size. No `app/lib`, crates, bindings, application behavior, appearance, dependencies, CI or test thresholds changed; no builds ran on the PC.

