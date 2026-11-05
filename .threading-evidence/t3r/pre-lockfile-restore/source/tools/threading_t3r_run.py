#!/usr/bin/env python3
"""Source-bound T3R migration fixture: host gates, 2-CPU pilot, then 4/12.
All attempts are retained; any failed pilot stops expansion.
"""
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
from threading_t3_validate import identity

ROOT = Path(__file__).resolve().parents[1]
RUNNER = ['python3','tools/threading_t3r_validate.py']

def main():
    folder=ROOT/'.threading-evidence/t3r'/(datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')+'-migration')
    folder.mkdir(parents=True)
    source=identity();(folder/'source.json').write_text(json.dumps(source,indent=2)+'\n')
    result={'pass':False,'commands':[],'guests':{},'stage3':'NOT_RUN'}
    def command(name,argv,timeout=600):
        result['commands'].append(argv)
        subprocess.run(RUNNER+['command','--timeout',str(timeout),name,'--']+argv,cwd=ROOT,check=True)
    try:
        command('topology-host-build',['rustc','--edition=2021','--test','kernel/src/arch/x86_64/cpu_topology.rs','-o','target/t3r-topology-test'])
        command('topology-host',['target/t3r-topology-test'])
        command('kernel-host',['cargo','test','-p','sunlight-kernel','--lib','--target','x86_64-unknown-linux-gnu','--offline'])
        command('compat-host',['cargo','test','-p','sunlight-compat-linux','--target','x86_64-unknown-linux-gnu','--offline'])
        command('parser-controls',['python3','-m','unittest','discover','-s','tools','-p','test_threading_t*.py','-v'])
        command('migration-build',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--features','thread_migration_test','--offline'])
        frozen=folder/'kernel.elf'
        command('freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/debug/sunlight-kernel',str(frozen)])
        command('abi',['python3','tools/check_syscall_stack_abi.py',str(frozen)])
        iso=folder/'boot.iso'
        command('iso',['bash','tools/make_hybrid_iso.sh',str(frozen),str(iso),'target/limine',str(ROOT)])
        result.update(elf_sha256=hashlib.sha256(frozen.read_bytes()).hexdigest(),iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest())
        if identity() != source: raise RuntimeError('source changed during build')
        for cpu in (2,4,12):
            argv=RUNNER+['guest',f'migration-{cpu}cpu','--iso',str(iso),'--cpus',str(cpu),'--timeout','120','--marker','[T3R] PASS']
            result['commands'].append(argv)
            code=subprocess.run(argv,cwd=ROOT).returncode
            result['guests'][str(cpu)]='PASS' if code == 0 else 'FAIL'
            if code: raise RuntimeError(f'{cpu}-CPU migration failed; expansion stopped')
            if cpu==2: result['stage3']='RUNNING'
        result['stage3']='PASS';result['pass']=True
    finally:
        (folder/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    return 0 if result['pass'] else 1

if __name__=='__main__': raise SystemExit(main())
