"""Mutation controls against an independently captured accepted native R2 log."""
import json
from pathlib import Path
import sys

from fastlane_m61b_r2_receipt import assess


def main():
    raw = Path(sys.argv[1]).read_text(errors="replace")
    assert assess(raw)["status"] == "PASS", "accepted native input required"
    lines = raw.splitlines(keepends=True)
    controls = {}
    selectors = {
        "terminal-removed": lambda line: line.startswith("[FL-HYBRID-K] event=terminal"),
        "inventory-removed": lambda line: line.startswith("[FL-R2-INVENTORY]"),
        "ack-removed": lambda line: line.startswith("[FL-TLB]") and "event=ack " in line,
        "physical-release-removed": lambda line: line.startswith("[FL-FRAME]") and "event=free " in line,
        "stale-control-removed": lambda line: "control=stale-policy-generation " in line,
        "direction-control-removed": lambda line: "control=invalid-directional-contract " in line,
        "foreign-caller-removed": lambda line: "control=foreign-caller " in line,
        "foreign-adoption-removed": lambda line: "control=foreign-reply-adoption " in line,
        "contention-removed": lambda line: "event=lock-contended " in line,
        "caller-exit-removed": lambda line: "event=caller-reaped mode=10 " in line,
        "closure-winner-removed": lambda line: "mode=4 " in line and "event=retire-return " in line,
        "remote-establishment-removed": lambda line: line.startswith("[FL-R2-REMOTE]"),
    }
    for name, select in selectors.items():
        index = next(index for index, line in enumerate(lines) if select(line))
        changed = "".join(lines[:index] + lines[index + 1:])
        controls[name] = assess(changed)["status"] == "FAIL"
    terminal = next(line for line in lines if line.startswith("[FL-HYBRID-K] event=terminal"))
    controls["duplicate-terminal"] = assess(raw + terminal)["status"] == "FAIL"
    changed = raw.replace("flushed=1", "flushed=0")
    controls["unflushed-ack"] = assess(changed)["status"] == "FAIL"
    changed = raw.replace("control=stale-policy-generation", "control=stale-policy-generation-missing")
    controls["renamed-original-control"] = assess(changed)["status"] == "FAIL"
    print(json.dumps({"status": "PASS" if all(controls.values()) else "FAIL", "controls": controls}, indent=2))
    return 0 if all(controls.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
