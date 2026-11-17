#!/usr/bin/env python3
"""Inventory ring-0 VFS ownership; --check is the final removal gate.

This source audit complements native tests; it cannot prove runtime routing.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "docs/filesystems/userspace-vfs-kernel-baseline.json"
PATTERNS = {
    "kernel-global namespace": re.compile(r"\bKERNEL_VFS\b"),
    "filesystem namespace construction": re.compile(r"\bVfs(?:::[A-Za-z_]+|\s*<)"),
    "filesystem backend construction": re.compile(r"\b(?:Tmpfs|RamFs|Fat32)::(?:new|with_limits|mount)\s*\("),
}


def inventory(root):
    findings = []
    for path in sorted((root / "kernel/src").rglob("*.rs")):
        for number, line in enumerate(path.read_text().splitlines(), 1):
            code = line.split("//", 1)[0]
            for category, pattern in PATTERNS.items():
                if pattern.search(code):
                    findings.append((str(path.relative_to(root)), number, category, code.strip()))
    return findings


def additions(findings, baseline):
    """Compare exact ownership sites, preserving multiplicity but ignoring lines.

    A total-count budget would allow one new owner to replace one deleted owner.
    Paths and code are therefore part of the identity. Baseline updates require
    review; this is a source guard, not a runtime or complete Rust syntax audit.
    """
    current = Counter((path, category, code) for path, _, category, code in findings)
    allowed = Counter((row["path"], row["category"], row["code"]) for row in baseline)
    return current - allowed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--no-growth", action="store_true", help="reject ownership sites absent from the reviewed migration baseline")
    mode.add_argument("--check", action="store_true", help="fail until all inventoried ring-0 owners are removed")
    args = parser.parse_args()
    findings = inventory(ROOT)
    for path, number, category, code in findings:
        print(f"{path}:{number}: {category}: {code}")
    print(f"{len(findings)} ring-0 VFS ownership references remain")
    if args.no_growth:
        try:
            baseline = json.loads(BASELINE.read_text())
            if baseline["version"] != 1:
                raise ValueError("unsupported baseline version")
            extra = additions(findings, baseline["findings"])
        except (OSError, ValueError, KeyError, TypeError) as error:
            print(f"Cannot validate migration baseline: {error}")
            return 1
        for (path, category, code), count in sorted(extra.items()):
            print(f"NEW ({count}): {path}: {category}: {code}")
        print(f"{sum(extra.values())} new ring-0 VFS ownership references")
        return int(bool(extra))
    return int(args.check and bool(findings))


if __name__ == "__main__":
    raise SystemExit(main())
