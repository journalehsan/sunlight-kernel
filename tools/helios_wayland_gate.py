#!/usr/bin/env python3
"""Capture and verify the Helios fixture on the guest's actual scanout."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time

from PIL import Image, ImageChops

from desktop_tty_ux_gate import Guest

PIXEL_HASHES = {
    False: (
        "ba4fb4c5d41af6e67d919507a5a9b5a9dcc964997eb7e79ec8fb445f672b83d7",
        "e15522ab2f0cb1759724f5aa2211c642e26c3952ba5e785f9d19d217936b9558",
    ),
    True: (
        "ae7ca47b24e3b100037f688c81eae9619dda8261201af57666b85726ee03a2a6",
        "c2efef4816e3f793923b2616f1af14ea987149331396bf9542ca8c1103e05553",
    ),
}
COMMIT_HASHES = {False: ("f2d6fd05", "21373585"), True: ("4fbc9f29", "a5734799")}
COMMIT = r"\[WL-FIXTURE\] wl_surface.commit frame=(\d) buffer=(\d+) hash=([a-f0-9]+)"


def pixel_hash(image, origin):
    left, top = origin
    return hashlib.sha256(image.crop((left, top, left + 256, top + 160)).tobytes()).hexdigest()


class Handshake:
    def __init__(self, guest, iso, mode, accel):
        self.guest = guest
        with iso.open("rb") as image:
            digest = hashlib.file_digest(image, "sha256").hexdigest()
        self.identity = {"run": str(guest.out), "client": mode, "accel": accel,
                         "image_sha256": digest}
        artifact_manifest = iso.parent / "artifacts.sha256"
        if artifact_manifest.exists():
            (guest.out / "artifacts.sha256").write_bytes(artifact_manifest.read_bytes())
        self.index = 0
        for name in ("transitions.jsonl", "acknowledgements.jsonl"):
            (guest.out / name).write_text("")

    def record(self, state, window, sequence, *, ack=False, **details):
        self.index += 1
        record = dict(self.identity, index=self.index, state=state, window=int(window),
                      sequence=sequence, **details)
        name = "acknowledgements.jsonl" if ack else "transitions.jsonl"
        with (self.guest.out / name).open("a") as log:
            log.write(json.dumps(record) + "\n")

    def acknowledge(self, window, sequence):
        marker = (f"frame-{sequence}-ack-received" if sequence < 3
                  else "removal-ack-received")
        self.guest.control.sendall(str(sequence).encode())
        self.record("sent", window, min(sequence, 2), ack=True, token=sequence)
        self.guest.wait(f"[WL-CAPTURE] {marker}\n", timeout=10)
        self.record("received", window, min(sequence, 2), ack=True, token=sequence)

    def verified(self, name, image, origin, expected):
        assert image.size == (1280, 800), image.size
        with Image.open(self.guest.out / f"{name}.png") as saved:
            assert saved.size == image.size and saved.convert("RGB").tobytes() == image.tobytes()
        digest = pixel_hash(image, origin)
        if expected is not None:
            assert digest == expected, f"{name}: crop SHA-256 {digest}, expected {expected}"
        return {"pixel_sha256": digest, "dimensions": image.size,
                "png_sha256": hashlib.sha256((self.guest.out / f"{name}.png").read_bytes()).hexdigest()}


def capture(guest, name):
    ppm = guest.out / f"{name}.ppm"
    guest.request("screendump", {"filename": str(ppm)})
    with Image.open(ppm) as source:
        image = source.convert("RGB")
    image.save(guest.out / f"{name}.png")
    return image


def wait_scanout(guest, name, predicate):
    deadline = time.monotonic() + 15
    while True:
        image = capture(guest, name)
        state = predicate(image)
        if state is not None:
            return image, state
        if time.monotonic() >= deadline:
            raise AssertionError(f"{name}: expected scanout state did not appear")


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
    phase = "boot ordering"
    guest = Guest(iso, out, vga="std", accel=accel, serial_control=not interactive)
    try:
        handshake = None if interactive else Handshake(guest, iso, mode, accel)
        stall = os.environ.get("HELIOS_WAYLAND_STALL_CAPTURE") == "1"
        guest.wait("[TTY]  Login screen ready")
        guest.wait("[init] Name server: listening")
        guest.wait("[SESSION-FOUNDATION] SERVICE_READY PASS")
        guest.key("ret")
        guest.type("root")
        guest.key("ret")
        if libwayland:
            phase = "process lifecycle"
            guest.wait("[WL-LIB] entered main", timeout=30)
            phase = "request dispatch"
            guest.wait("[WL-LIB] discovered compositor, shm and xdg-shell", timeout=15)
        phase = "boot ordering"
        guest.wait("[SESSION] switched to F2 GraphicalDesktop")
        locations = []
        for frame in (1, 2):
            phase = "SGP redraw"
            guest.wait("[WL-SGP] redraw window=", frame, timeout=30)
            windows = re.findall(r"\[WL-SGP\] redraw window=(\d+)", guest.read())
            assert len(windows) == frame and len(set(windows)) == 1, windows
            if handshake:
                marker = f"[WL-CAPTURE] WAIT_FRAME_{frame}_CAPTURE_ACK\n"
                guest.wait(marker, timeout=10)
                serial = guest.read()
                commits = re.findall(COMMIT, serial)
                assert [entry[0] for entry in commits] == [str(seq) for seq in range(1, frame + 1)]
                assert commits[-1][2] == COMMIT_HASHES[canvas][frame - 1], commits
                commit_position = serial.rindex(f"wl_surface.commit frame={frame} ")
                assert commit_position < min(
                    serial.rindex(f"[WL-SGP] redraw window={windows[0]}"),
                    serial.index(marker),
                ), "commit must precede redraw and buffer release"
                handshake.record(f"frame-{frame}-committed", windows[0], frame,
                                 buffer=int(commits[-1][1]), commit_hash=commits[-1][2])
                if stall and frame == 1:
                    phase = "capture-ack timeout"
                    guest.wait("[WL-CLIENT] FAIL capture-ack timeout frame=1", timeout=40)
                    guest.wait(f"[WL-SGP] removed window={windows[0]}\n", timeout=15)
                    serial = guest.read()
                    assert len(re.findall(COMMIT, serial)) == 1
                    assert serial.count("[WL-SGP] redraw window=") == 1
                    assert "frame-1-ack-received" not in serial
                    assert "WAIT_FRAME_2_CAPTURE_ACK" not in serial
                    handshake.record("capture-ack-timeout-cleanup", windows[0], 1)
                    (guest.out / "fault-injection.json").write_text(json.dumps({
                        "timeout": "observed", "frame_2": "not committed",
                        "acknowledgement": "withheld", "window_removed": int(windows[0]),
                    }) + "\n")
                    raise AssertionError("injected host stall: guest capture-ack timeout frame=1")
            name = ("before-click", "after-click")[frame - 1] if interactive else f"frame-{frame}"
            phase = "scanout capture"
            def visible(shot):
                origin = locate(shot, frame)
                if origin is not None and (not handshake or (
                    shot.size == (1280, 800) and
                    pixel_hash(shot, origin) == PIXEL_HASHES[canvas][frame - 1]
                )):
                    return origin
                return None

            image, origin = wait_scanout(guest, name, visible)
            if handshake:
                handshake.record(f"frame-{frame}-visible", windows[0], frame, origin=origin)
                info = handshake.verified(name, image, origin, PIXEL_HASHES[canvas][frame - 1])
                handshake.record(f"frame-{frame}-captured", windows[0], frame, **info)
                phase = "capture acknowledgement"
                handshake.acknowledge(windows[0], frame)
            if interactive and frame == 1:
                assert "[WL-FIXTURE] click " not in guest.read(), "click preceded initial capture"
            locations.append(origin)
        assert locations[0] == locations[1], locations
        phase = "process lifecycle"
        guest.wait("[WL-FIXTURE] peer EOF after frame=2", timeout=25)
        if handshake:
            handshake.record("peer-disconnected", windows[0], 2)
            guest.wait("[WL-CAPTURE] WAIT_SGP_REMOVAL\n", timeout=10)
        guest.wait("[WL-FIXTURE] complete after disconnect", timeout=25)
        phase = "window removal"
        if interactive:
            cleanup = input_cleanup(guest)
        else:
            guest.wait(f"[WL-SGP] removed window={windows[0]}\n", timeout=15)
            if handshake:
                serial = guest.read()
                assert (serial.index("frame-2-ack-received") <
                        serial.index("[WL-FIXTURE] peer EOF after frame=2") <
                        serial.index(f"[WL-SGP] removed window={windows[0]}\n"))
                handshake.record("sgp-window-removed", windows[0], 2)
        phase = "scanout capture"
        left, top = locations[1]
        rect = (left, top, left + 256, top + 160)
        last = image.crop(rect)

        def absent(shot):
            if handshake and shot.size != (1280, 800):
                return None
            changed = ImageChops.difference(last, shot.crop(rect)).getbbox()
            return True if changed and all(locate(shot, frame) is None for frame in (1, 2)) else None

        final, _ = wait_scanout(guest, "after-disconnect", absent)
        if handshake:
            info = handshake.verified("after-disconnect", final, locations[1], None)
            handshake.record("after-disconnect-captured", windows[0], 2, **info)
            handshake.acknowledge(windows[0], 3)
        if sunapp:
            phase = "process lifecycle"
            guest.wait("[CANVAS-SUNAPP] waitpid pid=", timeout=15)
        elif handshake:
            exit_marker = re.compile(r"\[SCHED\] process_mark_finished pid=\d+ name='helios-wayland-client' reason=exit code=0\b")
            deadline = time.monotonic() + 10
            while not exit_marker.search(guest.read()):
                assert time.monotonic() < deadline, "client did not exit cleanly after removal"
                time.sleep(0.1)
        serial = guest.read()
        windows = re.findall(r"\[WL-SGP\] redraw window=(\d+)", serial)
        assert len(windows) == 2 and windows[0] == windows[1], windows
        if not interactive:
            assert f"[WL-SGP] removed window={windows[0]}" in serial
        if libwayland:
            phase = "configure/ack"
            assert "[WL-FIXTURE] ack_configure serial=" in serial
            phase = "buffer lifecycle"
            pool_bytes = 4096 + 2 * 1024 * 160 if canvas else 170509
            assert f"[WL-FIXTURE] create_pool bytes={pool_bytes} fd_mark=" in serial
            phase = "frame callback"
            assert "[WL-LIB] frame 2 released and callback completed" in serial
        else:
            assert "[WL-FIXTURE] create_pool bytes=170509 fd_mark=5" in serial
            assert "[WL-FIXTURE] skipped invalid buffer=10" in serial
        assert "[WL-CLIENT] FAIL" not in serial and "[WL-FIXTURE] FAIL" not in serial
        commits = re.findall(COMMIT, serial)
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
            phase = "process lifecycle"
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
    except Exception as error:
        serial = guest.read()
        if "[CANVAS-SUNAPP] artifact digest mismatch" in serial:
            phase = "bundle installation"
        elif "[WL-FIXTURE] FAIL desktop unavailable" in serial:
            phase = "boot ordering"
        elif "[WL-CLIENT] FAIL connect" in serial or "[WL-FIXTURE] FAIL listen" in serial:
            phase = "endpoint readiness"
        elif "[WL-CLIENT] FAIL registry dispatch" in serial:
            phase = "request dispatch"
        (guest.out / "result.json").write_text(json.dumps({
            "accel": accel, "mode": mode, "vcpus": 2, "verdict": "FAIL",
            "classification": phase, "error": str(error),
        }, indent=2) + "\n")
        raise
    finally:
        guest.close()
        serial = guest.read()
        (guest.out / "lifecycle.log").write_text("\n".join(
            line for line in serial.splitlines()
            if any(marker in line for marker in (
                "[WL-", "[CANVAS-SUNAPP]", "GraphicalDesktop", "process_mark_finished",
            ))
        ) + "\n")
        images = sorted((*guest.out.glob("*.ppm"), *guest.out.glob("*.png")))
        (guest.out / "captures.sha256").write_text("".join(
            f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in images
        ))


if __name__ == "__main__":
    iso, out, accel, *mode = sys.argv[1:]
    if accel not in ("kvm", "tcg"):
        raise ValueError(f"unknown accelerator: {accel}")
    if mode not in ([], ["wire"], ["libwayland"], ["canvas"], ["canvas-sunapp"], ["canvas-sunapp-input"]):
        raise ValueError(f"unknown gate mode: {mode}")
    run(Path(iso), Path(out), accel, mode[0] if mode else "wire")
