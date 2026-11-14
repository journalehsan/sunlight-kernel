from pathlib import Path
import subprocess,json,shlex,time
E=Path(__file__).resolve().parent; R=Path.cwd(); rows=[]
def run(name,args):
 args=list(map(str,args));p=subprocess.run(args,capture_output=True,timeout=600)
 (E/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+p.stdout+p.stderr+f'\nexit={p.returncode}\n'.encode())
 rows.append(dict(name=name,command=args,exit=p.returncode));(E/'exit-run-results.json').write_text(json.dumps(rows,indent=2)+'\n'); print(name,p.returncode,flush=True);return p.returncode
elf=R/'target/r2b3-exit-matrix.elf'
if run('exit-matrix-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',E/'exit-matrix.c','-o',elf])==0:
 for mode,expected in [(0,17),(1,23),(2,29),(3,137)]:
  for cpu in [1,2,4]:
   name=f'exit-mode-{mode}-cpu-{cpu}'
   run(name,['python3','tools/native-c/guest.py',elf,'--arg',mode,'--cpus',cpu,'--timeout',120,'--expect-exit',expected,'--out',E/name])
run('exit-api-negative',['python3','tools/native-c/cc.py','-std=c11','-Wall','-Wextra','-Werror','-c',E.parent/'results-2026-11-14-runtime-r2b2/exit-api-probe.c','-o',R/'target/r2b3-exit-api.o'])
run('exit-link-negative',['python3','tools/native-c/cc.py',E.parent/'results-2026-11-14-runtime-r2b2/exit-link-probe.c','-o',R/'target/r2b3-exit-link.elf'])

if run('detached-capacity-build',['python3',E/'build-detached-capacity.py'])==0:
 for cpu in [1,2,4]:
  run(f'detached-capacity-cpu-{cpu}',['python3','tools/native-c/guest.py',R/'target/r2b3-detached-capacity.elf','--cpus',cpu,'--timeout',120,'--expect-exit',0,'--out',E/f'detached-capacity-cpu-{cpu}'])
for cpu in [1,4]:
 run(f'regression-suite-cpu-{cpu}',['python3','tools/native-c/guest.py','--suite','--cpus',cpu,'--timeout',180,'--out',E/f'regression-suite-cpu-{cpu}'])
 run(f'heap-abi-cpu-{cpu}',['python3','tools/native-c/guest.py',R/'target/r2b3-heap-abi.elf','--cpus',cpu,'--timeout',90,'--expect-exit',0,'--out',E/f'heap-abi-cpu-{cpu}'])
run('tcp-loopback-cpu-4',['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',4,'--timeout',180,'--out',E/'tcp-loopback-cpu-4'])
