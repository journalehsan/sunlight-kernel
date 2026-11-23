#!/usr/bin/env python3
"""Replay on an isolated 4 GiB guest with DHCP, without DHCP, or without a NIC."""
import argparse
import ipaddress
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
cli.add_argument("--mode", choices=["lease", "unconfigured", "unavailable", "btop"],
                 default="lease")
opts = cli.parse_args()
out = opts.evidence.resolve()
out.mkdir(parents=True, exist_ok=False)
os.environ["BTOP_TCG"] = "1"
os.environ.pop("BTOP_DISK", None)
if opts.mode != "unavailable":
    executable = shutil.which("qemu-system-x86_64")
    extra = ["-netdev", "user,id=testnet,restrict=on,dhcpstart=10.0.2.42",
             "-device", "virtio-net-pci,netdev=testnet,disable-modern=on"]
    if opts.mode == "unconfigured":
        # Keep the real VirtIO backend and its link while withholding DHCP
        # traffic for longer than the service's one acquisition window.
        extra += ["-object", "filter-buffer,id=hold,netdev=testnet,queue=all,interval=600000000"]
    extra += ["-object", f"filter-dump,id=frames,netdev=testnet,file={out / 'network.pcap'}"]
    wrapper = out / "qemu-system-x86_64"
    wrapper.write_text(f"""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = [{executable!r}, *sys.argv[1:], *{extra!r}]
snapshot = json.dumps(args, indent=2)
Path({str(out / 'command.json')!r}).write_text(snapshot)
os.execv(args[0], args)
""")
    wrapper.chmod(0o755)
    os.environ["PATH"] = f"{out}:{os.environ['PATH']}"

if opts.mode == "btop":
    result = subprocess.run([sys.executable, str(root / "tools/btop-02/replay.py"),
                             str(opts.iso), str(out / "tty")], check=False)
    serial = (out / "tty/serial.log").read_text(errors="replace")
    assert "[TTY]  exit: /bin/btop --version -> 0" in serial
    assert re.search(r"nr=45 .*a2=0x2000 .*result=128 ", serial)
    assert re.search(r"nr=45 .*a2=0x2000 .*result=84 ", serial)
    assert re.search(r'nr=2 .*result=[0-9]+ path=Some\("/sys/class/net/eth0/statistics/(rx|tx)_bytes"\)', serial)
    if result.returncode:
        assert (out / "tty/05-no-exit.png").is_file()
        assert re.search(r"nr=270 .*result=-95 ", serial)
        print("BLOCKED: masked pselect6 and keyboard exit")
    print("PASS: btop reached both interface dumps and the Ethernet counter reads")
    sys.exit(0)

guest = Guest(opts.iso, out)

def command(cmd):
    for part in re.split("(_)", cmd):
        if part == "_":
            guest.key("shift", "minus")
        elif part:
            guest.type(part)
    guest.key("ret")
    guest.wait(f"[TTY]  exit: {cmd} -> ", timeout=35)
    log = guest.read()
    assert f"[TTY]  exit: {cmd} -> 0" in log
    return next(line.split(" -> ", 1)[1] for line in log.splitlines()
                if f"[TTY]  cmd: {cmd} -> " in line)

try:
    guest.wait("[TTY]  Login screen ready")
    time.sleep(4)
    guest.key("ret")
    guest.type("root")
    guest.key("tab")
    guest.key("spc")
    guest.key("ret")
    guest.wait("[TTY]  Built-in shell ready", timeout=45)

    if opts.mode == "unavailable":
        assert "errno=100 unchanged=1" in command("/bin/ifaddrs unavailable")
        print("PASS: unavailable service/backend state returns ENETDOWN")
    else:
        guest.wait("[NET]  smoltcp interface up over kernel frame proxy", timeout=60)
        listing = command("/bin/ifaddrs")
        assert "cleanup=70" in listing
        for name in ("lo", "eth0"):
            native = command(f"networkctl status {name}")
            identity = re.search(rf"{name}:\\n  id:\s+(\d+)", native)
            address = re.search(r"address:\s+([0-9.]+/\d+|-)", native)
            assert identity and address, native
            assert f"{name}#{identity[1]} " in listing, (native, listing)
            ip = address[1]
            if ip != "-":
                iface = ipaddress.IPv4Interface(ip)
                assert f"{name}={iface.ip}/{iface.netmask} " in listing, (native, listing)
            else:
                assert f"{name}=" not in listing, listing
            print(f"PASS: {name} id={identity[1]} address={ip}")
        if opts.mode == "lease":
            assert "[DHCP] ACK received" in guest.read()
            assert "eth0=10.0.2.42/" in listing
        else:
            assert "[DHCP] no lease acquired" in guest.read()
            assert "eth0=" not in listing
        assert "PASS" in command("/bin/route-scope")
        assert "publication=-1" in command("/bin/net_server")
        print("PASS: 70 enumerations and cleanup, protocol scope, native publication denied")
    guest.wait("name='ifaddrs' user_frames=", timeout=10)
    guest.capture("01-result")
finally:
    guest.close()
