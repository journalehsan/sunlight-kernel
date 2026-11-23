#!/usr/bin/env python3
"""Run pinned plain btop, a musl probe, and native reference in the real TTY."""
import argparse
import os
from pathlib import Path
import re
import shutil
import sys
import time

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "target/btop-01"))
from qmp_fixture import Guest

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("iso", type=Path)
parser.add_argument("evidence", type=Path)
modes = parser.add_mutually_exclusive_group()
modes.add_argument("--tasks-monitor", action="store_true", help="open the native Tasks Monitor on the desktop")
modes.add_argument("--network", action="store_true", help="isolated QEMU gateway ping, no external networking")
opts = parser.parse_args()
out = opts.evidence.resolve()
out.mkdir(parents=True, exist_ok=False)
os.environ["BTOP_TCG"] = "1"
os.environ.pop("BTOP_DISK", None)
if opts.network:
    # Give the original fixture an isolated slirp peer and retain exact argv.
    executable = shutil.which("qemu-system-x86_64")
    if not executable:
        raise SystemExit("QEMU not found")
    wrapper = out / "qemu-system-x86_64"
    wrapper.write_text(f"""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = [{executable!r}, *sys.argv[1:],
        "-netdev", "user,id=testnet,restrict=on",
        "-device", "virtio-net-pci,netdev=testnet,disable-modern=on",
        "-object", "filter-dump,id=frames,netdev=testnet,file={out / "network.pcap"}"]
Path({str(out / "command.json")!r}).write_text(json.dumps(args, indent=2))
os.execv(args[0], args)
""")
    wrapper.chmod(0o755)
    os.environ["PATH"] = f"{out}:{os.environ['PATH']}"

g = Guest(opts.iso, out)

def cmd(s, wait=True):
    before = g.read().count(f"[TTY]  exit: {s} -> ")
    for part in re.split("([_:])", s):
        if part in ("_", ":"):
            g.key("shift", "minus" if part == "_" else "semicolon")
        elif part:
            g.type(part)
    g.key("ret")
    if wait:
        g.wait(f"[TTY]  exit: {s} -> ", before + 1, timeout=35)

try:
    g.wait("[TTY]  Login screen ready", timeout=90)
    time.sleep(4)
    g.key("ret")
    g.type("root")
    if opts.tasks_monitor:
        # Leave the default desktop mode selected at login.
        g.key("ret")
        g.wait("[SESSION] switched to F2 GraphicalDesktop", timeout=45)
        time.sleep(4)
        g.open_search("tasks")
        g.key("ret")
        g.wait("app=tasks phase=app_main_started", timeout=35)
        time.sleep(3)
        g.capture("02-tasks-monitor")
        time.sleep(2)
        g.capture("03-tasks-monitor")
    else:
        g.key("tab")
        g.key("spc")
        g.key("ret")
        g.wait("[TTY]  Built-in shell ready", timeout=45)
        g.capture("01-tty-ready")
    if opts.network:
        cmd("/bin/btop-reference net")
        cmd("ping 10.0.2.2")
        cmd("/bin/btop-reference net")
        g.capture("02-network")
    elif not opts.tasks_monitor:
        for setting in ("TERM=xterm", "LANG=C.UTF-8", "XDG_CONFIG_HOME=/tmp/btopfixture",
                        "XDG_STATE_HOME=/tmp/btopfixture"):
            cmd("export " + setting, wait=False)
            time.sleep(1)
        cmd("/bin/btop --version")
        g.capture("02-version")
        cmd("/bin/btop", wait=False)
        for i in range(3):
            time.sleep(2)
            g.capture(f"03-dashboard-{i}")
        g.key("h")
        g.capture("04-help")
        g.key("esc")
        g.key("q")
        # Preserve the trace even when pselect or a worker fault prevents a
        # successful quit. The shell's reported exit is checked separately.
        try:
            g.wait("[TTY]  exit: /bin/btop -> ", timeout=20)
        except RuntimeError:
            g.capture("05-no-exit")
            raise
        cmd("echo btopafter", wait=False)
        time.sleep(1)
        match = re.search(r"\[EXEC\] /bin/btop fg pid=(\d+)", g.read())
        if not match:
            raise RuntimeError("btop foreground PID absent")
        ids = re.findall(r"\[EXEC\] /bin/btop fg pid=(\d+)", g.read())
        cmd(f"/bin/btop-reference {ids[-1]}")
        cmd("/bin/proc-native")
        cmd("cat /tmp/btopfixture/btop.log")
        g.capture("05-after")
except Exception:
    g.capture("failure")
    raise
finally:
    g.close()
