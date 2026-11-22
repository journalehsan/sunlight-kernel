#!/usr/bin/env python3
"""Controlled two-run DN-27 input on the actual Sunlight desktop via PS/2."""
import argparse
import json
from pathlib import Path
import re
import sys
import time

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "tools"))
from desktop_tty_ux_gate import Guest

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("iso", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--trace-only", action="store_true", help="stop after the first Shift matrix")
options = parser.parse_args()
guest = Guest(options.iso, options.output)
with (guest.out / "inputs.jsonl").open("w") as log:
    def record(**values):
        print(json.dumps(values), file=log, flush=True)

    def key(*keys):
        record(keys=keys)
        guest.key(*keys)

    def shot(name):
        record(capture=name)
        text = None if options.trace_only else guest.capture(name)
        print(name, flush=True)
        return text

    def wait_session(count, timeout):
        deadline = time.monotonic() + timeout
        # Native debug_log may append a newline between the label and code.
        result = r"\[CHRONOS\] DOS session completed code=\s*0(?:\n|$)"
        while len(re.findall(result, guest.read())) < count:
            if guest.process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f"Timed out waiting for DOS session {count}")
            time.sleep(.1)

    def launch(query, title, prefix):
        old_count = guest.read().count(f"{title} phase=window_visible_on_screen")
        key("meta_l", "k")
        # Search retains its query, including a query longer than 12 chars.
        for _ in range(24):
            key("backspace")
        record(type=query)
        guest.type(query)
        shot(f"{prefix}-search")
        key("ret")
        guest.wait(f"{title} phase=window_visible_on_screen", old_count + 1, timeout=25)
        pattern = re.escape(title) + r" phase=window_visible_on_screen[^\n]* pid=(\d+)"
        pid = re.findall(pattern, guest.read())[-1]
        time.sleep(6)
        shot(f"{prefix}-first")
        return pid

    try:
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

        dn_pids = set()
        for run, enter in enumerate(("ret", "kp_enter"), 1):
            prefix = f"dn-{run}"
            pid = launch("dos navigator", "DOS Navigator 1.51", prefix)
            assert pid not in dn_pids
            dn_pids.add(pid)
            key("f10")
            shot(f"{prefix}-f10")
            key("m")
            key("n")
            shot(f"{prefix}-manager-new")
            key("c")
            shot(f"{prefix}-new-panels")
            key("tab")
            key("down")
            shot(f"{prefix}-directory-selected")
            key(enter)
            shot(f"{prefix}-directory-opened")
            key("ctrl", "f3")
            shot(f"{prefix}-ctrl-f3")
            key("esc")
            shot(f"{prefix}-escape")
            if run == 1:
                def edge(k, down):
                    record(edge=k, down=down)
                    guest.request("input-send-event", {"events": [{"type": "key",
                        "data": {"down": down, "key": {"type": "qcode", "data": k}}}]})
                    time.sleep(.18)

                key("kp_add")
                shot("mask-initial")
                key("home")
                for _ in range(8):
                    key("delete")
                guest.type("abcdef")
                key("home")
                key("right")
                key("right")
                shot("plain-right")
                key("left")
                shot("plain-left")
                for side in ("shift", "shift_r"):
                    key("home")
                    for _ in range(3):
                        key("right")
                    edge(side, True)
                    for i, arrow in enumerate(("left", "left", "right", "right", "right", "right", "left")):
                        key(arrow)
                        shot(f"{side}-{i}-{arrow}")
                    edge("right", True)
                    edge(side, False)
                    edge("right", False)
                    shot(f"{side}-released-before-arrow")
                    key("left")
                    shot(f"{side}-plain-after-release")
                if options.trace_only:
                    print("DN-27 trace matrix complete", flush=True)
                    break
                key("esc")
                shot(f"{prefix}-mask-cancel")
                key("kp_add")
                shot(f"{prefix}-mask-reopened")
                key("ret")
                shot(f"{prefix}-mask-accepted")
                # Work in the panel opened by Enter. Do not confirm Delete.
                for navigation in ("up", "down", "left", "right", "home", "end",
                                   "pgup", "pgdn", "insert", "delete"):
                    key(navigation)
                    shot(f"{prefix}-dedicated-{navigation}")
                key("esc")
                key("kp_8")
                shot(f"{prefix}-keypad-up-num-off")
                key("kp_2")
                shot(f"{prefix}-keypad-down-num-off")
                key("num_lock")
                key("pause")
                for digit in ("kp_1", "kp_2", "kp_3", "kp_4", "kp_5",
                              "kp_6", "kp_7", "kp_8", "kp_9", "kp_0", "kp_decimal"):
                    key(digit)
                shot(f"{prefix}-keypad-command-line-num-on")
                key("esc")
                for operator in ("kp_add", "kp_subtract", "kp_multiply", "kp_divide"):
                    key(operator)
                shot(f"{prefix}-keypad-arithmetic")
                key("esc")
                key("shift", "kp_8")
                shot(f"{prefix}-shift-keypad-num-on")
                key("ctrl", "up")
                key("alt", "kp_8")
                key("alt", "kp_decimal")
                key("up")
                shot(f"{prefix}-unmodified-after-chords")
                key("num_lock")
                key("esc")
            else:
                key("kp_add")
                key("home")
                for _ in range(8):
                    key("delete")
                guest.type("abbc")
                key("left")
                key("backspace")
                shot("mask-regression-backspace")
                key("home")
                key("delete")
                key("right")
                key("end")
                shot("mask-regression-delete")
                key("home")
                key("insert")
                key("z")
                shot("mask-regression-insert")
                key("ctrl_r", "left")
                key("alt_r", "right")
                shot("mask-regression-chords")
                key("esc")
            key("alt", "x")
            shot(f"{prefix}-alt-x")
            key("y")
            try:
                wait_session(run, timeout=5)
            except RuntimeError:
                # Retain an independent menu exit route if the shortcut doesn't
                # reach confirmation in this application state.
                key("esc")
                key("f10")
                key("f")
                key("x")
                shot(f"{prefix}-file-exit")
                key("y")
                wait_session(run, timeout=20)
            guest.wait(f"process_mark_finished pid={pid} name='sunlight-chronos' reason=exit code=0",
                       timeout=15)
            guest.wait(f"process_reaped pid={pid} name='sunlight-chronos'", timeout=15)
            guest.wait(f"app_instance_removed pid={pid} name=DOS Navigator 1.51", timeout=15)
            assert f"app_terminate_requested pid={pid} " not in guest.read()
            shot(f"{prefix}-restored")

        if options.trace_only:
            sys.exit(0)
        mine = launch("mines", "Sunlight Mines", "mines")
        key("spc")
        shot("mines-play")
        key("esc")
        wait_session(3, timeout=20)
        shot("mines-guest-exit")
        assert f"process_mark_finished pid={mine} " not in guest.read()
        key("ctrl", "w")
        guest.wait(f"app_terminate_requested pid={mine} name=Sunlight Mines signal=SIGTERM",
                   timeout=15)
        guest.wait(f"process_reaped pid={mine} name='sunlight-chronos'", timeout=15)
        assert re.search(
            rf"process_mark_finished pid={mine} name='sunlight-chronos' "
            r"reason=(exit|pending-fatal-signal) code=143\b", guest.read()
        )
        guest.wait(f"app_instance_removed pid={mine} name=Sunlight Mines", timeout=15)
        shot("mines-restored")
    finally:
        guest.close()
