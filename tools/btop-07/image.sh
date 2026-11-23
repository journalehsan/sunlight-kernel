#!/usr/bin/env bash
set -euxo pipefail
out="$PWD/target/btop-07/final"
file=kernel/src/arch/x86_64/syscall.rs
python3 - "$file" "$out/trace-only.patch" <<'PY'
import difflib,sys
from pathlib import Path
src,patch=map(Path,sys.argv[1:])
s=src.read_text()
loc='    if linux_compat\n        && option_env!("SUNLIGHT_INJECT_PHASE") == Some("yazi-phase1")\n        && original_linux_num == 91'
assert s.count(loc)==1
record='''    if linux_compat && option_env!("SUNLIGHT_LINUX_STARTUP_TRACE") == Some("1") {
        use core::sync::atomic::{AtomicUsize, Ordering};
        static EVENTS: AtomicUsize = AtomicUsize::new(0);
        static SELECT_EVENTS: AtomicUsize = AtomicUsize::new(0);
        {
            let pid = sched::with_scheduler(|s| s.current_process().pid);
            let path_ptr = match original_linux_num {
                2 | 4 | 6 | 21 | 83 | 89 | 137 => Some(original_args[0]),
                257 | 258 | 262 | 267 | 269 | 332 | 439 => Some(original_args[1]),
                _ => None,
            };
            let path = path_ptr.and_then(|ptr| read_user_cstr(ptr, USER_PATH_MAX).ok());
            if original_linux_num == 270 && SELECT_EVENTS.fetch_add(1, Ordering::Relaxed) >= 16 {
                // Keep repeated masked waits out of the discovery trace budget.
            } else if !path.as_deref().is_some_and(|b| b.starts_with(b"/proc") || b.starts_with(b"/sys/class/net") || b.starts_with(b"/sys/dev/block") || b == b"/mnt")
                && !matches!(original_linux_num, 41 | 44 | 45 | 46 | 47 | 99 | 217 | 270)
            {
                // Do not consume the trace budget for unrelated operations.
            } else {
            let seq = EVENTS.fetch_add(1, Ordering::Relaxed) + 1;
            if seq <= 1024 {
            crate::serial_println!(
                "[LINUX-STARTUP-TRACE] seq={} pid={} nr={} a0={:#x} a1={:#x} a2={:#x} a3={:#x} result={} path={:?}",
                seq, pid, original_linux_num, original_args[0], original_args[1],
                original_args[2], original_args[3], result as i64,
                path.as_deref().and_then(|bytes| core::str::from_utf8(bytes).ok()),
            );
            }
            }
        }
    }

'''
t=s.replace(loc,record+loc)
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

fault = Path("kernel/src/arch/x86_64/interrupts.rs")
old = fault.read_text()
anchor = '    serial_println!("[FAULT] Page Fault at'
i = old.index(anchor)
updated = old[:i] + '    serial_println!("[BTOP-02-FAULT] rip={:#x}", _stack_frame.instruction_pointer.as_u64());\n' + old[i:]
patch.write_text(''.join(difflib.unified_diff(s.splitlines(True),t.splitlines(True),f'a/{src}',f'b/{src}')) +
                 ''.join(difflib.unified_diff(old.splitlines(True),updated.splitlines(True),f'a/{fault}',f'b/{fault}')))
PY
python3 - "$out/ramfs-config-package.patch" <<'PY2'
import difflib, sys
from pathlib import Path
result=[]
def edit(file, before, after):
    old=Path(file).read_text()
    assert old.count(before)==1
    new=old.replace(before,after)
    result.extend(difflib.unified_diff(old.splitlines(True),new.splitlines(True),f'a/{file}',f'b/{file}'))
anchor='pub static INITRAMFS: &[RamEntry] = &[\n'
edit('sunlight-fs/src/ramfs.rs',anchor,anchor+'''    RamEntry::dir("/tmp/btopfixture", 0, 0, mode::DIR_700),
    RamEntry::file("/bin/masked-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-06/masked-select-guest.elf")),
    RamEntry::file("/bin/masked-select-original", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-06/masked-select-original.elf")),
    RamEntry::file("/bin/sig01", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-06/sig01-guest.elf")),
    RamEntry::file("/bin/tty-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-03/tty-select.elf")),
    RamEntry::file("/bin/storage-probe", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-07/final/probe.elf")),
    RamEntry::dir("/tmp/btopfixture/btop", 0, 0, mode::DIR_700),
    RamEntry::file("/tmp/btopfixture/btop/btop.conf", 0, 0, mode::FILE_600, include_bytes!("../../tools/btop-07/btop.conf")),
    RamEntry::file("/bin/btop", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-01/btop")),
    RamEntry::file("/bin/ifaddrs", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-04/ifaddrs.elf")),
    RamEntry::file("/bin/route-scope", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-04/route-scope.elf")),
    RamEntry::file("/bin/net_server", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-04/publish-denied.elf")),
    RamEntry::file("/bin/proc-native", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-02/native-reference.elf")),
    RamEntry::file("/bin/btop-reference", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-02/procfs.elf")),
''')
edit('kernel/src/main.rs',
    '../../target/x86_64-unknown-none/release/sunlight-vfs-server',
    '../../target/btop-07/final/vfs-config-fixture.elf')
edit('kernel/src/main.rs',
    '../../target/x86_64-unknown-none/release/net_server',
    '../../target/btop-04/net_server')
Path(sys.argv[1]).write_text(''.join(result))
PY2
git apply "$out/ramfs-config-package.patch"
trap 'git apply -R "$out/ramfs-config-package.patch"' EXIT
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
    cargo build --target-dir target/embedded-build --release --package sunlight-vfs-server
cp target/embedded-build/x86_64-unknown-none/release/sunlight-vfs-server "$out/vfs-config-fixture.elf"
if [[ ${1:-} == trace ]]; then
    git apply "$out/trace-only.patch"
    trap 'git apply -R "$out/trace-only.patch"; git apply -R "$out/ramfs-config-package.patch"' EXIT
    export SUNLIGHT_LINUX_STARTUP_TRACE=1
    suffix=-trace-config
else
    unset SUNLIGHT_LINUX_STARTUP_TRACE
    suffix=
fi
cargo build --package sunlight-kernel --features native_document_viewer
cp target/x86_64-unknown-none/debug/sunlight-kernel "$out/kernel-ramfs$suffix.elf"
bash tools/make_hybrid_iso.sh "$out/kernel-ramfs$suffix.elf" "$out/guest-ramfs$suffix.iso" target/limine "$PWD"
