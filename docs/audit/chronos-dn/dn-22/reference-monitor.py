from pathlib import Path
import os
import subprocess, time
out = Path(os.environ.get("DN22_OUT", "docs/audit/chronos-dn/dn-22"))
root = Path("/tmp/dn22")
with (out / "reference-monitor.log").open("wb") as log:
    q = subprocess.Popen(["qemu-system-x86_64", "-m", "32", "-display", "none",
        "-monitor", "stdio", "-drive", f"file={root}/boot.img,format=raw,if=floppy",
        "-drive", f"file=fat:rw:{root}/ref-drive-verified,format=raw", "-boot", "a"],
        stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True)
    def cmd(s, pause=.35):
        q.stdin.write(s+"\n"); q.stdin.flush(); time.sleep(pause)
    def keys(*args):
        for key in args: cmd("sendkey " + key)
    def shot(name):
        cmd("info registers"); cmd("xp /2hx 0xbc"); cmd(f"screendump {root}/ref-{name}.ppm")
    try:
        time.sleep(5)
        shot("shell")
        keys("c","h","e","c","k","d","n","ret")
        time.sleep(3)
        shot("direct")
        keys("f10","m","down","up","ret")
        shot("picker")
        keys("ret")
        time.sleep(3)
        shot("panels")
        keys("tab","down")
        shot("focus")
        keys("alt-x","y","ret")
        shot("exit")
        cmd("quit")
    finally:
        q.wait(timeout=4)
