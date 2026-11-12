#!/usr/bin/env python3
"""Phase 22 class linkage gate; does not claim exception execution acceptance."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
from collections import Counter
ROOT = Path(__file__).resolve().parents[3]
p = argparse.ArgumentParser()
p.add_argument('--out', type=Path, required=True)
out = p.parse_args().out.resolve()
out.mkdir(parents=True, exist_ok=False)
def run(name, args, expected=0, data=None):
    args = list(map(str, args))
    r = subprocess.run(args, cwd=ROOT, input=data, capture_output=True, timeout=240)
    (out/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+r.stdout+r.stderr+('\nexit='+str(r.returncode)+'\n').encode())
    assert r.returncode == expected, name
    return r.stdout+r.stderr
run('build', ['bash', 'tools/build_native_cxx.sh'])
provenance = ROOT/'sunlight-cxx/vendor/libcxxabi'
manifest = json.loads((provenance/'PROVENANCE.json').read_text())
for f, digest in manifest['files'].items():
    assert hashlib.sha256((provenance/f).read_bytes()).hexdigest() == digest, f
for standard in ('c++14', 'c++17', 'c++20'):
    run('headers-'+standard, ['python3','tools/native-c/cxx.py','-std='+standard,'-Wall','-Wextra','-Werror','-fsyntax-only','-x','c++','-'], data=b'#include <exception>\n#include <stdexcept>\n#include <typeinfo>\nstatic_assert(__EXCEPTIONS && __GXX_RTTI, "enabled");\n')
lib = ROOT/'target/native-c/sysroot/lib/libsunlight_cxx.a'
exports = run('exports', ['llvm-nm','-g','--defined-only',lib])
counts=Counter(line.split()[-1] for line in exports.splitlines() if len(line.split())==3)
for symbol in (b'_ZTVN10__cxxabiv117__class_type_infoE',b'_ZTVN10__cxxabiv120__si_class_type_infoE',b'_ZdlPvm',b'_ZdlPv',b'__dynamic_cast'):
    assert counts[symbol] == 1, symbol
elf=out/'classes.elf'
run('link', ['python3','tools/native-c/cxx.py','-Wall','-Wextra','-Werror','tools/native-c/tests/cxx-exception-class.cpp','-Wl,--wrap=malloc,--wrap=free','-Wl,-Map='+str(out/'classes.map'),'-o',elf])
assert not run('undefined',['llvm-nm','-u',elf]).strip()
info=run('elf',['llvm-readelf','-h','-l','-d',elf])
assert b'INTERP' not in info and b'NEEDED' not in info
mapping=(out/'classes.map').read_text()
assert '/usr/lib/' not in mapping and '/usr/local/lib/' not in mapping
for obj in ('exception','private_typeinfo','stdlib_typeinfo'):
    run(obj+'-relocations',['llvm-readelf','-r','-s','-S',ROOT/'target/native-cxx'/(obj+'.o')])
run('assembly',['llvm-objdump','-dr','--demangle',ROOT/'target/native-cxx/exception.o'])
source=out/'lifecycle.cpp'
source.write_text('#include <string>\n#include <typeinfo>\nstruct Base { virtual ~Base() = default; }; struct Derived : Base {};\nstd::string initialized("static initialization OK");\nthread_local std::string local("TLS OK");\nint main() { Base* b = new Derived; bool ok = dynamic_cast<Derived*>(b) && typeid(*b)==typeid(Derived); delete b; return !ok + local.size(); }\n')
run('lifecycle-compile',['python3','tools/native-c/cxx.py','-c',source,'-o',out/'lifecycle.o'])
run('lifecycle-symbols',['llvm-nm','-u',out/'lifecycle.o'])
run('lifecycle-relocations',['llvm-readelf','-S','-s','-r',out/'lifecycle.o'])
run('lifecycle-assembly',['llvm-objdump','-dr','--demangle',out/'lifecycle.o'])
run('lifecycle-link',['python3','tools/native-c/cxx.py',out/'lifecycle.o','-o',out/'lifecycle.elf'])
probe=ROOT/'docs/audit/probes/cpp-runtime.cpp'
assert probe.read_bytes()==subprocess.check_output(['git','show','HEAD:docs/audit/probes/cpp-runtime.cpp'],cwd=ROOT)
(out/'historical.sha256').write_text(hashlib.sha256(probe.read_bytes()).hexdigest()+'\n')
print('Class target headers/linkage/ownership and upstream integrity PASS; native execution requires guest suite. EH/thread/historical runtime NOT_RUN.')
