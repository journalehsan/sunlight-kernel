#!/usr/bin/env python3
"""Compile the public ABI and inspect the native linked export index; no host dl* calls."""
import argparse
import hashlib
from pathlib import Path
import shlex
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[3]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--out', type=Path, required=True)
out = p.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)
(out / '.gitignore').write_text('*\n!*.log\n!.gitignore\n')
def run(name, args):
    args = list(map(str, args))
    r = subprocess.run(args, cwd=ROOT, capture_output=True)
    (out / (name + '.log')).write_text('$ ' + shlex.join(args) + '\n' + (r.stdout+r.stderr).decode(errors='replace') + '\nexit=' + str(r.returncode) + '\n')
    assert r.returncode == 0, name
    return r.stdout
source = ROOT / 'docs/audit/probes/dlfcn.c'
(out / 'source.log').write_text(hashlib.sha256(source.read_bytes()).hexdigest()+'\n'+source.read_text())
elf = out / 'dlfcn.elf'
run('historical-build', ['python3', 'tools/native-c/cc.py', '-Wall', '-Wextra', '-Werror', source, '-o', elf])
run('readelf', ['llvm-readelf', '-h', '-l', '-S', '-s', '-r', elf])
symbols = {}
for line in run('nm', ['llvm-nm', '--defined-only', elf]).decode().splitlines():
    fields = line.split()
    if len(fields) == 3:
        symbols[fields[2]] = int(fields[0], 16)
data = elf.read_bytes()
assert struct.unpack_from('<H', data, 16)[0] == 2
phoff = struct.unpack_from('<Q', data, 32)[0]
phsize, phnum = struct.unpack_from('<HH', data, 54)
start, end = symbols['__sl_exports_start'], symbols['__sl_exports_end']
loads = [struct.unpack_from('<IIQQQQQQ', data, phoff+i*phsize) for i in range(phnum)]
segment, = [s for s in loads if s[0] == 1 and s[3] <= start and end <= s[3]+s[5]]
assert segment[1] == 4, 'metadata must be read-only, non-executable PT_LOAD'
assert end-start == 128
index = data[segment[2]+start-segment[3]:segment[2]+end-segment[3]]
for offset, name in zip(range(0,128,32), ('malloc','free','calloc','realloc')):
    assert index[offset:offset+24] == name.encode().ljust(24,b'\0')
    address = struct.unpack_from('<Q', index, offset+24)[0]
    assert address == symbols[name]
    assert any(s[0] == 1 and s[1] == 5 and s[3] <= address < s[3]+s[5] for s in loads)
(out / 'index.log').write_text('ET_EXEC, zero load bias; four exact linked symbol addresses in read-only PT_LOAD; all targets in RX PT_LOAD. PASS\n')
resource = subprocess.check_output(['clang','-print-resource-dir'], text=True).strip()
for compiler, language, standard in [('clang','c','c11'), ('clang++','c++','c++17')]:
    run(language+'-abi', [compiler,'--target=x86_64-unknown-none-elf','-std='+standard,'-Wall','-Wextra','-Werror','-ffreestanding','-nostdinc','-isystem',resource+'/include','-I','sunlight-libc/include','-x',language,'-c','tools/native-c/tests/dlfcn.c','-o',out/(language+'.o')])
print('Public C/C++ ABI and linked export index: PASS; guest acceptance is separate')
