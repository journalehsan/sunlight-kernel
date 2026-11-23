#!/usr/bin/env python3
"""Use a Linux PTY to run the same musl binaries executed in the guest."""
import errno
import os
from pathlib import Path
import pty
import resource
import select
import subprocess
import time

root = Path(__file__).resolve().parents[2]
out = root / "target/btop-03"
subprocess.run([out / "ifaddrs.elf"], check=True)
failure = subprocess.run(
    [out / "ifaddrs.elf"], capture_output=True, text=True,
    preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_NOFILE, (3, 3)),
)
print(f"Linux no-free-fd control: {failure.stdout}", end="")
assert failure.returncode == 1
assert f"rc=-1 errno={errno.EMFILE} unchanged=1" in failure.stdout
master, slave = pty.openpty()
with subprocess.Popen([out / "tty-select.elf"], stdin=slave, stdout=slave, stderr=slave) as process:
    os.close(slave)
    try:
        deadline = time.monotonic() + 15
        output = b""
        injected = False
        while time.monotonic() < deadline:
            if not select.select([master], [], [], 0.2)[0]:
                continue
            try:
                data = os.read(master, 4096)
            except OSError as exc:
                if exc.errno != errno.EIO:
                    raise
                break
            if not data:
                break
            output += data
            if b"SELECT-INJECT" in output and not injected:
                time.sleep(0.5)
                os.write(master, b"x")
                injected = True
        print(output.decode(), end="")
        assert process.wait(timeout=2) == 0
        assert b"select mask rc=-1 errno=4 caught=2 restored=1" in output
    finally:
        os.close(master)
        if process.poll() is None:
            process.kill()
