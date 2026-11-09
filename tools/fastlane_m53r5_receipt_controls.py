#!/usr/bin/env python3
"""Compile the actual quota module and reject safe receipt duplication/forgery.

These are compile-time controls, never native runtime misuse evidence.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    quota = ROOT / 'kernel/src/memory/shared_quota.rs'
    deps = ROOT / 'target/x86_64-unknown-linux-gnu/debug/deps'
    libraries = sorted(deps.glob('libspin-*.rlib'))
    if not libraries:
        raise RuntimeError('build kernel host tests first to supply actual spin dependency')
    prefix = '#[allow(dead_code)]\n#[path=' + json.dumps(str(quota)) + '] mod quota;\n'
    cases = {
        'valid-move': ('pub fn keep(r: quota::Reservation) -> quota::Reservation { r }', None),
        'clone-denied': ('fn bound<T: Clone>() {} pub fn attack() { bound::<quota::Reservation>(); }', 'E0277'),
        'copy-denied': ('fn bound<T: Copy>() {} pub fn attack() { bound::<quota::Reservation>(); }', 'E0277'),
        'second-move-denied': ('pub fn attack(r: quota::Reservation) { let a = r; let b = r; drop((a,b)); }', 'E0382'),
        'forged-fields-denied': ('pub fn attack() { let _ = quota::Reservation { owner: quota::ProcessIdentity {pid: 1, incarnation: 1}, session: None, bytes: 4096 }; }', 'E0451'),
    }
    rows = []
    for name, (body, error) in cases.items():
        source = args.out / (name.replace('-', '_') + '.rs')
        source.write_text(prefix + body + '\n')
        command = ['rustc','--edition=2021','--crate-type=lib','--crate-name',name.replace('-','_'),
                   '--extern','spin='+str(libraries[0]),'-L','dependency='+str(deps),
                   str(source),'-o',str(args.out/(name+'.rlib'))]
        r = subprocess.run(command,cwd=ROOT,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=60)
        (args.out/(name+'.log')).write_text(r.stdout)
        passed = r.returncode == 0 if error is None else r.returncode != 0 and ('error['+error+']') in r.stdout
        rows.append({'name':name,'command':command,'expected_error':error,'exit_code':r.returncode,'status':'PASS' if passed else 'FAIL'})
    result = {'scope':'actual-source compile-time controls only','quota_source_sha256':hashlib.sha256(quota.read_bytes()).hexdigest(),
              'spin_rlib':str(libraries[0]),'spin_sha256':hashlib.sha256(libraries[0].read_bytes()).hexdigest(),
              'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'cases':rows,
              'status':'PASS' if all(r['status']=='PASS' for r in rows) else 'FAIL'}
    (args.out/'validation-summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(result['status'])
    return int(result['status']!='PASS')

if __name__ == '__main__':
    raise SystemExit(main())
