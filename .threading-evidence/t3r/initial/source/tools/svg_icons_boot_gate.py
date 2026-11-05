"""Exercise native SVG icon consumers in an isolated desktop VM."""
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target/svg-icons-proof"
OUTPUT.mkdir(parents=True, exist_ok=True)
SERIAL = OUTPUT / "serial.log"
SERIAL.write_text("")
QMP_PATH = f"/tmp/sunlight-svg-icons-{os.getpid()}.sock"


def output():
    return SERIAL.read_text(errors="replace")


def wait_for(predicate, label, timeout=40):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        text = output()
        if any(marker in text for marker in ("KERNEL PANIC", "[FAULT]", "[EXCEPTION]", "panicked at")):
            raise RuntimeError(f"Guest fault during {label}\n{text[-8000:]}")
        if predicate(text):
            return
        if process.poll() is not None:
            raise RuntimeError(f"QEMU exited during {label}")
        time.sleep(0.25)
    raise RuntimeError(f"Timed out: {label}\n{output()[-8000:]}")


def qmp(stream, command, arguments=None):
    request = {"execute": command}
    if arguments is not None:
        request["arguments"] = arguments
    stream.write(json.dumps(request).encode() + b"\n")
    stream.flush()
    while True:
        line = stream.readline()
        if not line:
            raise RuntimeError("QMP disconnected")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(response["error"])
        if "return" in response:
            return response["return"]


def key(stream, *names):
    qmp(stream, "send-key", {
        "keys": [{"type": "qcode", "data": name} for name in names],
        "hold-time": 60,
    })
    time.sleep(0.15)


def launch(stream, name):
    previous = output().count("name=Run policy=")
    key(stream, "ctrl", "spc")
    wait_for(lambda text: text.count("name=Run policy=") > previous, "Run dialog")
    time.sleep(0.5)
    for character in name:
        key(stream, {"-": "minus"}.get(character, character))
    key(stream, "ret")


def capture(stream, name):
    time.sleep(1)
    qmp(stream, "screendump", {"filename": str(OUTPUT / f"{name}.ppm")})


command = [
    "qemu-system-x86_64", "-cdrom", str(ROOT / "target/svg-icons.iso"),
    "-serial", f"file:{SERIAL}", "-display", "none", "-m", "4096M", "-smp", "2",
    "-device", "virtio-rng-pci,disable-modern=on",
    "-qmp", f"unix:{QMP_PATH},server=on,wait=off", "-no-reboot", "-no-shutdown",
]
if Path("/dev/kvm").exists():
    command += ["-enable-kvm", "-cpu", "host"]

with (OUTPUT / "qemu.log").open("w") as log:
    process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    try:
        wait_for(lambda text: "[SESSION-FOUNDATION] SHELL_READY PASS" in text, "desktop ready", 180)
        wait_for(lambda text: "[WELCOME-WIZARD] AUTO_LAUNCH PASS" in text, "welcome settled")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(10)
            connection.connect(QMP_PATH)
            stream = connection.makefile("rwb")
            stream.readline()
            qmp(stream, "qmp_capabilities")
            capture(stream, "desktop")
            launch(stream, "sunlight-files")
            wait_for(lambda text: "Sunlight Files phase=window_visible_on_screen" in text, "Files visible")
            capture(stream, "files")
            key(stream, "alt", "f4")
            launch(stream, "control-panel")
            wait_for(lambda text: "[CONTROL-PANEL] starting" in text, "Control Panel started")
            wait_for(lambda text: "System Preferences phase=window_visible_on_screen" in text, "Control Panel visible")
            capture(stream, "control-panel")
            key(stream, "alt", "f4")
            launch(stream, "sunlight-docview")
            wait_for(lambda text: "Sunlight Dialog phase=window_visible_on_screen" in text, "file dialog visible")
            wait_for(lambda text: len(set(re.findall(r"\[ICON-SVG\] pid=(\d+) runtime raster ready", text))) >= 4,
                     "SVG rendering in all four consumers")
            capture(stream, "file-dialog")
        print("SVG desktop, Files, Control Panel, and file dialog gate PASSED", flush=True)
        print(f"Serial log and screenshots: {OUTPUT}", flush=True)
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        Path(QMP_PATH).unlink(missing_ok=True)
