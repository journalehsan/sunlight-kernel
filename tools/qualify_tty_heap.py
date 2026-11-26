#!/usr/bin/env python3
"""Exercise native TTY tabs in a fresh, optical-only compressed boot guest."""
import argparse
import json
from pathlib import Path
import time

from desktop_tty_ux_gate import Guest
from qualify_boot_resource_prototype import GATES


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--ram", type=int, required=True)
    parser.add_argument("--firmware", default="")
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--scrollback", action="store_true",
                        help="verify rendered history using framebuffer OCR")
    parser.add_argument("--extended", action="store_true",
                        help="run sustained output and the native-tests shell PTY suite")
    args = parser.parse_args()
    guest = Guest(args.iso, args.out, "std", accel="tcg", offline=True,
                  ram=args.ram, optical_firmware=args.firmware)
    result = {"status": "failed", "command_response_ms": []}

    def command(value, marker):
        guest.type(value)
        count = guest.read().count(marker)
        start = time.monotonic()
        guest.request("send-key", {"keys": [{"type": "qcode", "data": "ret"}],
                                   "hold-time": 60})
        while guest.read().count(marker) <= count:
            if guest.process.poll() is not None or time.monotonic() - start >= 60:
                raise RuntimeError(f"Guest timeout: {marker}")
            time.sleep(0.01)
        result["command_response_ms"].append({
            "command": value, "wall_ms": round((time.monotonic() - start) * 1000),
        })
        time.sleep(max(0, 0.15 - (time.monotonic() - start)))

    try:
        for marker in (*GATES, "[TTY]  Login screen ready", "[PTY] registered",
                       "[DOC-HEAP] workload PASS"):
            guest.wait(marker)
        result["boot_and_document"] = True
        if not args.baseline:
            guest.wait("[TTY-HEAP] growth probe PASS", timeout=5)
        guest.key("ret")
        guest.type("root")
        guest.key("tab")
        guest.key("spc")
        guest.key("ret")
        guest.wait("[TTY]  Built-in shell ready", timeout=30)
        command("echo tabone", "[TTY]  cmd: echo tabone -> tabone")
        guest.key("ctrl", "t")
        guest.wait("[TTY]  New authenticated shell tab OK", timeout=30)
        command("echo tabtwo", "[TTY]  cmd: echo tabtwo -> tabtwo")
        for path in ("/etc/locale.gen", "/tests/cat-big", "/tests/cat-big"):
            command(f"cat {path}", f"[TTY]  exit: cat {path} -> 0")
        if args.scrollback:
            guest.key("ctrl", "1")
            command("echo ancient meadow", "[TTY]  cmd: echo ancient meadow -> ancient meadow")
            command("ls /bin", "[TTY]  exit: ls /bin -> 0")
            command("echo fresh garden", "[TTY]  cmd: echo fresh garden -> fresh garden")
            guest.key("home")
            history = guest.capture("scrollback-home").lower().splitlines()
            guest.key("end")
            tail = guest.capture("scrollback-end").lower().splitlines()
            if "ancient meadow" not in history or "fresh garden" not in tail:
                raise RuntimeError("Expected rendered scrollback lines missing")
            if "fresh garden" in history or "ancient meadow" in tail:
                raise RuntimeError("Rendered history and tail were not distinct")
            result["rendered_scrollback"] = True
            guest.key("ctrl", "2")
        if args.extended:
            for _ in range(12):
                command("ls /bin", "[TTY]  exit: ls /bin -> 0")
            command("sshl --pty-liveness-tests", "[TTY]  exit: sshl --pty-liveness-tests -> 0")
            expected = Path(__file__).with_name("tests") / "shell_pty_liveness.expected"
            for marker in expected.read_text().splitlines():
                guest.wait(marker, timeout=3)
            result["pty_liveness"] = True
        guest.key("pgup")
        guest.key("home")
        guest.key("end")
        guest.key("ctrl", "1")
        command("echo tabonealive", "[TTY]  cmd: echo tabonealive -> tabonealive")
        guest.key("ctrl", "2")
        command("echo tabtwoalive", "[TTY]  cmd: echo tabtwoalive -> tabtwoalive")
        guest.key("ctrl", "w")
        time.sleep(2)
        command("echo recovered", "[TTY]  cmd: echo recovered -> recovered")
        if "panic stage=runtime" in guest.read() or "[KERNEL PANIC]" in guest.read():
            raise RuntimeError("Guest panic")
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        (guest.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        guest.close()


if __name__ == "__main__":
    main()
