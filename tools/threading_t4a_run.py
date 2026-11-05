#!/usr/bin/env python3
"""T4A source-bound fixtures. Race expansion requires a passing 2-CPU pilot."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
from threading_t4a_validate import ROOT, BASE, identity, run


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('suite',choices=('races','std','linux','production'))
    parser.add_argument('--cpus',type=int,nargs='+')
    a=parser.parse_args()
    cpus=a.cpus or ([2,4,12] if a.suite=='races' else [2,1,4,12] if a.suite=='std' else [2])
    if any(c not in (1,2,4,12) for c in cpus) or a.suite=='races' and (2 not in cpus or 1 in cpus):
        parser.error('races requires 2-CPU pilot; locked rendezvous needs distinct CPUs')
    folder=BASE/(datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')+'-suite-'+a.suite)
    folder.mkdir(parents=True)
    source=identity();(folder/'source.json').write_text(json.dumps(source,indent=2)+'\n')
    result={'pass':False,'suite':a.suite,'attempts':[],'guests':{}}
    def command(name,argv,timeout=900):
        ok,attempt=run(a.suite+'-'+name,argv,timeout)
        result['attempts'].append(str(attempt.relative_to(ROOT)))
        if not ok: raise RuntimeError(f'{name} failed; evidence {attempt}')
    kernel=['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--offline']
    try:
        if a.suite=='races':
            command('host-kernel',['cargo','test','-p','sunlight-kernel','--lib','--target','x86_64-unknown-linux-gnu','--offline'])
            command('host-linux',['cargo','test','-p','sunlight-compat-linux','--target','x86_64-unknown-linux-gnu','--offline'])
            command('host-scheduler',['python3','tools/test_scheduler_host.py'])
            command('receipts',['python3','-m','unittest','discover','-s','tools','-p','test_threading_t4a.py','-v'])
            command('build',kernel+['--features','atomic_futex_test'])
        elif a.suite=='std':
            command('users',['bash','tools/build_native_std.sh','build','-p','sunlight-docserv','--no-default-features','--features','native-hayro-smoke','--release','--offline'])
            command('supervisor',['env','RUSTFLAGS=-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone','cargo','build','-p','sunlightd','--release','--features','document-service-gate','--offline'])
            command('kernel',['env','SUNLIGHT_INJECT_PHASE=document-service']+kernel+['--features','key_inject,document_service,native_document_service'])
        elif a.suite=='linux':
            command('build',['env','SUNLIGHT_INJECT_PHASE=helios-thread-probe']+kernel+['--features','key_inject'])
        else:
            command('restore-supervisor',['env','RUSTFLAGS=-C link-arg=-Tservices/user-space.ld -C relocation-model=static','cargo','build','-p','sunlightd','--release','--no-default-features','--offline'])
            command('debug',kernel+['--no-default-features'])
            command('release',kernel+['--release','--no-default-features'])
        frozen=folder/'kernel.elf'
        profile='release' if a.suite=='production' else 'debug'
        command('freeze',['objcopy','--strip-debug',str(ROOT/f'target/x86_64-unknown-none/{profile}/sunlight-kernel'),str(frozen)])
        command('abi',['python3','tools/check_syscall_stack_abi.py',str(frozen)])
        iso=folder/'boot.iso'
        command('iso',['bash','tools/make_hybrid_iso.sh',str(frozen),str(iso),'target/limine',str(ROOT)])
        result.update(kernel_sha256=hashlib.sha256(frozen.read_bytes()).hexdigest(),iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest())
        if identity()!=source: raise RuntimeError('source changed while building')
        markers={
            'races':[],
            'std':['[NATIVE-STD] T4A mutex/condvar handoffs=1536 independent=4 progress/TLS/resources OK','[NATIVE-STD] collections/clocks/locks/TLS/join OK','[NATIVE-STD] T2 Hayro workers=4 concurrent render/join OK','[NATIVE-STD] shared descriptors/OOM/group exit OK','[DOCSERV] native Hayro IPC open/render/close OK'],
            'linux':['THREAD_PROBE PASS','fd_table=ok eventfd=ok','child_tid_clear=ok'],
            'production':['[SunlightOS] Foundation Complete','[SunlightOS] sunlightd OK','[LOGIN-TUI] input ready','[SunlightOS] timezone OK'],
        }[a.suite]
        if a.suite=='production':
            forbidden=[b'T4A ',b'[T4A]',b't4a_fixture',b'[T3R] begin',b'[T2] begin']
            result['production_exclusion']={m.decode(): m not in frozen.read_bytes() for m in forbidden}
            if not all(result['production_exclusion'].values()): raise RuntimeError('test hooks in production')
        def guest(cpu):
            ok,attempt=run(a.suite+f'-{cpu}cpu',[],300,iso,cpu,markers,a.suite=='races')
            return cpu,ok,str(attempt.relative_to(ROOT))
        remaining=list(cpus)
        if 2 in remaining:
            cpu,ok,attempt=guest(2);result['guests'][str(cpu)]={'pass':ok,'attempt':attempt};remaining.remove(2)
            if not ok: raise RuntimeError('2-CPU pilot failed; native expansion stopped')
        with ThreadPoolExecutor(max_workers=2) as pool:
            for cpu,ok,attempt in pool.map(guest,remaining):result['guests'][str(cpu)]={'pass':ok,'attempt':attempt}
        result['pass']=all(g['pass'] for g in result['guests'].values())
    finally:
        (folder/'result.json').write_text(json.dumps(result,indent=2)+'\n')
        print(folder,flush=True)
    return 0 if result['pass'] else 1


if __name__=='__main__':raise SystemExit(main())
