import pathlib,subprocess,json,time,hashlib
R=pathlib.Path.cwd(); E=pathlib.Path(__file__).resolve().parent; P=R/'docs/audit/probes/native-foundation'; rows=[]
files=['sunlight-libc/src/condvar.rs','sunlight-libc/src/string.rs','sunlight-libc/src/inttypes.rs','sunlight-libc/src/termination.rs','sunlight-libc/src/lib.rs','sunlight-libc/include/pthread.h','sunlight-libc/include/string.h','sunlight-libc/include/stdlib.h','sunlight-libc/include/unistd.h']
hashes={f:hashlib.sha256((R/f).read_bytes()).hexdigest() for f in files};(E/'validated-source.json').write_text(json.dumps(hashes,indent=2))
def run(n,cmd):
 print('START',n,flush=True); t=time.monotonic()
 with (E/(n+'.log')).open('w') as f:
  f.write(json.dumps(cmd)+'\n');f.flush();rc=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT).returncode
 rows.append(dict(name=n,command=cmd,exit=rc,seconds=time.monotonic()-t,source_matches=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())));(E/'final-executions.json').write_text(json.dumps(rows,indent=2));print('END',n,rc,flush=True);return rc
if run('locked-final',['bash','tools/build_native_c.sh']):raise SystemExit(1)
cases=[]
for name,src,code in [('condvar',E/'condvar.c',0),('libc',E/'libc.c',0),('exit',E/'immediate-exit.c',37),('heap',P/'results-2026-11-14-runtime-r1/heap-abi.c',0),('admission',P/'results-2026-11-14-runtime-r2b4/retained-admission.c',0)]+[(n,R/('tools/native-c/tests/'+n+'.c'),0) for n in ['pthread-tsd-once','pthread-tsd-exit-race','pthread-once-recursive','strerror-threads']]:
 elf=R/('target/py1-final-'+name+'.elf')
 if run(name+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-std=c11','-Wall','-Wextra','-Werror',str(src),'-o',str(elf)]):raise SystemExit(1)
 cases.append(dict(name=name,elf=str(elf),expected_exit=code))
for name,src in [('lifecycle',P/'results-2026-11-14-runtime-r2b4/lifecycle.c'),('exit-matrix',P/'results-2026-11-14-runtime-r2b3/exit-matrix.c')]:
 if run(name+'-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(src),'-o',str(R/('target/py1-final-'+name+'.elf'))]):raise SystemExit(1)
lifecycle=json.loads((P/'results-2026-11-14-runtime-r2b4/validated/cases.json').read_text())
for c in lifecycle:c['elf']=c['elf'].replace('r2b4-','py1-final-')
(E/'lifecycle-cases.json').write_text(json.dumps(lifecycle,indent=2));(E/'final-focused-cases.json').write_text(json.dumps(cases,indent=2))
for cpu in [1,2,4]:
 for label,path,timeout in [('focused-final',E/'final-focused-cases.json',180),('lifecycle-final',E/'lifecycle-cases.json',180),('regression-final',None,240)]:
  args=['--cases',str(path)] if path else ['--suite']; n=label+'-cpu-'+str(cpu)
  run(n,['python3','tools/native-c/guest.py',*args,'--cpus',str(cpu),'--timeout',str(timeout),'--out',str(E/n)])
for cpu in [2,4]:
 n='tcp-final-cpu-'+str(cpu);run(n,['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',str(cpu),'--timeout','180','--out',str(E/n)])
run('pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',str(E/'pthread-abi')])
run('cxx-abi',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(E/'cxx-abi')])
run('locked-final-repeat',['bash','tools/build_native_c.sh'])
