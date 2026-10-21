"""Summarize Flutter's JSON reporter, including per-file elapsed times."""

import json
import sys
from collections import Counter
from pathlib import Path


def report(path):
    suites, tests, starts, ends = {}, {}, {}, {}
    counts = Counter()
    for line in Path(path).read_text(encoding="utf-8-sig").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        kind = event.get("type")
        if kind == "suite":
            suite = event["suite"]
            suites[suite["id"]] = suite["path"]
        elif kind == "testStart":
            test = event["test"]
            tests[test["id"]] = test
            if not test["name"].startswith("loading "):
                starts.setdefault(test["suiteID"], event["time"])
        elif kind == "testDone":
            test = tests.get(event["testID"])
            if test is None or test["name"].startswith("loading "):
                continue
            ends[test["suiteID"]] = event["time"]
            if not event.get("hidden", False):
                counts["skipped" if event["skipped"] else event["result"]] += 1
        elif kind == "done":
            print(f"Reporter elapsed: {event['time'] / 1000:.3f}s")
    print("Tests:", dict(counts), "total:", sum(counts.values()))
    times = [(ends[sid] - start, suites.get(sid, str(sid))) for sid, start in starts.items() if sid in ends]
    for elapsed, name in sorted(times, reverse=True)[:10]:
        print(f"{elapsed / 1000:8.3f}s  {Path(name).name}")


for filename in sys.argv[1:]:
    print(filename)
    report(filename)
