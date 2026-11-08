#!/usr/bin/env python3
"""Offline Rabbit fixtures with deterministic delayed image responses.

Run from any directory; no third-party packages, external assets or DNS.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--image-delay-ms", type=int, default=150)
    args = parser.parse_args()
    if not 0 <= args.image_delay_ms <= 60_000:
        parser.error("image delay must be between 0 and 60000 ms")

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            name = urlsplit(self.path).path.lstrip("/")
            if name in ("text.html", "large.html", "archwiki.html", "failure.html"):
                body = (ROOT / name).read_bytes()
                content_type = "text/html; charset=utf-8"
            elif name in ("logo.jpg", "screen.jpg", "footer.jpg", "portrait.jpg", "bad.jpg"):
                time.sleep(args.image_delay_ms / 1000)
                if name == "bad.jpg":
                    body = b"deliberate invalid JPEG"
                elif name == "portrait.jpg":
                    body = (ROOT.parent / "portrait-size.jpg").read_bytes()
                else:
                    body = (REPO / "sun-img/tests/fixtures/gray.jpg").read_bytes()
                content_type = "image/jpeg"
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass  # a rapid navigation deliberately cancels the reader

    with ThreadingHTTPServer((args.bind, args.port), Handler) as server:
        print(f"Rabbit offline fixtures: http://{args.bind}:{args.port}/archwiki.html", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
