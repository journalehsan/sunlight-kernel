#!/usr/bin/env python3
"""Small QMP UI driver for reproducible native recorder validation.

Example: python3 tools/voice_recorder_qmp.py /tmp/voice-recorder-qmp.sock key r
Commands: key QCODE [QCODE ...], type TEXT, click X Y [WIDTH HEIGHT],
shot PATH, hmp COMMAND, quit. The VM must already be running.
"""
import json
import socket
import sys
import time


class Qmp:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.connect(path)
        self.stream = self.socket.makefile("rwb", buffering=0)
        json.loads(self.stream.readline())
        self.call("qmp_capabilities")

    def call(self, command, arguments=None):
        request = {"execute": command}
        if arguments is not None:
            request["arguments"] = arguments
        self.stream.write(json.dumps(request).encode() + b"\n")
        while True:
            result = json.loads(self.stream.readline())
            if "error" in result:
                raise RuntimeError(result["error"])
            if "return" in result:
                return result["return"]

    def key(self, *codes):
        for down, ordered in [(True, codes), (False, tuple(reversed(codes)))]:
            for code in ordered:
                self.call("input-send-event", {"events": [{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": code}}}]})
                time.sleep(0.1)
            time.sleep(0.5)

    def type(self, text):
        plain = {" ": "spc", "/": "slash", ".": "dot", "-": "minus", "\n": "ret", "\t": "tab"}
        shifted = {"_": "minus", ":": "semicolon"}
        for ch in text:
            if ch in shifted:
                self.key("shift", shifted[ch])
            elif ch.isupper():
                self.key("shift", ch.lower())
            else:
                self.key(plain.get(ch, ch))

    def click(self, x, y, width=1280, height=800):
        self.call("input-send-event", {"events": [
            {"type": "abs", "data": {"axis": "x", "value": round(x * 32767 / (width - 1))}},
            {"type": "abs", "data": {"axis": "y", "value": round(y * 32767 / (height - 1))}},
        ]})
        time.sleep(0.1)
        for down in [True, False]:
            self.call("input-send-event", {"events": [{"type": "btn", "data": {"button": "left", "down": down}}]})
            time.sleep(0.1)


if __name__ == "__main__":
    qmp = Qmp(sys.argv[1])
    action, *args = sys.argv[2:]
    if action == "key":
        qmp.key(*args)
    elif action == "type":
        qmp.type(args[0])
    elif action == "click":
        qmp.click(*map(int, args))
    elif action == "shot":
        options = {"filename": args[0], "format": "png"}
        if len(args) > 1:
            options["device"] = args[1]
        qmp.call("screendump", options)
    elif action == "move":
        dx, dy, count = map(int, args)
        for _ in range(count):
            qmp.call("human-monitor-command", {"command-line": f"mouse_move {dx} {dy}"})
            time.sleep(0.08)
    elif action == "button":
        qmp.call("human-monitor-command", {"command-line": "mouse_button 1"})
        time.sleep(0.25)
        qmp.call("human-monitor-command", {"command-line": "mouse_button 0"})
        time.sleep(0.5)
    elif action == "hmp":
        print(qmp.call("human-monitor-command", {"command-line": args[0]}))
    elif action == "quit":
        qmp.call("quit")
    else:
        raise SystemExit("Unknown command")
