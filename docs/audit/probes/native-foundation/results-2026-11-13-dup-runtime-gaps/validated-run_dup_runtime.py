#!/usr/bin/env python3
"""Bounded target-only duplication coverage; never changes production ABI.

Every command is recorded with its external timeout and exit. Separate
assertion checks are recorded in summary.json, including suite rows.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess

ROOT = Path(__file__).resolve().parents[3]
p = argparse.ArgumentParser()
p.add_argument('--out', type=Path, required=True)
p.add_argument('--suite', action='store_true')
p.add_argument('--focused-only', action='store_true')
a = p.parse_args()
out = a.out.resolve()
out.mkdir(parents=True, exist_ok=False)
ledger, results = [], {}


def run(name, args, seconds=240):
    command = ['timeout', '--signal=TERM', '--kill-after=10', str(seconds), *map(str, args)]
    with (out / (name + '.log')).open('wb') as f:
        f.write(('$ ' + shlex.join(command) + '\n').encode())
        f.flush()
        r = subprocess.run(command, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT)
        f.write(('\nexit=' + str(r.returncode) + '\n').encode())
    ledger.append(dict(name=name, command=command, exit=r.returncode))
    (out / 'commands.json').write_text(json.dumps(ledger, indent=2))
    return r.returncode


def save():
    (out / 'summary.json').write_text(json.dumps(results, indent=2))


def required(name, args, seconds=240, phase='build'):
    rc = run(name, args, seconds)
    diagnostic = (out / (name + '.log')).read_text()
    results[name] = {phase: 'PASS' if rc == 0 else 'TIMEOUT' if rc == 124 else
                     'LINK_FAIL' if phase == 'link' and 'ld.lld: error:' in diagnostic else 'BUILD_FAIL'}
    if phase == 'link' and rc == 0:
        results[name]['build'] = 'PASS'
    save()
    if rc:
        raise SystemExit(name + ' failed; see preserved log')


required('native-build', ['bash', 'tools/build_native_c.sh'])
archive = ROOT / 'target/native-c/sysroot/lib/libsunlight_native_c.a'
required('archive-symbols', ['llvm-nm', '-A', '-g', '--defined-only', archive])
symbols = (out / 'archive-symbols.log').read_text()
assert not re.search(r'\b[ TW] (dup|dup2|dup3)$', symbols, re.M)
libname = re.search(r':(sunlight_libc-[^.]+)\.[^\n]+ T (\S+libc4dup2)\n', symbols)
assert libname, 'production wrapper member not found'
deps = ROOT / 'target/native-c/x86_64-sunlightos/release/deps'
rlib = deps / (libname[1].replace('sunlight_libc-', 'libsunlight_libc-', 1) + '.rlib')
release = re.search(r' T (\S+net_bridge16release_detached)\n', symbols)[1]
core, = deps.glob('libcore-*.rlib')
required('adapter-build', [ROOT / 'target/native-std/rustc', '--edition=2021',
    '--crate-type=lib', '--emit=obj', '-C', 'opt-level=0', '-C', 'panic=abort',
    '--target', ROOT / 'tools/targets/x86_64-sunlightos.json', '-Z', 'unstable-options',
    '-L', 'dependency=' + str(deps), '--extern', 'core=' + str(core),
    '--extern', 'sunlight_libc=' + str(rlib),
    ROOT / 'tools/native-c/tests/dup2-wrapper-adapter.rs', '-o', out / 'adapter.o'])
cc = ['python3', ROOT / 'tools/native-c/cc.py', '-Wall', '-Wextra', '-Werror']
for name, src, flags in [
    ('wrapper', 'dup-runtime.c', ['-DWRAPPER_TEST', '-DRELEASE_SYMBOL="' + release + '"',
        out / 'adapter.o', '-Wl,--wrap=' + release]),
    ('linux-handler', 'dup-runtime.c', []),
    ('linux-entry', 'dup3-linux-entry.c', []),
    *[(n, n + '.c', []) for n in ('dup2-replacement', 'epoll-lifecycle', 'epoll',
                                  'inet-socket', 'tcp-epoll-lt', 'tcp-epoll-revalidation')],
]:
    driver = cc
    if name == 'linux-entry':
        # A Linux actor uses Linux syscalls even at startup. The native C
        # startup cannot execute under that personality. Match the existing
        # raw Helios actors' static freestanding link model.
        driver = ['clang', '--target=x86_64-unknown-none-elf', '-mno-red-zone',
                  '-ffreestanding', '-fno-stack-protector', '-fno-pie',
                  '-nostdlib', '-static', '-fuse-ld=lld', '-Wall', '-Wextra', '-Werror',
                  '-Wl,-T,' + str(ROOT / 'services/user-space.ld')]
    required(name + '-link', [*driver, ROOT / 'tools/native-c/tests' / src, *flags,
             '-Wl,-Map=' + str(out / (name + '.map')), '-o', out / (name + '.elf')], phase='link')
# Explicit Linux ELF personality marker; no instruction changes.
f = out / 'linux-entry.elf'
b = bytearray(f.read_bytes())
assert b[7] == 0
b[7] = 3
f.write_bytes(b)
for name in ('adapter.o', 'wrapper.elf', 'linux-handler.elf', 'linux-entry.elf'):
    required(name + '-symbols', ['llvm-nm', '--demangle', out / name])
    required(name + '-assembly', ['llvm-objdump', '-dr', '--demangle', out / name])
    if name.endswith('.elf'):
        required(name + '-headers', ['llvm-readelf', '-h', '-l', '-d', out / name])
        required(name + '-undefined', ['llvm-nm', '-u', out / name])
        assert not (out / (name + '-undefined.log')).read_text().split('\n', 1)[1].split('exit=')[0].strip()
assert 'sunlight_libc::dup2' in (out / 'wrapper.elf-symbols.log').read_text()
assert 'sunlight_libc::net_bridge::lookup' in (out / 'wrapper.elf-symbols.log').read_text()

for cpu in (2, 4):
    for name in ('wrapper', 'linux-handler', 'linux-entry', 'dup2-replacement',
                 'epoll-lifecycle', 'epoll', 'inet-socket', 'tcp-epoll-lt', 'tcp-epoll-revalidation'):
        if a.focused_only and name not in ('wrapper', 'linux-handler', 'linux-entry'):
            continue
        key = name + '-' + str(cpu)
        args = ['python3', ROOT / 'tools/native-c/guest.py', out / (name + '.elf'),
                '--cpus', str(cpu), '--timeout', '110', '--expect-exit', '0', '--out', out / key]
        if name in ('wrapper', 'linux-handler', 'dup2-replacement', 'tcp-epoll-lt', 'tcp-epoll-revalidation'):
            args.append('--net')
        if name == 'linux-entry':
            args.append('--linux-actor')
        rc = run(key, args, 300)
        resultfile = out / key / 'results.json'
        rows = json.loads(resultfile.read_text()) if resultfile.exists() else {}
        stdout = (out / key / 'elf.stdout.txt').read_text() if (out / key / 'elf.stdout.txt').exists() else ''
        passes = len(re.findall(r'^.*: PASS ', stdout, re.M))
        fails = re.findall(r'^.*: FAIL .*$', stdout, re.M)
        serialpath = out / key / 'serial.log'
        serial = serialpath.read_text(errors='replace') if serialpath.exists() else ''
        inet = re.findall(r'\[NATIVE-C-INET\] before=(\d+) after=(\d+)', serial)
        cleanup = re.findall(r'\[NATIVE-C-RESULT\].*', serial)
        resources_ok = bool(inet) and all(x == y for x, y in inet) and bool(cleanup) and all('clean=1 ' in x for x in cleanup)
        success = rc == 0 and rows and all(v.get('pass_') for v in rows.values()) and not fails and resources_ok
        if name in ('wrapper', 'linux-handler'):
            success = success and 'dup-runtime: PASS complete' in stdout
        if name == 'linux-entry':
            success = success and 'linux-entry: PASS complete' in stdout
        results[key] = dict(runtime='PASS' if success else 'TIMEOUT' if rc == 124 else 'FAIL',
                            assertion_passes=passes, assertion_failures=fails, rows=rows,
                            inet_refs=inet, kernel_cleanup=cleanup)
        save()
    if a.suite:
        key = 'suite-' + str(cpu)
        rc = run(key, ['python3', ROOT / 'tools/native-c/guest.py', '--suite', '--cpus', str(cpu),
                      '--timeout', '180', '--out', out / key], 600)
        path = out / key / 'results.json'
        rows = json.loads(path.read_text()) if path.exists() else {}
        passed = sum(bool(v.get('pass_')) for v in rows.values())
        results[key] = dict(runtime='PASS' if rc == 0 and len(rows) == 94 and passed == 94 else
                            'TIMEOUT' if rc == 124 else 'FAIL', total=len(rows), passed=passed, rows=rows)
        save()
if not a.focused_only:
    required('cxx-checker', ['python3', ROOT / 'tools/native-c/tests/run_cxx_runtime.py', '--out', out / 'cxx'], 360)
    required('poll-abi', ['python3', ROOT / 'tools/native-c/tests/run_poll.py', '--out', out / 'poll-abi'], 300)
required('kernel-check', ['cargo', 'check', '-p', 'sunlight-kernel', '--locked'])
required('libc-check', ['cargo', 'check', '-p', 'sunlight-libc', '--features', 'native-c', '--locked'])
required('network-check', ['cargo', 'check', '-p', 'sunlight-net', '-p', 'sunlight-ipc', '-p', 'sunlight-net-server', '--locked'])
required('diff-check', ['git', 'diff', '--check'])
files = [archive, ROOT / 'target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a', rlib,
         core, ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel', *out.glob('*.elf'), out / 'adapter.o']
(out / 'hashes.json').write_text(json.dumps({str(f):hashlib.sha256(f.read_bytes()).hexdigest()
                                          for f in files if f.exists()}, indent=2))
print(json.dumps({k:{f:v for f,v in row.items() if f != 'rows'} for k,row in results.items()}, indent=2))
