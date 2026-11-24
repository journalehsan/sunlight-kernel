#!/usr/bin/env python3
"""Capture and verify the Helios fixture on the guest's actual scanout."""
import json
from pathlib import Path
import re
import sys
import time

from PIL import Image

from desktop_tty_ux_gate import Guest


def capture(guest, name):
    ppm = guest.out / f"{name}.ppm"
    guest.request("screendump", {"filename": str(ppm)})
    with Image.open(ppm) as source:
        image = source.convert("RGB")
    image.save(guest.out / f"{name}.png")
    return image


def pattern(image, frame, canvas=False):
    if canvas:
        background, accent = (
            ((24, 45, 75), (40, 220, 255)) if frame == 1
            else ((68, 35, 24), (255, 203, 50))
        )
        bar_end = 96 if frame == 1 else 216
        dark = (16, 20, 28)
        pixels = image.load()
        for top in range(image.height - 159):
            for left in range(image.width - 255):
                if (pixels[left + 8, top + 8] == background
                        and pixels[left + 23, top + 32] == background
                        and pixels[left + 24, top + 32] == accent
                        and all(pixels[left + x, top + 34] == accent for x in range(24, 232))
                        and all(pixels[left + x, top + 116] == (
                            accent if x < bar_end else dark) for x in range(24, 232))
                        and any(pixels[left + x, top + y] == (255, 255, 255)
                                for y in range(48, 64) for x in range(24, 232))):
                    return left, top
        return None
    colors = [(0x28, 0xdc, 0xff), (0x12, 0x5d, 0xd4)] if frame == 1 else [
        (0xff, 0xcb, 0x32), (0xd4, 0x24, 0x09)
    ]
    pixels = image.load()
    for top in range(image.height - 159):
        for left in range(image.width - 255):
            if pixels[left + 8, top + 8] != colors[0]:
                continue
            if pixels[left + 120, top + 80] != (255, 255, 255):
                continue
            if all(pixels[left + x, top + y] == (
                (255, 255, 255) if 104 <= x < 152 and 60 <= y < 100
                else colors[((x if frame == 1 else y) // 16) % 2]
            ) for y in range(8, 152, 4) for x in range(8, 248, 4)):
                return (left, top)
    return None


def input_pattern(image, frame):
    pixels = image.load()
    background, accent = (24, 45, 75), (40, 220, 255)
    button = accent if frame == 1 else (255, 203, 50)
    for top in range(image.height - 159):
        for left in range(image.width - 255):
            if (pixels[left + 8, top + 8] == background
                    and pixels[left + 23, top + 32] == background
                    and pixels[left + 24, top + 32] == accent
                    and all(pixels[left + x, top + 120] == button for x in range(24, 128))
                    and any(pixels[left + x, top + y] == button
                            for x in range(160, 216) for y in range(104, 120))):
                return left, top
    return None


def input_cleanup(guest):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        serial = guest.read()
        redraw = re.search(r"\[WL-SGP\] redraw window=(\d+)", serial)
        assert redraw, "missing fixture window"
        window = redraw[1]
        if f"[WL-SGP] removed window={window}" in serial:
            return "explicit-close"
        owner = re.search(rf"\[APP_LIFECYCLE\] app_window_attached pid=(\d+) win_id={window}\b", serial)
        if owner:
            reap = f"[DISPLAY] display_windows_reaped_for_process pid={owner[1]} count=1"
            if reap in serial.split("[WL-FIXTURE] complete after disconnect", 1)[1]:
                return "disconnect-owner-reap"
        time.sleep(0.1)
    raise AssertionError("scanout cleanup: no removal for the fixture window")


def run(iso, out, accel, mode):
    interactive = mode == "canvas-sunapp-input"
    sunapp = mode == "canvas-sunapp" or interactive
    canvas = mode == "canvas" or sunapp
    libwayland = mode == "libwayland" or canvas
    locate = input_pattern if interactive else lambda image, frame: pattern(image, frame, canvas)
    (out / "result.json").unlink(missing_ok=True)
    guest = Guest(iso, out, vga="std", accel=accel)
    try:
        guest.wait("[TTY]  Login screen ready")
        guest.wait("[init] Name server: listening")
        guest.key("ret")
        guest.type("root")
        guest.key("ret")
        guest.wait("[SESSION] switched to F2 GraphicalDesktop")
        if libwayland:
            guest.wait("[WL-LIB] entered main", timeout=30)
            guest.wait("[WL-LIB] discovered compositor, shm and xdg-shell", timeout=15)
        locations = []
        for frame in (1, 2):
            guest.wait("[WL-SGP] redraw window=", frame, timeout=30)
            name = ("before-click", "after-click")[frame - 1] if interactive else f"frame-{frame}"
            image = capture(guest, name)
            if interactive and frame == 1:
                assert "[WL-FIXTURE] click " not in guest.read(), "click preceded initial capture"
            locations.append(locate(image, frame))
            assert locations[-1] is not None, f"frame {frame} missing on scanout"
        assert locations[0] == locations[1], locations
        guest.wait("[WL-FIXTURE] complete after disconnect", timeout=25)
        if interactive:
            cleanup = input_cleanup(guest)
        else:
            guest.wait("[WL-SGP] removed window=", timeout=15)
        gone = capture(guest, "after-disconnect")
        assert all(locate(gone, frame) is None for frame in (1, 2)), "fixture still on scanout"
        if sunapp:
            guest.wait("[CANVAS-SUNAPP] waitpid pid=", timeout=15)
        serial = guest.read()
        windows = re.findall(r"\[WL-SGP\] redraw window=(\d+)", serial)
        assert len(windows) == 2 and windows[0] == windows[1], windows
        if not interactive:
            assert f"[WL-SGP] removed window={windows[0]}" in serial
        if libwayland:
            assert "[WL-FIXTURE] ack_configure serial=" in serial
            pool_bytes = 4096 + 2 * 1024 * 160 if canvas else 170509
            assert f"[WL-FIXTURE] create_pool bytes={pool_bytes} fd_mark=" in serial
            assert "[WL-LIB] frame 2 released and callback completed" in serial
        else:
            assert "[WL-FIXTURE] create_pool bytes=170509 fd_mark=5" in serial
            assert "[WL-FIXTURE] skipped invalid buffer=10" in serial
        assert "[WL-CLIENT] FAIL" not in serial and "[WL-FIXTURE] FAIL" not in serial
        commits = re.findall(r"\[WL-FIXTURE\] wl_surface.commit frame=(\d) buffer=(\d+) hash=([a-f0-9]+)", serial)
        assert [seq for seq, _, _ in commits] == ["1", "2"], commits
        if canvas:
            rendered = re.findall(
                r"\[WL-CANVAS\] frame=(\d) buffer=(\d+) hash=([a-f0-9]+) stride=1024 XRGB8888",
                serial,
            )
            assert rendered == commits and commits[0][1] != commits[1][1], (rendered, commits)
            assert all(f"[WL-LIB] frame {frame} released and callback completed" in serial
                       for frame in (1, 2)), serial[-2000:]
            name = "sunlight-ui-wayland" if sunapp else "helios-wayland-client"
            finished = re.search(
                rf"\[SCHED\] process_mark_finished pid=(\d+) name='{name}' reason=exit code=0\b",
                serial,
            )
            assert finished, "Canvas client did not exit cleanly"
            if sunapp:
                pid = finished[1]
                assert f"[CANVAS-SUNAPP] waitpid pid={pid} status=Ok(0)" in serial
                assert f"[CANVAS-SUNAPP] spawned pid={pid} executable=/Applications/SunlightCanvas.sunapp/Program/sunlight-ui-wayland" in serial
                assert re.search(rf"\[CANVAS-SUNAPP\] child context pid={pid} uid=0 gid=0 cwd=/ env_count=0 capabilities=0\b", serial)
                if not interactive:
                    assert commits == [("1", "10", "4fbc9f29"), ("2", "11", "a5734799")], commits
        if not libwayland:
            assert [buf for _, buf, _ in commits] == ["11", "12"], commits
        assert commits[0][2] != commits[1][2], commits
        evidence = {"accel": accel, "vcpus": 2, "window": int(windows[0]),
                    "origin": locations[0], "commits": commits, "verdict": "PASS"}
        if sunapp:
            evidence.update(mode=mode, pid=int(pid), exit_status=0)
        if interactive:
            evidence["cleanup"] = cleanup
            enter = re.search(r"\[WL-INPUT\] enter surface=(\d+) x_fixed=16384 y_fixed=28672 serial=2", serial)
            assert enter, "missing client pointer focus/enter"
            surface = int(enter[1])
            sent = f"[WL-FIXTURE] click surface={surface} x=64 y=112 button=272 state=1 serial=3"
            hit = (f"[WL-INPUT] hit surface={surface} x_fixed=16384 y_fixed=28672"
                   " button=272 state=1 serial=3 counter=1")
            assert serial.count(sent) == serial.count(hit) == 1, "missing or duplicate pointer hit"
            assert (serial.index("[WL-CANVAS] frame=1") < enter.start() < serial.index(hit)
                    < serial.index("[WL-CANVAS] frame=2")), "unexpected input/paint order"
            evidence["input"] = {
                "surface": surface, "logical": [64, 112], "fixed": [16384, 28672],
                "enter_serial": 2, "button": 272, "state": 1, "serial": 3,
                "counter_before": 0, "counter_after": 1,
            }
        (guest.out / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")
        print(json.dumps(evidence))
    finally:
        guest.close()


if __name__ == "__main__":
    iso, out, accel, *mode = sys.argv[1:]
    if accel not in ("kvm", "tcg"):
        raise ValueError(f"unknown accelerator: {accel}")
    if mode not in ([], ["libwayland"], ["canvas"], ["canvas-sunapp"], ["canvas-sunapp-input"]):
        raise ValueError(f"unknown gate mode: {mode}")
    run(Path(iso), Path(out), accel, mode[0] if mode else "wire")
