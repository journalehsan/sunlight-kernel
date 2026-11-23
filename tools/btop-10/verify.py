"""Reconcile guest read receipts with QEMU requests and the offline FAT map."""
from collections import Counter
import json
from pathlib import Path
import re
import sys

run = Path(sys.argv[1])
payload = set(map(int, (run.parent / "payload-lbas.txt").read_text().split()))
serial = (run / "serial.log").read_bytes()
trace = (run / "qemu.log").read_bytes()
phases = [json.loads(line) for line in (run / "phases.jsonl").read_text().splitlines()]
mounted = run.joinpath("block-idle-start.json").exists()
assert b"completion trace exhausted" not in serial
assert len(phases) == 4
assert (b"[BTOP08] VFS mount=2 source=2 parent_device=1 start=2048 blocks=131072 path=/mnt"
        in serial) == mounted


def stats(phase):
    return {row["device"]: row["stats"] for row in json.loads(
        (run / f"block-{phase}.json").read_text()) if "device" in row}


def qemu_reads(lines):
    pending = {}
    result = Counter()
    for line in lines.splitlines():
        match = re.fullmatch(
            rb"virtio_blk_handle_read vdev (\S+) req (\S+) sector (\d+) nsectors (\d+)", line)
        if match:
            device, request, sector, count = match.groups()
            assert (device, request) not in pending
            pending[device, request] = int(sector), int(count)
        match = re.fullmatch(
            rb"virtio_blk_req_complete vdev (\S+) req (\S+) status (\d+)", line)
        if match:
            device, request, status = match.groups()
            sector, count = pending.pop((device, request), (0, 0))
            if status == b"0":
                result.update(range(sector, sector + count))
    assert not pending
    return result


initial = stats(phases[0]["phase"])
assert initial["ide1-cd0"]["rd_bytes"] > 0
boot = qemu_reads(trace[:phases[0]["qemu"]])
if mounted:
    assert boot[0] > 0
    print(f"boot ISO bytes={initial['ide1-cd0']['rd_bytes']} outside-partition LBA0={boot[0]}")
else:
    assert not boot and b"[BTOP10] completion seq=" not in serial
    print(f"no fixture; boot ISO bytes={initial['ide1-cd0']['rd_bytes']}")

for before, after in zip(phases, phases[1:]):
    receipts = re.findall(rb"\[BTOP10\] completion seq=(\d+) parent=(\d+) lba=(\d+) bytes=(\d+)",
                          serial[before["serial"]:after["serial"]])
    sectors = Counter()
    for _, parent, sector, size in receipts:
        assert parent == b"1" and size == b"512"
        assert 2048 <= int(sector) < 133120
        sectors[int(sector)] += 1
    assert sectors == qemu_reads(trace[before["qemu"]:after["qemu"]])
    previous, current = stats(before["phase"]), stats(after["phase"])
    if mounted:
        assert current["fixture"]["rd_bytes"] - previous["fixture"]["rd_bytes"] == sum(sectors.values()) * 512
        assert current["fixture"]["wr_bytes"] == previous["fixture"]["wr_bytes"] == 0
    if mounted and after["phase"] != "after-idle":
        assert set(sectors) >= payload
    else:
        assert not sectors
    data_reads = sum(sectors[sector] for sector in payload)
    total = sum(sectors.values())
    print(f"{after['phase']} ns={before['monotonic_ns']}:{after['monotonic_ns']} "
          f"receipts={len(receipts)} payload={data_reads} other={total - data_reads} "
          f"bytes={total * 512} LBAs_other={sorted(set(sectors) - payload)}")
