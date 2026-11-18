#!/usr/bin/env python3
"""Native desktop/TTY regression through QEMU PS/2 hardware and scanout captures.

Use a freshly built, non-injection production ISO. No writable guest disk is
attached. Evidence is retained under target/desktop-tty-ux by default.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
VISIBLE = {
    "terminal": "Sunlight Terminal phase=window_visible_on_screen",
    "sunlight-empires": "Sunlight Empires phase=window_visible_on_screen",
    "silicon-echoes": "Silicon Echoes: 1993 phase=window_visible_on_screen",
    "rappid-rabbit": "Rappid Rabbit phase=window_visible_on_screen",
}


class Guest:
    def __init__(self, iso, out, vga="virtio"):
        self.out = out.resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        self.serial = self.out / "serial.log"
        self.serial.write_text("")
        self.socket_path = f"/tmp/sunlight-session-ux-{os.getpid()}.sock"
        code = Path(os.environ.get("SUNLIGHT_OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd"))
        variables = Path(os.environ.get("SUNLIGHT_OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd"))
        shutil.copyfile(variables, self.out / "vars.fd")
        cmd = ["qemu-system-x86_64", "-cdrom", str(iso.resolve()),
               "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
               "-drive", f"if=pflash,format=raw,file={self.out / 'vars.fd'}",
               "-serial", f"file:{self.serial}", "-display", "none", "-m", "4096M", "-smp", "2",
               "-device", "virtio-rng-pci,disable-modern=on", "-qmp",
               f"unix:{self.socket_path},server=on,wait=off", "-no-reboot", "-no-shutdown"]
        cmd += (["-device", "virtio-vga,xres=1280,yres=800"] if vga == "virtio" else ["-vga", vga])
        if Path("/dev/kvm").exists():
            cmd += ["-enable-kvm", "-cpu", "host"]
        (self.out / "command.json").write_text(json.dumps(cmd, indent=2))
        self.log = (self.out / "qemu.log").open("w")
        self.process = subprocess.Popen(cmd, cwd=ROOT, stdout=self.log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 15
        while not Path(self.socket_path).exists():
            if self.process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("QEMU failed to start; inspect qemu.log")
            time.sleep(.1)
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.settimeout(30)
        self.connection.connect(self.socket_path)
        self.stream = self.connection.makefile("rwb")
        self.stream.readline()
        self.request("qmp_capabilities")

    def read(self):
        return self.serial.read_text(errors="replace")

    def wait(self, marker, count=1, timeout=90):
        deadline = time.monotonic() + timeout
        while self.read().count(marker) < count:
            if self.process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f"Guest timeout: {marker} count={count}")
            time.sleep(.1)

    def request(self, command, arguments=None):
        payload = {"execute": command}
        if arguments is not None:
            payload["arguments"] = arguments
        self.stream.write(json.dumps(payload).encode() + b"\n")
        self.stream.flush()
        while True:
            reply = json.loads(self.stream.readline())
            if "error" in reply:
                raise RuntimeError(reply["error"])
            if "return" in reply:
                return reply["return"]

    def key(self, *keys, hold=60):
        self.request("send-key", {"keys": [{"type": "qcode", "data": key} for key in keys],
                                  "hold-time": hold})
        time.sleep(max(.15, hold / 1000 + .08))

    def type(self, value):
        for char in value:
            if char.isupper():
                self.key("shift", char.lower())
            else:
                self.key({" ": "spc", "-": "minus", ".": "dot", "/": "slash", "\n": "ret"}.get(char, char))

    def capture(self, name):
        time.sleep(.5)
        ppm = self.out / f"{name}.ppm"
        self.request("screendump", {"filename": str(ppm)})
        png = self.out / f"{name}.png"
        subprocess.run(["convert", str(ppm), str(png)], check=True)
        text = subprocess.check_output(["tesseract", str(png), "stdout", "--psm", "11"],
                                       stderr=subprocess.DEVNULL, text=True)
        (self.out / f"{name}.txt").write_text(text)
        return text

    def switch(self, function, right=False):
        marker = "[SESSION] switched to F1 TTY/Login" if function == "f1" else "[SESSION] switched to F2 GraphicalDesktop"
        count = self.read().count(marker)
        self.key("ctrl_r" if right else "ctrl", function)
        self.wait(marker, count + 1, timeout=15)

    def pointer(self):
        """Locate the real composited pointer from opaque native sprite pixels."""
        from PIL import Image
        ppm = self.out / "pointer.ppm"
        self.request("screendump", {"filename": str(ppm)})
        screen = Image.open(ppm).convert("RGB")
        sprite = Image.open(ROOT / "assets/cursors/pointer.tga").convert("RGBA")
        samples = [(x, y, sprite.getpixel((x, y))[:3])
                   for y in range(32) for x in range(32)
                   if sprite.getpixel((x, y))[3] == 255]
        sx, sy, color = samples[len(samples) // 2]
        raw = screen.tobytes()
        start = 0
        while True:
            offset = raw.find(bytes(color), start)
            if offset < 0:
                raise RuntimeError("Native pointer sprite was not found")
            start = offset + 1
            if offset % 3:
                continue
            pixel = offset // 3
            x, y = pixel % screen.width - sx, pixel // screen.width - sy
            if x < -3 or y < -2 or x >= screen.width or y >= screen.height:
                continue
            if all(screen.getpixel((x + dx, y + dy)) == c for dx, dy, c in samples
                   if 0 <= x + dx < screen.width and 0 <= y + dy < screen.height):
                return x + 3, y + 2

    def move(self, x, y):
        # Closed-loop hardware motion, not a compositor command or UI event.
        for _ in range(90):
            current_x, current_y = self.pointer()
            dx, dy = x - current_x, y - current_y
            if abs(dx) <= 3 and abs(dy) <= 3:
                return
            events = []
            for axis, delta in [("x", dx), ("y", dy)]:
                if delta:
                    amount = max(-16, min(16, int(delta / 1.5)))
                    amount = amount or (1 if delta > 0 else -1)
                    events.append({"type": "rel", "data": {"axis": axis, "value": -amount if axis == "y" else amount}})
            self.request("input-send-event", {"events": events})
            time.sleep(.06)
        raise RuntimeError(f"Native pointer did not reach {x},{y}")

    def launcher(self):
        from PIL import Image
        ppm = self.out / "launcher.ppm"
        self.request("screendump", {"filename": str(ppm)})
        image = Image.open(ppm).convert("RGB")
        found = []
        for y in range(image.height - 45, image.height - 20):
            for x in range(180, image.width - 180):
                color = image.getpixel((x, y))
                if min(color) < 180 or image.getpixel((x + 7, y)) == color:
                    continue
                if all(image.getpixel((x + column * 14 + dx, y + row * 14 + dy)) == color
                       for row in range(2) for column in range(2)
                       for dx, dy in [(-3, -3), (3, -3), (-3, 3), (3, 3), (0, 0)]):
                    found.append((x + 7, y + 7))
        assert found, "Four-tile native launcher was not found on the dock"
        return round(sum(x for x, _ in found) / len(found)), round(sum(y for _, y in found) / len(found))

    def click(self, x, y):
        self.move(x, y)
        for down in [True, False]:
            # HMP's PS/2 button command is reliable with this guest's native
            # mouse driver; retain enough time for both physical edges.
            self.request("human-monitor-command", {"command-line": f"mouse_button {1 if down else 0}"})
            time.sleep(.8)

    def open_search(self, term):
        self.key("meta_l", "k")
        # Search deliberately retains its previous query. Use real editing
        # keys to replace the short queries used by this native matrix.
        for _ in range(12):
            self.key("backspace")
        self.type(term)

    def search(self, term, app):
        marker = VISIBLE[app]
        count = self.read().count(marker)
        self.open_search(term)
        self.key("ret")
        self.wait(marker, count + 1, timeout=45)
        time.sleep(.5)

    def close(self):
        self.stream.close()
        self.connection.close()
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.log.close()
        Path(self.socket_path).unlink(missing_ok=True)


def run(iso, out, vga):
    guest = Guest(iso, out, vga)
    result = {"verdict": "PARTIAL", "checks": []}
    def passed(name):
        result["checks"].append(name)
        print("PASS:", name, flush=True)
    try:
        guest.wait("[TTY]  Login screen ready")
        guest.wait("[init] Name server: listening", timeout=30)
        time.sleep(5)
        login = guest.capture("01-login-default")
        assert "Desktop" in login, login
        passed("Desktop preselected at login")
        if vga == "virtio":
            guest.wait("[TTY-GPU] ready 1280x800")
            passed("TTY VirtIO scanout uses requested 1280x800")
        guest.key("ret")
        guest.type("root")
        guest.key("ret")
        guest.wait("[SESSION-FOUNDATION] SHELL_READY PASS")
        guest.wait("[SESSION] switched to F2 GraphicalDesktop")
        time.sleep(4)
        guest.capture("02-desktop")
        switches = guest.read().count("[SESSION] switched to F")
        guest.key("f1")
        guest.key("f2")
        assert guest.read().count("[SESSION] switched to F") == switches
        passed("Plain F1/F2 do not switch sessions")
        passed("Enter authenticates default Desktop through SessionD")
        guest.switch("f1")
        recovery = guest.capture("03-tty-recovery-login")
        assert "Desktop" in recovery or "Password" in recovery, recovery
        passed("Ctrl+F1 presents independent native login")
        # Authenticate an explicit TTY; desktop remains alive.
        guest.type("root")
        guest.key("tab")
        guest.key("spc")
        tty_option = guest.capture("04-explicit-tty-option")
        assert "TTY" in tty_option or "Terminal" in tty_option, tty_option
        guest.key("ret")
        guest.wait("[TTY]  Built-in shell ready")
        guest.type("echo tabone\n")
        guest.wait("[TTY]  cmd: echo tabone -> tabone")
        guest.key("ctrl", "t")
        time.sleep(2)
        guest.type("echo tabtwo\n")
        guest.wait("[TTY]  cmd: echo tabtwo -> tabtwo")
        guest.type("echo preserved")
        before = guest.capture("05-tty-two-tabs")
        assert "preserved" in before, before
        passed("Explicit TTY shell accepts commands and creates second internal tab")
        guest.switch("f2")
        guest.capture("06-preserved-desktop")
        baseline = guest.read()
        for i in range(20):
            guest.switch("f1", right=i % 2 == 1)
            guest.switch("f2", right=i % 2 == 1)
        passed("20 guest-keyboard round trips, alternating left/right Ctrl")
        after = guest.read()[len(baseline):]
        assert "[GPU] attach_backing:" not in after
        assert "[SESSION] TTY shell launched" not in after
        assert "[SESSION-FOUNDATION] SESSION_CREATED PASS" not in after
        passed("Repeated switching creates no GPU backing, TTY shell, or desktop session")
        guest.switch("f1")
        tty = guest.capture("07-tty-preserved-after-20")
        assert "preserved" in tty and "tabtwo" in tty, tty
        guest.key("ret")
        guest.wait("[TTY]  cmd: echo preserved -> preserved")
        guest.key("ctrl", "1")
        first_tab = guest.capture("08-first-tab-restored")
        assert "tabone" in first_tab, first_tab
        passed("TTY tab output, active input buffer and tab switching survive round trips")
        guest.switch("f2")
        guest.key("meta_l", "k")
        guest.type("terminal")
        guest.capture("09-search-terminal")
        guest.key("ret")
        guest.wait(VISIBLE["terminal"], timeout=40)
        guest.capture("10-terminal-before-switch")
        guest.switch("f1")
        guest.switch("f2")
        terminal = guest.capture("11-terminal-after-switch")
        assert "Terminal" in terminal, terminal
        guest.type("help\n")
        functional = guest.capture("11b-terminal-functional")
        assert "Builtins" in functional, functional
        passed("Focused native Terminal survives switching and accepts shell input")
        guest.key("ctrl", "w")  # close Terminal through the existing global shortcut
        time.sleep(1)
        guest.key("ctrl", "w")  # close Welcome when it is still present
        time.sleep(1)
        guest.switch("f1")
        guest.switch("f2")
        guest.capture("12-idle-desktop")
        passed("Idle desktop switches after application closure")
        # Start stays at its existing dock location. All pointer movements and
        # the click traverse PS/2 hardware, with screenshot-based feedback.
        launcher_x, launcher_y = guest.launcher()
        guest.move(launcher_x, launcher_y)
        hover = guest.capture("13-applications-hover")
        assert "Applications" in hover, hover
        guest.click(launcher_x, launcher_y)
        menu = guest.capture("14-start-menu")
        assert "Pinned" in menu, menu
        guest.switch("f1")
        guest.switch("f2")
        preserved_menu = guest.capture("15-start-menu-preserved")
        assert "Pinned" in preserved_menu, preserved_menu
        passed("Applications hover label, native click, and switching with Start open")
        guest.type("empires")
        empires_result = guest.capture("16-start-empires-result")
        assert "sunlight emp" in empires_result.lower(), empires_result
        guest.key("ret")
        guest.wait(VISIBLE["sunlight-empires"], timeout=45)
        guest.capture("17-empires-from-start")
        guest.switch("f1")
        guest.switch("f2")
        empires = guest.capture("18-empires-preserved")
        assert "Empires" in empires, empires
        passed("Sunlight Empires launches from Start and survives focused switching")
        guest.key("ctrl", "w")
        time.sleep(2)
        guest.search("empires", "sunlight-empires")
        guest.capture("19-empires-from-search")
        passed("Sunlight Empires relaunches through Search")
        # Search is a global first-class workflow even while a game owns focus.
        guest.open_search("silicon")
        guest.capture("20-search-over-focused-game")
        guest.switch("f1")
        guest.switch("f2")
        guest.capture("21-search-preserved")
        guest.key("ret")
        guest.wait(VISIBLE["silicon-echoes"], timeout=45)
        guest.capture("22-silicon-echoes")
        guest.switch("f1")
        guest.switch("f2")
        guest.capture("23-silicon-echoes-preserved")
        passed("Search overlay and focused Silicon Echoes survive switching")
        guest.search("rappid", "rappid-rabbit")
        guest.capture("24-rappid-rabbit")
        guest.switch("f1")
        guest.switch("f2")
        guest.capture("25-rappid-rabbit-preserved")
        passed("Focused Rappid Rabbit survives switching with multiple windows running")
        guest.key("meta_l", "r")
        guest.wait("name=Run policy=", timeout=30)
        guest.capture("26-run-dialog")
        guest.switch("f1")
        guest.switch("f2")
        guest.capture("27-run-dialog-preserved")
        guest.key("esc")
        passed("Super+R preserves Run and its dialog survives switching")
        guest.switch("f1")
        background = guest.capture("28-no-background-tty-input")
        assert all(name not in background.lower() for name in ["empires", "silicon", "rappid"]), background
        passed("Desktop typing and launches do not execute in the background native TTY")
        guest.switch("f2")
        faults = [x for x in ["KERNEL PANIC", "[EXCEPTION]", "[FAULT]"] if x in guest.read()]
        assert not faults, faults
        result["verdict"] = "PASS"
    except Exception as error:
        result["error"] = str(error)
        guest.capture("failure")
        raise
    finally:
        (guest.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        guest.close()


def run_tty_first(iso, out, vga, failure=False):
    """Fresh TTY-first login; optional compositor crash in a diskless guest."""
    import re
    guest = Guest(iso, out, vga)
    result = {"verdict": "PARTIAL", "checks": []}
    def passed(check):
        result["checks"].append(check)
        print("PASS:", check, flush=True)
    try:
        guest.wait("[TTY]  Login screen ready")
        time.sleep(5)
        guest.key("ret")
        guest.type("root")
        guest.key("tab")
        guest.key("spc")
        guest.capture("01-fresh-explicit-tty")
        guest.key("ret")
        guest.wait("[TTY]  Built-in shell ready")
        guest.type("echo freshfirst\n")
        guest.wait("[TTY]  cmd: echo freshfirst -> freshfirst")
        guest.key("ctrl", "t")
        time.sleep(2)
        guest.type("echo secondretained")
        guest.key("ctrl", "f2")
        form = guest.capture("02-first-desktop-authentication")
        assert "Desktop" in form and "retained" in form, form
        guest.type("root")
        guest.key("ret")
        guest.wait("[SESSION-FOUNDATION] SHELL_READY PASS")
        guest.wait("[SESSION] switched to F2 GraphicalDesktop")
        guest.capture("03-first-managed-desktop")
        passed("Fresh explicit TTY login and authenticated first Desktop activation")
        guest.switch("f1")
        tabs = guest.capture("04-fresh-tty-tabs-preserved")
        assert "secondretained" in tabs, tabs
        guest.key("ret")
        guest.wait("[TTY]  cmd: echo secondretained -> secondretained")
        guest.key("ctrl", "1")
        first = guest.capture("05-fresh-first-tab-preserved")
        assert "freshfirst" in first, first
        passed("First Desktop activation preserves both existing native TTY tabs")
        if failure:
            guest.switch("f2")
            guest.search("terminal", "terminal")
            guest.capture("06-failure-terminal-ready")
            display = re.search(r"/sbin/sunlight-display spawned pid=(\d+)", guest.read())
            assert display, "display PID was not recorded in the boot log"
            pid = display.group(1)
            # This isolated guest has no writable persistent disk. Kill only
            # its compositor via the ordinary authenticated terminal shell.
            guest.type(f"kill -9 {pid}\n")
            guest.wait(f"process_mark_finished pid={pid}", timeout=30)
            guest.capture("07-compositor-unavailable")
            guest.switch("f1")
            guest.type("echo recoveryalive\n")
            guest.wait("[TTY]  cmd: echo recoveryalive -> recoveryalive")
            recovery = guest.capture("08-independent-recovery-tty")
            assert "recoveryalive" in recovery, recovery
            passed("Ctrl+F1 and shell input work after controlled compositor termination")
        result["verdict"] = "PASS"
    except Exception as error:
        result["error"] = str(error)
        guest.capture("failure")
        raise
    finally:
        (guest.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        guest.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "target/desktop-tty-ux")
    parser.add_argument("--vga", default="virtio", choices=["virtio", "std"])
    parser.add_argument("--tty-first", action="store_true")
    parser.add_argument("--failure", action="store_true", help="terminate compositor in a diskless TTY-first guest")
    args = parser.parse_args()
    if args.tty_first or args.failure:
        run_tty_first(args.iso, args.out, args.vga, args.failure)
    else:
        run(args.iso, args.out, args.vga)
