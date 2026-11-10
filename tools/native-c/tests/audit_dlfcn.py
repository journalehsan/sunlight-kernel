#!/usr/bin/env python3
"""Reproduce Phase 10's header and link blockers without supplying dl* stubs."""
import argparse
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
(out / '.gitignore').write_text('*\n!*.log\n!.gitignore\n!diagnostic-include/\n')
tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
before = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest()
          for p in tracked if p and (ROOT / p).is_file()}


def run(name, argv, expected=0):
    result = subprocess.run(list(map(str, argv)), cwd=ROOT, capture_output=True, timeout=240)
    data = result.stdout + result.stderr
    (out / (name + '.log')).write_text('$ ' + shlex.join(list(map(str, argv))) + '\n' +
        data.decode(errors='replace') + '\nexit=' + str(result.returncode) + '\n')
    assert result.returncode == expected, (name, result.returncode, expected)
    return data


source = ROOT / 'docs/audit/probes/dlfcn.c'
(out / 'source.log').write_text(before[str(source.relative_to(ROOT))] + '  docs/audit/probes/dlfcn.c\n' + source.read_text())
run('build-runtime', ['bash', 'tools/build_native_c.sh'])
command = [sys.executable, 'tools/native-c/cc.py', '-std=c11', '-Wall', '-Wextra', '-Werror', source]
assert b"'dlfcn.h' file not found" in run('missing-header', command + ['-o', out / 'dlfcn.elf'], 1)
include = out / 'diagnostic-include'
include.mkdir()
# Diagnostic declarations only. This directory is never installed in the sysroot.
(include / 'dlfcn.h').write_text('''/* Diagnostic only: no native capability is advertised. */
#ifndef PHASE10_DIAGNOSTIC_DLFCN_H
#define PHASE10_DIAGNOSTIC_DLFCN_H
#define RTLD_NOW 2
#ifdef __cplusplus
extern "C" {
#endif
void *dlopen(const char *, int);
void *dlsym(void *, const char *);
int dlclose(void *);
char *dlerror(void);
#ifdef __cplusplus
}
#endif
#endif
''')
run('header-resolved-object', command + ['-I', include, '-c', '-o', out / 'dlfcn.o'])
undefined = run('object-undefined', ['llvm-nm', '--undefined-only', out / 'dlfcn.o'])
for symbol in (b'dlopen', b'dlsym', b'dlclose'):
    assert symbol in undefined
link = run('header-resolved-link', command + ['-I', include, '-o', out / 'dlfcn.elf'], 1)
for symbol in (b'dlopen', b'dlsym', b'dlclose'):
    assert b'undefined symbol: ' + symbol in link
assert not (out / 'dlfcn.elf').exists()
symbols = run('runtime-symbols', ['llvm-nm', '-g', '--defined-only', 'target/native-c/sysroot/lib/libsunlight_native_c.a'])
names = {line.split()[-1] for line in symbols.splitlines() if len(line.split()) == 3}
assert b'malloc' in names
assert not names.intersection({b'dlopen', b'dlsym', b'dlclose', b'dlerror'})
baseline = ROOT / 'docs/audit/probes/native-foundation/results-2026-11-10-dirent-final-2cpu/dirent.elf'
run('baseline-elf', ['llvm-readelf', '-h', '-l', '-S', '-s', '-r', baseline])
run('elf-host-tests', ['cargo', 'test', '-p', 'sunlight-elf', '--target', 'x86_64-unknown-linux-gnu'])
changed = [p for p, digest in before.items() if not (ROOT / p).is_file() or hashlib.sha256((ROOT / p).read_bytes()).hexdigest() != digest]
assert not changed, changed
(out / 'preservation.log').write_text(f'{len(before)} pre-existing tracked files unchanged during audit.\n')
(out / 'verdict.log').write_text('Case C: successful main-program symbol resolution required; native runtime resolver absent.\n'
    'No shared object requested by historical source. No implementation or successful ELF.\n'
    'Guest 2/4 CPU, dl* host/ABI behavior, and complete suite reruns: NOT_RUN (stop at capability blocker).\n'
    'Baseline counts unchanged. Phase 8 hardware write-fault and forced stale-TLB tests remain NOT_RUN.\n')
print('Phase 10: genuine native dynamic-symbol-loader blocker; diagnostic checks PASS')
