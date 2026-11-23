"""Serial Linux v6.12 reference model and independent guest interval receipt."""
from collections import Counter
import json
from pathlib import Path
import re
import sys


def reference(events):
    stamp = active = ticks = completed_us = union_us = previous = 0
    starts = {}
    result = []
    for now, event, request in events:
        tick = now // 10_000
        union_us += (now - previous) if active else 0
        previous = now
        if (event != "sample" or active) and tick > stamp:
            if event == "end" or active:
                ticks += tick - stamp
            stamp = tick
        if event == "start":
            starts[request] = now
            active += 1
        elif event == "end":
            active -= 1
            completed_us += now - starts.pop(request)
        else:
            result.append((completed_us // 1000, ticks * 10, active, union_us))
    return result


def model():
    cases = [
        ("early", [(100, "start", 1), (1600, "end", 1), (1800, "sample", 0)],
         [(1, 0, 0, 1500)]),
        ("sub-ms sum before rounding", [(100, "start", 1), (700, "end", 1),
          (800, "start", 2), (1400, "end", 2), (1500, "sample", 0)], [(1, 0, 0, 1200)]),
        ("tick edge", [(9999, "start", 1), (10001, "end", 1), (10002, "sample", 0)],
         [(0, 10, 0, 2)]),
        ("long", [(19000, "start", 1), (51000, "end", 1), (60000, "sample", 0)],
         [(32, 40, 0, 32000)]),
        ("overlap and active sample", [(11000, "start", 1), (23000, "start", 2),
          (35000, "sample", 0), (42000, "end", 1), (48000, "end", 2), (59000, "sample", 0)],
         [(0, 20, 2, 24000), (56, 30, 0, 37000)]),
    ]
    for name, events, expected in cases:
        actual = reference(events)
        assert actual == expected, (name, actual, expected)
        print(name, actual)


def guest(run):
    serial = (run / "serial.log").read_bytes()
    observed = re.compile(
        rb"\[BTOP19\] id=(\d+) lba=(\d+) first=\((\d+), (\d+), (\d+), (\d+)\)"
        rb" last=\((\d+), (\d+), (\d+), (\d+)\)"
    )
    rows = []
    for match in observed.finditer(serial):
        identity, lba, cpu, start, start_next, hz, end_cpu, end, end_next, end_hz = map(
            int, match.groups()
        )
        assert identity == 2 and 2048 <= lba < 133120
        assert hz == end_hz > 0 and cpu == end_cpu
        assert start <= start_next <= end <= end_next
        rows.append((match.start(), cpu, end - start, start_next - start, end_next - end))
    assert b"[BTOP19] registration" in serial and rows
    assert len(rows) == serial.count(b"[BTOP16] read ")
    assert list((match[1], match[2]) for match in observed.finditer(serial)) == re.findall(
        rb"\[BTOP16\] read parent=1 id=(\d+) lba=(\d+)", serial
    )
    print("CPU counts:", dict(Counter(row[1] for row in rows)))
    steps = [step for row in rows for step in row[3:] if step > 0]
    print("consecutive sample step ns (min/max):", min(steps), max(steps))
    mount = serial.index(b"[BTOP19] mount")
    phases = [("mount", mount)]
    phases.extend((p["phase"], p["serial"]) for p in map(
        json.loads, (run / "phases.jsonl").read_text().splitlines()
    ))
    prev = 0
    for name, offset in phases:
        durations = [row[2] for row in rows if prev <= row[0] < offset]
        prev = offset
        print(name, "count", len(durations), "sum_ns", sum(durations), "range_ns",
              (min(durations), max(durations)) if durations else None,
              "sub_10ms", sum(delta < 10_000_000 for delta in durations))
    assert prev > rows[-1][0]


if __name__ == "__main__":
    model()
    if len(sys.argv) > 1:
        guest(Path(sys.argv[1]))
