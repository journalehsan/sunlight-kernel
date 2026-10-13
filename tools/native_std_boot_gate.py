"""Isolated native FP foundation gate; does not touch a user VM or disk."""
import pathlib
import subprocess
import time

for cpus in (2, 4):
    log = pathlib.Path(f"target/native-std-fp-{cpus}cpu.log")
    log.write_text("")
    command = ["qemu-system-x86_64", "-cdrom", "target/native-std-foundation.iso",
               "-serial", f"file:{log}", "-display", "none", "-m", "2048M",
               "-smp", str(cpus), "-device", "virtio-rng-pci,disable-modern=on",
               "-no-reboot", "-no-shutdown"]
    if pathlib.Path("/dev/kvm").exists():
        command += ["-enable-kvm", "-cpu", "host"]
    process = subprocess.Popen(command)
    try:
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            output = log.read_text(errors="replace")
            if ("[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK" in output
                    and "[NATIVE-STD] native PDF parser and vector raster OK" in output):
                print(f"Native FP foundation passed with {cpus} CPUs", flush=True)
                break
            if any(marker in output for marker in ("[NATIVE-STD] FP isolation FAILED",
                    "[NATIVE-STD] FP spawn FAILED", "[NATIVE-STD] FP probe panicked", "[NATIVE-STD] parser/raster FAILED",
                    "KERNEL PANIC", "[EXCEPTION]")):
                raise RuntimeError(output[-12000:])
            if process.poll() is not None:
                raise RuntimeError("QEMU exited before native FP verdict")
            time.sleep(0.25)
        else:
            raise RuntimeError("Native FP verdict timed out\n" + output[-12000:])
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
