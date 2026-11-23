"""Reconstruct guest busy intervals and independently check QEMU read LBAs."""
from collections import Counter
import json
from pathlib import Path
import re
import sys


def union(intervals):
    merged = []
    for start, end in sorted(intervals):
        if start == end:
            continue
        if merged and start <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return merged


def reads(trace):
    pending, sectors, devices = {}, Counter(), set()
    for line in trace.splitlines():
        req = re.fullmatch(
            rb"virtio_blk_handle_read vdev (\S+) req (\S+) sector (\d+) nsectors (\d+)", line)
        if req:
            device, token, lba, count = req.groups()
            assert (device, token) not in pending
            pending[device, token] = (int(lba), int(count))
            devices.add(device)
        done = re.fullmatch(rb"virtio_blk_req_complete vdev (\S+) req (\S+) status (\d+)", line)
        if done:
            device, token, status = done.groups()
            request = pending.pop((device, token), None)
            if request is not None:
                assert status == b"0"
                lba, count = request
                sectors.update(range(lba, lba + count))
    assert not pending and len(devices) <= 1
    return sectors


run = Path(sys.argv[1])
payload = set(map(int, (run.parent / "payload-lbas.txt").read_text().split()))
serial = (run / "serial.log").read_bytes()
trace = (run / "qemu.log").read_bytes()
phases = [json.loads(line) for line in (run / "phases.jsonl").read_text().splitlines()]
mounted = run.joinpath("block-idle-start.json").exists()
assert len(phases) == 4 and b"[BTOP14] trace exhausted" not in serial
assert (b"[BTOP08] VFS mount=2 source=2 parent_device=1 start=2048 blocks=131072 path=/mnt"
        in serial) == mounted
assert trace.count(b"virtio_blk_handle_write") == 0

pattern = re.compile(
    rb"\[BTOP14\] seq=(\d+) parent=(\d+) partition=(\d+) lba=(\d+) start=(\d+) "
    rb"end=(\d+) active=(\d+) idle=(\d+)")
records, intervals = [], []
overlap = 0
for match in pattern.finditer(serial):
    seq, parent, part, lba, start, end, active, idle = map(int, match.groups())
    assert (seq, parent, part) == (len(records) + 1, 1, 2)
    assert 2048 <= lba < 133120 and start <= end
    assert start % 10_000_000 == end % 10_000_000 == 0
    overlap += any(start < prior_end and prior_start < end for prior_start, prior_end in intervals)
    intervals.append((start, end))
    expected = sum(right - left for left, right in union(intervals))
    assert active == idle == expected
    records.append((match.start(), lba, start, end))
assert serial.count(b"[BTOP14]") == len(records)
assert bool(records) == mounted
assert all(before[3] <= after[2] for before, after in zip(records, records[1:]))


def stats(mark):
    return {row["device"]: row["stats"] for row in
            json.loads((run / f"block-{mark['phase']}.json").read_text()) if "device" in row}


boot = reads(trace[:phases[0]["qemu"]])
cd_bytes = stats(phases[0])["ide1-cd0"]["rd_bytes"]
assert cd_bytes > 0 and (boot[0] > 0 if mounted else not boot)
assert mounted or (b"virtio_blk_" not in trace and not records)
print(f"boot CD bytes={cd_bytes} fixture LBA0 reads={boot[0]} overlap={overlap}")
print(f"all events={len(records)} merged_ns={union(intervals)}")
for before, after in zip(phases, phases[1:]):
    events = [row for row in records if before["serial"] <= row[0] < after["serial"]]
    sectors = Counter(row[1] for row in events)
    assert sectors == reads(trace[before["qemu"]:after["qemu"]])
    data = {lba: sectors[lba] for lba in payload if lba in sectors}
    if mounted:
        prev, curr = stats(before)["fixture"], stats(after)["fixture"]
        assert prev["wr_bytes"] == curr["wr_bytes"] == 0
        assert curr["rd_bytes"] - prev["rd_bytes"] == len(events) * 512
    if mounted and after["phase"] != "after-idle":
        assert data == dict.fromkeys(payload, 1)
    else:
        assert not sectors
    merged = union((start, end) for _, _, start, end in events)
    print(f"{after['phase']}: reads={len(events)} payload={len(data)} "
          f"busy_ns={sum(end - start for start, end in merged)} merged_ns={merged}")
