"""Run the existing SFS5B4 raw-block control on a new unpartitioned file."""
import hashlib
import json
from pathlib import Path
import re
import stat
import subprocess
import sys
import time


def main():
    iso = Path(sys.argv[1]).resolve(strict=True)
    out = Path(sys.argv[2]).resolve()
    assert iso.is_file()
    out.mkdir()
    initial = bytearray(8192 * 512)
    marker = b"SFS5A-ONLY-7b93143a-disposable-v1!"
    initial[:len(marker)] = marker
    image = out / "disposable.img"
    with image.open("xb") as stream:
        stream.write(initial)
    info = image.lstat()
    assert stat.S_ISREG(info.st_mode) and info.st_nlink == 1
    assert info.st_size == len(initial) and image.read_bytes() == initial
    allowed = {62: 0x62, 63: 0xB4}
    assert all(0 < lba < info.st_size // 512 for lba in allowed)
    serial = out / "serial.log"
    command = [
        "qemu-system-x86_64", "-nodefaults", "-machine", "q35", "-accel", "tcg",
        "-m", "4096", "-smp", "4", "-cdrom", str(iso), "-boot", "d",
        "-vga", "std", "-display", "none", "-monitor", "none", "-serial", f"file:{serial}",
        "-drive", f"id=fixture,file={image},if=none,format=raw,cache=writeback",
        "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=fixture",
        "-device", "virtio-rng-pci,disable-modern=on", "-nic", "none",
        "-no-reboot", "-no-shutdown",
    ]
    for event in ("handle_write", "rw_complete", "req_complete"):
        command.extend(["-trace", f"enable=virtio_blk_{event}"])
    (out / "command.json").write_text(json.dumps(dict(
        argv=command, allowed_lbas=list(allowed),
        initial_sha256=hashlib.sha256(initial).hexdigest()), indent=2))
    ready = False
    with (out / "qemu.log").open("x") as trace, subprocess.Popen(
        command, stdout=trace, stderr=trace
    ) as guest:
        try:
            deadline = time.monotonic() + 90
            while guest.poll() is None and time.monotonic() < deadline:
                if serial.exists():
                    log = serial.read_text(errors="replace")
                    if "[SFS5B4] READY" in log:
                        ready = True
                        break
                    if "panicked at" in log:
                        break
                time.sleep(0.05)
        finally:
            if guest.poll() is None:
                guest.terminate()
            guest.wait(timeout=10)

    pending = {}
    completed = []
    submitted = []
    for line in (out / "qemu.log").read_text().splitlines():
        match = re.search(r"virtio_blk_handle_write vdev (\S+) req (\S+) sector (\d+) nsectors (\d+)", line)
        if match:
            dev, token, lba, sectors = match.groups()
            request = (int(lba), int(sectors))
            assert request[0] in allowed and request[1] == 1, line
            assert (dev, token) not in pending, line
            pending[dev, token] = request
            submitted.append(request)
        match = re.search(r"virtio_blk_req_complete vdev (\S+) req (\S+) status (\d+)", line)
        if match:
            dev, token, status = match.groups()
            request = pending.pop((dev, token), None)
            if request and status == "0":
                completed.append(request)
    actual = image.read_bytes()
    changed = [lba for lba in range(8192)
               if actual[lba * 512:(lba + 1) * 512] != initial[lba * 512:(lba + 1) * 512]]
    print(f"ready={ready} submitted={submitted} completed={completed} changed={changed}")
    print(f"final_sha256={hashlib.sha256(actual).hexdigest()}")
    for lba, fill in allowed.items():
        initial[lba * 512:(lba + 1) * 512] = bytes([fill]) * 512
    assert ready and completed == submitted == [(62, 1), (63, 1)] and not pending
    assert actual == initial


if __name__ == "__main__":
    main()
