#!/usr/bin/env python3
"""Capture and verify the Helios fixture on the guest's actual scanout."""
import json
from pathlib import Path
import re
import sys

from PIL import Image

from desktop_tty_ux_gate import Guest


def capture(guest, name):
    ppm = guest.out / f"{name}.ppm"
    guest.request("screendump", {"filename": str(ppm)})
    with Image.open(ppm) as source:
        image = source.convert("RGB")
    image.save(guest.out / f"{name}.png")
    return image


def pattern(image, frame):
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


def run(iso, out, accel, libwayland=False):
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
            image = capture(guest, f"frame-{frame}")
            locations.append(pattern(image, frame))
            assert locations[-1] is not None, f"frame {frame} missing on scanout"
        assert locations[0] == locations[1], locations
        guest.wait("[WL-FIXTURE] complete after disconnect", timeout=25)
        guest.wait("[WL-SGP] removed window=", timeout=15)
        gone = capture(guest, "after-disconnect")
        assert all(pattern(gone, frame) is None for frame in (1, 2)), "fixture still on scanout"
        serial = guest.read()
        windows = re.findall(r"\[WL-SGP\] redraw window=(\d+)", serial)
        assert len(windows) == 2 and windows[0] == windows[1], windows
        assert f"[WL-SGP] removed window={windows[0]}" in serial
        if libwayland:
            assert "[WL-FIXTURE] ack_configure serial=" in serial
            assert "[WL-FIXTURE] create_pool bytes=170509 fd_mark=" in serial
            assert "[WL-LIB] frame 2 released and callback completed" in serial
        else:
            assert "[WL-FIXTURE] create_pool bytes=170509 fd_mark=5" in serial
            assert "[WL-FIXTURE] skipped invalid buffer=10" in serial
        assert "[WL-CLIENT] FAIL" not in serial and "[WL-FIXTURE] FAIL" not in serial
        commits = re.findall(r"\[WL-FIXTURE\] wl_surface.commit frame=(\d) buffer=(\d+) hash=([a-f0-9]+)", serial)
        assert [seq for seq, _, _ in commits] == ["1", "2"], commits
        if not libwayland:
            assert [buf for _, buf, _ in commits] == ["11", "12"], commits
        assert commits[0][2] != commits[1][2], commits
        evidence = {"accel": accel, "vcpus": 2, "window": int(windows[0]),
                    "origin": locations[0], "commits": commits, "verdict": "PASS"}
        (guest.out / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")
        print(json.dumps(evidence))
    finally:
        guest.close()


if __name__ == "__main__":
    iso, out, accel, *mode = sys.argv[1:]
    if accel not in ("kvm", "tcg"):
        raise ValueError(f"unknown accelerator: {accel}")
    if mode not in ([], ["libwayland"]):
        raise ValueError(f"unknown gate mode: {mode}")
    run(Path(iso), Path(out), accel, libwayland=bool(mode))
