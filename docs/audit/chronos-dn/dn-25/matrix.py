"""Join captured Chronos INT 16h results with delimited SeaBIOS probe rows."""
import csv
from pathlib import Path
import re
import sys

root = Path(__file__).parent
keys = {
    "up": (0x48, 1), "down": (0x50, 1), "left": (0x4b, 1), "right": (0x4d, 1),
    "home": (0x47, 1), "end": (0x4f, 1), "pgup": (0x49, 1), "pgdn": (0x51, 1),
    "insert": (0x52, 1), "delete": (0x53, 1), "kp_decimal": (0x53, 0),
    "kp_add": (0x4e, 0), "kp_subtract": (0x4a, 0), "kp_multiply": (0x37, 0),
    "kp_divide": (0x35, 1), "kp_enter": (0x1c, 1), "ret": (0x1c, 0),
}
keys.update({f"kp_{i}": (scan, 0) for i, scan in enumerate(
    (0x52, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47, 0x48, 0x49))})
chronos = {}
for line in (root / "focused-tests.log").read_text().splitlines():
    if line.startswith("DN25 num="):
        num = int(line[-1])
    elif line.startswith("DN25 scan="):
        event = dict(re.findall(r"(\w+)=([\da-f]+)", line))
        identity = (num, int(event["scan"], 16), *(int(event[x]) for x in
                    ["e0", "shift", "ctrl", "alt"]))
        chronos.setdefault(identity, {})[event["ah"]] = event
writer = csv.writer(sys.stdout, lineterminator="\n")
writer.writerow(["num_lock", "key", "scan", "e0", "baseline_enhanced_source",
                 "chronos_queue_word", "chronos_01", "chronos_11", "chronos_10",
                 "status_flags", "read_flags", "reference_01", "reference_11",
                 "reference_10", "status"])
seen = 0
mismatches = 0
with (root / "seabios-delimited.csv").open() as f:
    for line in f:
        if not re.match(r"[01],", line):
            continue
        num, chord, raw = line.strip().split(",")
        parts = chord.split("+")
        modifier = parts[0] if len(parts) > 1 else ""
        scan, e0 = keys[parts[-1]]
        if scan == 0x1c:
            baseline = f"{(0xa6 if e0 else 0x1c):02X}00" if modifier == "alt" else (
                f"{(0xe0 if e0 else 0x1c):02X}{(0x0a if modifier == 'ctrl' else 0x0d):02X}")
        elif scan in (0x35, 0x37, 0x4a, 0x4c, 0x4e):
            baseline = "EMPTY"
        else:
            baseline = f"{scan:02X}00"
        actual = chronos[(int(num), scan, e0, *(int(modifier == m)
                                             for m in ("shift", "ctrl", "alt")))]
        if raw == "EMPTY":
            ref_legacy = ref_enhanced = ref_read = "EMPTY"
        else:
            ref_enhanced, flags, bx, cx, dx, ref_legacy, ref_read, read_flags, _ = raw.split()
            assert (flags, bx, cx, dx, read_flags) == ("0207", "ABCD", "1234", "9876", "0247")
        legacy = actual["01"]["ax"].upper() if "10" in actual else "EMPTY"
        enhanced = actual["11"]["ax"].upper() if "10" in actual else "EMPTY"
        read = actual["10"]["ax"].upper() if "10" in actual else "EMPTY"
        queue = enhanced
        if modifier == "alt" and not e0 and scan in (0x37, 0x4a, 0x4e):
            queue = f"{scan:02X}F0"
        matches = (legacy, enhanced, read) == (ref_legacy, ref_enhanced, ref_read)
        seen += 1
        mismatches += not matches
        writer.writerow([num, chord, f"{scan:02X}", e0, baseline, queue, legacy,
                         enhanced, read, actual["11"]["flags"],
                         actual.get("10", {}).get("flags", ""), ref_legacy,
                         ref_enhanced, ref_read, "MATCH" if matches else "DIFFERENT"])
assert seen == len(chronos) == 216, (seen, len(chronos))
sys.exit(bool(mismatches))
