#!/usr/bin/env python3
"""Replay btop and an optional storage probe through the guest hardware TTY."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "target/btop-01"))
from qmp_fixture import Guest

cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument("iso", type=Path)
cli.add_argument("evidence", type=Path)
cli.add_argument("--disk", type=Path)
cli.add_argument("--probe", action="store_true")
cli.add_argument("--deny-create", action="store_true",
                 help="compare the BTop-09 musl probe with a native cp create")
cli.add_argument("--readonly", action="store_true",
                 help="attach the verified fixture with a read-only QEMU backend")
cli.add_argument("--allocate", action="store_true",
                 help="copy the probe to the verified disposable FAT partition, sample, and remove it")
args = cli.parse_args()
if args.allocate and not (args.disk and args.probe):
    cli.error("--allocate requires --disk and --probe")
if args.readonly and (not args.disk or args.allocate):
    cli.error("--readonly requires --disk and excludes --allocate")
out = args.evidence.resolve()
out.mkdir(parents=True, exist_ok=False)
os.environ["BTOP_TCG"] = "1"
os.environ.pop("BTOP_DISK", None)
if args.disk:
    disk = args.disk.resolve(strict=True)
    if not disk.is_file() or args.disk.is_symlink() or "," in str(disk):
        cli.error("--disk requires a disposable regular file without commas in its path")
    subprocess.run([sys.executable, str(Path(__file__).with_name("fat-reference.py")),
                    str(disk), *(["--empty"] if args.allocate else [])], check=True)
    os.environ["BTOP_DISK"] = str(disk)
qemu = shutil.which("qemu-system-x86_64")
extra = ["-netdev", "user,id=testnet,restrict=on,dhcpstart=10.0.2.42",
         "-device", "virtio-net-pci,netdev=testnet,disable-modern=on",
         "-object", f"filter-dump,id=frames,netdev=testnet,file={out / 'network.pcap'}"]
wrapper = out / "qemu-system-x86_64"
wrapper.write_text(f"""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = [{qemu!r}, *sys.argv[1:], *{extra!r}]
if {args.readonly!r}:
    args = [arg.replace('id=fixture,snapshot=on', 'id=fixture,readonly=on')
            for arg in args]
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

def match(pattern, timeout=20, start=0):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = re.search(pattern, guest.read()[start:], re.M)
        if result:
            print(result[0], flush=True)
            return result
        time.sleep(.03)
    raise RuntimeError(f"Guest timeout: {pattern}")

def run_command(cmd, status, timeout=25):
    previous = len(guest.read())
    command(cmd)
    return match(rf"\[TTY\]  exit: {re.escape(cmd)} -> {status}", timeout, previous)

def blockstats(phase):
    (out / f"block-{phase}.json").write_text(
        json.dumps(guest.request("query-blockstats"), indent=2))

try:
    guest.wait("[TTY]  Login screen ready")
    time.sleep(4)
    guest.key("ret")
    guest.type("root")
    for key in ("tab", "spc", "ret"):
        guest.key(key)
    guest.wait("[TTY]  Built-in shell ready", timeout=45)
    guest.wait("[TTY-IPC] shell cap lookup succeeded", timeout=20)
    if args.disk:
        guest.wait("[PART] Mbr start=2048 blocks=131072")
        guest.wait("[VFS] FAT volume mounted at /mnt")
        for query in ("devices", "mounts"):
            command("storagectl " + query)
            match(rf"\[TTY\]  exit: storagectl {query} -> [01]")
        blockstats("idle-start")
        time.sleep(3)
        blockstats("idle-end")
    if args.probe:
        run_command("/bin/storage-probe", "[01]")
    if args.deny_create:
        run_command("/bin/storage-probe", "0")
        if args.disk:
            run_command("cp /bin/storage-probe /mnt/BTOP09.BIN", "1")
    if args.allocate:
        path = "/mnt/btop07.bin"
        run_command(f"cp /bin/storage-probe {path}", "0", timeout=45)
        blockstats("after-copy")
        run_command("/bin/storage-probe", "0")
        run_command(f"cp {path} /tmp/btop07.bin", "0", timeout=45)
        blockstats("after-readback")
        run_command(f"rm {path}", "0")
        blockstats("after-remove")
        run_command("/bin/storage-probe", "0")
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
    reaped = re.findall(r"\[SCHED\] process_reaped pid=(\d+) name='btop'", guest.read())
    assert pid in reaped and len(set(reaped)) >= 2, reaped
    command("/bin/proc-native")
    match(r"\[TTY\]  exit: /bin/proc-native -> 0")
    guest.capture("after")
    log = guest.read()
    for path in ("/proc/stat", "/proc/meminfo", f"/proc/{pid}/stat",
                 "/sys/class/net/eth0/statistics/rx_bytes"):
        assert log.count(f'path=Some("{path}")') >= 2, path
finally:
    guest.close()
