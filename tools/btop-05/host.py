#!/usr/bin/env python3
"""Run the static probe under a Linux PTY; inject only after its marker."""
import errno
import os
import pty
import select
import signal
import sys
import time

pid, fd = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], sys.argv[1:])
log = bytearray()
injected = False
status = None
try:
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if not select.select([fd], [], [], 0.2)[0]:
            continue
        try:
            data = os.read(fd, 4096)
        except OSError as exc:
            if exc.errno == errno.EIO:
                break
            raise
        if not data:
            break
        log.extend(data)
        if not injected and b"MASKED-SELECT-INJECT" in log:
            os.write(fd, b"x")
            injected = True
    if time.monotonic() >= deadline:
        raise TimeoutError("masked pselect probe")
    status = os.waitpid(pid, 0)[1]
finally:
    if status is None:
        try:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        except ProcessLookupError:
            pass
    os.close(fd)
    sys.stdout.buffer.write(log)

sys.exit(os.waitstatus_to_exitcode(status))
