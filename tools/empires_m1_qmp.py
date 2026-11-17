#!/usr/bin/env python3
"""Native UI evidence helper; never injects game state or bypasses input.

Use with a running QEMU -qmp unix:SOCKET,server=on,wait=off instance.
Commands: key QCODE..., type TEXT, move DX DY [COUNT], click [left|right],
button BUTTON down|up, shot PATH. Coordinates for move are relative hardware
motion, subject to the ordinary guest pointer policy. Inspect screenshots to
confirm positions before clicking. Uses the repository's existing QMP client.
"""
import argparse
import json
import time
from pathlib import Path
from voice_recorder_qmp import Qmp


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("socket")
    parser.add_argument("command", choices=["key", "type", "clear", "move", "click", "button", "shot", "quit", "hmp", "goto"])
    parser.add_argument("args", nargs="*")
    args = parser.parse_args()
    q = Qmp(args.socket)
    q.socket.settimeout(10)

    def key(*codes):
        for down, ordered in [(True, codes), (False, tuple(reversed(codes)))]:
            for code in ordered:
                q.call("input-send-event", {"events": [{"type": "key", "data": {
                    "down": down, "key": {"type": "qcode", "data": code}}}]})
            time.sleep(.35)

    def button(name, down):
        q.call("input-send-event", {"events": [{"type": "btn", "data": {"button": name, "down": down}}]})
        time.sleep(.2)

    if args.command == "key":
        key(*args.args)
    elif args.command == "type":
        plain = {" ": "spc", "/": "slash", ".": "dot", "-": "minus", "\n": "ret", "\t": "tab"}
        for ch in args.args[0]:
            if ch.isupper():
                key("shift", ch.lower())
            else:
                key(plain.get(ch, ch))
    elif args.command == "clear":
        for _ in range(int(args.args[0]) if args.args else 64):
            key("backspace")
    elif args.command == "move":
        dx, dy = map(int, args.args[:2])
        for _ in range(int(args.args[2]) if len(args.args) > 2 else 1):
            q.call("human-monitor-command", {"command-line": f"mouse_move {dx} {dy}"})
            time.sleep(.6)
    elif args.command == "click":
        name = args.args[0] if args.args else "left"
        button(name, True)
        button(name, False)
    elif args.command == "button":
        button(args.args[0], args.args[1] == "down")
    elif args.command == "hmp":
        print(q.call("human-monitor-command", {"command-line": " ".join(args.args)}))
    elif args.command == "goto":
        from PIL import Image
        template = Image.open(Path(__file__).resolve().parents[1] / "assets/cursors/pointer.tga").convert("RGBA")
        tp = template.load()
        points = [(x,y,tp[x,y][:3]) for y in range(template.height) for x in range(template.width) if tp[x,y][3] == 255]
        ax,ay,color = next(p for p in points if sum(p[2]) > 740)
        shot = "/tmp/empires-m1-cursor.png"
        def position():
            q.call("screendump", {"filename":shot, "format":"png"})
            im=Image.open(shot).convert("RGB"); ip=im.load()
            for n,pixel in enumerate(im.getdata()):
                if pixel != color: continue
                ox=n%im.width-ax; oy=n//im.width-ay
                if ox<0 or oy<0 or ox+template.width>im.width or oy+template.height>im.height: continue
                if all(ip[ox+x,oy+y] == c for x,y,c in points[::12]) and all(ip[ox+x,oy+y] == c for x,y,c in points):
                    return ox+3,oy+2
            raise RuntimeError("pointer template not visible; inspect a screenshot")
        target=tuple(map(int,args.args)); gain=[1.5,1.5]
        for attempt in range(120):
            before=position(); diff=[target[i]-before[i] for i in range(2)]
            if max(map(abs,diff)) <= 3: break
            axis=0 if abs(diff[0])>=abs(diff[1]) else 1
            amount=min(100,max(1,round(abs(diff[axis])/gain[axis]))) * (1 if diff[axis]>0 else -1)
            delta=[0,0];delta[axis]=amount
            q.call("human-monitor-command", {"command-line":f"mouse_move {delta[0]} {delta[1]}"})
            time.sleep(.65)
            after=position(); moved=abs(after[axis]-before[axis])
            if moved: gain[axis]=max(.1,moved/abs(amount))
        else: raise RuntimeError(f"pointer did not reach target: {position()}")
        print(json.dumps({"pointer":before,"target":target,"moves":attempt}))
    elif args.command == "quit":
        q.call("quit")
    elif args.command == "shot":
        q.call("screendump", {"filename": args.args[0], "format": "png"})
    record={"action": args.command, "args": args.args if args.args != ["root"] else ["[fixture credential]"], "executed": True}
    print(json.dumps(record))
    evidence=Path(__file__).resolve().parents[1]/"target/empires-m1/qmp-actions.jsonl"
    evidence.parent.mkdir(parents=True,exist_ok=True)
    with evidence.open("a") as log:log.write(json.dumps(record)+"\n")
    q.stream.close()
    q.socket.close()


if __name__ == "__main__":
    main()
