#!/usr/bin/env python3
"""Run remote delivery and TLB gates after the shared build matrix ends."""
from pathlib import Path
import subprocess,time,os,json
root=Path(__file__).resolve().parents[1]
folder=root/'.threading-evidence/t3'
# One shared target directory: serialize all kernel builds and freezes.
while True:
    rows=(folder/'matrix.log').read_text().splitlines()
    if any(row.startswith('production ') for row in rows): break
    time.sleep(2)
if any((folder/name).exists() for name in ('remote-wake','handoff','doorbell','tlb','current-clippy')):
    raise SystemExit('Retained architecture evidence already exists; rerun frozen images with threading_t3_validate.py or choose fresh --evidence-dir paths for individual fixture tools.')
runner=['python3','tools/threading_t3_validate.py']
def run(name,argv):
    result=subprocess.run(runner+['command','--timeout','1000',name,'--']+argv,cwd=root)
    print(name,result.returncode,flush=True)
    return result.returncode
run('canonical-t1',['python3','tools/threading_t3_regressions.py','t1'])
run('remote-wake',['python3','tools/test_remote_wake_smp.py','--evidence-dir',str(folder/'remote-wake'),'--timeout','180'])
run('handoff',['python3','tools/test_remote_wake_smp.py','--pin-release','--evidence-dir',str(folder/'handoff'),'--timeout','180'])
run('doorbell',['python3','tools/test_native_doorbell.py','--evidence-dir',str(folder/'doorbell'),'--timeout','180'])
if not run('tlb-build',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--features','mm2b_smp_test','--offline']):
    image=folder/'tlb';image.mkdir(exist_ok=True)
    run('tlb-freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/debug/sunlight-kernel',str(image/'kernel.elf')])
    run('tlb-iso',['bash','tools/make_hybrid_iso.sh',str(image/'kernel.elf'),str(image/'boot.iso'),'target/limine',str(root)])
    for cpus,profile in [(12,'standard'),(12,'sparse'),(12,'high')]:
        marker=f'[MM-2B] {cpus} CPUs pre-touched, replaced, acknowledged, and observed new content: OK'
        code=subprocess.run(runner+['guest','tlb-'+profile,'--iso',str(image/'boot.iso'),'--cpus',str(cpus),'--topology',profile,'--timeout','180','--marker',marker,'--marker','[MM-2B] PMM accounting returned to baseline: OK'],cwd=root).returncode
        print('tlb',profile,code,flush=True)

run('ordinary-debug-clippy',['python3','tools/threading_t3_clippy.py',str(folder/'current-clippy')])
run('canonical-production',['python3','tools/threading_t3_regressions.py','production'])
