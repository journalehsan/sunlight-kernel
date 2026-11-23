#!/usr/bin/env bash
# Reuse the BTop-01 checked packaging recipe. Never overwrite its evidence.
set -euo pipefail
cd "$(dirname "$0")/../.."
out="$PWD/target/btop-02"
mkdir -p "$out"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c -
"${MUSL_CC:-musl-gcc}" -std=c11 -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie \
    tools/btop-02/probe.c -o "$out/procfs.elf"
bash tools/stamp_helios_elf.sh "$out/procfs.elf"
python3 tools/native-c/cc.py -O2 -Wall -Wextra -Werror tools/btop-02/native-reference.c -o "$out/native-reference.elf"
sed -e 's|out="\$PWD/target/btop-01"|out="$PWD/target/btop-02"|g' \
    -e 's|../../target/btop-01/vfs-config-fixture.elf|../../target/btop-02/vfs-config-fixture.elf|g' \
    -e 's|../../target/btop-01/reference|../../target/btop-02/procfs.elf|g' \
    -e '/^    RamEntry::file("\/bin\/btop-reference"/i\    RamEntry::file("/bin/proc-native", 0, 0, mode::FILE_755,\n        include_bytes!("../../target/btop-02/native-reference.elf")),' \
    target/btop-01/ramfs-config-image.sh > "$out/image.sh"
# The historical trace spends its budget on loader/TTY calls. Restrict the
# replay trace to procfs, and record the actual RIP on an unhandled user fault.
python3 - "$out/image.sh" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace("        let seq = EVENTS.fetch_add(1, Ordering::Relaxed);\n        if seq < 1024 {\n", "        {\n")
s = s.replace("            crate::serial_println!(\n                \"[LINUX-STARTUP-TRACE]", """            if !path.as_deref().is_some_and(|b| b.starts_with(b"/proc"))
                && !matches!(original_linux_num, 41 | 99 | 217)
            {
                // Do not consume the trace budget for unrelated operations.
            } else if EVENTS.fetch_add(1, Ordering::Relaxed) < 1024 {
            let seq = EVENTS.load(Ordering::Relaxed);
            crate::serial_println!(
                "[LINUX-STARTUP-TRACE]""")
s = s.replace("            );\n        }\n    }\n\n'''", "            );\n            }\n        }\n    }\n\n'''")
s = s.replace("patch.write_text(''.join(difflib.unified_diff(s.splitlines(True),t.splitlines(True),f'a/{src}',f'b/{src}')))", """fault = Path("kernel/src/arch/x86_64/interrupts.rs")
old = fault.read_text()
anchor = '    serial_println!("[FAULT] Page Fault at'
i = old.index(anchor)
updated = old[:i] + '    serial_println!("[BTOP-02-FAULT] rip={:#x}", _stack_frame.instruction_pointer.as_u64());\\n' + old[i:]
patch.write_text(''.join(difflib.unified_diff(s.splitlines(True),t.splitlines(True),f'a/{src}',f'b/{src}')) +
                 ''.join(difflib.unified_diff(old.splitlines(True),updated.splitlines(True),f'a/{fault}',f'b/{fault}')))""")
p.write_text(s)
PY
bash "$out/image.sh" trace
sha256sum target/btop-01/btop "$out/"{procfs.elf,native-reference.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} \
    > "$out/final.sha256"
