#!/usr/bin/env python3
"""Verify native TCP/TLS against local fixtures, without upstream DNS or HTTPS.

Build the normal ISO first, then run:
    python3 tools/network_tls_gate.py --cpus 12 --uefi
Requires QEMU and OpenSSL. Uses a fresh, diskless guest and an ephemeral test CA.
Serial output and packet capture remain under target/network-tls/.
"""

import base64
import html
import hashlib
import urllib.parse
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
    codes = {}
    authorizations = 0
    exchanges = 0
    issued_secrets = set()
    connection_requests = 0
    shield_requests = []

    def log_message(self, *_args):
        pass

    def respond(self, send_body):
        if self.path == "/ca.der":
            data = self.ca_der
        elif self.path == "/large":
            data = LARGE_BODY
        else:
            data = b"SUNLIGHT TLS FIXTURE OK\n"
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
        url = urllib.parse.urlsplit(self.path)
        if url.path in ("/shield", "/images/good.jpg", "/ads/banners/sale.png", "/css/ads-native.css"):
            type(self).shield_requests.append(url.path)
            root = Path(__file__).resolve().parents[1]
            if url.path == "/shield":
                data = (root / "rappid-rabbit/tests/fixtures/content-shield.html").read_bytes()
                mime = "text/html"
            elif url.path.endswith(".css"):
                data, mime = b"body { background: red; }", "text/css"
            else:
                data, mime = (root / "sun-img/tests/fixtures/gray.jpg").read_bytes(), "image/jpeg"
            self.send_response(200)
            self.send_header("Content-Type", mime)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if url.path.startswith("/connections/"):
            # Eleven separate TLS connections per fetch, all owned by the
            # same daemon. Twelve batches exceed both its former ~84-socket
            # heap failure point and the TCP table's 128-slot limit.
            remaining = int(url.path.rsplit("/", 1)[1])
            type(self).connection_requests += 1
            if remaining:
                self.send_response(302)
                self.send_header("Location", f"/connections/{remaining - 1}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
        if url.path.startswith("/authorize/"):
            fields = urllib.parse.parse_qs(url.query, strict_parsing=True)
            case = url.path.rsplit("/", 1)[1]
            assert fields["client_id"] == ["sunlight-native-mock"]
            assert fields["code_challenge_method"] == ["S256"]
            assert fields["scope"] == ["mail.read"]
            redirect = fields["redirect_uri"][0]
            parsed = urllib.parse.urlsplit(redirect)
            assert parsed.hostname == "127.0.0.1" and parsed.port and parsed.path == "/oauth/callback"
            type(self).authorizations += 1
            if case == "interactive":
                href = html.escape("/authorize/success?" + url.query, quote=True)
                body = ("<html><head><title>Microsoft - Mock test mode</title></head><body>"
                        "<h1>Microsoft mock sign-in</h1><p>Local test provider. No real Microsoft credentials required.</p>"
                        "<p>Connect mock@sunlight.test to SunlightOS for this live session.</p>"
                        f'<p><a href="{href}">Authorize Microsoft mock account</a></p>'
                        "<p>You can cancel sign-in from Online Accounts.</p></body></html>").encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if case in ("timeout", "cancel"):
                body = b"<title>Mock authorization pending</title>"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            state = fields["state"][0]
            code = hashlib.sha256(state.encode()).hexdigest()
            type(self).issued_secrets.add(code)
            type(self).codes[code] = (fields, case)
            query = {"state": "wrong" if case == "wrong-state" else state}
            query.update({"error": "access_denied"} if case == "denied" else {"code": code})
            self.send_response(302)
            self.send_header("Location", redirect + "?" + urllib.parse.urlencode(query))
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.respond(True)

    def do_POST(self):
        assert self.path == "/token"
        length = int(self.headers.get("Content-Length", "0"))
        assert 0 < length < 8192
        posted = urllib.parse.parse_qs(self.rfile.read(length).decode(), strict_parsing=True)
        fields, case = type(self).codes.pop(posted["code"][0])
        type(self).issued_secrets.add(posted["code_verifier"][0])
        assert posted["client_id"] == fields["client_id"]
        assert posted["redirect_uri"] == fields["redirect_uri"]
        assert posted["grant_type"] == ["authorization_code"]
        challenge = base64.urlsafe_b64encode(hashlib.sha256(posted["code_verifier"][0].encode()).digest()).rstrip(b"=").decode()
        assert [challenge] == fields["code_challenge"]
        type(self).exchanges += 1
        body = (b'{"error":"mock_error"}' if case == "token-error" else
                b'{"access_token":"native-mock-access","refresh_token":"native-mock-refresh","token_type":"Bearer","expires_in":3600,"scope":"mail.read"}')
        if case == "exchange-timeout":
            self.send_response(200)
            self.send_header("Content-Length", "12000")
            self.end_headers()
            try:
                for _ in range(120):
                    self.wfile.write(b" " * 100)
                    self.wfile.flush()
                    time.sleep(1)
            except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
                pass
            return
        if case == "token-timeout":
            time.sleep(10)
            return
        self.send_response(500 if case == "token-error" else 200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
            pass


LONG_HOST = "long-hostname-for-tls.sunlight.test"

# 192 KiB = 48 TLS receives (4 KiB shared page each). Repeated
# LARGE_FETCH_REPEATS times this drives 384 receives through the single
# sunlight-tls daemon in one boot: a per-receive shared-region leak would
# exhaust the daemon's 256-region address-space ledger partway through and fail
# a later transfer, while a correct map/copy/free path streams every one of
# them and leaves the daemon healthy. Each transfer stays well below fetch's
# 2 MiB no-reclaim bump heap (which leaks per-chunk scratch), so the client
# never becomes the limiting factor.
LARGE_BODY_SIZE = 192 * 1024
LARGE_BODY = (b"sunlight-https-large-body\n" * ((LARGE_BODY_SIZE // 26) + 1))[:LARGE_BODY_SIZE]
LARGE_FETCH_REPEATS = 8
CONNECTION_BATCHES = 12
CONNECTION_REDIRECTS = 10


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
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 8443 if secure and args.account_broker_flow else 0), Fixture)
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
        if not args.tcg and os.access("/dev/kvm", os.R_OK | os.W_OK):
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

        def wait_for(marker, start=0):
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                text = serial.read_text(errors="replace")
                if any(s in text for s in ("KERNEL PANIC", "[EXCEPTION]", "[FAULT]")):
                    raise RuntimeError(f"Guest fault; see {serial}")
                if marker in text[start:]:
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
                        "command-line": "sendkey " + keys.get(character, character) + " 50"})
                    time.sleep(0.13)

            request("qmp_capabilities")
            type_text("\nroot\n")
            wait_for("[TTY]  Built-in shell ready")
            wait_for("[SUNLIGHT-TLS] Entering IPC loop")
            http_url = f"http://10.0.2.2:{servers[0].server_port}"
            https_url = f"https://10.0.2.2:{servers[1].server_port}"
            proxy = f"--socks5-hostname 10.0.2.2:{socks.server_address[1]}"
            if args.account_broker_probe:
                wait_for("[ACCOUNT-BROKER] native service ready")
            cases = [
                (f"fetch -o /tmp/untrusted {https_url}/payload", 1),
                (f"fetch -o /tmp/ca.der {http_url}/ca.der", 0),
                ("certificatectl install ca testca /tmp/ca.der", 0),
                *[(f"fetch -o /tmp/connections{i} {https_url}/connections/{CONNECTION_REDIRECTS}", 0)
                  for i in range(CONNECTION_BATCHES)],
                (f"fetch -o /tmp/after-connections {http_url}/payload", 0),
                *[(f"fetch -o /tmp/large{i} {https_url}/large", 0) for i in range(LARGE_FETCH_REPEATS)],
                (f"fetch -o /tmp/after-large {https_url}/payload", 0),
                *[(f"fetch -o /tmp/verified{i} {https_url}/slow", 0) for i in range(3)],
                (f"fetch {proxy} -o /tmp/proxy-denied https://blocked.sunlight.test:{servers[1].server_port}/payload", 1),
                (f"fetch {proxy} -o /tmp/proxy-https https://{LONG_HOST}:{servers[1].server_port}/payload", 0),
                (f"fetch {proxy} -o /tmp/proxy-http http://{LONG_HOST}:{servers[0].server_port}/payload", 0),
            ]
            if args.account_broker_probe:
                cases.insert(0, ("/sbin/sunlight-account-broker --probe", 0))
                cases.insert(1, ("/sbin/sunlight-account-broker --loopback-probe", 0))
            cases.append((f"fetch --resolve {LONG_HOST}:{servers[1].server_port}:10.0.2.2 -o /tmp/direct-https https://{LONG_HOST}:{servers[1].server_port}/payload", 0))
            cases.append((f"fetch --resolve wrong.sunlight.test:{servers[1].server_port}:10.0.2.2 -o /tmp/wrong-host https://wrong.sunlight.test:{servers[1].server_port}/payload", 1))
            if args.account_broker_flow:
                index=next(i for i,(command,_) in enumerate(cases) if command.startswith("certificatectl install"))+1
                cases[index:index]=[(f"/sbin/sunlight-account-broker --mock 8443 {mode}",0) for mode in range(10)]
            if args.account_broker_storage:
                cases += [
                    ("/sbin/sunlight-account-broker --storage-seed", 0),
                    ("sunlightctl restart sunlight-kv", 0),
                    ("/sbin/sunlight-account-broker --storage-check", 0),
                    (f"fetch -o /tmp/restart-tls {https_url}/payload", 0),
                    ("/sbin/sunlight-account-broker --storage-delete", 0),
                    ("sunlightctl restart sunlight-kv", 0),
                    ("/sbin/sunlight-account-broker --storage-absent", 0),
                    ("cat /var/lib/sunlight-kv/broker.store", 1),
                ]
            if args.account_provider_preflight:
                cases.append(("/sbin/sunlight-account-broker --real-provider", 3))
            if args.arch_resolve:
                cases.append((f"fetch --resolve archlinux.org:443:{args.arch_resolve} -o /tmp/arch-direct.html https://archlinux.org", 0))
            if args.arch_proxy:
                cases.append((f"fetch --socks5-hostname {args.arch_proxy} -o /tmp/archlinux.html https://archlinux.org", 0))
            if args.ui_only:
                cases = [(f"fetch -o /tmp/ca.der {http_url}/ca.der", 0),
                         ("certificatectl install ca testca /tmp/ca.der", 0)]
            if args.content_shield_only:
                cases = []
            for command, expected in cases:
                start = len(serial.read_text(errors="replace"))
                type_text(command + "\n")
                text = wait_for(f"[TTY]  exit: {command} ->", start)
                if f"[TTY]  exit: {command} -> {expected}\n" not in text:
                    raise RuntimeError(f"Unexpected exit status: {command}; see {serial}")
                print(f"PASS (exit {expected}): {command}", flush=True)
            if not args.ui_only and not args.content_shield_only:
                expected_connections = CONNECTION_BATCHES * (CONNECTION_REDIRECTS + 1)
                if Fixture.connection_requests != expected_connections:
                    raise RuntimeError(f"Connection soak incomplete: {Fixture.connection_requests}/{expected_connections}")
                print(f"PASS: {expected_connections} TLS connections without daemon restart", flush=True)
                if args.account_broker_probe and "[ACCOUNT-BROKER] native IPC probe PASS" not in text:
                    raise RuntimeError("Missing native account broker IPC probe")
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
                if args.account_broker_flow:
                    assert Fixture.authorizations >= 6, "Rabbit did not open mock authorizations"
                    assert Fixture.exchanges == 5, "Unexpected number of one-use token exchanges"
                    required = ["native mock code exchanged; completion", "callback rejected: state",
                                "native HTTP callback replay rejected; volatile writes=1", "callback timeout",
                                "cancelled; listener and session released", "provider failure",
                                "[RABBIT] native broker authorization URL received"]
                    assert all(marker in text for marker in required), "Missing native broker outcome evidence"
                    assert "native-mock-access" not in text and "native-mock-refresh" not in text
                    assert all(secret not in text for secret in Fixture.issued_secrets), "Code/verifier appeared in guest serial output"
                    print("Native Rabbit/local mock authorization gate passed (not real-provider sign-in)", flush=True)
                if args.account_provider_preflight:
                    assert "real provider blocked: local Microsoft client ID missing or invalid" in text
                    print("Real-provider preflight BLOCKED: no local Microsoft registration; no live authorization attempted", flush=True)
            if args.online_accounts_ui:
                from online_accounts_ui_gate import verify_ui
                verify_ui(request, type_text, wait_for, serial, output)
                text = serial.read_text(errors="replace")
                assert "native-mock-access" not in text and "native-mock-refresh" not in text
                assert all(secret not in text for secret in Fixture.issued_secrets)
            if args.content_shield:
                from content_shield_ui_gate import verify_ui
                verify_ui(request, type_text, wait_for, serial, output, http_url, Fixture.shield_requests)
                assert "/shield" in Fixture.shield_requests and "/images/good.jpg" in Fixture.shield_requests
                assert Fixture.shield_requests.count("/ads/banners/sale.png") == 1
                assert Fixture.shield_requests.count("/css/ads-native.css") == 1
                print("Content Shield native request, DOM, settings and UI gate passed", flush=True)
            label = "Content Shield" if args.content_shield_only else "TCP/TLS"
            print(f"{label} gate passed on {args.cpus} CPUs; evidence: {output}", flush=True)
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
    parser.add_argument("--tcg", action="store_true", help="Use software emulation even when /dev/kvm is accessible")
    parser.add_argument("--content-shield", action="store_true", help="Verify the opt-in native Shield probe and capture browser screenshots")
    parser.add_argument("--content-shield-only", action="store_true", help="Run only the Content Shield native fixture (requires --content-shield)")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--evidence", default="target/network-tls")
    parser.add_argument("--account-provider-preflight", action="store_true",
                        help="Assert missing-registration failure in native-provider-test image (not live success)")
    parser.add_argument("--account-broker-flow", action="store_true",
                        help="Exercise native-mock image against local OAuth/TLS provider on port 8443")
    parser.add_argument("--account-broker-probe", action="store_true",
                        help="Also verify native broker launch and fail-closed IPC (not browser authorization)")
    parser.add_argument("--account-broker-storage", action="store_true", help="Verify 1K/4K KV records and deletion across live daemon restarts")
    parser.add_argument("--online-accounts-ui", action="store_true", help="Click through Control Panel, Microsoft mock sign-in and Disconnect; capture native screenshots")
    parser.add_argument("--ui-only", action="store_true", help="Run only CA setup and interactive UI (use after the full network/storage regression gate)")
    parser.add_argument("--arch-resolve", help="Also fetch Arch directly using this IPv4 DNS override")
    parser.add_argument("--arch-proxy", help="Also fetch archlinux.org through this guest-visible SOCKS IPv4:port")
    arguments = parser.parse_args()
    if arguments.content_shield_only and not arguments.content_shield:
        parser.error("--content-shield-only requires --content-shield")
    if arguments.ui_only and not arguments.online_accounts_ui:
        parser.error("--ui-only requires --online-accounts-ui")
    if arguments.online_accounts_ui and not arguments.account_broker_flow:
        parser.error("--online-accounts-ui requires --account-broker-flow and a native-mock image")
    with tempfile.TemporaryDirectory(prefix="sunlight-tls-") as directory:
        gate(arguments, Path(directory))
