#!/usr/bin/env python3
"""Drive the masked wait and btop through the guest's hardware TTY."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sys
import time

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "target/btop-01"))
from qmp_fixture import Guest

cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument("iso", type=Path)
cli.add_argument("evidence", type=Path)
cli.add_argument("--btop", action="store_true")
args = cli.parse_args()
out = args.evidence.resolve()
out.mkdir(parents=True, exist_ok=False)
os.environ["BTOP_TCG"] = "1"
os.environ.pop("BTOP_DISK", None)
if args.btop:
    qemu = shutil.which("qemu-system-x86_64")
    extra = ["-netdev", "user,id=testnet,restrict=on,dhcpstart=10.0.2.42",
             "-device", "virtio-net-pci,netdev=testnet,disable-modern=on",
             "-object", f"filter-dump,id=frames,netdev=testnet,file={out / 'network.pcap'}"]
    wrapper = out / "qemu-system-x86_64"
    wrapper.write_text(f"""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = [{qemu!r}, *sys.argv[1:], *{extra!r}]
Path({str(out / 'network-command.json')!r}).write_text(json.dumps(args, indent=2))
os.execv(args[0], args)
""")
    wrapper.chmod(0o755)
    os.environ["PATH"] = f"{out}:{os.environ['PATH']}"

guest = Guest(args.iso, out)

def command(cmd):
    for part in re.split("(_)", cmd):
        if part == "_":
            guest.key("shift", "minus")
        elif part:
            guest.type(part)
    guest.key("ret")

def match(pattern, timeout=20):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = re.search(pattern, guest.read(), re.M)
        if result:
            print(result[0], flush=True)
            return result
        time.sleep(.03)
    raise RuntimeError(f"Guest timeout: {pattern}")

try:
    guest.wait("[TTY]  Login screen ready")
    time.sleep(4)
    guest.key("ret")
    guest.type("root")
    for key in ("tab", "spc", "ret"):
        guest.key(key)
    guest.wait("[TTY]  Built-in shell ready", timeout=45)
    guest.wait("[TTY-IPC] shell cap lookup succeeded", timeout=20)
    if not args.btop:
        for program, num_deliveries in (("masked-select-original", 2), ("masked-select", 3)):
            command("/bin/" + program)
            pid = match(rf"\[EXEC\] /bin/{program} fg pid=(\d+)")[1]
            match(rf"\[PSELECT\] park tid={pid} cpu=\d+ nfds=1")
            guest.key("x")
            match(rf"\[TTY\]  exit: /bin/{program} -> 0")
            match(rf"\[SCHED\] process_reaped pid={pid}\b")
            log = guest.read()
            parks = list(re.finditer(rf"\[PSELECT\] park tid={pid} cpu=(\d+) nfds=0", log))
            wake = match(rf"\[PSELECT\] enqueue tid={pid} sender=(\d+) cpu=(\d+)"
                         r" state=BlockedOnTimer registered=true wake=true")
            park = next(m for m in reversed(parks) if m.end() < wake.start())
            print(f"waiter CPU={park[1]} sender CPU={wake[2]}", flush=True)
            assert park[1] != wake[2], "signal did not wake a waiter parked on another vCPU"
            assert log.count(f"[SIG01] pselect-delivery tid={pid} ") == num_deliveries
            assert log.count(f"[SIG01] sigreturn tid={pid} ") == num_deliveries
        guest.capture("fixture")
    else:
        for setting in ("TERM=xterm", "LANG=C.UTF-8", "XDG_CONFIG_HOME=/tmp/btopfixture",
                        "XDG_STATE_HOME=/tmp/btopfixture"):
            command("export " + setting)
            time.sleep(.5)
        command("/bin/btop")
        pid = match(r"\[EXEC\] /bin/btop fg pid=(\d+)")[1]
        for i in range(3):
            time.sleep(2)
            guest.capture(f"dashboard-{i}")
        guest.key("h")
        help_text = guest.capture("help")
        assert "Description:" in help_text and "Quits program." in help_text
        guest.key("h")  # Help's own documented close binding.
        for attempt in range(6):
            dashboard = guest.capture(f"dashboard-return-{attempt}")
            if "Description:" not in dashboard and "3.39 GiB" in dashboard:
                break
            time.sleep(.5)
        else:
            raise RuntimeError("Help did not close back to the dashboard")
        guest.key("q")
        match(r"\[TTY\]  exit: /bin/btop -> 0")
        match(rf"\[SCHED\] process_reaped pid={pid}\b")
        command("/bin/proc-native")
        match(r"\[TTY\]  exit: /bin/proc-native -> 0")
        guest.capture("after")
        log = guest.read()
        for path in ("/proc/stat", "/proc/meminfo", f"/proc/{pid}/stat",
                     "/sys/class/net/eth0/statistics/rx_bytes"):
            assert log.count(f'path=Some("{path}")') >= 2, path
finally:
    guest.close()
