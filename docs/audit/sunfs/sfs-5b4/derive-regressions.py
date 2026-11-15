#!/usr/bin/env python3
"""Select only completed final-source runs; retain every other attempt."""
import hashlib
import json
from pathlib import Path
import re

OUT=Path(__file__).resolve().parent
ROOT=OUT.parents[3]
FINAL_INPUTS = ['kernel/Cargo.toml', 'kernel/build.rs', 'kernel/src/main.rs', 'kernel/src/arch/x86_64/interrupts.rs', 'kernel/src/arch/x86_64/syscall.rs', 'sunlight-virtio/src/blk.rs', 'kernel/src/sfs_reset.rs', 'kernel/src/sfs5b4_test.rs', 'tools/helios-probes/sfs5b4-probe.S', 'tools/sunfs5b4_validate.py', 'Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'rust-toolchain.toml']


def read(p): return json.loads(p.read_text())
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def put(n,v): (OUT/n).write_text(json.dumps(v,indent=2)+'\n')


def qualifies(raw):
    integrity=raw/'input-integrity.json'
    before=raw/'source-before.json'
    if not integrity.is_file() or not before.is_file(): return False
    if read(integrity)['status']!='PASS': return False
    frozen=read(before)
    return all(frozen.get(n)==sha(ROOT/n) for n in FINAL_INPUTS)


def main():
    candidates=sorted((OUT/'raw').glob('regression-20*'))
    raw=next((p for p in reversed(candidates) if (p/'hosted-result.json').is_file() and qualifies(p)),None)
    if raw is None: raise SystemExit('Final-source regression still pending')
    commands=read(raw/'hosted-result.json')
    for row in commands:
        if row['returncode']!=0: raise AssertionError(row)
    checks=[json.loads(line) for line in (raw/'commands.jsonl').read_text().splitlines()]
    assert len(checks)==10, checks
    assert all(r['returncode']==0 for r in checks)
    counts={}
    for key,file in [('sunfs_debug','hosted-debug.log'),('sunfs_release','hosted-release.log'),('storage','storage.log'),('kernel','kernel-lifecycle.log')]:
        counts[key]=sum(int(n) for n in re.findall(r'test result: ok\. (\d+) passed',(raw/file).read_text()))
    corpus={};hashes={}
    for profile in ['debug','release']:
        corpus[profile]={};hashes[profile]={}
        for subsystem in ['sfs2','sfs3','sfs4']:
            files=sorted((raw/profile/subsystem).glob('*.jsonl'))
            files=[p for p in files if p.name.startswith('crash-') or p.name=='reordering.jsonl']
            rows=[json.loads(line) for p in files for line in p.read_text().splitlines() if line]
            assert all(r.get('status')=='PASS' for r in rows)
            assert all('expected' not in r or r.get('selected') in r['expected'] for r in rows)
            corpus[profile][subsystem]=dict(cases=len(rows),all_pass=True,selected_expected=True)
            hashes[profile].update({str(p.relative_to(raw/profile)):sha(p) for p in files})
        assert sum(s['cases'] for s in corpus[profile].values())==13142
    assert hashes['debug']==hashes['release']
    native={}
    for milestone,expected in [('sfs5b1',12),('sfs5b2',6),('sfs5b3',18)]:
        options=sorted((OUT/'raw').glob(f'regression-native-{milestone}-20*'))
        chosen=next((p for p in reversed(options) if (p/'native-result.json').exists() and qualifies(p) and len(read(p/'native-result.json').get('boots',[]))==expected),None)
        if chosen is None: raise SystemExit(f'final {milestone} native replay still pending')
        rows=read(chosen/'native-result.json')['boots']
        assert all(r['status']=='PASS' for r in rows)
        native[milestone]=dict(boots=len(rows),status='PASS',result=str((chosen/'native-result.json').relative_to(OUT)),
            qualification='SFS5A/5B1 normal/cold-restart' if milestone=='sfs5b1' else 'existing reset/held-request scope; not true async or new-epoch qualification')
    put('regression-results.json',dict(status='PARTIAL',selected_completed_final_source_commands='PASS',run=str(raw.relative_to(OUT)),
        counts=counts,commands=commands,checks=checks,crash_corpus=corpus,crash_corpus_hashes=hashes,debug_release_corpora_equal=True,
        debug_reuse=read(raw/'debug-reuse.json') if (raw/'debug-reuse.json').exists() else None,
        native_replays=native,input_integrity=str((raw/'input-integrity.json').relative_to(OUT)),
        historical_failures=['raw/early-boot-stall.json','raw/cargo-clean-recovery.json','raw/observer-gap-v1/finding.json'],
        missing='required full native fault/interleaving and retirement matrix remains incomplete; see native-fault-matrix.json',
        reproduced_staged_defects=['raw/staged-reset-provenance-red.log','raw/staged-admission-red.log'],
        green='raw/staged-final-green.log',physical_hardware='UNQUALIFIED'))
    attempts=[]
    for p in sorted((OUT/'raw').iterdir()):
        if not p.is_dir() or not (p/'source-before.json').exists(): continue
        native_result=p/'native-result.json'
        attempts.append(dict(attempt=p.name,input_integrity=read(p/'input-integrity.json') if (p/'input-integrity.json').exists() else 'NOT_FINISHED',
            qualifies_final_source=qualifies(p),native_status=read(native_result).get('status') if native_result.exists() else None,
            native_boots=len(read(native_result).get('boots',[])) if native_result.exists() else 0,
            device_model_qualification='see raw/observer-gap-v1/finding.json; v1 QEMU runs excluded from final model qualification' if p.name.startswith('native-') else 'separate prior milestone scope'))
    put('raw/attempt-ledger.json',dict(attempts=attempts,rule='input FAIL and pre-repair runs remain excluded even if scoped assertions passed'))
    print('PASS available final regressions',counts,native)


if __name__=='__main__': main()
