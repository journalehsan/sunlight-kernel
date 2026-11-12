#!/usr/bin/env python3
"""Target-only Phase 22 ABI/unwind/image checks; guest.py executes the ELFs."""
import argparse, hashlib, json, shlex, subprocess
from pathlib import Path
from collections import Counter
ROOT=Path(__file__).resolve().parents[3]
p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True)
out=p.parse_args().out.resolve();out.mkdir(parents=True,exist_ok=False)
def run(name,args,expected=0,data=None):
 args=list(map(str,args));r=subprocess.run(args,cwd=ROOT,input=data,capture_output=True,timeout=240)
 (out/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+r.stdout+r.stderr+('\nexit='+str(r.returncode)+'\n').encode())
 assert r.returncode==expected,(name,r.returncode)
 return r.stdout+r.stderr
run('build',['bash','tools/build_native_cxx.sh'])
for vendor in ('libcxxabi','libunwind'):
 base=ROOT/'sunlight-cxx/vendor'/vendor;manifest=json.loads((base/'PROVENANCE.json').read_text())
 for f,h in manifest['files'].items(): assert hashlib.sha256((base/f).read_bytes()).hexdigest()==h,(vendor,f)
strong=set()
for archive in ('libsunlight_cxx.a','libsunlight_eh.a','libsunlight_native_c.a'):
 lib=ROOT/'target/native-c/sysroot/lib'/archive
 symbols=run(archive+'-symbols',['llvm-nm','-g','--defined-only',lib])
 entries=[l.split() for l in symbols.splitlines() if len(l.split())==3]
 owned=Counter(e[-1] for e in entries if e[-2] not in (b'W',b'V'))
 assert not (strong & set(owned)),('strong ownership overlap',archive,strong&set(owned))
 assert all(n==1 for n in owned.values()),('duplicate strong export',archive)
 strong.update(owned)
 info=run(archive+'-elf',['llvm-readelf','-h',lib]);assert b'Advanced Micro Devices X86-64' in info
for std in ('c++14','c++17','c++20'):
 run('headers-'+std,['python3','tools/native-c/cxx.py','-std='+std,'-Wall','-Wextra','-Werror','-fsyntax-only','-x','c++','-'],data=b'#include <thread>\n#include <typeinfo>\n#include <cstdlib>\nstatic_assert(__EXCEPTIONS && __GXX_RTTI, "enabled");\n')
for name,source,flags in [('execution','tools/native-c/tests/cxx-runtime-execution.cpp',['-Wl,--wrap=malloc,--wrap=free,--wrap=pthread_create']),('historical','docs/audit/probes/cpp-runtime.cpp',[])]:
 elf=out/(name+'.elf')
 run(name+'-link',['python3','tools/native-c/cxx.py',source,*flags,'-Wl,-Map='+str(out/(name+'.map')),'-o',elf])
 assert not run(name+'-undefined',['llvm-nm','-u',elf]).strip()
 symbols=run(name+'-symbols',['llvm-nm','-g','--defined-only',elf])
 for required in (b'__gxx_personality_v0',b'_Unwind_RaiseException',b'__cxa_throw',b'__cxa_get_globals',b'__wrap_main',b'__eh_frame_start',b'__eh_frame_hdr_start'):
  assert required in symbols,required
 info=run(name+'-elf',['llvm-readelf','-h','-l','-S','-d',elf])
 assert b'INTERP' not in info and b'NEEDED' not in info
 for section in (b'.eh_frame ',b'.eh_frame_hdr ',b'.gcc_except_table ',b'.tbss '): assert section in info,section
 mapping=(out/(name+'.map')).read_text();assert '/usr/lib/' not in mapping and '/usr/local/lib/' not in mapping
 frames=run(name+'-frames',['llvm-dwarfdump','--eh-frame',elf]);assert b'FDE' in frames and b'CIE' in frames
 run(name+'-assembly',['llvm-objdump','-dr','--demangle',elf])
probe=ROOT/'docs/audit/probes/cpp-runtime.cpp'
assert probe.read_bytes()==subprocess.check_output(['git','show','HEAD:docs/audit/probes/cpp-runtime.cpp'],cwd=ROOT)
(out/'historical.sha256').write_text(hashlib.sha256(probe.read_bytes()).hexdigest()+'\n')
# Preserve original command literally; it has no native startup/libraries.
diag=run('historical-original',['clang++','--target=x86_64-unknown-none-elf','-nostdinc','-I'+str(ROOT/'sunlight-libc/include'),'-nostdlib','-fuse-ld=lld',probe,'-o',out/'original.elf'],1)
assert b'undefined symbol' in diag and b'file not found' not in diag
local=out/'local-static.cpp';local.write_text('#include <string>\nstd::string& f() { static std::string s("x"); return s; }\nint main() { return f().empty(); }\n')
run('local-static-link',['python3','tools/native-c/cxx.py',local,'-o',out/'local-static.elf'])
guard=run('local-static-symbols',['llvm-nm','-g','--defined-only',out/'local-static.elf'])
for required in (b'__cxa_guard_acquire',b'__cxa_guard_release',b'__cxa_guard_abort',b'__sl_cxx_guard_thread_id'):
 assert required in guard,required
assert not run('local-static-undefined',['llvm-nm','-u',out/'local-static.elf']).strip()
print('Target ABI/unwind ownership, upstream integrity, final ELF/frame inspection PASS. Original command LINK_FAIL; configured historical native ELF linked. Local static guard ABI linked with target-built implementation. Runtime requires both guest configurations.')
