#!/usr/bin/env python3
"""Phase 7 production libc unit tests and native C/C++ ABI/export checks.

Native clock/sleep execution requires guest.py --suite.
"""
import argparse
from collections import Counter
import hashlib
from pathlib import Path
import shlex
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out', type=Path, required=True)
out = parser.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)
(out / '.gitignore').write_text('*\n!*.log\n!.gitignore\n')


def run(name, argv):
    argv = list(map(str, argv))
    result = subprocess.run(argv, cwd=ROOT, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=180)
    (out / (name + '.log')).write_text('$ ' + shlex.join(argv) + '\n' +
        result.stdout.decode(errors='replace') + '\nexit=' + str(result.returncode) + '\n')
    if result.returncode:
        sys.exit(f'{name}: FAIL; see {out / (name + ".log")}')
    return result.stdout


# These existing tests issue native getuid/getgid, not Linux host syscalls.
run('libc', ['cargo', 'test', '-p', 'sunlight-libc', '--lib', '--target',
    'x86_64-unknown-linux-gnu', '--', '--test-threads=1',
    '--skip', 'secret_store::tests::default_options_are_private_and_bounded',
    '--skip', 'secret_store::tests::path_policy_rejects_traversal_and_staging_names'])
run('build-runtime', ['bash', 'tools/build_native_c.sh'])
resource = subprocess.check_output(['clang', '-print-resource-dir'], text=True).strip()
for language, compiler, standard in [('c', 'clang', 'c11'), ('c++', 'clang++', 'c++17')]:
    for feature, macro in [('base', []), ('posix', ['-D_POSIX_C_SOURCE=200809L']), ('gnu', ['-D_GNU_SOURCE'])]:
        for reverse in (False, True):
            name = language + '-' + feature + ('-reverse' if reverse else '')
            run(name, [compiler, '--target=x86_64-unknown-none-elf', '-std=' + standard,
                '-Wall', '-Wextra', '-Werror', '-ffreestanding', '-nostdinc',
                '-isystem', resource + '/include', '-I', ROOT / 'sunlight-libc/include',
                *macro, *(['-DREVERSE_INCLUDES'] if reverse else []), '-x', language,
                '-c', 'tools/native-c/tests/time-abi.c', '-o', out / (name + '.o')])
symbols = run('native-symbols', ['llvm-nm', '-g', '--defined-only',
    'target/native-c/sysroot/lib/libsunlight_native_c.a'])
counts = Counter(line.split()[-1] for line in symbols.decode().splitlines() if len(line.split()) == 3)
for symbol in ('clock_gettime', 'nanosleep', 'sl_std_clock', 'sl_std_futex', 'getrandom', 'write', 'atexit', 'exit', '_start', 'sl_std_entropy', 'mkstemp', 'tmpfile'):
    if counts[symbol] != 1:
        sys.exit(f'{symbol}: expected one canonical definition, observed {counts[symbol]}')
source = ROOT / 'docs/audit/probes/clock.c'
(out / 'historical-source.log').write_text(hashlib.sha256(source.read_bytes()).hexdigest() +
    '  docs/audit/probes/clock.c\n' + source.read_text())
elf = out / 'clock.elf'
run('historical-build', [sys.executable, 'tools/native-c/cc.py', '-std=c11',
    '-Wall', '-Wextra', '-Werror', source, '-o', elf])
header = run('historical-elf', ['llvm-readelf', '-h', '-l', elf])
if run('historical-undefined', ['llvm-nm', '--undefined-only', elf]).strip():
    sys.exit('historical ELF has unresolved symbols')
symbols = run('historical-symbols', ['llvm-nm', '--defined-only', elf])
addresses = {line.split()[-1]: int(line.split()[0], 16)
             for line in symbols.splitlines() if len(line.split()) == 3}
data = elf.read_bytes()
assert data[:8] == b'\x7fELF\x02\x01\x01\x00' and data[9:15] != b'HLNX01'
assert int.from_bytes(data[16:18], 'little') == 2
assert int.from_bytes(data[18:20], 'little') == 62
entry = int.from_bytes(data[24:32], 'little')
assert entry == addresses.get(b'_start', 0) != 0
assert b'INTERP' not in header and b'DYNAMIC' not in header
phoff = int.from_bytes(data[32:40], 'little')
phsize = int.from_bytes(data[54:56], 'little')
phnum = int.from_bytes(data[56:58], 'little')
assert phsize == 56
executable_entry = False
for i in range(phnum):
    kind, flags, offset, vaddr, _, filesz, memsz, align = struct.unpack_from(
        '<IIQQQQQQ', data, phoff + i * phsize)
    if kind == 1:
        assert filesz <= memsz and offset + filesz <= len(data)
        assert flags & 3 != 3  # no writable executable LOAD
        assert align == 4096 and offset % align == vaddr % align
        executable_entry |= bool(flags & 1 and vaddr <= entry < vaddr + filesz)
    if kind == 0x6474e551:
        assert not flags & 1  # non-executable stack
assert executable_entry
(out / 'historical-validation.log').write_text(
    'native ELF64 x86-64 ET_EXEC; canonical entry; no interpreter/dynamic/unresolved symbols; W^X: PASS\n'
    'sha256=' + hashlib.sha256(data).hexdigest() + '\n')
run('historical-disassembly', ['llvm-objdump', '-d', elf])
print('libc host tests, 12 C/C++ ABI checks and unique native exports: PASS')
print('unchanged historical source: native compile/link PASS; host execution NOT_RUN')
