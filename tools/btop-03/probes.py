#!/usr/bin/env python3
"""Replay independent probes through the authenticated framebuffer TTY."""
import argparse
import os
from pathlib import Path
import re
import sys
import time

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "target/btop-01"))
from qmp_fixture import Guest

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("iso", type=Path)
parser.add_argument("evidence", type=Path)
args = parser.parse_args()
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

    guest.type("/bin/ifaddrs")
    guest.key("ret")
    guest.wait("[TTY]  exit: /bin/ifaddrs -> ", timeout=20)
    guest.capture("01-ifaddrs")

    guest.type("/bin/tty-select")
    guest.key("ret")
    # Foreground stdout is replayed to serial at exit. The invalid-fd
    # syscall immediately precedes the blocking key test, so synchronize on
    # its live trace and give the probe time to park.
    deadline = time.monotonic() + 20
    while not re.search(r"nr=270 .* result=-9 ", guest.read()):
        if time.monotonic() >= deadline:
            raise RuntimeError("missing live invalid-fd trace before key wait")
        time.sleep(0.05)
    time.sleep(0.5)
    guest.key("x")
    guest.wait("[TTY]  exit: /bin/tty-select -> ", timeout=20)
    guest.capture("02-select")
    log = guest.read()
    if not re.search(r"select key rc=1 read=1 except=0 .* n=1 char=120", log):
        raise RuntimeError("missing successful key wait evidence")
    if "[TTY]  exit: /bin/tty-select -> 0" not in log:
        raise RuntimeError("TTY readiness probe failed")
finally:
    guest.close()
