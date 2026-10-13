"""Run only our QEMU instance, stop after the native document RPC verdict."""
import pathlib
import os
import json
import socket
import subprocess
import time

log = pathlib.Path("target/docserv-serial.log")
log.write_text("")
command = ["qemu-system-x86_64", "-cdrom", "target/docserv-test.iso",
           "-serial", f"file:{log}", "-display", "none", "-m", "2048M",
           "-smp", "2", "-device", "virtio-rng-pci,disable-modern=on",
           "-no-reboot", "-no-shutdown"]
qmp_path = f"/tmp/sunlight-docserv-qmp-{os.getpid()}.sock"
command += ["-qmp", f"unix:{qmp_path},server=on,wait=off"]
ui_gate = os.environ.get("DOCSERV_TEST_UI") == "1"
marker = "[DOCVIEW] native window navigation/zoom OK" if ui_gate else "[DOCSERV] native IPC open/render/text/close OK"

def screenshot():
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(5)
        connection.connect(qmp_path)
        stream = connection.makefile("rwb")
        stream.readline()
        for request in [{"execute": "qmp_capabilities"},
                        {"execute": "screendump", "arguments": {"filename": str(pathlib.Path("target/docview-native.ppm").resolve())}},
                        {"execute": "send-key", "arguments": {"keys": [{"type": "qcode", "data": "q"}]}}]:
            stream.write(json.dumps(request).encode() + b"\n")
            stream.flush()
            while True:
                response = json.loads(stream.readline())
                if "return" in response or "error" in response:
                    if "error" in response:
                        raise RuntimeError(response["error"])
                    break
if pathlib.Path("/dev/kvm").exists():
    command += ["-enable-kvm", "-cpu", "host"]
process = subprocess.Popen(command)
try:
    deadline = time.monotonic() + 120
    captured = False
    while time.monotonic() < deadline:
        output = log.read_text(errors="replace")
        if marker in output:
            if ui_gate and not captured:
                screenshot()
                captured = True
            if ui_gate and ("[DOCVIEW] native close OK" not in output or "[FILES] native PDF thumbnail worker OK" not in output):
                time.sleep(0.1)
                continue
            print("Native document service gate passed", flush=True)
            break
        if "failed to spawn sunlight-docserv" in output:
            print(output[-6000:])
            raise SystemExit("Native document service ELF failed to load")
        if any("process_mark_finished" in line and ("name='sunlight-docserv'" in line or (ui_gate and "name='sunlight-docview'" in line))
               for line in output.splitlines()):
            print(output[-9000:])
            raise SystemExit("Native document service exited before its verdict")
        if process.poll() is not None:
            raise SystemExit("QEMU exited before the document service verdict")
        time.sleep(0.5)
    else:
        print(output[-12000:])
        raise SystemExit("Native document service gate timed out")
finally:
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
