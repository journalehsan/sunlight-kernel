"""Verify lifetime snapshots from raw endpoints and successful QEMU reads."""
from collections import Counter
import json
from pathlib import Path
import re
import runpy
import sys


run = Path(sys.argv[1])
serial = (run / "serial.log").read_bytes()
trace = (run / "qemu.log").read_bytes()
boot_only = "--boot-only" in sys.argv
if not boot_only:
    runpy.run_path(str(Path(__file__).parents[1] / "btop-14/verify.py"))
    phases = [json.loads(line) for line in (run / "phases.jsonl").read_text().splitlines()]
else:
    phases = []


def reads(raw, single=False):
    issued = Counter()
    result = Counter()
    for line in raw.splitlines():
        request = re.fullmatch(
            rb"virtio_blk_handle_read vdev (\S+) req (\S+) sector (\d+) nsectors (\d+)", line)
        if request:
            device, token, lba, size = request.groups()
            lba, size = int(lba), int(size)
            assert (device, token) not in issued
            assert not single or size == 1
            issued[device, token] = (lba, size)
        finished = re.fullmatch(rb"virtio_blk_req_complete vdev (\S+) req (\S+) status (\d+)", line)
        if finished and (finished[1], finished[2]) in issued:
            device, token, status = finished.groups()
            assert (device, token) in issued and status == b"0"
            lba, size = issued.pop((device, token))
            result.update(range(lba, lba + size))
    assert not issued and b"virtio_blk_handle_write" not in raw
    return result


mount = re.search(rb"\[BTOP08\] VFS .* path=/mnt", serial)
if not mount:
    assert b"[BTOP16]" not in serial and not reads(trace)
    print("no disk: no registration, read events, or totals")
    sys.exit(0)

registration, = re.finditer(
    rb"\[BTOP16\] register parent=(\d+) id=(\d+) start=(\d+) blocks=(\d+)", serial)
parent, identity, base, blocks = map(int, registration.groups())
discovery = list(re.finditer(
    rb"^virtio_blk_handle_read .* sector 1 nsectors 1\r?$",
    trace, re.M))[-1]
first_child = re.search(rb"^virtio_blk_handle_read ", trace[discovery.end():], re.M)
assert first_child is not None
boundary = discovery.end() + first_child.start()
before_registration = reads(trace[:boundary])
excluded = sum(before_registration.values())
print(f"pre-registration: {excluded} sectors, {before_registration[0]} touching LBA0, "
      f"{sum(n for lba, n in before_registration.items() if base <= lba < base + blocks)} in child extent")
events = []
count = sectors = elapsed = snapshots = 0
for line in re.finditer(rb"\[BTOP16\] (read|totals) ([^\r\n]+)", serial):
    values = dict((key, int(value)) for key, value in re.findall(rb"(\w+)=(\d+)", line[2]))
    assert values[b"id"] == identity and line.start() > registration.start()
    if line[1] == b"totals":
        assert snapshots == count
        assert (values[b"completions"], values[b"sectors"], values[b"ns"]) == (count, sectors, elapsed)
        snapshots += 1
    else:
        assert snapshots == count + 1
        lba, start, end, ok, size = (values[key] for key in (b"lba", b"start", b"end", b"ok", b"bytes"))
        assert values[b"parent"] == parent and base <= lba < base + blocks
        assert ok == 1 and size == 512
        assert 0 <= start <= end and start % 10_000_000 == end % 10_000_000 == 0
        assert not events or start >= events[-1][-1]
        count += 1
        sectors += 1
        elapsed += end - start
        events.append((line.start(), lba, start, end))
assert snapshots == count + 1
assert serial.count(b"[BTOP16]") == 1 + snapshots + count
print(f"at mount: {sum(pos < mount.start() for pos, *_ in events)} reads")
if boot_only:
    assert all(pos < mount.start() for pos, *_ in events)
    physical = reads(trace[boundary:], single=True)
    assert physical == Counter(lba for _, lba, _, _ in events)
    print(f"boot count/sectors/ns={(count, sectors, elapsed)} sectors excluded={excluded}")
    sys.exit(0)

prior = (0, 0, 0)
for phase in phases:
    prefix = [event for event in events if event[0] < phase["serial"]]
    guest = Counter(lba for _, lba, _, _ in prefix)
    physical = reads(trace[boundary:phase["qemu"]], single=True)
    assert excluded > 0 and guest == physical
    qmp = json.loads((run / f"block-{phase['phase']}.json").read_text())
    fixture, = [row["stats"] for row in qmp if row.get("device") == "fixture"]
    assert fixture["rd_bytes"] == (sum(guest.values()) + excluded) * 512
    assert fixture["wr_bytes"] == 0
    absolute = (len(prefix), sum(guest.values()), sum(end - start for _, _, start, end in prefix))
    delta = tuple(value - old for value, old in zip(absolute, prior))
    print(f"{phase['phase']}: count/sectors/ns={absolute} delta={delta} sectors excluded={excluded}")
    prior = absolute
assert prior == (count, sectors, elapsed)
