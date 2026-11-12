#!/usr/bin/env python3
"""Compile native termios C/C++ ABI and verify canonical exports; guest tests separate."""
import argparse
from collections import Counter
import hashlib
from pathlib import Path
import subprocess
import shlex
ROOT = Path(__file__).resolve().parents[3]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--out', required=True, type=Path)
out = p.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)
(out / '.gitignore').write_text('*\n!*.log\n!.gitignore\n')
def run(name, command):
    command = list(map(str, command))
    r = subprocess.run(command, cwd=ROOT, capture_output=True, timeout=240)
    (out / (name + '.log')).write_text('$ ' + shlex.join(command) + '\n' + (r.stdout+r.stderr).decode(errors='replace') + '\nexit=' + str(r.returncode) + '\n')
    assert r.returncode == 0, name
    return r.stdout
run('build', ['bash', 'tools/build_native_c.sh'])
resource = subprocess.check_output(['clang', '-print-resource-dir'], text=True).strip()
for compiler, lang, std in [('clang', 'c', 'c11'), ('clang++', 'c++', 'c++17')]:
    for feature, flags in [('base', []), ('posix', ['-D_POSIX_C_SOURCE=200809L']), ('gnu', ['-D_GNU_SOURCE'])]:
        for reverse in (False, True):
            name = lang + '-' + feature + str(int(reverse))
            run(name, [compiler, '--target=x86_64-unknown-none-elf', '-std='+std,
                '-Wall', '-Wextra', '-Werror', '-ffreestanding', '-nostdinc',
                '-isystem', resource+'/include', '-I', ROOT/'sunlight-libc/include',
                *flags, *(['-DREVERSE_INCLUDES'] if reverse else []), '-x', lang, '-c',
                ROOT/'tools/native-c/tests/termios-abi.c', '-o', out/(name+'.o')])
nm = run('exports', ['llvm-nm', '-g', '--defined-only', ROOT/'target/native-c/sysroot/lib/libsunlight_native_c.a'])
counts = Counter(line.split()[-1] for line in nm.decode().splitlines() if len(line.split()) == 3)
for symbol in ('tcgetattr', 'tcsetattr', 'cfgetispeed', 'cfgetospeed', 'cfsetispeed', 'cfsetospeed', 'tcdrain', 'tcflush', 'tcflow', 'tcsendbreak', 'isatty'):
    assert counts[symbol] == 1, symbol
source = ROOT/'docs/audit/probes/termios.c'
assert source.read_bytes() == subprocess.check_output(['git', 'show', 'HEAD:docs/audit/probes/termios.c'], cwd=ROOT)
(out/'historical-sha256.log').write_text(hashlib.sha256(source.read_bytes()).hexdigest()+'\n')
run('historical', ['python3', 'tools/native-c/cc.py', '-std=c11', '-Wall', '-Wextra', '-Werror', source, '-o', out/'termios.elf'])
assert not run('undefined', ['llvm-nm', '--undefined-only', out/'termios.elf']).strip()
print('12 termios C/C++ ABI checks, exports, unchanged historical link: PASS')
