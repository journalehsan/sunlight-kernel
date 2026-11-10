#!/usr/bin/env python3
"""Phase 6 production libc unit tests and native C/C++ ABI/export checks.

Native guest execution and service accounting require guest.py --suite.
"""
import argparse
from collections import Counter
import hashlib
from pathlib import Path
import shlex
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
                '-c', 'tools/native-c/tests/random-abi.c', '-o', out / (name + '.o')])
symbols = run('native-symbols', ['llvm-nm', '-g', '--defined-only',
    'target/native-c/sysroot/lib/libsunlight_native_c.a'])
counts = Counter(line.split()[-1] for line in symbols.decode().splitlines() if len(line.split()) == 3)
for symbol in ('getrandom', 'write', 'atexit', 'exit', '_start', 'sl_std_entropy', 'mkstemp', 'tmpfile'):
    if counts[symbol] != 1:
        sys.exit(f'{symbol}: expected one canonical definition, observed {counts[symbol]}')
source = ROOT / 'docs/audit/probes/getrandom.c'
(out / 'historical-source.log').write_text(hashlib.sha256(source.read_bytes()).hexdigest() +
    '  docs/audit/probes/getrandom.c\n' + source.read_text())
elf = out / 'getrandom.elf'
run('historical-build', [sys.executable, 'tools/native-c/cc.py', '-std=c11',
    '-Wall', '-Wextra', '-Werror', source, '-o', elf])
run('historical-elf', ['llvm-readelf', '-h', '-l', elf])
if run('historical-undefined', ['llvm-nm', '--undefined-only', elf]).strip():
    sys.exit('historical ELF has unresolved symbols')
run('historical-disassembly', ['llvm-objdump', '-d', elf])
print('libc host tests, 12 C/C++ ABI checks and unique native exports: PASS')
print('unchanged historical source: native compile/link PASS; host execution NOT_RUN')
