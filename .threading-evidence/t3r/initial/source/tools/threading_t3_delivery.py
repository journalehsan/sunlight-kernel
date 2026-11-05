#!/usr/bin/env python3
"""Exercise actual sparse/high-ID remote wake and Doorbell on frozen images."""
import json,time,subprocess
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
root=Path(__file__).resolve().parents[1]
folder=root/'.threading-evidence/t3'
runner=['python3','tools/threading_t3_validate.py']
for fixture,tag in [('remote-wake','F13NR'),('handoff','R2V'),('doorbell','F13NA')]:
    deadline=time.monotonic()+2400
    result=folder/fixture/'results.json'
    while not result.exists() or len(json.loads(result.read_text())['results']) < 4:
        if time.monotonic()>deadline: raise RuntimeError('standard fixture did not finish: '+fixture)
        time.sleep(2)
    def guest(entry):
        profile,cpus=entry
        return profile,subprocess.run(runner+['guest',fixture+'-'+profile,'--iso',str(folder/fixture/'sunlightos.iso'),'--cpus',str(cpus),'--topology',profile,'--timeout','180','--marker',f'[{tag}] PASS cpus={cpus} '],cwd=root).returncode
    with ThreadPoolExecutor(max_workers=2) as pool:
        print(fixture,list(pool.map(guest,[('sparse',4),('high',2)])),flush=True)
