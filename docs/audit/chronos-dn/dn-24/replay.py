#!/usr/bin/env python3
"""Controlled two-run DN-24 input on the actual Sunlight desktop via PS/2."""
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
        text = guest.capture(name)
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

        mine = launch("mines", "Sunlight Mines", "mines")
        key("spc")
        shot("mines-play")
        key("esc")
        wait_session(3, timeout=20)
        shot("mines-guest-exit")
        assert f"process_mark_finished pid={mine} " not in guest.read()
        key("ctrl", "w")
        guest.wait(f"process_mark_finished pid={mine} name='sunlight-chronos' reason=exit code=143",
                   timeout=15)
        guest.wait(f"process_reaped pid={mine} name='sunlight-chronos'", timeout=15)
        guest.wait(f"app_instance_removed pid={mine} name=Sunlight Mines", timeout=15)
        shot("mines-restored")
    finally:
        guest.close()
