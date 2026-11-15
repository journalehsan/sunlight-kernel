#!/usr/bin/env python3
"""Verify the sealed SFS-5B.3 files and preserved incoming evidence, read-only."""
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    seal = json.loads((OUT/'source-integrity.json').read_text())
    failures = []
    for name, expected in seal['sealed_files'].items():
        path = ROOT/name
        if not path.is_file() or digest(path) != expected:
            failures.append(name)
    baseline = json.loads((OUT/'source-baseline.json').read_text())['files']
    for name, expected in baseline.items():
        if name not in seal['changed_incoming_files']:
            path = ROOT/name
            if not path.is_file() or digest(path) != expected:
                failures.append(name)
    names = subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard'], cwd=ROOT).decode().split('\0')
    current = {n: digest(ROOT/n) for n in names if n and (ROOT/n).is_file()
               and not (ROOT/n).is_symlink() and not n.startswith(('docs/audit/', 'docs/filesystems/'))}
    expected = seal['execution_inputs']
    failures.extend(n for n in sorted(set(current)|set(expected)) if current.get(n) != expected.get(n))
    print(json.dumps(dict(status='FAIL' if failures else 'PASS',
                          failures=failures, sealed_files=len(seal['sealed_files']),
                          incoming_files=len(baseline)), indent=2))
    return bool(failures)


if __name__ == '__main__':
    raise SystemExit(main())
