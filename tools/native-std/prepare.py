#!/usr/bin/env python3
"""Stage the pinned Rust sources and SunlightOS std overlay without editing rustup."""
import pathlib
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
BASE = pathlib.Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
VERSION = subprocess.check_output(['rustc', '-Vv'], text=True)
if '8954863c8' not in VERSION:
    raise SystemExit('native std overlay requires rustc revision 8954863c8 (2026-06-05)')
STAGE = ROOT / 'target/native-std/sysroot'
LIBRARY = STAGE / 'lib/rustlib/src/rust/library'
STAGE.mkdir(parents=True, exist_ok=True)
for name in ('bin', 'libexec'):
    if (BASE / name).exists() and not (STAGE / name).exists():
        (STAGE / name).symlink_to(BASE / name, target_is_directory=True)
(STAGE / 'lib/rustlib').mkdir(parents=True, exist_ok=True)
for entry in (BASE / 'lib').iterdir():
    if entry.name != 'rustlib' and not (STAGE / 'lib' / entry.name).exists():
        (STAGE / 'lib' / entry.name).symlink_to(entry)
for entry in (BASE / 'lib/rustlib').iterdir():
    if entry.name != 'src' and not (STAGE / 'lib/rustlib' / entry.name).exists():
        (STAGE / 'lib/rustlib' / entry.name).symlink_to(entry)
if not LIBRARY.exists():
    shutil.copytree(BASE / 'lib/rustlib/src/rust/library', LIBRARY)

def replace(path, old, new):
    file = LIBRARY / path
    source = file.read_text()
    if new in source:
        return
    if source.count(old) != 1:
        raise SystemExit(f'Rust source drift: {path}: {old!r}')
    file.write_text(source.replace(old, new))

def select(path, body):
    replace('std/src/sys/' + path, 'cfg_select! {', 'cfg_select! {\n    target_os = "sunlightos" => {\n' + body + '\n    }')

select('pal/mod.rs', '        pub mod sunlightos;\n        pub use sunlightos::*;')
for module in ('alloc', 'random', 'thread', 'time', 'stdio', 'args', 'fs', 'env', 'paths'):
    exports = {'alloc': '', 'fs': 'use sunlightos as imp;', 'time': 'use sunlightos as imp;', 'paths': 'use sunlightos as imp;'}.get(module, 'pub use sunlightos::*;')
    select(module + '/mod.rs', f'        mod sunlightos;\n        {exports}')
select('io/error/mod.rs', '        mod sunlightos;\n        pub use sunlightos::*;')
replace('std/src/sys/thread_local/mod.rs', 'pub(crate) mod key {\n    cfg_select! {', '''pub(crate) mod key {
    cfg_select! {
        target_os = "sunlightos" => {
            mod racy;
            mod sunlightos;
            pub(super) use racy::LazyKey;
            pub(super) use sunlightos::{Key, get, set};
            pub(crate) use sunlightos::run_dtors;
            use sunlightos::{create, destroy};
        }''')
for lock in ('mutex', 'rwlock', 'condvar', 'once', 'thread_parking'):
    replace(f'std/src/sys/sync/{lock}/mod.rs', '        target_os = "linux",', '        target_os = "sunlightos",\n        target_os = "linux",')
replace('std/src/sys/args/mod.rs', '    target_family = "windows",', '    target_os = "sunlightos",\n    target_family = "windows",')
replace('std/build.rs', 'if target_os == "linux"', 'if target_os == "sunlightos" || target_os == "linux"')
replace('std/src/sys/path/unix.rs', 'any(unix, target_os = "hermit",',
        'any(unix, target_os = "sunlightos", target_os = "hermit",')
replace('std/src/sys/exit.rs', '    cfg_select! {', '''    cfg_select! {
        target_os = "sunlightos" => {
            unsafe { crate::sys::sunlightos::sl_std_exit(code) }
        }''')
for source in (ROOT / 'tools/native-std/overlay').rglob('*.rs'):
    destination = LIBRARY / 'std/src/sys' / source.relative_to(ROOT / 'tools/native-std/overlay')
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists() or destination.read_bytes() != source.read_bytes():
        shutil.copyfile(source, destination)
wrapper = ROOT / 'target/native-std/rustc'
wrapper_text = f'#!/bin/sh\nexec "{BASE}/bin/rustc" --sysroot "{STAGE}" "$@"\n'
if not wrapper.exists() or wrapper.read_text() != wrapper_text:
    wrapper.write_text(wrapper_text)
wrapper.chmod(0o755)
print(wrapper)
