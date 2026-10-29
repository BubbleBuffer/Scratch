---
type: topic
layer: state
summary: "Search operators and type filters over vault pages and external chat summaries, grouped results with anchored navigation, and live query streams."
parent: "[[architecture/index]]"
status: current
covers: ["crates/core/src/index.rs", "crates/core/src/query.rs", "crates/core/src/vault.rs", "crates/core/src/feed.rs"]
---


Current build source (2026-10-29): Scratch resolves BubblePaw through the public
`BubbleBuffer/BubblePaw-Core` Cargo git source at tag `v0.3.1`, pinned in
`Cargo.lock`. Current builds need no `BubblePaw-base-agent` sibling. Commands
below containing `--with ...BubblePaw-base-agent` record historical runs only;
retain the matching dependency checkout when reproducing those old revisions.
# Search queries and live queries

Status: search syntax, filters, execution and API merged. External sources, chat search, grouped results and the `type:` filter built 2026-10-12 ([note types, refactor 6](note-types.md#preparatory-refactors)). Live query registry and bridge streams are built 2026-10-15. The dynamic search card (block and inline chip) is built 2026-10-16.

## Syntax

- Words are ANDed; the last word matches as a prefix (as today). `"exact phrase"` is a phrase. `-word` excludes; `-` also negates operators (`-tag:work`).
- Comma means any of: `tag:work,home`. Repeating a key means all of: `tag:work tag:urgent`.
- `tag:` matches only the tags property and select options. A text property named Tags is searched by its name instead, as `Tags:<text>`.
- No `OR` keyword and no parentheses in the first version.
- Keys are case-insensitive and ignore spaces, `_` and `-` (`duedate:` matches "Due date"). Quoted keys work: `"due date":today`. Values with spaces are quoted.
- Built-in operators win over property keys; a property named like a built-in is reached as `"in":x`.
- `is:`, `done:`, `assignee:` and `calendar:` switch the query to item results (to-dos, task notes, dated rows, date mentions). `date:` then filters the item's date, page operators filter its page, and `undated` keeps items without a date. A property named `is`, `done`, `assignee`, `calendar`, `context`, `full` or `limit` keeps its meaning, so quote its key to make that explicit.
- Output-size operators: `context:N` (blocks around each match, default 2), `full:CHARS` (a page at most this long is shown whole, default 1500), `limit:N` (hits per call, default 20, max 200). `sort:<key>,desc` or `asc` sets the order.

## Operators

The table is generated from the operator table in `crates/core/src/search_ops.rs`, which also feeds the parser, the app's completion list, the `search_notes` description and `notes_help` topic `query`. The `vault_tools` help tests fail when it is stale.

<!-- generated: operators -->
| Operator | Values | Meaning | Example | Kind |
|---|---|---|---|---|
| `created:` | `DATE` | page created in that period | `created:week` | filter |
| `updated:` | `DATE` | page edited in that period | `updated:7d` | filter |
| `date:` | `DATE` | created, updated or any date property; with item operators, the item's date | `date:today..+7d` | filter |
| `tag:` | `name` | tagged (tag:work also matches work/meetings); #name is the same | `tag:work,home` | filter |
| `type:` | `document\|database\|chat` | page type | `type:database` | filter |
| `in:` | `[[Page]]\|"Title"\|memory\|path` | that page and its subtree; a path glob (* one level, ** any depth) matches only the pages it names; memory is the agent memory pages | `in:Work/*` | filter |
| `title:` | `word` | the title contains it | `title:plan` | filter |
| `links:` | `[[Page]]` | links to that page | `links:[[Roadmap]]` | filter |
| `from:` | `[[Page]]` | linked from that page | `from:[[Inbox]]` | filter |
| `has:` | `link\|attachment\|<property>` | has a link, an attachment or a non-empty property | `has:attachment` | filter |
| `sort:` | `updated\|created\|title\|<property>` | result order | `sort:title` | filter |
| `is:` | `todo\|task\|row\|mention` | item results: to-do blocks, task notes, dated database rows, date mentions | `is:todo` | filter |
| `done:` | `yes\|no` | items that are (not) done; implies item results | `done:no` | filter |
| `assignee:` | `me\|agent` | task notes assigned to you or the agent; implies item results | `assignee:agent` | filter |
| `calendar:` | `name` | task notes in that calendar category; implies item results | `calendar:Work` | filter |
| `undated` | (none) | with item operators: only items without a date (otherwise a plain word) | `is:todo undated` | filter |
| `context:` | `N` | blocks shown around each match (default 2, max 20) | `context:0` | output size |
| `full:` | `CHARS` | show a page whole when it is at most this long (default 1500, 0 never) | `full:4000` | output size |
| `limit:` | `N` | hits per call (default 20, max 200) | `limit:50` | output size |
| `<property>` | `value` | text contains; select option; number 5, >5 or 1..5; checkbox yes/no; dates use DATE; none means empty | `status:doing` | filter |
<!-- /generated -->

Trash and templates stay excluded.

## In: path patterns

A path is page titles joined by `/`, read from the top of the tree down. `*` and `**` make it a pattern. The user rules are in [search.md](../../docs/notes/search.md#in-pages-and-paths).

- The first literal segment resolves the way a plain `in:` value does. The canonical query string stores it as `[[id]]`, so renames of that page are survived. A pattern that starts with a glob or `**` stores no id.
- Execution walks the pages table (`id`, `parent`, `title`). It starts at the anchor page when the first segment is a page, and covers all pages when the first segment is a glob or `**`. `crates/core/src/search_path.rs` builds each page's chain of ancestor titles and matches the chain against the segments in Rust.
- A pattern without a glob matches a chain prefix: the page and its subtree. A pattern with a glob must match the whole chain. Matching is case-insensitive.
- An exact title or id wins over the path reading (`in_op` in `crates/core/src/search_query.rs`).
- Results are not cached beyond the per-write candidate cache.
- Live queries with a path pattern re-evaluate on every commit, because a rename above a page can change membership. The check is `path_sensitive` in `crates/core/src/live_query.rs`.
- Plain `in:` keeps the cached subtree path (`subtree` in `crates/core/src/search_exec.rs`).

## Date grammar

- Absolute: `2026`, `2026-10`, `2026-10-10`. A partial date covers the whole period.
- Named: `today`, `yesterday`, `tomorrow`, `week`, `lastweek`, `month`, `lastmonth`, `year`.
- Relative: `7d`, `2w`, `3m`, `1y` mean the last N; `+7d` means from now through the next N.
- Ranges and comparisons: `2026-09..2026-10`, `today..+7d`, `>2026-09` (after the end of September), `<today` (before today), `>=`, `<=`.
- `now` is the current minute for comparisons: `<now` matches timed values before now and all-day values before today (the overdue rule).
- Local time zone; day boundaries are local.

## Bad and ambiguous input

- An unknown `word:` is plain text, not an operator.
- A known key with an unreadable value (`created:banana`) is an error token: the UI marks it, search ignores it, and the agent gets a warning. It never turns the result empty.
- `in:"Title"` with several matches picks the best one and returns the resolved page. Chips and stored queries keep the page id, so renames don't break them.
- A query of only negatives means everything except, sorted by updated.

## Results

- Filters are hard constraints. Typo tolerance applies to text only; a correction never loosens a filter.
- With text: ranking as today (title boost, bm25) among candidates.
- Without text: sort by the filtered date field if there is exactly one (past range newest first, future range soonest first), otherwise by updated. `sort:` overrides.
- Totals are bounded ("200+").
- Each hit carries the filter values it matched (for example `due: 2026-10-12`, `tag: work`), so filter-only results have something to show.
- Agent output is rg-style: each page gives its title, id and path, then its matching blocks with `context:N` blocks around them, each block line starting with the block id `edit_page` takes. A short page (at most `full:CHARS`) is shown whole, and a query with only operators shows the start of each page.
- Item results (see Syntax) show each item's block id, `[ ]` or `[x]`, its date and the reference `update_task` takes.
- One result stays under 12,000 characters. When it would be larger, previews shrink first (no context, then shorter lines), and only then are hits left out; the result says what it trimmed. Narrow the query, page with `offset`, or raise `limit:` to see more.
- `read_page` with an unknown id or title returns a labelled list of pages whose titles contain those words. It never opens a near match.

## External sources and grouped search

Every hit carries its `source`, source-local `id`, `page_type` and optional opaque `target`. Vault pages use `vault`. External rows are kept separately from page metadata and share the title, FTS ranking and typo paths; they never enter the page tree, page count, page editing or vault sync. Vault clear and reindex rebuild only vault-owned rows, preserving external rows and their search text. Other sources own their upserts, removals and source clears.

Agent chats are the first source (`agent`, type `chat`). Their title rows and chapter and section rows search display names and summaries, with session and first/last message IDs in the target. There is no raw transcript search. Hub chats contribute title rows until outlines are available. [Agent wiring](../integrations/agent.md#chat-search-source) reconciles the source with the chat list and outline revisions. Existing page pickers, home inline search and page-title resolution keep their page-only scope; the core's `search_pages` helper constrains the source before ranking and the limit, retaining unknown vault types.

Ctrl+K and mobile search show Pages, then Chats, retaining core ranking within each group. Chat labels read `Chat title › Chapter name`, adding `› Section name` only for a section hit; numbers are not labels. Activation opens the chat at the stored anchor. All, Pages and Chats chips write or clear `type:` in the query; typing `type:document` or `type:database` also has a matching chip. Other operators and the typo/narrowing explanations remain available. Property, link and subtree filters still constrain rows to their actual indexed data; a chat has no vault properties or parent subtree.

The `in:"Title"` regression check resolves the title to a stored page ID, excludes an identically titled external row and retains the subtree after the page is renamed.

**Title-hit snippets (follow-up, 2026-10-12):** title-tier hits fetch their FTS body previews in one query over their stored IDs, before the title/body tiers are merged. Their title-first order is unchanged. A title-only match may show the opening body text; chapter and section rows therefore retain their summary previews too. Broad adaptive searches retain their existing path without snippets. Core regression tests cover full, adaptive and page-only search, including a page behind 120 identically titled chat rows.

**Follow-up verification (2026-10-12, Windows):** the merge from pinned master `b5d83c6` is built with regenerated bindings. `flutter analyze`, the final direct `dart analyze`, the add-on import check and its self-tests pass. Two later Flutter analyzer retries stalled and were stopped; the direct Dart run checks the final tree. Core, tools and bridge tests pass (311 tests), including three title-snippet regressions; the agent desktop file/note summary regression and the Flutter home page-path/snippet regression also pass.

The closed-section fixture now waits for its actual closed chapter, rather than `outlineFor` alone: that field can already refer to a stale empty outline before the revision poll reloads it. Original folding assertions and timeouts remain intact. All four anchor tests pass in three consecutive full suites using `--concurrency=1 --exclude-tags drive`. The full results are 946/946/945 passed, 8 skipped each, and 5/5/6 failures. All three include the known slash-menu heading failure and four grouped-search golden differences from restored summary previews; the third also fails the agent-mention timing gate (215.25 ms p95 versus 87.852 ms). A subsequent isolated Flutter timing run passes. Master has 883 passes, 6 skips and three failures: database table scrolling, the slash-menu heading and the title-snippet regression.

The earlier follow-up regenerated `mobile-light-search-paths.png` and `mobile-dark-search-paths.png` for the intended chips, grouping and highlighting. Follow-up 2 regenerates all four `*-search-groups.png` references, including the restored chapter/section summaries.

**Grouped-search stability (follow-up 2, 2026-10-12):** the reproduced 3.40% mobile difference has the same row order but different chat labels. The fixture opens a local agent before switching to phone layout; a later source sweep used the new platform's Hub title policy and expanded `Trip planning` to `Trip planning: Next steps`, changing line wrapping. Search labels now follow the backend the agent actually opened on. Genuine Hub titles and explicit renames retain their existing policy. Core ranking already has ID tie-breaks and is unchanged. The golden fixture waits for all four expected page/anchor IDs and labels, asserts their ranked order, then asserts the displayed row order before capture. Existing timeouts and golden tolerances remain unchanged.

The native-store fixture also waits for its cached session title and the indexed section row to reflect the same completed automatic rename; its original label assertions are unchanged. Verification on Windows: the new platform-switch regression fails with the old policy and passes with the fix; all four references regenerate in one run, followed by five consecutive successful golden runs (20 cases). The final focused source, grouping, result and anchor checks pass (60 passed, two replay checks skipped), and direct `dart analyze` reports no issues. Rust and bridge code are unchanged in this follow-up; the release DLL remains the previously built worktree DLL. This does not rerun or qualify the full-suite timing gates below.

**Performance limit:** the final Windows release warm-search p95 is 5.28 ms over 950 samples on 10k pages, and fuzzy p95 is 13.17 ms. The whole release gate is not qualified: uncached tag candidate construction still exceeds 20 ms (24.09 ms p95 on this branch; the same isolated gate also fails on pinned master at 27.71 ms). Full-suite Flutter timing stability is also not qualified by the isolated rerun. These are local test results, not packaged Android or physical-phone verification.

**Add-on integration (step 2, 2026-10-12):** the optional [Agent add-on](../addons/agent.md) registers its indexed source through `AddonContext.searchSources`. The kit contract includes group/type and row presentation metadata, opaque targets and source-owned navigation; independent sources can supply query callbacks. Grouped Ctrl+K and mobile search consume these registrations without parsing chat anchors. Agent rows still enter the shared core index through `AgentStore` and `VaultAgentSearchSink`, retaining persistent ranking, filtering and outline revision reconciliation. Unregistered external hits are excluded; with no indexed registrations the host uses page-only search before ranking/limit, preserving page results when Agent is disabled. Core/bridge APIs are unchanged. Existing grouped-search tests and four goldens pass unchanged after migration.

## Rust search latency repair (2026-10-12)

**Historical checks (Windows desktop, hostname not recorded):** the first implementation passed `CARGO_INCREMENTAL=0 cargo test --release -p vault_core` (280 passed, zero failed, two ignored). Its clean gate exited 0, with selective text at 12.338/15.195 ms p50/p95 and uncached tags at 3.018/4.273 ms, but filtered typo remained 17.551/22.857 ms against the stronger 20 ms goal. The final single-query fuzzy-score cache is implemented and its compiled oracle checks pass. These historical diagnostics do not qualify the gate; the self-hosted CI runner A/B acceptance is described below. The Flutter profile-mode check remains open.

On the same historical Windows diagnostic host (hostname not recorded), the unchanged gate on master `4f978b9` reproduced the selective failure at **22.062 / 37.897 ms p50/p95**. A separate clean diagnostic copy retained the 10,000-page corpus, warmups, sample counts, query workload and timing boundaries, but omitted its budget assertions so it could reach the later cases. Temporary per-stage probes and that diagnostic copy are removed from the final source. The diagnostic measured **22.640 / 28.649 ms** for selective text and **18.903 / 24.638 ms** for the previously unreached filtered typo; it is not a passing gate.

| Before stage | p50 ms | p95 ms | Samples |
| --- | ---: | ---: | ---: |
| Selective candidate-cache lookup | 0.008 | 0.013 | 50 |
| Selective breadth count | 1.614 | 2.229 | 50 |
| Selective title snippets | 4.125 | 5.708 | 50 |
| Selective ranked SQL | 7.880 | 10.548 | 50 |
| Selective matching-ID collection | 3.269 | 4.345 | 50 |
| Selective decoration | 2.901 | 4.921 | 50 |
| Uncached tag schema read (twice per query) | 2.054 | 3.017 | 200 |
| Uncached tag candidate SQL and set construction | 1.906 | 2.911 | 100 |
| Uncached tag filter-only sorting | 1.030 | 1.526 | 100 |
| Filtered typo matching-ID collection | 7.417 | 10.255 | 50 |

Stage p95s are not additive. Candidate lookup is not the selective bottleneck. `EXPLAIN QUERY PLAN` puts the title query's `json_each` scan inside its corpus-wide FTS loop. Title IDs now resolve once to a rowid membership list; ranked snippet retrieval also restricts the FTS cursor to the materialized, limited ranked rowids before joining. MATCH expressions, title-first order, bm25, tie-breaks and snippet arguments are unchanged. Candidate sets already below the breadth threshold skip the redundant posting count.

Uncached tags previously read and parsed all schemas twice per query, and sparse filter-only sorting walked `search_rows_updated` while testing candidate membership. Full schema definitions now share the existing `(data_version, total_changes)` cache guard and caller-transaction isolation; small sorted sets drive primary-key lookups from `rarray` before applying the existing order and window. The filtered typo's bounded total used to rank fuzzy titles and construct correction labels; its count path now reuses the same matcher, stops at the bound and collects only IDs. The ranked and count paths also share one complete, last-query fuzzy-score cache, keyed by normalized words and edit eligibility. Scope constraints apply when selecting hits or IDs. Title-vector replacement and foreground patching clear the scores. Ranked fuzzy hit production remains unchanged.

New regression tests compare snippets with SQLite's legacy per-row snippet function, check ordering and windows against direct SQL, and cover Unicode, external negative rowids, tag annotations, schema edits on both connections, rollback isolation, and fuzzy-count scope/membership. Existing test expectations are unchanged. No bridge API, app, binding, corpus, sample count or gate ceiling changes are made.

## Interleaved A/B acceptance (2026-10-13)

Acceptance runs through `search-ab.yml` in the private ops repo (BubbleBuffer/scratch-ops), a `workflow_dispatch` job on **the self-hosted CI runner** (`runs-on: [self-hosted, <ci-runner>]`). Each run has at least five alternating AB/BA real pairs and an interleaved baseline/baseline control in the same session. Both control sides use the identical retained baseline executable, verified by SHA-256. For each case, the control spread is `abs(control median-round p50 ratio - 1)` and the noise band is `max(5%, control spread)`. A case fails only when its candidate/baseline median-round p50 ratio exceeds `1 + band`; p95 ratios are informational. The job prints the band and verdict per case and exits nonzero on FAIL. **Control-spread cap (2026-10-13), per case:** a case whose control spread is 10% or more is rerun once; if it is still 10% or more, it is reported **inconclusive** and neither passes nor fails. A run passes only if every conclusive case is within its noise band and at most 2 cases are inconclusive. (A noisy control otherwise widens the band until any candidate passes.) The harness does not enforce the cap yet; that change comes with the next rerun. The self-hosted CI runner is not isolated: it shares the Proxmox host with the Linux machine (both report an i5-1135G7), so other containers' load reaches its runs. Under the cap, the 2026-10-13 final gate does not pass: every unfiltered case had control spread of 45-67%, far more than 2 inconclusive cases; a confirmation rerun waits for GitHub billing to be fixed. **The Linux machine A/B verdicts are superseded for acceptance**; local and Linux machine harness runs remain diagnostics. Absolute hosted-runner and UI latency qualification are separate.

Dispatch with Scratch revisions (omit `candidate` to use Scratch `main`; the old example revisions below are from the former private repo):

```bash
gh workflow run search-ab.yml -R BubbleBuffer/scratch-ops \
  -f baseline=4f978b9 -f candidate=f6e411c -f pairs=5
```

### The self-hosted CI runner result: pre-lane baseline versus master (2026-10-13)

**`verdict: FAIL (21 cases beyond band)`**. Baseline `4f978b9` versus candidate `f6e411c`; five real pairs and five same-binary control pairs, 20 interleaved rounds total. All 20 rounds exited zero with all 47 manifest cases and unchanged sample counts/correctness assertions. The gate exited 1 because 21 p50 ratios exceed their case's band. Work stopped at this initial result; the fixed-cost investigation below subsequently reopened diagnosis.

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


### Fixed-cost investigation (2026-10-13)

The nine non-merge core commits between `4f978b9` and `f6e411c` were inspected before dispatching comparisons. Ranked suspects: **`8712f52`** changes the exact-title scan's scope/mask/skip checks and snippet SQL; **`3beeb2b`** changes prefix expansion and removes **`0dcfef9`**'s temporary multiword result cache. `8148999` (embeds), `f09b10e` (type data), `e3b876d` (new-page colours), `47e2219` (Markdown type bodies), `3baf902` (partial imports) and `de06e2d` (import remapping) have no new repeated operation on the warm search path. Existing snapshot, tree-epoch, data-version and publication reads predate this range; ordinary unfiltered parsing also bypasses the schema catalog.

**Bisect 1:** parent `4f978b9` versus `8712f52` (run 38406423901 in the former private repo), five real and five same-binary control pairs: **FAIL, 2/47**. All 940 raw case records were independently parsed and reproduce the medians, ratios, bands and verdicts exactly; cleanup is clean. The uniform prefix increase is smaller than the original run, and broad-tag typed prefixes improve here, so this comparison alone does not isolate the entire original regression. No repair is claimed at this step.

| Case | Parent p50 ms | `8712f52` p50 ms | Ratio | Band | Verdict |
|---|---:|---:|---:|---:|---|
| Warm aggregate | 0.821 | 1.065 | 1.297 | 42.51% | PASS |
| `p` | 1.140 | 1.214 | 1.065 | 12.47% | PASS |
| `pr` | 0.741 | 0.818 | 1.104 | 15.89% | PASS |
| `pro` | 0.630 | 0.693 | 1.100 | 15.36% | PASS |
| `proj` | 0.699 | 0.752 | 1.076 | 14.52% | PASS |
| `project` | 0.669 | 0.718 | 1.073 | 12.94% | PASS |
| `g` | 1.172 | 1.265 | 1.079 | 17.88% | PASS |
| `coffee mor` | 3.631 | 3.892 | 1.072 | 12.73% | PASS |
| `zür` | 0.583 | 0.669 | 1.148 | 11.88% | FAIL |
| `big page` | 2.081 | 2.275 | 1.093 | 9.31% | FAIL |
| Typed `p`, broad tag | 3.970 | 2.595 | 0.654 | 5.00% | PASS |
| Typed `proj`, broad tag | 4.387 | 2.942 | 0.671 | 5.00% | PASS |
| Typed `project`, broad tag | 4.322 | 2.881 | 0.667 | 5.00% | PASS |

**Bisect 2:** parent `ced7d5f` versus `3beeb2b` (run 38408979216 in the former private repo), five real and five same-binary control pairs: **FAIL, 17/47**. Single-word p50 ratios are 0.978-1.012 and all pass; this commit does not reproduce their uniform regression. Its parent retains `0dcfef9`'s warm whole-result cache for broad unfiltered multiword queries, so the large multiword differences include removing that cache. Filter-only and typed-filter paths do not use that result cache, and their 31-50% increases remain under investigation. The release correctness build on the Linux machine overlapped this run; load is recorded in the artifacts. All 940 raw records independently reproduce the summary and cleanup is clean.

All failures, plus representative unchanged single words (milliseconds):

| Case | Parent p50 | `3beeb2b` p50 | Ratio | Band | Verdict |
|---|---:|---:|---:|---:|---|
| `query "p"` | 1.886 | 1.858 | 0.985 | 5.00% | PASS |
| `query "project"` | 1.229 | 1.234 | 1.004 | 5.00% | PASS |
| `query "g"` | 2.008 | 1.978 | 0.985 | 5.00% | PASS |
| `query "weekly review"` | 0.492 | 1.572 | 3.195 | 5.00% | FAIL |
| `query "coffee mor"` | 0.480 | 5.718 | 11.912 | 5.00% | FAIL |
| `fuzzy search, 10,000 pages, limit 50` | 3.187 | 3.588 | 1.126 | 7.41% | FAIL |
| `typo query "cofee mor"` | 0.484 | 9.646 | 19.930 | 5.00% | FAIL |
| `uncached tag candidate construction, 100 distinct filters, 10,000 pages` | 1.932 | 2.847 | 1.474 | 5.00% | FAIL |
| `filtered filter only, "tag:perf/selective", 10,000 pages` | 0.956 | 1.417 | 1.482 | 5.00% | FAIL |
| `filtered selective filter plus text, "project tag:perf/selective", 10,000 pages` | 10.665 | 14.051 | 1.317 | 5.00% | FAIL |
| `filtered broad filter plus broad text, "p tag:perf/broad", 10,000 pages` | 3.349 | 4.376 | 1.307 | 5.00% | FAIL |
| `filtered negatives only, "-tag:perf/excluded", 10,000 pages` | 1.015 | 1.499 | 1.477 | 5.00% | FAIL |
| `filtered deep in, "in:[[<ID>]]", 10,000 pages` | 2.025 | 3.042 | 1.502 | 5.00% | FAIL |
| `filtered tag on 10k, "tag:perf/all", 10,000 pages` | 0.826 | 1.236 | 1.496 | 5.00% | FAIL |
| `typed prefix "p", constant tag:perf/broad, 10,000 pages` | 3.170 | 4.325 | 1.364 | 5.00% | FAIL |
| `typed prefix "pr", constant tag:perf/broad, 10,000 pages` | 2.899 | 3.944 | 1.360 | 5.00% | FAIL |
| `typed prefix "pro", constant tag:perf/broad, 10,000 pages` | 2.824 | 3.807 | 1.348 | 5.00% | FAIL |
| `typed prefix "proj", constant tag:perf/broad, 10,000 pages` | 3.551 | 4.995 | 1.407 | 5.00% | FAIL |
| `typed prefix "project", constant tag:perf/broad, 10,000 pages` | 3.526 | 5.018 | 1.423 | 5.00% | FAIL |
| `filtered typo "gardne tag:perf/broad", 10,000 pages` | 4.714 | 6.559 | 1.391 | 5.00% | FAIL |

Raw controls prevent attributing those filtered failures directly to the commit. For tag-only, the same baseline executable's p50 spans **0.911-1.478 ms**, yet control-side medians **1.421/1.416 ms** cancel to **0.352% spread** and a **5% band**. Both faster and slower phases occur in the same binaries; no new filter-only scan, guard or allocation was found in this commit. Host scheduling/load is a confound, not a changed gate verdict; the focused loop comparison will be made after the release build finishes.

| Pair | Tag-only candidate/baseline | Tag-only same-binary control |
|---|---:|---:|
| 1 | 1.008 | 1.523 |
| 2 | 1.523 | 1.037 |
| 3 | 1.045 | 0.997 |
| 4 | 1.555 | 0.660 |
| 5 | 0.658 | 1.478 |

**Cause and repair:** `8712f52` replaced the explicit predicates in `fuzzy::title_hits_inner` with `in_scope` before the cheap ASCII-mask rejection, moving skip checks ahead of cheap rejection and changing the loop structure for the full cached title scan on each query. `f3d4b6d` restores candidate scope, page-only scope, ASCII mask, then skip checks explicitly; ranking, snippets, corrections, snapshots and invalidation are unchanged. This is not an inherent correctness or filtered-speed trade: schema caching, fuzzy-score sharing and the optimized filtered/snippet SQL remain intact.

**Bisect 3, isolated loop variant:** unmodified current-source parent `e72905a` versus `f3d4b6d` (run 38415238461 in the former private repo): **PASS, 0/47 beyond band**. Only the eight-line loop change differs in core code. Warm aggregate improves **13.5%**, and all 19 warm query medians improve (ratios 0.850-0.894), with 5% control bands throughout those cases. Every filtered and typo case passes. No lead-owned build overlaps this comparison. All 940 raw records independently reproduce the summary and cleanup is clean; helper inlining/assembly was not separately measured.

| Case | Unmodified p50 ms | Fixed p50 ms | Ratio | Band | Verdict |
|---|---:|---:|---:|---:|---|
| `warm search, 10,000 pages, limit 50` | 1.047 | 0.906 | 0.865 | 5.00% | PASS |
| `query "p"` | 1.385 | 1.210 | 0.874 | 5.00% | PASS |
| `query "pr"` | 0.975 | 0.842 | 0.864 | 5.00% | PASS |
| `query "proj"` | 0.913 | 0.802 | 0.878 | 5.00% | PASS |
| `query "project"` | 0.880 | 0.777 | 0.883 | 5.00% | PASS |
| `query "g"` | 1.424 | 1.250 | 0.878 | 5.00% | PASS |
| `query "a"` | 2.407 | 2.067 | 0.859 | 5.00% | PASS |
| `query "t"` | 2.304 | 1.986 | 0.862 | 5.00% | PASS |
| `query "coffee mor"` | 5.672 | 5.051 | 0.891 | 5.00% | PASS |
| `query "café"` | 1.089 | 0.951 | 0.873 | 5.00% | PASS |
| `query "big page"` | 2.304 | 2.059 | 0.894 | 5.00% | PASS |
| `typed prefix "p", constant tag:perf/broad, 10,000 pages` | 2.951 | 2.712 | 0.919 | 14.56% | PASS |
| `typed prefix "pr", constant tag:perf/broad, 10,000 pages` | 2.703 | 2.524 | 0.934 | 14.95% | PASS |
| `typed prefix "pro", constant tag:perf/broad, 10,000 pages` | 2.614 | 2.432 | 0.930 | 14.29% | PASS |
| `typed prefix "proj", constant tag:perf/broad, 10,000 pages` | 3.268 | 3.186 | 0.975 | 14.74% | PASS |
| `typed prefix "project", constant tag:perf/broad, 10,000 pages` | 3.239 | 3.177 | 0.981 | 11.48% | PASS |

**Correctness check, the Linux machine:** `<linux-build-helper> --with /v/projects/BubblePaw-base-agent /v/worktrees/notes-app/lane-search-perf -- bash -lc 'ulimit -n 65536; RUST_TEST_THREADS=2 CARGO_INCREMENTAL=0 cargo test --release -p vault_core -j 2'` exits zero: **378 passed, zero failures, two ignored**, including unit, integration and doc-test suites (child self-probe counts excluded). Existing expectations are unchanged. Local/remote SHA-256 hashes match for `fuzzy.rs`, `index.rs` and `search_exec.rs`; targets remain outside the PC worktree. This qualifies correctness, not Linux timing acceptance.

### Final baseline gate after the title-loop repair (2026-10-13)

The self-hosted CI runner final gate (run 38418744539 in the former private repo): **PASS, 0/47 cases beyond band**, baseline `4f978b9` versus fixed lane head **`c2da667`** (core repair `f3d4b6d`). Five real and five same-binary control pairs, 20 rounds, unchanged 47-case manifest/sample counts and correctness assertions; every round exits zero. All **940 raw case records** independently reproduce the full-precision medians, ratios, spreads, bands and verdicts. Both detached build worktrees are removed, source manifests match and cleanup records no errors. No lead-owned build overlaps this gate; no extra gate was dispatched.

**Interpretation limit:** unfiltered control bands are **45.45-67.17%** in this session. The agreed rule therefore passes even though several unfiltered candidate medians exceed the baseline (warm aggregate ratio **1.150**, coffee **1.375**, `recipies` typo **1.401**). This is the requested relative gate PASS, not evidence of baseline parity or elimination of every residual difference. The isolated loop comparison above is stronger evidence for the removed cost: **13.5% warm p50 improvement**, all warm cases improve with **5% bands**, and filtered/typo cases do not newly fail. Absolute hosted-runner, UI and physical-device qualification remain separate.

All cases below are reported. Times are median-of-round milliseconds; ratios are candidate/baseline. Bands and spreads are percentages. Rounded values are displayed; verdicts use full precision and p95 is informational.

| Case | Baseline p50 / p95 ms | Fixed p50 / p95 ms | p50 ratio | p95 ratio (info) | Control spread | Band | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| `warm search, 10,000 pages, limit 50` | 0.997 / 4.054 | 1.147 / 5.497 | 1.150 | 1.356 | 56.11% | 56.11% | PASS |
| `query "p"` | 1.378 / 1.493 | 1.489 / 1.673 | 1.081 | 1.121 | 53.18% | 53.18% | PASS |
| `query "pr"` | 0.916 / 1.014 | 1.073 / 1.182 | 1.171 | 1.166 | 54.33% | 54.33% | PASS |
| `query "pro"` | 0.789 / 0.899 | 0.940 / 1.027 | 1.191 | 1.142 | 57.33% | 57.33% | PASS |
| `query "proj"` | 0.849 / 0.937 | 1.006 / 1.123 | 1.185 | 1.199 | 61.08% | 61.08% | PASS |
| `query "project"` | 0.813 / 0.936 | 0.981 / 1.053 | 1.207 | 1.125 | 61.65% | 61.65% | PASS |
| `query "g"` | 1.437 / 1.527 | 1.564 / 1.696 | 1.088 | 1.111 | 54.35% | 54.35% | PASS |
| `query "ga"` | 0.964 / 1.053 | 1.087 / 1.310 | 1.128 | 1.244 | 54.00% | 54.00% | PASS |
| `query "gar"` | 0.852 / 1.036 | 0.997 / 1.183 | 1.170 | 1.142 | 56.09% | 56.09% | PASS |
| `query "gard"` | 0.954 / 1.075 | 1.134 / 1.306 | 1.189 | 1.215 | 59.18% | 59.18% | PASS |
| `query "garden"` | 0.901 / 1.133 | 1.058 / 1.204 | 1.174 | 1.063 | 59.38% | 59.38% | PASS |
| `query "a"` | 2.490 / 2.776 | 2.575 / 2.837 | 1.034 | 1.022 | 50.34% | 50.34% | PASS |
| `query "t"` | 2.411 / 2.574 | 2.487 / 2.777 | 1.032 | 1.079 | 52.17% | 52.17% | PASS |
| `query "tomato"` | 1.051 / 1.110 | 1.182 / 1.468 | 1.125 | 1.323 | 56.94% | 56.94% | PASS |
| `query "weekly review"` | 1.433 / 1.533 | 1.248 / 1.466 | 0.871 | 0.956 | 59.67% | 59.67% | PASS |
| `query "coffee mor"` | 4.391 / 4.767 | 6.037 / 6.917 | 1.375 | 1.451 | 56.61% | 56.61% | PASS |
| `query "zür"` | 0.738 / 0.807 | 0.874 / 1.226 | 1.184 | 1.519 | 57.74% | 57.74% | PASS |
| `query "café"` | 1.045 / 1.167 | 1.193 / 1.347 | 1.142 | 1.154 | 54.48% | 54.48% | PASS |
| `query "big page"` | 2.474 / 2.604 | 2.503 / 2.862 | 1.012 | 1.099 | 45.45% | 45.45% | PASS |
| `query "xylophone"` | 0.923 / 1.029 | 0.703 / 0.819 | 0.762 | 0.796 | 67.17% | 67.17% | PASS |
| `fuzzy search, 10,000 pages, limit 50` | 3.662 / 8.446 | 2.516 / 11.462 | 0.687 | 1.357 | 54.18% | 54.18% | PASS |
| `typo query "tomatoe"` | 4.133 / 4.488 | 2.719 / 3.152 | 0.658 | 0.702 | 53.35% | 53.35% | PASS |
| `typo query "gardne"` | 4.116 / 4.452 | 2.527 / 2.986 | 0.614 | 0.671 | 54.71% | 54.71% | PASS |
| `typo query "weekyl review"` | 3.657 / 4.469 | 2.591 / 2.941 | 0.709 | 0.658 | 64.88% | 64.88% | PASS |
| `typo query "cofee"` | 3.931 / 4.595 | 2.496 / 2.721 | 0.635 | 0.592 | 54.44% | 54.44% | PASS |
| `typo query "cofee mor"` | 7.865 / 9.559 | 8.890 / 9.880 | 1.130 | 1.034 | 56.64% | 56.64% | PASS |
| `typo query "projcet"` | 2.219 / 2.547 | 1.830 / 2.308 | 0.825 | 0.906 | 58.58% | 58.58% | PASS |
| `typo query "recipies"` | 8.240 / 10.397 | 11.545 / 12.861 | 1.401 | 1.237 | 60.35% | 60.35% | PASS |
| `typo query "gadr"` | 4.790 / 5.888 | 3.031 / 3.258 | 0.633 | 0.553 | 49.24% | 49.24% | PASS |
| `typo query "meeitng"` | 2.636 / 3.462 | 2.041 / 2.977 | 0.774 | 0.860 | 58.45% | 58.45% | PASS |
| `typo query "buget"` | 2.432 / 2.893 | 1.660 / 2.139 | 0.683 | 0.739 | 56.17% | 56.17% | PASS |
| `typo query "desgin"` | 3.445 / 3.905 | 2.115 / 2.471 | 0.614 | 0.633 | 52.64% | 52.64% | PASS |
| `typo query "journla"` | 2.084 / 2.313 | 1.666 / 2.053 | 0.799 | 0.888 | 61.68% | 61.68% | PASS |
| `typo query "databsae"` | 2.719 / 2.962 | 2.010 / 2.413 | 0.739 | 0.815 | 57.44% | 57.44% | PASS |
| `uncached tag candidate construction, 100 distinct filters, 10,000 pages` | 5.404 / 6.465 | 2.019 / 2.176 | 0.374 | 0.337 | 12.97% | 12.97% | PASS |
| `filtered filter only, "tag:perf/selective", 10,000 pages` | 2.930 / 3.024 | 1.015 / 1.090 | 0.346 | 0.360 | 14.94% | 14.94% | PASS |
| `filtered selective filter plus text, "project tag:perf/selective", 10,000 pages` | 18.168 / 18.385 | 10.258 / 10.929 | 0.565 | 0.594 | 5.25% | 5.25% | PASS |
| `filtered broad filter plus broad text, "p tag:perf/broad", 10,000 pages` | 5.335 / 5.425 | 3.437 / 3.774 | 0.644 | 0.696 | 11.93% | 11.93% | PASS |
| `filtered negatives only, "-tag:perf/excluded", 10,000 pages` | 2.772 / 2.831 | 1.004 / 1.092 | 0.362 | 0.386 | 11.40% | 11.40% | PASS |
| `filtered deep in, "in:[[<ID>]]", 10,000 pages` | 3.770 / 3.856 | 2.131 / 2.286 | 0.565 | 0.593 | 11.20% | 11.20% | PASS |
| `filtered tag on 10k, "tag:perf/all", 10,000 pages` | 2.346 / 2.458 | 0.831 / 0.891 | 0.354 | 0.362 | 13.42% | 13.42% | PASS |
| `typed prefix "p", constant tag:perf/broad, 10,000 pages` | 5.125 / 5.751 | 3.515 / 3.831 | 0.686 | 0.666 | 12.96% | 12.96% | PASS |
| `typed prefix "pr", constant tag:perf/broad, 10,000 pages` | 4.814 / 5.360 | 3.280 / 3.644 | 0.681 | 0.680 | 11.52% | 11.52% | PASS |
| `typed prefix "pro", constant tag:perf/broad, 10,000 pages` | 4.645 / 5.156 | 3.171 / 3.580 | 0.683 | 0.694 | 13.64% | 13.64% | PASS |
| `typed prefix "proj", constant tag:perf/broad, 10,000 pages` | 5.771 / 6.326 | 3.967 / 4.317 | 0.687 | 0.682 | 10.79% | 10.79% | PASS |
| `typed prefix "project", constant tag:perf/broad, 10,000 pages` | 5.720 / 6.388 | 3.959 / 4.340 | 0.692 | 0.679 | 10.67% | 10.67% | PASS |
| `filtered typo "gardne tag:perf/broad", 10,000 pages` | 10.870 / 11.009 | 4.581 / 5.825 | 0.421 | 0.529 | 3.86% | 5.00% | PASS |

### Earlier Linux machine diagnostics: superseded for acceptance

On the Linux machine (Intel Core i5-1135G7, four available CPUs), the old fixed 5% p50/p95 rule reported FAIL for `4f978b9` → `333d1ea` (five pairs, fourteen warm p50 cases) and `333d1ea` → `3beeb2b` (fourteen pairs). Coffee median p50/p95 improved from 6.128/8.078 to 5.783/7.905 ms; a later five-pair confirmation still failed two p95 cases. A separate identical-binary control failed 34/47 cases under ambient load (1-minute averages 3.10–18.68). These historical verdicts do not decide current acceptance. Raw records remain outside git in `/home/dev/work/search-ab-results/{merged-lane-v3,coffee-3beeb2b-quiet-5,control-333d1ea-same-binary-5}/`.

**Historical correctness verification, the Linux machine:** `<linux-build-helper> V:/worktrees/notes-app/lane-search-perf -- ... cargo test --release -p vault_core -j 2` passed: **324 tests, zero failures, two ignored**, with `RUST_TEST_THREADS=2` and the command's descriptor limit 65,536. The new complete-membership, serialized-fill, Unicode, overflow-boundary and phrase/operator fallback regression passes; existing expected values and goldens are unchanged. Local and remote modified-source SHA-256 hashes matched before the run. The first focused invocation selected zero tests and is compile evidence only; the full suite actually ran the new regression. Harness selftests also pass on Windows and the Linux machine. Absolute hosted-runner, Flutter UI/golden and physical-device qualification were not run here.

## Search index storage evaluation (2026-10-12)

**Evaluated 2026-10-12, not pursued (decided 2026-10-13).** Best option within the speed/snippet constraints saves 5.5%; external content saves nothing because the index holds the only copy of the text; smaller prefix indexes make prefix queries 3-6x slower; contentless changes snippets; `detail=column` breaks phrase queries. No production schema change. Starting commit `e46d35063b0c84a6bcffe65542ccc7b2ff783a79`. The fresh gate corpus is exactly 10,000 pages (9,999 ordinary pages plus the 2,000-block page), with **8,418,819 markdown bytes** and **29,454,336 index.sqlite bytes** after checkpoint. These fresh measurements replace the earlier approximate 32 MB size for this comparison.

Current state stays at schema **9**: standalone FTS5 title/body content, `unicode61 remove_diacritics 2`, `prefix='1 2 3'`, full detail, column sizes enabled, 4,096-byte SQLite pages, no automatic vacuum. Vault body text already exists only once inside SQLite, in `fts_content`; external rows also keep their source-owned text in `search_external`. An older schema is rebuilt on open by the existing version mechanism. This pass does not bump that version or install a new layout.

### Size and paired speed screening

Machine **The Linux machine**, Intel Core i5-1135G7, **4 available CPUs** (8 logical CPUs reported by lscpu). The host was loaded: uptime averages were **15.39 / 17.21 / 13.39** at 21:57:01 UTC and **12.01 / 13.48 / 12.94** at 22:03:35 UTC. No artificial load was added.

All values below are **decimal MB**. Raw total is after the option's rebuild/maintenance, before the common final VACUUM; compact total is after VACUUM. Content combines `fts_content` and any replacement normal content table. The speed column shows the worst of 37 per-query p50 ratios for that option, with **old → option milliseconds**. Calls alternate order each sample within the same process, **50 old/option pairs per query**. This is option screening, not full gate qualification: it includes the 19 exact and 13 typo gate queries plus five operator queries, but omits the gate's filtered metadata setup/workloads.

| Option | Raw total MB | Compact total MB | fts_data MB | Content MB | fts_docsize MB | fts_idx MB | Worst paired p50, old → option ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| before | 29.454 | 29.454 | 12.018 | 8.040 | 0.139 | 0.020 | Reference |
| rebuilt123 | 29.454 | 28.127 | 12.001 | 7.729 | 0.106 | 0.025 | `proj` 1.321 → 1.366 (1.03×) |
| external123 | 29.454 | 28.127 | 12.001 | 7.729 | 0.106 | 0.025 | `desgin` 1.740 → 1.818 (1.04×) |
| external_view123 | 29.454 | 27.914 | 12.001 | 7.516 | 0.106 | 0.025 | `pro` 0.820 → 1.165 (1.42×) |
| contentless123 | 29.454 | 20.398 | 12.001 | 0.000 | 0.106 | 0.025 | Rejected before timing |
| detail_column | 29.454 | 22.245 | 6.128 | 7.729 | 0.106 | 0.016 | Rejected before timing |
| detail_none | 29.454 | 18.149 | 2.036 | 7.729 | 0.106 | 0.012 | Rejected before timing |
| columnsize0 | 29.454 | 28.021 | 12.001 | 7.729 | 0.000 | 0.025 | `big page` 3.344 → 4.485 (1.34×) |
| prefix23 | 29.454 | 26.292 | 10.170 | 7.729 | 0.106 | 0.020 | `p` 1.339 → 10.477 (7.82×) |
| prefix12 | 29.454 | 24.621 | 8.499 | 7.729 | 0.106 | 0.020 | `gar` 0.878 → 2.927 (3.33×) |
| prefix1 | 29.454 | 21.610 | 5.493 | 7.729 | 0.106 | 0.016 | `gar` 0.894 → 2.647 (2.96×) |
| prefix2 | 29.454 | 22.790 | 6.672 | 7.729 | 0.106 | 0.016 | `p` 1.306 → 9.364 (7.17×) |
| prefix3 | 29.454 | 23.278 | 7.160 | 7.729 | 0.106 | 0.016 | `p` 1.323 → 14.429 (10.91×) |
| prefix_none | 29.454 | 19.767 | 3.654 | 7.729 | 0.106 | 0.012 | `p` 2.017 → 13.390 (6.64×) |
| page1024 | 31.223 | 30.660 | 12.315 | 9.284 | 0.104 | 0.022 | `xylophone` 1.060 → 1.358 (1.28×) |
| page8192 | 28.393 | 27.836 | 11.993 | 7.348 | 0.115 | 0.033 | `gar` 0.769 → 0.810 (1.05×) |
| page16384 | 28.508 | 27.935 | 11.977 | 7.176 | 0.131 | 0.049 | `gadr` 1.260 → 1.319 (1.05×) |
| auto_full | 28.172 | 28.164 | 12.001 | 7.729 | 0.106 | 0.025 | `coffee mor` 4.140 → 5.071 (1.22×) |
| auto_incremental | 28.713 | 28.164 | 12.001 | 7.729 | 0.106 | 0.025 | `t` 3.406 → 3.613 (1.06×) |
| vacuum_only | 28.135 | 28.135 | 12.018 | 7.729 | 0.106 | 0.016 | `coffee mor` 6.366 → 6.549 (1.03×) |
| optimize | 33.116 | 28.099 | 11.985 | 7.729 | 0.106 | 0.012 | `cofee mor` 7.491 → 7.995 (1.07×) |
| optimize_vacuum | 28.099 | 28.099 | 11.985 | 7.729 | 0.106 | 0.012 | `t` 2.273 → 3.050 (1.34×) |
| prefix_none_page8192 | 27.845 | 19.464 | 3.645 | 7.348 | 0.115 | 0.008 | `p` 1.989 → 13.137 (6.60×) |
| prefix1_page8192 | 27.845 | 21.316 | 5.480 | 7.348 | 0.115 | 0.025 | `gar` 1.255 → 3.801 (3.03×) |
| prefix12_page8192 | 27.845 | 24.330 | 8.495 | 7.348 | 0.115 | 0.025 | `gar` 1.334 → 3.915 (2.93×) |

Full **per-table/index dbstat bytes**, including `fts_idx`, metadata, empty tables, free-page counts and every per-query p50/p95 pair, are in [the measurement artifact](../../crates/core/benches/search_index_20261012.json). Before compaction, 127 free pages account for 520,192 bytes of the reference file. Automatic-vacuum pointer-map pages are recorded separately in the artifact because dbstat does not assign them to a named tree.

The external normal table merely relocates text: its compact total equals the internal-content control. The external view uses `pages.title` plus a body-only normal table and retained external text, saving only 212,992 bytes against that control. Its extra lookup produces a 42% p50 regression for `pro` in this screening.

Contentless tables return changed snippets/highlights. `detail=column` and `detail=none` reject the quoted-phrase query. They are excluded before API timing. The other 21 options match ordered rowids, BM25 doubles, snippets and highlights for all 22 SQL cases (including phrase, prefix, NEAR and a title column filter). All 21 also match the entire serialized adaptive, full, page-only and parsed-spec API results over 37 queries, including correction labels and source metadata. `columnsize=0` preserves those answers but is 34% slower for `big page`, saving only 106,496 bytes against the compact control. See [SQLite's FTS5 content, detail and column-size options](https://sqlite.org/fts5.html) for their contracts.

Every prefix reduction has a material regression. Keeping only `1` makes `ga`/`gar` nearly 3× slower; removing `1` makes `p` and `g` roughly 5–11× slower. Full positional data and all three prefix indexes therefore remain necessary for the current execution path. Optimizing FTS segments changes little: nearly all 12 MB of `fts_data` is payload, not free space (11,816,825 payload bytes versus 109,881 unused bytes in dbstat).

The best remaining measured layout is **8,192-byte SQLite pages plus VACUUM**, at **27,836,416 bytes**, only **5.49% below the fresh reference**; its worst screened p50 ratio is 1.05×. This does not deliver the requested substantial shrink. Index-size work is not pursued under the current speed/snippet constraints (decision 2026-10-13).

### Checks and limits

- Size/SQL/API probes completed successfully on the Linux machine. Existing expected values and goldens were not edited.
- A schema-9 migration regression was drafted for a provisional schema-10 candidate, but the candidate was withdrawn after the speed failure. The final draft was not run or shipped; it is kept outside the repository.
- Full release core testing was started for that provisional candidate. No passing result is claimed; the owned build was stopped at the design-choice boundary (exit 130).
- [The loaded-host diagnostic](../../crates/core/examples/search_index_ab.rs) retains the original gate's corpus, warmups, sample counts, limits, filtered workloads and quality assertions, omitting only six absolute timing assertions. It was built against the starting commit, but three full old/new gate pairs were not completed because no acceptable substantially smaller schema was selected.
- The original `search_perf.rs` and its absolute ceilings remain unchanged. Absolute numbers come from the lead's GitHub-hosted runner; acceptance uses the self-hosted CI runner dispatch job with its same-binary noise band (decision 2026-10-13). This storage screening does not qualify the gate.
- No app, bridge API, bindings, CI or script changes; no push. The work stops as measurements, not as an accepted size reduction.

## Execution and performance

- The index projects Tasks status (`todo`/`doing`/`done`, default `todo`) into a searchable `status` property for task pages without changing CRDT properties or Markdown exports; index version 12 rebuilds existing vault indexes once.
- The filter compiles through `query.rs` `filter_sql` (made crate-visible, numbered parameters) into a candidate id set, cached per normalized filter and SQLite `(data_version, total_changes)` guard. Schema definitions share that guard and transaction isolation. More typed text does not re-run the filter.
- Small candidate sets (up to 5,000 rows) restrict FTS with `+rowid IN rarray(?)` (rusqlite `array` feature); filter-only sorting visits those rows directly. Large sets run FTS first and test hits against the cached set. The breadth count counts filtered matches and is skipped when the candidate set itself is already below the threshold.
- The in-memory title and typo paths test hits against the cached set.
- The `pages.created_ms` index and indexed `(key, value, page)` `search_multi` table are present. Existing SEP/JSON property values are preserved; atomic backfill and triggers maintain the derived multi-select table.
- `in:` resolves the subtree once (recursive CTE) and caches it until `TreeChanged`.
- Perf cases added to `search_perf.rs`: filter only, selective filter plus text, broad filter plus broad text (worst case), negatives only, deep `in:`, tag on 10k, and typed prefixes under a constant filter (cache check).

## API

- `parse_search(q) -> { spec, tokens }`: each token has its byte span and kind (text, phrase, exclude, operator with key, value and resolved id, error). Called per keystroke for chips.
- `complete_search(q, cursor)`: keys (built-ins and existing property keys) and values (tag and select options, page titles for `in:` and `links:`, date words).
- `search_spec(spec, limit, offset) -> { hits, total_bounded, narrowed, warnings }`. `search(string)` is parse plus `search_spec`.
- `upsert_search_row(source, row)`, `delete_search_row(source, id)` and `clear_search_source(source)` let external sources own their rows. A row supplies its local ID, title, plain text body, note type, opaque target and updated timestamp.
- `spec.to_query_string()` prints a spec back as text. Dynamic links store the spec (with page ids) and display the string.
- Agent `search_notes` keeps its query string; its description lists the operators, and its result includes the parsed reading and warnings. `agenda`, `query_database` and `memory_search` are hidden aliases of it, added in plugin 0.6.3 and to be removed in the following version; their old arguments map onto a query (`in:[[Database]] -type:database` for rows, `in:memory` for memories, item operators for planning).

<a id="live-queries-proposed"></a>

## Live queries

- The app's live query cards take the same query language as the agent's `search_notes`. The app ignores `context:`, `full:` and `limit:`; a saved card keeps them in its query string.
- Every writer chunk commit and foreground index update advances an in-memory `u64` generation and publishes `IndexCommitted { generation, pages }` after the commit. Generations are monotonic within one vault open. `PageChanged` still fires early, before the background index write (up to 2 s earlier).
- `watch_query(spec, limit)` registers a vault-owned query and returns a subscription with a handle. Its first update is an ordered snapshot, with every hit in `added`, even when the result is empty. `unwatch_query(handle)` or dropping the subscription removes it. The caller chooses the hit cap; the card uses 50. Totals use `total_bounded` and saturate at 201 ("200+").
- A dedicated worker coalesces commit events into a union of committed page ids. A single SQL statement binds those ids with `rarray`, resolves their rowids, and probes the shared filter/FTS constraints. A page that was already in the last window also passes the gate. Only relevant queries are searched again; unchanged results produce no update. This work runs off the writer thread; committing only advances the counter and publishes the feed event.
- The text eligibility probe is conservative for fuzzy and Unicode matches, so it can cause an extra search whose unchanged result is suppressed. The registry retains the limited result window: removing a match outside that window can leave the total unchanged until the next qualifying commit or tree invalidation. Corpus-wide ranking changes from an otherwise unrelated page also do not bypass the specified gate. Indirect dependencies follow that same gate: for example, editing the outgoing links of a `from:` source that is outside the result window need not wake the query.
- Updates carry `{ generation, added, removed, moved, changed, total }`. A hit's identity is `(source, id)`; `removed` and `moved` carry that pair, with zero-based final positions for moves. New hits also have move entries so consumers can place them correctly. Apply removals and additions, place moved keys at their final positions simultaneously, then replace the hit data in `changed`. Changes compare the existing `SearchHit` fields, including title, snippet and matched filter values.
- Tree changes re-run queries with `in:` (and link-dependent operators). External-source commits and whole-index rebuilds publish an empty page-id list, which invalidates every live query.
- Collaborator edits use sync import on the foreground connection, rather than the vault writer. Sync import publishes its own commit events, so collaborators update the same live-query registry.
- The bridge exposes `watch_query(query, limit, sink)` and `unwatch_query(handle)`. Each stream update includes its handle, generation and an `initial` flag. Explicit unwatch finishes an idle stream. The pinned FRB `StreamSink` detects a closed Dart port on its next send; consumers should unwatch before cancelling an idle stream.
- The raw core `poll_changes` (in `media.rs`) and `wait_changes` (in `sync.rs`) include index commits. Sync transport uses `poll_sync_changes` and `wait_sync_changes`, with a separate document-only cursor and retention window: index-only activity neither triggers local or hub sync nor forces a history reset. The bridge's legacy polling methods use that document feed. Dart `ChangeFeed` also drops index commits before delivering batches, so existing page/tree listeners do not reload because of them. Handwritten Dart has no switch over `ChangeKind` to extend.

Verification on the Linux machine (2026-10-15): `cargo test -p vault_core --locked` passes with 703 passed, zero failed and six ignored; `cargo test --workspace --locked` passes with 1,056 passed, zero failed and six ignored. The 14 live-query integration tests cover initial snapshots, membership, moves, changed hits, unrelated edits, sync and Markdown imports, tree moves, source identity, unwatch, generations, registration races and computed dependencies. Unit tests also cover commit coalescing and sink teardown; transport tests check that index-only activity never starts sync or evicts document history. Bindings regenerate with Flutter on PATH and Flutter analysis reports no issues.

The unchanged release `search_perf` gate passes with timing assertions enabled over its 10,000-page corpus (`CARGO_INCREMENTAL=0 cargo test --release -p vault_core --test search_perf --locked -- --ignored --nocapture`). Normal search p50/p95 is 1.035/5.346 ms overall; fuzzy search is 2.282/10.648 ms overall. Across individual, filtered, uncached and typed-prefix cases, the highest normal p95 is 14.089 ms and the highest fuzzy p95 is 17.687 ms, below the [performance targets](../product/performance.md) of 20/30 ms. This measures Rust search, not UI presentation latency.

### Dynamic search card UI (approved 2026-10-13)

Mockup: [design/search-card/index.html](../../design/search-card/index.html), screenshots in `design/search-card/shots/`. Live queries in core and the bridge stream are built; the card UI is built as an add-on, see Current state below.

- **Form A, block card (default).** The header shows the query string, with `#tags` in the accent colour. Clicking the query edits it: Enter applies, Esc cancels. The header has a count and a collapse toggle.
- **Results by type:** a checkbox for tasks, a status label for board rows, an icon for pages.
- **Length:** the block shows 5 rows, then "Show N more".
- **Ticking a task** removes it from the list, with an Undo message.
- **Empty state:** a short line per query, e.g. "No open #work tasks."
- **Form B, inline chip (compact form of the same query),** e.g. "4 open #work tasks". It opens a popover on desktop and a bottom sheet on phone; the footer offers "Open in Search" and "Show as a block".
- **Settled 2026-10-15:** chip wording, checkboxes only for task pages, and the phone Undo placement (see Decisions).

**Current state (2026-10-16).** Form A is built as the optional [Search card](../addons/search-card.md) add-on. It has the header with query and count, edit on click, five rows then "Show N more", live updates, task checkboxes, and per-query count and empty wording (`queryWording`). Board rows show their Status as a pill; other pages show their own icon. The Form B inline chip is built: a query mark draws its live count in a paragraph, a tap opens the results panel (popover on desktop, sheet on phone, Undo above the footer), and "Show as a block" and "Show inline" convert between chip and block, each as one undo step. The chip has no keyboard action; see [Search card](../addons/search-card.md).

**Storage format.** A `query` block stores the query string printed by `spec.to_query_string()` in its plain block text, including resolved page references in that format. It has no children or rich-text marks and contributes no body text to the search index. An inline `Query(spec)` mark stores the same query string over a normal, searchable text label. Markdown round-trips the block as a fenced `query` code block and the chip as `<span data-query="…">label</span>`, with the query attribute escaped as HTML entities. Older clients show a query block as an unknown block, export it as an annotated blockquote, and index its query text.

## Decisions

- 2026-10-10: `date:` matches any date on the page, not only `updated`.
- 2026-10-10: bare relative durations mean the past; `+N` means the future.
- 2026-10-10: unreadable operator values are ignored with a warning, not an empty result.
- 2026-10-10: no `OR` or parentheses in the first version; a comma covers any-of.
- 2026-10-13: the dynamic search card is built as approved: Form A (block) by default, Form B (inline chip) as its compact form.
- 2026-10-15: the index projects Tasks status as a searchable `status` property (index version 12).
- 2026-10-15: every index commit bumps the index generation and emits `IndexCommitted`; `PageChanged` is unchanged.
- 2026-10-15: live queries push through a bridge stream, and the count uses `total_bounded` ("200+").
- 2026-10-15: a query block is a new `query` block kind, and an inline chip is a `Query(spec)` mark.
- 2026-10-15: the card is an add-on with no add-on dependencies; only task pages get a checkbox, and ticking sets status to done.
- 2026-10-15: the bridge stream unregisters a dropped Dart stream on its next push, so consumers call `unwatch_query` on dispose.
- 2026-10-15: the Search screen and the mention menu have no query entry point in v1.
- 2026-10-20: `in:` takes path globs (`Work/*` direct children, `Work/**` descendants, `**/Meetings` any depth). A plain `in:Work` keeps meaning Work and its subtree, and an exact title wins over the path reading.
- 2026-10-20: `tag:` matches only tags and select options. A text property named Tags is searched by its name.
- 2026-10-20: the agenda role is a calendar setting only. Search returns every dated database row, whatever its database's agenda role.
- 2026-10-20: overdue is `is:todo,task date:<now done:no`, which lists only things that can be ticked off.
- 2026-10-20: the query language gains item operators (`is:`, `done:`, `assignee:`, `calendar:`, `undated`), output-size operators (`context:`, `full:`, `limit:`), `sort:<key>,desc|asc` and `in:memory`; agent results are rg-style under a 12,000-character budget, and `agenda`, `query_database` and `memory_search` become hidden aliases of `search_notes`.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Architecture > Search queries and live queries).
