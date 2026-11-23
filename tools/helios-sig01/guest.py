#!/usr/bin/env python3
"""Read the SIG-01 verdicts through the real guest shell and serial console."""
import argparse
import os
from pathlib import Path
import re
import sys
import time

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "target/btop-01"))
from qmp_fixture import Guest

cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument("iso", type=Path)
cli.add_argument("evidence", type=Path)
args = cli.parse_args()
args.evidence.mkdir(parents=True, exist_ok=False)
os.environ["BTOP_TCG"] = "1"
os.environ.pop("BTOP_DISK", None)
guest = Guest(args.iso, args.evidence)
try:
    guest.wait("[TTY]  Login screen ready")
    time.sleep(4)
    guest.key("ret")
    guest.type("root")
    for key in ("tab", "spc", "ret"):
        guest.key(key)
    guest.wait("[TTY]  Built-in shell ready", timeout=45)
    guest.wait("[TTY-IPC] shell cap lookup succeeded", timeout=20)
    for mode in ("", "selector", "flags", "rip", "noncanon", "rsp", "fpptr", "fpstate", "altstack", "noactive"):
        command = "/bin/sig01" + (" " + mode if mode else "")
        guest.type(command)
        guest.key("ret")
        try:
            guest.wait(f"[TTY]  exit: {command} -> ", timeout=25)
        except TimeoutError:
            guest.capture(f"timeout-{mode or 'roundtrip'}")
            raise
        log = guest.read()
        exit_match = re.search(rf"\[TTY\]  exit: {re.escape(command)} -> (\d+)", log)
        output = next(line for line in log.splitlines() if f"[TTY]  cmd: {command} -> " in line)
        print(output, exit_match[1], flush=True)
        if not mode:
            guest.capture("roundtrip")
            assert int(exit_match[1]) == 0 and "SIG01 PASS" in output
            frames = re.findall(
                r"\[SIG01\] (timer-delivery|sigreturn) (tid=.*)", log
            )
            assert [event for event, _ in frames] == ["timer-delivery", "sigreturn"]
            assert frames[0][1] == frames[1][1]
            assert "rcx=0x123456789abcdef0 r11=0xfedcba9876543210" in frames[0][1]
            assert "masks=800/a02/800/800" in output and "uc=6" in output
        else:
            marker = "no-active armed" if mode == "noactive" else "bad-frame armed"
            assert int(exit_match[1]) == 139 and marker in output
    assert not re.search(r"\[FAULT\]|Kernel panic", guest.read()[log.index("[TTY]  Built-in shell ready"):])
finally:
    guest.close()
