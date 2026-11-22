"""Run a guest-code BIOS/BDA probe in a disposable copy of the desktop ISO."""

import json
import mmap
from pathlib import Path
import shutil
import subprocess
import sys
import time

from desktop import Guest


def make_image(source, destination):
    here = Path(__file__).resolve().parent
    original = (here.parents[3] / "SunlightMines.sunapp/Program/SUNMINE.EXE").read_bytes()
    binary = destination.with_suffix(".com")
    subprocess.run(["nasm", "-f", "bin", str(here / "modifiers.asm"), "-o", str(binary)],
                   check=True)
    probe = binary.read_bytes()
    assert source.resolve() != destination.resolve() and len(probe) <= len(original)
    shutil.copyfile(source, destination)
    with destination.open("r+b") as stream, mmap.mmap(stream.fileno(), 0) as image:
        offsets = []
        offset = image.find(original)
        while offset >= 0:
            offsets.append(offset)
            offset = image.find(original, offset + len(original))
        assert len(offsets) == 2, offsets
        print(f"Embedded Mines payload offsets: {offsets}; length: {len(original)}", flush=True)
        for offset in offsets:
            image[offset:offset + len(original)] = probe.ljust(len(original), bytes([0x90]))


def run(guest, log):
    def record(**fields):
        print(json.dumps(dict(serial_bytes=guest.serial.stat().st_size, **fields)),
              file=log, flush=True)

    def key(*keys):
        record(keys=keys)
        guest.key(*keys)

    def edge(side, pressed):
        record(edge=side, down=pressed)
        guest.request("input-send-event", {"events": [{"type": "key", "data": {
            "down": pressed, "key": {"type": "qcode", "data": side}}}]})
        time.sleep(.25)

    def frame(name):
        record(frame=name)
        print(name, guest.capture(name), flush=True)

    guest.wait("[TTY]  Login screen ready", timeout=80)
    guest.wait("[init] Name server: listening", timeout=30)
    time.sleep(3)
    key("ret")
    guest.type("root")
    key("ret")
    guest.wait("[SESSION-FOUNDATION] SHELL_READY PASS", timeout=70)
    guest.wait("[SESSION] switched to F2 GraphicalDesktop", timeout=50)
    key("meta_l", "k")
    guest.type("mines")
    key("ret")
    guest.wait("Sunlight Mines phase=window_visible_on_screen", timeout=30)
    time.sleep(2)
    key("alt", "tab")
    frame("initial")

    for first, second in (("shift", "shift_r"), ("shift_r", "shift")):
        edge(first, True)
        frame(f"{first}-make")
        edge(second, True)
        frame(f"{first}-both")
        edge(first, False)
        frame(f"{first}-first-break")
        edge(second, False)
        frame(f"{first}-second-break")

    for first, second in (("shift", "shift_r"), ("shift_r", "shift")):
        edge(first, True)
        edge(second, True)
        frame(f"{first}-before-focus-loss")
        key("alt", "tab")
        frame(f"{first}-focus-lost")
        key("alt", "tab")
        frame(f"{first}-focus-regained")
        edge(first, False)
        frame(f"{first}-delayed-first-break")
        key("right")
        frame(f"{first}-delayed-held-snapshot")
        edge(second, False)
        frame(f"{first}-delayed-second-break")
        edge(second, True)
        frame(f"{first}-fresh-make")
        edge(second, False)
        frame(f"{first}-fresh-break")


if __name__ == "__main__":
    source, destination, output = map(Path, sys.argv[1:])
    make_image(source, destination)
    guest = Guest(destination, output)
    try:
        with (guest.out / "inputs.jsonl").open("w") as log:
            run(guest, log)
    finally:
        guest.close()
