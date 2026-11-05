#!/usr/bin/env python3
"""Validate ordinary final builds after the focused affinity proof completes."""
import subprocess,time,json
from pathlib import Path
root=Path(__file__).resolve().parents[1]
f=root/'.threading-evidence/t3'
deadline=time.monotonic()+900
while True:
    complete=True
    for profile in ('sparse','high'):
        matches=sorted(f.glob('*-affinity-'+profile+'/result.json'))
        complete=complete and bool(matches) and json.loads(matches[-1].read_text()).get('pass',False)
    if complete:break
    if time.monotonic()>deadline:raise RuntimeError('affinity topology gates did not complete')
    time.sleep(2)
runner=['python3','tools/threading_t3_validate.py']
for name,argv in [('final-debug',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--offline']),('final-format',['python3','tools/threading_t3_format.py']),('final-parsers',['python3','-m','unittest','discover','-s','tools','-p','test_threading_t*.py']),('final-production',['python3','tools/threading_t3_regressions.py','production'])]:
    code=subprocess.run(runner+['command','--timeout','900',name,'--']+argv,cwd=root).returncode
    print(name,code,flush=True)
    if code:raise RuntimeError(name+' failed')
