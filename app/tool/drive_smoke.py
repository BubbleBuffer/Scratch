#!/usr/bin/env python3
"""Exercise a drive session using only its visible screen reading and CLI."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

import drive

ENTRY = re.compile(r'^\[(e\d+)\] (\w+) ("(?:\\.|[^"\\])*") @(-?[\d.]+),(-?[\d.]+) ([\d.]+)x([\d.]+)(.*)$')


def reading(output: str) -> list[dict]:
    entries = []
    for line in output.splitlines():
        match = ENTRY.match(line)
        if match:
            ref, role, label, x, y, w, h, state = match.groups()
            entries.append(dict(ref=ref, role=role, label=json.loads(label),
                                x=float(x), y=float(y), width=float(w), height=float(h), state=state))
    return entries


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    form = parser.add_mutually_exclusive_group()
    form.add_argument("--desktop", action="store_true")
    form.add_argument("--mobile", action="store_true")
    parser.add_argument("--dark", action="store_true")
    parser.add_argument("--vault", default="fresh")
    parser.add_argument("--now")
    parser.add_argument("--preserve-vault", type=Path,
                        help="copy the session vault here before stop, for copy-vault acceptance")
    args = parser.parse_args()
    timings = []
    owner = None
    begun = False

    def run(*words: str) -> str:
        started = time.monotonic()
        result = subprocess.run([sys.executable, str(Path(__file__).with_name("drive.py")), *words],
                                cwd=drive.APP.parent, capture_output=True, text=True,
                                encoding="utf-8", timeout=210 if words[0] == "start" else 65,
                                creationflags=drive.hidden_flags())
        elapsed = time.monotonic() - started
        timings.append((" ".join(words), elapsed))
        print(f"$ drive {' '.join(words)} [{elapsed:.3f}s]", flush=True)
        print(result.stdout.rstrip(), flush=True)
        if result.returncode:
            raise AssertionError(result.stdout.strip() or result.stderr.strip())
        return result.stdout

    def look(label: str) -> tuple[str, list[dict]]:
        output = run("look", "--label", label)
        entries = reading(output)
        assert entries, "look did not expose a screen reading"
        return output, entries

    def named(entries: list[dict], label: str) -> list[dict]:
        return [entry for entry in entries if label in entry["label"] and
                len(entry["label"]) < len(label) + 120]

    def focused_near(entries: list[dict], hint: str) -> bool:
        hints = [e for e in entries if hint.lower() in e["label"].lower()]
        fields = [e for e in entries if e["role"] == "textfield" and
                  "focused" in e["state"] and 'value=""' in e["state"]]
        return any(max(f["x"], h["x"]) < min(f["x"] + f["width"], h["x"] + h["width"]) and
                   max(f["y"], h["y"]) < min(f["y"] + f["height"], h["y"] + h["height"])
                   for f in fields for h in hints)

    def empty_near(entries: list[dict], hint: str) -> list[dict]:
        hints = [e for e in entries if hint.lower() in e["label"].lower()]
        return [f for f in entries if f["role"] == "textfield" and 'value=""' in f["state"] and
                any(max(f["x"], h["x"]) < min(f["x"] + f["width"], h["x"] + h["width"]) and
                    max(f["y"], h["y"]) < min(f["y"] + f["height"], h["y"] + h["height"])
                    for h in hints)]

    try:
        words = ["start", "--mobile" if args.mobile else "--desktop", "--vault", args.vault]
        if args.dark:
            words.append("--dark")
        if args.now:
            words += ["--now", args.now]
        run(*words)
        begun = True
        owner = drive.read_json(drive.OWNER)
        output, entries = look("smoke-home")
        seed_titles = ("Weekly review", "Garden plan", "Lisbon in May", "Home life") if args.vault == "demo" else ("Getting started",)
        if args.mobile:
            assert "Home" in output or "Scratch" in output, "home title missing"
        else:
            assert any(title in output for title in seed_titles), "seeded home title missing"
        page_title = "Weekly review" if args.vault == "demo" else "Getting started"
        # Mobile exposes the page list on Home; desktop exposes its sidebar.
        # If the list lies below the fold, move through it as a person would.
        for _ in range(8):
            candidates = named(entries, page_title)
            if candidates:
                break
            width = 390 if args.mobile else 1440
            run("scroll", f"{width // 2},650", "450")
            output, entries = look("smoke-find-page")
        assert candidates, f"page {page_title!r} is not visible"
        assert page_title in output, "seeded page title missing after scrolling"
        # Leftmost is the sidebar when both sidebar and Home list contain it.
        candidate = min(candidates, key=lambda e: (e["x"], e["y"]))
        run("tap", candidate["ref"])
        output, entries = look("smoke-page")
        assert page_title in output, "opened page title missing"
        body_text = ("A little work, some time outdoors, and something to look forward to."
                     if args.vault == "demo" else
                     "Everything here is saved on this device as you type.")
        fields = [e for e in entries if e["role"] == "textfield" and
                  (body_text in e["label"] or body_text in e["state"])]
        if not fields:
            fields = [e for e in entries if body_text in e["label"]]
        assert fields, "editor body is absent from visible semantics"
        run("tap", fields[0]["ref"])
        run("type", "Hello from drive")
        output, _ = look("smoke-typed")
        assert "Hello from drive" in output, "typed text is absent"
        run("key", "ctrl+z")
        output, _ = look("smoke-undone")
        assert "Hello from drive" not in output, "undo did not remove typed text"
        run("key", "ctrl+a")
        run("key", "ctrl+k")
        output, entries = look("smoke-link")
        assert "Paste link" in output, "Ctrl+K did not expose the link popover"
        link_fields = empty_near(entries, "Paste link")
        assert link_fields, "link URL field is not visible"
        if not focused_near(entries, "Paste link"):
            # LinkBox's shortcut autofocus is not reliable in this app harness.
            # Activate the visible URL field with the same click a user makes.
            run("tap", link_fields[0]["ref"])
            output, entries = look("smoke-link-focused")
        assert focused_near(entries, "Paste link"), "link URL field is not focused"
        run("key", "escape")
        # Scratch binds Ctrl+K to links and Ctrl+P to search. Preserve real keys.
        run("key", "ctrl+p")
        output, entries = look("smoke-search")
        if args.mobile and not focused_near(entries, "Search"):
            search_fields = empty_near(entries, "Search")
            assert search_fields, "mobile search field is not visible"
            run("tap", search_fields[0]["ref"])
            output, entries = look("smoke-search-focused")
        assert focused_near(entries, "Search"), "search field is not focused"
        run("key", "escape")
        if args.mobile:
            output, entries = look("smoke-after-search-escape")
            cancel = named(entries, "Cancel")
            if cancel:
                # Mobile SearchScreen exposes Cancel and does not bind Escape.
                run("tap", cancel[0]["ref"])
                output, entries = look("smoke-search-cancelled")
                assert not named(entries, "Cancel"), "mobile search did not close"
        # The mobile app has no sidebar; resize to expose the same real row.
        if args.mobile:
            run("resize", "--desktop")
        output, entries = look("smoke-before-hover")
        candidates = named(entries, page_title)
        assert candidates, "sidebar page missing before hover"
        row = min(candidates, key=lambda e: (e["x"], e["y"]))
        run("hover", row["ref"])
        look("smoke-hover")
        run("resize", "--mobile")
        look("smoke-mobile")
        if args.preserve_vault:
            state = run("state")
            vault = next((line[7:] for line in state.splitlines() if line.startswith("vault: ")), None)
            assert vault, "state did not return the session vault path"
            assert not args.preserve_vault.exists(), "preserve-vault destination already exists"
            shutil.copytree(vault, args.preserve_vault)
            print(f"preserved vault: {args.preserve_vault.resolve()}", flush=True)
        run("stop")
        begun = False
        assert not drive.SESSION.exists(), "session.json remains after stop"
        assert not drive.owned_processes(owner, drive.processes()), "session processes remain after stop"
        print("smoke passed; no owned Flutter or Dart processes remain", flush=True)
        return 0
    except (AssertionError, OSError, drive.DriveError, subprocess.SubprocessError) as exc:
        print(f"smoke failed: {exc}", file=sys.stderr, flush=True)
        return 1
    finally:
        if begun:
            try:
                run("stop")
            except Exception as exc:
                print(f"cleanup failed: {exc}", file=sys.stderr)
        print("timings:", flush=True)
        for command, elapsed in timings:
            print(f"  {elapsed:.3f}s {command}", flush=True)


if __name__ == "__main__":
    sys.exit(main())
