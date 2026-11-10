#!/usr/bin/env python3
"""Build native ELFs and execute canonical stdio with host descriptor adapters.

Each run requires a new evidence directory. Never execute native ELFs on Linux.
The host C binary links the same stdio.rs with explicitly separate Linux I/O.
"""
import argparse
import csv
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out', type=Path, required=True)
options = parser.parse_args()
OUT = options.out.resolve()
OUT.mkdir(parents=True, exist_ok=False)
(OUT / '.gitignore').write_text('*\n!*.log\n!*.csv\n!.gitignore\n')

def run(name, argv, *, input=None, expected=0):
    result = subprocess.run(list(map(str, argv)), cwd=ROOT, input=input,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    (OUT / (name + '.log')).write_text(
        '$ ' + shlex.join(map(str, argv)) + '\n' +
        ('stdin=' + repr(input) + '\n' if input is not None else '') +
        'stdout:\n' + result.stdout.decode(errors='replace') +
        '\nstderr:\n' + result.stderr.decode(errors='replace') +
        '\nexit=' + str(result.returncode) + '\n')
    if result.returncode != expected:
        sys.exit(f'{name}: FAIL (exit {result.returncode}, expected {expected}); see {OUT / (name + ".log")}')
    return result

run('build-runtime', ['bash', 'tools/build_native_c.sh'])
run('host-vfs', ['cargo', 'test', '-p', 'sunlight-fs', '--lib', '--target',
    'x86_64-unknown-linux-gnu', '--', '--test-threads=1'])
run('host-fd-vfs', ['cargo', 'test', '--manifest-path', 'tools/tests/fd-description/Cargo.toml',
    '--target', 'x86_64-unknown-linux-gnu', '--', '--test-threads=1'])
run('host-tests-build', ['rustc', '--edition=2021', '--test', 'tools/native-c/tests/stdio-host.rs', '-o', OUT / 'stdio-tests'])
run('host-tests', [OUT / 'stdio-tests'])
run('host-adapter-build', ['rustc', '--edition=2021', '--crate-type=staticlib', 'tools/native-c/tests/stdio-host.rs', '-o', OUT / 'libstdio-host.a'])
flags = ['-std=c11', '-Wall', '-Wextra', '-Werror', '-fno-builtin']
run('host-c-build', ['clang', *flags, '-I', ROOT / 'sunlight-libc/include',
                     'tools/native-c/tests/stdio.c', OUT / 'libstdio-host.a', '-ldl', '-lpthread', '-lm', '-o', OUT / 'stdio-host'])
with tempfile.TemporaryDirectory(prefix='sunlight-stdio-') as directory:
    result = run('host-c-run', [OUT / 'stdio-host', Path(directory) / 'file'], input=b'Q\n')
expected_stdout = (b'stdio: native -42 42 0x2a -1234567890 7 %\n'
                   b'stdio: puts\n!\nstdio: vprintf V\nstdio: file PASS; tmpfile PASS\n')
if result.stdout != expected_stdout or result.stderr != b'stdio: stderr 0007\n':
    sys.exit('host-c-run: FAIL (output mismatch)')
# Execute the adapted historical tmpfile probe through the canonical FILE code.
run('host-original-build', ['clang', *flags, '-I', ROOT / 'sunlight-libc/include',
    'docs/audit/probes/stdio.c', OUT / 'libstdio-host.a', '-ldl', '-lpthread', '-lm', '-o', OUT / 'stdio-original-host'])
run('host-original-run', [OUT / 'stdio-original-host'])
rows = []
for name, source in [('stdio', 'tools/native-c/tests/stdio.c'), ('original-stdio', 'docs/audit/probes/stdio.c'),
                     ('open-unlink', 'tools/native-c/tests/open-unlink.c')]:
    binary = OUT / (name + '-native')
    run(name + '-native-build', [sys.executable, 'tools/native-c/cc.py', *flags, source, '-o', binary])
    header = run(name + '-elf', ['llvm-readelf', '-h', '-l', binary]).stdout
    undefined = run(name + '-undefined', ['llvm-nm', '--undefined-only', binary]).stdout
    symbols = run(name + '-symbols', ['llvm-nm', '--defined-only', binary]).stdout
    addresses = {line.split()[-1]: int(line.split()[0], 16) for line in symbols.splitlines() if len(line.split()) == 3}
    data = binary.read_bytes()
    assert data[:4] == b'\x7fELF' and data[7] == 0 and data[9:15] != b'HLNX01'
    assert int.from_bytes(data[16:18], 'little') == 2 and int.from_bytes(data[18:20], 'little') == 62
    assert int.from_bytes(data[24:32], 'little') == addresses.get(b'_start', 0) != 0
    assert b'INTERP' not in header and not undefined.strip()
    run(name + '-disassembly', ['llvm-objdump', '-d', binary])
    rows.append(dict(probe=name, native_build='PASS', elf='PASS', guest='NOT_RUN',
                     host_adapter='NOT_RUN: native syscall probe' if name == 'open-unlink' else 'PASS'))
run('native-symbols', ['llvm-nm', '-g', '--defined-only', 'target/native-c/sysroot/lib/libsunlight_native_c.a'])
run('cpp-header', ['clang++', '--target=x86_64-unknown-none-elf', '-std=c++17', '-nostdinc',
    '-isystem', subprocess.check_output(['clang', '-print-resource-dir'], text=True).strip() + '/include',
    '-I', ROOT / 'sunlight-libc/include', '-x', 'c++', '-fsyntax-only', '-'], input=b'#include <stdio.h>\n#include <errno.h>\n')
(OUT / 'guest.log').write_text('guest=NOT_RUN\nNo guest command issued by this host-only regression runner. Use tools/native-c/guest.py --suite; '
    'the isolated guest gate captures native exit/output/reaping. Host adapters replace only descriptor/allocator/errno calls and do not validate SunlightOS kernel I/O.\n'
    'Native probe requires argv[1]=disposable writable path, stdin=Q\\n, exit=0 and exact stdout/stderr from host-c-run.log.\n'
    'Historical stdio.c requires tmpfile; its host pass does not validate guest execution.\n'
    'open-unlink-native requires argv[1]=absent disposable path on tmpfs and exit=0.\n')
with (OUT / 'results.csv').open('w') as stream:
    writer = csv.DictWriter(stream, fieldnames=rows[0].keys())
    writer.writeheader()
    writer.writerows(rows)
print('stdio host tests: PASS (see host-tests.log for count)')
print('actual tmpfs/VFS and kernel descriptor host tests: PASS (see host-vfs.log, host-fd-vfs.log)')
print('stdio C host adapter: PASS (exit 0; exact stdout/stderr)')
print('historical tmpfile probe host adapter: PASS (exit 0)')
print('stdio/open-unlink native builds and ELF checks: PASS (3 probes)')
print('C++ headers: PASS')
print('guest execution: NOT_RUN (host-only runner; use tools/native-c/guest.py --suite)')
