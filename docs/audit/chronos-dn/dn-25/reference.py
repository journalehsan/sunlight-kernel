"""Replay physical keys against the installed SeaBIOS, independently of Sunlight."""
import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument("--bios", type=Path, default=Path("/usr/share/qemu/bios-256k.bin"))
bios = ap.parse_args().bios.resolve()
source = Path(__file__).with_name("reference.asm")
print(f"bios_sha256={hashlib.sha256(bios.read_bytes()).hexdigest()}", flush=True)
with tempfile.TemporaryDirectory() as path:
    tmp = Path(path)
    floppy, debug, firmware, qmp = (tmp / x for x in ("probe", "debug", "firmware", "qmp"))
    subprocess.run(["nasm", "-f", "bin", str(source), "-o", str(floppy)], check=True)
    with floppy.open("ab") as f:
        f.truncate(1440 * 1024)
    cmd = ["qemu-system-x86_64", "-bios", str(bios), "-display", "none",
           "-serial", "none", "-monitor", "none", "-boot", "a",
           "-drive", f"file={floppy},if=floppy,format=raw,readonly=on",
           "-debugcon", f"file:{debug}", "-chardev", f"file,id=firmware,path={firmware}",
           "-device", "isa-debugcon,iobase=0x402,chardev=firmware",
           "-qmp", f"unix:{qmp},server=on,wait=off"]
    vm = subprocess.Popen(cmd)
    try:
        deadline = time.monotonic() + 10
        while not (debug.exists() and debug.read_text().startswith("R\n")):
            if time.monotonic() > deadline or vm.poll() is not None:
                raise RuntimeError("SeaBIOS probe did not boot")
            time.sleep(.05)
        print(next(iter(firmware.read_text(errors="replace").splitlines()), "firmware_banner=unavailable"))
        with socket.socket(socket.AF_UNIX) as sock:
            sock.connect(str(qmp))
            with sock.makefile("rwb") as stream:
                stream.readline()

                def request(name, args=None):
                    message = {"execute": name}
                    if args is not None:
                        message["arguments"] = args
                    stream.write(json.dumps(message).encode() + b"\n")
                    stream.flush()
                    while True:
                        reply = json.loads(stream.readline())
                        if "error" in reply:
                            raise RuntimeError(reply["error"])
                        if "return" in reply:
                            return reply["return"]

                request("qmp_capabilities")

                def send(*keys):
                    request("send-key", {"hold-time": 45,
                                         "keys": [{"type": "qcode", "data": k} for k in keys]})
                def key(*keys):
                    before = len(debug.read_text().splitlines())
                    send(*keys)
                    time.sleep(.20)
                    # F12 is a delimiter: an empty row has no BIOS word before it.
                    send("f12")
                    deadline = time.monotonic() + 4
                    while time.monotonic() < deadline:
                        rows = debug.read_text().splitlines()[before:]
                        if rows and rows[-1].startswith("8600 ") and len(rows[-1].split()) == 9:
                            time.sleep(.08)
                            return [" ".join(line.split()) for line in rows[:-1]]
                        time.sleep(.025)
                    raise RuntimeError((keys, rows))

                print("num,key,status11_AX FLAGS BX CX DX,status01_AX,read10_AX FLAGS,shift02_AX")
                keys = (
                    "up down left right home end pgup pgdn insert delete "
                    "kp_0 kp_1 kp_2 kp_3 kp_4 kp_5 kp_6 kp_7 kp_8 kp_9 "
                    "kp_decimal kp_add kp_subtract kp_multiply kp_divide kp_enter ret"
                ).split()
                initial = key("kp_8")[0].split()[0]
                if initial == "4838":
                    key("num_lock")
                elif initial != "4800":
                    raise RuntimeError(f"Unexpected keypad baseline: {initial}")
                for locked in (0, 1):
                    if locked:
                        assert not key("num_lock")
                    for modifier in ((), ("shift",), ("ctrl",), ("alt",)):
                        for name in keys:
                            chord = (*modifier, name)
                            rows = key(*chord)
                            print(f"{locked},{'+'.join(chord)},{';'.join(rows) or 'EMPTY'}",
                                  flush=True)
    finally:
        vm.terminate()
        vm.wait(timeout=5)
