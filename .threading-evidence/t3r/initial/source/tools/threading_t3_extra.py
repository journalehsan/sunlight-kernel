#!/usr/bin/env python3
"""Freeze architecture regression images and run native topology controls."""
from pathlib import Path
import subprocess
from concurrent.futures import ThreadPoolExecutor
root=Path(__file__).resolve().parents[1]
runner=['python3','tools/threading_t3_validate.py']
def run(argv):
    return subprocess.run(argv,cwd=root).returncode
# Wait for the final T1 image to be frozen, without contending with kernel builds.
import time,json
while True:
    suites=sorted((root/'.threading-evidence/t3').glob('*-suite-t1'))
    final=suites[-1]
    if (final/'boot.iso').exists() and (final/'kernel.elf').exists(): break
    time.sleep(1)
iso=final/'boot.iso'
with ThreadPoolExecutor(max_workers=2) as pool:
    tasks=[('sparse',4),('nonzero',1),('high',2)]
    results=list(pool.map(lambda entry: run(runner+['guest','topology-'+entry[0],'--iso',str(iso),'--cpus',str(entry[1]),'--topology',entry[0],'--timeout','180','--marker','[T1] PASS','--marker',f'[SCHED-B] PASS cpus={entry[1]} ']),tasks))
print(dict(zip((t[0] for t in tasks),results)),flush=True)
