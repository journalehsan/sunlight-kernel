#!/usr/bin/env python3
"""Seal/check source identity, incoming preservation and SFS5B4 artifacts."""
import argparse
import difflib
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]
REPORT = ROOT/'docs/filesystems/sunfs/sfs-5b4-async-dma-scheduler-fencing.md'
CHANGED = ['kernel/Cargo.toml','kernel/build.rs','kernel/src/main.rs','kernel/src/arch/x86_64/interrupts.rs','kernel/src/arch/x86_64/syscall.rs','sunlight-virtio/src/blk.rs']
ADDED = ['kernel/src/sfs_reset.rs','kernel/src/sfs5b4_test.rs','tools/helios-probes/sfs5b4-probe.S','tools/sunfs5b4_validate.py']


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def write(p, v):
    p.write_text(json.dumps(v, indent=2)+'\n')


def command(argv):
    return subprocess.check_output(argv, cwd=ROOT)


def seal():
    assert '{{FINAL_RESULTS}}' not in REPORT.read_text()
    before = json.loads((OUT/'raw/incoming-manifest.json').read_text())
    changed, missing = [], []
    for name, digest in before.items():
        p = ROOT/name
        if not p.is_file(): missing.append(name)
        elif sha(p) != digest: changed.append(name)
    unexpected = sorted(set(changed)-set(CHANGED))
    assert not unexpected and not missing, (unexpected, missing)
    assert set(changed)==set(CHANGED), changed
    prior = [n for n in before if n.startswith(('docs/audit/sunfs/','docs/filesystems/sunfs/'))]
    assert all(sha(ROOT/n)==before[n] for n in prior)
    final = {}
    delta = ''
    for name in CHANGED+ADDED+['Cargo.toml','Cargo.lock','.cargo/config.toml','rust-toolchain.toml']:
        p = OUT/'raw/final-source'/name
        p.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(ROOT/name,p)
        final[name] = sha(p)
        if name in CHANGED:
            candidate = ROOT/'docs/audit/sunfs/sfs-5b3/raw/final-source'/name
            if candidate.is_file() and sha(candidate)==before[name]: old=candidate.read_bytes()
            else:
                old=command(['git','show',f'HEAD:{name}'])
                assert hashlib.sha256(old).hexdigest()==before[name], name
            b = OUT/'raw/baseline-source'/name
            b.parent.mkdir(parents=True,exist_ok=True);b.write_bytes(old)
            delta+=''.join(difflib.unified_diff(old.decode().splitlines(True),(ROOT/name).read_text().splitlines(True),fromfile='a/'+name,tofile='b/'+name))
        elif name in ADDED:
            delta+=''.join(difflib.unified_diff([], (ROOT/name).read_text().splitlines(True),fromfile='/dev/null',tofile='b/'+name))
    (OUT/'raw/milestone-source.patch').write_text(delta)
    write(OUT/'raw/final-source-manifest.json',final)
    (OUT/'raw/final-status.txt').write_bytes(command(['git','status','--porcelain=v1','-uall']))
    head=command(['git','rev-parse','HEAD']).decode().strip()
    baseline=json.loads((OUT/'source-baseline.json').read_text())
    assert head==baseline['revision']
    compilation=[]
    for pattern in ['target/x86_64-unknown-none/debug/sunlight-kernel.d','target/x86_64-unknown-none/debug/.fingerprint/sunlight-kernel-*/*.json']:
        for src in ROOT.glob(pattern):
            p=OUT/'raw/compilation-records'/src.relative_to(ROOT)
            p.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,p)
            compilation.append(dict(path=str(src.relative_to(ROOT)),sha256=sha(src)))
    write(OUT/'raw/compilation-inputs.json',dict(records=compilation,
        flags_file='.cargo/config.toml',toolchain='raw/rustc-version.txt',cargo='raw/cargo-version.txt',
        actual_invocations='per native invocation.json and regression commands.jsonl',
        boundary='post-build depfiles/fingerprints; per-invocation ELF/ISO and source manifests authoritative; not a hermetic generated-input attestation'))
    required=['source-baseline.json','transport-assumptions.md','qemu-backend-design.md','qemu-source-integrity.json','device-event-graph.json',
        'dma-reset-overlap.json','negative-access-controls.json','scheduler-coordinator.json','smp-contention.json','forward-progress.json',
        'fencing-decision.json','dma-retirement.json','queue-epoch.json','native-fault-matrix.json','durability-uncertainty.json',
        'remaining-blockers.json','regression-results.json','production-changes.json','validation-summary.json']
    assert all((OUT/n).is_file() for n in required)
    records={str(p.relative_to(OUT)):sha(p) for p in OUT.rglob('*') if p.is_file() and not p.is_symlink()
        and p.name not in ['source-integrity.json','evidence-seal.json']}
    write(OUT/'evidence-seal.json',dict(files=records,report=dict(path=str(REPORT.relative_to(ROOT)),sha256=sha(REPORT))))
    write(OUT/'source-integrity.json',dict(status='PASS',revision=head,branch=baseline['branch'],
        incoming_regular_files_checked=len(before),prior_sfs_files_preserved=len(prior),unexpected_changes=unexpected,missing=missing,
        modified_existing=changed,added_sources=ADDED,final_manifest='raw/final-source-manifest.json',
        evidence_seal='evidence-seal.json',evidence_seal_sha256=sha(OUT/'evidence-seal.json'),
        source_delta='raw/milestone-source.patch',commit=False,push=False))
    print('SEALED',len(records),'artifacts;',len(prior),'prior SFS files preserved')


def check():
    integrity=json.loads((OUT/'source-integrity.json').read_text())
    assert sha(OUT/'evidence-seal.json')==integrity['evidence_seal_sha256']
    manifest=json.loads((OUT/'evidence-seal.json').read_text())
    errors=[n for n,h in manifest['files'].items() if not (OUT/n).is_file() or sha(OUT/n)!=h]
    assert not errors, errors
    final=json.loads((OUT/'raw/final-source-manifest.json').read_text())
    assert all(sha(ROOT/n)==h for n,h in final.items())
    before=json.loads((OUT/'raw/incoming-manifest.json').read_text())
    assert all((ROOT/n).is_file() and sha(ROOT/n)==h for n,h in before.items() if n not in CHANGED)
    assert sha(ROOT/manifest['report']['path'])==manifest['report']['sha256']
    assert command(['git','rev-parse','HEAD']).decode().strip()==integrity['revision']
    print('PASS: final source, evidence seal, report, incoming files, prior evidence and HEAD')


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--seal',action='store_true');args=parser.parse_args()
    seal() if args.seal else check()
