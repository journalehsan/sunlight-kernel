import pathlib,subprocess,json,hashlib,time
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;P=R/'docs/audit/probes/native-foundation';rows=[]
files=list(json.loads((E/'formatted-source.json').read_text()))+['sunlight-libc/include/time.h','sunlight-libc/include/bits/time.h'];hashes={f:hashlib.sha256((R/f).read_bytes()).hexdigest() for f in files};(E/'verified-source.json').write_text(json.dumps(hashes,indent=2))
def run(n,cmd):
 print('START',n,flush=True);t=time.monotonic()
 with (E/(n+'.log')).open('w') as f:
  f.write(json.dumps(cmd)+'\n');f.flush();rc=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT).returncode
 rows.append(dict(name=n,command=cmd,exit=rc,seconds=time.monotonic()-t,source_matches=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())));(E/'verified-executions.json').write_text(json.dumps(rows,indent=2));print('END',n,rc,flush=True);return rc
assert run('verified-locked',['bash','tools/build_native_c.sh'])==0
cases=[]
inputs=[('condvar',E/'condvar.c',0),('libc',E/'libc.c',0),('exit',E/'immediate-exit.c',37),('heap',P/'results-2026-11-14-runtime-r1/heap-abi.c',0),('admission',P/'results-2026-11-14-runtime-r2b4/retained-admission.c',0)]+[(n,R/('tools/native-c/tests/'+n+'.c'),0) for n in ['pthread-tsd-once','pthread-tsd-exit-race','pthread-once-recursive','strerror-threads']]+[(n,E/(n+'.c'),0) for n in ['condvar-timeout','string-guard']]+[('header-abi',E/'header-abi-v3.cpp',0)]
for n,src,code in inputs:
 elf=R/('target/py1-verified-'+n+'.elf');flags=['-std=c++17'] if src.suffix=='.cpp' else ['-std=c11']
 assert run('verified-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin',*flags,'-Wall','-Wextra','-Werror',str(src),'-o',str(elf)])==0
 cases.append(dict(name=n,elf=str(elf),expected_exit=code))
for n,src in [('lifecycle',P/'results-2026-11-14-runtime-r2b4/lifecycle.c'),('exit-matrix',P/'results-2026-11-14-runtime-r2b3/exit-matrix.c'),('exit-controls',E/'exit-controls.c')]:
 assert run('verified-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(src),'-o',str(R/('target/py1-verified-'+n+'.elf'))])==0
for code in [0,37,255]:cases.append(dict(name='owner-exit-'+str(code),elf=str(R/'target/py1-verified-exit-controls.elf'),expected_exit=code,args=[str(code)]))
lifecycle=json.loads((P/'results-2026-11-14-runtime-r2b4/validated/cases.json').read_text())
for c in lifecycle:c['elf']=c['elf'].replace('r2b4-','py1-verified-');c['name']='lifecycle-'+c['name']
(E/'verified-cases.json').write_text(json.dumps(cases+lifecycle,indent=2))
for cpu in [1,2,4]:
 for label,args in [('verified-functional',['--cases',str(E/'verified-cases.json')]),('verified-regression',['--suite'])]:
  n=label+'-cpu-'+str(cpu);run(n,['python3','tools/native-c/guest.py',*args,'--cpus',str(cpu),'--timeout','240','--out',str(E/n)])
for cpu in [2,4]:
 n='verified-tcp-cpu-'+str(cpu);run(n,['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',str(cpu),'--timeout','180','--out',str(E/n)])
run('verified-pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',str(E/'verified-pthread-abi')])
run('verified-cxx-abi',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(E/'verified-cxx-abi')])
run('verified-locked-repeat',['bash','tools/build_native_c.sh'])
