#!/usr/bin/env python3
"""Deterministic host-side TCP peer for the QEMU user-mode network.

QEMU's user-mode backend exposes the host loopback as the guest gateway alias
10.0.2.2, so this peer binds 127.0.0.1 only. It binds port 0 (no check-then-bind
race), keeps the listener open, and prints one explicit READY line with the
selected port. The listener is closed immediately after accept so the guest's
follow-up connect is deterministically refused.

Modes:
  echo   validate the five length-prefixed frames and return byte-exact echoes
  stall  read one frame, then withhold every response until the guest deadline
"""
import argparse
import socket
import sys
import time

SIZES = (1, 7, 64, 255, 1024)


class PeerFailure(Exception):
    pass


def log(message):
    print(message, flush=True)


def recv_exact(conn, length, deadline):
    data = bytearray()
    while len(data) < length:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise PeerFailure("deadline while reading")
        conn.settimeout(remaining)
        chunk = conn.recv(length - len(data))
        if not chunk:
            raise PeerFailure("unexpected EOF after %d/%d bytes" % (len(data), length))
        data.extend(chunk)
    return bytes(data)


def send_all(conn, data, deadline):
    offset = 0
    while offset < len(data):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise PeerFailure("deadline while writing")
        conn.settimeout(remaining)
        sent = conn.send(data[offset:])
        if sent <= 0:
            raise PeerFailure("send returned %d" % sent)
        offset += sent


def expected_payload(index):
    size = SIZES[index]
    payload = bytearray(4 + size)
    payload[0:4] = (index + 1).to_bytes(4, "big")
    for offset in range(size):
        payload[4 + offset] = (index * 37 + offset * 11) & 0xFF
    payload[4 + size // 2] = 0
    payload[4 + size // 3] = 0
    return bytes(payload)


def serve_echo(conn, deadline):
    exchanges = 0
    total = 0
    for index in range(len(SIZES)):
        header = recv_exact(conn, 4, deadline)
        length = int.from_bytes(header, "big")
        expected = expected_payload(index)
        if length != len(expected):
            raise PeerFailure("frame %d length %d != %d" % (index, length, len(expected)))
        payload = recv_exact(conn, length, deadline)
        if payload != expected:
            raise PeerFailure("frame %d payload mismatch" % index)
        send_all(conn, header + payload, deadline)
        exchanges += 1
        total += length
    terminator = recv_exact(conn, 4, deadline)
    if terminator != b"\x00\x00\x00\x00":
        raise PeerFailure("terminator was %r" % terminator)
    return exchanges, total


def serve_stall(conn, deadline):
    header = recv_exact(conn, 4, deadline)
    length = int.from_bytes(header, "big")
    if length != 8:
        raise PeerFailure("stall frame length %d != 8" % length)
    payload = recv_exact(conn, length, deadline)
    if payload != b"stalled!":
        raise PeerFailure("stall payload was %r" % payload)
    # Hold the connection open without writing. The guest bridge must surface
    # its own bounded recv deadline; this side only waits for the peer to close.
    conn.settimeout(max(1.0, deadline - time.monotonic()))
    try:
        while conn.recv(4096):
            pass
    except socket.timeout:
        pass
    except OSError:
        pass
    return 1, length


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("echo", "stall"), default="echo")
    parser.add_argument("--deadline", type=float, default=120.0,
                        help="overall wall-clock deadline in seconds")
    parser.add_argument("--connect-timeout", type=float, default=60.0,
                        help="maximum wait for the guest connection")
    opts = parser.parse_args()

    deadline = time.monotonic() + opts.deadline
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    conn = None
    try:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        host, port = listener.getsockname()
        log("HOST-PEER READY mode=%s addr=%s port=%d" % (opts.mode, host, port))
        listener.settimeout(opts.connect_timeout)
        conn, peer = listener.accept()
        # Close the listener before the exchange so the guest's later connect
        # probe sees a refused port rather than a queued second connection.
        listener.close()
        listener = None
        log("HOST-PEER ACCEPTED mode=%s peer=%s:%d" % (opts.mode, peer[0], peer[1]))
        if opts.mode == "echo":
            exchanges, total = serve_echo(conn, deadline)
            log("HOST-PEER PASS mode=echo exchanges=%d bytes=%d" % (exchanges, total))
        else:
            exchanges, total = serve_stall(conn, deadline)
            log("HOST-PEER PASS mode=stall received_frames=%d bytes=%d" % (exchanges, total))
        return 0
    except (PeerFailure, OSError) as error:
        log("HOST-PEER FAIL mode=%s error=%s" % (opts.mode, error))
        return 1
    finally:
        if conn is not None:
            try:
                conn.close()
            except OSError:
                pass
        if listener is not None:
            try:
                listener.close()
            except OSError:
                pass


if __name__ == "__main__":
    sys.exit(main())
