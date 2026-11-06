#!/usr/bin/env python3
"""Type through QEMU PS/2 hardware; never inject DOM events or editor values.
Only invented fixture data is used. Evidence lives under target/.
"""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/qemu-keyboard-smoke"
OUT.mkdir(parents=True, exist_ok=True)
(OUT / "result.txt").unlink(missing_ok=True)
SERIAL = OUT / "serial.log"
SERIAL.write_text("")
QMP = f"/tmp/sunlight-keyboard-{os.getpid()}.sock"
firmware = Path(os.environ.get("SUNLIGHT_OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd"))
variables = Path(os.environ.get("SUNLIGHT_OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd"))
shutil.copyfile(variables, OUT / "ovmf-vars.fd")
command = ["qemu-system-x86_64", "-cdrom", str(OUT / "account-fixture.iso"),
           "-drive", f"if=pflash,format=raw,readonly=on,file={firmware}",
           "-drive", f"if=pflash,format=raw,file={OUT / 'ovmf-vars.fd'}",
           "-serial", f"file:{SERIAL}", "-display", "none", "-m", "4096M", "-smp", "4",
           "-vga", "virtio", "-device", "virtio-rng-pci,disable-modern=on",
           "-qmp", f"unix:{QMP},server=on,wait=off", "-no-reboot", "-no-shutdown"]
if Path("/dev/kvm").exists():
    command += ["-enable-kvm", "-cpu", "host"]


def wait_for(marker, timeout=180):
    deadline = time.monotonic() + timeout
    while marker not in SERIAL.read_text(errors="replace"):
        if process.poll() is not None or time.monotonic() > deadline:
            raise RuntimeError(f"Guest timeout: {marker}; inspect serial.log")
        time.sleep(0.25)


def request(name, arguments=None):
    payload = {"execute": name}
    if arguments is not None:
        payload["arguments"] = arguments
    stream.write(json.dumps(payload).encode() + b"\n")
    stream.flush()
    while True:
        result = json.loads(stream.readline())
        if "error" in result:
            raise RuntimeError(result["error"])
        if "return" in result:
            return result["return"]


def key(*names):
    request("send-key", {"keys": [{"type": "qcode", "data": name} for name in names], "hold-time": 60})
    time.sleep(0.35)


def text(value):
    for c in value:
        if ord(c) > 127:
            key("ctrl", "shift", "u")
            text(f"{ord(c):x}")
            key("ret")
        elif c == "@":
            key("shift", "2")
        else:
            key({" ": "spc", "-": "minus", ".": "dot", "/": "slash"}.get(c, c))


def capture(name):
    time.sleep(2)
    ppm = OUT / f"{name}.ppm"
    request("screendump", {"filename": str(ppm)})
    subprocess.run(["convert", str(ppm), str(OUT / f"{name}.png")], check=True)
    # The fixture uses an 8-pixel bitmap font. OCR needs an enlarged copy;
    # retain the original scanout above as the visual evidence.
    ocr = OUT / f"{name}-ocr.png"
    subprocess.run(["convert", str(ppm), "-crop", "520x510+425+120", "-scale", "300%", str(ocr)], check=True)
    return subprocess.check_output(["tesseract", str(ocr), "stdout", "--psm", "6"], stderr=subprocess.DEVNULL, text=True)


with (OUT / "qemu.log").open("w") as qemu_log:
    process = subprocess.Popen(command, cwd=ROOT, stdout=qemu_log, stderr=subprocess.STDOUT)
    try:
        wait_for("[SESSION-FOUNDATION] SHELL_READY PASS")
        wait_for("[WELCOME-WIZARD] AUTO_LAUNCH PASS", 30)
        time.sleep(2)
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(20)
            connection.connect(QMP)
            stream = connection.makefile("rwb")
            stream.readline()
            request("qmp_capabilities")
            key("ctrl", "spc")
            wait_for("name=Run policy=", 20)
            text("rappid-rabbit --account-fixture")
            key("ret")
            wait_for("Account Lab", 60)
            time.sleep(3)
            initial = capture("qemu-fixture-initial")
            assert "Connect a demo account" in initial, "fixture window missing"
            # Native scalar composition transports accented and combining text.
            text("démo\u0301")
            key("backspace")  # remove o + combining accent as one grapheme
            text("o@example.test")
            typed = capture("qemu-interactive-typing")
            assert "démo@example.test" in typed, "native Unicode commit/grapheme editing failed"
            key("tab")  # password
            text("invented-é-pass")
            key("tab")  # show password (leave masked)
            key("tab")  # remember
            key("spc")
            capture("qemu-tab-navigation")
            key("tab")  # submit
            key("tab")  # secondary link
            key("tab")  # wrap to email
            key("shift", "tab")  # wrap backwards to link
            key("shift", "tab")  # submit
            key("shift", "tab")  # remember
            key("shift", "tab")  # show
            key("shift", "tab")  # password
            key("ret")
            time.sleep(2)
            final = capture("qemu-fixture-success")
            assert "Demo account connected" in final, "Enter submission/completion failed"
        serial = SERIAL.read_text(errors="replace")
        assert "invented-é-pass" not in serial
        assert not any(marker in serial for marker in ["KERNEL PANIC", "[EXCEPTION]", "[FAULT]"])
        (OUT / "result.txt").write_text("PASS: native PS/2 typing, scalar composition, Tab/Shift+Tab, checkbox Space, Enter, async success.\n")
        print("Native keyboard fixture QEMU gate PASS")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        Path(QMP).unlink(missing_ok=True)
