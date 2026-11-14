from pathlib import Path
import subprocess,json,shlex
R=Path.cwd(); E=Path(__file__).resolve().parent; results=[]
def run(name,args,timeout=240):
 args=list(map(str,args));p=subprocess.run(args,capture_output=True,timeout=timeout)
 (E/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+p.stdout+p.stderr+f'\nexit={p.returncode}\n'.encode());results.append(dict(name=name,exit=p.returncode,command=args));(E/'supplemental-results.json').write_text(json.dumps(results,indent=2));print(name,p.returncode,flush=True);return p.returncode
for name,source,cpus in [('exit-race','tools/native-c/tests/pthread-tsd-exit-race.c',[2,4]),('recursion','tools/native-c/tests/pthread-once-recursive.c',[2]),('strerror-threads','tools/native-c/tests/strerror-threads.c',[2,4]),('process-exit',E/'process-exit-no-tsd.c',[2])]:
 elf=E/(name+'.elf')
 if run(name+'-final-build',['python3','tools/native-c/cc.py','-O2','-Wall','-Wextra','-Werror',source,'-o',elf])==0:
  for cpu in cpus:run(name+f'-guest-{cpu}',['python3','tools/native-c/guest.py',elf,'--cpus',cpu,'--timeout','90','--expect-exit','0','--out',E/(name+f'-guest-{cpu}')])
run('host-tcb-final',['cargo','test','-p','sunlight-libc','--target','x86_64-unknown-linux-gnu','tls::tests','--','--nocapture'])
run('locked-final-repeat',['bash','tools/build_native_c.sh'])
