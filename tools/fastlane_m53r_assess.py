#!/usr/bin/env python3
"""Additional M5.3R receipts; complements, never replaces, M1/M2 assessment."""
import json
import re
import sys


def records(raw, prefix):
    result = []
    for line in raw.splitlines():
        if line.startswith(prefix):
            pairs = re.findall(r"(\w+)=(\S+)", line)
            row = dict(pairs)
            if len(row) != len(pairs):
                raise ValueError("duplicate field")
            result.append(row)
    return result


def assess(raw):
    errors = []
    resumed = []
    reclaimed = []
    checkpoints = []
    try:
        if any(marker in raw for marker in ("PANIC", "panicked at", "checkpoint=FAIL")):
            raise ValueError("guest invariant failure")
        waits = [r for r in records(raw, "[FL-COORD] ") if r.get("event") == "wait"]
        for row in waits:
            yields = int(row["yields"])
            if int(row["checked_resumes"]) != yields:
                raise ValueError("unchecked continuation resume")
            if yields:
                before, after, top = (int(row[k]) for k in ("rsp_before", "rsp_after", "stack_top"))
                if before != after or not top - 32768 <= before < top - 256:
                    raise ValueError("continuation stack mismatch")
                if any(int(row[k]) <= 0 for k in ("pid", "inc", "vm")) or row["resume_if"] != "0":
                    raise ValueError("invalid continuation identity or IF")
                resumed.append(row)
        if not resumed:
            raise ValueError("no checked native continuation")
        quota = records(raw, "[FL-QUOTA] ")
        reclaimed = [r for r in quota if r.get("event") == "reclaimed"]
        checkpoints = [r for r in quota if r.get("event") == "checkpoint"]
        backing = [r for r in records(raw, "[FL-FRAME] ") if r.get("event") == "backing"]
        if not reclaimed or len(reclaimed) != len(backing):
            raise ValueError("backing and quota receipt count mismatch")
        for row in reclaimed:
            n = {k: int(v) for k, v in row.items() if k != "event"}
            if any(n[k] <= 0 for k in ("pid", "inc", "session", "epoch", "bytes", "frames", "frame")):
                raise ValueError("invalid quota identity")
            if n["bytes"] != n["frames"] * 4096:
                raise ValueError("backing byte mismatch")
            for kind in ("process", "session"):
                if n[kind + "_before"] - n["bytes"] != n[kind + "_after"]:
                    raise ValueError("quota release mismatch")
            if n["free_after"] - n["free_before"] != n["frames"]:
                raise ValueError("physical release mismatch")
        if sorted(r["frame"] for r in backing) != sorted(r["frame"] for r in reclaimed):
            raise ValueError("quota physical identity mismatch")
        final = [r for r in checkpoints if r.get("checkpoint") == "999"]
        if len(final) != 1:
            raise ValueError("missing final quota checkpoint")
        for row in checkpoints:
            if any(int(row[k]) != 0 for k in ("account_bytes", "account_entries", "restricted_receipts", "objects", "views", "frames")):
                raise ValueError("quota or restricted inventory drift")
    except (ValueError, KeyError, TypeError) as error:
        errors.append(str(error))
    return {"status": "FAIL" if errors else "PASS", "errors": errors,
            "checked_waits": resumed, "reclaimed": reclaimed, "checkpoints": checkpoints,
            "scope": "exercised M1 continuations and M2 restricted backing only",
            "general_login_membership": "BLOCKED", "legacy_scheduled_lifecycle": "NOT_RUN"}


if __name__ == "__main__":
    from pathlib import Path
    result = assess(Path(sys.argv[1]).read_text(errors="replace"))
    print(json.dumps(result, indent=2))
    raise SystemExit(result["status"] != "PASS")
