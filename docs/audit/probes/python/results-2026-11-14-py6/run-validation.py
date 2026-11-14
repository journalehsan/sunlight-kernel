import pathlib,subprocess,json,hashlib,time
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;P=E.parent/'results-2026-11-14-py1';Q=E.parent/'results-2026-11-14-py2';rows=[]
paths=[p for d in ['sunlight-libc','kernel/src','ipc/src','tools/native-c','sunlight-fs','sunshell/src','sunlight-files/src','services/vfs_server/src','sunlight-fat/src','sunlight-block/src','sunlight-mm/src','tools/native-std/overlay'] for p in (R/d).rglob('*') if p.is_file() and '.git' not in p.parts and '__pycache__' not in p.parts and 'target' not in p.parts]
hashes={str(p.relative_to(R)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths};(E/'validated-source.json').write_text(json.dumps(hashes,indent=2))
def run(n,cmd):
 print('START',n,flush=True);t=time.monotonic();before=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())
 with (E/(n+'.log')).open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 rows.append(dict(name=n,command=cmd,exit=p.returncode,seconds=time.monotonic()-t,source_matches=before and all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())));(E/'validated-executions.json').write_text(json.dumps(rows,indent=2));print('END',n,p.returncode,flush=True);return p.returncode
assert run('validated-locked',['bash','tools/build_native_c.sh'])==0
for n,extension in [('format','c'),('stream','c'),('wide','c'),('header','cpp'),('math-declarations','c')]:
 assert run('validated-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(Q/(n+'-test.'+extension)),'-o',str(R/'target'/('py3-'+n+'.elf'))])==0
cases=json.loads((P/'verified-cases.json').read_text());seen=set()
for c in cases:
 old=pathlib.Path(c['elf']);new=R/'target'/old.name.replace('py1-verified','py4-validated');c['elf']=str(new)
 if new in seen:continue
 seen.add(new)
 n=old.stem.replace('py1-verified-','');src= {'heap':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/heap-abi.c','admission':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b4/retained-admission.c','lifecycle':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b4/lifecycle.c','exit-matrix':R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b3/exit-matrix.c','header-abi':P/'header-abi-v3.cpp','exit':P/'immediate-exit.c'}.get(n)
 if src is None:src=P/(n+'.c') if (P/(n+'.c')).is_file() else R/'tools/native-c/tests'/(n+'.c')
 assert run('validated-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(src),'-o',str(new)])==0
extra=json.loads((Q/'focused-cases.json').read_text());
for c in extra:c['elf']=c['elf'].replace('target/py2-','target/py3-')
cases+=extra+json.loads(((E.parent/'results-2026-11-14-py3')/'focused-cases.json').read_text());(E/'all-functional-cases.json').write_text(json.dumps(cases,indent=2))
for n in ['contracts','exit','filesystem']:
 assert run('validated-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str((E.parent/'results-2026-11-14-py3')/(n+'-test.c')),'-o',str(R/'target'/('py3-'+n+'.elf'))])==0
for n in ['langinfo','process','contracts','filesystem']:
 assert run('py4-'+n+'-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str((E.parent/'results-2026-11-14-py4')/(n+'-test.c')),'-o',str(R/'target'/('py4-'+n+'.elf'))])==0
newcases=[dict(name='py4-'+n,elf=str(R/'target'/('py4-'+n+'.elf')),expected_exit=0) for n in ['langinfo','process','contracts','filesystem']]
newcases.append(dict(name='py4-process-owner-exit',elf=str(R/'target/py4-process.elf'),args=['owner-exit'],expected_exit=0))
cases+=newcases
(E/'all-functional-cases.json').write_text(json.dumps(cases,indent=2))
assert run('py5-filesystem-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str((E.parent/'results-2026-11-14-py5')/'filesystem-test.c'),'-o',str(R/'target/py5-filesystem.elf')])==0
cases.append(dict(name='py5-filesystem',elf=str(R/'target/py5-filesystem.elf'),expected_exit=0))
(E/'all-functional-cases.json').write_text(json.dumps(cases,indent=2))
assert run('py6-filesystem-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(E/'filesystem-test.c'),'-o',str(R/'target/py6-filesystem.elf')])==0
cases.append(dict(name='py6-filesystem',elf=str(R/'target/py6-filesystem.elf'),expected_exit=0))
(E/'all-functional-cases.json').write_text(json.dumps(cases,indent=2))
assert run('vfs-host-final',['cargo','test','--locked','-p','sunlight-fs','--target','x86_64-unknown-linux-gnu'])==0
assert run('fat-host-final',['cargo','test','--locked','-p','sunlight-fat','--target','x86_64-unknown-linux-gnu'])==0
assert run('files-build-final',['bash','tools/build_svg_apps.sh','-p','sunlight-files','--locked'])==0
for cpu in [1,2,4]:
 for label,args in [('functional',['--cases',str(E/'all-functional-cases.json')]),('legacy',['--suite'])]:
  n='validated-'+label+'-cpu-'+str(cpu);run(n,['python3','tools/native-c/guest.py',*args,'--cpus',str(cpu),'--timeout','240','--out',str(E/n)])
for cpu in [2,4]:
 n='validated-tcp-cpu-'+str(cpu);run(n,['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',str(cpu),'--timeout','180','--out',str(E/n)])
run('validated-pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',str(E/'validated-pthread-abi')])
run('validated-cxx-abi',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(E/'validated-cxx-abi')])
run('validated-locked-repeat',['bash','tools/build_native_c.sh'])

for cpu in [1,2,4]:
 n='shell-cpu-'+str(cpu)
 run(n,['python3','tools/native-c/guest.py','--cases',str(E/'shell-cases.json'),'--cpus',str(cpu),'--timeout','120','--out',str(E/n)])
