#!/usr/bin/env python3
"""Build/freeze a single 2-CPU Stage 1 pilot; stop on any failed step.

Never use a passing pilot as closure of the complete eight-control matrix.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
from threading_t2_validate import identity

ROOT = Path(__file__).resolve().parents[1]
RUNNER = ['python3', 'tools/threading_t2r_validate.py']
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--case', choices=('global', 'vm', 'immediate'), required=True)
p.add_argument('--winner', choices=(0, 1), type=int, default=0)
a = p.parse_args()
case = ('global', 'vm', 'immediate').index(a.case)
folder = ROOT / '.threading-evidence/t2r' / (
    datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ') + f'-pilot-{a.case}-winner{a.winner}')
folder.mkdir(parents=True)
(folder / 'source.json').write_text(json.dumps(identity(), indent=2) + '\n')
result = {'pass': False, 'case': a.case, 'winner': a.winner, 'cpus': 2,
          'scope': 'Stage 1 pilot only', 'commands': []}
print(folder, flush=True)


def command(name, argv):
    result['commands'].append(argv)
    subprocess.run(RUNNER + ['command', '--timeout', '240', f't2r-{name}', '--'] + argv,
                   cwd=ROOT, check=True)


try:
    command('build', ['env', f'SUNLIGHT_T2R_CASE={a.case}', f'SUNLIGHT_T2R_WINNER={a.winner}',
                     'cargo', 'build', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel',
                     '--features', 'thread_lifecycle_race_test', '--offline'])
    frozen = folder / 'kernel.elf'
    command('freeze', ['objcopy', '--strip-debug',
                      'target/x86_64-unknown-none/debug/sunlight-kernel', str(frozen)])
    result['kernel_sha256'] = hashlib.sha256(frozen.read_bytes()).hexdigest()
    iso = folder / 'boot.iso'
    command('iso', ['bash', 'tools/make_hybrid_iso.sh', str(frozen), str(iso),
                    'target/limine', str(ROOT)])
    result['iso_sha256'] = hashlib.sha256(iso.read_bytes()).hexdigest()
    command('abi', ['python3', 'tools/check_syscall_stack_abi.py', str(frozen)])
    argv = RUNNER + ['guest', f'pilot-{a.case}-winner{a.winner}-2cpu', '--iso', str(iso),
                     '--cpus', '2', '--timeout', '120', '--marker',
                     f'[T2R] PILOT PASS case={case} cpus=2']
    result['commands'].append(argv)
    subprocess.run(argv, cwd=ROOT, check=True)
    result['pass'] = True
finally:
    (folder / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
