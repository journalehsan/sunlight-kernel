#!/usr/bin/env python3
"""Minimal stdlib pcap reader and TCP stream reconstructor.

Used when tshark/scapy are unavailable. It understands the Ethernet/IPv4/TCP
subset emitted by QEMU's filter-dump for this fixture; it is not a general
packet analyser.
"""
import struct


def read_frames(path):
    data = path.read_bytes()
    if len(data) < 24 or data[:4] != b"\xd4\xc3\xb2\xa1":
        raise ValueError("unreadable pcap")
    offset = 24
    frames = []
    while offset + 16 <= len(data):
        _, _, incl, _ = struct.unpack_from("<IIII", data, offset)
        offset += 16
        if offset + incl > len(data):
            break
        frames.append(data[offset:offset + incl])
        offset += incl
    return frames


def parse(path):
    """Return (arp_pairs, tcp_segments, counts, ipv4_pairs)."""
    arp_pairs = []
    segments = []
    ipv4_pairs = []
    counts = {"frames": 0, "arp": 0, "ipv4": 0, "tcp": 0, "udp": 0}
    for frame in read_frames(path):
        counts["frames"] += 1
        if len(frame) < 14:
            continue
        ethertype = int.from_bytes(frame[12:14], "big")
        if ethertype == 0x0806 and len(frame) >= 42:
            counts["arp"] += 1
            sender = ".".join(str(b) for b in frame[28:32])
            target = ".".join(str(b) for b in frame[38:42])
            opcode = int.from_bytes(frame[20:22], "big")
            arp_pairs.append((opcode, sender, target))
            continue
        if ethertype != 0x0800 or len(frame) < 34:
            continue
        counts["ipv4"] += 1
        ip = frame[14:]
        ihl = (ip[0] & 0x0F) * 4
        if ihl < 20 or len(ip) < ihl:
            continue
        total = int.from_bytes(ip[2:4], "big")
        proto = ip[9]
        src = ".".join(str(b) for b in ip[12:16])
        dst = ".".join(str(b) for b in ip[16:20])
        ipv4_pairs.append((src, dst))
        if proto == 17:
            counts["udp"] += 1
            continue
        if proto != 6 or len(ip) < ihl + 20:
            continue
        counts["tcp"] += 1
        tcp = ip[ihl:total]
        sport, dport, seq, ack, offset_flags = struct.unpack_from("!HHIIH", tcp, 0)
        data_offset = ((offset_flags >> 12) & 0x0F) * 4
        flags = offset_flags & 0x01FF
        payload = tcp[data_offset:] if data_offset <= len(tcp) else b""
        segments.append({
            "src": src, "dst": dst, "sport": sport, "dport": dport,
            "seq": seq, "ack": ack, "flags": flags, "payload": payload,
        })
    return arp_pairs, segments, counts, ipv4_pairs


def reconstruct(segments):
    """Return {(src, sport, dst, dport): bytes} preserving sequence order."""
    flows = {}
    for segment in segments:
        if not segment["payload"]:
            continue
        key = (segment["src"], segment["sport"], segment["dst"], segment["dport"])
        flows.setdefault(key, {})[segment["seq"]] = segment["payload"]
    streams = {}
    for key, chunks in flows.items():
        base = min(chunks)
        end = max(seq + len(payload) for seq, payload in chunks.items())
        buffer = bytearray(end - base)
        for seq, payload in chunks.items():
            buffer[seq - base:seq - base + len(payload)] = payload
        streams[key] = bytes(buffer)
    return streams
