#!/usr/bin/env python3
"""Record the first launch outcome through the real SunApp launcher."""
import argparse
import json
from pathlib import Path

from desktop_tty_ux_gate import Guest
from helios_wayland_gate import capture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("accelerator", choices=("tcg", "kvm"))
    args = parser.parse_args()
    if args.accelerator == "kvm" and not Path("/dev/kvm").exists():
        parser.error("KVM unavailable: /dev/kvm does not exist")
    guest = Guest(args.iso, args.output, accel=args.accelerator)
    result = {"accelerator": args.accelerator, "vcpus": 2, "launch": "unverified"}
    try:
        guest.wait("[QALCULATE-PROBE] launch failed:")
        serial = guest.read()
        attempt = serial.split("[QALCULATE-PROBE] launch /Applications/Qalculate.sunapp", 1)[1]
        assert "phase=spawn_entered" in attempt, "manifest did not resolve through to spawn"
        assert "[ELF] header rejected: NotStaticExecutable" in attempt, "unexpected launch blocker"
        assert "[QALCULATE-PROBE] launch failed: SpawnFailed(" in attempt, "missing launcher outcome"
        result.update(launch="blocked", first_blocker="ELF execution",
                      manifest="resolved", app_exit_status=None,
                      qt_plugin_loaded=False, app_connected_to_wayland=False)
        capture(guest, "after-rejection")
    finally:
        (guest.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        guest.close()


if __name__ == "__main__":
    main()
