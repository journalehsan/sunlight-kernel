"""Isolated native CLI boot gate using the QMP login workflow of network_tls_gate.
No disk images or existing VMs are opened. Run tools/test_sunjs.sh first.
"""
import json
import os
import pathlib
import socket
import subprocess
import time

root = pathlib.Path(__file__).resolve().parents[1]
log = root / "target/sunjs-guest.log"
log.write_text("")
qmp_path = pathlib.Path(f"/tmp/sunjs-gate-{os.getpid()}.sock")
command = ["qemu-system-x86_64", "-cdrom", str(root / "target/sunjs-test.iso"),
           "-serial", f"file:{log}", "-display", "none", "-m", "2048M",
           "-smp", "2", "-device", "virtio-rng-pci,disable-modern=on",
           "-qmp", f"unix:{qmp_path},server=on,wait=off", "-no-reboot", "-no-shutdown"]
if pathlib.Path("/dev/kvm").exists():
    command += ["-enable-kvm", "-cpu", "host"]
process = subprocess.Popen(command)


def wait_for(marker):
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        output = log.read_text(errors="replace")
        if marker in output:
            return
        if process.poll() is not None or any(x in output for x in ("KERNEL PANIC", "[EXCEPTION]")):
            raise RuntimeError(output[-8000:])
        time.sleep(0.25)
    raise RuntimeError(f"SunJS guest timeout waiting for {marker}\n" + output[-8000:])


try:
    wait_for("[LOGIN-TUI] input ready")
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(10)
        connection.connect(str(qmp_path))
        stream = connection.makefile("rwb")
        stream.readline()

        def request(execute, arguments=None):
            payload = {"execute": execute}
            if arguments:
                payload["arguments"] = arguments
            stream.write(json.dumps(payload).encode() + b"\n")
            stream.flush()
            while True:
                line = stream.readline()
                if not line:
                    raise RuntimeError("QMP disconnected")
                response = json.loads(line)
                if "error" in response:
                    raise RuntimeError(response["error"])
                if "return" in response:
                    return

        def type_text(text):
            keys = {" ": "spc", "/": "slash", "-": "minus", "\n": "ret"}
            for character in text:
                request("human-monitor-command", {
                    "command-line": "sendkey " + keys.get(character, character) + " 35"})
                time.sleep(0.085)

        request("qmp_capabilities")
        type_text("\nroot\n")
        wait_for("[TTY]  sunshell endpoint found")
        type_text("/bin/sunjs --smoke\n")
        wait_for("[SUNJS] native smoke PASS")
        wait_for("exit: /bin/sunjs --smoke -> 0")
        print("SunJS native CLI guest smoke PASS")
finally:
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    qmp_path.unlink(missing_ok=True)
