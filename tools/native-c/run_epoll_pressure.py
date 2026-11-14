#!/usr/bin/env python3
"""Run the native epoll LT POLLOUT send-backpressure / recovery fixture.

external (default): guest native client -> net_server -> virtio-net -> QEMU
slirp -> host peer (tools/native-c/tcp_pressure_peer.py). The host peer
announces the control/data/reset ports with an explicit READY line; the guest
is started only after that.

loopback: guest native client -> net_server -> smoltcp; the peer is a second
guest process (tcp-poll-pressure-service) that withholds reads on command.
Peer reset is external-only because native libc exposes no setsockopt/SO_LINGER
and shutdown() is unsupported.

Saturation and recovery are driven over the control channel, never by fixed
sleeps, and the deterministic stream is validated by the peer. QEMU is always
2048 MiB. Historical runners and fixtures are not modified.
"""
import argparse
import json
from pathlib import Path
import queue
import re
import shlex
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[2]


def run(argv, log, timeout):
    with log.open("w") as stream:
        stream.write("$ " + shlex.join(argv) + "\n")
        stream.flush()
        try:
            proc = subprocess.run(argv, cwd=ROOT, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=timeout)
            code = proc.returncode
        except subprocess.TimeoutExpired as error:
            stream.write("timeout: %s\n" % error)
            code = 124
        stream.write("\nexit=%d\n" % code)
    return code


def pump(stream, messages, log_path):
    with log_path.open("w") as log:
        for line in stream:
            log.write(line)
            log.flush()
            messages.put(line)
    messages.put(None)


def wait_for(messages, prefix, timeout):
    deadline = time.monotonic() + timeout
    seen = []
    while time.monotonic() < deadline:
        try:
            line = messages.get(timeout=deadline - time.monotonic())
        except queue.Empty:
            break
        if line is None:
            break
        seen.append(line)
        if line.startswith(prefix):
            return line, seen
    return None, seen


def guest_summary(guest_out, summary):
    serial = (guest_out / "serial.log").read_text(errors="replace") \
        if (guest_out / "serial.log").is_file() else ""
    results = json.loads((guest_out / "results.json").read_text()) \
        if (guest_out / "results.json").is_file() else {}
    stdout = results.get("elf", {}).get("stdout", "")
    summary["guest_stdout"] = stdout
    summary["guest_pass_line"] = next(
        (line for line in stdout.splitlines() if ": PASS" in line), None)
    summary["guest_fail_line"] = next(
        (line for line in stdout.splitlines() if ": FAIL" in line), None)
    summary["guest_clean"] = bool(
        re.search(r"\[NATIVE-C-RESULT\] case=0 .* clean=1 ", serial))
    summary["guest_done"] = "[NATIVE-C-DONE] cases=1" in serial
    summary["checks"]["guest_pass_line"] = summary["guest_pass_line"] is not None
    summary["checks"]["guest_clean"] = summary["guest_clean"]
    summary["checks"]["guest_done"] = summary["guest_done"]
    lease = re.search(r"\[DHCP\] lease acquired address=([0-9.]+)/(\d+) gateway=([0-9.]+)"
                      r" dns=([0-9.]+)", serial)
    if lease:
        summary["dhcp_lease"] = {"address": lease.group(1),
                                 "prefix": int(lease.group(2)),
                                 "gateway": lease.group(3), "dns": lease.group(4)}
    summary["checks"]["dhcp_lease_present"] = lease is not None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("external", "loopback"),
                        default="external")
    parser.add_argument("--cpus", type=int, choices=(2, 4), required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=420,
                        help="guest wall-clock seconds")
    parser.add_argument("--run-id")
    parser.add_argument("--net-subnet")
    parser.add_argument("--host-alias")
    parser.add_argument("--skip-build", action="store_true")
    opts = parser.parse_args()
    if opts.net_subnet and not opts.host_alias:
        parser.error("--net-subnet requires --host-alias")

    out = opts.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    run_id = opts.run_id or "%s-%dcpu-%s" % (
        opts.mode, opts.cpus, time.strftime("%Y%m%dT%H%M%SZ"))
    host_alias = opts.host_alias or "10.0.2.2"
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
        code = run(["bash", "tools/build_native_c.sh"],
                   out / "native-archive.log", 1800)
        summary["checks"]["native_archive"] = code == 0
        if code != 0:
            (out / "pressure-summary.json").write_text(
                json.dumps(summary, indent=2) + "\n")
            return 1

    client_src = ("tools/native-c/tests/tcp-epoll-pressure-client.c"
                  if opts.mode == "external"
                  else "tools/native-c/tests/tcp-epoll-pressure-loopback-client.c")
    client_elf = out / "tcp-poll-pressure-client.elf"
    build = [sys.executable, "tools/native-c/cc.py", "-std=c11",
             "-Wall", "-Wextra", "-Werror", "-Wl,-t",
             client_src,
             "-o", str(client_elf)]
    summary["commands"].append(shlex.join(build))
    if run(build, out / "client-build.log", 300) != 0:
        summary["checks"]["client_build"] = False
        (out / "pressure-summary.json").write_text(
            json.dumps(summary, indent=2) + "\n")
        return 1
    summary["checks"]["client_build"] = True

    peer = None
    peer_thread = None
    guest_out = out / "guest"
    service_elf = None
    if opts.mode == "loopback":
        service_elf = out / "tcp-poll-pressure-service.elf"
        build_service = [sys.executable, "tools/native-c/cc.py", "-std=c11",
                         "-Wall", "-Wextra", "-Werror", "-Wl,-t",
                         "tools/native-c/tests/tcp-poll-pressure-service.c",
                         "-o", str(service_elf)]
        summary["commands"].append(shlex.join(build_service))
        if run(build_service, out / "service-build.log", 300) != 0:
            summary["checks"]["service_build"] = False
            (out / "pressure-summary.json").write_text(
                json.dumps(summary, indent=2) + "\n")
            return 1
        summary["checks"]["service_build"] = True
        guest_cmd = [sys.executable, "tools/native-c/guest.py", str(client_elf),
                     "--net", "--cpus", str(opts.cpus), "--out", str(guest_out),
                     "--timeout", str(opts.timeout), "--service", str(service_elf)]
    else:
        peer_cmd = [sys.executable, "tools/native-c/tcp_pressure_peer.py",
                    "--deadline", "180"]
        summary["commands"].append(shlex.join(peer_cmd))
        peer = subprocess.Popen(peer_cmd, cwd=ROOT, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, bufsize=1)
        messages = queue.Queue()
        peer_thread = threading.Thread(
            target=pump, args=(peer.stdout, messages, out / "host-peer.log"),
            daemon=True)
        peer_thread.start()
        ready, seen = wait_for(messages, "HOST-PEER READY", 30)
        if ready is None:
            peer.kill()
            summary["checks"]["host_peer_ready"] = False
            summary["host_peer_lines"] = seen
            (out / "pressure-summary.json").write_text(
                json.dumps(summary, indent=2) + "\n")
            return 1
        ports = dict(re.findall(r"(\w+)=(\d+)", ready))
        summary["checks"]["host_peer_ready"] = True
        summary["host_peer_ready"] = ready.strip()
        summary["host_ports"] = ports
        summary["guest_destination"] = "%s:ctrl=%s data=%s reset=%s" % (
            host_alias, ports.get("ctrl"), ports.get("data"), ports.get("reset"))
        guest_cmd = [sys.executable, "tools/native-c/guest.py", str(client_elf),
                     "--net", "--cpus", str(opts.cpus), "--out", str(guest_out),
                     "--timeout", str(opts.timeout),
                     "--arg", host_alias, "--arg", ports["ctrl"],
                     "--arg", ports["data"], "--arg", ports["reset"],
                     "--arg", run_id]
    if opts.net_subnet:
        guest_cmd += ["--net-subnet", opts.net_subnet]
    summary["commands"].append(shlex.join(guest_cmd))
    guest_code = run(guest_cmd, out / "guest-runner.log", opts.timeout + 600)
    summary["checks"]["guest_runner"] = guest_code == 0
    guest_summary(guest_out, summary)

    if peer is not None:
        try:
            peer_code = peer.wait(timeout=90)
        except subprocess.TimeoutExpired:
            peer.kill()
            peer_code = 124
        peer_thread.join(timeout=10)
        summary["host_peer_exit"] = peer_code
        summary["checks"]["host_peer_exit"] = peer_code == 0

        host_log = (out / "host-peer.log").read_text(errors="replace") \
            if (out / "host-peer.log").is_file() else ""
        summary["host_peer_pass_line"] = next(
            (line.strip() for line in host_log.splitlines()
             if line.startswith("HOST-PEER PASS")), None)
        summary["checks"]["host_peer_pass_line"] = \
            summary["host_peer_pass_line"] is not None

    if (guest_out / "network-summary.log").is_file():
        summary["nic_capture"] = (guest_out / "network-summary.log").read_text()
    pcap = guest_out / "network.pcap"
    if pcap.is_file():
        try:
            import pcap_tcp
            counts = pcap_tcp.parse(pcap)[2]
            summary["pcap"] = counts
            if opts.mode == "external":
                summary["checks"]["tcp_frames_present"] = counts.get("tcp", 0) > 0
            else:
                # Loopback traffic stays inside the guest smoltcp interface;
                # the sibling check is that no TCP frame crosses the NIC.
                summary["checks"]["loopback_no_nic_tcp"] = counts.get("tcp", 0) == 0
        except Exception as error:  # noqa: BLE001
            summary["pcap_error"] = str(error)

    ok = all(summary["checks"].values())
    summary["result"] = "PASS" if ok else "FAIL"
    (out / "pressure-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
