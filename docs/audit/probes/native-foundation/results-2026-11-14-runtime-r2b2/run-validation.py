from pathlib import Path
import subprocess,json,shlex,time
R=Path.cwd(); E=Path(__file__).resolve().parent
results=[]
def run(name,args,timeout=600):
 args=list(map(str,args));print(name,flush=True); started=time.time()
 try:
  p=subprocess.run(args,cwd=R,capture_output=True,timeout=timeout);rc=p.returncode;out=p.stdout+p.stderr
 except subprocess.TimeoutExpired as p:rc=124;out=(p.stdout or b'')+(p.stderr or b'')+b'\nSUPERVISOR TIMEOUT\n'
 (E/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+out+f'\nexit={rc}\n'.encode())
 results.append(dict(name=name,command=args,exit=rc,elapsed_seconds=time.time()-started));(E/'run-results.json').write_text(json.dumps(results,indent=2)+'\n');print(name,rc,flush=True);return rc
for name,source in [('initial-exit',E/'initial-exit.c'),('detached-churn',E/'detached-churn.c'),('focused','tools/native-c/tests/pthread-tsd-once.c'),('tls-exit-race','tools/native-c/tests/pthread-tsd-exit-race.c')]:
 if run(name+'-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',source,'-o',E/(name+'.elf')])==0:
  for cpu in [1,2,4]:
   run(name+f'-cpu-{cpu}',['python3','tools/native-c/guest.py',E/(name+'.elf'),'--cpus',cpu,'--timeout','120','--expect-exit',0,'--out',E/(name+f'-cpu-{cpu}')])
run('regression-suite',['python3','tools/native-c/guest.py','--suite','--cpus',2,'--timeout',180,'--out',E/'regression-suite'])
run('cxx-abi',['python3','tools/native-c/tests/run_cxx_runtime.py','--out',E/'cxx-abi'])
run('heap-abi-build',['python3','tools/native-c/cc.py','-O0',E.parent/'results-2026-11-14-runtime-r1/heap-abi.c','-o',E/'heap-abi.elf'])
run('heap-abi',['python3','tools/native-c/guest.py',E/'heap-abi.elf','--cpus',2,'--timeout',90,'--expect-exit',0,'--out',E/'heap-abi'])
run('tcp-loopback',['python3','tools/native-c/run_poll_tcp.py','--mode','loopback','--cpus',2,'--timeout',180,'--out',E/'tcp-loopback'])
run('pthread-abi',['python3','tools/native-c/tests/run_pthread.py','--out',E/'pthread-abi'])
run('locked-final',['bash','tools/build_native_c.sh'])
