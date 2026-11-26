#!/usr/bin/env python3
"""Run fresh optical-only guests and retain the boot milestone evidence."""
import argparse
import json
import subprocess
import time
from pathlib import Path

GATES = (
    "[BOOT-RES] stage=pre-scheduler",
    "[HEAP] Initializing 16 MiB",
    "[BOOT-RES] VFS inventory ready",
    "[SunlightOS] Phase 3.0 OK",
    "[SHM]  VFS read 2048 bytes via shared memory: OK",
    "[init] VFS namespace ready before service spawn",
    "[SWAP-1] policy configured; fault path is kernel-resident",
    "[SESSION-FOUNDATION] SERVICE_READY PASS",
    "[SESSION-CONFIG] SERVICE_READY PASS",
    "[BOOT-RES] stage=session-bundles",
    "[SESSION-CONFIG] ELIGIBLE_BUNDLES PASS",
)


def boot(iso, out, ram, firmware, extra_gates=()):
    stem = out / f"{'uefi' if firmware else 'bios'}-{ram}"
    serial = stem.with_suffix(".serial.log")
    serial.write_text("")
    command = [
        "qemu-system-x86_64", "-accel", "tcg", "-cdrom", str(iso.resolve()),
        "-serial", f"file:{serial.resolve()}", "-m", str(ram), "-smp", "2",
        "-display", "none", "-nic", "none", "-no-reboot", "-no-shutdown",
        "-device", "virtio-rng-pci,disable-modern=on",
        "-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0",
    ]
    if firmware:
        command.extend(["-bios", str(firmware)])
    stem.with_suffix(".args.json").write_text(json.dumps(command, indent=2) + "\n")
    with stem.with_suffix(".qemu.log").open("w") as log:
        with subprocess.Popen(command, stdout=log, stderr=log) as guest:
            status = "timeout"
            deadline = time.monotonic() + 100
            try:
                while time.monotonic() < deadline:
                    output = serial.read_text(errors="replace")
                    if ("[KERNEL PANIC]" in output or
                            "[DOC-HEAP] probe failed:" in output or guest.poll() is not None):
                        status = "failed"
                        break
                    if all(gate in output for gate in GATES + extra_gates):
                        status = "passed"
                        break
                    time.sleep(0.1)
            finally:
                if guest.poll() is None:
                    guest.terminate()
                guest.wait(timeout=10)
    stem.with_suffix(".result").write_text(status + "\n")
    print(f"{stem.name}: {status}", flush=True)
    return status == "passed"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--firmware", type=Path)
    parser.add_argument("--ram", nargs="+", type=int, default=[2048, 1024, 768, 512])
    parser.add_argument("--document-heap", action="store_true")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    extra_gates = ("[DOCSERV] ready", "[DOC-HEAP] workload PASS") if args.document_heap else ()
    results = [boot(args.iso, args.out, ram, args.firmware, extra_gates) for ram in args.ram]
    raise SystemExit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
