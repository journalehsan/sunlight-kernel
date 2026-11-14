from pathlib import Path
import subprocess,json,shlex,time,hashlib
E=Path(__file__).resolve().parent
rows=[]
for name in ['detached-accounting','tls-controlled']:
 for cpu in [1,2,4]:
  stem=f'{name}-cpu-{cpu}'
  args=['python3','tools/native-c/guest.py',str(E/(name+'.elf')),'--cpus',str(cpu),'--timeout','120','--expect-exit','0','--out',str(E/stem)]
  started=time.time();print(stem,flush=True)
  try:
   p=subprocess.run(args,capture_output=True,timeout=600);rc=p.returncode;data=p.stdout+p.stderr
  except subprocess.TimeoutExpired as p:rc=124;data=(p.stdout or b'')+(p.stderr or b'')+b'\nSUPERVISOR TIMEOUT\n'
  (E/(stem+'.log')).write_bytes(('$ '+shlex.join(args)+'\n').encode()+data+f'\nexit={rc}\n'.encode())
  rows.append(dict(name=stem,command=args,exit=rc,elapsed_seconds=time.time()-started));(E/'supplemental-results.json').write_text(json.dumps(rows,indent=2)+'\n'); print(stem,rc,flush=True)
# Check the R2A vendor manifest afresh; its recorded hashes remain immutable.
expected=json.loads((E.parent/'results-2026-11-14-runtime-r2a/libm-vendor-integrity.json').read_text())
actual={p:hashlib.sha256((Path('sunlight-libc/vendor/libm')/p).read_bytes()).hexdigest() for p in expected}
(E/'libm-vendor-check.json').write_text(json.dumps(dict(status='PASS' if actual==expected else 'FAIL',reference='results-2026-11-14-runtime-r2a/libm-vendor-integrity.json',checked_files=len(actual),differences=[p for p in actual if actual[p]!=expected[p]]),indent=2)+'\n')
