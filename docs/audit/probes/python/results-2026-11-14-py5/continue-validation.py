import pathlib,subprocess,json,hashlib,time
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;P=E.parent/'results-2026-11-14-py1';Q=E.parent/'results-2026-11-14-py2';rows=json.loads((E/'validated-executions.json').read_text())
hashes=json.loads((E/'validated-source.json').read_text())
def run(n,cmd):
 print('START',n,flush=True);t=time.monotonic()
 with (E/(n+'.log')).open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 rows.append(dict(name=n,command=cmd,exit=p.returncode,seconds=time.monotonic()-t,source_matches=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())));(E/'validated-executions.json').write_text(json.dumps(rows,indent=2));print('END',n,p.returncode,flush=True);return p.returncode
cases=json.loads((E/'all-functional-cases.json').read_text())
assert run('py5-filesystem-retry-build',['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra',str(E/'filesystem-test.c'),'-o',str(R/'target/py5-filesystem.elf')])==0
cases.append(dict(name='py5-filesystem',elf=str(R/'target/py5-filesystem.elf'),expected_exit=0))
(E/'all-functional-cases.json').write_text(json.dumps(cases,indent=2))
for cpu in [1,2,4]:
 for label,args in [('functional',['--cases',str(E/'all-functional-cases.json')]),('legacy',['--suite'])]:
  n='validated-'+label+'-cpu-'+str(cpu);run(n,['python3','tools/native-c/guest.py',*args,'--cpus',str(cpu),'--timeout','240','--out',str(E/n)])
for cpu in [2,4]:
 n='validated-tcp-cpu-'+str(cpu);run(n,['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',str(cpu),'--timeout','180','--out',str(E/n)])
run('validated-pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',str(E/'validated-pthread-abi')])
run('validated-cxx-abi',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(E/'validated-cxx-abi')])
run('validated-locked-repeat',['bash','tools/build_native_c.sh'])
