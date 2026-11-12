#!/usr/bin/env python3
"""Record actual target exception dependencies; never execute target ELFs on host."""
import argparse
import hashlib
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
p = argparse.ArgumentParser()
p.add_argument('--out', required=True, type=Path)
out = p.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)

def run(name, args, expected=0):
    r = subprocess.run(list(map(str, args)), cwd=ROOT, capture_output=True, timeout=240)
    (out / (name + '.log')).write_bytes(r.stdout + r.stderr)
    assert r.returncode == expected, (name, r.returncode)
    return r.stdout + r.stderr

probe = ROOT / 'docs/audit/probes/cpp-runtime.cpp'
assert probe.read_bytes() == subprocess.check_output(['git', 'show', 'HEAD:docs/audit/probes/cpp-runtime.cpp'], cwd=ROOT)
(out / 'historical.sha256').write_text(hashlib.sha256(probe.read_bytes()).hexdigest() + '\n')
diag = run('historical', ['clang++', '--target=x86_64-unknown-none-elf', '-nostdinc', '-I'+str(ROOT/'sunlight-libc/include'), '-nostdlib', '-fuse-ld=lld', probe, '-o', out/'historical.elf'], 1)
assert b"undefined symbol" in diag and b"file not found" not in diag
obj = out / 'exception.o'
run('class-compile', ['python3', 'tools/native-c/cc.py', '-std=c++17', '-Wall', '-Wextra', '-Werror', '-I'+str(ROOT/'sunlight-cxx/include'), '-c', 'sunlight-cxx/src/exception.cpp', '-o', obj])
symbols = run('class-undefined', ['llvm-nm', '-u', obj])
for symbol in (b'_ZTVN10__cxxabiv117__class_type_infoE', b'_ZTVN10__cxxabiv120__si_class_type_infoE', b'_ZdlPvm'):
    assert symbol in symbols
src = out / 'throw.cpp'
src.write_text('#include <stdexcept>\nint main() { try { throw std::runtime_error("exception OK"); } catch (const std::exception& e) { return e.what()[0] != \'e\'; } }\n')
run('throw-compile', ['python3', 'tools/native-c/cc.py', '-std=c++17', '-I'+str(ROOT/'sunlight-cxx/include'), '-c', src, '-o', out/'throw.o'])
symbols = run('throw-undefined', ['llvm-nm', '-u', out/'throw.o'])
for symbol in (b'__cxa_allocate_exception', b'__cxa_throw', b'__cxa_begin_catch', b'__cxa_end_catch', b'__gxx_personality_v0'):
    assert symbol in symbols
run('throw-sections', ['llvm-readelf', '-S', out/'throw.o'])
print('Discovery PASS; historical LINK_FAIL (original command supplies no native libraries); class object ABI references recorded; native linkage uses class runner; runtime NOT_RUN')
