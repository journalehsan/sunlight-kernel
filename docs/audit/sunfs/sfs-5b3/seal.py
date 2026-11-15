#!/usr/bin/env python3
"""Seal completed evidence and verify only the intended incoming files changed."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent
REPORT = ROOT/'docs/filesystems/sunfs/sfs-5b3-active-io-fencing.md'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    baseline = json.loads((OUT/'source-baseline.json').read_text())['files']
    changed = [n for n, expected in baseline.items() if not (ROOT/n).is_file() or sha(ROOT/n) != expected]
    allowed = {'kernel/Cargo.toml','kernel/src/main.rs','kernel/src/arch/x86_64/interrupts.rs'}
    assert set(changed) == allowed, changed
    native = sorted((OUT/'raw').glob('native-*/native-result.json'))[-1]
    result = json.loads(native.read_text())
    assert len(result['boots']) == 18 and all(row['status']=='PASS' for row in result['boots'])
    native_inputs = json.loads((native.parent/'source-before.json').read_text())
    names = subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard'],cwd=ROOT).decode().split('\0')
    current = {n:sha(ROOT/n) for n in names if n and (ROOT/n).is_file() and not (ROOT/n).is_symlink()
               and not n.startswith(('docs/audit/','docs/filesystems/'))}
    assert current == native_inputs, 'final execution-input mismatch'
    for pattern in ['native-*/input-integrity.json','hosted-*/input-integrity.json','checks-*/input-integrity.json']:
        path = sorted((OUT/'raw').glob(pattern))[-1]
        assert json.loads(path.read_text())['status']=='PASS', str(path)
    assert json.loads((OUT/'validation-summary.json').read_text())['status']=='PARTIAL'
    required = ['source-baseline.json','transport-threat-model.md','normative-fencing-contract.md',
                'instrumented-backend.json','qemu-source-identity.json','active-io-witness.json',
                'dma-access-witness.json','reset-lifecycle.json','queue-epoch-closure.json',
                'dma-retirement.json','quarantine-accounting.json','host-fault-matrix.json',
                'native-fault-matrix.json','smp-progress.json','durability-uncertainty.json',
                'remaining-blockers.json','regression-results.json','production-changes.json',
                'validation-summary.json']
    assert all((OUT/name).is_file() for name in required)
    sealed = {str(p.relative_to(ROOT)):sha(p) for p in OUT.rglob('*') if p.is_file()
              and p.name != 'source-integrity.json' and '__pycache__' not in p.parts}
    for name in [*changed,'kernel/src/sfs5b3_test.rs','tools/sunfs5b3_validate.py',str(REPORT.relative_to(ROOT))]:
        sealed[name] = sha(ROOT/name)
    prior = [n for n in baseline if n.startswith(('docs/audit/sunfs/','docs/filesystems/sunfs/'))]
    value = dict(status='PASS', captured_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                 changed_incoming_files=changed, preserved_prior_sunfs_files=len(prior),
                 incoming_regular_files=len(baseline), execution_inputs=current, sealed_files=sealed,
                 exclusions=['source-integrity.json is the seal itself', 'ignored build outputs and symlinks are not in the baseline',
                             'QEMU /tmp source and binary have separate identity manifests and per-run binary hashes'],
                 earlier_failed_attempt='raw/development-attempts.json',
                 verifier='python3 docs/audit/sunfs/sfs-5b3/verify-integrity.py')
    (OUT/'source-integrity.json').write_text(json.dumps(value,indent=2)+'\n')
    print('sealed',len(sealed),'files; preserved',len(prior),'prior SunFS files')


if __name__ == '__main__':
    main()
