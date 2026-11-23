#!/usr/bin/env bash
set -euo pipefail
out=target/btop-08/final
mkdir -p "$out"
sed -e 's@target/btop-07/final@target/btop-08/final@g' \
    -e 's@tools/btop-07/btop.conf@tools/btop-08/btop.conf@g' \
    tools/btop-07/image.sh > "$out/base-image.sh"
python3 - "$out/evidence.patch" <<'PY'
import difflib
import sys
from pathlib import Path

patch = []
def edit(file, anchor, replacement):
    old = Path(file).read_text()
    assert old.count(anchor) == 1
    new = old.replace(anchor, replacement)
    patch.extend(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                      "a/" + file, "b/" + file))

anchor = '            let n = count.min(data.len()).min(bytes.len());\n'
edit("kernel/src/arch/x86_64/syscall_procfs.rs", anchor, anchor + '''
            if matches!(file.node, Node::MountInfo(_)) && start == 0 && n != 0 {
                crate::serial_println!("[BTOP08] read pid={} fd={} offset={} bytes={}",
                    s.current_process().pid, fd, start, n);
            }
''')
anchor = '                        let _ = vfs.set_mount_source(mountpoint, info.id);\n'
edit("kernel/src/main.rs", anchor, anchor + '''
                        let row = (0..sunlight_fs::vfs::MAX_MOUNTS)
                            .filter_map(|i| vfs.mount_info(i))
                            .find(|row| row.path == mountpoint).unwrap();
                        serial_println!("[BTOP08] VFS mount={} source={} parent_device={} start={} blocks={} path={}",
                            row.id, row.source, info.parent, info.start, info.blocks, row.path);
''')
Path(sys.argv[1]).write_text("".join(patch))
PY
git apply "$out/evidence.patch"
trap 'git apply -R "$out/evidence.patch"' EXIT
bash "$out/base-image.sh" trace
