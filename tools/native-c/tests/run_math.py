#!/usr/bin/env python3
"""Validate the native math ABI and unchanged historical ELF; guests are separate."""
import argparse
from collections import Counter
import hashlib
from pathlib import Path
import shlex
import subprocess

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out', type=Path, required=True)
out = parser.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)
(out / '.gitignore').write_text('*\n!*.log\n!.gitignore\n')


def run(name, command):
    command = list(map(str, command))
    result = subprocess.run(command, cwd=ROOT, capture_output=True, timeout=240)
    (out / (name + '.log')).write_text('$ ' + shlex.join(command) + '\n' +
        (result.stdout + result.stderr).decode(errors='replace') +
        '\nexit=' + str(result.returncode) + '\n')
    assert result.returncode == 0, name
    return result.stdout


run('build-runtime', ['bash', 'tools/build_native_c.sh'])
resource = subprocess.check_output(['clang', '-print-resource-dir'], text=True).strip()
for compiler, language, standard in [('clang', 'c', 'c11'), ('clang++', 'c++', 'c++17')]:
    for feature, defines in [('base', []), ('posix', ['-D_POSIX_C_SOURCE=200809L']), ('gnu', ['-D_GNU_SOURCE'])]:
        for reverse in (False, True):
            name = language + '-' + feature + ('-reverse' if reverse else '')
            run(name, [compiler, '--target=x86_64-unknown-none-elf', '-std=' + standard,
                '-Wall', '-Wextra', '-Werror', '-ffreestanding', '-nostdinc',
                '-isystem', resource + '/include', '-I', ROOT / 'sunlight-libc/include',
                *defines, *(['-DREVERSE_INCLUDES'] if reverse else []), '-x', language,
                '-c', ROOT / 'tools/native-c/tests/math-abi.c', '-o', out / (name + '.o')])
symbols = run('native-exports', ['llvm-nm', '-g', '--defined-only',
    ROOT / 'target/native-c/sysroot/lib/libsunlight_native_c.a'])
counts = Counter(line.split()[-1] for line in symbols.decode().splitlines() if len(line.split()) == 3)
for symbol in ('sqrt', 'sqrtf', 'fabs', 'fabsf'):
    assert counts[symbol] == 1, symbol
source = ROOT / 'docs/audit/probes/math.c'
assert hashlib.sha256(source.read_bytes()).hexdigest() == '50b362bbac72b4a16a66aed9ece0a13e43bb8c87786d99ec57cf08a23c28feff'
(out / 'historical-source.log').write_text(hashlib.sha256(source.read_bytes()).hexdigest() + '\n' + source.read_text())
run('historical-object', ['python3', ROOT / 'tools/native-c/cc.py', '-std=c11',
    '-Wall', '-Wextra', '-Werror', '-c', source, '-o', out / 'math.o'])
undefined = run('historical-required-symbols', ['llvm-nm', '--undefined-only', out / 'math.o'])
assert {line.split()[-1] for line in undefined.decode().splitlines()} == {'sqrt', 'fabs'}
elf = out / 'math.elf'
run('historical-build', ['python3', ROOT / 'tools/native-c/cc.py', '-std=c11',
    '-Wall', '-Wextra', '-Werror', source, '-o', elf])
headers = run('historical-elf', ['llvm-readelf', '-h', '-l', '-s', elf])
assert b'INTERP' not in headers and b'DYNAMIC' not in headers
assert not run('historical-undefined', ['llvm-nm', '--undefined-only', elf]).strip()
run('historical-disassembly', ['llvm-objdump', '-d', elf])
(out / 'historical-hash.log').write_text(hashlib.sha256(elf.read_bytes()).hexdigest() + '\n')
run('contract-build', ['python3', ROOT / 'tools/native-c/cc.py', '-std=c11',
    '-Wall', '-Wextra', '-Werror', '-O2', '-fno-builtin',
    ROOT / 'tools/native-c/tests/math.c', '-o', out / 'math-contract.elf'])
run('contract-disassembly', ['llvm-objdump', '-d', out / 'math-contract.elf'])
print('12 native C/C++ ABI checks, unique export and unchanged historical link: PASS')
