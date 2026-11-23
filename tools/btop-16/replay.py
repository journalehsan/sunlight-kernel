"""Run the unchanged read-only baseline with more time between TTY keys."""
from pathlib import Path
import runpy
import sys
import time

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "target/btop-01"))
from qmp_fixture import Guest

key = Guest.key


def paced_key(self, *keys, hold=60):
    key(self, *keys, hold=hold)
    time.sleep(.25)


Guest.key = paced_key
runpy.run_path(str(root / "tools/btop-10/replay.py"), run_name="__main__")
