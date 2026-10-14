#!/usr/bin/env python3
"""Repeat a prebuilt guest gate; every boot and deadline gets its own result.

Build once with tools/test.sh shell-pty-liveness, then use a copied ISO so a
later build cannot change the image beneath this matrix. No failed boot is
retried or excluded. Guest assertions check stream contents and I/O semantics.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
STREAM_SUMMARY = re.compile(
    rb"\[SSHL-PTY\] stream bytes=(\d+) read_size=(\d+) slow=(true|false) "
    rb"reads=(\d+) empty=(\d+) elapsed_ms=(\d+) sleep_ms=(\d+) sleeps=(\d+) "
    rb"ipc_ms=(\d+) ipc_max_ms=(\d+) PASS")


def failure_snapshot(path, output, cpus):
    """Stop only a failed guest, then retain registers and kernel stack words."""
    records = []
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(1)
            connection.connect(str(path))
            with connection.makefile("rwb", buffering=0) as stream:
                records.append(json.loads(stream.readline()))

                def execute(command, arguments=None):
                    request = {"execute": command}
                    if arguments is not None:
                        request["arguments"] = arguments
                    stream.write(json.dumps(request).encode() + b"\n")
                    while True:
                        reply = json.loads(stream.readline())
                        records.append({"request": request, "reply": reply})
                        if "return" in reply or "error" in reply:
                            return

                execute("qmp_capabilities")
                execute("stop")
                for command in ["info cpus", "info registers -a"]:
                    execute("human-monitor-command", {"command-line": command})
                for cpu in range(cpus):
                    execute("human-monitor-command", {"command-line": f"cpu {cpu}"})
                    execute("human-monitor-command", {"command-line": "x /32gx $rsp"})
    except (OSError, ValueError) as error:
        records.append({"snapshot_error": str(error)})
    output.write_text(json.dumps(records, indent=2) + "\n")


def boot(iso, cpus, run, directory, phase, deadline, expected, qmp_snapshot=False, memory_mb=1024):
    stem = directory / f"{phase}-cpu{cpus}-run{run}"
    serial = stem.with_suffix(".serial.log")
    final = expected[-1].encode()
    matched = set()
    streams = []
    last_stream_offset = -1
    status = "TIMEOUT"
    start = time.monotonic()
    qmp_directory = tempfile.TemporaryDirectory(prefix="sshl-qmp-")
    qmp_path = Path(qmp_directory.name) / "qmp.sock"
    command = ["qemu-system-x86_64", "-cdrom", str(iso), "-serial", f"file:{serial}",
               "-display", "none", "-m", f"{memory_mb}M", "-smp", str(cpus),
               "-device", "virtio-rng-pci,disable-modern=on",
               "-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0",
               "-no-reboot", "-no-shutdown"]
    if qmp_snapshot:
        command.extend(["-qmp", f"unix:{qmp_path},server=on,wait=off"])
    if os.access("/dev/kvm", os.R_OK | os.W_OK):
        command.append("-enable-kvm")
    with stem.with_suffix(".qemu.log").open("wb") as diagnostics:
        process = subprocess.Popen(command, cwd=ROOT, stdout=diagnostics, stderr=diagnostics)
        try:
            offset = 0
            tail = b""
            while time.monotonic() - start < deadline:
                if serial.exists():
                    with serial.open("rb") as stream:
                        stream.seek(offset)
                        chunk = stream.read(65536)
                        offset += len(chunk)
                    window = tail + chunk
                    window_offset = offset - len(chunk) - len(tail)
                    for sample in STREAM_SUMMARY.finditer(window):
                        position = window_offset + sample.start()
                        if position <= last_stream_offset:
                            continue
                        last_stream_offset = position
                        if len(streams) < 32:
                            values = sample.groups()
                            streams.append({
                                "bytes_verified": int(values[0]), "read_size": int(values[1]),
                                "slow": values[2] == b"true", "reads": int(values[3]),
                                "empty_reads": int(values[4]), "elapsed_ms": int(values[5]),
                                "sleep_ms": int(values[6]), "sleeps": int(values[7]),
                                "ipc_ms": int(values[8]), "ipc_max_ms": int(values[9]),
                            })
                    for marker in expected:
                        if marker.encode() in window:
                            matched.add(marker)
                    if b"[SSHL-TEST] FAIL" in window or b"Kernel Panic" in window:
                        status = "FAIL"
                        break
                    if final in window:
                        status = "PASS" if len(matched) == len(expected) else "MISSING_MARKERS"
                        break
                    tail = window[-4096:]
                if process.poll() is not None:
                    status = "QEMU_EXIT"
                    break
                time.sleep(0.25)
        finally:
            if qmp_snapshot and status != "PASS" and process.poll() is None:
                failure_snapshot(qmp_path, stem.with_suffix(".qmp.json"), cpus)
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            qmp_directory.cleanup()
    result = {"cpus": cpus, "run": run, "status": status, "memory_mb": memory_mb,
              "elapsed_s": round(time.monotonic() - start, 2),
              "deadline_s": deadline, "serial": str(serial),
              "streams": streams,
              "missing": [marker for marker in expected if marker not in matched]}
    print(json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", type=Path, required=True)
    parser.add_argument("--cpus", type=int, nargs="+", default=[1, 2, 4])
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--memory-mb", type=int, default=1024,
                        help="Explicit guest RAM; default 1024 MiB")
    parser.add_argument("--deadline", type=int, default=300)
    parser.add_argument("--qmp-snapshot", action="store_true",
                        help="Capture CPU registers/stacks on failure; requires socket permission")
    parser.add_argument("--phase", choices=["shell-pty-liveness", "shell-phase2", "shell-phase1.5"], default="shell-pty-liveness")
    parser.add_argument("--output", type=Path, default=ROOT / "target/pty-liveness-matrix")
    args = parser.parse_args()
    if args.runs < 1 or args.deadline < 1 or args.memory_mb < 1 or any(cpu < 1 for cpu in args.cpus):
        parser.error("CPU counts, repetitions, deadlines and memory must be positive")
    iso = args.iso.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256(iso.read_bytes()).hexdigest()
    expected = (ROOT / "tools/tests" / {
        "shell-pty-liveness": "shell_pty_liveness.expected",
        "shell-phase2": "shell_phase2.expected",
        "shell-phase1.5": "shell_phase1_5.expected",
    }[args.phase]).read_text().splitlines()
    expected = [line for line in expected if line and not line.startswith("#")]
    results = []
    for cpu in args.cpus:
        for run in range(1, args.runs + 1):
            results.append(boot(iso, cpu, run, args.output, args.phase, args.deadline, expected,
                                args.qmp_snapshot, args.memory_mb))
            (args.output / "results.json").write_text(json.dumps(
                {"iso": str(iso), "sha256": digest, "phase": args.phase,
                 "expected": expected, "results": results}, indent=2) + "\n")
    return int(any(result["status"] != "PASS" for result in results))


if __name__ == "__main__":
    raise SystemExit(main())
