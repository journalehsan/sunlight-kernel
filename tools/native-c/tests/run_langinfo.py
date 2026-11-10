#!/usr/bin/env python3
"""Validate the native CODESET ABI and unchanged historical ELF; guests are separate."""
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
                '-c', ROOT / 'tools/native-c/tests/langinfo-abi.c', '-o', out / (name + '.o')])
symbols = run('native-exports', ['llvm-nm', '-g', '--defined-only',
    ROOT / 'target/native-c/sysroot/lib/libsunlight_native_c.a'])
counts = Counter(line.split()[-1] for line in symbols.decode().splitlines() if len(line.split()) == 3)
assert counts['nl_langinfo'] == 1
assert counts['setlocale'] == 1
for unsupported in ('nl_langinfo_l', 'newlocale', 'uselocale'):
    assert not counts[unsupported], unsupported
source = ROOT / 'docs/audit/probes/langinfo.c'
(out / 'historical-source.log').write_text(hashlib.sha256(source.read_bytes()).hexdigest() + '\n' + source.read_text())
elf = out / 'langinfo.elf'
run('historical-build', ['python3', ROOT / 'tools/native-c/cc.py', '-std=c11',
    '-Wall', '-Wextra', '-Werror', source, '-o', elf])
headers = run('historical-elf', ['llvm-readelf', '-h', '-l', '-s', elf])
assert b'INTERP' not in headers and b'DYNAMIC' not in headers
assert not run('historical-undefined', ['llvm-nm', '--undefined-only', elf]).strip()
run('historical-disassembly', ['llvm-objdump', '-d', elf])
(out / 'historical-hash.log').write_text(hashlib.sha256(elf.read_bytes()).hexdigest() + '\n')
print('12 native C/C++ ABI checks, unique export and unchanged historical link: PASS')
