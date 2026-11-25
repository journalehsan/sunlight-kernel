#!/usr/bin/env python3
"""Record the Qalculate SunApp launch and process outcome from guest serial."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import time

from desktop_tty_ux_gate import Guest
from helios_wayland_gate import capture


PREFIX = "[QALCULATE-PROBE] "
LAUNCH = PREFIX + "launch /Applications/Qalculate.sunapp"
REJECTION = "[ELF] header rejected: NotStaticExecutable"
SPAWN = re.compile(
    r"^launch\[\d+\] source=boot /Applications/Qalculate\.sunapp/Program/\S+"
    r" phase=spawn_entered\b", re.MULTILINE
)
HANDOFF_REJECTION = re.compile(r"\[HELIOS-HANDOFF\] rejected stage=(.+?) pid=\d+")


def observe_launch(guest, timeout):
    deadline = time.monotonic() + timeout
    result = {"launch": "unverified", "spawn_entered": False, "app_exit_status": None}
    while True:
        serial = guest.read()
        _, marker, attempt = serial.partition(LAUNCH)
        if marker:
            result["launch"] = "attempted"
            spawn = SPAWN.search(attempt)
            result["spawn_entered"] = spawn is not None
            child = re.search(r"\[QALCULATE-PROBE\] child started pid=(\d+)\n", attempt)
            if child:
                result.update(launch="started", pid=int(child[1]))
            failure = re.search(r"\[QALCULATE-PROBE\] launch failed: (.+)\n", attempt)
            if failure:
                result.update(launch="failed", error=failure[1])
                if spawn:
                    launch_serial = attempt[spawn.end():failure.start()]
                    handoff = HANDOFF_REJECTION.search(launch_serial)
                    if handoff:
                        result["handoff_stage"] = handoff[1]
                    if REJECTION in launch_serial:
                        result["loader_error"] = "NotStaticExecutable"
                return result, "launch-failed"
            status = re.search(r"\[QALCULATE-PROBE\] child status=(.+)\n", attempt)
            if status:
                code = re.fullmatch(r"Ok\((-?\d+)\)", status[1])
                result.update(launch="started", wait_status=status[1],
                              app_exit_status=int(code[1]) if code else None)
                return result, "child-status"
        if PREFIX + "display unavailable" in serial:
            result["error"] = "display unavailable"
            return result, "display-unavailable"
        if PREFIX + "launcher panic" in serial:
            result["error"] = "launcher panic"
            return result, "launcher-panic"
        if guest.process.poll() is not None:
            result["error"] = "QEMU exited"
            return result, "guest-exited"
        if time.monotonic() >= deadline:
            result["error"] = "timed out waiting for launcher outcome"
            return result, "timeout"
        time.sleep(0.1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("accelerator", choices=("tcg", "kvm"))
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--expect-static-loader-rejection", action="store_true",
                        help="verify the historical static-loader rejection")
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("--timeout must be positive")
    if args.accelerator == "kvm" and not Path("/dev/kvm").exists():
        parser.error("KVM unavailable: /dev/kvm does not exist")
    with args.iso.open("rb") as iso:
        digest = hashlib.file_digest(iso, "sha256").hexdigest()

    guest = Guest(args.iso, args.output, accel=args.accelerator)
    result = {"accelerator": args.accelerator, "vcpus": 2, "image_sha256": digest}
    try:
        observed, outcome = observe_launch(guest, args.timeout)
        result.update(observed, outcome=outcome)
        if guest.process.poll() is None:
            capture(guest, outcome)
        if args.expect_static_loader_rejection:
            passed = (outcome == "launch-failed" and result["spawn_entered"]
                      and result.get("loader_error") == "NotStaticExecutable"
                      and result["error"].startswith("SpawnFailed("))
        else:
            passed = outcome == "child-status" and result["app_exit_status"] == 0
        if not passed:
            raise SystemExit(f"Qalculate guest outcome: {outcome}. See {guest.out / 'result.json'}")
    finally:
        try:
            (guest.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        finally:
            guest.close()


if __name__ == "__main__":
    main()
