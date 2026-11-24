#!/usr/bin/env python3
"""Run Git at the hardware TTY of a disposable BIOS QEMU guest."""

import argparse
import json
import re
import socket
import subprocess
import time
from pathlib import Path


def run(iso, output, executable, accel):
    output.mkdir(parents=True, exist_ok=False)
    serial = output / "serial.log"
    process = subprocess.Popen([
        "qemu-system-x86_64", "-cdrom", str(iso.resolve()), "-boot", "d",
        "-accel", accel, "-m", "4096", "-smp", "2", "-nic", "none",
        "-device", "virtio-rng-pci,disable-modern=on",
        "-device", "virtio-vga,xres=1280,yres=800", "-display", "none",
        "-serial", f"file:{serial.name}", "-qmp", "unix:qmp,server=on,wait=off",
        "-no-reboot", "-no-shutdown",
    ], cwd=output)

    def wait_for(predicate, description, timeout=60):
        deadline = time.monotonic() + timeout
        while process.poll() is None and time.monotonic() < deadline:
            if result := predicate():
                return result
            time.sleep(0.1)
        raise RuntimeError(f"Timed out waiting for {description}; see {serial}")

    def read_log():
        return serial.read_text(errors="replace") if serial.exists() else ""

    try:
        qmp = output / "qmp"
        wait_for(qmp.exists, "QMP")
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(15)
            connection.connect(str(qmp))
            stream = connection.makefile("rwb")
            stream.readline()

            def request(command, **arguments):
                stream.write(json.dumps({"execute": command, "arguments": arguments}).encode() + b"\n")
                stream.flush()
                while line := stream.readline():
                    reply = json.loads(line)
                    if "error" in reply:
                        raise RuntimeError(reply["error"])
                    if "return" in reply:
                        return reply["return"]
                raise RuntimeError("QMP disconnected")

            def key(name):
                request("human-monitor-command", **{"command-line": f"sendkey {name} 30"})
                time.sleep(0.075)

            def type_text(value):
                keys = {" ": "spc", "-": "minus", ".": "dot", "/": "slash",
                        "\n": "ret", "=": "equal", ">": "shift-dot", "@": "shift-2",
                        "_": "shift-minus", ":": "shift-semicolon"}
                for character in value:
                    key("shift-" + character.lower() if character.isupper()
                        else keys.get(character, character))

            def check_command(name, command, expected=None):
                start = len(read_log())
                type_text(command + "\n")
                marker = re.escape(command)
                match = wait_for(
                    lambda: re.search(rf"\[TTY\]  exit: {marker} -> (\d+)", read_log()[start:]),
                    name)
                captured = re.search(rf"\[TTY\]  cmd: {marker} -> (.*)", read_log()[start:])
                if captured is None:
                    raise RuntimeError(f"Missing TTY output for {name}")
                terminal = captured[1].replace(r"\n", "\n")
                (output / f"{name}.txt").write_text(terminal)
                result = match[1] == "0" and (not expected or re.search(expected, terminal, re.M))
                print(f"{'PASS' if result else 'FAIL'} {name} (exit {match[1]})", flush=True)
                return bool(result)

            def check(name, arguments, expected=None, cwd="/tmp/git-helios"):
                command = f"{executable} --no-pager"
                if cwd:
                    command += f" -C {cwd}"
                return check_command(name, f"{command} {arguments}", expected)

            request("qmp_capabilities")
            wait_for(lambda: "[TTY]  Login screen ready" in read_log(), "login", timeout=120)
            time.sleep(4)
            type_text("\nroot")
            for name in ("tab", "spc", "ret"):
                key(name)
            wait_for(lambda: "[TTY-IPC] shell cap lookup succeeded" in read_log(), "shell")
            results = [check("version", "--version", r"git version 2\.56\.1", cwd=None)]
            results.append(check("init", "-c init.defaultBranch=main init /tmp/git-helios", cwd=None))
            for name, args in (("name", "config user.name Fixture"),
                               ("email", "config user.email fixture@example.invalid")):
                results.append(check(name, args))
            results.append(check_command("create", "echo baseline > /tmp/git-helios/tracked.txt"))
            results.append(check("add", "add tracked.txt"))
            results.append(check("commit", "commit -m first"))
            results.append(check("status", "status", r"working tree clean"))
            results.append(check("log", "log -1 --oneline", r"\bfirst\b"))
            results.append(check_command("modify", "echo modified > /tmp/git-helios/tracked.txt"))
            results.append(check("diff", "diff", r"\+modified"))
            results.append(check("branch", "branch side"))
            results.append(check("checkout", "checkout side"))
            results.append(check("active-branch", "branch --show-current", r"^side\s*$"))
            results.append(check("clone", "clone /tmp/git-helios /tmp/git-clone", cwd=None))
            if results[-1]:
                results.append(check("clone-history", "log -1 --oneline", r"\bfirst\b",
                                     cwd="/tmp/git-clone"))
            return all(results)
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--executable", default="/bin/git")
    parser.add_argument("--accel", choices=("tcg", "kvm"), default="tcg")
    options = parser.parse_args()
    raise SystemExit(not run(options.iso, options.output, options.executable, options.accel))
