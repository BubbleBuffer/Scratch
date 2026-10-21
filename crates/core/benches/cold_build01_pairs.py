#!/usr/bin/env python3
"""Run cold_ab.py pairs one at a time on build01, with home-server load read from this PC.

Run from the Windows PC (stdlib only):
  python crates/core/benches/cold_build01_pairs.py --remote-root ROOT [--pairs 1,2,3,4,5]
      [--evidence-dir DIR]

ROOT is an absolute build01 directory holding notes-app/ (the checkout), fixtures/ and bin/.
The orchestrator creates ROOT/pairs, which must not exist yet. For each pair N, in order:
  1. ssh.exe home-server uptime, saved locally with a source line, then scp.exe to
     ROOT/pairs/pair-N.host-load-before
  2. ssh.exe build01 timeout --kill-after=10 1150 python3 ROOT/notes-app/crates/core/benches/cold_ab.py
     run --pairs N --fixture-root ROOT/fixtures --bin-dir ROOT/bin --out ROOT/pairs --load-host ''
     build01 timeout sends TERM at 1150 s and KILL 10 s later to its process group (the harness and
     the benchmark processes it started; no --foreground). cold_ab.py keeps its own per-benchmark
     --timeout (default 3600 s). The 1200 s local ssh.exe limit is only a backstop.
  3. ssh.exe home-server uptime again, then scp.exe to ROOT/pairs/pair-N.host-load-after
  4. on build01: the harness's pair-N.load-before/after (runner readings) move to
     pair-N.runner-load-before/after, and the host readings are copied to
     pair-N.load-before/after and compared with cmp.
After the last pair: cold_ab.py report over ROOT/pairs into ROOT/report, also under the build01
timeout.

Nothing is retried, and a runner reading is never presented as a home-server reading. The first
failed step stops the run and keeps the evidence. timeout's exit codes: 124 means TERM was sent at
the limit, 137 means KILL was needed. The pair's home-server reading stays owed (end_due) until its
after-reading and upload both succeed. If the harness run fails, or the after-reading or its upload
fails, the finally block makes one more home-server reading attempt and keeps it as
pair-N.host-load-after only. That pair is not published: its runner readings stay under the harness
names, and their source line says fallback. The report runs only when every requested pair
completed. Evidence and provenance go to the evidence directory on the PC (a new temp directory
unless --evidence-dir is given).
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import platform
import re
import shlex
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SSH_EXE = r"C:\Windows\System32\OpenSSH\ssh.exe"
SCP_EXE = r"C:\Windows\System32\OpenSSH\scp.exe"
BUILD_HOST = "build01"
LOAD_HOST = "home-server"
# Local limit on each ssh.exe call that runs a remote harness step. The remote step stops itself
# first (under_timeout below), so this limit is only a backstop for a hung ssh link.
PROCESS_TIMEOUT_S = 1200
# build01 GNU timeout around each remote cold_ab.py run and report: TERM at HARNESS_TERM_S, KILL
# KILL_AFTER_S later. Without --foreground, timeout signals its own process group, which holds the
# harness and the benchmark processes it started, so nothing is left running on build01.
HARNESS_TERM_S = 1150
KILL_AFTER_S = 10
QUICK_TIMEOUT_S = 120
if HARNESS_TERM_S + KILL_AFTER_S >= PROCESS_TIMEOUT_S:
    raise RuntimeError("the build01 timeout must end before the local ssh.exe timeout")
REPO_DIR = "notes-app"
HARNESS_REL = "crates/core/benches/cold_ab.py"
SSH_OPTS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=30"]
LOAD = re.compile(
    r"load average:\s*(\d+(?:\.\d*)?|\.\d+),\s*(\d+(?:\.\d*)?|\.\d+),\s*(\d+(?:\.\d*)?|\.\d+)"
)


class Stop(RuntimeError):
    """A step failed. The run stops here and the evidence stays on disk."""


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path: Path) -> str | None:
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def parse_pairs(text: str) -> list[int]:
    try:
        pairs = [int(part) for part in text.split(",")]
    except ValueError:
        pairs = []
    if not pairs or min(pairs) < 1 or len(set(pairs)) != len(pairs):
        raise argparse.ArgumentTypeError(f"expected distinct pair numbers >= 1, e.g. 1,2,3; got {text!r}")
    return pairs


def ok(record: dict) -> bool:
    return record["returncode"] == 0 and not record["timed_out"] and record["error"] is None


def under_timeout(argv: list[str]) -> list[str]:
    """Wrap a remote command in build01 GNU timeout: TERM at HARNESS_TERM_S, KILL KILL_AFTER_S later."""
    return ["timeout", f"--kill-after={KILL_AFTER_S}", str(HARNESS_TERM_S), *argv]


def describe(record: dict) -> str:
    notes = {124: " (build01 timeout sent TERM)", 137: " (build01 timeout needed KILL)"}
    rc = record["returncode"]
    return (f"rc {rc}{notes.get(rc, '')}, timed_out {record['timed_out']}, "
            f"error {record['error']}")


class Commands:
    """Runs local commands. Each one's stdout and stderr go to files in the evidence directory."""

    def __init__(self, evidence: Path):
        self.evidence = evidence
        self.records: list[dict] = []

    def run(self, name: str, argv: list[str], timeout: int) -> dict:
        stdout, stderr = self.evidence / f"{name}.stdout.txt", self.evidence / f"{name}.stderr.txt"
        record = {"name": name, "argv": argv, "timeout_s": timeout, "started_utc": utc_now(),
                  "returncode": None, "timed_out": False, "error": None,
                  "stdout": stdout.name, "stderr": stderr.name}
        began = time.monotonic()
        with open(stdout, "wb") as out, open(stderr, "wb") as err:
            try:
                record["returncode"] = subprocess.run(
                    argv, stdin=subprocess.DEVNULL, stdout=out, stderr=err, timeout=timeout
                ).returncode
            except subprocess.TimeoutExpired:
                record["timed_out"] = True
            except OSError as error:
                record["error"] = str(error)
        record["wall_s"] = round(time.monotonic() - began, 1)
        record["ended_utc"] = utc_now()
        self.records.append(record)
        return record

    def text(self, record: dict) -> str:
        return (self.evidence / record["stdout"]).read_text(encoding="utf-8", errors="replace")


class Orchestrator:
    def __init__(self, root: str, pairs: list[int], evidence: Path):
        self.root = root
        self.pairs = pairs
        self.evidence = evidence
        self.commands = Commands(evidence)
        self.started = utc_now()
        self.pairs_dir = f"{root}/pairs"
        self.harness = f"{root}/{REPO_DIR}/{HARNESS_REL}"
        self.fixtures = f"{root}/fixtures"
        self.bins = f"{root}/bin"
        self.report_dir = f"{root}/report"
        self.preflighted = False
        self.revision: str | None = None
        self.harness_sha256: str | None = None
        self.done: list[int] = []
        self.end_due: int | None = None
        self.report_ok = False
        self.status = "running"
        self.stop_reason: str | None = None
        self.errors: list[str] = []

    def ssh_build(self, name: str, remote: list[str], timeout: int) -> dict:
        command = " ".join(shlex.quote(part) for part in remote)
        return self.commands.run(name, [SSH_EXE, *SSH_OPTS, BUILD_HOST, command], timeout)

    def scp_up(self, name: str, local: Path, remote: str) -> None:
        record = self.commands.run(
            name, [SCP_EXE, *SSH_OPTS, str(local), f"{BUILD_HOST}:{shlex.quote(remote)}"], QUICK_TIMEOUT_S
        )
        if not ok(record):
            raise Stop(f"{name}: scp.exe to {remote} failed (rc {record['returncode']}, error {record['error']})")

    def scp_down(self, name: str, remote: str, local: Path) -> None:
        record = self.commands.run(
            name, [SCP_EXE, *SSH_OPTS, f"{BUILD_HOST}:{shlex.quote(remote)}", str(local)], QUICK_TIMEOUT_S
        )
        if not ok(record):
            raise Stop(f"{name}: scp.exe from {remote} failed (rc {record['returncode']}, error {record['error']})")

    def home_load(self, tag: str) -> Path:
        """One reading from the PC: ssh.exe home-server uptime, kept with a source line."""
        record = self.commands.run(f"{tag}.uptime", [SSH_EXE, *SSH_OPTS, LOAD_HOST, "uptime"], QUICK_TIMEOUT_S)
        text = self.commands.text(record)
        if not ok(record) or len(LOAD.findall(text)) != 1:
            raise Stop(f"{tag}: no load reading from {LOAD_HOST} (rc {record['returncode']}, "
                       f"timed_out {record['timed_out']}, error {record['error']}); see {tag}.uptime.stdout.txt")
        local = self.evidence / f"{tag}.txt"
        local.write_text(f"source: ssh {LOAD_HOST} uptime, run from the Windows PC via ssh.exe\n"
                         f"{text.strip()}\n", encoding="utf-8")
        return local

    def preflight(self) -> None:
        for exe in (SSH_EXE, SCP_EXE):
            if not Path(exe).is_file():
                raise Stop(f"{exe} is missing; nothing was run")
        repo = f"{self.root}/{REPO_DIR}"
        checks = [
            "command -v timeout >/dev/null",
            f"test -f {shlex.quote(self.harness)}",
            f"test -d {shlex.quote(self.fixtures)}",
            f"test -d {shlex.quote(self.bins)}",
            f"test ! -e {shlex.quote(self.pairs_dir)}",
            f"git -C {shlex.quote(repo)} rev-parse HEAD",
            f"sha256sum {shlex.quote(self.harness)}",
            f"mkdir {shlex.quote(self.pairs_dir)}",
        ]
        record = self.ssh_build("preflight", ["sh", "-c", " && ".join(checks)], QUICK_TIMEOUT_S)
        lines = self.commands.text(record).splitlines()
        if not ok(record) or len(lines) < 2 or not re.fullmatch(r"[0-9a-f]{40}", lines[0]) \
                or not re.fullmatch(r"[0-9a-f]{64}\s+\S.*", lines[1]):
            raise Stop(f"build01 preflight failed (rc {record['returncode']}, timed_out {record['timed_out']}, "
                       f"error {record['error']}); see preflight.stderr.txt; nothing was run")
        self.revision = lines[0]
        self.harness_sha256 = lines[1].split()[0]
        self.preflighted = True

    def publish_host_readings(self, pair: int) -> None:
        """Keep the harness's runner readings under runner- names, then put the host readings in their place."""
        steps = []
        for edge in ("before", "after"):
            harness = f"{self.pairs_dir}/pair-{pair}.load-{edge}"
            runner = f"{self.pairs_dir}/pair-{pair}.runner-load-{edge}"
            host = f"{self.pairs_dir}/pair-{pair}.host-load-{edge}"
            steps += [
                f"test ! -e {shlex.quote(runner)} && mv {shlex.quote(harness)} {shlex.quote(runner)}",
                f"test ! -e {shlex.quote(harness)} && cp {shlex.quote(host)} {shlex.quote(harness)} "
                f"&& cmp -s {shlex.quote(host)} {shlex.quote(harness)}",
            ]
        record = self.ssh_build(f"pair-{pair}.publish", ["sh", "-c", " && ".join(steps)], QUICK_TIMEOUT_S)
        if not ok(record):
            raise Stop(f"pair {pair}: moving the runner readings or copying the host readings failed "
                       f"(rc {record['returncode']}); evidence kept on {BUILD_HOST}; see pair-{pair}.publish.stderr.txt")

    def run_pair(self, pair: int) -> None:
        tag = f"pair-{pair}"
        before = self.home_load(f"{tag}.host-load-before")
        self.scp_up(f"{tag}.host-load-before.scp", before, f"{self.pairs_dir}/{tag}.host-load-before")
        # cold_ab.py keeps its own per-benchmark limit (--timeout, default 3600 s). The build01 timeout
        # around the whole run (HARNESS_TERM_S) is the pair's limit, and it fires before the local one.
        harness_argv = under_timeout(["python3", self.harness, "run", "--pairs", str(pair),
                                      "--fixture-root", self.fixtures, "--bin-dir", self.bins,
                                      "--out", self.pairs_dir, "--load-host", ""])
        # end_due stays set until the after-reading and its upload are both complete, so finally can
        # still make the home-server reading if the run or either of those steps fails.
        self.end_due = pair
        record = self.ssh_build(f"{tag}.harness-run", harness_argv, PROCESS_TIMEOUT_S)
        if not ok(record):
            raise Stop(f"pair {pair}: harness run failed ({describe(record)}); not retried; "
                       f"see {tag}.harness-run.stdout.txt and .stderr.txt")
        after = self.home_load(f"{tag}.host-load-after")
        self.scp_up(f"{tag}.host-load-after.scp", after, f"{self.pairs_dir}/{tag}.host-load-after")
        self.end_due = None
        self.publish_host_readings(pair)
        self.done.append(pair)
        print(f"pair {pair}: complete; host readings published as pair-{pair}.load-before/after", flush=True)

    def end_reading(self) -> None:
        """Runs in the finally block while end_due is set (harness run, after-reading or upload failed).

        One home-server reading, kept as evidence only. It is never published as the pair's load files.
        """
        pair = self.end_due
        tag = f"pair-{pair}.host-load-after"
        try:
            local = self.home_load(tag)
            self.scp_up(f"{tag}.end.scp", local, f"{self.pairs_dir}/{tag}")
        except Stop as error:
            self.errors.append(f"end reading for pair {pair} (not published as pair load): {error}")

    def make_report(self) -> None:
        spec = f"{self.pairs_dir}={','.join(str(pair) for pair in self.pairs)}"
        report_argv = under_timeout(["python3", self.harness, "report", "--source", spec,
                                     "--bin-dir", self.bins, "--out-dir", self.report_dir])
        record = self.ssh_build("report", report_argv, PROCESS_TIMEOUT_S)
        if not ok(record):
            raise Stop(f"report failed ({describe(record)}); see report.stderr.txt")
        self.scp_down("report.json.scp", f"{self.report_dir}/cold-ab-5pairs.json",
                      self.evidence / "cold-ab-5pairs.json")
        self.report_ok = True

    def run_all(self) -> None:
        self.preflight()
        for pair in self.pairs:
            self.run_pair(pair)
        self.make_report()

    def write_provenance(self) -> None:
        here = Path(__file__).resolve()
        local_harness = here.with_name("cold_ab.py")
        payload = {
            "status": self.status,
            "stop_reason": self.stop_reason,
            "errors": self.errors,
            "started_utc": self.started,
            "written_utc": utc_now(),
            "argv": sys.argv,
            "host": platform.node(),
            "python": sys.version.split()[0],
            "orchestrator": {"path": str(here), "sha256": sha256_file(here)},
            "pc_worktree_harness": {"path": str(local_harness), "sha256": sha256_file(local_harness)},
            "build_host": BUILD_HOST,
            "load_host": LOAD_HOST,
            "timeouts": {
                "local_ssh_process_s": PROCESS_TIMEOUT_S,
                "build01_timeout_term_s": HARNESS_TERM_S,
                "build01_timeout_kill_after_s": KILL_AFTER_S,
                "build01_timeout_wrapper": "GNU timeout without --foreground (signals its process group)",
                "cold_ab_per_benchmark": "cold_ab.py --timeout, default 3600 s, left at its default",
            },
            "remote_root": self.root,
            "pairs_requested": self.pairs,
            "pairs_completed": self.done,
            "remote_notes_app_head": self.revision,
            "remote_harness_sha256": self.harness_sha256,
            "report_written": self.report_ok,
            "binary_identity": "the harness records binary sha256 and fixture fingerprints in "
                               "<root>/pairs/runner-*.json; cold-ab-5pairs.json (copied back when the "
                               "report ran) lists the binaries under 'binaries'",
            "commands": self.commands.records,
        }
        (self.evidence / "provenance.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    def finish(self) -> None:
        self.write_provenance()
        if self.preflighted:
            try:
                self.scp_up("provenance.scp", self.evidence / "provenance.json",
                            f"{self.pairs_dir}/orchestrator-provenance.json")
            except Stop as error:
                self.errors.append(f"provenance copy to {BUILD_HOST}: {error}")
            self.write_provenance()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--remote-root", required=True,
                        help="absolute build01 directory with notes-app/, fixtures/ and bin/; "
                             "pairs/ is created here and must not exist yet")
    parser.add_argument("--pairs", type=parse_pairs, default="1,2,3,4,5",
                        help="pair numbers, run one after another in this order (default 1,2,3,4,5)")
    parser.add_argument("--evidence-dir",
                        help="new local directory for evidence and provenance (default: a new temp directory)")
    return parser


def main(argv=None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    root = args.remote_root.rstrip("/")
    if not root.startswith("/"):
        parser.error("--remote-root must be an absolute build01 path, e.g. /home/dev/<task-root>")
    if args.evidence_dir:
        evidence = Path(args.evidence_dir)
        try:
            evidence.mkdir(parents=True, exist_ok=False)
        except FileExistsError:
            parser.error(f"--evidence-dir {evidence} already exists; give a new directory")
    else:
        evidence = Path(tempfile.mkdtemp(prefix="cold-build01-pairs-"))
    orch = Orchestrator(root, args.pairs, evidence)
    print(f"evidence: {evidence}", flush=True)
    try:
        orch.run_all()
        orch.status = "complete"
    except Stop as error:
        orch.status = "stopped"
        orch.stop_reason = str(error)
        print(f"stopped: {error}", file=sys.stderr, flush=True)
    finally:
        if orch.end_due is not None:
            orch.end_reading()
        orch.finish()
    print(f"status: {orch.status}; pairs completed {orch.done}; "
          f"report {'written' if orch.report_ok else 'not written'}; evidence {evidence}")
    return 0 if orch.status == "complete" else 1


if __name__ == "__main__":
    sys.exit(main())
