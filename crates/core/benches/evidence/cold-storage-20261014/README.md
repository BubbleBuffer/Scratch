# Cold storage verification and timings, 2026-10-14

Master is pinned to `f9fdc814167d71fd03ad329b5be75ea1cfbee52a`; the candidate is `036f4e96c3c365ed3b22067f780a70ed549a46f6`. The before-change profile uses the merged production tree `e9e448cdf730a7038d925864187f8ef39995db2b` from `3462914`. Its test-only overlay checks the new wrong-state regression against the original guard; it does not change the release binary. Both sides use byte-identical public-API benchmark sources. Source and binary hashes, commands and fixture fingerprints are recorded in the JSON files.

All measurements ran on build01, with four available CPUs and two build jobs. The five default-feature pairs alternate master/branch order, use disposable copies of the retained v3 aged fixtures, and run serially without an idle wait. Every pair completed on its first attempt: 20 successful runs, zero failures, retries or exclusions. Source fingerprints and the four benchmark binaries remained fixed. The ten `pair-*.host-load-*.txt` files contain actual `ssh home-server uptime` readings made from the PC. Per-phase runner readings are identified separately in the report.

Each history run samples 60 ordinary pages and one 2,000-block page, with five repeats for warm calls. Version-list and pre-root old-view digests must match. The sync benchmark runs its core export/import phase before HTTP first sync into a separate empty receiver, checks every received VV and runs the existing delta assertions. OS caches were not dropped. These are noisy relative measurements, without a stability or significance claim.

| Fresh ordinary-page operation | Master median | Branch median | Branch/master |
| --- | ---: | ---: | ---: |
| Open | 0.768 ms | 0.714 ms | 0.930 |
| Edit | 0.562 ms | 0.581 ms | 1.034 |
| History list | 0.755 ms | 2.370 ms | 3.139 |
| Old version | 2.950 ms | 4.702 ms | 1.594 |
| First HTTP sync, all 10,001 pages | 45.56 s | 58.13 s | 1.276 |

[The full report](cold-ab-5pairs.md) includes warm calls, the big page, export and wire bytes, individual pairs, paired ratios and all load readings. [The JSON report](cold-ab-5pairs.json), `runs.jsonl`, five `runner-*.json` files and [orchestration provenance](orchestrator-provenance.json) retain structured evidence. The report was rendered again after making its old v4 caveats conditional on the historical source path; all numerical fields remained identical.

The feature-enabled before/after profiles use fresh source caches and identical fixture fingerprints. HTTP verification (fork + compare, including the new head check after the change) falls from 8.197015 s to 0.497673 s; source-export verification falls from 10.978518 s to 0.538398 s. Both after runs compare all 10,001 reconstructed states and perform zero verification forks. The HTTP hub export span falls from 30.096212 s to 22.405128 s. [Profile summary](profile-summary.json) and four `branch-*-before/after.json` files retain all counters and hashes. Inclusive spans overlap; receiver archive work is included in some HTTP counters. These are single diagnostic runs, separate from the five uninstrumented pairs.

At candidate `036f4e9`, linux01 validation passed 600 tests across core, sync and hub, with six ignored and the coordinator-approved inherited journal-purge test filtered out. The feature-enabled prefix subset passed ten tests, including a valid sealed prefix with matching versions/frontiers but wrong state. That regression also passed against the original guard on build01. All eight two-writer integration tests passed. The explicit aged batch stress passed for 500 and 10,001 pages: lock holds 235.62 ms and 4,282.68 ms; total times 534.28 ms and 10,467.47 ms. These are measured holds, not an upper bound for other loads. The owned linux01 fixture directory and uploaded archive were removed.

Raw phase logs, fixtures, pinned binaries and build logs remain under `/home/builder/work/cold-storage-20261014`; small test logs remain under `/home/dev/lane-storage-*-20261014.log` on linux01. Windows runtime behavior was not tested. Sending cold prefixes as stored remains deferred.
