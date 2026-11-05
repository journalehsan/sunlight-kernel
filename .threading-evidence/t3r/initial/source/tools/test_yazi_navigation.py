#!/usr/bin/env python3
"""Exercise an ISO built by tools/test.sh yazi-phase1 through QMP keyboard input.

The guest uses the pinned, unchanged Yazi payload and the normal keyboard/TTY
path. Artifacts are kept in --output-dir, including failure registers and stack.
"""

import argparse
import json
import pathlib
import re
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", type=pathlib.Path, default=pathlib.Path("target/sunlightos.iso"))
    parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    parser.add_argument("--cycles", type=int, default=30)
    parser.add_argument("--cpus", type=int, default=16)
    parser.add_argument("--timeout", type=int, default=360)
    args = parser.parse_args()
    if args.cycles < 1 or args.timeout < 1 or args.cpus < 1:
        parser.error("cycles, cpus and timeout must be positive")
    iso = args.iso.resolve(strict=True)
    out = args.output_dir.resolve()
    out.mkdir(parents=True, exist_ok=False)
    serial = out / "serial.log"
    qmp = None
    process = None
    leader = None

    def log():
        return serial.read_text(errors="replace") if serial.exists() else ""

    def command(name, arguments=None):
        qmp.write((json.dumps({"execute": name, "arguments": arguments or {}}) + "\n").encode())
        while True:
            line = qmp.readline()
            if not line:
                raise RuntimeError("QMP disconnected")
            response = json.loads(line)
            if "error" in response:
                raise RuntimeError(str(response["error"]))
            if "return" in response:
                return response["return"]

    def monitor(text):
        return command("human-monitor-command", {"command-line": text})

    def check(allow_exit=False):
        text = log()
        if process.poll() is not None:
            raise RuntimeError("QEMU exited unexpectedly")
        if re.search(r"\[FAULT\]|\[PANIC\]|panicked at|Out of memory|mmap failed .*NoMemory", text):
            raise RuntimeError("guest fault, panic, or allocation failure")
        if re.search(r"process_mark_finished .*name='yazi'.*code=[1-9]", text):
            raise RuntimeError("Yazi task exited with an error")
        if not allow_exit:
            if "TTY mode → cooked" in text:
                raise RuntimeError("Yazi restored cooked mode before quit")
            if leader and f"process_mark_finished pid={leader} name='yazi'" in text:
                raise RuntimeError("Yazi leader exited before quit")
        return text

    def wait_for(predicate, description, allow_exit=False):
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            if predicate(check(allow_exit)):
                return
            time.sleep(0.5)
        raise RuntimeError(f"timeout waiting for {description}")

    def keys(sequence):
        for key in sequence:
            check()
            monitor(f"sendkey {key} 50")
            time.sleep(0.7)

    with tempfile.TemporaryDirectory(prefix="yazi-qmp-") as tmp, (out / "qemu.log").open("w") as stderr:
        endpoint = pathlib.Path(tmp) / "control.sock"
        try:
            process = subprocess.Popen([
                "qemu-system-x86_64", "-cdrom", str(iso),
                "-serial", f"file:{serial}", "-display", "none",
                "-m", "4096M", "-smp", str(args.cpus),
                "-device", "virtio-rng-pci,disable-modern=on",
                "-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0",
                "-qmp", f"unix:{endpoint},server=on,wait=off", "-no-reboot", "-no-shutdown",
            ], stdout=subprocess.DEVNULL, stderr=stderr)
            wait_for(lambda _: endpoint.exists(), "QMP socket")
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(20)
                connection.connect(str(endpoint))
                qmp = connection.makefile("rwb", buffering=0)
                qmp.readline()
                command("qmp_capabilities")
                try:
                    wait_for(lambda text: "[HELIOS-YAZI] first render confirmed" in text, "first render")
                    match = re.search(r"spawn: /bin/yazi pid=(\d+)", log())
                    if not match:
                        raise RuntimeError("missing Yazi leader PID")
                    leader = match[1]
                    print(f"Yazi PID {leader}: first frame", flush=True)
                    time.sleep(5)  # Let the smoke gate's injected Down key finish.
                    keys(["g-h"])
                    wait_for(lambda text: "chdir(/root) -> ok" in text, "home navigation")
                    for cycle in range(args.cycles):
                        keys(["down", "up"])
                        for key, path in [("left", "/"), ("right", "/root")]:
                            marker = f"chdir({path}) -> ok"
                            before = log().count(marker)
                            keys([key])
                            wait_for(lambda text: text.count(marker) > before, f"directory change to {path}")
                        if (cycle + 1) % 5 == 0:
                            print(f"Navigation cycles: {cycle + 1}/{args.cycles}", flush=True)
                    if len(re.findall(r"\[HELIOS\] chdir\(", log())) < args.cycles:
                        raise RuntimeError("keys did not produce enough actual directory changes")
                    # From /root, select /home in the fixed guest root listing,
                    # then enter its user directory, matching the reported path.
                    keys(["left", "g", "g", "down", "down", "down", "right", "right"])
                    wait_for(lambda text: "chdir(/home/user) -> ok" in text, "/home/user navigation")
                    keys(["down", "up", "left", "right"] * 5)
                    monitor(f"screendump {out / 'navigation.ppm'}")
                    check()
                    print("Navigation survived; requesting normal quit", flush=True)
                    monitor("sendkey q")
                    wait_for(
                        lambda text: re.search(
                            rf"process_mark_finished pid={leader} name='yazi'.*code=0(?:\s|$)", text
                        ), "clean leader exit", allow_exit=True,
                    )
                    time.sleep(5)
                    check(allow_exit=True)
                    if "TTY mode → cooked" not in log():
                        raise RuntimeError("quit did not restore cooked TTY mode")
                    monitor(f"screendump {out / 'exit.ppm'}")
                    print(f"PASS: navigation, /home/user, quit status 0, cooked TTY; {out}", flush=True)
                except Exception:
                    command("stop")
                    diagnostics = [monitor("info registers -a")]
                    for address in re.findall(r"\[FAULT\].*rsp=(0x[0-9a-f]+)", log()):
                        diagnostics.append(monitor(f"x /128gx {address}"))
                    (out / "registers.txt").write_text("\n".join(diagnostics))
                    monitor(f"screendump {out / 'failure.ppm'}")
                    raise
        finally:
            if qmp:
                qmp.close()
            if process and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
