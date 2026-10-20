#!/usr/bin/env python3
"""Decode the existing TTY session marker framing, preserving raw evidence.

session_foundation_log sends prefix, body and newline via separate DebugLog
calls; the kernel appends a newline to each call. Decode only that fixed
prefix/newline boundary. This is not a replacement for permission tests.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serial", type=Path)
    args = parser.parse_args()
    raw = args.serial.read_bytes()
    text = raw.decode(errors="replace")
    prefix = "[SESSION-FOUNDATION] \n"
    decoded = text.replace(prefix, "[SESSION-FOUNDATION] ")
    expected = [line for line in (ROOT / "tools/tests/session_foundation.expected").read_text().splitlines()
                if line and not line.startswith("#")]
    missing = [line for line in expected if line not in decoded]
    faults = [marker for marker in ("KERNEL PANIC", "[FAULT]", "[EXCEPTION]", "UNEXPECTED", " FAIL")
              if marker in decoded]
    tty = "services/tty_server/src/main.rs"
    baseline = subprocess.check_output(["git", "show", f"HEAD:{tty}"], cwd=ROOT)
    tty_unchanged = baseline == (ROOT / tty).read_bytes()
    # Refuse normalization if the source that establishes this framing changed.
    passed = not missing and not faults and tty_unchanged and "[PMM] OK" in decoded
    output = args.serial.with_suffix(".decoded.log")
    output.write_text(decoded)
    result = {
        "pass": passed, "scope": "native desktop/session assertions; fixed legacy log framing decoded",
        "raw_serial": str(args.serial), "raw_sha256": hashlib.sha256(raw).hexdigest(),
        "decoded_serial": str(output), "joined_prefix_fragments": text.count(prefix),
        "expected_assertions": len(expected), "missing": missing, "fault_markers": faults,
        "tty_source_identical_to_HEAD": tty_unchanged,
        "raw_line_gate_result": "separate legacy test.sh result must be retained; not overwritten",
    }
    args.serial.with_suffix(".verification.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
