"""Retained orchestration, no source mutation, builds serialized."""
import sys,subprocess,json,time
from pathlib import Path
root=Path.cwd();base=root/'.threading-evidence/t3r'
# Wait only for our owned migration runner to finish, preserving its verdict.
while not any('handoff' in p.read_text() for p in []):
 runs=sorted(base.glob('*-migration/result.json'))
 if len(runs)>=2: break
 time.sleep(1)
latest=json.loads(runs[-1].read_text())
if not latest['pass']: raise SystemExit('Final pilot/matrix failed; compatibility expansion NOT_RUN')
runner=['python3','tools/threading_t3r_validate.py']
results=[]
def run(name,args,timeout=1200):
 code=subprocess.run(runner+['command','--timeout',str(timeout),name,'--']+args,cwd=root).returncode
 results.append({'name':name,'argv':args,'exit_code':code})
 (base/'finish-results.json').write_text(json.dumps(results,indent=2)+'\n')
 return code
run('remote-wake',['python3','tools/test_remote_wake_smp.py','--cpus','2','12','--evidence-dir',str(base/'remote-wake'),'--timeout','150'])
run('handoff',['python3','tools/test_remote_wake_smp.py','--pin-release','--cpus','2','12','--evidence-dir',str(base/'handoff'),'--timeout','150'])
run('native-std-hayro',['python3','tools/threading_t3r_regressions.py','std','--cpus','2','4','12'])
run('native-fp',['python3','tools/threading_t3r_regressions.py','fp','--cpus','2','4','12'])
run('ordinary-debug',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--offline'])
run('ordinary-debug-freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/debug/sunlight-kernel',str(base/'ordinary-debug.elf')])
run('production',['python3','tools/threading_t3r_regressions.py','production','--cpus','2'])
run('release-feature-exclusion',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--release','--no-default-features','--features','thread_migration_test','--offline'])
run('release-feature-freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/release/sunlight-kernel',str(base/'release-feature.elf')])
run('tlb-build',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--features','mm2b_smp_test','--offline'])
image=base/'tlb';image.mkdir(exist_ok=False)
run('tlb-freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/debug/sunlight-kernel',str(image/'kernel.elf')])
run('tlb-iso',['bash','tools/make_hybrid_iso.sh',str(image/'kernel.elf'),str(image/'boot.iso'),'target/limine',str(root)])
run('tlb-12cpu',runner+['guest','tlb-12cpu','--iso',str(image/'boot.iso'),'--cpus','12','--timeout','180','--marker','[MM-2B] 12 CPUs pre-touched, replaced, acknowledged, and observed new content: OK','--marker','[MM-2B] PMM accounting returned to baseline: OK'])
