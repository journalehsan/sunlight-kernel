#!/usr/bin/env python3
"""Freeze each final-source kernel before running bounded native regressions."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
from threading_t2_validate import identity
ROOT = Path(__file__).resolve().parents[1]
RUNNER = ['python3', 'tools/threading_t2_validate.py']
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('suite', choices=['admission','t1','auth','linux','fp','std','production'])
p.add_argument('--cpus', type=int, nargs='+', default=[1,2,4,12])
a = p.parse_args()
if any(c not in (1,2,4,12) for c in a.cpus): p.error('supported CPU matrix: 1/2/4/12')
folder = ROOT / '.threading-evidence/t2' / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ') + '-suite-' + a.suite)
folder.mkdir(parents=True)
print(folder, flush=True)
(folder/'source.json').write_text(json.dumps(identity(), indent=2)+'\n')
commands=[]
def run(name, argv, timeout=900):
    commands.append(argv)
    subprocess.run(RUNNER+['command','--timeout',str(timeout),a.suite+'-'+name,'--']+argv, cwd=ROOT, check=True)
kernel = ['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--offline']
markers = {
 'admission':['[T2] PASS'],
 't1':['[T1] PASS'],
 'auth':['[T1-AUTH] PASS','[CAP-R41] COMPLETE','[CAP-P] native PASS','[EXEC-R2] eight replacements PASS; entering original /bin/true'],
 'linux':['THREAD_PROBE PASS','fd_table=ok eventfd=ok','child_tid_clear=ok'],
 'fp':['[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK','[NATIVE-STD] native PDF parser and vector raster OK'],
 'std':['[NATIVE-STD] T2 Hayro workers=4 concurrent render/join OK','[NATIVE-STD] T2 full-ledger TLS failure rollback cycles=3 cleanup=allocation-free OK','[NATIVE-STD] T2 capacity=32 EAGAIN cycles=3 join/detach/TLS recovery OK','[NATIVE-STD] collections/clocks/locks/TLS/join OK','[NATIVE-STD] shared descriptors/OOM/group exit OK','[DOCSERV] native Hayro IPC open/render/close OK'],
 'production':['[SunlightOS] Foundation Complete','[SunlightOS] sunlightd OK','[LOGIN-TUI] input ready','[SunlightOS] timezone OK'],
}[a.suite]
result={'suite':a.suite,'pass':False,'commands':commands,'guests':{}}
try:
    if a.suite=='admission': run('build',kernel+['--features','thread_admission_test'])
    elif a.suite=='t1': run('build',kernel+['--features','syscall_stack_test'])
    elif a.suite=='auth': run('build',['env','SUNLIGHT_CAP_EXEC_PROBE=1']+kernel+['--features','syscall_stack_auth_test'])
    elif a.suite=='linux': run('build',['env','SUNLIGHT_INJECT_PHASE=helios-thread-probe']+kernel+['--features','key_inject'])
    elif a.suite=='fp':
        run('users',['env',f'RUSTFLAGS=-C link-arg=-T{ROOT}/services/user-space.ld','cargo','build','--manifest-path','tools/native-std-probe/Cargo.toml','--target','tools/targets/x86_64-sunlightos.json','--bin','native-fp-probe','--release','-Z','build-std=core,alloc','-Z','json-target-spec','--offline'])
        run('build',['env','SUNLIGHT_INJECT_PHASE=native-std']+kernel+['--features','key_inject,native_std_probe'])
    elif a.suite=='std':
        run('users',['bash','tools/build_native_std.sh','build','-p','sunlight-docserv','--no-default-features','--features','native-hayro-smoke','--release','--offline'])
        run('supervisor',['env','RUSTFLAGS=-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone','cargo','build','-p','sunlightd','--release','--features','document-service-gate','--offline'])
        run('build',['env','SUNLIGHT_INJECT_PHASE=document-service']+kernel+['--features','key_inject,document_service,native_document_service'])
    else:
        run('restore-supervisor',['env','RUSTFLAGS=-C link-arg=-Tservices/user-space.ld -C relocation-model=static','cargo','build','-p','sunlightd','--release','--no-default-features','--offline'])
        run('release-build',kernel+['--release','--no-default-features'])
    elf=ROOT/'target/x86_64-unknown-none'/('release' if a.suite=='production' else 'debug')/'sunlight-kernel'
    frozen=folder/'kernel.elf'
    run('freeze',['objcopy','--strip-debug',str(elf),str(frozen)])
    result['kernel_sha256']=hashlib.sha256(frozen.read_bytes()).hexdigest()
    run('iso',['bash','tools/make_hybrid_iso.sh',str(frozen),str(folder/'boot.iso'),'target/limine',str(ROOT)])
    run('abi',['python3','tools/check_syscall_stack_abi.py',str(frozen)])
    if a.suite=='production':
        raw=frozen.read_bytes()
        forbidden=[b'[T2] begin',b'[T2-FAULT]',b'[T1] PASS',b'[T1-AUTH] PASS']
        result['production_exclusion']={m.decode(): m not in raw for m in forbidden}
        if not all(result['production_exclusion'].values()): raise RuntimeError('test hook in production image')
    def guest(cpu):
        argv=RUNNER+['guest',a.suite+f'-{cpu}cpu','--iso',str(folder/'boot.iso'),'--cpus',str(cpu),'--timeout','300']
        for marker in markers+([f'[SCHED-B] PASS cpus={cpu} '] if a.suite=='t1' else []): argv+=['--marker',marker]
        code=subprocess.run(argv,cwd=ROOT).returncode
        return cpu,code
    # Admission's 2-CPU pilot must pass before expansion.
    cpus=list(a.cpus)
    if a.suite=='admission' and 2 in cpus:
        cpu,code=guest(2); result['guests'][str(cpu)]=code
        if code: raise RuntimeError('2-CPU admission pilot failed; stop before SMP expansion')
        cpus.remove(2)
    with ThreadPoolExecutor(max_workers=2) as pool:
        for cpu,code in pool.map(guest,cpus): result['guests'][str(cpu)]=code
    result['pass']=all(code==0 for code in result['guests'].values())
finally:
    (folder/'result.json').write_text(json.dumps(result,indent=2)+'\n')
raise SystemExit(0 if result['pass'] else 1)
