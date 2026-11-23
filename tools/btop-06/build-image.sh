#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/btop-06
mkdir -p "$out"
sha256sum --check target/btop-04/final.sha256
musl-gcc -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie -pthread \
    tools/btop-06/masked-select.c -o "$out/masked-select.elf"
cp "$out/masked-select.elf" "$out/masked-select-guest.elf"
bash tools/stamp_helios_elf.sh "$out/masked-select-guest.elf"
cp target/btop-05/masked-select-guest.elf "$out/masked-select-original.elf"
cp target/helios-sig01/sig01-guest.elf "$out/sig01-guest.elf"
sed 's|out="\$PWD/target/btop-04"|out="$PWD/target/btop-06"|; s|target/btop-04/vfs-config-fixture.elf|target/btop-06/vfs-config-fixture.elf|; /RamEntry::dir("\/tmp\/btopfixture"/a\    RamEntry::file("/bin/masked-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-06/masked-select-guest.elf")),\n    RamEntry::file("/bin/masked-select-original", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-06/masked-select-original.elf")),\n    RamEntry::file("/bin/sig01", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-06/sig01-guest.elf")),\n    RamEntry::file("/bin/tty-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-03/tty-select.elf")),' \
    target/btop-04/image.sh > "$out/image.sh"
# Place only the two test binaries' waiter and sender on different vCPUs.
# Apply the placement inside the generated image recipe's reversible trace patch.
python3 - "$out/image.sh" <<'PY2'
import sys
from pathlib import Path
recipe = Path(sys.argv[1])
text = recipe.read_text()
anchor = 't=s.replace(loc,record+loc)'
assert text.count(anchor) == 1
text = text.replace(anchor, anchor + """
clone = '    let idx = sched.publish_thread(slot, child, crate::sched::current_cpu_id());'
start = t.index('fn sys_linux_clone(')
i = t.index(clone, start)
placement = '''    if sched.online_cores >= 2 && matches!(
        sched.current_process().name_str(), "masked-select" | "masked-select-original"
    ) {
        sched.current_process_mut().cpu_mask = 1;
        child.cpu_mask = 2;
        crate::sched::request_park_after_sysret();
    }
'''
t = t[:i] + placement + t[i:]
""")
recipe.write_text(text)
PY2
SUNLIGHT_BTOP06_WAIT_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 bash "$out/image.sh" trace
sha256sum target/btop-01/btop "$out/"{masked-select.elf,masked-select-guest.elf,masked-select-original.elf,sig01-guest.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} > "$out/artifacts.sha256"
