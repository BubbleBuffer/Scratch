#!/usr/bin/env python3
"""A/B search gate for the release-only 10,000-page search performance test.

See README.md next to this file for invocation and artifact semantics.

Subcommands:
  run       build baseline, then candidate, sequentially; run the report-only A/B rounds.
            Every session also runs same-binary control pairs (the baseline executable
            against itself), interleaved with the real pairs. The control is always on.
  patch     copy a test file, adding the report-only switch when it is absent
  selftest  check manifest, parser, validator, control band, summary, patch and rendering
            (no Cargo, no benchmarks)

Standard library only. Works on Windows and Linux.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import hashlib
import io
import json
import os
import platform
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PACKAGE = "vault_core"
TEST_TARGET = "search_perf"
TEST_FILE = "crates/core/tests/search_perf.rs"
TEST_NAME = "warm_search_p95_under_20_ms"
SWITCH = "SEARCH_PERF_REPORT_ONLY"
BAND_FLOOR = 0.05  # minimum noise band on the median p50 ratio; the control spread can widen it
DEFAULT_PAIRS = 5
MIN_PAIRS = 5
JOBS_CAP = 2  # default cargo -j cap, so the default build stays modest on the Linux machine
RUN_TIMEOUT_S = 900
EXPECTED_CASE_COUNT = 47
EXIT_PASS, EXIT_FAIL, EXIT_INVALID = 0, 1, 2
HISTORICAL_BASELINE = "4f978b9"  # last commit before the report-only switch; selftest's patch input

# ---------------------------------------------------------------- gate manifest
# Mirrors the printed cases of warm_search_p95_under_20_ms in search_perf.rs.
# check_source_manifest() compares these lists with the test file, so a drift
# in either place invalidates the gate instead of silently changing the set.

WARM_QUERIES = [
    "p", "pr", "pro", "proj", "project", "g", "ga", "gar", "gard", "garden",
    "a", "t", "tomato", "weekly review", "coffee mor", "zür", "café", "big page", "xylophone",
]
TYPO_QUERIES = [
    "tomatoe", "gardne", "weekyl review", "cofee", "cofee mor", "projcet", "recipies",
    "gadr", "meeitng", "buget", "desgin", "journla", "databsae",
]
# (name, query) as printed; the deep-subtree id is normalized to <ID>.
FILTER_CASES = [
    ("filter only", "tag:perf/selective"),
    ("selective filter plus text", "project tag:perf/selective"),
    ("broad filter plus broad text", "p tag:perf/broad"),
    ("negatives only", "-tag:perf/excluded"),
    ("deep in", "in:[[<ID>]]"),
    ("tag on 10k", "tag:perf/all"),
]
TYPED_PREFIXES = ["p", "pr", "pro", "proj", "project"]
FILTERED_TYPO = "gardne tag:perf/broad"
SAMPLES = 50
UNCACHED_SAMPLES = 100


class GateError(Exception):
    """Setup, patch, build, source or validation failure that invalidates the gate."""


def gate_manifest() -> list[tuple[str, int]]:
    """The 47 printed cases in print order, each with its expected sample count."""
    cases = [("warm search, 10,000 pages, limit 50", len(WARM_QUERIES) * SAMPLES)]
    cases += [(f'query "{q}"', SAMPLES) for q in WARM_QUERIES]
    cases += [("fuzzy search, 10,000 pages, limit 50", len(TYPO_QUERIES) * SAMPLES)]
    cases += [(f'typo query "{q}"', SAMPLES) for q in TYPO_QUERIES]
    cases += [("uncached tag candidate construction, 100 distinct filters, 10,000 pages", UNCACHED_SAMPLES)]
    cases += [(f'filtered {name}, "{query}", 10,000 pages', SAMPLES) for name, query in FILTER_CASES]
    cases += [(f'typed prefix "{p}", constant tag:perf/broad, 10,000 pages', SAMPLES) for p in TYPED_PREFIXES]
    cases += [(f'filtered typo "{FILTERED_TYPO}", 10,000 pages', SAMPLES)]
    if len(cases) != EXPECTED_CASE_COUNT:
        raise GateError(f"manifest has {len(cases)} cases, expected {EXPECTED_CASE_COUNT}")
    if len({label for label, _ in cases}) != len(cases):
        raise GateError("manifest has duplicate labels")
    return cases


STR_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')


def check_source_manifest(text: str) -> list[str]:
    """Return problems where the test source no longer matches the manifest lists."""
    problems = []
    for name, expected in (("QUERIES", WARM_QUERIES), ("TYPO_QUERIES", TYPO_QUERIES)):
        match = re.search(rf"const {name}: &\[&str\] = &\[(.*?)\];", text, re.S)
        if not match:
            problems.append(f"{name} array not found")
            continue
        found = STR_RE.findall(match.group(1))
        if found != expected:
            problems.append(f"{name} differs from manifest ({len(found)} vs {len(expected)} entries)")
    for name, query in FILTER_CASES:
        if f'"{name}"' not in text:
            problems.append(f"filter case name {name!r} not in source")
        if query.startswith("in:"):
            if 'format!("in:[[{}]]"' not in text:
                problems.append("deep-subtree case format not in source")
        elif f'"{query}"' not in text:
            problems.append(f"filter query {query!r} not in source")
    prefixes = ", ".join(f'"{p}"' for p in TYPED_PREFIXES)
    if f"let prefixes = [{prefixes}];" not in text:
        problems.append("typed prefix list differs from manifest")
    if f'let typo = "{FILTERED_TYPO}";' not in text:
        problems.append("filtered typo differs from manifest")
    return problems


# ---------------------------------------------------------------- patch


HELPER_ANCHOR = '#[test]\n#[ignore = "release-only 10,000-page search performance gate"]\n'
HELPER = (
    "/// `SEARCH_PERF_REPORT_ONLY=1` keeps printing every timing case but skips the\n"
    "/// timing asserts. Correctness asserts always run.\n"
    "fn report_only() -> bool {\n"
    '    std::env::var("SEARCH_PERF_REPORT_ONLY").as_deref() == Ok("1")\n'
    "}\n\n"
)
# The six timing asserts. Each old snippet must occur exactly once in a file
# without the switch. Only the condition changes; messages stay as they are.
TIMING_PATCHES = [
    (
        '        overall < BUDGET && slow.is_empty(),\n        "search exceeded 20 ms',
        '        report_only() || (overall < BUDGET && slow.is_empty()),\n        "search exceeded 20 ms',
    ),
    (
        '        overall < FUZZY_BUDGET && slow.is_empty(),\n        "fuzzy search exceeded 30 ms',
        '        report_only() || (overall < FUZZY_BUDGET && slow.is_empty()),\n        "fuzzy search exceeded 30 ms',
    ),
    (
        '        p95 < BUDGET,\n        "uncached tag',
        '        report_only() || p95 < BUDGET,\n        "uncached tag',
    ),
    (
        '        slow.is_empty(),\n        "filtered search exceeded 20 ms',
        '        report_only() || slow.is_empty(),\n        "filtered search exceeded 20 ms',
    ),
    (
        '        slow.is_empty(),\n        "filtered typed prefixes exceeded 20 ms',
        '        report_only() || slow.is_empty(),\n        "filtered typed prefixes exceeded 20 ms',
    ),
    (
        '        p95 < FUZZY_BUDGET,\n        "filtered typo exceeded 30 ms',
        '        report_only() || p95 < FUZZY_BUDGET,\n        "filtered typo exceeded 30 ms',
    ),
]
REPORT_ONLY_CALL = "report_only() ||"


def patch_text(text: str) -> tuple[str, bool]:
    """Return (text, applied). Adds the switch and gates the six timing asserts.

    Leaves the text unchanged when the switch already exists, after checking that
    all six conditions are gated. Keeps CRLF line endings if the input has them.
    """
    crlf = "\r\n" in text
    src = text.replace("\r\n", "\n") if crlf else text
    if SWITCH in src:
        gated = src.count(REPORT_ONLY_CALL)
        if gated != len(TIMING_PATCHES):
            raise GateError(
                f"{SWITCH} present but {gated} of {len(TIMING_PATCHES)} timing conditions are gated"
            )
        return text, False
    anchors = src.count(HELPER_ANCHOR)
    if anchors != 1:
        raise GateError(f"test anchor found {anchors} times, expected 1")
    out = src.replace(HELPER_ANCHOR, HELPER + HELPER_ANCHOR, 1)
    for old, new in TIMING_PATCHES:
        found = out.count(old)
        if found != 1:
            raise GateError(
                f"timing condition found {found} times, expected 1: {old.splitlines()[0]!r}"
            )
        out = out.replace(old, new, 1)
    if crlf:
        out = out.replace("\n", "\r\n")
    return out, True


def patch_file(src: Path, dst: Path) -> bool:
    """Copy src to dst, patching when the switch is absent. Returns applied."""
    text = src.read_bytes().decode("utf-8")
    out, applied = patch_text(text)
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_bytes(out.encode("utf-8"))
    return applied


# ---------------------------------------------------------------- parse


DEEP_ID_RE = re.compile(r"in:\[\[[^\]\s]*\]\]")
CASE_RE = re.compile(
    r"^(?P<label>.+): p50 (?P<p50>\d+\.\d+) ms, p95 (?P<p95>\d+\.\d+) ms \(n=(?P<n>\d+),"
)
RESULT_OK_RE = re.compile(r"^test result: ok\. 1 passed; 0 failed")
# libtest with --nocapture writes "test <name> ... " before the test's first line of output
# and can put its slow-test notice inline too. Strip both so the printed lines parse.
INLINE_PREFIX_RE = re.compile(rf"^test {re.escape(TEST_NAME)} \.\.\.\s*")
SLOW_NOTICE_RE = re.compile(rf"test {re.escape(TEST_NAME)} has been running for over \d+ seconds")


def normalize_label(label: str) -> str:
    return DEEP_ID_RE.sub("in:[[<ID>]]", label)


def parse_output(text: str) -> dict:
    """Extract every printed timing case, the info line and failure markers."""
    cases, info, panics = [], [], []
    test_passed = False
    for raw in text.splitlines():
        line = SLOW_NOTICE_RE.sub("", raw.rstrip())
        line = INLINE_PREFIX_RE.sub("", line, count=1).rstrip()
        match = CASE_RE.match(line)
        if match:
            cases.append(
                {
                    "label": normalize_label(match["label"]),
                    "p50_ms": float(match["p50"]),
                    "p95_ms": float(match["p95"]),
                    "n": int(match["n"]),
                }
            )
            continue
        if line.startswith("word index,"):
            info.append(normalize_label(line))
        if "panicked at" in line or "test result: FAILED" in line:
            panics.append(line.strip())
        if RESULT_OK_RE.match(line):
            test_passed = True
    return {"cases": cases, "info": info, "panics": panics, "test_passed": test_passed}


# ---------------------------------------------------------------- validate


def validate_round(run: dict, expected: list[tuple[str, int]]) -> list[str]:
    """Check one round against expected (label, n) pairs in print order."""
    errors = []
    if run["exit_code"] != 0:
        errors.append(f"exit code {run['exit_code']}")
    if run["timed_out"]:
        errors.append("timed out")
    if run["panics"]:
        errors.append("panic or failure line: " + run["panics"][0])
    if not run["test_passed"]:
        errors.append("no 'test result: ok. 1 passed; 0 failed' line")
    if len(run["info"]) != 1:
        errors.append(f"expected one 'word index' line, found {len(run['info'])}")
    cases = run["cases"]
    labels = [c["label"] for c in cases]
    dupes = sorted({x for x in labels if labels.count(x) > 1})
    if dupes:
        errors.append(f"duplicate labels: {dupes[:3]}")
    exp_labels = [label for label, _ in expected]
    if len(cases) != len(expected):
        errors.append(
            f"printed {len(cases)} cases, expected {len(expected)} (truncated or extra output)"
        )
    elif labels != exp_labels:
        index = next(i for i, (a, b) in enumerate(zip(labels, exp_labels)) if a != b)
        errors.append(f"case {index + 1} is {labels[index]!r}, expected {exp_labels[index]!r}")
    else:
        for case, (label, n) in zip(cases, expected):
            if case["n"] != n:
                errors.append(f"{label!r}: n={case['n']}, expected {n}")
    for case in cases:
        if case["p95_ms"] < case["p50_ms"]:
            errors.append(f"malformed case line (p95 < p50): {case['label']!r}")
            break
    return errors


def check_round_consistency(reference: list[dict], run: dict) -> list[str]:
    """Every label must keep the sample count it had in the first round."""
    ref = {c["label"]: c["n"] for c in reference}
    return [
        f"{c['label']!r}: n={c['n']} changed from round 1 n={ref[c['label']]}"
        for c in run["cases"]
        if c["label"] in ref and ref[c["label"]] != c["n"]
    ]


def check_side_rounds(rounds: list[dict], pairs: int, side: str) -> list[str]:
    if len(rounds) != pairs:
        return [f"{side}: {len(rounds)} rounds recorded, expected {pairs} (missing rounds)"]
    return []


def _ratio(candidate_ms: float, baseline_ms: float) -> float | None:
    return candidate_ms / baseline_ms if baseline_ms > 0 else None


def summarize(
    baseline: list[dict], candidate: list[dict], control_baseline: list[dict], control_probe: list[dict]
) -> dict:
    """Per case: the candidate's median-round p50 ratio against the control band, and the verdict.

    Real pairs: candidate median p50 / baseline median p50 (the verdict ratio).
    Control pairs: the baseline binary run against itself. Its probe median p50 / its
    control-baseline median p50 gives the spread abs(ratio - 1), so the band is
    max(BAND_FLOOR, spread). A case fails only when its candidate p50 ratio > 1 + band.
    p95 ratios are printed for information and do not decide the verdict.
    A control ratio that cannot be computed makes the run invalid (GateError).
    """
    labels = [c["label"] for c in baseline[0]["cases"]]
    rows = []
    for index, label in enumerate(labels):
        med_b50 = statistics.median(r["cases"][index]["p50_ms"] for r in baseline)
        med_b95 = statistics.median(r["cases"][index]["p95_ms"] for r in baseline)
        med_c50 = statistics.median(r["cases"][index]["p50_ms"] for r in candidate)
        med_c95 = statistics.median(r["cases"][index]["p95_ms"] for r in candidate)
        med_k_ref50 = statistics.median(r["cases"][index]["p50_ms"] for r in control_baseline)
        med_k_ref95 = statistics.median(r["cases"][index]["p95_ms"] for r in control_baseline)
        med_k50 = statistics.median(r["cases"][index]["p50_ms"] for r in control_probe)
        med_k95 = statistics.median(r["cases"][index]["p95_ms"] for r in control_probe)
        r50 = _ratio(med_c50, med_b50)
        r95 = _ratio(med_c95, med_b95)
        control50 = _ratio(med_k50, med_k_ref50)
        if control50 is None:
            raise GateError(f"control p50 ratio is undefined for {label!r} (zero control-baseline p50)")
        spread = abs(control50 - 1.0)
        band = max(BAND_FLOOR, spread)
        beyond = r50 is None or r50 > 1.0 + band
        rows.append(
            {
                "label": label,
                "baseline_p50_ms": med_b50,
                "baseline_p95_ms": med_b95,
                "candidate_p50_ms": med_c50,
                "candidate_p95_ms": med_c95,
                "p50_ratio": r50,
                "p95_ratio": r95,
                "control_baseline_p50_ms": med_k_ref50,
                "control_baseline_p95_ms": med_k_ref95,
                "control_probe_p50_ms": med_k50,
                "control_probe_p95_ms": med_k95,
                "control_p50_ratio": control50,
                "control_spread": spread,
                "band": band,
                "verdict": "fail" if beyond else "pass",
            }
        )
    beyond_count = sum(1 for r in rows if r["verdict"] == "fail")
    overall = "PASS" if rows and beyond_count == 0 else "FAIL"
    return {
        "rows": rows,
        "verdict": overall,
        "band_floor": BAND_FLOOR,
        "cases_beyond_band": beyond_count,
    }


def verdict_line(result: dict) -> str:
    """The final line printed by run: PASS or FAIL with the count of cases beyond their band."""
    if result.get("verdict") not in ("PASS", "FAIL") or not result.get("summary"):
        return "verdict: INVALID"
    return f"verdict: {result['verdict']} ({result['summary']['cases_beyond_band']} cases beyond band)"


def format_case_table(summary: dict) -> list[str]:
    """Markdown table of per-case medians, ratios, control spread, band and verdict."""
    def ratio(value: float | None) -> str:
        return "n/a" if value is None else f"{value:.3f}"

    lines = [
        "| Case | Base p50 | Cand p50 | p50 ratio | Base p95 | Cand p95 | p95 ratio (info) "
        "| Control p50 ratio | Spread | Band | Verdict |",
        "|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    for row in summary["rows"]:
        lines.append(
            f"| {row['label']} | {row['baseline_p50_ms']:.3f} | {row['candidate_p50_ms']:.3f} | "
            f"{ratio(row['p50_ratio'])} | {row['baseline_p95_ms']:.3f} | {row['candidate_p95_ms']:.3f} | "
            f"{ratio(row['p95_ratio'])} | {ratio(row['control_p50_ratio'])} | "
            f"{row['control_spread']:.3f} | {row['band']:.3f} | {row['verdict']} |"
        )
    return lines


def order_for_pair(pair: int) -> str:
    """Pair 1 is AB (first side first), pair 2 is BA, and so on."""
    return "AB" if pair % 2 == 1 else "BA"


GATE_SIDES = ("baseline", "candidate")
CONTROL_SIDES = ("control-baseline", "control-probe")  # both run the baseline executable
ALL_SIDES = GATE_SIDES + CONTROL_SIDES
SIDE_BINARY = {
    "baseline": "baseline",
    "candidate": "candidate",
    "control-baseline": "baseline",
    "control-probe": "baseline",
}


def session_plan(pairs: int) -> list[dict]:
    """The rounds of one session, in run order.

    Per pair p: a real pair (baseline and candidate, order by parity) then a control pair
    (control-baseline and control-probe, both the baseline executable, order opposite to the
    real pair). Each side gets exactly `pairs` rounds.
    """
    plan = []
    for pair in range(1, pairs + 1):
        for family, sides in (("gate", GATE_SIDES), ("control", CONTROL_SIDES)):
            order = order_for_pair(pair) if family == "gate" else order_for_pair(pair + 1)
            ordered = list(sides) if order == "AB" else list(reversed(sides))
            for side in ordered:
                plan.append(
                    {"pair": pair, "family": family, "order": order, "side": side, "binary": SIDE_BINARY[side]}
                )
    return plan


def verify_binary(path: Path, expected_sha256: str) -> str:
    """Check the saved executable still has the SHA-256 recorded at build time. Raises GateError."""
    actual = sha256_file(path)
    if actual != expected_sha256:
        raise GateError(
            f"binary changed on disk: {path.name} sha256 {actual[:12]} != recorded {expected_sha256[:12]}"
        )
    return actual


# ---------------------------------------------------------------- machine and load


def cpu_times() -> tuple[int, int] | None:
    """Cumulative (busy, total) for the whole machine. Utilization = delta busy / delta total."""
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        class FILETIME(ctypes.Structure):
            _fields_ = [("lo", wintypes.DWORD), ("hi", wintypes.DWORD)]

        idle, kernel, user = FILETIME(), FILETIME(), FILETIME()
        ok = ctypes.windll.kernel32.GetSystemTimes(
            ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user)
        )
        if not ok:
            return None

        def ft(f: FILETIME) -> int:
            return (f.hi << 32) | f.lo

        # Kernel time already includes idle time.
        total = ft(kernel) + ft(user)
        return total - ft(idle), total
    try:
        fields = Path("/proc/stat").read_text().splitlines()[0].split()[1:]
        vals = [int(x) for x in fields[:8]]
    except (OSError, ValueError, IndexError):
        return None
    total = sum(vals)
    return total - (vals[3] + vals[4]), total


def utilization_pct(before: tuple[int, int] | None, after: tuple[int, int] | None) -> float | None:
    if before is None or after is None or after[1] <= before[1]:
        return None
    return round(100.0 * (after[0] - before[0]) / (after[1] - before[1]), 2)


def load_average() -> object:
    if os.name == "nt" or not hasattr(os, "getloadavg"):
        return "unavailable (Windows has no load average)"
    return [round(x, 2) for x in os.getloadavg()]


def default_jobs() -> int:
    return max(1, min(JOBS_CAP, os.cpu_count() or 1))


def _windows_cpu_info() -> tuple[str | None, int | None]:
    script = (
        "$p = Get-CimInstance Win32_Processor; $p.Name; "
        "($p | Measure-Object -Property NumberOfCores -Sum).Sum"
    )
    try:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
            capture_output=True, text=True, timeout=30,
        ).stdout.split()
    except (OSError, subprocess.TimeoutExpired):
        return None, None
    name = " ".join(out[:-1]) if len(out) > 1 else None
    cores = int(out[-1]) if out and out[-1].isdigit() else None
    return name, cores


def machine_identity() -> dict:
    cpu_name = os.environ.get("PROCESSOR_IDENTIFIER") or platform.processor() or None
    physical = None
    if os.name == "nt":
        ps_name, physical = _windows_cpu_info()
        cpu_name = ps_name or cpu_name
    else:
        try:
            pairs = set()
            phys = core = None
            for line in Path("/proc/cpuinfo").read_text().splitlines() + [""]:
                if line.startswith("physical id"):
                    phys = line.split(":")[1].strip()
                elif line.startswith("core id"):
                    core = line.split(":")[1].strip()
                elif not line.strip() and phys is not None and core is not None:
                    pairs.add((phys, core))
                    phys = core = None
            physical = len(pairs) or None
            for line in Path("/proc/cpuinfo").read_text().splitlines():
                if line.startswith("model name"):
                    cpu_name = line.split(":", 1)[1].strip()
                    break
        except OSError:
            pass
    cargo = None
    try:
        cargo = subprocess.run(
            ["cargo", "--version"], capture_output=True, text=True, timeout=60
        ).stdout.strip() or None
    except (OSError, subprocess.TimeoutExpired):
        pass
    return {
        "hostname": socket.gethostname(),
        "os": platform.platform(),
        "machine": platform.machine(),
        "cpu": cpu_name,
        "physical_cores": physical,
        "logical_cpus": os.cpu_count(),
        "python": platform.python_version(),
        "cargo": cargo,
    }


# ---------------------------------------------------------------- git and build


def git(repo: Path, *args: str, check: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=check
    )


def resolve_rev(repo: Path, rev: str) -> str:
    proc = git(repo, "rev-parse", "--verify", "--quiet", f"{rev}^{{commit}}")
    if proc.returncode != 0 or not proc.stdout.strip():
        raise GateError(f"revision does not resolve to a commit: {rev}")
    return proc.stdout.strip()


def find_test_executable(json_stdout: str) -> str:
    found = set()
    for line in json_stdout.splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if (
            msg.get("reason") == "compiler-artifact"
            and msg.get("profile", {}).get("test")
            and msg.get("target", {}).get("name") == TEST_TARGET
            and msg.get("executable")
        ):
            found.add(msg["executable"])
    if len(found) != 1:
        raise GateError(f"expected one {TEST_TARGET} test executable, found {sorted(found)}")
    return found.pop()


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def is_within(child: Path, parent: Path) -> bool:
    try:
        child.resolve().relative_to(parent.resolve())
        return True
    except ValueError:
        return False


def overlaps(a: Path, b: Path) -> bool:
    return is_within(a, b) or is_within(b, a)


def build_revision(
    label: str,
    args,
    repo: Path,
    out: Path,
    target: Path,
    work_root: Path,
    cleanup_errors: list[str],
    record: dict,
) -> None:
    """Check out record["sha"] in a detached worktree, patch and check source, build, keep the binary.

    Fills `record` in place, so a failed build still keeps what it did and its cleanup state.
    Only this revision's worktree is removed, with `git worktree remove` for this path.
    """
    sha = record["sha"]
    work = work_root / f"{label}-{sha[:8]}"
    record.update(
        {
            "patched": None,
            "source_manifest_ok": None,
            "binary": None,
            "binary_sha256": None,
            "worktree_removed": None,
        }
    )
    if work.exists():
        raise GateError(f"worktree path already exists: {work}")
    added = git(repo, "worktree", "add", "--detach", str(work), sha)
    if added.returncode != 0:
        raise GateError(f"git worktree add failed for {label}: {added.stderr.strip()}")
    try:
        test_path = work / TEST_FILE
        if not test_path.is_file():
            raise GateError(f"{TEST_FILE} missing at {sha[:8]}")
        record["patched"] = patch_file(test_path, test_path)
        problems = check_source_manifest(test_path.read_bytes().decode("utf-8"))
        record["source_manifest_ok"] = not problems
        if problems:
            raise GateError(f"{label}: source does not match the manifest: " + "; ".join(problems[:3]))
        logs = out / "build"
        logs.mkdir(parents=True, exist_ok=True)
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(target)
        env["CARGO_INCREMENTAL"] = "0"
        cmd = [
            args.cargo, "test", "--release", "--locked", "-p", PACKAGE, "--test", TEST_TARGET,
            "--no-run", "--message-format=json", "-j", str(args.jobs),
        ]
        proc = subprocess.run(
            cmd, cwd=work, env=env, capture_output=True, text=True,
            encoding="utf-8", errors="replace",
        )
        (logs / f"{label}.stdout.json").write_text(proc.stdout, encoding="utf-8")
        (logs / f"{label}.stderr.txt").write_text(proc.stderr, encoding="utf-8")
        if proc.returncode != 0:
            raise GateError(f"cargo build failed for {label} (see build/{label}.stderr.txt)")
        exe_src = Path(find_test_executable(proc.stdout))
        bins = out / "bin"
        bins.mkdir(parents=True, exist_ok=True)
        exe_dst = bins / f"{label}-{sha[:8]}{exe_src.suffix}"
        shutil.copy2(exe_src, exe_dst)
        record["binary"] = exe_dst.relative_to(out).as_posix()
        record["binary_sha256"] = sha256_file(exe_dst)
    finally:
        removed = git(repo, "worktree", "remove", "--force", str(work))
        record["worktree_removed"] = removed.returncode == 0
        if not record["worktree_removed"]:
            cleanup_errors.append(
                f"git worktree remove failed for {work.name}: {removed.stderr.strip()}; directory deleted"
            )
            shutil.rmtree(work, ignore_errors=True)
            git(repo, "worktree", "prune")  # drop the admin entry for the deleted directory


# ---------------------------------------------------------------- rounds


def run_round(exe: Path, side: str, pair: int, order: str, out: Path, timeout: int) -> dict:
    cmd = [str(exe), "--ignored", "--exact", TEST_NAME, "--nocapture", "--test-threads=1"]
    env = os.environ.copy()
    env[SWITCH] = "1"
    cpu0, load0 = cpu_times(), load_average()
    start = time.monotonic()
    timed_out = False
    try:
        proc = subprocess.run(
            cmd, cwd=out, env=env, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=timeout,
        )
        stdout, stderr, code = proc.stdout, proc.stderr, proc.returncode
    except subprocess.TimeoutExpired as exc:
        def text_of(v) -> str:
            return v.decode("utf-8", "replace") if isinstance(v, bytes) else (v or "")

        stdout, stderr, code, timed_out = text_of(exc.stdout), text_of(exc.stderr), None, True
    wall = time.monotonic() - start
    cpu1, load1 = cpu_times(), load_average()
    raw_name = f"pair-{pair:02d}-{order}-{side}.txt"
    raw_dir = out / "raw"
    raw_dir.mkdir(parents=True, exist_ok=True)
    (raw_dir / raw_name).write_text(
        f"$ {' '.join(cmd)}\n{SWITCH}=1\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        encoding="utf-8",
    )
    parsed = parse_output(stdout + "\n" + stderr)
    return {
        "pair": pair,
        "order": order,
        "side": side,
        "raw": f"raw/{raw_name}",
        "exit_code": code,
        "timed_out": timed_out,
        "wall_s": round(wall, 2),
        "load_average_before": load0,
        "load_average_after": load1,
        "cpu_utilization_pct": utilization_pct(cpu0, cpu1),
        **parsed,
    }


def persist(out: Path, result: dict) -> None:
    (out / "result.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False, default=str), encoding="utf-8"
    )


def _show(d: dict, key: str) -> str:
    value = d.get(key)
    return "not set" if value in (None, "") else str(value)


def render_markdown(result: dict) -> str:
    """Render summary.md. Safe when revisions, machine or rounds were never set."""
    base = result.get("baseline") or {}
    cand = result.get("candidate") or {}
    base_sha = _show(base, "sha")
    cand_sha = _show(cand, "sha")
    lines = [f"# Search gate A/B: {result.get('verdict', 'INVALID')}", ""]
    if result.get("error"):
        lines += [f"Invalid: {result['error']}", ""]
    lines += [
        f"- Baseline: `{base_sha}` (patched: {_show(base, 'patched')}, "
        f"source manifest ok: {_show(base, 'source_manifest_ok')})",
        f"- Candidate: `{cand_sha}` (patched: {_show(cand, 'patched')}, "
        f"source manifest ok: {_show(cand, 'source_manifest_ok')})",
        f"- Pairs: {_show(result, 'pairs')} real (baseline vs candidate) and "
        f"{_show(result, 'pairs')} control (baseline vs baseline), interleaved, AB/BA alternating, "
        f"report-only ({SWITCH}=1)",
        f"- Control: the saved baseline executable, SHA-256 "
        f"`{_show(base, 'binary_sha256')}`, checked before every round",
        f"- Cases: {EXPECTED_CASE_COUNT} printed cases per round, checked against the manifest",
        f"- Cargo flags: {', '.join(result.get('cargo_flags', [])) or 'not set'}",
        f"- Verdict rule: per case, the candidate median-round p50 ratio must be <= 1 + band, "
        f"where band = max({BAND_FLOOR}, |control median-round p50 ratio - 1|). "
        "p95 ratios are informational only.",
        "",
        "## Machine",
        "",
    ]
    machine = result.get("machine") or {}
    lines += [f"- {key}: {value}" for key, value in machine.items()] or ["- not recorded"]
    lines.append("")
    summary = result.get("summary")
    if summary:
        lines += [
            "## Cases (median of round values, ms)",
            "",
            f"Band = max({BAND_FLOOR}, control spread). A case fails when its p50 ratio exceeds 1 + band. "
            f"Cases beyond band: {summary.get('cases_beyond_band', 'n/a')}.",
            "",
        ]
        lines += format_case_table(summary)
        lines.append("")
    rounds = result.get("rounds") or []
    lines += [
        "## Per-run load",
        "",
        "Each row covers the whole round process, including its untimed corpus setup.",
        "",
        "| Pair | Family | Order | Side | Binary | Wall s | CPU util % | Load avg before | Load avg after | Exit |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for run in rounds:
        lines.append(
            f"| {run.get('pair')} | {run.get('family')} | {run.get('order')} | {run.get('side')} | "
            f"{run.get('binary')} | {run.get('wall_s')} | "
            f"{run.get('cpu_utilization_pct')} | {run.get('load_average_before')} | "
            f"{run.get('load_average_after')} | {run.get('exit_code')} |"
        )
    if result.get("cleanup_errors"):
        lines += ["", "## Cleanup problems", ""] + [f"- {e}" for e in result["cleanup_errors"]]
    lines += ["", "Raw outputs: `raw/`. Structured result: `result.json`.", ""]
    return "\n".join(lines)


# ---------------------------------------------------------------- commands


def run_gate(args, repo: Path, out: Path, target: Path, work_root: Path, result: dict) -> int:
    if overlaps(work_root, out) or overlaps(work_root, target):
        raise GateError("worktree root must not overlap the output or target directory")
    manifest = gate_manifest()
    result["manifest_cases"] = len(manifest)
    result["baseline"]["sha"] = resolve_rev(repo, args.baseline)
    result["candidate"]["sha"] = resolve_rev(repo, args.candidate)
    persist(out, result)
    # Sequential builds into one shared target dir. Each binary is copied out before the next build.
    for label in ("baseline", "candidate"):
        build_revision(label, args, repo, out, target, work_root, result["cleanup_errors"], result[label])
        persist(out, result)
    exes = {binary: out / result[binary]["binary"] for binary in ("baseline", "candidate")}
    expected_sha = {binary: result[binary]["binary_sha256"] for binary in ("baseline", "candidate")}
    rounds: dict[str, list[dict]] = {side: [] for side in ALL_SIDES}
    first_cases: list[dict] | None = None
    for step in session_plan(args.pairs):
        side, pair, order = step["side"], step["pair"], step["order"]
        binary = step["binary"]
        # Same saved baseline executable for baseline and both control sides; checked before each round.
        verify_binary(exes[binary], expected_sha[binary])
        run = run_round(exes[binary], side, pair, order, out, args.run_timeout)
        run["family"] = step["family"]
        run["binary"] = result[binary]["binary"]
        run["binary_sha256"] = expected_sha[binary]
        result["rounds"].append(run)
        errors = validate_round(run, manifest)
        if first_cases is None:
            first_cases = run["cases"]
        else:
            errors += check_round_consistency(first_cases, run)
        persist(out, result)
        if errors:
            raise GateError(f"pair {pair} {side}: " + "; ".join(errors))
        rounds[side].append(run)
    for binary in ("baseline", "candidate"):
        verify_binary(exes[binary], expected_sha[binary])  # still unchanged after the last round
    for side in ALL_SIDES:
        problems = check_side_rounds(rounds[side], args.pairs, side)
        if problems:
            raise GateError("; ".join(problems))
    result["summary"] = summarize(
        rounds["baseline"], rounds["candidate"], rounds["control-baseline"], rounds["control-probe"]
    )
    result["verdict"] = result["summary"]["verdict"]
    return EXIT_PASS if result["verdict"] == "PASS" else EXIT_FAIL


def cmd_run(args) -> int:
    if args.pairs < MIN_PAIRS:
        print(f"error: --pairs must be at least {MIN_PAIRS}", file=sys.stderr)
        return EXIT_INVALID
    if args.jobs < 1:
        print("error: --jobs must be at least 1", file=sys.stderr)
        return EXIT_INVALID
    repo = Path(args.repo).resolve()
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = Path(args.out_dir).resolve() if args.out_dir else repo / "target" / "search-gate" / stamp
    target = Path(args.target_dir).resolve() if args.target_dir else repo / "target" / "search-gate-cargo"
    if is_within(out, repo) and not is_within(out, repo / "target"):
        print("error: --out-dir inside the repository must be under target/ (raw outputs stay out of git)", file=sys.stderr)
        return EXIT_INVALID
    out.mkdir(parents=True, exist_ok=True)
    owned_work = args.work_dir is None
    work_root = Path(tempfile.mkdtemp(prefix="search-gate-wt-")) if owned_work else Path(args.work_dir).resolve()
    work_root.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "started_utc": stamp,
        "pairs": args.pairs,
        "jobs": args.jobs,
        "cargo_flags": ["--locked", "-j", str(args.jobs)],
        "machine": machine_identity(),
        "out_dir": str(out),
        "target_dir": str(target),
        "work_root": str(work_root),
        "baseline": {"rev": args.baseline},
        "candidate": {"rev": args.candidate},
        "rounds": [],
        "summary": None,
        "verdict": "INVALID",
        "error": None,
        "cleanup_errors": [],
    }
    code = EXIT_INVALID
    try:
        code = run_gate(args, repo, out, target, work_root, result)
    except Exception as exc:  # recorded in result.json; the gate is then INVALID
        result["error"] = f"{type(exc).__name__}: {exc}"
        result["verdict"] = "INVALID"
        code = EXIT_INVALID
    finally:
        # Only the temporary worktree root is removed. out_dir and target_dir are never touched.
        if owned_work:
            try:
                shutil.rmtree(work_root)
            except OSError as exc:
                result["cleanup_errors"].append(f"could not remove temporary worktree root {work_root}: {exc}")
        persist(out, result)
        (out / "summary.md").write_text(render_markdown(result), encoding="utf-8")
    if result.get("summary"):
        print("\n".join(format_case_table(result["summary"])))
        print("p95 ratios are informational; the verdict uses p50 ratios against each case's band.")
    print(verdict_line(result))
    print(f"summary: {out / 'summary.md'}")
    print(f"result:  {out / 'result.json'}")
    if result["error"]:
        print(f"error:   {result['error']}", file=sys.stderr)
    for problem in result["cleanup_errors"]:
        print(f"cleanup: {problem}", file=sys.stderr)
    return code


def cmd_patch(args) -> int:
    src, dst = Path(args.src), Path(args.dst)
    try:
        applied = patch_file(src, dst)
    except GateError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return EXIT_INVALID
    print(f"patched: {applied} -> {dst}")
    return EXIT_PASS


# ---------------------------------------------------------------- self-test

# Synthetic subset with explicit expectations. The full 47-case manifest is
# checked against the test source instead of being rebuilt from output.
SYNTHETIC_SAMPLE = "\n".join([
    "running 1 test",
    "word index, 10,000 pages: 5 words, 2 deletion keys, 9 estimated bytes, 1.000 ms build",
    "warm search, 10,000 pages, limit 50: p50 0.400 ms, p95 0.900 ms (n=2500, target p95 < 20 ms)",
    'query "p": p50 0.100 ms, p95 0.200 ms (n=50, target p95 < 20 ms)',
    'filtered deep in, "in:[[01J9ABC]]", 10,000 pages: p50 1.000 ms, p95 2.000 ms (n=50, target p95 < 20 ms)',
    'filtered tag on 10k, "tag:perf/all", 10,000 pages: p50 1.500 ms, p95 2.500 ms (n=50, target p95 < 20 ms)',
    'filtered typo "gardne tag:perf/broad", 10,000 pages: p50 3.000 ms, p95 4.000 ms (n=50, fuzzy ceiling p95 < 30 ms)',
    "test warm_search_p95_under_20_ms ... ok",
    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
])
SUBSET = [
    ("warm search, 10,000 pages, limit 50", 2500),
    ('query "p"', 50),
    ('filtered deep in, "in:[[<ID>]]", 10,000 pages', 50),
    ('filtered tag on 10k, "tag:perf/all", 10,000 pages', 50),
    ('filtered typo "gardne tag:perf/broad", 10,000 pages', 50),
]


def cmd_selftest(args) -> int:
    failures = []

    def check(name: str, ok: bool, detail: str = "") -> None:
        print(("ok   " if ok else "FAIL ") + name + (f" ({detail})" if detail and not ok else ""))
        if not ok:
            failures.append(name)

    # Manifest: 47 cases, unique labels, counts 950/650/100 and 50s.
    manifest = gate_manifest()
    check("manifest has 47 cases", len(manifest) == EXPECTED_CASE_COUNT, str(len(manifest)))
    check("manifest labels are unique", len({label for label, _ in manifest}) == len(manifest))
    check(
        "manifest sample counts",
        manifest[0] == ("warm search, 10,000 pages, limit 50", 950)
        and manifest[20] == ("fuzzy search, 10,000 pages, limit 50", 650)
        and manifest[34] == ("uncached tag candidate construction, 100 distinct filters, 10,000 pages", 100)
        and sum(1 for _, n in manifest if n == 50) == 44,
        str([manifest[0], manifest[20], manifest[34]]),
    )

    # Parser: normalization, info line, result line, numeric fields.
    parsed = parse_output(SYNTHETIC_SAMPLE)
    labels = [c["label"] for c in parsed["cases"]]
    check("parser counts all five synthetic cases", len(labels) == 5, str(len(labels)))
    check("parser normalizes deep id", labels[2] == 'filtered deep in, "in:[[<ID>]]", 10,000 pages')
    check("parser keeps info line", len(parsed["info"]) == 1)
    check("parser sees ok result line", parsed["test_passed"] and not parsed["panics"])
    check(
        "parser reads p50/p95/n",
        parsed["cases"][0] == {"label": labels[0], "p50_ms": 0.4, "p95_ms": 0.9, "n": 2500},
    )
    two_ids = parse_output(
        'filtered deep in, "in:[[AAA]]", 10,000 pages: p50 1.000 ms, p95 2.000 ms (n=50, x)\n'
        'filtered deep in, "in:[[BBB]]", 10,000 pages: p50 1.000 ms, p95 2.000 ms (n=50, x)'
    )
    check(
        "two random ids normalize to one label",
        len({c["label"] for c in two_ids["cases"]}) == 1,
    )

    # libtest inline prefix (seen on the Linux machine): the first output line starts with "test <name> ... ".
    prefixed_text = SYNTHETIC_SAMPLE.replace(
        "word index,", f"test {TEST_NAME} ... word index,", 1
    )
    prefixed = parse_output(prefixed_text)
    check(
        "inline libtest prefix: word index line still parsed",
        len(prefixed["info"]) == 1 and prefixed["info"][0].startswith("word index,"),
        str(prefixed["info"]),
    )
    check("inline libtest prefix: five cases still parsed", len(prefixed["cases"]) == 5)
    prefixed_run = {
        "exit_code": 0, "timed_out": False, "panics": [], "test_passed": prefixed["test_passed"],
        "info": prefixed["info"], "cases": prefixed["cases"],
    }
    prefix_errors = validate_round(prefixed_run, SUBSET)
    check("inline libtest prefix: validator accepts round", prefix_errors == [], str(prefix_errors))
    # Prefix plus a slow-test notice on the same line, as libtest can print it.
    noticed = parse_output(
        SYNTHETIC_SAMPLE.replace(
            "word index,",
            f"test {TEST_NAME} ... test {TEST_NAME} has been running for over 60 seconds word index,",
            1,
        )
    )
    check("slow notice before word index: info still parsed", len(noticed["info"]) == 1, str(noticed["info"]))
    # Only this test's prefix is stripped; another test's prefix is not part of the gate output.
    other = parse_output("test some_other_test ... word index, 10,000 pages: 1 words")
    check("other test prefix is not stripped", other["info"] == [] and other["cases"] == [])

    # Validator against explicit synthetic expectations.
    clean = {
        "exit_code": 0, "timed_out": False, "panics": [], "test_passed": True,
        "info": parsed["info"], "cases": parsed["cases"],
    }
    check("validator accepts clean synthetic round", validate_round(clean, SUBSET) == [])
    check(
        "validator rejects missing word index line",
        any("word index" in e for e in validate_round(clean | {"info": []}, SUBSET)),
    )
    check("validator rejects nonzero exit", bool(validate_round(clean | {"exit_code": 101}, SUBSET)))
    check("validator rejects timeout", bool(validate_round(clean | {"timed_out": True, "exit_code": None}, SUBSET)))
    check("validator rejects panic", bool(validate_round(clean | {"panics": ["panicked at x"]}, SUBSET)))
    check("validator rejects missing ok line", bool(validate_round(clean | {"test_passed": False}, SUBSET)))
    truncated = validate_round(clean | {"cases": parsed["cases"][:-1]}, SUBSET)
    check("validator rejects truncated round", any("printed 4 cases" in e for e in truncated), str(truncated))
    check("validator rejects extra case", bool(validate_round(clean, SUBSET[:-1])))
    dup = clean | {"cases": parsed["cases"] + [parsed["cases"][1]]}
    dup_errors = validate_round(dup, SUBSET + [SUBSET[1]])
    check("validator rejects duplicate labels", any("duplicate" in e for e in dup_errors), str(dup_errors))
    wrong_n = [("warm search, 10,000 pages, limit 50", 2400)] + SUBSET[1:]
    check("validator rejects n mismatch against expectation", any("n=2500" in e for e in validate_round(clean, wrong_n)))
    swapped = clean | {"cases": [parsed["cases"][1], parsed["cases"][0]] + parsed["cases"][2:]}
    check("validator rejects reordered labels", bool(validate_round(swapped, SUBSET)))
    bad_order = clean | {"cases": [parsed["cases"][0] | {"p50_ms": 2.0, "p95_ms": 1.0}] + parsed["cases"][1:]}
    check("validator rejects p95 below p50", any("p95 < p50" in e for e in validate_round(bad_order, SUBSET)))

    # Consistency across rounds: n must not change after round 1.
    later_same = clean
    later_changed = clean | {"cases": [parsed["cases"][0] | {"n": 40}] + parsed["cases"][1:]}
    check("cross-round n unchanged passes", check_round_consistency(clean["cases"], later_same) == [])
    check("cross-round n change rejected", bool(check_round_consistency(clean["cases"], later_changed)))

    check("side check flags missing rounds", len(check_side_rounds([clean] * 4, 5, "baseline")) == 1)
    check("side check accepts full rounds", check_side_rounds([clean] * 5, 5, "baseline") == [])
    check(
        "pair orders alternate AB BA",
        [order_for_pair(p) for p in range(1, 6)] == ["AB", "BA", "AB", "BA", "AB"],
    )

    # Control band and summary. Each helper builds five rounds per side for one or more cases.
    def rounds_for(cases: list[tuple[str, list[float], list[float]]]) -> list[dict]:
        count = len(cases[0][1])
        return [
            {"cases": [{"label": lab, "p50_ms": p50[i], "p95_ms": p95[i], "n": 50} for lab, p50, p95 in cases]}
            for i in range(count)
        ]

    def uniform(p50: float, p95: float, label: str = "a") -> list[dict]:
        return rounds_for([(label, [p50] * 5, [p95] * 5)])

    def row_for(cand50: float, probe50: float, *, base50: float = 1.0, ref50: float = 1.0,
                cand95: float | None = None, base95: float | None = None) -> dict:
        """One case: real baseline/candidate and control ref/probe, p95 defaulting to twice p50."""
        base95 = base95 if base95 is not None else base50 * 2
        cand95 = cand95 if cand95 is not None else cand50 * 2
        return summarize(
            uniform(base50, base95), uniform(cand50, cand95),
            uniform(ref50, base95), uniform(probe50, probe50 * 2),
        )["rows"][0]

    quiet = row_for(1.0, 1.0)
    check(
        "quiet control, candidate 1.00: pass with band 0.05",
        quiet["verdict"] == "pass" and quiet["band"] == BAND_FLOOR and quiet["control_spread"] == 0.0,
        str(quiet),
    )
    check("candidate 1.06 fails with quiet control (band 0.05)", row_for(1.06, 1.0)["verdict"] == "fail")
    # Floor boundary: candidate ratio exactly 1.05 (2.1 / 2.0) is at 1 + band and passes.
    floor_edge = row_for(2.1, 1.0, base50=2.0)
    check(
        "floor boundary: ratio 1.05 with band 0.05 passes",
        floor_edge["p50_ratio"] == 1.05 and floor_edge["band"] == 0.05 and floor_edge["verdict"] == "pass",
        str(floor_edge),
    )
    check("just over the floor (1.055) fails", row_for(2.11, 1.0, base50=2.0)["verdict"] == "fail")
    # Control spread widens the band above the floor; the boundary is inclusive.
    wide = row_for(1.25, 1.25)
    check(
        "control spread 0.25 widens band; ratio 1.25 at 1 + band passes",
        wide["band"] == 0.25 and wide["control_spread"] == 0.25 and wide["verdict"] == "pass",
        str(wide),
    )
    check("candidate 1.30 fails even with spread 0.25", row_for(1.3, 1.25)["verdict"] == "fail")
    # Control below 1 counts too: spread is an absolute difference.
    below = row_for(1.07, 0.92)
    check(
        "control 0.92 gives spread 0.08 (absolute), band 0.08; candidate 1.07 passes",
        abs(below["control_spread"] - 0.08) < 1e-9 and abs(below["band"] - 0.08) < 1e-9 and below["verdict"] == "pass",
        str(below),
    )
    check("candidate 1.09 fails with spread 0.08", row_for(1.09, 0.92)["verdict"] == "fail")
    # A noisy control below the floor does not shrink the band: band is max(0.05, spread).
    check("control 1.02 keeps band at the 0.05 floor", row_for(1.04, 1.02)["band"] == 0.05)
    check("control 1.02, candidate 1.06 fails", row_for(1.06, 1.02)["verdict"] == "fail")

    # p95 ratios are informational: a large p95 ratio does not fail a case whose p50 is in band.
    p95_info = row_for(1.0, 1.0, cand95=3.0, base95=2.0)
    check(
        "p95 ratio 1.5 is informational: case passes",
        p95_info["verdict"] == "pass" and p95_info["p95_ratio"] == 1.5,
        str(p95_info),
    )
    check(
        "p95 informational: overall PASS with p95 ratio 1.5",
        summarize(uniform(1.0, 2.0), uniform(1.0, 3.0), uniform(1.0, 2.0), uniform(1.0, 2.0))["verdict"] == "PASS",
    )
    # p50 still decides: a good p95 does not rescue an over-band p50.
    p50_slow = row_for(1.3, 1.0, cand95=1.0)
    check("p50 over band fails even with p95 ratio 0.5", p50_slow["verdict"] == "fail" and p50_slow["p95_ratio"] == 0.5)

    # Counting and the final line: one slow case out of two.
    two_cases = summarize(
        rounds_for([("a", [1.0] * 5, [2.0] * 5), ("b", [1.0] * 5, [2.0] * 5)]),
        rounds_for([("a", [1.0] * 5, [2.0] * 5), ("b", [1.3] * 5, [2.6] * 5)]),
        rounds_for([("a", [1.0] * 5, [2.0] * 5), ("b", [1.0] * 5, [2.0] * 5)]),
        rounds_for([("a", [1.0] * 5, [2.0] * 5), ("b", [1.0] * 5, [2.0] * 5)]),
    )
    check("one case beyond band is counted", two_cases["cases_beyond_band"] == 1 and two_cases["verdict"] == "FAIL")
    check(
        "FAIL line names the count",
        verdict_line({"verdict": "FAIL", "summary": two_cases}) == "verdict: FAIL (1 cases beyond band)",
    )
    check(
        "PASS line reports zero cases beyond band",
        verdict_line({"verdict": "PASS", "summary": {"cases_beyond_band": 0}}) == "verdict: PASS (0 cases beyond band)",
    )
    check("INVALID line", verdict_line({"verdict": "INVALID", "summary": None}) == "verdict: INVALID")
    check("case table marks a failing row", "| fail |" in "\n".join(format_case_table(two_cases)))

    # A zero control-baseline p50 makes the control ratio undefined: the run is invalid, not a pass.
    try:
        row_for(1.0, 1.0, ref50=0.0)
        check("zero control-baseline p50 is invalid", False)
    except GateError:
        check("zero control-baseline p50 is invalid", True)

    # Session plan: real and control pairs interleave; control rounds use the baseline binary only.
    plan = session_plan(5)
    check("session plan has 4 rounds per pair", len(plan) == 20, str(len(plan)))
    check(
        "each side gets 5 rounds",
        all(sum(1 for s in plan if s["side"] == side) == 5 for side in ALL_SIDES),
    )
    check(
        "control rounds all run the baseline binary",
        all(s["binary"] == "baseline" for s in plan if s["family"] == "control")
        and {s["side"] for s in plan if s["family"] == "control"} == set(CONTROL_SIDES),
    )
    check(
        "pair 1: real AB then control BA, interleaved",
        [(s["family"], s["side"]) for s in plan[:4]] == [
            ("gate", "baseline"), ("gate", "candidate"), ("control", "control-probe"), ("control", "control-baseline"),
        ],
        str([(s["family"], s["side"]) for s in plan[:4]]),
    )
    check(
        "pair 2: real BA then control AB",
        [(s["family"], s["side"]) for s in plan[4:8]] == [
            ("gate", "candidate"), ("gate", "baseline"), ("control", "control-baseline"), ("control", "control-probe"),
        ],
    )

    # Binary check: the saved executable must still match its recorded SHA-256.
    with tempfile.TemporaryDirectory(prefix="gate-selftest-") as tmp:
        exe = Path(tmp) / "baseline.bin"
        exe.write_bytes(b"baseline executable bytes")
        recorded = sha256_file(exe)
        try:
            verify_binary(exe, recorded)
            verified = True
        except GateError:
            verified = False
        check("verify_binary accepts the recorded sha256", verified)
        exe.write_bytes(b"changed bytes")
        try:
            verify_binary(exe, recorded)
            check("verify_binary refuses a changed binary", False)
        except GateError as exc:
            check("verify_binary refuses a changed binary", "binary changed" in str(exc))

    check("side check flags a missing control side", len(check_side_rounds([clean] * 4, 5, "control-probe")) == 1)

    # Control cannot be turned off from the command line.
    try:
        with contextlib.redirect_stderr(io.StringIO()):
            build_parser().parse_args(
                ["run", "--baseline", "a", "--candidate", "b", "--no-control"]
            )
        check("run refuses a control-off flag", False)
    except SystemExit:
        check("run refuses a control-off flag", True)

    # Jobs default and worktree overlap and out-dir guards.
    check("jobs cap is 2 for the default cargo -j", JOBS_CAP == 2 and 1 <= default_jobs() <= 2)
    check("overlap guard flags worktree inside out dir", overlaps(Path("/x/out/wt"), Path("/x/out")))
    check("overlap guard allows sibling dirs", not overlaps(Path("/x/wt"), Path("/x/out")))
    repo_probe = Path("/x/repo")
    check("out-dir under repo target is allowed", is_within(repo_probe / "target" / "g", repo_probe / "target"))
    check("out-dir elsewhere in repo is refused", is_within(repo_probe / "docs" / "g", repo_probe) and not is_within(repo_probe / "docs" / "g", repo_probe / "target"))

    # Rendering: setup failure with no sha, patched or machine must not raise.
    try:
        text = render_markdown({"verdict": "INVALID", "error": "setup failed", "pairs": 5})
        check("render with missing sha/patched is safe", "not set" in text and "INVALID" in text)
    except Exception as exc:  # report, do not hide
        check("render with missing sha/patched is safe", False, repr(exc))

    def skip(name: str, detail: str) -> None:
        print(f"skip {name} ({detail})")

    # Patch input is the historical unpatched revision, never HEAD: HEAD may already
    # carry the switch, which would make every patch check meaningless.
    repo = Path(args.repo).resolve()
    base_text = None
    try:
        base_sha = resolve_rev(repo, args.unpatched_rev)
    except GateError as exc:
        check(f"unpatched revision {args.unpatched_rev} resolves", False, str(exc))
    else:
        shown = subprocess.run(
            ["git", "-C", str(repo), "show", f"{base_sha}:{TEST_FILE}"], capture_output=True, check=False
        )
        if shown.returncode != 0:
            check(f"{TEST_FILE} readable at {args.unpatched_rev}", False, shown.stderr.decode(errors="replace").strip())
        else:
            base_text = shown.stdout.decode("utf-8")
    patched = None
    if base_text is None:
        skip("patch checks", "no unpatched revision text")
    else:
        problems = check_source_manifest(base_text)
        check("historical source matches manifest", problems == [], str(problems[:2]))
        patched, applied = patch_text(base_text)
        check("patch applies to unpatched revision", applied)
        check("patch count is six gated timing asserts", patched.count(REPORT_ONLY_CALL) == 6)
        again, applied_again = patch_text(patched)
        check("patch is idempotent when switch present", not applied_again and again == patched)
        strict_sample = base_text.replace(TIMING_PATCHES[2][0], TIMING_PATCHES[2][0].replace("p95 < BUDGET", "p95 > BUDGET"))
        try:
            patch_text(strict_sample)
            check("patch refuses drifted timing condition", False)
        except GateError:
            check("patch refuses drifted timing condition", True)
        crlf, _ = patch_text(base_text.replace("\n", "\r\n"))
        check("patch keeps CRLF line endings", "\r\n" in crlf and "\n" not in crlf.replace("\r\n", ""))
        check("patch of CRLF text equals LF result", crlf.replace("\r\n", "\n") == patched)

    # Working-tree candidate: always checked against the manifest; compared byte for byte
    # only when it carries the switch (an unswitched checkout cannot equal the patch output).
    candidate_file = repo / TEST_FILE
    if not candidate_file.exists():
        skip("candidate checks", f"{TEST_FILE} missing in working tree")
    else:
        cand_text = candidate_file.read_text(encoding="utf-8")
        problems = check_source_manifest(cand_text)
        check("candidate source matches manifest", problems == [], str(problems[:2]))
        drifted = cand_text.replace('"xylophone"', '"xylo"', 1)
        check("source manifest flags a changed query", bool(check_source_manifest(drifted)))
        if patched is None:
            skip("patched baseline equals candidate", "no patch output")
        elif SWITCH not in cand_text:
            skip("patched baseline equals candidate", "working-tree file has no switch")
        else:
            check(
                "patched baseline equals candidate file byte for byte",
                patched.encode("utf-8") == candidate_file.read_bytes(),
            )

    print(f"selftest: {'PASS' if not failures else 'FAIL'} ({len(failures)} failed)")
    return EXIT_PASS if not failures else EXIT_FAIL


# ---------------------------------------------------------------- cli


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="A/B search-gate harness: baseline vs candidate revision, report-only, AB/BA pairs.",
        epilog=(
            "examples:\n"
            "  python crates/core/benches/ab_search_gate.py run --baseline <rev> --candidate <rev>\n"
            "  python crates/core/benches/ab_search_gate.py run --baseline <rev> --candidate <rev> "
            "--pairs 7 --out-dir <dir> --jobs 2\n"
            "  python crates/core/benches/ab_search_gate.py patch <src.rs> <dst.rs>\n"
            "  python crates/core/benches/ab_search_gate.py selftest\n"
            "run exits 0 on PASS, 1 on FAIL (a case's p50 ratio over its control band), 2 when the gate is INVALID.\n"
            "See crates/core/benches/README.md for artifact semantics."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser(
        "run",
        help="build both revisions sequentially and run the A/B rounds (heavy)",
        description=(
            "Builds baseline, then candidate, in detached worktrees under a temporary directory "
            "(each removed after its build), into one shared CARGO_TARGET_DIR with cargo --locked. "
            f"Keeps each test binary, then runs {TEST_NAME} with {SWITCH}=1 in AB/BA pairs. "
            "Control: every session also runs same-binary pairs (the baseline executable against "
            "itself, same SHA-256, interleaved with the real pairs); it cannot be turned off. "
            "Verdict: each case's median-round p50 ratio (candidate / baseline) must be <= "
            f"1 + band, where band = max({BAND_FLOOR}, |control median-round p50 ratio - 1|). "
            "p95 ratios are printed for information only. "
            f"Patches a revision's test file only when {SWITCH} is absent. Every round must print "
            f"the {EXPECTED_CASE_COUNT} manifest cases with the expected sample counts; duplicate "
            "labels, count changes, missing rounds or correctness failures invalidate the gate."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    run.add_argument("--baseline", required=True, help="baseline revision (commit, branch or tag)")
    run.add_argument("--candidate", required=True, help="candidate revision")
    run.add_argument(
        "--pairs", type=int, default=DEFAULT_PAIRS,
        help=f"real and control pairs each (default {DEFAULT_PAIRS}, minimum {MIN_PAIRS}); 4 rounds per pair",
    )
    run.add_argument(
        "--jobs", type=int, default=default_jobs(),
        help=f"cargo build jobs, passed as -j (default max(1, min({JOBS_CAP}, CPUs)) = {default_jobs()})",
    )
    run.add_argument("--repo", default=".", help="repository to resolve revisions in (default: current directory)")
    run.add_argument("--out-dir", help="output directory (default: <repo>/target/search-gate/<UTC stamp>)")
    run.add_argument("--target-dir", help="shared CARGO_TARGET_DIR (default: <repo>/target/search-gate-cargo)")
    run.add_argument(
        "--work-dir",
        help="parent for the detached worktrees (default: a fresh temp directory, removed at the end)",
    )
    run.add_argument("--cargo", help="cargo executable (default: cargo on PATH)")
    run.add_argument("--run-timeout", type=int, default=RUN_TIMEOUT_S, help=f"seconds per benchmark run (default {RUN_TIMEOUT_S})")
    run.set_defaults(func=cmd_run)

    patch = sub.add_parser(
        "patch",
        help="copy a test file, adding the report-only switch if absent",
        description="Writes dst from src. If the switch already exists, dst is an exact copy.",
    )
    patch.add_argument("src", help="source search_perf.rs")
    patch.add_argument("dst", help="destination path")
    patch.set_defaults(func=cmd_patch)

    selftest = sub.add_parser(
        "selftest",
        help="check manifest, parser, validator, summary and patch without Cargo or benchmarks",
    )
    selftest.add_argument("--repo", default=".", help="repository to read revisions from (default: current directory)")
    selftest.add_argument(
        "--unpatched-rev", default=HISTORICAL_BASELINE,
        help=f"revision whose search_perf.rs has no switch, used as the patch input (default {HISTORICAL_BASELINE})",
    )
    selftest.set_defaults(func=cmd_selftest)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "run":
        args.cargo = args.cargo or "cargo"
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
