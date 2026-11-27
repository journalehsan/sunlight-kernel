#!/usr/bin/env python3
"""Run the isolated probe using a freshly created disk and a separate boot ISO."""
import argparse
import pathlib
import re
import shutil
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--n1i", action="store_true", help="expect the N1I lease probe")
    modes.add_argument("--n1j", action="store_true", help="expect the N1J live endpoint probe")
    modes.add_argument("--n1m", action="store_true", help="expect native service block requests")
    modes.add_argument("--n1o", action="store_true", help="qualify the native FAT32 attachment")
    parser.add_argument("--fsinfo", choices=("populated", "unknown"), default="unknown",
                        help="initial FSInfo hints for the native FAT32 probe")
    parser.add_argument("--check-n1o-artifacts", type=pathlib.Path,
                        help="verify a previously closed native FAT32 probe directory")
    args = parser.parse_args()
    if args.check_n1o_artifacts:
        check_n1o(args.check_n1o_artifacts, args.fsinfo)
        return
    attestation = (
        b"\n[VFS-N1M] PASS cycles=64 data=129 barriers=129 image_retired=1 pending=0 mapping_users=0 reset=VerifiedRetained"
        if args.n1m else
        b"\n[VFS-N1J] PASS cycles=64 data=129 barriers=129 image_retired=1 pending=0 mapping_users=0 reset=VerifiedRetained"
        if args.n1j else
        b"\n[VFS-N1I] PASS cycles=64 data=128 barriers=192 pending=0 mapping_users=0 reset=VerifiedRetained"
        if args.n1i else
        b"\n[VFS-N1H] PASS cycles=64 completed=193 pending=0 mapping_users=0 barriers=64"
    )
    if args.n1o:
        attestation = b"\n[VFS-N1O] PASS ring3 FAT lifecycle cycles=4"
    target = pathlib.Path("target")
    stage = "n1o" if args.n1o else "n1m" if args.n1m else "n1j" if args.n1j else "n1i" if args.n1i else "n1h"
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"vfs-{stage}-qemu-", dir=target))
    print(work, flush=True)
    elf = target / "x86_64-unknown-none/debug/sunlight-kernel"
    iso = work / "probe.iso"
    disk = work / "disposable.raw"
    serial = work / "serial.log"
    marker = b"VFS-N1H-DISPOSABLE-ONLY!"
    expected = bytearray(8192 * 512)
    if args.n1o:
        offset = 2048 * 512
        prefix = b"VFS-N1O-DISPOSABLE-ONLY!".ljust(offset, b"\0")
        volume = work / "initial.fat"
        keep = work / "KEEP.TXT"
        keep.write_bytes(b"untouched")
        subprocess.run(["mkfs.fat", "-F", "32", "-s", "1", "-C", str(volume),
                        str((131072 - 2048) // 2)], check=True)
        subprocess.run(["mcopy", "-i", str(volume), str(keep), "::KEEP.TXT"], check=True)
        with volume.open("r+b") as fat:
            boot = fat.read(512)
            info = int.from_bytes(boot[48:50], "little")
            backup = int.from_bytes(boot[50:52], "little")
            fat.seek(info * 512 + 488)
            hints = fat.read(8)
            if args.fsinfo == "unknown":
                hints = b"\xff" * 8
            for sector in (info, backup + info):
                fat.seek(sector * 512)
                assert fat.read(4) == b"RRaA"
                fat.seek(sector * 512 + 488)
                fat.write(hints)
        with disk.open("wb") as image, volume.open("rb") as fat:
            image.write(prefix)
            shutil.copyfileobj(fat, image)
    else:
        expected[:len(marker)] = marker
        disk.write_bytes(expected)
    with (work / "iso.log").open("wb") as log:
        subprocess.run(
            ["xorriso", "-indev", str(target / "sunlightos.iso"), "-outdev", str(iso),
             "-map", str(elf), "/boot/sunlight-kernel.elf", "-boot_image", "any", "replay"],
            stdout=log, stderr=subprocess.STDOUT, check=True,
        )
        subprocess.run(
            [str(target / "limine/bin/limine"), "bios-install", str(iso)],
            stdout=log, stderr=subprocess.STDOUT, check=True,
        )
    with (work / "qemu.log").open("wb") as log, subprocess.Popen(
        ["qemu-system-x86_64", "-m", "1024M", "-smp", "1", "-cpu", "max",
         "-display", "none", "-monitor", "none", "-no-reboot", "-nic", "none",
         "-boot", "d", "-cdrom", str(iso), "-serial", f"file:{serial}",
         "-drive", f"file={disk},format=raw,if=none,id=n1h,cache=writeback",
         "-device", "virtio-blk-pci,drive=n1h,disable-modern=on"],
        stdout=log, stderr=subprocess.STDOUT,
    ) as guest:
        try:
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline and guest.poll() is None:
                if serial.exists():
                    output = serial.read_bytes()
                    if attestation in output:
                        if args.n1m and b"[VFS-N1M] ring3 readback and release cycles complete" not in output:
                            raise RuntimeError(f"missing native service attestation: {serial}")
                        break
                    if b"KERNEL PANIC" in output or b"[VFS-N1O] ring3 panic" in output:
                        raise RuntimeError(f"guest panicked: {serial}")
                time.sleep(0.1)
            else:
                raise RuntimeError(f"guest did not attest settlement: {serial}")
        finally:
            if guest.poll() is None:
                guest.terminate()
                try:
                    guest.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    guest.kill()
                    guest.wait()
    if args.n1o:
        check_n1o(work, args.fsinfo)
        return
    for cycle in range(64):
        start = (128 + cycle) * 512
        expected[start:start + 512] = bytes(cycle ^ (index & 255) for index in range(512))
    if disk.read_bytes() != expected:
        raise RuntimeError(f"backing image differs from expected full image: {disk}")
    print("PASS: guest settlement and full backing image")


def check_n1o(work, fsinfo):
    output = (work / "serial.log").read_bytes()
    assert b"\n[VFS-N1O] PASS ring3 FAT lifecycle cycles=4" in output
    assert output.count(b"[VFS-N1P] ring3 FAT extended truncated unlinked") == 1
    offset = 2048 * 512
    prefix = b"VFS-N1O-DISPOSABLE-ONLY!".ljust(offset, b"\0")
    disk = work / "disposable.raw"
    volume = work / "initial.fat"
    settled = re.findall(
        rb"\[VFS-N1O\] settled lease=(\d+) pending=0 mapping_users=0 queue=(\S+) bounce=(\S+) retained_bytes=20480",
        output,
    )
    assert len(settled) == 4 and len({row[1:] for row in settled}) == 1
    generations = [int(row[0]) for row in settled]
    assert generations == sorted(set(generations))
    events = re.findall(
        rb"\[VFS-N1O\] (ring3 FAT initialized|ring3 N1D published|settled lease=\d+|ring3 topology withdrawn|released lease=\d+)",
        output,
    )
    assert events == [
        event for generation in generations for event in (
            b"ring3 FAT initialized", b"ring3 N1D published", b"settled lease=%d" % generation,
            b"ring3 topology withdrawn", b"released lease=%d" % generation,
        )
    ]
    final = work / "final.fat"
    assert disk.stat().st_size == 131072 * 512
    with disk.open("rb") as image:
        assert image.read(offset) == prefix
        after = image.read()
    if final.exists():
        assert final.read_bytes() == after
    else:
        final.write_bytes(after)
    subprocess.run(["fsck.fat", "-n", str(volume)], check=True)
    subprocess.run(["fsck.fat", "-n", str(final)], check=True)
    for name, content in (("NEW.TXT", b"native FAT32 attachment"), ("KEEP.TXT", b"untouched")):
        actual = subprocess.check_output(["mtype", "-i", str(final), f"::{name}"])
        assert actual == content
    before = volume.read_bytes()
    assert len(before) == len(after)
    changed = [pos // 512 for pos in range(0, len(before), 512)
               if before[pos:pos + 512] != after[pos:pos + 512]]
    reserved = int.from_bytes(before[14:16], "little")
    fat_size = int.from_bytes(before[36:40], "little")
    root = reserved + before[16] * fat_size
    assert before[13] == 1 and before[16] == 2
    assert int.from_bytes(before[44:48], "little") == 2
    count = (len(before) // 512) - root
    fat_bytes = before[reserved * 512: (reserved + fat_size) * 512]
    free_before = sum(not (int.from_bytes(fat_bytes[i*4:i*4+4], "little") & 0xfffffff)
                      for i in range(2, count + 2))
    info = int.from_bytes(before[48:50], "little")
    backup = int.from_bytes(before[50:52], "little")
    for lba in (info, backup + info):
        hints = before[lba * 512 + 488:lba * 512 + 496]
        if fsinfo == "populated":
            assert int.from_bytes(hints[:4], "little") == free_before
        else:
            assert hints == b"\xff" * 8
    assert all(0xffffff8 <= int.from_bytes(fat_bytes[i:i+4], "little") <= 0xfffffff
               for i in (8, 12))
    assert fat_bytes[16:24] == bytes(8)
    expected = bytearray(before)
    for lba in (info, backup + info):
        expected[lba * 512 + 488:lba * 512 + 496] = b"\xff" * 8
    for lba in (reserved, reserved + fat_size):
        expected[lba * 512 + 16:lba * 512 + 20] = bytes.fromhex("f8ffff0f")
    entry = root * 512 + 32
    assert before[root * 512:root * 512 + 11] == b"KEEP    TXT"
    assert before[entry:entry + 32] == bytes(32)
    payload = b"native FAT32 attachment"
    expected[entry:entry + 11] = b"NEW     TXT"
    expected[entry + 26:entry + 28] = (4).to_bytes(2, "little")
    expected[entry + 28:entry + 32] = len(payload).to_bytes(4, "little")
    expected[(root + 2) * 512:(root + 3) * 512] = payload.ljust(512, b"\0")
    expected[(root + 3) * 512:(root + 4) * 512] = b"\x6b" * 512
    assert after == expected, f"unexpected image bytes; changed FAT LBAs {changed}"
    print(f"PASS: {fsinfo} FSInfo, four native FAT cycles, stable DMA, "
          f"preserved prefix, free clusters {free_before} -> {free_before - 1}; "
          f"changed FAT LBAs {changed}")


if __name__ == "__main__":
    main()
