#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 1 ]]; then
    echo "usage: $0 NEW-EVIDENCE-DIR" >&2
    exit 2
fi
cd "$(dirname "$0")/../.."
out=$(realpath -m "$1")
mkdir -- "$out"
python3 - "$out/cases.json" <<'PY'
import json
from pathlib import Path
import sys

p = Path("target/btop-02").resolve()
base = dict(elf=str(p / "procfs.elf"), abi="linux", expected_exit=0)
# The isolated harness starts Linux PIDs at 100. Verify that identity below
# rather than silently accepting a different fixture as the reaped target.
cases = [
    dict(base, name="procfs-visible"),
    dict(base, name="procfs-after-exit", args=["100"]),
    dict(name="procfs-native-reference", elf=str(p / "native-reference.elf"),
         abi="native", expected_exit=0),
]
Path(sys.argv[1]).write_text(json.dumps(cases, indent=2))
PY
python3 tools/native-c/guest.py --cases "$out/cases.json" --mixed-abi --memory-mb 4096 \
    --cpus 4 --timeout 120 --out "$out/guest" | tee "$out/run.log"
python3 - "$out" <<'PY'
from pathlib import Path
import re
import sys

out = Path(sys.argv[1])
log = (out / "run.log").read_text()
serial = (out / "guest/serial.log").read_text(errors="replace")
assert re.search(r"\[NATIVE-C-START\] case=0 name=procfs-visible pid=100 .* uid=1000 abi=linux", serial)
assert re.search(r"process_reaped pid=100 ", serial)
assert re.search(r"PROCFS-PASS pid=100 hidden=- cores=4", log)
assert re.search(r"PROCFS-PASS pid=101 hidden=100 cores=4", log)
assert "PROCFS-RECOVERED /proc/self/stat after EAGAIN" in log
second = re.search(r"PROCFS-PASS pid=101 .* mem=(\d+)/(\d+)", log)
native = re.search(r"NATIVE-TELEMETRY version=5 free=(\d+) total=(\d+)", log)
assert second and native
free, total = map(int, second.groups())
native_free, native_total = map(int, native.groups())
assert total == native_total and abs(free - native_free) <= 8192
print(f"PASS: lifecycle and native memory reference (KiB): procfs {free}/{total}, native {native_free}/{native_total}")
PY
