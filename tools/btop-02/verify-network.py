#!/usr/bin/env python3
"""Match the two procfs observations against isolated QEMU Ethernet frames."""
import argparse
import re
from pathlib import Path
import struct

cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument("evidence", type=Path)
args = cli.parse_args()
serial = (args.evidence / "serial.log").read_text(errors="replace")
rows = re.findall(r"NET-SAMPLE eth0: ((?:\d+ ){15}\d+)\\n", serial)
assert len(rows) == 2, "expected two complete Ethernet samples"
before, after = [tuple(map(int, row.split())) for row in rows]
assert serial.count("64 bytes from 10.0.2.2:") == 4
capture = (args.evidence / "network.pcap").read_bytes()
assert capture[:4] == bytes.fromhex("d4c3b2a1")
assert struct.unpack_from("<I", capture, 20)[0] == 1  # Ethernet link type
assert b'"user,id=testnet,restrict=on"' in (args.evidence / "command.json").read_bytes()
guest = bytes.fromhex("525400123456")  # QEMU default single-card MAC
totals = [0, 0, 0, 0]  # RX bytes/packets, TX bytes/packets
prefixes = [tuple(totals)]
offset = 24
echoes = [0, 0]
while offset < len(capture):
    _, _, size, original = struct.unpack_from("<IIII", capture, offset)
    offset += 16
    frame = capture[offset : offset + size]
    offset += size
    assert len(frame) == size == original and size >= 14
    tx = frame[6:12] == guest
    if not tx:
        assert frame[:6] == guest or frame[0] & 1, "unexpected destination"
    i = 2 if tx else 0
    totals[i] += size
    totals[i + 1] += 1
    prefixes.append(tuple(totals))
    if frame[12:14] == b"\x08\x00" and len(frame) >= 42 and frame[23] == 1:
        ip_header_length = (frame[14] & 0x0F) * 4
        if frame[14 + ip_header_length] == (8 if tx else 0):
            echoes[i // 2] += 1

def metric(row):
    return row[0], row[1], row[8], row[9]

b, a = metric(before), metric(after)
assert echoes == [4, 4], echoes
assert b in prefixes and a in prefixes and prefixes.index(a) > prefixes.index(b), (b, a, prefixes)
change = tuple(high - low for low, high in zip(b, a))
assert change[1] >= 4 and change[3] >= 4
print(f"PASS: eth0 pcap matches both samples RX bytes/frames and TX bytes/frames: {b} -> {a}, delta {change}, 4 local echo pairs")
