#!/usr/bin/env python3
"""Run the masked or BTop-03 unmasked gate through the authenticated guest TTY."""
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
cli.add_argument("--unmasked", action="store_true")
args = cli.parse_args()
program = "tty-select" if args.unmasked else "masked-select"
args.evidence.mkdir(parents=True, exist_ok=False)
os.environ["BTOP_TCG"] = "1"
os.environ.pop("BTOP_DISK", None)
guest = Guest(args.iso, args.evidence)
try:
    guest.wait("[TTY]  Login screen ready")
    time.sleep(4)
    guest.key("ret")
    guest.type("root")
    guest.key("tab")
    guest.key("spc")
    guest.key("ret")
    guest.wait("[TTY]  Built-in shell ready", timeout=45)
    guest.wait("[TTY-IPC] shell cap lookup succeeded", timeout=20)
    guest.capture("00-shell")
    guest.type(f"/bin/{program}")
    guest.key("ret")
    deadline = time.monotonic() + 25
    injected = False
    while time.monotonic() < deadline:
        log = guest.read()
        # The live pselect trace shows entry into the parked TTY wait.
        trace = log
        if args.unmasked:
            invalid = re.search(r"nr=270 .* result=-9 ", log)
            trace = log[invalid.end():] if invalid else ""
        if not injected and re.search(r"nr=270 a0=0x1 .* result=270 ", trace):
            guest.key("x")
            injected = True
        result = re.search(rf"\[TTY\]  exit: /bin/{program} -> (\d+)", log)
        if result:
            break
        time.sleep(0.05)
    else:
        guest.capture("02-timeout")
        raise TimeoutError(f"{program} did not exit")
    guest.capture("01-result")
    print(next(line for line in log.splitlines() if f"[TTY]  cmd: /bin/{program} ->" in line))
    if args.unmasked:
        assert injected and re.search(r"select key rc=1 read=1 except=0 .* n=1 char=120", log)
        assert re.search(r"select invalid rc=-1 errno=9 unchanged=1", log)
    sys.exit(int(result[1]))
finally:
    guest.close()
