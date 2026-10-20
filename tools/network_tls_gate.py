#!/usr/bin/env python3
"""Verify native TCP/TLS against local fixtures, without upstream DNS or HTTPS.

Build the normal ISO first, then run:
    python3 tools/network_tls_gate.py --cpus 12 --uefi
Requires QEMU and OpenSSL. Uses a fresh, diskless guest and an ephemeral test CA.
Serial output and packet capture remain under target/network-tls/.
"""

import argparse
import http.server
import json
import os
from pathlib import Path
import socket
import socketserver
import select
import ssl
import subprocess
import tempfile
import threading
import time


class Fixture(http.server.BaseHTTPRequestHandler):
    ca_der = b""

    def log_message(self, *_args):
        pass

    def respond(self, send_body):
        data = self.ca_der if self.path == "/ca.der" else b"SUNLIGHT TLS FIXTURE OK\n"
        # Keep TCP alive while timed/resolved exercise their UDP poll paths.
        if self.path == "/slow":
            time.sleep(3)
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if send_body:
            self.wfile.write(data)

    def do_HEAD(self):
        self.respond(False)

    def do_GET(self):
        self.respond(True)


LONG_HOST = "long-hostname-for-tls.sunlight.test"


class SocksFixture(socketserver.BaseRequestHandler):
    allowed_ports = set()
    seen_hosts = []

    def read_exact(self, count):
        result = b""
        while len(result) < count:
            block = self.request.recv(count - len(result))
            if not block:
                raise ConnectionError("SOCKS client closed")
            result += block
        return result

    def handle(self):
        self.request.settimeout(20)
        if self.read_exact(3) != b"\x05\x01\x00":
            return
        self.request.sendall(b"\x05\x00")
        if self.read_exact(4) != b"\x05\x01\x00\x03":
            return
        name = self.read_exact(self.read_exact(1)[0]).decode("ascii")
        port = int.from_bytes(self.read_exact(2), "big")
        if port not in self.allowed_ports or name not in ("10.0.2.2", LONG_HOST):
            self.request.sendall(b"\x05\x02\x00\x01\x7f\x00\x00\x01\x00\x00")
            return
        self.seen_hosts.append(name)
        with socket.create_connection(("127.0.0.1", port), timeout=20) as upstream:
            self.request.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
            while True:
                ready, _, _ = select.select([self.request, upstream], [], [], 20)
                if not ready:
                    return
                for source in ready:
                    data = source.recv(8192)
                    if not data:
                        return
                    (upstream if source is self.request else self.request).sendall(data)


class SocksServer(socketserver.ThreadingTCPServer):
    daemon_threads = True


def certificates(root):
    def openssl(*args):
        subprocess.run(["openssl", *args], cwd=root, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    (root / "server.ext").write_text(
        "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\n"
        f"extendedKeyUsage=serverAuth\nsubjectAltName=IP:10.0.2.2,DNS:{LONG_HOST}\n")
    openssl("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
            "-nodes", "-days", "2", "-subj", "/CN=Sunlight Regression CA",
            "-addext", "basicConstraints=critical,CA:TRUE", "-keyout", "ca.key", "-out", "ca.pem")
    openssl("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
            "-subj", "/CN=Sunlight Regression Server", "-keyout", "server.key", "-out", "server.csr")
    openssl("x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key",
            "-CAcreateserial", "-days", "2", "-extfile", "server.ext", "-out", "server.pem")
    openssl("x509", "-in", "ca.pem", "-outform", "DER", "-out", "ca.der")


def gate(args, root):
    certificates(root)
    Fixture.ca_der = (root / "ca.der").read_bytes()
    servers = []
    process = None
    try:
        for secure in (False, True):
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
            servers.append(server)
            if secure:
                context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
                context.load_cert_chain(root / "server.pem", root / "server.key")
                server.socket = context.wrap_socket(server.socket, server_side=True)
            threading.Thread(target=server.serve_forever, daemon=True).start()

        SocksFixture.allowed_ports = {server.server_port for server in servers}
        socks = SocksServer(("127.0.0.1", 0), SocksFixture)
        servers.append(socks)
        threading.Thread(target=socks.serve_forever, daemon=True).start()

        output = Path(args.evidence).resolve()
        output.mkdir(parents=True, exist_ok=True)
        serial = output / "serial.log"
        serial.write_text("")
        qmp_path = root / "qmp.sock"
        command = [
            "qemu-system-x86_64", "-cdrom", str(Path(args.iso).resolve()),
            "-display", "none", "-serial", f"file:{serial}", "-m", "4096M",
            "-smp", str(args.cpus), "-qmp", f"unix:{qmp_path},server=on,wait=off",
            "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0,disable-modern=on",
            "-object", f"filter-dump,id=dump,netdev=net0,file={output / 'network.pcap'}",
            "-device", "virtio-rng-pci,disable-modern=on", "-no-reboot", "-no-shutdown",
        ]
        if os.access("/dev/kvm", os.R_OK | os.W_OK):
            command += ["-enable-kvm", "-cpu", "host"]
        else:
            command += ["-accel", "tcg,thread=multi", "-cpu", "max"]
        if args.uefi:
            candidates = [Path(p) for p in (
                "/usr/share/edk2/x64", "/usr/share/edk2-ovmf/x64", "/usr/share/OVMF")]
            firmware = next((p for p in candidates if (p / "OVMF_CODE.4m.fd").exists()), None)
            if firmware is None:
                raise RuntimeError("Cannot find 4 MiB OVMF firmware")
            command += ["-drive", f"if=pflash,format=raw,readonly=on,file={firmware / 'OVMF_CODE.4m.fd'}",
                        "-drive", f"if=pflash,format=raw,snapshot=on,file={firmware / 'OVMF_VARS.4m.fd'}"]

        with (output / "qemu.log").open("w") as qemu_log:
            process = subprocess.Popen(command, stdout=qemu_log, stderr=qemu_log)

        def wait_for(marker):
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                text = serial.read_text(errors="replace")
                if any(s in text for s in ("KERNEL PANIC", "[EXCEPTION]", "[FAULT]")):
                    raise RuntimeError(f"Guest fault; see {serial}")
                if marker in text:
                    return text
                if process.poll() is not None:
                    raise RuntimeError(f"QEMU exited; see {output / 'qemu.log'}")
                time.sleep(0.2)
            raise RuntimeError(f"Timed out waiting for {marker}; see {serial}")

        wait_for("[LOGIN-TUI] input ready")
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(10)
            connection.connect(str(qmp_path))
            stream = connection.makefile("rwb")
            stream.readline()

            def request(execute, arguments=None):
                payload = {"execute": execute}
                if arguments:
                    payload["arguments"] = arguments
                stream.write(json.dumps(payload).encode() + b"\n")
                stream.flush()
                while True:
                    response = json.loads(stream.readline())
                    if "error" in response:
                        raise RuntimeError(response["error"])
                    if "return" in response:
                        return

            def type_text(text):
                keys = {" ": "spc", "/": "slash", ":": "shift-semicolon",
                        ".": "dot", "-": "minus", "\n": "ret"}
                for character in text:
                    request("human-monitor-command", {
                        "command-line": "sendkey " + keys.get(character, character) + " 35"})
                    time.sleep(0.085)

            request("qmp_capabilities")
            type_text("\nroot\n")
            wait_for("[TTY]  Built-in shell ready")
            wait_for("[SUNLIGHT-TLS] Entering IPC loop")
            http_url = f"http://10.0.2.2:{servers[0].server_port}"
            https_url = f"https://10.0.2.2:{servers[1].server_port}"
            proxy = f"--socks5-hostname 10.0.2.2:{socks.server_address[1]}"
            cases = [
                (f"fetch -o /tmp/untrusted {https_url}/payload", 1),
                (f"fetch -o /tmp/ca.der {http_url}/ca.der", 0),
                ("certificatectl install ca testca /tmp/ca.der", 0),
                *[(f"fetch -o /tmp/verified{i} {https_url}/slow", 0) for i in range(3)],
                (f"fetch {proxy} -o /tmp/proxy-denied https://blocked.sunlight.test:{servers[1].server_port}/payload", 1),
                (f"fetch {proxy} -o /tmp/proxy-https https://{LONG_HOST}:{servers[1].server_port}/payload", 0),
                (f"fetch {proxy} -o /tmp/proxy-http http://{LONG_HOST}:{servers[0].server_port}/payload", 0),
            ]
            cases.append((f"fetch --resolve {LONG_HOST}:{servers[1].server_port}:10.0.2.2 -o /tmp/direct-https https://{LONG_HOST}:{servers[1].server_port}/payload", 0))
            cases.append((f"fetch --resolve wrong.sunlight.test:{servers[1].server_port}:10.0.2.2 -o /tmp/wrong-host https://wrong.sunlight.test:{servers[1].server_port}/payload", 1))
            if args.arch_resolve:
                cases.append((f"fetch --resolve archlinux.org:443:{args.arch_resolve} -o /tmp/arch-direct.html https://archlinux.org", 0))
            if args.arch_proxy:
                cases.append((f"fetch --socks5-hostname {args.arch_proxy} -o /tmp/archlinux.html https://archlinux.org", 0))
            for command, expected in cases:
                type_text(command + "\n")
                text = wait_for(f"[TTY]  exit: {command} ->")
                if f"[TTY]  exit: {command} -> {expected}\n" not in text:
                    raise RuntimeError(f"Unexpected exit status: {command}; see {serial}")
                print(f"PASS (exit {expected}): {command}", flush=True)
            if "InvalidCertificate(UnknownIssuer)" not in text:
                raise RuntimeError("Untrusted certificate was not rejected as UnknownIssuer")
            if "[SUNLIGHT-TLS] hs_OK host=10.0.2.2" not in text:
                raise RuntimeError("Missing successful verified TLS handshake")
            if f"[SUNLIGHT-TLS] hs_OK host={LONG_HOST}" not in text:
                raise RuntimeError("Full TLS hostname did not survive IPC")
            if LONG_HOST not in SocksFixture.seen_hosts:
                raise RuntimeError("SOCKS proxy did not receive the destination hostname")
            if f"[DNSDBG] resolve request host='{LONG_HOST}'" in text:
                raise RuntimeError("SOCKS request leaked into local DNS")
            if (args.arch_proxy or args.arch_resolve) and "[SUNLIGHT-TLS] hs_OK host=archlinux.org" not in text:
                raise RuntimeError("Arch HTTPS verification did not succeed")
            print(f"TCP/TLS gate passed on {args.cpus} CPUs; evidence: {output}", flush=True)
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        for server in servers:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", default="target/sunlightos.iso")
    parser.add_argument("--cpus", type=int, default=12)
    parser.add_argument("--uefi", action="store_true")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--evidence", default="target/network-tls")
    parser.add_argument("--arch-resolve", help="Also fetch Arch directly using this IPv4 DNS override")
    parser.add_argument("--arch-proxy", help="Also fetch archlinux.org through this guest-visible SOCKS IPv4:port")
    arguments = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="sunlight-tls-") as directory:
        gate(arguments, Path(directory))
