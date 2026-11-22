"""Replay dual Shift on the stock DOS Navigator SunApp via emulated PS/2."""

import argparse
import json
from pathlib import Path
import re
import sys
import time

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
from desktop_tty_ux_gate import Guest


def replay(guest, log):
    def record(**fields):
        print(json.dumps(dict(serial_bytes=guest.serial.stat().st_size, **fields)),
              file=log, flush=True)

    def key(*keys):
        record(keys=keys)
        guest.key(*keys)

    def edge(side, down):
        record(edge=side, down=down)
        guest.request("input-send-event", {"events": [{"type": "key", "data": {
            "down": down, "key": {"type": "qcode", "data": side}}}]})
        time.sleep(.22)

    def shot(name, expected=None):
        guest.capture(name)
        if expected is None:
            return
        with Image.open(guest.out / f"{name}.png") as frame:
            rgb = frame.convert("RGB")
            colors = [rgb.getpixel((600 + 9 * index, 341)) for index in range(6)]
        assert set(colors) <= {(12, 20, 37), (111, 162, 230)}, (name, colors)
        observed = "".join(char for char, color in zip("abcdef", colors)
                           if color == (111, 162, 230))
        record(frame=name, observed=observed, expected=expected)
        assert observed == expected, (name, observed, expected)

    def launch():
        title = "DOS Navigator 1.51"
        marker = f"{title} phase=window_visible_on_screen"
        count = guest.read().count(marker)
        key("meta_l", "k")
        for _ in range(24 if count else 0):
            key("backspace")
        record(type="dos navigator")
        guest.type("dos navigator")
        shot(f"search-{count + 1}")
        key("ret")
        guest.wait(marker, count + 1, timeout=30)
        time.sleep(6)
        return re.findall(re.escape(marker) + r"[^\n]* pid=(\d+)", guest.read())[-1]

    guest.wait("[TTY]  Login screen ready", timeout=80)
    guest.wait("[init] Name server: listening", timeout=30)
    time.sleep(3)
    key("ret")
    record(type="root")
    guest.type("root")
    key("ret")
    guest.wait("[SESSION-FOUNDATION] SHELL_READY PASS", timeout=70)
    guest.wait("[SESSION] switched to F2 GraphicalDesktop", timeout=50)
    shot("desktop")

    first_pid = launch()
    key("f10")
    key("m")
    key("n")
    key("c")
    key("tab")
    key("down")
    key("ret")
    key("ctrl", "f3")
    key("esc")
    key("kp_add")
    shot("file-mask")
    key("home")
    for _ in range(8):
        key("delete")
    record(type="abcdef")
    guest.type("abcdef")
    for first, second in (("shift", "shift_r"), ("shift_r", "shift")):
        key("home")
        for _ in range(3):
            key("right")
        shot(f"{first}-plain", "")
        edge(first, True)
        edge(second, True)
        key("left")
        shot(f"{first}-both-left", "c")
        key("right")
        shot(f"{first}-both-right", "")
        key("right")
        shot(f"{first}-both-extend", "d")
        edge(first, False)
        key("right")
        shot(f"{first}-one-held", "de")
        edge(second, False)
        key("left")
        shot(f"{first}-both-released", "")
        edge(second, True)
        key("left")
        shot(f"{first}-fresh", "d")
        edge(second, False)
        key("right")
        shot(f"{first}-fresh-released", "")

    key("esc")
    shot("mask-canceled")
    key("kp_add")
    key("ret")
    shot("mask-accepted")
    for run, pid in enumerate((first_pid, None), 1):
        if pid is None:
            pid = launch()
            assert pid != first_pid
        key("alt", "x")
        shot(f"exit-confirm-{run}")
        key("y")
        guest.wait("DOS session completed code=", run, timeout=20)
        assert len(re.findall(r"DOS session completed code=\s*0\b", guest.read())) == run
        guest.wait(f"process_mark_finished pid={pid} name='sunlight-chronos' reason=exit code=0",
                   timeout=20)
        guest.wait(f"process_reaped pid={pid} name='sunlight-chronos'", timeout=20)
        guest.wait(f"app_instance_removed pid={pid} name=DOS Navigator 1.51", timeout=20)
        assert f"app_terminate_requested pid={pid} " not in guest.read()
        shot(f"restored-{run}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    guest = Guest(args.iso, args.output)
    try:
        with (guest.out / "inputs.jsonl").open("w") as log:
            replay(guest, log)
    finally:
        guest.close()
