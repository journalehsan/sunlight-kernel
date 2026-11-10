#!/usr/bin/env python3
"""Phase 5 host logic and native header/export checks, never host-run native ELFs.

Guest acceptance is a separate tools/native-c/guest.py --suite --cpus 2/4 run.
"""
import argparse
from collections import Counter
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out', type=Path, required=True)
options = parser.parse_args()
out = options.out.resolve()
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


# These two existing tests call native getuid/getgid, whose syscall numbers are
# unrelated Linux operations. Do not issue them in this host test process.
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
                '-c', 'tools/native-c/tests/unistd-abi.c', '-o', out / (name + '.o')])
symbols = run('native-symbols', ['llvm-nm', '-g', '--defined-only',
    'target/native-c/sysroot/lib/libsunlight_native_c.a'])
counts = Counter(line.split()[-1] for line in symbols.decode().splitlines() if len(line.split()) == 3)
for symbol in ('write', 'atexit', 'exit', '_start', 'sl_std_write', 'sl_std_exit', 'lseek', 'mkstemp', 'tmpfile'):
    if counts[symbol] != 1:
        sys.exit(f'{symbol}: expected one canonical definition, observed {counts[symbol]}')
print('libc host logic: PASS (see libc.log; two native-credential tests skipped)')
print('native runtime, C11/C++17 feature macros/include orders and unique exports: PASS')
print('guest: NOT_RUN by this host/build runner; use guest.py --suite on 2 and 4 CPUs')
