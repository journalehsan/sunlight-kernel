import pathlib,subprocess,json,time,hashlib
R=pathlib.Path(__file__).resolve().parents[5]; E=pathlib.Path(__file__).resolve().parent; rows=[]
source=json.loads((E/'source-baseline.json').read_text())['files']
def run(name,args,timeout=1800):
 print('START',name,flush=True);start=time.monotonic()
 with (E/(name+'.log')).open('w') as f:
  f.write(json.dumps(args)+'\n');f.flush()
  try:rc=subprocess.run(args,cwd=R,stdout=f,stderr=subprocess.STDOUT,timeout=timeout).returncode
  except subprocess.TimeoutExpired:rc=124
 rows.append(dict(name=name,command=args,exit=rc,seconds=time.monotonic()-start,source_matches=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in source.items() if p.startswith(('kernel/','ipc/','sunlight-libc/','tools/')))))
 (E/'extra-executions.json').write_text(json.dumps(rows,indent=2)+'\n');print('END',name,rc,flush=True);return rc
for cpu in [1,4]:run('resources-cpu-'+str(cpu),['python3','tools/native-c/guest.py','--cases',str(E/'resource-cases.json'),'--cpus',str(cpu),'--timeout','180','--out',str(E/('resources-cpu-'+str(cpu)))])
run('tls-runner',['python3',str(E/'run-tls.py')])
run('heap-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(E.parent/'results-2026-11-14-runtime-r1/heap-abi.c'),'-o',str(R/'target/r2b5-heap.elf')])
run('admission-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(E.parent/'results-2026-11-14-runtime-r2b4/retained-admission.c'),'-o',str(R/'target/r2b5-admission.elf')])
(E/'supplemental-cases.json').write_text(json.dumps([dict(name=n,elf=str(R/('target/r2b5-'+e+'.elf')),expected_exit=0) for n,e in [('heap-abi','heap'),('retained-admission','admission')]],indent=2)+'\n')
for cpu in [1,2,4]:run('supplemental-cpu-'+str(cpu),['python3','tools/native-c/guest.py','--cases',str(E/'supplemental-cases.json'),'--cpus',str(cpu),'--timeout','120','--out',str(E/('supplemental-cpu-'+str(cpu)))])
run('cxx-abi-vendor',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',str(E/'cxx-abi-vendor')])
run('pty-host',['cargo','test','--locked','--offline','--target','x86_64-unknown-linux-gnu','--manifest-path','pty_server/Cargo.toml'])
run('foreground-build',['rustc','--edition=2021','--test','tools/foreground-scope-proof.rs','-o',str(R/'target/r2b5-foreground')])
run('foreground-host',[str(R/'target/r2b5-foreground')])
run('retirement-runner',['python3',str(E/'run-retirement.py')])
