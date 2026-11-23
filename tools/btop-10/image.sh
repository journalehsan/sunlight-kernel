#!/usr/bin/env bash
set -euo pipefail
out=target/btop-10/final
mkdir -p "$out"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c
sed -e 's@target/btop-08/final@target/btop-10/final@g' \
    tools/btop-08/image.sh > "$out/base.sh"
python3 - "$out/completion.patch" <<'PY'
import difflib
import sys
from pathlib import Path

path = Path("kernel/src/main.rs")
old = path.read_text()
anchor = "        unsafe { blk.read_typed(lba, buf) }.legacy()"
assert old.count(anchor) == 1
new = old.replace(anchor, """        let result = unsafe { blk.read_typed(lba, buf) }.legacy();
        if result.is_ok() {
            use core::sync::atomic::{AtomicU64, Ordering};
            static COMPLETED: AtomicU64 = AtomicU64::new(0);
            let sequence = COMPLETED.fetch_add(1, Ordering::Relaxed) + 1;
            if sequence <= 4096 {
                serial_println!("[BTOP10] completion seq={} parent={} lba={} bytes={}",
                    sequence, VIRTIO_DEVICE_ID.load(Ordering::Acquire), lba, buf.len());
            } else if sequence == 4097 {
                serial_println!("[BTOP10] completion trace exhausted");
            }
        }
        result""")
Path(sys.argv[1]).write_text("".join(difflib.unified_diff(
    old.splitlines(True), new.splitlines(True), f"a/{path}", f"b/{path}")))
PY
git apply "$out/completion.patch"
trap 'git apply -R "$out/completion.patch"' EXIT
SUNLIGHT_BTOP06_WAIT_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 bash "$out/base.sh"
