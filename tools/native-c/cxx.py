#!/usr/bin/env python3
"""Bounded native C++ driver with target-built LLVM ABI/unwind/guard support.
No host C++ headers, objects or runtime libraries are used.
"""
from pathlib import Path
import subprocess
import sys
ROOT = Path(__file__).resolve().parents[2]
args = sys.argv[1:]
link = bool(args) and not any(x in args for x in ('-E', '-S', '-c', '-fsyntax-only', '--version'))
if link:
    archive = ROOT / 'target/native-c/sysroot/lib/libsunlight_cxx.a'
    if not archive.is_file():
        sys.exit('run bash tools/build_native_cxx.sh first')
    args += [str(archive), str(archive.with_name('libsunlight_eh.a')),
             '-Wl,--wrap=main,--undefined=__wrap_main,--eh-frame-hdr']
sys.exit(subprocess.call([sys.executable, str(ROOT/'tools/native-c/cc.py'), '-std=c++17', '-fhosted',
                         '-I'+str(ROOT/'sunlight-cxx/include'), *args]))
