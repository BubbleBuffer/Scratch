#!/usr/bin/env python3
"""Run feature-instrumented first-sync phases on fresh, disposable fixture copies.

Run on the Linux machine; no builds, installations, or changes to the source fixtures.
The uninstrumented A/B pairs use cold_ab.py instead.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

from cold_ab import capture_load, copy_tree, fingerprint, remove_owned, sha256


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, default=Path('/home/dev/cold-history-profile-20261013-v1'))
    p.add_argument('--fixture-root', type=Path, default=Path('/home/dev/cold-history-measure-20261013-v3'))
    p.add_argument('--sides', default='master,branch')
    p.add_argument('--phases', default='core-export,http-first-sync')
    p.add_argument('--tag', default='1')
    p.add_argument('--load-host', default='home-server')
    a = p.parse_args()
    out = a.out.resolve()
    for phase in a.phases.split(','):
        if phase not in ('core-export', 'http-first-sync'):
            raise ValueError('unknown phase')
        for side in a.sides.split(','):
            if side not in ('master', 'branch'):
                raise ValueError('unknown side')
            fixture = (a.fixture_root / ('aged-hot' if side == 'master' else 'aged-cold')).resolve()
            binary = out / 'bin' / (side + '-profile')
            label = f'{side}-{phase}-{a.tag}'
            work, log = out / label, out / 'logs' / (label + '.log')
            if work.exists() or log.exists() or work.is_symlink():
                raise ValueError(f'refusing to overwrite {label}')
            before = fingerprint(fixture)
            if before['symlinks']:
                raise ValueError('fixture contains symlinks')
            digest = sha256(binary)
            work.mkdir()
            capture_load(out / 'logs' / (label + '.load-before'), a.load_host)
            copy_tree(fixture, work / 'hub')
            command = [str(binary), str(work), '10000', '2000', '--only', phase]
            started = time.monotonic()
            with log.open('wb') as stream:
                result = subprocess.run(command, stdin=subprocess.DEVNULL,
                                        stdout=stream, stderr=subprocess.STDOUT, timeout=1200)
            elapsed = time.monotonic() - started
            capture_load(out / 'logs' / (label + '.load-after'), a.load_host)
            if fingerprint(fixture) != before or sha256(binary) != digest:
                raise ValueError('source fixture or pinned binary changed')
            reports = [json.loads(line.removeprefix('phase-json '))
                       for line in log.read_text().splitlines() if line.startswith('phase-json ')]
            record = dict(label=label, command=command, binary_sha256=digest,
                          exit=result.returncode, wall_seconds=elapsed, fixture=before,
                          phases=reports, load_source='see adjacent load files')
            (out / 'logs' / (label + '.json')).write_text(json.dumps(record, indent=2) + '\n')
            print(f'{label}: exit={result.returncode} wall={elapsed:.1f}s reports={len(reports)}', flush=True)
            if result.returncode or len(reports) != 1:
                raise RuntimeError(f'{label} failed; log and disposable copy preserved')
            remove_owned(out, label)


if __name__ == '__main__':
    main()
