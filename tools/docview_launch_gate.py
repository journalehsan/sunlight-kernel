"""UEFI launch regression: the default viewer opens without a PDF renderer."""
import json
import os
import pathlib
import shutil
import socket
import subprocess
import time

root = pathlib.Path(__file__).resolve().parent.parent
output = root / "target/docview-launch-fix"
output.mkdir(parents=True, exist_ok=True)
serial = output / "uefi-serial.log"
serial.write_text("")
firmware = pathlib.Path(os.environ.get("SUNLIGHT_OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd"))
variables = pathlib.Path(os.environ.get("SUNLIGHT_OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd"))
shutil.copyfile(variables, output / "ovmf-vars.fd")
qmp_path = f"/tmp/sunlight-docview-launch-{os.getpid()}.sock"
command = [
    "qemu-system-x86_64", "-cdrom", str(root / "target/docview-launch-test.iso"),
    "-drive", f"if=pflash,format=raw,readonly=on,file={firmware}",
    "-drive", f"if=pflash,format=raw,file={output / 'ovmf-vars.fd'}",
    "-serial", f"file:{serial}", "-display", "none", "-m", "4096M", "-smp", "12",
    "-vga", "virtio", "-device", "virtio-rng-pci,disable-modern=on",
    "-qmp", f"unix:{qmp_path},server=on,wait=off", "-no-reboot", "-no-shutdown",
]
if pathlib.Path("/dev/kvm").exists():
    command += ["-enable-kvm", "-cpu", "host"]


def qmp(stream, name, arguments=None):
    request = {"execute": name}
    if arguments is not None:
        request["arguments"] = arguments
    stream.write(json.dumps(request).encode() + b"\n")
    stream.flush()
    while True:
        line = stream.readline()
        if not line:
            raise RuntimeError("QMP disconnected")
        result = json.loads(line)
        if "error" in result:
            raise RuntimeError(result["error"])
        if "return" in result:
            return result["return"]


def key(stream, *names):
    qmp(stream, "send-key", {"keys": [{"type": "qcode", "data": n} for n in names], "hold-time": 60})
    time.sleep(0.12)


with (output / "qemu.log").open("w") as qemu_log:
    process = subprocess.Popen(command, cwd=root, stdout=qemu_log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + 150
        while "[SESSION-FOUNDATION] SHELL_READY PASS" not in serial.read_text(errors="replace"):
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("desktop did not become ready")
            time.sleep(0.25)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(10)
            connection.connect(qmp_path)
            stream = connection.makefile("rwb")
            stream.readline()
            qmp(stream, "qmp_capabilities")
            # Exercise the desktop Run dialog's shared sun-exec launch path.
            # Wait for the optional Welcome window before opening Run, so it
            # cannot steal focus while the application name is being typed.
            settle_deadline = time.monotonic() + 20
            while "[WELCOME-WIZARD] AUTO_LAUNCH PASS" not in serial.read_text(errors="replace"):
                if time.monotonic() > settle_deadline:
                    raise RuntimeError("desktop welcome did not settle")
                time.sleep(0.25)
            time.sleep(1)
            key(stream, "ctrl", "spc")
            runner_deadline = time.monotonic() + 10
            while "name=Run policy=" not in serial.read_text(errors="replace"):
                if time.monotonic() > runner_deadline:
                    raise RuntimeError("desktop Run dialog did not open")
                time.sleep(0.25)
            time.sleep(0.5)
            for character in "sunlight-docview":
                key(stream, {"-": "minus"}.get(character, character))
            key(stream, "ret")
            deadline = time.monotonic() + 30
            while "[DOCVIEW] Open Document dialog requested" not in serial.read_text(errors="replace"):
                text = serial.read_text(errors="replace")
                if "UnsupportedPersonality" in text or process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("viewer launch failed; inspect uefi-serial.log")
                time.sleep(0.25)
            time.sleep(2)
            qmp(stream, "screendump", {"filename": str(output / "viewer-open-dialog.ppm")})
        text = serial.read_text(errors="replace")
        assert "[DOCVIEW] window ready" in text
        assert "Sunlight Dialog phase=window_visible_on_screen" in text
        assert "name='sunlight-docview' reason=exit" not in text
        assert "UnsupportedPersonality" not in text
        assert "path=/bin/sunlight-docview" in text or "spawn: /bin/sunlight-docview" in text
        assert "[DOCSERV] ready" not in text, "default image enabled the renderer"
        print("Document Viewer UEFI launch gate PASSED (renderer disabled)")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        pathlib.Path(qmp_path).unlink(missing_ok=True)
