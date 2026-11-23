"""Compare pinned blk-mq accounting with candidate Helios boundary events."""
import json
from pathlib import Path
import re
import sys


def linux(events):
    starts = {}
    completed = sectors = total = 0
    samples = []
    for kind, req, value in events:
        if kind == "init":
            starts[req] = value
        elif kind == "retire":
            sectors += value >> 9
        elif kind == "done":
            completed += 1
            total += value - starts.pop(req)
        elif kind == "sample":
            samples.append((completed, sectors, total, total // 1_000_000))
    return samples


def candidate(events):
    arrivals = {}
    retired = []
    samples = []
    for kind, req, value in events:
        if kind == "admit":
            arrivals[req] = value
        elif kind in ("ok", "error"):
            retired.append((kind == "ok", value - arrivals.pop(req)))
        elif kind == "sample":
            elapsed = sum(duration for _, duration in retired)
            samples.append((len(retired), sum(ok for ok, _ in retired),
                            elapsed, elapsed // 1_000_000))
    return samples


def model():
    for label, second, sample, first_end, second_end in (
        ("sequential", 800_000, 900_000, 700_000, 1_400_000),
        ("overlap", 200_000, 400_000, 700_000, 800_000),
    ):
        first = [("init", 1, 100_000), ("dispatch", 1, 300_000)]
        second_start = [("init", 2, second)]
        first_done = [("retire", 1, 512), ("done", 1, first_end)]
        active = [("sample", 0, sample)]
        middle = (first_done + second_start + active if label == "sequential"
                  else second_start + active + first_done)
        events = first + middle + [("retire", 2, 512), ("done", 2, second_end),
                                   ("sample", 0, second_end)]
        mapped = [({"init": "admit", "done": "ok"}.get(kind, kind), req, value)
                  for kind, req, value in events]
        expected = [(1, 1, 600_000, 0) if label == "sequential" else (0, 0, 0, 0),
                    (2, 2, 1_200_000, 1)]
        assert linux(events) == candidate(mapped) == expected
        print(label, expected)

    sample = ("sample", 0, 900_000)
    failed = linux([("init", 1, 0), ("retire", 1, 512), ("done", 1, 900_000), sample])
    assert failed == [(1, 1, 900_000, 0)]
    assert candidate([("admit", 1, 0), ("error", 1, 900_000), sample]) == [
        (1, 0, 900_000, 0)]
    assert candidate([("admit", 1, 0), ("timeout", 1, 900_000), sample]) == [(0, 0, 0, 0)]
    assert linux([("init", 1, 0), ("retire", 1, 512), sample,
                  ("retire", 1, 512), ("done", 1, 1_200_000), sample]) == [
        (0, 1, 0, 0), (1, 2, 1_200_000, 1)]
    print("device error: same duration, different sector semantics; timeout unresolved;"
          " Linux partial retirement does not publish duration")


def guest(run):
    serial = (run / "serial.log").read_bytes()
    stamp = rb"\((\d+), (\d+), (\d+), (\d+)\)"
    pattern = (rb"\[BTOP20\] parent=(\d+) id=(\d+) lba=(\d+) retained=(\d+) "
               rb"published=(\d+) ok=(\d+) first=" + stamp + rb" last=" + stamp)
    if b"[BTOP20]" not in serial:
        assert b"path=/mnt" not in serial and b"[BTOP16]" not in serial
        print("no attributed events")
        return
    registration, = re.finditer(rb"\[BTOP20\] registration id=(\d+) stamp=" + stamp, serial)
    child, boot_cpu, registered, _, hz = map(int, registration.groups())
    calibration = re.search(rb"\[TIME\] PIT calibration .* tsc_hz=(\d+)", serial)
    assert calibration and calibration.start() < registration.start()
    assert int(calibration[1]) == hz > 0 and registered > 0
    mount, = re.finditer(rb"\[BTOP20\] mount path=/mnt stamp=" + stamp, serial)
    events = []
    for match in re.finditer(pattern, serial):
        parent, device, lba, retained, published, ok, cpu, start, start2, shz, end_cpu, end, end2, ehz = (
            map(int, match.groups()))
        assert registration.start() < match.start()
        assert device == child and published == ok == 1 and cpu == end_cpu
        assert shz == ehz == hz and 0 < start <= start2 < end <= end2
        if not retained:
            assert cpu == boot_cpu and start >= registered
        events.append((match.start(), parent, device, lba, cpu, end - start, retained))
    coarse = [tuple(map(int, match.groups())) for match in re.finditer(
        rb"\[BTOP16\] read parent=(\d+) id=(\d+) lba=(\d+)", serial)]
    assert [event[1:4] for event in events] == coarse
    assert len(events) + 2 == serial.count(b"[BTOP20]")
    print("registration: (0 completions, 0 sectors, 0 ns)")
    checkpoints = [("mount", mount.start())]
    for line in (run / "phases.jsonl").read_text().splitlines():
        phase = json.loads(line)
        checkpoints.append((phase["phase"], phase["serial"]))
    previous = 0
    for label, offset in checkpoints:
        prefix = [event for event in events if event[0] < offset]
        window = prefix[previous:]
        total = sum(event[5] for event in prefix)
        print(f"{label}: absolute={(len(prefix), len(prefix), total)} "
              f"model_ms={total // 1_000_000} delta_count={len(window)} "
              f"delta_ns={sum(event[5] for event in window)} "
              f"cpus={sorted({event[4] for event in window})}")
        previous = len(prefix)
    assert previous == len(events)
    print(f"boot adapter={sum(not event[6] for event in events)} retained="
          f"{sum(event[6] for event in events)} interval_ns="
          f"{min(event[5] for event in events)}..{max(event[5] for event in events)}")


if __name__ == "__main__":
    if len(sys.argv) == 1:
        model()
    else:
        guest(Path(sys.argv[1]))
