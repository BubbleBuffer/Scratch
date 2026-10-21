#!/usr/bin/env python3
"""Paired cold-history A/B runner and aggregate report. Run on the Linux machine (Linux only).

Runs the pinned default (non-instrumented) release binaries one at a time, each on a
fresh copy of an aged fixture, then aggregates completed pairs into JSON and Markdown.
Standard library only.

Subcommands:
  run     run the requested pairs, serially. Odd pair: master then branch. Even pair:
          branch then master. Each pair runs the history phase for both sides, then the
          sync phase for both sides.
  report  aggregate completed pairs (at least five) from one or more output roots.

Usage:
  python3 cold_ab.py run --pairs 3
  python3 cold_ab.py run --pairs 3,4,5 [--retry-side-once] [--load-host home-server]
  python3 cold_ab.py run --pairs 4 --dry-run          # prints the plan, writes nothing
  python3 cold_ab.py report                           # v4 pairs 1,2 + v5 pairs 3,4,5
  python3 cold_ab.py report --source ROOT=1,2 --source ROOT2=3,4,5 --out-dir DIR

Defaults: fixtures in cold-history-measure-20261013-v3 (master uses aged-hot, branch uses
aged-cold, the states the v4 pairs measured; each log header is checked against that),
binaries in cold-history-measure-20261013-v4/bin, output in cold-history-measure-20261013-v5.

Failure policy: a side-run that fails validation is kept as <label>.failed-<k> (log, load
files, working directory, JSON record) and stops the run. --retry-side-once reruns that
whole side once with a fresh copy; the first failure stays on disk. Flush-lock timeouts
are classified as known-flush-lock-timeout and listed separately. Nothing hides them.

Load: one reading at the start and end of each pair and of each side-run. The source is
`ssh <load-host> uptime` when that works, else /proc/loadavg on this runner. Each file
states its source on a `source:` line. v4 readings predate that line.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import fcntl
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT_V3 = "/home/dev/cold-history-measure-20261013-v3"
ROOT_V4 = "/home/dev/cold-history-measure-20261013-v4"
ROOT_V5 = "/home/dev/cold-history-measure-20261013-v5"
SIDES = ("master", "branch")
KINDS = ("history", "sync")
# Positional arguments of sync_bench: the fixtures hold 10,000 pages plus one 2,000-block page.
PAGES, BIG_BLOCKS = 10000, 2000
MIN_PAIRS = 5
NUMBER = r"(?:\d+(?:\.\d*)?|\.\d+)"
LATENCIES = (
    "page open cold (fresh vault)",
    "page open warm (repeat load)",
    "history list first (fresh vault)",
    "history list repeat",
    "old version first (fresh vault)",
    "old version repeat",
    "edit first (fresh vault, cold page)",
    "edit second (same vault)",
)
BIG_LATENCIES = tuple(name + ", big page" for name in LATENCIES)
LOCK_TIMEOUT = "timed out waiting for the vault flush lock"
PANIC = re.compile(r"panicked at|assertion[^\n]*failed", re.I)
P50 = re.compile(r"^(.+?): p50 (" + NUMBER + r") ms,.*\(n=(\d+)\)\s*$", re.M)
HISTORY_HEADER = re.compile(r"^history dir=(.*?) samples=(\d+) big_samples=(\d+) (.*?)\s*$", re.M)
EXPORT = re.compile(r"^full export pages=(\d+) bytes=(\d+) elapsed_ms=(" + NUMBER + r")\s*$", re.M)
CHECK = re.compile(r"^check versions_mismatch=(\d+) old_view_mismatch=(\d+) of (\d+)\s*$", re.M)
SYNC_HUB = re.compile(r"^hub vault: (\d+) pages, (" + NUMBER + r") MB on disk\s*$", re.M)
SYNC_CORE = re.compile(
    r"^core full sync of (\d+) pages: export (" + NUMBER + r") s \((" + NUMBER + r") MB\), "
    r"import (" + NUMBER + r") s, flush (" + NUMBER + r") s\s*$",
    re.M,
)
SYNC_HTTP = re.compile(
    r"^hub full sync of (\d+) pages into an empty vault: (" + NUMBER + r") s \((\d+) rounds, "
    r"(" + NUMBER + r") MB down, (" + NUMBER + r") MB up\), then flush (" + NUMBER + r") s\s*$",
    re.M,
)
SYNC_CLIENT = re.compile(r"^client vault after sync: (" + NUMBER + r") MB on disk\s*$", re.M)
SYNC_DELTA = {
    name: re.compile(
        r"^delta sync, " + re.escape(name) + r": p50 (" + NUMBER + r") ms, max (" + NUMBER + r") ms \(n=20\)\s*$",
        re.M,
    )
    for name in ("nothing changed", "one page edited on the hub", "one page edited on the client")
}
LOAD = re.compile(r"load average:\s*(" + NUMBER + r"),\s*(" + NUMBER + r"),\s*(" + NUMBER + r")")
SOURCE = re.compile(r"^source: (.*?)\s*$", re.M)
LABEL = re.compile(r"^aged-(master|branch)(-sync)?-(\d+)(?:\.failed-(\d+))?$")


class Invalid(Exception):
    """A log or load file that does not pass validation."""


class Fatal(RuntimeError):
    """Unexpected condition: stop now and leave everything on disk."""


class PairStopped(RuntimeError):
    """A side-run failed and was kept on disk, so the pair cannot complete."""


def iso_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def iso_at(timestamp: float) -> str:
    return dt.datetime.fromtimestamp(timestamp, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_pairs(text: str) -> list[int]:
    try:
        numbers = [int(part) for part in text.split(",") if part.strip()]
    except ValueError as error:
        raise Fatal(f"bad pair list {text!r}") from error
    if not numbers or any(n < 1 for n in numbers) or len(set(numbers)) != len(numbers):
        raise Fatal(f"bad pair list {text!r}: give positive, distinct pair numbers such as 3,4,5")
    return numbers


def label_for(pair: int, side: str, kind: str) -> str:
    return f"aged-{side}-{pair}" if kind == "history" else f"aged-{side}-sync-{pair}"


def binary_for(bin_dir: Path, side: str, kind: str) -> Path:
    return bin_dir / (f"{side}-bench" if kind == "history" else f"{side}-sync")


def order_for(pair: int) -> tuple[str, str]:
    return ("master", "branch") if pair % 2 else ("branch", "master")


def overlaps(a: Path, b: Path) -> bool:
    return a == b or a in b.parents or b in a.parents


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fingerprint(root: Path) -> dict:
    """File count, byte total and link count of a tree. Links are counted and never followed."""
    files = nbytes = links = 0
    for dirpath, dirnames, filenames in os.walk(root):
        for name in dirnames:
            if os.path.islink(os.path.join(dirpath, name)):
                links += 1
        for name in filenames:
            path = os.path.join(dirpath, name)
            if os.path.islink(path):
                links += 1
                continue
            files += 1
            nbytes += os.lstat(path).st_size
    return {"files": files, "bytes": nbytes, "symlinks": links}


def copy_tree(src: Path, dst: Path) -> None:
    """Copy the contents of src, hidden files included, into the new directory dst."""
    dst.mkdir()
    try:
        subprocess.run(["cp", "-a", "--", f"{src}/.", str(dst)], check=True)
    except subprocess.CalledProcessError as error:
        raise Fatal(f"copying {src} to {dst} failed; the partial copy stays on disk") from error


def remove_owned(out: Path, name: str) -> None:
    """Remove one disposable directory that this runner created inside out. Refuses links."""
    path = out / name
    if os.path.islink(path):
        raise Fatal(f"refusing to remove {path}: it is a link")
    if not os.path.lexists(path):
        return
    if not path.is_dir():
        raise Fatal(f"refusing to remove {path}: not a directory")
    real = path.resolve(strict=True)
    if real.parent != out.resolve() or real.name != name:
        raise Fatal(f"refusing to remove {path}: it resolves to {real}")
    for dirpath, dirnames, filenames in os.walk(path):
        for entry in dirnames + filenames:
            if os.path.islink(os.path.join(dirpath, entry)):
                raise Fatal(f"refusing to remove {path}: link at {os.path.join(dirpath, entry)}")
    shutil.rmtree(path)


def move_if_exists(src: Path, dst: Path) -> None:
    if os.path.lexists(src):
        if os.path.lexists(dst):
            raise Fatal(f"cannot keep {src.name}: {dst.name} already exists")
        os.rename(src, dst)


def capture_load(path: Path, host: str) -> None:
    """One load reading. Writes a source: line, then the line that carries the load average."""
    if host:
        try:
            done = subprocess.run(
                ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", host, "uptime"],
                stdin=subprocess.DEVNULL,
                capture_output=True,
                text=True,
                timeout=15,
            )
        except (OSError, subprocess.TimeoutExpired):
            done = None
        if done is not None and done.returncode == 0 and LOAD.search(done.stdout):
            path.write_text(f"source: ssh {host} uptime\n{done.stdout.strip()}\n", encoding="utf-8")
            return
    try:
        one, five, fifteen = Path("/proc/loadavg").read_text(encoding="utf-8").split()[:3]
    except (OSError, ValueError) as error:
        raise Fatal("no load reading: ssh failed and /proc/loadavg is unreadable") from error
    reason = f"ssh {host} uptime unavailable" if host else "ssh disabled"
    path.write_text(
        f"source: fallback /proc/loadavg on this runner ({reason})\n"
        f"{iso_now()} load average: {one}, {five}, {fifteen}\n",
        encoding="utf-8",
    )


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise Invalid(f"cannot read {path.name}: {error}") from error


def unique(pattern: re.Pattern, text: str, what: str):
    found = list(pattern.finditer(text))
    if len(found) != 1:
        raise Invalid(f"{what}: expected one matching line, found {len(found)}")
    return found[0]


def read_load(path: Path) -> dict:
    raw = read_text(path)
    found = list(LOAD.finditer(raw))
    if len(found) != 1:
        raise Invalid(f"{path.name}: expected one load-average reading, found {len(found)}")
    source = SOURCE.search(raw)
    return {
        "file": path.name,
        "load_1_5_15": [float(v) for v in found[0].groups()],
        "source": source.group(1) if source else "not recorded (v4 uptime output)",
        "raw": raw.strip(),
    }


def parse_history_text(text: str, side: str) -> dict:
    if PANIC.search(text):
        raise Invalid("panic or failed assertion in the log")
    head = unique(HISTORY_HEADER, text, "history header")
    samples, big = int(head.group(2)), int(head.group(3))
    fields = {key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", head.group(4))}
    needed = ("hot_files", "hot_bytes", "cold_files", "cold_bytes", "dict_files", "dict_bytes", "copy_skipped")
    missing = [key for key in needed if key not in fields]
    if missing:
        raise Invalid(f"history header lacks {missing}")
    if fields["copy_skipped"] or big != 1 or samples <= big:
        raise Invalid(f"skipped fixture entries or unexpected sample counts (samples={samples}, big={big})")
    cold, dict_files = fields["cold_files"], fields["dict_files"]
    if side == "master" and (cold or dict_files):
        raise Invalid(f"master must run on a hot-only fixture; header has cold_files={cold} dict_files={dict_files}")
    if side == "branch" and not (cold and dict_files):
        raise Invalid(f"branch must run on a fixture with cold files; header has cold_files={cold} dict_files={dict_files}")
    metrics, counts = {}, {}
    for match in P50.finditer(text):
        name, value, count = match.group(1), float(match.group(2)), int(match.group(3))
        if name in metrics:
            raise Invalid(f"duplicate metric: {name}")
        if count < 1:
            raise Invalid(f"no samples for: {name}")
        metrics[name], counts[name] = value, count
    missing = [name for name in LATENCIES + BIG_LATENCIES if name not in metrics]
    if missing:
        raise Invalid(f"missing p50 metrics: {missing}")
    export = unique(EXPORT, text, "full export line")
    pages = int(export.group(1))
    if pages != fields["hot_files"] or int(export.group(2)) <= 0:
        raise Invalid(f"full export pages={pages} or bytes={export.group(2)} do not match the fixture")
    check = unique(CHECK, text, "history check line")
    mismatches = int(check.group(1)) + int(check.group(2))
    if mismatches or int(check.group(3)) != samples:
        raise Invalid(f"history mismatches={mismatches}, checked {check.group(3)} of {samples}")
    return {
        "samples": samples,
        "big_samples": big,
        "pages": pages,
        "files": {"hot": fields["hot_files"], "cold": cold, "dict": dict_files},
        "bytes": {"hot": fields["hot_bytes"], "cold": fields["cold_bytes"], "dict": fields["dict_bytes"]},
        "metrics": metrics,
        "p50_samples": counts,
        "export_ms": float(export.group(3)),
        "export_bytes": int(export.group(2)),
        "checks": {"versions_mismatch": int(check.group(1)), "old_view_mismatch": int(check.group(2)),
                   "checked": int(check.group(3))},
    }


def parse_sync_text(text: str) -> dict:
    if PANIC.search(text):
        raise Invalid("panic or failed assertion in the log")
    hub = unique(SYNC_HUB, text, "hub vault line")
    core = unique(SYNC_CORE, text, "core full sync line")
    http = unique(SYNC_HTTP, text, "HTTP full sync line")
    client = unique(SYNC_CLIENT, text, "client vault line")
    deltas = {name: unique(pattern, text, f"delta sync, {name}") for name, pattern in SYNC_DELTA.items()}
    if int(hub.group(1)) != int(http.group(1)) or int(core.group(1)) != int(http.group(1)):
        raise Invalid("page counts differ between the hub, core sync and HTTP sync")
    # The bench prints these in order; the last line is the one that follows every sync assert.
    ordered = [core, http, client, deltas["nothing changed"], deltas["one page edited on the hub"],
               deltas["one page edited on the client"]]
    if [m.start() for m in ordered] != sorted(m.start() for m in ordered):
        raise Invalid("sync lines out of order; the run did not reach its completion marker")
    return {
        "pages": int(http.group(1)),
        "hub_pages": int(hub.group(1)),
        "hub_mb": float(hub.group(2)),
        "client_mb": float(client.group(1)),
        "http_s": float(http.group(2)),
        "rounds": int(http.group(3)),
        "down_mb": float(http.group(4)),
        "up_mb": float(http.group(5)),
        "flush_s": float(http.group(6)),
        "core": {"export_s": float(core.group(2)), "export_mb": float(core.group(3)),
                 "import_s": float(core.group(4)), "flush_s": float(core.group(5))},
        "delta_p50_ms": {name: float(match.group(1)) for name, match in deltas.items()},
        "completion_marker": "one page edited on the client",
    }


def analyze_run(root: Path, label: str, kind: str, side: str) -> dict:
    try:
        text = read_text(root / f"{label}.log")
        evidence = parse_history_text(text, side) if kind == "history" else parse_sync_text(text)
        evidence.update({
            "label": label,
            "kind": kind,
            "side": side,
            "log": str(root / f"{label}.log"),
            "load_before": read_load(root / f"{label}.load-before"),
            "load_after": read_load(root / f"{label}.load-after"),
        })
        return evidence
    except Invalid as error:
        raise Invalid(f"{label}: {error}") from error


def analyze_pair(root: Path, pair: int) -> dict:
    runs = {}
    for side in SIDES:
        history = analyze_run(root, label_for(pair, side, "history"), "history", side)
        sync = analyze_run(root, label_for(pair, side, "sync"), "sync", side)
        if sync["hub_pages"] != history["pages"] or sync["pages"] != history["pages"]:
            raise Invalid(
                f"pair {pair} {side}: history pages {history['pages']}, sync pages {sync['pages']}, "
                f"hub pages {sync['hub_pages']}"
            )
        runs[side] = {"history": history, "sync": sync}
    master, branch = runs["master"]["history"], runs["branch"]["history"]
    if (master["samples"], master["big_samples"], master["pages"]) != (branch["samples"], branch["big_samples"], branch["pages"]):
        raise Invalid(f"pair {pair}: master and branch sample or page counts differ")
    return {"pair": pair, "root": str(root), "runs": runs}


def classify(text: str, kind: str, side: str) -> tuple[str, str]:
    if LOCK_TIMEOUT in text:
        return "known-flush-lock-timeout", "owned separately; not retried by this runner"
    panic = PANIC.search(text)
    if panic:
        start = text.rfind("\n", 0, panic.start()) + 1
        return "panic", " ".join(text[start:].splitlines()[:2])
    try:
        if kind == "history":
            parse_history_text(text, side)
        else:
            parse_sync_text(text)
    except Invalid as error:
        return "validation", str(error)
    return "valid", "log validates on its own"


# ---------------------------------------------------------------- runner


class Runner:
    def __init__(self, args):
        self.out = Path(args.out).resolve()
        self.fixture_root = Path(args.fixture_root).resolve()
        self.bin_dir = Path(args.bin_dir).resolve()
        self.fixtures = {
            "master": self.fixture_root / args.master_fixture,
            "branch": self.fixture_root / args.branch_fixture,
        }
        self.bins = {(kind, side): binary_for(self.bin_dir, side, kind) for kind in KINDS for side in SIDES}
        self.load_host = args.load_host
        self.retry_once = args.retry_side_once
        self.timeout = args.timeout
        self.min_free = int(args.min_free_gib * 2**30)
        self.argv = list(sys.argv)
        self.lock = None
        self.bin_sha: dict[str, str] = {}
        self.fp: dict[str, dict] = {}

    def prepare(self, pairs: list[int]) -> None:
        for side, fixture in self.fixtures.items():
            if not fixture.is_dir() or fixture.is_symlink() or not (fixture / ".vault").is_dir():
                raise Fatal(f"{side} fixture {fixture} is missing, is a link, or has no .vault")
            if overlaps(self.out, fixture):
                raise Fatal(f"output {self.out} overlaps fixture {fixture}")
        if overlaps(self.out, self.bin_dir) or overlaps(self.out, self.fixture_root):
            raise Fatal(f"output {self.out} overlaps the fixture or binary directories")
        for binary in self.bins.values():
            if not binary.is_file() or binary.is_symlink() or not os.access(binary, os.X_OK):
                raise Fatal(f"{binary} must be an executable regular file")
        self.out.mkdir(parents=True, exist_ok=True)
        self.lock = open(self.out / ".cold-ab.lock", "a")
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise Fatal(f"another cold_ab run holds {self.out}/.cold-ab.lock") from error
        self.check_labels_free(pairs)
        self.bin_sha = {str(path): sha256(path) for path in self.bins.values()}
        for side, fixture in self.fixtures.items():
            self.fp[side] = fingerprint(fixture)
            if self.fp[side]["symlinks"]:
                raise Fatal(f"fixture {fixture} contains links; refusing to copy it")
        self.write_provenance(pairs)

    def check_labels_free(self, pairs: list[int]) -> None:
        for pair in pairs:
            names = [f"pair-{pair}.load-before", f"pair-{pair}.load-after", f"pair-{pair}.complete.json"]
            for kind in KINDS:
                for side in SIDES:
                    label = label_for(pair, side, kind)
                    names += [label + s for s in ("", ".edit", ".log", ".load-before", ".load-after")]
            for name in names:
                if os.path.lexists(self.out / name):
                    raise Fatal(f"{name} already exists in {self.out}; choose another --out or other pairs")

    def write_provenance(self, pairs: list[int]) -> None:
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        payload = {
            "started_utc": iso_now(),
            "pairs": pairs,
            "argv": self.argv,
            "host": platform.node(),
            "platform": platform.platform(),
            "python": sys.version.split()[0],
            "load_host": self.load_host,
            "retry_side_once": self.retry_once,
            "timeout_s": self.timeout,
            "pages": PAGES,
            "big_blocks": BIG_BLOCKS,
            "fixtures": {side: {"path": str(path), "fingerprint": self.fp[side]}
                         for side, path in self.fixtures.items()},
            "binaries": self.bin_sha,
        }
        (self.out / f"runner-{stamp}.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    def check_fixture(self, side: str) -> None:
        if fingerprint(self.fixtures[side]) != self.fp[side]:
            raise Fatal(f"fixture {self.fixtures[side]} changed; the original fixtures must stay unchanged")

    def check_binary(self, binary: Path) -> None:
        if sha256(binary) != self.bin_sha[str(binary)]:
            raise Fatal(f"{binary} changed since the run started")

    def record(self, entry: dict) -> None:
        with open(self.out / "runs.jsonl", "a", encoding="utf-8") as stream:
            stream.write(json.dumps(entry, sort_keys=True) + "\n")

    def next_failed(self, label: str) -> int:
        number = 1
        suffixes = ("", ".edit", ".log", ".json", ".load-before", ".load-after")
        while any(os.path.lexists(self.out / f"{label}.failed-{number}{s}") for s in suffixes):
            number += 1
        return number

    def run_side(self, pair: int, kind: str, side: str) -> None:
        label = label_for(pair, side, kind)
        for number in range(1, 3 if self.retry_once else 2):
            if self.attempt(pair, kind, side, number):
                return
        raise PairStopped(f"{label} failed and is kept as {label}.failed-*; pair {pair} cannot complete")

    def attempt(self, pair: int, kind: str, side: str, number: int) -> bool:
        out = self.out
        label = label_for(pair, side, kind)
        fixture = self.fixtures[side]
        binary = self.bins[(kind, side)]
        work, edit, log = out / label, out / (label + ".edit"), out / (label + ".log")
        self.check_fixture(side)
        self.check_binary(binary)
        if shutil.disk_usage(out).free < self.min_free:
            raise Fatal(f"less than {self.min_free / 2**30:.1f} GiB free on {out}")
        capture_load(out / f"{label}.load-before", self.load_host)
        started, began = iso_now(), time.monotonic()
        if kind == "history":
            copy_tree(fixture, work)
            copied = work
            command = [str(binary), "--history", str(work)]
        else:
            work.mkdir()
            copy_tree(fixture, work / "hub")
            copied = work / "hub"
            command = [str(binary), str(work), str(PAGES), str(BIG_BLOCKS)]
        if fingerprint(copied) != self.fp[side]:
            raise Fatal(f"the copy of {fixture} does not match the original")
        exit_code, timed_out = None, False
        with open(log, "wb") as stream:
            try:
                exit_code = subprocess.run(
                    command, cwd=out, stdin=subprocess.DEVNULL, stdout=stream,
                    stderr=subprocess.STDOUT, timeout=self.timeout,
                ).returncode
            except subprocess.TimeoutExpired:
                timed_out = True
        wall = time.monotonic() - began
        capture_load(out / f"{label}.load-after", self.load_host)
        self.check_fixture(side)
        self.check_binary(binary)
        problem, detail = None, None
        if timed_out:
            problem, detail = "timeout", f"no exit within {self.timeout} s"
        elif exit_code != 0:
            problem, detail = "nonzero-exit", f"exit status {exit_code}"
        else:
            try:
                analyze_run(out, label, kind, side)
            except Invalid as error:
                problem, detail = "validation", str(error)
        if problem is not None and LOCK_TIMEOUT in read_text(log):
            problem, detail = "known-flush-lock-timeout", "owned separately; fresh retry only with --retry-side-once"
        entry = {
            "pair": pair, "kind": kind, "side": side, "label": label, "attempt": number,
            "status": "ok" if problem is None else "failed",
            "failure_class": problem, "reason": detail,
            "exit_code": exit_code, "timed_out": timed_out,
            "command": command, "binary": str(binary), "binary_sha256": self.bin_sha[str(binary)],
            "fixture": str(fixture), "fixture_fingerprint": self.fp[side],
            "started_utc": started, "wall_seconds": round(wall, 1),
        }
        if problem is None:
            remove_owned(out, label)
            remove_owned(out, label + ".edit")
            self.record(entry)
            print(f"{label}: ok in {wall:.0f} s (attempt {number})", flush=True)
            return True
        keep = self.next_failed(label)
        suffix = f".failed-{keep}"
        move_if_exists(work, out / (label + suffix))
        move_if_exists(edit, out / (label + suffix + ".edit"))
        move_if_exists(log, out / (label + suffix + ".log"))
        for phase in ("load-before", "load-after"):
            move_if_exists(out / f"{label}.{phase}", out / f"{label}{suffix}.{phase}")
        self.record(entry)
        (out / (label + suffix + ".json")).write_text(json.dumps(entry, indent=2) + "\n", encoding="utf-8")
        print(f"{label}: FAILED ({problem}: {detail}); kept as {label}{suffix}", file=sys.stderr, flush=True)
        return False

    def run_pair(self, pair: int) -> None:
        order = order_for(pair)
        capture_load(self.out / f"pair-{pair}.load-before", self.load_host)
        for kind in KINDS:
            for side in order:
                self.run_side(pair, kind, side)
        capture_load(self.out / f"pair-{pair}.load-after", self.load_host)
        try:
            analyze_pair(self.out, pair)
        except Invalid as error:
            raise Fatal(f"pair {pair} ran to the end but does not validate as a pair: {error}") from error
        complete = {"pair": pair, "order": list(order), "status": "complete", "finished_utc": iso_now()}
        (self.out / f"pair-{pair}.complete.json").write_text(json.dumps(complete, indent=2) + "\n", encoding="utf-8")
        print(f"pair {pair}: complete ({' then '.join(order)})", flush=True)

    def finish(self) -> None:
        for path, digest in self.bin_sha.items():
            if sha256(Path(path)) != digest:
                raise Fatal(f"{path} changed during the run")
        for side in SIDES:
            self.check_fixture(side)


def plan_lines(args, pairs: list[int]) -> list[str]:
    out = Path(args.out).resolve()
    bin_dir = Path(args.bin_dir).resolve()
    fixture_root = Path(args.fixture_root).resolve()
    fixtures = {"master": fixture_root / args.master_fixture, "branch": fixture_root / args.branch_fixture}
    lines = [f"output: {out}", f"fixtures: master={fixtures['master']} branch={fixtures['branch']}",
             f"load: ssh {args.load_host} uptime, else /proc/loadavg" if args.load_host else "load: /proc/loadavg",
             "retry: one whole side-run" if args.retry_side_once else "retry: off; a failed side stops the run"]
    for pair in pairs:
        order = order_for(pair)
        lines.append(f"pair {pair}: {' then '.join(order)}; history phase, then sync phase")
        for kind in KINDS:
            for side in order:
                label = label_for(pair, side, kind)
                binary = binary_for(bin_dir, side, kind)
                if kind == "history":
                    lines.append(f"  {label}: copy {fixtures[side]} -> {out / label}; "
                                 f"{binary} --history {out / label} > {label}.log")
                else:
                    lines.append(f"  {label}: copy {fixtures[side]} -> {out / label / 'hub'}; "
                                 f"{binary} {out / label} {PAGES} {BIG_BLOCKS} > {label}.log")
    return lines


def cmd_run(args) -> int:
    pairs = parse_pairs(args.pairs)
    if args.dry_run:
        print("\n".join(plan_lines(args, pairs)))
        return 0
    runner = Runner(args)
    runner.prepare(pairs)
    for pair in pairs:
        runner.run_pair(pair)
    runner.finish()
    print(f"done: pairs {pairs} complete in {runner.out}")
    return 0


# ---------------------------------------------------------------- report


def summarize_metric(included: list[dict], getter) -> dict:
    per_side = {side: [getter(item, side) for item in included] for side in SIDES}
    master, branch = per_side["master"], per_side["branch"]
    master_median, branch_median = statistics.median(master), statistics.median(branch)
    return {
        "master_median": master_median,
        "branch_median": branch_median,
        "branch_over_master": branch_median / master_median if master_median else None,
        "master_min": min(master),
        "master_max": max(master),
        "branch_min": min(branch),
        "branch_max": max(branch),
        "pairs": [item["pair"] for item in included],
        "master_by_pair": master,
        "branch_by_pair": branch,
        "branch_over_master_by_pair": [b / m if m else None for m, b in zip(master, branch)],
    }


FIELD_GETTERS = {
    "full export time (ms)": lambda it, s: it["runs"][s]["history"]["export_ms"],
    "full export bytes": lambda it, s: it["runs"][s]["history"]["export_bytes"],
    "HTTP full sync time (s)": lambda it, s: it["runs"][s]["sync"]["http_s"],
    "HTTP bytes down (MB, bench prints 0.1 MB)": lambda it, s: it["runs"][s]["sync"]["down_mb"],
    "HTTP bytes up (MB, bench prints 0.01 MB)": lambda it, s: it["runs"][s]["sync"]["up_mb"],
}

METHOD = {
    "aggregate": "per side: median over the complete pairs of each run's reported p50 (latency), "
                 "or of each run's total (full export, HTTP full sync)",
    "ratio": "branch / master of the two medians; per-pair branch / master is also listed",
    "pairs": "only complete pairs: all four side-runs validate and agree on page counts",
    "validation": "completion markers present, zero history version and old-view mismatches, "
                  "sync lines in order, no panic, fixture state per side (master hot-only, branch with cold files)",
    "claims": "no stability or significance verdict; medians of five values from relative A/B runs",
}


def check_consistency(included: list[dict]) -> None:
    first = included[0]
    for item in included:
        for side in SIDES:
            run = item["runs"][side]["history"]
            base = first["runs"][side]["history"]
            if (run["samples"], run["big_samples"], run["pages"]) != (base["samples"], base["big_samples"], base["pages"]):
                raise Fatal(f"pair {item['pair']} {side}: sample or page counts differ from pair {first['pair']}")
            if run["files"] != base["files"] or run["bytes"] != base["bytes"]:
                raise Fatal(f"pair {item['pair']} {side}: fixture file counts or bytes differ; the copies are not identical")


def binary_info(bin_dir: Path) -> dict:
    info = {}
    for name in ("master-bench", "branch-bench", "master-sync", "branch-sync"):
        path = bin_dir / name
        try:
            info[name] = {"path": str(path), "sha256": sha256(path), "bytes": path.stat().st_size,
                          "mtime_utc": iso_at(path.stat().st_mtime)}
        except OSError as error:
            info[name] = {"path": str(path), "error": str(error)}
    return info


def boundary_rows(included: list[dict]) -> list[dict]:
    rows = []
    for item in included:
        root = Path(item["root"])
        for edge in ("before", "after"):
            path = root / f"pair-{item['pair']}.load-{edge}"
            if path.exists():
                reading = read_load(path)
                rows.append({"pair": item["pair"], "edge": edge,
                             "load_1_5_15": reading["load_1_5_15"], "source": reading["source"]})
    return rows


def load_rows(included: list[dict]) -> list[dict]:
    rows = []
    for item in included:
        for side in SIDES:
            for kind in KINDS:
                run = item["runs"][side][kind]
                rows.append({
                    "pair": item["pair"], "side": side, "phase": kind, "label": run["label"],
                    "start": run["load_before"]["load_1_5_15"], "start_source": run["load_before"]["source"],
                    "end": run["load_after"]["load_1_5_15"], "end_source": run["load_after"]["source"],
                })
    return rows


def retried_runs(included: list[dict]) -> list[dict]:
    rows = []
    for item in included:
        root = Path(item["root"])
        for side in SIDES:
            for kind in KINDS:
                label = item["runs"][side][kind]["label"]
                retained = sorted(p.name for p in root.glob(f"{label}.failed-*.log"))
                if retained:
                    rows.append({"pair": item["pair"], "label": label, "retained_failures": retained})
    return rows


def scan_attempts(sources, included: list[dict], excluded: list[dict]) -> list[dict]:
    kept = {(item["root"], item["pair"]) for item in included}
    reasons = {(row["root"], row["pair"]): row["reason"] for row in excluded}
    rows = []
    for root, _pairs in sources:
        for path in sorted(root.glob("aged-*.log")):
            name = path.name[: -len(".log")]
            match = LABEL.match(name)
            if match is None:
                continue
            side, sync, pair, failed = match.group(1), match.group(2), int(match.group(3)), match.group(4)
            if failed is None and (str(root), pair) in kept:
                continue
            kind = "sync" if sync else "history"
            try:
                meta_path = root / f"{name}.json"
                if failed is not None and meta_path.exists():
                    meta = json.loads(meta_path.read_text(encoding="utf-8"))
                    failure_class, detail = meta["failure_class"], meta["reason"]
                else:
                    failure_class, detail = classify(read_text(path), kind, side)
            except (Invalid, ValueError, KeyError) as error:
                failure_class, detail = "unreadable", str(error)
            if failed is not None:
                status = "failed, retained"
            elif failure_class == "valid":
                status = "valid run, pair not in report"
            else:
                status = "failed, not in report"
            rows.append({
                "source": root.name, "label": name, "kind": kind, "side": side,
                "status": status,
                "failure_class": failure_class, "detail": detail,
                "pair_note": reasons.get((str(root), pair), ""),
            })
    return rows


def build_report(included, excluded, sources, bin_dir: Path) -> dict:
    first = included[0]["runs"]
    latency = {
        name: summarize_metric(included, lambda it, s, n=name: it["runs"][s]["history"]["metrics"][n])
        for name in LATENCIES
    }
    big = {
        name: summarize_metric(included, lambda it, s, n=name: it["runs"][s]["history"]["metrics"][n])
        for name in BIG_LATENCIES
    }
    fields = {label: summarize_metric(included, getter) for label, getter in FIELD_GETTERS.items()}
    attempts = scan_attempts(sources, included, excluded)
    return {
        "status": "complete",
        "generated_utc": iso_now(),
        "pairs": [item["pair"] for item in included],
        "pair_sources": [{"root": str(root), "pairs": pairs} for root, pairs in sources],
        "method": METHOD,
        "setup": {
            "pages": PAGES,
            "big_blocks": BIG_BLOCKS,
            "samples_per_history_run": first["master"]["history"]["samples"],
            "big_samples_per_history_run": first["master"]["history"]["big_samples"],
            "fixture_state": {side: {"files": first[side]["history"]["files"],
                                     "bytes": first[side]["history"]["bytes"],
                                     "identical_across_pairs": True} for side in SIDES},
        },
        "normal_pages_ms": latency,
        "big_page_ms": big,
        "export_and_http": fields,
        "loads": load_rows(included),
        "pair_boundary_loads": boundary_rows(included),
        "retried_runs": retried_runs(included),
        "failed_attempts": attempts,
        "excluded_pairs": excluded,
        "binaries": binary_info(bin_dir),
        "runs": {str(item["pair"]): {side: {kind: item["runs"][side][kind] for kind in KINDS}
                                     for side in SIDES} for item in included},
    }


def num(value, digits: int = 3) -> str:
    return "n/a" if value is None else f"{value:,.{digits}f}"


def span(low, high, digits: int = 3) -> str:
    return f"{num(low, digits)} - {num(high, digits)}"


def table(headers: list[str], rows: list[list]) -> list[str]:
    lines = ["| " + " | ".join(headers) + " |", "| " + " | ".join("---" for _ in headers) + " |"]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return lines


def summary_rows(metrics: dict, units: dict, digits_for) -> list[list]:
    rows = []
    for name, entry in metrics.items():
        digits = digits_for(name)
        rows.append([
            name, units.get(name, "ms"),
            num(entry["master_median"], digits), num(entry["branch_median"], digits),
            num(entry["branch_over_master"], 3),
            span(entry["master_min"], entry["master_max"], digits),
            span(entry["branch_min"], entry["branch_max"], digits),
        ])
    return rows


def raw_rows(metrics: dict, digits_for) -> list[list]:
    rows = []
    for name, entry in metrics.items():
        digits = digits_for(name)
        rows.append([name, "master", *[num(v, digits) for v in entry["master_by_pair"]]])
        rows.append(["", "branch", *[num(v, digits) for v in entry["branch_by_pair"]]])
        rows.append(["", "branch / master", *[num(v, 3) for v in entry["branch_over_master_by_pair"]]])
    return rows


def render_markdown(report: dict) -> str:
    lines: list[str] = []
    add = lines.append
    pairs = report["pairs"]
    pair_headers = [f"P{p}" for p in pairs]
    add("# Cold-history A/B: five completed pairs")
    add("")
    add(f"Status: **{report['status']}**. Generated {report['generated_utc']}.")
    add("")
    for src in report["pair_sources"]:
        add(f"- Source `{Path(src['root']).name}`: pairs {', '.join(str(p) for p in src['pairs'])}")
    add("")
    add("Pair order: odd pairs run master then branch, even pairs branch then master. New pairs 3-5 run the "
        "history phase for both sides, then the sync phase for both sides.")
    add("")
    state = report["setup"]["fixture_state"]
    add("## Setup")
    add("")
    add(f"- History run: {report['setup']['samples_per_history_run'] - report['setup']['big_samples_per_history_run']} "
        f"normal samples and {report['setup']['big_samples_per_history_run']} big-page sample, "
        f"{report['setup']['pages']:,} total pages including one {report['setup']['big_blocks']:,}-block page.")
    add("- Sync run: `sync_bench <work> 10000 2000` on a copy of the fixture as the hub; HTTP full sync into an empty vault.")
    for side in SIDES:
        f = state[side]["files"]
        b = state[side]["bytes"]
        add(f"- {side} fixture state (identical in all five pairs): hot {f['hot']:,} files / {b['hot']:,} bytes; "
            f"cold {f['cold']:,} files / {b['cold']:,} bytes; dictionary {f['dict']} file / {b['dict']:,} bytes.")
    add("")
    add("Binaries (pinned default, non-instrumented):")
    add("")
    lines += table(["Binary", "sha256", "bytes", "mtime (UTC)"],
                   [[name, info.get("sha256", info.get("error")), f"{info.get('bytes', 0):,}",
                     info.get("mtime_utc", "")] for name, info in report["binaries"].items()])
    add("")
    add("## Normal pages (ms): median over pairs of each run's reported p50")
    add("")
    lines += table(["Metric", "Unit", "Master median", "Branch median", "Branch / master",
                    "Master range", "Branch range"],
                   summary_rows(report["normal_pages_ms"], {}, lambda name: 3))
    add("")
    add("## Big page, 2,000 blocks (ms): median over pairs of each run's reported p50")
    add("")
    lines += table(["Metric", "Unit", "Master median", "Branch median", "Branch / master",
                    "Master range", "Branch range"],
                   summary_rows(report["big_page_ms"], {}, lambda name: 3))
    add("")
    add("## Full export and HTTP sync: totals per run, median over pairs")
    add("")
    units = {"full export time (ms)": "ms", "full export bytes": "bytes",
             "HTTP full sync time (s)": "s", "HTTP bytes down (MB, bench prints 0.1 MB)": "MB",
             "HTTP bytes up (MB, bench prints 0.01 MB)": "MB"}
    lines += table(["Metric", "Unit", "Master median", "Branch median", "Branch / master",
                    "Master range", "Branch range"],
                   summary_rows(report["export_and_http"], units,
                                lambda name: 0 if "bytes" in name and "MB" not in name else (2 if "(s)" in name else 3)))
    add("")
    add("## Raw pairs (not medians)")
    add("")
    add("Normal pages (ms)")
    add("")
    lines += table(["Metric", "Side", *pair_headers],
                   raw_rows(report["normal_pages_ms"], lambda name: 3))
    add("")
    add("Big page (ms)")
    add("")
    lines += table(["Metric", "Side", *pair_headers], raw_rows(report["big_page_ms"], lambda name: 3))
    add("")
    add("Full export and HTTP sync")
    add("")
    lines += table(["Metric", "Side", *pair_headers],
                   raw_rows(report["export_and_http"],
                            lambda name: 0 if "bytes" in name and "MB" not in name else (2 if "(s)" in name else 3)))
    add("")
    add("## Load (1 / 5 / 15 minute, raw readings)")
    add("")
    add("Each row is one side-run. Start and end readings are taken around the run. Source shows whether the "
        "reading came from `ssh` to the home server or from this runner's /proc/loadavg.")
    add("")
    lines += table(["Pair", "Side", "Phase", "Run", "Start 1/5/15", "Start source", "End 1/5/15", "End source"],
                   [[row["pair"], row["side"], row["phase"], row["label"],
                     " / ".join(f"{v:.2f}" for v in row["start"]), row["start_source"],
                     " / ".join(f"{v:.2f}" for v in row["end"]), row["end_source"]]
                    for row in report["loads"]])
    add("")
    if report["pair_boundary_loads"]:
        add("Pair boundary readings:")
        add("")
        lines += table(["Pair", "Edge", "1/5/15", "Source"],
                       [[row["pair"], row["edge"], " / ".join(f"{v:.2f}" for v in row["load_1_5_15"]),
                         row["source"]] for row in report["pair_boundary_loads"]])
        add("")
    sources = collections.Counter(
        src for row in report["loads"] for src in (row["start_source"], row["end_source"])
    )
    add("Reading sources: " + "; ".join(f"{count} x {src}" for src, count in sorted(sources.items())) + ".")
    add("")
    add("## Retained failures and exclusions")
    add("")
    known = [row for row in report["failed_attempts"] if row["failure_class"] == "known-flush-lock-timeout"]
    add(f"Known flush-lock timeouts listed here: {len(known)}. They are owned separately; their failed attempts "
        "remain separate from the selected successful pairs.")
    add("")
    if report["failed_attempts"]:
        lines += table(["Source", "Run", "Kind", "Side", "Status", "Failure class", "Detail"],
                       [[row["source"], row["label"], row["kind"], row["side"], row["status"],
                         row["failure_class"], row["detail"]] for row in report["failed_attempts"]])
    else:
        add("No failed or excluded runs were found in the sources.")
    add("")
    if report["retried_runs"]:
        add("Runs that were retried (a failed attempt is kept beside the successful one):")
        add("")
        lines += table(["Pair", "Successful run", "Retained failures"],
                       [[row["pair"], row["label"], ", ".join(row["retained_failures"])]
                        for row in report["retried_runs"]])
        add("")
    if report["excluded_pairs"]:
        lines += table(["Source", "Pair", "Reason"],
                       [[Path(row["root"]).name, row["pair"], row["reason"]] for row in report["excluded_pairs"]])
        add("")
    add("## Caveats")
    add("")
    add("- Five pairs is the minimum. Each median is over five values, with no confidence interval, and no "
        "stability or significance verdict is claimed.")
    add("- Master and branch do not read the same on-disk state. Master reads the hot-only fixture; branch reads "
        "the fixture with cold files. The ratios compare those two states, as the v4 pairs did.")
    add("- Cold means a freshly opened vault in the bench process. OS page caches were not dropped.")
    add("- HTTP byte counts are the bench's printed MB values (0.1 MB precision for down, 0.01 MB for up). "
        "Full export bytes are exact.")
    add("- Load readings have no threshold applied. Each reading names its source.")
    if any(Path(source["root"]).resolve() == Path(ROOT_V4).resolve()
           for source in report["pair_sources"]):
        add("- Historical v4 pair 2 master sync is the fresh retry after a failed attempt; the failed attempt was kept.")
        add("- The v4 pair 3 master history completed, but its sync failed and no branch pair 3 exists. v4 pair 3 "
            "is not in this report.")
    add("")
    return "\n".join(lines)


def cmd_report(args) -> int:
    specs = args.source or [f"{ROOT_V4}=1,2", f"{ROOT_V5}=3,4,5"]
    sources = []
    for spec in specs:
        root, _, pairs = spec.rpartition("=")
        if not root or not pairs:
            raise Fatal(f"--source expects ROOT=PAIRS, got {spec!r}")
        sources.append((Path(root).resolve(), parse_pairs(pairs)))
    included, excluded = [], []
    for root, pairs in sources:
        for pair in pairs:
            try:
                included.append(analyze_pair(root, pair))
            except Invalid as error:
                excluded.append({"root": str(root), "pair": pair, "reason": str(error)})
    included.sort(key=lambda item: item["pair"])
    numbers = [item["pair"] for item in included]
    if len(set(numbers)) != len(numbers):
        raise Fatal(f"a pair number appears in more than one source: {numbers}")
    if len(included) < MIN_PAIRS:
        for row in excluded:
            print(f"excluded {row['root']} pair {row['pair']}: {row['reason']}", file=sys.stderr)
        raise Fatal(f"{len(included)} complete pairs; at least {MIN_PAIRS} are required, so no report was written")
    check_consistency(included)
    report = build_report(included, excluded, sources, Path(args.bin_dir).resolve())
    out_dir = Path(args.out_dir) if args.out_dir else Path(ROOT_V5) / "report"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "cold-ab-5pairs.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    (out_dir / "cold-ab-5pairs.md").write_text(render_markdown(report), encoding="utf-8")
    print(f"wrote {out_dir / 'cold-ab-5pairs.md'} and cold-ab-5pairs.json; pairs {numbers}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="run pairs serially on fresh fixture copies")
    run.add_argument("--pairs", required=True, help="pair numbers, e.g. 3 or 3,4,5")
    run.add_argument("--fixture-root", default=ROOT_V3)
    run.add_argument("--master-fixture", default="aged-hot", help="fixture directory for master runs")
    run.add_argument("--branch-fixture", default="aged-cold", help="fixture directory for branch runs")
    run.add_argument("--bin-dir", default=f"{ROOT_V4}/bin")
    run.add_argument("--out", default=ROOT_V5)
    run.add_argument("--load-host", default="home-server",
                     help='ssh alias for uptime; "" reads /proc/loadavg only')
    run.add_argument("--retry-side-once", action="store_true",
                     help="rerun a failed side once with a fresh copy (the failure stays on disk)")
    run.add_argument("--timeout", type=int, default=3600, help="seconds per benchmark process")
    run.add_argument("--min-free-gib", type=float, default=4.0)
    run.add_argument("--dry-run", action="store_true", help="print the plan and exit")
    run.set_defaults(func=cmd_run)

    report = sub.add_parser("report", help="aggregate complete pairs (at least five)")
    report.add_argument("--source", action="append", metavar="ROOT=PAIRS",
                        help=f"repeatable; default {ROOT_V4}=1,2 and {ROOT_V5}=3,4,5")
    report.add_argument("--bin-dir", default=f"{ROOT_V4}/bin", help="for the binary checksums in the report")
    report.add_argument("--out-dir", default=None, help=f"default {ROOT_V5}/report")
    report.set_defaults(func=cmd_report)
    return parser


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except Fatal as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    except PairStopped as error:
        print(f"stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
