from pathlib import Path
import subprocess,json,shlex
R=Path.cwd(); E=Path(__file__).resolve().parent
results=[]
def run(name,args,timeout=360):
 args=list(map(str,args));print(name,flush=True)
 try:
  p=subprocess.run(args,cwd=R,capture_output=True,timeout=timeout);rc=p.returncode;output=p.stdout+p.stderr
 except subprocess.TimeoutExpired as p:rc=124;output=(p.stdout or b'')+(p.stderr or b'')+b'\nSUPERVISOR TIMEOUT\n'
 (E/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+output+f'\nexit={rc}\n'.encode());results.append(dict(name=name,command=args,exit=rc));(E/'run-results.json').write_text(json.dumps(results,indent=2));print(name,rc,flush=True);return rc
run('locked-final',['bash','tools/build_native_c.sh'])
if run('focused-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror','tools/native-c/tests/pthread-tsd-once.c','-o',E/'focused.elf'])==0:
 for cpus in [2,1,4]:run(f'acceptance-focused-{cpus}',['python3','tools/native-c/guest.py',E/'focused.elf','--cpus',cpus,'--timeout','120','--expect-exit','0','--out',E/f'acceptance-focused-{cpus}'])
for language,driver,standard in [('c','cc.py','c11'),('cxx','cxx.py','c++17')]:
 for feature in ['base','posix','gnu']:
  flags=[] if feature=='base' else ['-D_POSIX_C_SOURCE=200809L'] if feature=='posix' else ['-D_GNU_SOURCE']
  run('header-'+language+'-'+feature,['python3','tools/native-c/'+driver,'-x','c' if language=='c' else 'c++','-std='+standard,'-Wall','-Wextra','-Werror',*flags,'-c','tools/native-c/tests/pthread-tsd-headers.c','-o',E/(language+'-'+feature+'.o')])
 # Link separately so -x cannot reinterpret archives.
 if run('header-link-'+language,['python3','tools/native-c/'+driver,E/(language+'-base.o'),'-o',E/('header-'+language+'.elf')])==0:
  run('header-guest-'+language,['python3','tools/native-c/guest.py',E/('header-'+language+'.elf'),'--cpus','2','--timeout','90','--expect-exit','0','--out',E/('header-guest-'+language)])
run('owner-exit-build',['python3','tools/native-c/cc.py',E/'owner-exit-limitation.c','-o',E/'owner-exit.elf'])
run('owner-exit-observation',['python3','tools/native-c/guest.py',E/'owner-exit.elf','--cpus','2','--timeout','90','--expect-exit','0','--out',E/'owner-exit-observation'])
run('acceptance-suite',['python3','tools/native-c/guest.py','--suite','--cpus','2','--timeout','180','--out',E/'acceptance-suite'],600)
run('cxx-abi-final',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',E/'cxx-abi-final'])
source=E.parent/'results-2026-11-14-runtime-r1/heap-abi.c'
run('heap-abi-build',['python3','tools/native-c/cc.py','-O0',source,'-o',E/'heap-abi.elf'])
run('heap-abi-guest',['python3','tools/native-c/guest.py',E/'heap-abi.elf','--cpus','2','--timeout','90','--expect-exit','0','--out',E/'heap-abi-guest'])
run('tcp-loopback',['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus','2','--timeout','180','--out',E/'tcp-loopback'],600)
run('pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',E/'pthread-abi'])
run('locked-repeat',['bash','tools/build_native_c.sh'])
print(json.dumps(results,indent=2))
