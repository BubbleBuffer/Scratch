# Search gate A/B harness

`ab_search_gate.py` compares a baseline and a candidate revision on the
release-only 10,000-page search gate (`warm_search_p95_under_20_ms` in
`crates/core/tests/search_perf.rs`). Standard library Python only. It runs on
Windows (Git Bash) and Linux.

The gate is report-only: `SEARCH_PERF_REPORT_ONLY=1` keeps every timing case
printed and skips the six timing asserts. Correctness asserts always run. The
harness sets the switch itself. A revision without the switch gets it applied
to its temporary copy only; no git revision is changed.

## Acceptance and diagnostics

- **Acceptance is the build01 workflow.** The `search-ab` GitHub workflow runs
  this harness on build01 with `baseline`, `candidate` and `pairs` inputs. Its
  verdict is the one that counts.
- **linux01 and local runs are diagnostic only.** They are for development and
  investigation. A result from them does not accept or reject a candidate, and
  it does not qualify hosted absolute latency or UI latency.

## Invocation

```
python crates/core/benches/ab_search_gate.py run --baseline <rev> --candidate <rev>
python crates/core/benches/ab_search_gate.py run --baseline <rev> --candidate <rev> --pairs 5 --jobs 2 --out-dir <dir>
python crates/core/benches/ab_search_gate.py patch <src search_perf.rs> <dst search_perf.rs>
python crates/core/benches/ab_search_gate.py selftest      # no Cargo, no benchmarks
python crates/core/benches/ab_search_gate.py run --help
```

- `run` is heavy. Run it on build01 (acceptance) or linux01 (diagnostic), not on the PC.
- The same-binary control is always on. There is no flag to turn it off.
- `--jobs` is passed to cargo as `-j`. Default `max(1, min(2, CPUs))`.
- `--cargo` names the cargo executable (default `cargo` on PATH).
- `--run-timeout` is seconds per round (default 900).
- Builds run one after the other in detached worktrees under a temporary
  directory (`--work-dir` to override). Each worktree is removed right after
  its build. Both builds share one `CARGO_TARGET_DIR`
  (default `target/search-gate-cargo`) and use `cargo test --release --locked`.

## Session: real pairs and control pairs

A session has `--pairs` real pairs and `--pairs` control pairs (default 5 each),
so 4 rounds per pair and `4 x pairs` rounds in total. Each pair is:

- a **real pair**: one `baseline` round and one `candidate` round, order AB, BA, ...
- a **control pair**: one `control-baseline` round and one `control-probe` round.
  Both run the baseline executable, the same saved file whose SHA-256 is in
  `result.json`. The order is the opposite of the real pair's.

Real and control pairs alternate in one session (real 1, control 1, real 2,
control 2, ...), so both families see the same ambient conditions. Each round
is one process with `SEARCH_PERF_REPORT_ONLY=1`, run with `--test-threads=1`.
Before every round the harness recomputes the SHA-256 of the executable it is
about to run, and it checks both executables again after the last round. A
changed file makes the gate INVALID.

## Validation

Before parsing, the harness strips libtest's inline `test <name> ... ` prefix
(and a slow-test notice) from output lines, so the first printed line still
counts. A round is rejected when it has a non-zero exit, a timeout, a panic, no
`test result: ok. 1 passed` line, not exactly one `word index` line, duplicate
labels, a case count other than the 47 manifest cases, a case out of order, a
sample count other than the manifest value, or a sample count that changed from
round 1. A side with fewer rounds than `--pairs` (any of the four sides) is
rejected. A revision is rejected when its test source no longer matches the
manifest. A baseline or candidate executable whose SHA-256 has changed is
rejected. A control ratio that cannot be computed (zero control-baseline p50) is
rejected. Any rejection makes the gate INVALID.

The manifest (cases, sample counts, query lists) is the one in `ab_search_gate.py`.
It is checked against the test source of each revision. The harness does not
change it.

## Verdict

The verdict uses p50 only. For each case:

- `candidate ratio` = median over the candidate rounds of p50 / median over the
  baseline rounds of p50.
- `control ratio` = median over the control-probe rounds of p50 / median over
  the control-baseline rounds of p50. Both sides are the baseline executable, so
  this is the noise of the same binary in this session.
- `control spread` = abs(control ratio - 1).
- `band` = max(0.05, control spread).
- A case fails only when `candidate ratio > 1 + band`. The boundary is inclusive.

The gate verdict is PASS only if no case fails. p95 ratios are printed for
information and do not decide the verdict. Median p50 and p95 per side, the
control spread and the band are printed for every case.

Exit codes: `0` PASS, `1` FAIL, `2` INVALID. The final stdout line is
`verdict: PASS (0 cases beyond band)`, `verdict: FAIL (n cases beyond band)`,
or `verdict: INVALID`.

## Machine and load

- `summary.md` and `result.json` record the machine once: hostname, OS, CPU,
  physical and logical cores, Python and cargo versions.
- Every round records wall time, machine CPU utilization over the round
  (`cpu_utilization_pct`), and load average before and after. Windows has no
  load average and reports it as unavailable.
- The utilization and wall time cover the whole round process, including the
  untimed corpus setup inside the test.

## Artifacts (`--out-dir`, default `target/search-gate/<UTC stamp>/`)

- `summary.md`: verdict, revisions, control and band rule, machine, per-case
  table (medians, candidate and p95 ratios, control ratio, spread, band,
  verdict), per-run load, cleanup problems.
- `result.json`: everything above in structured form, including each round's
  parsed cases, its `family` (`gate` or `control`), `side`, `binary` and
  `binary_sha256`. It is rewritten after every step. `verdict` stays `INVALID`
  until the gate completes.
- `raw/pair-NN-<AB|BA>-<side>.txt`, with `<side>` one of `baseline`,
  `candidate`, `control-baseline`, `control-probe`: exact command, stdout and
  stderr of each round. These are the source of truth.
- `bin/<side>-<sha8>.exe` (or no suffix on Linux): each revision's test binary,
  with its SHA-256 in `result.json`. The control rounds use the `baseline`
  file, not a copy.
- `build/<side>.stdout.json` and `.stderr.txt`: cargo output for each build.

`--out-dir` inside the repository must be under `target/`, which git ignores.
Raw outputs must not be committed. `out-dir` and `target` are never deleted.

## Running on linux01 (diagnostic)

The steps below run from Git Bash on the PC, or in a shell on linux01. Both
revisions must resolve in the clone, so use a full clone (not `--depth`). The
candidate must also be on origin, so push the candidate branch first.

```
/c/Windows/System32/OpenSSH/ssh.exe -o BatchMode=yes linux01 'git clone https://github.com/BubbleBuffer/notes-app.git /home/dev/work/search-ab'
/c/Windows/System32/OpenSSH/ssh.exe -o BatchMode=yes linux01 'cd /home/dev/work/search-ab && git fetch origin <candidate-branch> && git checkout --detach origin/<candidate-branch> && python3 crates/core/benches/ab_search_gate.py selftest'
/c/Windows/System32/OpenSSH/ssh.exe -o BatchMode=yes linux01 'mkdir -p /home/dev/work/search-ab-worktrees && ln -sfn /home/dev/work/BubblePaw-base-agent /home/dev/work/search-ab-worktrees/BubblePaw-base-agent && test -d /home/dev/work/BubblePaw-base-agent && ulimit -n 65536 && cd /home/dev/work/search-ab && . /home/dev/.cargo/env && python3 crates/core/benches/ab_search_gate.py run --baseline <baseline-rev> --candidate <candidate-rev> --pairs 5 --jobs 2 --work-dir /home/dev/work/search-ab-worktrees --target-dir /home/dev/work/.target/search-ab --out-dir /home/dev/work/search-ab-results/<name>'
```

Worktree root and sibling dependencies. The clone must sit directly under
`/home/dev/work`, next to the sibling `/home/dev/work/BubblePaw-base-agent`.
The workspace's path dependencies are relative (for example
`crates/agent/Cargo.toml`: `../../../BubblePaw-base-agent/...`). Cargo reads
every workspace member's path dependencies to resolve the lockfile, so a
missing sibling fails the build even with `-p vault_core`.

- Each revision's worktree is `<work-dir>/<label>-<sha8>`. From there the
  relative path reaches `<work-dir>/BubblePaw-base-agent`, not
  `/home/dev/work/BubblePaw-base-agent`. The default temporary root
  (`/tmp/search-gate-wt-*`) would look for `<temp-root>/BubblePaw-base-agent`,
  which the harness never creates. So the run passes `--work-dir`.
- The setup line creates the worktree parent and a symlink to the existing
  sibling inside it. A symlink, not a copy, keeps one shared checkout of the
  sibling. The harness never deletes the sibling: with `--work-dir` it does not
  remove the root, and `git worktree remove` touches only each worktree.
- `ulimit -n 65536` raises the open-file limit for the run. The baseline round
  failed during its untimed corpus setup with "Too many open files" under the
  default limit. This changes the environment, not the workload.
- The private clone needs GitHub authentication; a transient credential helper
  can supply credentials for clone/fetch without saving them on linux01.
- `linux01-run` ships only the working tree (tracked and untracked files, no
  `.git`). It can run the selftest, but the gate resolves revisions from git
  history, so use the full clone above for a real run.

## Selftest

`selftest` runs the manifest, parser, validator, same-binary control band
(boundaries, spread, floor), informational p95 behaviour, the session plan,
the binary SHA-256 check, the CLI guard against turning the control off, the
summary and final-line rendering, jobs cap, overlap and out-dir guards, and patch
checks. It needs no Cargo or benchmarks. The patch input is the historical
unpatched revision `4f978b9` (`--unpatched-rev` to change it), never HEAD. HEAD
already has the switch, so it cannot test the patch. The working-tree
`search_perf.rs` is checked against the manifest. It is compared byte for byte
with the patch output only when it carries the switch.

The historical-revision checks need `--unpatched-rev` to resolve in `--repo`.
Without git history they fail, and the selftest reports that check as FAIL.

## Known limits

- Dependencies are part of the comparison. Between `4f978b9` and HEAD,
  `Cargo.lock` gained 41 lines and `crates/sync/Cargo.toml` gained 2.
- Each p95 comes from 50 samples per round, so p95 ratios are noisier than p50
  ratios. p95 is informational, so this noise does not decide the verdict.
- The control measures the same binary against itself in one session. It
  estimates the noise of this machine right now; it does not prove that the
  noise is the same on another run or host.
- Windows has no load average. Load comparisons are only available on Linux.
- A run takes `4 x pairs` rounds, about twice the rounds of the earlier gate
  (which had `2 x pairs`).
- Earlier record, before the control band (rule: both median p50 and median p95
  ratios at most 1.05, no control): on linux01, `4f978b9` versus `333d1ea` with
  five AB/BA pairs gave **FAIL**, with fourteen warm cases over 1.05 in median
  p50. See the [superseded diagnostic record](../../../wiki/architecture/search-queries.md#interleaved-ab-acceptance-2026-10-13).
  That result does not apply to the current rule.
