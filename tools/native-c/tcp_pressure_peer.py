#!/usr/bin/env python3
"""Deterministic host-side TCP peer for POLLOUT send-backpressure fixtures.

Topology (external mode): guest native client -> net_server -> virtio-net ->
QEMU user-mode networking (10.0.2.2 host alias) -> this peer on 127.0.0.1.

The peer binds three ephemeral loopback ports (control, data1, data2) and
prints one explicit READY line carrying all three. It never infers readiness
from time; the guest drives every transition over the control connection:

  data1 (main stream):
    'H' withhold application reads (guest can fill the path)
    'D' resume draining
    'Q' validate the deterministic stream and write a verdict line
    'F' shutdown(SHUT_WR): a real receive-side FIN half-close
  data2 (reset stream):
    'h' withhold reads, 'd' drain, 'R' SO_LINGER(0) close -> real RST
  'X' exit

The main stream is an 8-byte big-endian payload length followed by
payload[i] = (i * 31 + 7) & 0xff. Validation is byte-exact length plus an
independently recomputed FNV-1a 64 hash; a cap bounds total memory.
"""
import argparse
import socket
import sys
import threading
import time

FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x100000001B3
MASK = (1 << 64) - 1
MAX_STREAM = 64 * 1024 * 1024


def fnv1a(data):
    h = FNV_OFFSET
    for byte in data:
        h ^= byte
        h = (h * FNV_PRIME) & MASK
    return h


def expected_hash(payload_len):
    h = FNV_OFFSET
    header = payload_len.to_bytes(8, "big")
    for byte in header:
        h ^= byte
        h = (h * FNV_PRIME) & MASK
    for i in range(payload_len):
        h ^= (i * 31 + 7) & 0xFF
        h = (h * FNV_PRIME) & MASK
    return h


def log(message):
    print(message, flush=True)


class Reader(threading.Thread):
    """Gate-able data reader that accumulates one deterministic stream."""

    def __init__(self, conn, draining, label):
        super().__init__(daemon=True)
        self.conn = conn
        self.draining = draining
        self.label = label
        self.buffer = bytearray()
        self.payload_len = None
        self.overflow = False
        self.error = None
        self.eof = False
        self.stop = threading.Event()

    def run(self):
        while not self.stop.is_set():
            if not self.draining.is_set():
                self.draining.wait(0.02)
                continue
            self.conn.settimeout(0.05)
            try:
                chunk = self.conn.recv(65536)
            except socket.timeout:
                continue
            except OSError as error:
                self.error = str(error)
                return
            if not chunk:
                self.eof = True
                return
            if len(self.buffer) + len(chunk) > MAX_STREAM:
                self.overflow = True
                return
            self.buffer.extend(chunk)
            if self.payload_len is None and len(self.buffer) >= 8:
                self.payload_len = int.from_bytes(self.buffer[:8], "big")

    def stream_complete(self):
        return (self.payload_len is not None
                and len(self.buffer) >= 8 + self.payload_len)

    def validate(self):
        if self.payload_len is None:
            return "no header", 0, 0
        expected_len = 8 + self.payload_len
        got = bytes(self.buffer[:expected_len])
        expected = expected_hash(self.payload_len)
        actual = fnv1a(got)
        if len(got) != expected_len:
            return ("short stream %d/%d" % (len(got), expected_len),
                    expected, actual)
        if actual != expected:
            return "hash mismatch", expected, actual
        return None, expected, actual


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deadline", type=float, default=180.0,
                        help="overall wall-clock deadline in seconds")
    parser.add_argument("--connect-timeout", type=float, default=90.0,
                        help="maximum wait for guest connections")
    opts = parser.parse_args()

    deadline = time.monotonic() + opts.deadline
    control_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    data1_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    data2_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    control = data1 = data2 = None
    reader1 = reader2 = None
    try:
        for listener in (control_listener, data1_listener, data2_listener):
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            # A small guest-facing receive buffer makes the host window a
            # deliberate, bounded part of the saturation path.
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 32768)
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
        control_port = control_listener.getsockname()[1]
        data1_port = data1_listener.getsockname()[1]
        data2_port = data2_listener.getsockname()[1]
        log("HOST-PEER READY mode=pressure ctrl=%d data=%d reset=%d"
            % (control_port, data1_port, data2_port))

        for listener in (control_listener, data1_listener, data2_listener):
            listener.settimeout(opts.connect_timeout)
        control, _ = control_listener.accept()
        data1, _ = data1_listener.accept()
        data2, _ = data2_listener.accept()
        for listener in (control_listener, data1_listener, data2_listener):
            listener.close()
        control_listener = data1_listener = data2_listener = None
        control.setblocking(True)
        log("HOST-PEER ACCEPTED mode=pressure")

        drain1 = threading.Event()
        drain2 = threading.Event()
        reader1 = Reader(data1, drain1, "data1")
        reader2 = Reader(data2, drain2, "data2")
        reader2.buffer = bytearray()  # data2 bytes are not validated
        reader1.start()
        reader2.start()

        verdict = {"emitted": False}

        def emit_verdict():
            if verdict["emitted"]:
                return
            verdict["emitted"] = True
            wait = time.monotonic() + 60.0
            while not reader1.stream_complete() and time.monotonic() < wait:
                time.sleep(0.02)
            if reader1.overflow:
                line = "HOST-PEER FAIL mode=pressure error=stream-overflow"
                code = 1
            else:
                error, expected, actual = reader1.validate()
                if error is None:
                    line = ("HOST-PEER PASS mode=pressure bytes=%d payload=%d "
                            "fnv=%016x" % (len(reader1.buffer),
                                           reader1.payload_len, actual))
                    code = 0
                else:
                    line = ("HOST-PEER FAIL mode=pressure error=%s expected=%016x "
                            "actual=%016x bytes=%d"
                            % (error, expected, actual, len(reader1.buffer)))
                    code = 1
            log(line)
            try:
                control.sendall(line.encode() + b"\n")
            except OSError:
                pass
            verdict["code"] = code

        while time.monotonic() < deadline:
            try:
                command = control.recv(1)
            except OSError as error:
                log("HOST-PEER FAIL mode=pressure error=control-%s" % error)
                break
            if not command:
                break
            byte = command
            if byte == b"H":
                drain1.clear()
            elif byte == b"D":
                drain1.set()
            elif byte == b"h":
                drain2.clear()
            elif byte == b"d":
                drain2.set()
            elif byte == b"Q":
                emit_verdict()
            elif byte == b"F":
                try:
                    data1.shutdown(socket.SHUT_WR)
                except OSError as error:
                    log("HOST-PEER FAIL mode=pressure error=fin-%s" % error)
            elif byte == b"R":
                try:
                    data2.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                                     b"\x01\x00\x00\x00\x00\x00\x00\x00")
                except OSError:
                    pass
                try:
                    data2.close()
                except OSError:
                    pass
                data2 = None
            elif byte == b"X":
                break
            else:
                log("HOST-PEER FAIL mode=pressure error=unknown-command-%r" % byte)
                break

        # Drain anything still in flight so the guest's close is observed.
        drain1.set()
        drain2.set()
        for reader in (reader1, reader2):
            if reader is not None:
                reader.stop.set()
        if not verdict["emitted"]:
            emit_verdict()
        code = verdict.get("code", 1)
        if reader1.error or reader1.overflow:
            code = 1
        return code
    except (socket.timeout, OSError) as error:
        log("HOST-PEER FAIL mode=pressure error=%s" % error)
        return 1
    finally:
        for resource in (control, data1, data2,
                         control_listener, data1_listener, data2_listener):
            if resource is not None:
                try:
                    resource.close()
                except OSError:
                    pass


if __name__ == "__main__":
    sys.exit(main())
