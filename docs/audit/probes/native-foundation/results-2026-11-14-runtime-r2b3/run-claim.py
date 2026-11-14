from pathlib import Path
import subprocess,json,shlex,time
E=Path(__file__).resolve().parent; R=Path.cwd(); rows=[]
def run(name,args):
 args=list(map(str,args));p=subprocess.run(args,capture_output=True,timeout=600)
 (E/(name+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+p.stdout+p.stderr+f'\nexit={p.returncode}\n'.encode())
 rows.append(dict(name=name,command=args,exit=p.returncode));(E/'claim-run-results.json').write_text(json.dumps(rows,indent=2)+'\n'); print(name,p.returncode,flush=True);return p.returncode
if run('handle-claim-build',['python3',E/'build-handle-claim.py'])==0:
 for cpu in [1,2,4]:
  run(f'handle-claim-cpu-{cpu}',['python3','tools/native-c/guest.py',R/'target/r2b3-handle-claim.elf','--cpus',cpu,'--timeout',120,'--expect-exit',0,'--out',E/f'handle-claim-cpu-{cpu}'])
