#!/usr/bin/env python3
"""Close Linux affinity proof after the serialized architecture builds end."""
import time,subprocess,json
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
root=Path(__file__).resolve().parents[1]
f=root/'.threading-evidence/t3'
deadline=time.monotonic()+2400
while True:
    end=sorted(f.glob('*-canonical-production/result.json'))
    if end and json.loads(end[-1].read_text()).get('pass'):break
    if time.monotonic()>deadline:raise RuntimeError('canonical production did not complete')
    time.sleep(2)
subprocess.run(['python3','tools/threading_t3_regressions.py','linux'],cwd=root,check=True)
iso=sorted(f.glob('*-suite-linux'))[-1]/'boot.iso'
def run(entry):
    profile,cpus=entry
    return profile,subprocess.run(['python3','tools/threading_t3_validate.py','guest','affinity-'+profile,'--iso',str(iso),'--cpus',str(cpus),'--topology',profile,'--timeout','180','--marker','THREAD_PROBE PASS','--marker',f'affinity_mask={(1<<cpus)-1:016x}'],cwd=root).returncode
with ThreadPoolExecutor(max_workers=2) as pool:print(list(pool.map(run,[('sparse',4),('high',2)])),flush=True)
