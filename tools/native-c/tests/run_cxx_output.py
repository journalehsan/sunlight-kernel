#!/usr/bin/env python3
"""Native-only header, archive ownership and ELF checks; runtime gate is guest.py."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
ROOT = Path(__file__).resolve().parents[3]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--out', required=True, type=Path)
out = p.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)
(out/'.gitignore').write_text('*\n!*.log\n!*.json\n!.gitignore\n')
def run(name, cmd, expected=0, data=None):
    cmd = list(map(str, cmd))
    r = subprocess.run(cmd, cwd=ROOT, input=data, capture_output=True, timeout=240)
    (out/(name+'.log')).write_text('$ '+shlex.join(cmd)+'\n'+(r.stdout+r.stderr).decode(errors='replace')+'\nexit='+str(r.returncode)+'\n')
    assert (r.returncode == 0) if expected == 0 else (r.returncode != 0), name
    return r.stdout + r.stderr
run('build', ['bash','tools/build_native_cxx.sh'])
for std in ('c++14', 'c++17', 'c++20'):
    run('header-'+std, ['python3','tools/native-c/cxx.py','-std='+std,'-Wall','-Wextra','-Werror',
        '-fsyntax-only','-x','c++','-'], data=b'#include <iostream>\n#include <stdio.h>\nstatic_assert(sizeof(long)==8, "LP64");\nstatic_assert(sizeof(long long)==8, "64-bit");\nstatic_assert(__EXCEPTIONS && __GXX_RTTI, "language features must remain enabled");\n')
# Exactly the historical include-path rules, without compiler resource headers.
run('historical-header', ['clang++','--target=x86_64-unknown-none-elf','-nostdinc','-I',ROOT/'sunlight-libc/include','-fsyntax-only','-x','c++','-'],data=b'#include <iostream>\n')
lib=ROOT/'target/native-c/sysroot/lib/libsunlight_cxx.a'
nm=run('archive-symbols',['llvm-nm','-g','--defined-only',lib])
counts=Counter(line.split()[-1] for line in nm.decode().splitlines() if len(line.split())==3)
for symbol in ('_ZSt4cout','_ZSt4cerr','_ZSt4clog'):
    assert counts[symbol]==1, symbol
libc=run('libc-symbols',['llvm-nm','-g','--defined-only',ROOT/'target/native-c/sysroot/lib/libsunlight_native_c.a'])
libc_symbols={line.split()[-1] for line in libc.decode().splitlines() if len(line.split())==3}
assert not (set(counts)&libc_symbols), 'C/C++ archive ownership collision'
for name in ('cxx-output','cxx-backend'):
    elf=out/(name+'.elf')
    run('link-'+name,['python3','tools/native-c/cxx.py','-Wall','-Wextra','-Werror',ROOT/'tools/native-c/tests'/(name+'.cpp'),*(['-Wl,--wrap=write'] if name=='cxx-backend' else []),'-Wl,-Map='+str(out/(name+'.map')),'-o',elf])
    assert not run('undefined-'+name,['llvm-nm','--undefined-only',elf]).strip()
    info=run('elf-'+name,['llvm-readelf','-h','-l','-d',elf])
    assert b'INTERP' not in info and b'NEEDED' not in info
    mapping=(out/(name+'.map')).read_text()
    assert '/usr/lib/' not in mapping and '/usr/local/lib/' not in mapping
    assert 'libsunlight_cxx.a' in mapping and 'libsunlight_native_c.a' in mapping
source=ROOT/'docs/audit/probes/cpp-runtime.cpp'
assert source.read_bytes()==subprocess.check_output(['git','show','HEAD:docs/audit/probes/cpp-runtime.cpp'],cwd=ROOT)
(out/'historical-sha256.json').write_text(json.dumps({'sha256':hashlib.sha256(source.read_bytes()).hexdigest()}))
diag=run('historical',['clang++','--target=x86_64-unknown-none-elf','-nostdinc','-I'+str(ROOT/'sunlight-libc/include'),'-nostdlib','-fuse-ld=lld',source,'-o',out/'historical.elf'],expected=1)
assert b"undefined symbol" in diag and b"file not found" not in diag
print('native output headers/archive ownership/ELF dependencies: PASS')
print('historical cpp-runtime: LINK_FAIL (original command supplies no native libraries); guest NOT_RUN')
print('output runtime: NOT_RUN by this runner; use guest.py --suite on both CPU configurations')
