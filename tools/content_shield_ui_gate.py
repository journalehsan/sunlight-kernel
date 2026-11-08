"""Hardware-input and native-scanout Shield fixture driver, no injected page JS."""
from collections import Counter
import csv
import io
from pathlib import Path
import subprocess
import time
from PIL import Image


def verify_ui(request, type_text, wait_for, serial, output, http_url, requests):
    def key(name):
        request("human-monitor-command", {"command-line": f"sendkey {name} 80"})
        time.sleep(.4)

    def capture(name):
        path = output / (name + ".ppm")
        request("screendump", {"filename": str(path)})
        image = path.with_suffix(".png")
        Image.open(path).save(image)
        return image

    def words(path, panel=False):
        source = path
        if panel:
            left, top = settings_origin
            source = path.with_name(path.stem + "-ocr.png")
            Image.open(path).crop((left, top, left + 410, top + 248)).resize((1230, 744)).save(source)
        text = subprocess.check_output(["tesseract", str(source), "stdout", "--psm", "6" if panel else "11", "tsv"], stderr=subprocess.DEVNULL, text=True)
        rows = list(csv.DictReader(io.StringIO(text), delimiter='\t'))
        if panel:
            for row in rows:
                row['left'] = str(left + int(row['left']) // 3)
                row['top'] = str(top + int(row['top']) // 3)
                row['width'] = str(int(row['width']) // 3)
                row['height'] = str(int(row['height']) // 3)
        return rows

    cursor_templates = []
    cursor_root = Path(__file__).resolve().parents[1] / "assets/cursors"
    for name, hx, hy in [("pointer", 3, 2), ("hand", 14, 3), ("text", 16, 15),
                         ("resize-h", 16, 16), ("resize-v", 16, 16),
                         ("resize-nwse", 16, 16), ("resize-nesw", 16, 16),
                         ("move", 16, 16), ("wait", 16, 16), ("question", 2, 2)]:
        sprite = Image.open(cursor_root / (name + ".tga")).convert("RGBA")
        opaque = [(x, y, pixel[:3]) for y in range(32) for x in range(32)
                  if (pixel := sprite.getpixel((x, y)))[3] == 255]
        frequencies = Counter(rgb for _, _, rgb in opaque)
        anchors = sorted(opaque, key=lambda p: (p[2] in [(255, 255, 255), (0, 0, 0)], frequencies[p[2]]))[:2]
        cursor_templates.append((hx, hy, opaque, anchors))

    def pointer_position():
        # Inspect the real compositor cursor. Feedback avoids assumptions about
        # PS/2 direction, acceleration, clipping, and coalesced motion packets.
        path = output / "shield-pointer.ppm"
        request("screendump", {"filename": str(path)})
        frame = Image.open(path).convert("RGB")
        raw = frame.tobytes()
        width, height = frame.size
        for hx, hy, opaque, anchors in cursor_templates:
            for ax, ay, color in anchors:
                start = 0
                while (found := raw.find(bytes(color), start)) >= 0:
                    start = found + 3
                    if found % 3:
                        continue
                    pixel = found // 3
                    left, top = pixel % width - ax, pixel // width - ay
                    if not (-31 <= left < width and -31 <= top < height):
                        continue
                    visible = [(x, y, c) for x, y, c in opaque
                               if 0 <= left + x < width and 0 <= top + y < height]
                    if len(visible) >= 4 and all(frame.getpixel((left + x, top + y)) == c for x, y, c in visible):
                        return left + hx, top + hy
        return None

    def click_at(x, y):
        for _ in range(100):
            current = pointer_position()
            if current is None:
                raise RuntimeError("Native compositor cursor not visible")
            dx, dy = x - current[0], y - current[1]
            if abs(dx) <= 3 and abs(dy) <= 3:
                break
            def motion(delta):
                if abs(delta) <= 3:
                    return 0
                sign = 1 if delta > 0 else -1
                return sign * (100 if abs(delta) > 24 else max(1, abs(delta) // 2))
            request("human-monitor-command", {"command-line": f"mouse_move {motion(dx)} {motion(dy)}"})
            time.sleep(.2)
        else:
            raise RuntimeError(f"Pointer did not reach native control at {x},{y}")
        request("human-monitor-command", {"command-line": "mouse_button 1"})
        time.sleep(.3)
        request("human-monitor-command", {"command-line": "mouse_button 0"})
        time.sleep(2)

    def panel_state(name):
        path = capture(name)
        rows = words(path, panel=True)
        return path, rows, any(r['text'].lower() == 'enable' for r in rows)

    def open_settings():
        # A first click may only focus a newly launched native window. Observe
        # the panel before retrying, so an already open panel is not toggled shut.
        for _ in range(3):
            click_at(*shield_button)
            path, rows, opened = panel_state("shield-settings")
            if opened:
                return path, rows
        raise RuntimeError(f"Shield checkbox not visible; see {path}")

    def checked(path):
        left, top = settings_origin
        frame = Image.open(path).convert("RGB")
        return any(frame.getpixel((left + x, top + y)) == (255, 165, 0)
                   for x in range(17, 26) for y in range(43, 52))

    def set_enabled(wanted):
        for _ in range(3):
            path, rows = open_settings()
            if checked(path) != wanted:
                row = next(r for r in rows if r['text'].lower() == 'enable')
                click_at(int(row['left']) + int(row['width'])//2, int(row['top']) + int(row['height'])//2)
                capture("shield-settings-changed")
            for _ in range(3):
                # Dismissing outside the panel is idempotent even if a focus
                # click precedes the widget click in the native event stream.
                click_at(*dismiss_point)
                _, _, opened = panel_state("shield-settings-closed")
                if not opened:
                    break
            else:
                raise RuntimeError("Native settings panel did not close")
            path = capture("shield-policy-applied")
            visible = 'Advertisement' in ' '.join(r['text'] for r in words(path))
            if visible == (not wanted):
                return
        raise RuntimeError("Native checkbox state did not reach its requested value")

    def reload_generation(generation):
        marker = f"[RABBIT][PRESENT] gen={generation} milestone=page-ready"
        for _ in range(3):
            click_at(*reload_button)
            if f"[RABBIT][DOM] gen={generation} stage=before fetch" in serial.read_text(errors='replace'):
                wait_for(marker)
                return
        raise RuntimeError("Native reload button did not start navigation")

    # Native windows need the graphical desktop; the network harness initially
    # logs in to the fullscreen TTY. Follow the existing Online Accounts gate's
    # desktop login and launch the probe through the native Run dialog.
    type_text("exit\n")
    time.sleep(2)
    type_text("\nroot")
    key("tab")
    key("spc")
    key("ret")
    wait_for("[VORTEX] desktop layer registered, entering event loop")
    time.sleep(6)
    key("ctrl-spc")
    wait_for("Run phase=window_visible_on_screen")
    time.sleep(3)
    # GUI text rendering can outlast the TTY harness's 130 ms key cadence in
    # software emulation. Allow each real hardware key to be processed.
    command = f"rappid-rabbit --shield-probe {http_url}/shield\n"
    codes = {" ": "spc", "/": "slash", ":": "shift-semicolon", ".": "dot", "-": "minus", "\n": "ret"}
    for ch in command:
        key(codes.get(ch, ch))
        time.sleep(.8)
    capture("shield-launcher-after-command")
    wait_for("[SHIELD-NATIVE] PASS")
    wait_for("[SHIELD-NATIVE] preferences roundtrip PASS")
    wait_for("[SHIELD-NATIVE] fixture window ready")
    time.sleep(3)
    enabled = capture("shield-enabled")
    enabled_words = words(enabled)
    # Locate the painted title of the fixed 1080x720 fixture window. Its
    # client origin determines the visible tab-strip and reload button points.
    caption = next(r for r in enabled_words if r['text'] == 'Content' and int(r['top']) < 120)
    settings_origin = (int(caption['left']) + 645, int(caption['top']) + 88)
    shield_button = (int(caption['left']) + 1043, int(caption['top']) + 34)
    reload_button = (int(caption['left']) + 976, int(caption['top']) + 64)
    dismiss_point = (int(caption['left']) + 100, int(caption['top']) + 600)
    enabled_text = ' '.join(r['text'] for r in enabled_words)
    assert "Normal" in enabled_text and "Advertisement" not in enabled_text, enabled_text
    assert "/ads/banners/sale.png" not in requests and "/css/ads-native.css" not in requests
    set_enabled(False)
    disabled = capture("shield-disabled")
    disabled_text = ' '.join(r['text'] for r in words(disabled))
    assert "Advertisement" in disabled_text, disabled_text
    reload_generation(2)
    assert requests.count("/ads/banners/sale.png") == 1 and requests.count("/css/ads-native.css") == 1
    capture("shield-disabled-after-reload")
    set_enabled(True)
    reload_generation(3)
    assert requests.count("/ads/banners/sale.png") == 1 and requests.count("/css/ads-native.css") == 1
    time.sleep(2)
    restored = capture("shield-restored")
    restored_text = ' '.join(r['text'] for r in words(restored))
    assert "Normal" in restored_text and "Advertisement" not in restored_text, restored_text
    # Preferences are written only through browser chrome's authenticated KV path.
    settings = capture("shield-after-toggle")
    open_settings()
    settings = capture("shield-preferences")
    rendered = ' '.join(r['text'] for r in words(settings, panel=True))
    assert "saved" in rendered.lower(), rendered
    key("alt-f4")
    print(f"PASS: native screenshots {enabled}, {disabled}, {restored}", flush=True)
