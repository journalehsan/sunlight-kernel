#!/usr/bin/env python3
import subprocess,pathlib,json,hashlib,time,shutil,os
ROOT=pathlib.Path(__file__).resolve().parents[5]
OUT=pathlib.Path(os.environ.get("R2B4_OUTPUT",str(pathlib.Path(__file__).resolve().parent)))
OUT.mkdir(parents=True,exist_ok=True)
files=['ipc/src/lib.rs','kernel/src/process/foreground.rs','sunlight-libc/src/lib.rs','sunlight-libc/src/sys.rs','tools/foreground-scope-proof.rs','kernel/src/process/mod.rs','kernel/src/sched/mod.rs','kernel/src/sched/native_group.rs','kernel/src/sched/thread_admission.rs','kernel/src/arch/x86_64/syscall.rs','tools/native-c/guest.py','sunlight-libc/src/pthread.rs','sunlight-libc/include/pthread.h']
def hashes():return {p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in files}
source=hashes();(OUT/'validated-source.json').write_text(json.dumps(source,indent=2)+'\n')
rows=[]
def run(name,cmd,timeout=1800):
 start=time.monotonic();print('START',name,flush=True)
 with (OUT/(name+'.log')).open('w') as f:
  f.write(json.dumps(cmd)+'\n');f.flush()
  try:code=subprocess.run(cmd,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT,timeout=timeout).returncode
  except subprocess.TimeoutExpired:code=124
 rows.append(dict(name=name,command=cmd,exit=code,seconds=time.monotonic()-start,source_matches=hashes()==source))
 (OUT/'executions.json').write_text(json.dumps(rows,indent=2)+'\n')
 print('END',name,code,flush=True)
 return code
run('locked-final',['bash','tools/build_native_c.sh'])
for cpu in [1,2,4]:
 run('lifecycle-cpu-'+str(cpu),['python3','tools/native-c/guest.py','--cases',str(OUT/'cases.json'),'--cpus',str(cpu),'--timeout','180','--out',str(OUT/('lifecycle-cpu-'+str(cpu)))])
for cpu in [1,2,4]:
 run('regression-cpu-'+str(cpu),['python3','tools/native-c/guest.py','--suite','--cpus',str(cpu),'--timeout','240','--out',str(OUT/('regression-cpu-'+str(cpu)))])
for cpu in [2,4]:
 run('tcp-cpu-'+str(cpu),['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',str(cpu),'--timeout','180','--out',str(OUT/('tcp-cpu-'+str(cpu)))])
run('pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',str(OUT/'pthread-abi')])
run('kernel-host',['cargo','test','--locked','--offline','--target','x86_64-unknown-linux-gnu','-p','sunlight-kernel','--lib'])
run('ipc-host',['cargo','test','--locked','--offline','--target','x86_64-unknown-linux-gnu','-p','sunlight-ipc','--lib'])
run('scheduler-host',['python3','tools/test_scheduler_host.py'])
p=ROOT/'target/scheduler-b-evidence/focused-host.log'
if p.exists():shutil.copy2(p,OUT/'scheduler-focused.log')
run('production-check',['cargo','check','--locked','--offline','-p','sunlight-kernel','--bin','sunlight-kernel'])
print('FINISHED',flush=True)
