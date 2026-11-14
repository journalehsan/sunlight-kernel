#!/usr/bin/env python3
"""Run the native TCP epoll acceptance fixture.

Modes:
  loopback  client-owned listeners + scripted support service
  external  guest client -> net_server -> QEMU user net -> host TCP peer

The QEMU guest is always 2048 MiB. Loopback traffic is guest-internal; the
external mode keeps the existing host peer and NIC capture. Historical poll,
socket and full-suite runners are intentionally not modified.
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


def guest_summary(out, guest_out, summary):
    serial = (guest_out / "serial.log").read_text(errors="replace") \
        if (guest_out / "serial.log").is_file() else ""
    results = json.loads((guest_out / "results.json").read_text()) \
        if (guest_out / "results.json").is_file() else {}
    stdout = results.get("elf", {}).get("stdout", "")
    summary["guest_stdout"] = stdout
    summary["guest_pass_line"] = next(
        (line for line in stdout.splitlines() if ": PASS" in line), None)
    summary["guest_clean"] = bool(
        re.search(r"\[NATIVE-C-RESULT\] case=0 .* clean=1 ", serial))
    summary["guest_done"] = "[NATIVE-C-DONE] cases=1" in serial
    summary["checks"]["guest_pass_line"] = summary["guest_pass_line"] is not None
    summary["checks"]["guest_clean"] = summary["guest_clean"]
    summary["checks"]["guest_done"] = summary["guest_done"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("external",), required=True)
    parser.add_argument("--cpus", type=int, choices=(2, 4), required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300,
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
        code = run(["bash", "tools/build_native_c.sh"], out / "native-archive.log", 1800)
        summary["checks"]["native_archive"] = code == 0
        if code != 0:
            (out / "poll-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
            return 1

    service_elf = None
    client_src = "tools/native-c/tests/tcp-poll-client.c"
    if opts.mode == "loopback":
        service_elf = out / "tcp-poll-service.elf"
        build = [sys.executable, "tools/native-c/cc.py", "-std=c11",
                 "-Wall", "-Wextra", "-Werror", "-Wl,-t",
                 "tools/native-c/tests/tcp-poll-service.c",
                 "-o", str(service_elf)]
        summary["commands"].append(shlex.join(build))
        if run(build, out / "service-build.log", 300) != 0:
            summary["checks"]["service_build"] = False
            (out / "poll-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
            return 1
        summary["checks"]["service_build"] = True
    else:
        client_src = "tools/native-c/tests/tcp-epoll-external-client.c"

    client_elf = out / "tcp-poll-client.elf"
    build = [sys.executable, "tools/native-c/cc.py", "-std=c11",
             "-Wall", "-Wextra", "-Werror", "-Wl,-t", client_src, "-o", str(client_elf)]
    summary["commands"].append(shlex.join(build))
    if run(build, out / "client-build.log", 300) != 0:
        summary["checks"]["client_build"] = False
        (out / "poll-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        return 1
    summary["checks"]["client_build"] = True

    peer = None
    peer_thread = None
    guest_args = []
    guest_out = out / "guest"
    if opts.mode == "external":
        peer_cmd = [sys.executable, "tools/native-c/tcp_host_peer.py",
                    "--mode", "echo", "--deadline", "120"]
        summary["commands"].append(shlex.join(peer_cmd))
        peer = subprocess.Popen(peer_cmd, cwd=ROOT, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, bufsize=1)
        messages = queue.Queue()
        peer_thread = threading.Thread(target=pump,
                                       args=(peer.stdout, messages, out / "host-peer.log"),
                                       daemon=True)
        peer_thread.start()
        ready, seen = wait_for(messages, "HOST-PEER READY", 30)
        if ready is None:
            peer.kill()
            summary["checks"]["host_peer_ready"] = False
            summary["host_peer_lines"] = seen
            (out / "poll-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
            return 1
        port = int(re.search(r"port=(\d+)", ready).group(1))
        summary["checks"]["host_peer_ready"] = True
        summary["host_peer_ready"] = ready.strip()
        summary["host_port"] = port
        summary["guest_destination"] = "%s:%d" % (host_alias, port)
        guest_args = ["--arg", str(port), "--arg", host_alias, "--arg", run_id]

    guest_cmd = [sys.executable, "tools/native-c/guest.py", str(client_elf),
                 "--net", "--cpus", str(opts.cpus), "--out", str(guest_out),
                 "--timeout", str(opts.timeout), *guest_args]
    if service_elf is not None:
        guest_cmd += ["--service", str(service_elf)]
    if opts.net_subnet:
        guest_cmd += ["--net-subnet", opts.net_subnet]
    summary["commands"].append(shlex.join(guest_cmd))
    guest_code = run(guest_cmd, out / "guest-runner.log", opts.timeout + 600)
    summary["checks"]["guest_runner"] = guest_code == 0
    guest_summary(out, guest_out, summary)

    if peer is not None:
        try:
            peer_code = peer.wait(timeout=60)
        except subprocess.TimeoutExpired:
            peer.kill()
            peer_code = 124
        peer_thread.join(timeout=10)
        summary["host_peer_exit"] = peer_code
        summary["checks"]["host_peer_exit"] = peer_code == 0

    if (guest_out / "network-summary.log").is_file():
        summary["nic_capture"] = (guest_out / "network-summary.log").read_text()

    ok = all(summary["checks"].values())
    summary["result"] = "PASS" if ok else "FAIL"
    (out / "poll-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
