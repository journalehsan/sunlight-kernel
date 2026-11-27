#!/usr/bin/env python3
"""Run the isolated probe using a freshly created disk and a separate boot ISO."""
import argparse
import pathlib
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--n1i", action="store_true", help="expect the N1I lease probe")
    modes.add_argument("--n1j", action="store_true", help="expect the N1J live endpoint probe")
    modes.add_argument("--n1m", action="store_true", help="expect native service block requests")
    args = parser.parse_args()
    attestation = (
        b"\n[VFS-N1M] PASS cycles=64 data=129 barriers=129 image_retired=1 pending=0 mapping_users=0 reset=VerifiedRetained"
        if args.n1m else
        b"\n[VFS-N1J] PASS cycles=64 data=129 barriers=129 image_retired=1 pending=0 mapping_users=0 reset=VerifiedRetained"
        if args.n1j else
        b"\n[VFS-N1I] PASS cycles=64 data=128 barriers=192 pending=0 mapping_users=0 reset=VerifiedRetained"
        if args.n1i else
        b"\n[VFS-N1H] PASS cycles=64 completed=193 pending=0 mapping_users=0 barriers=64"
    )
    target = pathlib.Path("target")
    stage = "n1m" if args.n1m else "n1j" if args.n1j else "n1i" if args.n1i else "n1h"
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"vfs-{stage}-qemu-", dir=target))
    print(work, flush=True)
    elf = target / "x86_64-unknown-none/debug/sunlight-kernel"
    iso = work / "probe.iso"
    disk = work / "disposable.raw"
    serial = work / "serial.log"
    marker = b"VFS-N1H-DISPOSABLE-ONLY!"
    expected = bytearray(8192 * 512)
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
                    if b"KERNEL PANIC" in output:
                        raise RuntimeError(f"kernel panicked: {serial}")
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
    for cycle in range(64):
        start = (128 + cycle) * 512
        expected[start:start + 512] = bytes(cycle ^ (index & 255) for index in range(512))
    if disk.read_bytes() != expected:
        raise RuntimeError(f"backing image differs from expected full image: {disk}")
    print("PASS: guest settlement and full backing image")


if __name__ == "__main__":
    main()
