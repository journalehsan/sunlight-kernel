#!/usr/bin/env python3
"""Run the native C client against a real host-side TCP peer.

Topology: guest native client -> bridge -> net_server -> virtio-net ->
QEMU user-mode networking (gateway/host alias) -> host listener.

The host peer binds 127.0.0.1:0, announces the selected port, and the guest is
started only after that explicit readiness line. Both endpoints must report
success, and the NIC capture must contain the reconstructed framed stream.

The default QEMU user-mode topology (10.0.2.0/24, host alias 10.0.2.2) is
unchanged. `--net-subnet A.B.C.D/PREFIX` passes an explicit `net=` to the
QEMU `-netdev user` backend and requires an explicit `--host-alias`; the
destination is never inferred from the DHCP lease at runtime.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shlex
import subprocess
import sys
import threading
import time

import pcap_tcp

ROOT = Path(__file__).resolve().parents[2]
SIZES = (1, 7, 64, 255, 1024)


def expected_frames():
    frames = []
    for index, size in enumerate(SIZES):
        payload = bytearray(4 + size)
        payload[0:4] = (index + 1).to_bytes(4, "big")
        for offset in range(size):
            payload[4 + offset] = (index * 37 + offset * 11) & 0xFF
        payload[4 + size // 2] = 0
        payload[4 + size // 3] = 0
        frames.append(len(payload).to_bytes(4, "big") + bytes(payload))
    return frames


def pump(stream, messages, log_path):
    with log_path.open("w") as log:
        for line in stream:
            log.write(line)
            log.flush()
            messages.put(line)
    messages.put(None)


def wait_for(messages, predicate, timeout):
    deadline = time.monotonic() + timeout
    seen = []
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return None, seen
        try:
            line = messages.get(timeout=remaining)
        except queue.Empty:
            return None, seen
        if line is None:
            return None, seen
        seen.append(line)
        if predicate(line):
            return line, seen


def run(argv, log_path, timeout, env=None):
    with log_path.open("w") as log:
        log.write("$ " + shlex.join(argv) + "\n")
        log.flush()
        try:
            proc = subprocess.run(argv, cwd=ROOT, env=env, stdout=log,
                                  stderr=subprocess.STDOUT, timeout=timeout)
            code = proc.returncode
        except subprocess.TimeoutExpired as error:
            log.write("timeout: %s\n" % error)
            code = 124
        log.write("\nexit=%d\n" % code)
    return code


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("echo", "stall"), default="echo")
    parser.add_argument("--cpus", type=int, default=2, choices=(1, 2, 4))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300, help="guest wall-clock seconds")
    parser.add_argument("--run-id", default=None)
    parser.add_argument("--net-subnet", metavar="A.B.C.D/PREFIX", default=None,
                        help="explicit QEMU user-mode subnet (default: QEMU's 10.0.2.0/24)")
    parser.add_argument("--host-alias", default=None,
                        help="explicit guest-visible host alias to connect to "
                             "(default: 10.0.2.2; required with --net-subnet)")
    parser.add_argument("--skip-build", action="store_true")
    opts = parser.parse_args()

    if opts.net_subnet and not opts.host_alias:
        parser.error("--net-subnet requires an explicit --host-alias")
    if opts.host_alias and not re.fullmatch(r"\d{1,3}(?:\.\d{1,3}){3}", opts.host_alias):
        parser.error("--host-alias must be a dotted IPv4 address")
    if opts.host_alias and any(int(part) > 255 for part in opts.host_alias.split(".")):
        parser.error("--host-alias has an out-of-range octet")
    host_alias = opts.host_alias or "10.0.2.2"

    out = opts.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    run_id = opts.run_id or ("%s-%dcpu-%s" % (opts.mode, opts.cpus, time.strftime("%Y%m%dT%H%M%SZ")))
    summary = {
        "mode": opts.mode,
        "cpus": opts.cpus,
        "memory_mib": 2048,
        "run_id": run_id,
        "net_subnet": opts.net_subnet,
        "host_alias": host_alias,
        "commands": [],
        "checks": {},
    }

    if not opts.skip_build:
        code = run(["bash", "tools/build_native_c.sh"], out / "native-archive.log", 1800)
        summary["checks"]["native_archive"] = code == 0
        if code != 0:
            (out / "external-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
            return 1

    client_src = "tools/native-c/tests/tcp-echo-client.c" if opts.mode == "echo" \
        else "tools/native-c/tests/tcp-stall-client.c"
    client_elf = out / ("tcp-%s-client.elf" % opts.mode)
    compile_cmd = [sys.executable, "tools/native-c/cc.py", "-std=c11", "-Wall", "-Wextra",
                   "-Werror", client_src, "-o", str(client_elf)]
    summary["commands"].append(shlex.join(compile_cmd))
    code = run(compile_cmd, out / "client-build.log", 300)
    summary["checks"]["client_build"] = code == 0
    if code != 0:
        (out / "external-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        return 1

    peer_cmd = [sys.executable, "tools/native-c/tcp_host_peer.py", "--mode", opts.mode,
                "--deadline", "120"]
    summary["commands"].append(shlex.join(peer_cmd))
    peer = subprocess.Popen(peer_cmd, cwd=ROOT, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, bufsize=1)
    messages = queue.Queue()
    thread = threading.Thread(target=pump, args=(peer.stdout, messages, out / "host-peer.log"),
                              daemon=True)
    thread.start()
    ready, seen = wait_for(messages, lambda line: line.startswith("HOST-PEER READY"), 30)
    if ready is None:
        peer.kill()
        summary["checks"]["host_peer_ready"] = False
        summary["host_peer_lines"] = seen
        (out / "external-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        return 1
    match = re.search(r"port=(\d+)", ready)
    port = int(match.group(1))
    summary["checks"]["host_peer_ready"] = True
    summary["host_peer_ready"] = ready.strip()
    summary["host_port"] = port
    summary["guest_destination"] = "%s:%d" % (host_alias, port)

    guest_out = out / "guest"
    guest_cmd = [sys.executable, "tools/native-c/guest.py", str(client_elf), "--net",
                 "--cpus", str(opts.cpus), "--out", str(guest_out),
                 "--timeout", str(opts.timeout),
                 "--arg", str(port), "--arg", host_alias, "--arg", run_id]
    if opts.net_subnet:
        guest_cmd += ["--net-subnet", opts.net_subnet]
    summary["commands"].append(shlex.join(guest_cmd))
    guest_code = run(guest_cmd, out / "guest-runner.log", opts.timeout + 600)
    summary["checks"]["guest_runner"] = guest_code == 0

    try:
        peer_code = peer.wait(timeout=60)
    except subprocess.TimeoutExpired:
        peer.kill()
        peer_code = 124
    thread.join(timeout=10)
    summary["host_peer_exit"] = peer_code
    summary["checks"]["host_peer_exit"] = peer_code == 0

    serial = (guest_out / "serial.log").read_text(errors="replace") \
        if (guest_out / "serial.log").is_file() else ""
    results_path = guest_out / "results.json"
    guest_stdout = ""
    if results_path.is_file():
        guest_results = json.loads(results_path.read_text())
        guest_stdout = guest_results.get("elf", {}).get("stdout", "")
    summary["guest_stdout"] = guest_stdout
    summary["guest_pass_line"] = next(
        (line for line in guest_stdout.splitlines()
         if line.startswith("tcp-%s: PASS" % opts.mode)), None)
    summary["guest_clean"] = bool(re.search(r"\[NATIVE-C-RESULT\] case=0 .* clean=1 ", serial))
    summary["guest_done"] = "[NATIVE-C-DONE] cases=1" in serial
    summary["checks"]["guest_pass_line"] = summary["guest_pass_line"] is not None
    summary["checks"]["guest_clean"] = summary["guest_clean"]
    summary["checks"]["guest_done"] = summary["guest_done"]

    lease = re.search(r"\[DHCP\] lease acquired address=([0-9.]+)/(\d+) gateway=([0-9.]+)"
                      r" dns=([0-9.]+)", serial)
    if lease:
        summary["dhcp_lease"] = {"address": lease.group(1), "prefix": int(lease.group(2)),
                                 "gateway": lease.group(3), "dns": lease.group(4)}
    summary["checks"]["dhcp_lease_present"] = lease is not None
    summary["checks"]["dhcp_gateway_matches_host_alias"] = (
        lease is not None and lease.group(3) == host_alias)

    host_log = (out / "host-peer.log").read_text(errors="replace") \
        if (out / "host-peer.log").is_file() else ""
    summary["host_peer_pass_line"] = next(
        (line.strip() for line in host_log.splitlines() if line.startswith("HOST-PEER PASS")), None)
    summary["checks"]["host_peer_pass_line"] = summary["host_peer_pass_line"] is not None

    pcap = guest_out / "network.pcap"
    if pcap.is_file():
        arp_pairs, segments, counts, ipv4_pairs = pcap_tcp.parse(pcap)
        streams = pcap_tcp.reconstruct(segments)
        summary["pcap"] = counts
        summary["arp"] = [{"opcode": op, "sender": sender, "target": target}
                          for op, sender, target in arp_pairs]
        summary["ipv4_endpoints"] = sorted({endpoint for pair in ipv4_pairs for endpoint in pair})
        summary["tcp_segments"] = len(segments)
        summary["checks"]["tcp_frames_present"] = counts["tcp"] > 0
        guest_ip = summary.get("dhcp_lease", {}).get("address")
        gateway = summary.get("dhcp_lease", {}).get("gateway")
        if opts.mode == "echo":
            frames = expected_frames()
            expected_out = b"".join(frames) + b"\x00\x00\x00\x00"
            expected_back = b"".join(frames)
        else:
            expected_out = b"\x00\x00\x00\x08stalled!"
            expected_back = b""
        outbound = None
        inbound = None
        if guest_ip and gateway:
            outbound = next((data for (src, sport, dst, dport), data in streams.items()
                             if src == guest_ip and dst == gateway and dport == port), None)
            inbound = next((data for (src, sport, dst, dport), data in streams.items()
                            if src == gateway and dst == guest_ip and sport == port), None)
        if opts.mode == "stall" and inbound is None:
            inbound = b""
        summary["reconstructed"] = {
            "outbound_bytes": len(outbound) if outbound is not None else None,
            "outbound_sha256": sha256(outbound) if outbound is not None else None,
            "inbound_bytes": len(inbound) if inbound is not None else None,
            "inbound_sha256": sha256(inbound) if inbound is not None else None,
            "expected_outbound_sha256": sha256(expected_out),
            "expected_inbound_sha256": sha256(expected_back),
        }
        summary["checks"]["outbound_stream_exact"] = outbound == expected_out
        summary["checks"]["inbound_stream_exact"] = inbound == expected_back
        summary["checks"]["arp_for_gateway"] = any(
            target == gateway or sender == gateway for _, sender, target in arp_pairs)
        summary["checks"]["arp_for_host_alias"] = any(
            target == host_alias or sender == host_alias for _, sender, target in arp_pairs)
        if opts.mode == "echo":
            summary["checks"]["bidirectional_tcp"] = bool(outbound) and bool(inbound)
        if host_alias != "10.0.2.2":
            summary["checks"]["no_legacy_alias_10_0_2_2"] = not any(
                "10.0.2.2" in pair for pair in ipv4_pairs)
    else:
        summary["checks"]["pcap_present"] = False

    ok = all(summary["checks"].values())
    summary["result"] = "PASS" if ok else "FAIL"
    (out / "external-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print("external %s: %s" % (opts.mode, summary["result"]))
    for name, value in summary["checks"].items():
        print("  %-28s %s" % (name, "PASS" if value else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
