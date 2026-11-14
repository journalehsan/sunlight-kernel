import pathlib,subprocess,json,time,os
R=pathlib.Path(__file__).resolve().parents[5];E=pathlib.Path(__file__).resolve().parent;O=pathlib.Path(os.environ.get('R2B4_OUTPUT',str(E/'final')));rows=[]
def run(name,args):
 t=time.monotonic()
 with (O/(name+'.log')).open('w') as f:
  f.write(json.dumps(args)+'\n');f.flush();rc=subprocess.run(args,cwd=R,stdout=f,stderr=subprocess.STDOUT,timeout=1800).returncode
 rows.append(dict(name=name,command=args,exit=rc,seconds=time.monotonic()-t));(O/'supplemental-executions.json').write_text(json.dumps(rows,indent=2)+'\n');print(name,rc,flush=True)
run('heap-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(R/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/heap-abi.c'),'-o',str(R/'target/r2b4-heap.elf')])
run('admission-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(E/'retained-admission.c'),'-o',str(R/'target/r2b4-admission.elf')])
cases=[dict(name='heap-abi',elf=str(R/'target/r2b4-heap.elf'),expected_exit=0),dict(name='retained-admission',elf=str(R/'target/r2b4-admission.elf'),expected_exit=0)]
(O/'supplemental-cases.json').write_text(json.dumps(cases,indent=2)+'\n')
for cpu in [1,2,4]:run('supplemental-cpu-'+str(cpu),['python3','tools/native-c/guest.py','--cases',str(O/'supplemental-cases.json'),'--cpus',str(cpu),'--timeout','120','--out',str(O/('supplemental-cpu-'+str(cpu)))])
run('cxx-abi-vendor',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(O/'cxx-abi-vendor')])
