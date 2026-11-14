import pathlib,subprocess,json,hashlib,time
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;P=E.parent/'results-2026-11-14-py1';Q=E.parent/'results-2026-11-14-py2';rows=[]
paths=[p for d in ['sunlight-libc','kernel/src','ipc/src','tools/native-c'] for p in (R/d).rglob('*') if p.is_file() and '.git' not in p.parts and '__pycache__' not in p.parts and 'target' not in p.parts]
hashes={str(p.relative_to(R)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths};(E/'validated-source.json').write_text(json.dumps(hashes,indent=2))
def run(n,cmd):
 print('START',n,flush=True);t=time.monotonic()
 with (E/(n+'.log')).open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 rows.append(dict(name=n,command=cmd,exit=p.returncode,seconds=time.monotonic()-t,source_matches=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())));(E/'validated-executions.json').write_text(json.dumps(rows,indent=2));print('END',n,p.returncode,flush=True);return p.returncode
assert run('validated-locked',['bash','tools/build_native_c.sh'])==0
for n,extension in [('format','c'),('stream','c'),('wide','c'),('header','cpp'),('math-declarations','c')]:
 assert run('validated-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(Q/(n+'-test.'+extension)),'-o',str(R/'target'/('py3-'+n+'.elf'))])==0
cases=json.loads((P/'verified-cases.json').read_text());seen=set()
for c in cases:
 old=pathlib.Path(c['elf']);new=R/'target'/old.name.replace('py1-verified','py3-validated');c['elf']=str(new)
 if new in seen:continue
 seen.add(new)
 n=old.stem.replace('py1-verified-','');src= {'heap':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/heap-abi.c','admission':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b4/retained-admission.c','lifecycle':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b4/lifecycle.c','exit-matrix':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b3/exit-matrix.c','header-abi':P/'header-abi-v3.cpp','exit':P/'immediate-exit.c'}.get(n)
 if src is None:src=P/(n+'.c') if (P/(n+'.c')).is_file() else R/'tools/native-c/tests'/(n+'.c')
 assert run('validated-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(src),'-o',str(new)])==0
extra=json.loads((Q/'focused-cases.json').read_text());
for c in extra:c['elf']=c['elf'].replace('target/py2-','target/py3-')
cases+=extra+json.loads((E/'focused-cases.json').read_text());(E/'all-functional-cases.json').write_text(json.dumps(cases,indent=2))
for n in ['contracts','exit','filesystem']:
 assert run('validated-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(E/(n+'-test.c')),'-o',str(R/'target'/('py3-'+n+'.elf'))])==0
for cpu in [1,2,4]:
 for label,args in [('functional',['--cases',str(E/'all-functional-cases.json')]),('legacy',['--suite'])]:
  n='validated-'+label+'-cpu-'+str(cpu);run(n,['python3','tools/native-c/guest.py',*args,'--cpus',str(cpu),'--timeout','240','--out',str(E/n)])
for cpu in [2,4]:
 n='validated-tcp-cpu-'+str(cpu);run(n,['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',str(cpu),'--timeout','180','--out',str(E/n)])
run('validated-pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',str(E/'validated-pthread-abi')])
run('validated-cxx-abi',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(E/'validated-cxx-abi')])
run('validated-locked-repeat',['bash','tools/build_native_c.sh'])
