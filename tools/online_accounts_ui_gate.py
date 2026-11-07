"""Native QEMU UI driver used by network_tls_gate.py; hardware input and scanout only."""
import csv
import io
from pathlib import Path
import subprocess
import time
from PIL import Image


def verify_ui(request, type_text, wait_for, serial, output):
    def key(name):
        request("human-monitor-command", {"command-line": "sendkey " + name + " 80"})
        time.sleep(.4)

    def capture(name):
        path = output / (name + ".ppm")
        request("screendump", {"filename": str(path)})
        subprocess.run(["convert", str(path), str(path.with_suffix('.png'))], check=True)
        return path.with_suffix('.png')

    def words(path):
        data = subprocess.check_output(["tesseract", str(path), "stdout", "--psm", "11", "tsv"], stderr=subprocess.DEVNULL, text=True)
        return list(csv.DictReader(io.StringIO(data), delimiter='\t'))

    def click_at(x, y):
        # Clamp to origin, then use the compositor's low-speed zone. Its
        # default gain is 9/8, with acceleration starting above magnitude 2.
        # Large jumps are capped to 24 pixels and cannot position precisely.
        for _ in range(80):
            request("human-monitor-command", {"command-line": "mouse_move -100 100"})
            time.sleep(.025)
        for axis, pixels in [("x", x), ("y", y)]:
            remaining = round(pixels * 8 / 9)
            while remaining:
                step = min(2, remaining)
                dx, dy = (step, 0) if axis == "x" else (0, -step)
                request("human-monitor-command", {"command-line": f"mouse_move {dx} {dy}"})
                remaining -= step
                time.sleep(.025)
        time.sleep(.3)
        request("human-monitor-command", {"command-line": "mouse_button 1"})
        time.sleep(.15)
        request("human-monitor-command", {"command-line": "mouse_button 0"})
        time.sleep(1)

    def click(word, name, timeout=30, last=False):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            path = capture(name)
            rows = words(path)
            for row in reversed(rows) if last else rows:
                if word.lower() in row['text'].lower():
                    click_at(int(row['left']) + int(row['width'])//2, int(row['top']) + int(row['height'])//2)
                    return
            time.sleep(1)
        raise RuntimeError(f"Cannot find UI text {word!r}; see {path}")

    def click_button(width, name):
        path = capture(name)
        frame = Image.open(path).convert("RGB")
        pixels = frame.load()
        spans = {}
        # Orange native buttons need their own region detection: full-screen
        # sparse-text OCR often omits black text on the accent background.
        for y in range(90, frame.height):
            x = 0
            while x < frame.width:
                if pixels[x, y] != (255, 165, 0):
                    x += 1
                    continue
                start = x
                while x < frame.width and pixels[x, y] == (255, 165, 0):
                    x += 1
                if x - start == width:
                    spans.setdefault((start, x), []).append(y)
        for (left, right), ys in spans.items():
            if 24 <= max(ys) - min(ys) + 1 <= 48:
                click_at((left + right)//2, (min(ys) + max(ys))//2)
                return
        raise RuntimeError(f"Native button width {width} not visible; see {path}")

    type_text("exit\n")
    time.sleep(2)
    # Select root, enter the repository's fixture password, choose Desktop.
    type_text("\nroot")
    key("tab")
    key("spc")
    key("ret")
    wait_for("[VORTEX] desktop layer registered, entering event loop")
    time.sleep(4)
    key("ctrl-spc")
    type_text("control-panel\n")
    click("Online", "01-control-panel-navigation", 60)
    time.sleep(2)
    capture("02-online-accounts-empty")
    click_button(140, "03-add-account")
    click_button(240, "04-provider-picker")
    wait_for("[ONLINE-ACCOUNTS-UI] authorization started")
    click("Authorize", "05-browser-authorization", 60)
    wait_for("[ONLINE-ACCOUNTS] session record committed; metadata ready")
    wait_for("[ONLINE-ACCOUNTS-UI] authorization result; list refreshed")
    time.sleep(3)
    connected = capture("06-online-accounts-connected")
    rendered = ' '.join(row['text'] for row in words(connected))
    assert "mock@sunlight.test" in rendered, f"Account identifier not rendered: {rendered}"
    assert "Connected" in rendered, f"Connected status not rendered: {rendered}"
    click_button(108, "07-disconnect")
    wait_for("[ONLINE-ACCOUNTS] disconnected; record and index removed")
    time.sleep(2)
    empty = capture("08-online-accounts-disconnected")
    rendered = ' '.join(row['text'] for row in words(empty))
    assert "mock@sunlight.test" not in rendered, "Disconnected account still rendered"
    assert "No online accounts" in rendered, f"Empty state not rendered: {rendered}"
    print("Online Accounts native interactive UI gate PASS", flush=True)
