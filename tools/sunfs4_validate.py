#!/usr/bin/env python3
"""SFS-4 hosted qualification. Never modifies previous milestone evidence."""
import datetime
import hashlib
import json
import os
import gzip
import re
from concurrent.futures import ThreadPoolExecutor
import pathlib
import subprocess
import sys
import tarfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-4'
CRATES = ['sunlight-sunfs', 'sunlight-sunfs-lab', 'sunlight-sunfs-check']
MODIFIED = {'sunlight-sunfs/src/engine.rs', 'sunlight-sunfs/src/engine/recovery.rs', 'sunlight-sunfs-check/src/lib.rs'}
NEW = {'sunlight-sunfs/src/engine/cleaner.rs', 'sunlight-sunfs-check/src/relocation.rs', 'sunlight-sunfs-lab/tests/cleaning.rs', 'tools/sunfs4_validate.py', 'docs/filesystems/sunfs/sfs-4-segment-cleaner-forward-progress.md'}

def sha(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda: f.read(1024*1024), b''):
            h.update(b)
    return h.hexdigest()

def write(p, v):
    p.write_text(json.dumps(v, indent=2)+'\n')

def sources():
    paths = [ROOT/'Cargo.toml', ROOT/'Cargo.lock', pathlib.Path(__file__)]
    for crate in CRATES:
        paths += [p for p in (ROOT/crate).rglob('*') if p.is_file()]
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths)}

def cargo(action, packages=CRATES, extra=()):
    return ['cargo', action, *sum((['-p', p] for p in packages), []), '--offline', '--locked', *extra]

def run(argv, raw, name, env=None):
    start = time.monotonic_ns()
    with (raw/name).open('w') as f:
        p = subprocess.run(argv, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, env=env)
    row = dict(argv=argv, returncode=p.returncode, elapsed_ns=time.monotonic_ns()-start, log=name, cargo_target_dir=os.environ.get('CARGO_TARGET_DIR'))
    with (raw/'commands.jsonl').open('a') as f:
        f.write(json.dumps(row)+'\n')
    print(name, p.returncode, flush=True)
    if p.returncode:
        raise RuntimeError(row)
    return (raw/name).read_text()

def compress(root):
    rows=[]
    for p in sorted(root.rglob('*')):
        if p.suffix not in ('.img','.trace','.ownership'):
            continue
        b=p.read_bytes()
        q=p.with_suffix(p.suffix+'.gz')
        q.write_bytes(gzip.compress(b,mtime=0))
        rows.append(dict(path=str(q.relative_to(root)), original_bytes=len(b),original_sha256=hashlib.sha256(b).hexdigest(),compressed_sha256=sha(q)))
        p.unlink()
    write(root/'lossless-compression.json',rows)

def corpus(root,sub):
    return {p.stem:[json.loads(line) for line in p.read_text().splitlines()] for p in sorted((root/sub).glob('*.jsonl')) if p.stem!='performance'}

def integrity(verify=False):
    baseline=json.loads((OUT/'source-baseline.json').read_text())['sha256']
    changed=[p for p,h in baseline.items() if not (ROOT/p).is_file() or sha(ROOT/p)!=h]
    names=subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard'],cwd=ROOT).decode().split('\0')
    new=[p for p in names if p and p not in baseline and (ROOT/p).is_file() and not (ROOT/p).is_symlink()]
    unexpected=sorted(set(changed)-MODIFIED)
    bad_new=[p for p in new if p not in NEW and not p.startswith('docs/audit/sunfs/sfs-4/')]
    diff=subprocess.run(['git','diff','--check'],cwd=ROOT,capture_output=True,text=True)
    paths=sorted(set(changed+new)-{'docs/audit/sunfs/sfs-4/source-integrity.json'})
    outputs={p:sha(ROOT/p) for p in paths}
    result=dict(status='PASS' if not (unexpected or bad_new or diff.returncode) else 'FAIL', unexpected_modified=unexpected,unexpected_new=bad_new,git_diff_check=diff.returncode,modified_incoming=changed,prior_evidence_preserved=not any(p.startswith('docs/audit/sunfs/') for p in changed),incoming_regular_files=len(baseline),scope='all incoming tracked/unignored regular files; ignored build products and symlink metadata excluded',outputs_sha256=outputs)
    assert result['status']=='PASS',result
    if verify:
        old=json.loads((OUT/'source-integrity.json').read_text())
        assert old['outputs_sha256']==outputs,'sealed outputs changed'
    else:
        write(OUT/'source-integrity.json',result)
    return result

def qualify():
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw=OUT/'raw'/f'qualification-{stamp}'
    raw.mkdir()
    before=sources()
    write(raw/'compiled-source.json',before)
    with tarfile.open(raw/'source-snapshot.tar.gz','w:gz') as a:
        for p in before: a.add(ROOT/p,arcname=p,recursive=False)
    write(OUT/'validation-summary.json',dict(milestone='SFS-4',overall='SFS-4 INCOMPLETE',status='NOT_RUN',attempt=str(raw.relative_to(OUT))))
    run(['rustc','--version','--verbose'],raw,'toolchain.log')
    run(cargo('check',['sunlight-sunfs'],['--target','x86_64-unknown-none']),raw,'no-std.log')
    profiles={};counts={}
    for profile in ['dev','release']:
        host=['--target','x86_64-unknown-linux-gnu']+(['--release'] if profile=='release' else [])
        env=dict(os.environ)
        for sub,key in [('sfs1','SUNFS_EVIDENCE_DIR'),('sfs2','SUNFS_SFS2_EVIDENCE_DIR'),('sfs3','SUNFS_SFS3_EVIDENCE_DIR'),('sfs4','SUNFS_SFS4_EVIDENCE_DIR')]:
            p=raw/profile/sub;p.mkdir(parents=True);env[key]=str(p)
        artifacts=run(cargo('test',extra=[*host,'--no-run','--message-format=json']),raw,f'compile-tests-{profile}.log',env)
        binaries={}
        for line in artifacts.splitlines():
            if not line.startswith('{'): continue
            item=json.loads(line)
            if item.get('reason')=='compiler-artifact' and item.get('executable') and item.get('profile',{}).get('test'):
                binaries[(item['package_id'],item['target']['name'])]=item['executable']
        assert len(binaries)==10,binaries
        # Distinct test binaries write distinct SFS evidence namespaces. Build
        # once, then execute the independent hosted suites concurrently.
        with ThreadPoolExecutor(max_workers=4) as pool:
            futures=[pool.submit(run,[exe,'--nocapture'],raw,f'tests-{profile}-{i}-{name}.log',env) for i,((_,name),exe) in enumerate(sorted(binaries.items()))]
            logs=[f.result() for f in futures]
        log='\n'.join(logs)
        (raw/f'tests-{profile}.log').write_text(log)
        run(cargo('test',extra=[*host,'--doc']),raw,f'doctests-{profile}.log',env)
        counts[profile]=sum(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))
        assert counts[profile]==54,counts
        profiles[profile]={sub:corpus(raw/profile,sub) for sub in ['sfs2','sfs3','sfs4']}
        for sub in ['sfs2','sfs3']:
            assert profiles[profile][sub]==corpus(OUT/'raw/baseline'/profile,sub),f'legacy corpus changed: {profile}/{sub}'
        for sub,cs in profiles[profile].items():
            for name,rows in cs.items():
                assert all(r['status']=='PASS' for r in rows),(sub,name)
                if name.startswith('crash-') or name=='reordering':
                    assert all(r['selected'] in r['expected'] for r in rows),(sub,name)
        run(cargo('build',['sunlight-sunfs-lab','sunlight-sunfs-check'],[*host,'--bins','--examples']),raw,f'build-{profile}.log')
        compress(raw/profile)
    assert profiles['dev']==profiles['release'],'deterministic profile mismatch'
    log=run(cargo('test',['sunlight-block','sunlight-fat','sunlight-fs'],['--lib','--target','x86_64-unknown-linux-gnu']),raw,'storage-regressions.log')
    regressions=list(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))
    assert regressions==[29,9,101],regressions
    run(cargo('clippy',extra=['--all-targets','--target','x86_64-unknown-linux-gnu','--','-D','warnings']),raw,'clippy.log')
    run(['cargo','fmt',*sum((['-p',p] for p in CRATES),[]),'--','--check'],raw,'format.log')
    run(['python3','tools/sunfs_golden.py'],raw,'golden.log')
    cpus=sorted(os.sched_getaffinity(0))[:2]
    assert len(cpus)==2,'two available logical CPUs required for replay'
    replay_case=f'mixed-seed-{0x53465334}-event-0-power-before'
    replays=[]
    for cpu in cpus:
        p=raw/f'cpu-{cpu}';p.mkdir()
        env=dict(os.environ,SUNFS_SFS4_EVIDENCE_DIR=str(p),SUNFS_SFS4_CASE=replay_case)
        run(['taskset','-c',str(cpu),*cargo('test',['sunlight-sunfs-lab'],['--test','cleaning','--target','x86_64-unknown-linux-gnu','f10_interrupted','--','--nocapture'])],raw,f'cpu-{cpu}.log',env)
        replays.append(corpus(raw,f'cpu-{cpu}'))
        compress(p)
    assert replays[0]==replays[1]
    assert sum(len(rows) for rows in replays[0].values())==1
    assert before==sources(),'compiled source changed'
    common=dict(status='PASS',attempt=str(raw.relative_to(OUT)),compiled_source_sha256=before,scope='bounded hosted exclusive-owner full-snapshot cleaner')
    write(OUT/'regression-results.json',dict(common,tests_per_profile=counts,legacy_tests_per_profile=43,new_tests_per_profile=11,storage_regressions=139,storage_by_crate=regressions,no_std_alloc='PASS',locked_offline='PASS',clippy='PASS',rustfmt='PASS',golden='PASS',format_layout_changes=[]))
    sizes={sub:{k:len(v) for k,v in profiles['dev'][sub].items()} for sub in profiles['dev']}
    new=sum(len(v) for k,v in profiles['dev']['sfs4'].items() if k.startswith('crash-'))
    legacy=sum(len(v) for sub in ['sfs2','sfs3'] for k,v in profiles['dev'][sub].items() if k.startswith('crash-') or k=='reordering')
    assert legacy==10182
    write(OUT/'crash-matrix.json',dict(common,per_profile=sizes,new_cleaner_schedules_per_profile=new,legacy_schedules_per_profile=legacy,total_schedules_per_profile=new+legacy,seed=0x53465334,source_manifest=str((raw/'compiled-source.json').relative_to(OUT)),faults=['read failure','power before/after every read/write/flush','failed and partial writes 0/1/511/512/2048/4095/4096','torn byte/sector fragments','uncertain write none/all durable','flush failure none/one/all pending writes durable'],retained_cases=['two generations','three generations with R0','persistent orphan'],assertions=['exact old/new selected generation','all retained closures independently validate','exact identity/namespace/orphan/byte assertions','every initially protected block unchanged','reservation held until fenced remount','retry or explicit degraded read-only'],counting='unique scenario/event/fault tuples per profile; CPU replays not added',admission_cuts='zero-budget/refusal images plus power-before-first dependency write after full admission',native='NOT_RUN',physical_durability='UNQUALIFIED'))
    write(OUT/'cpu-independent-replay.json',dict(common,logical_cpus=cpus,case=replay_case,corpora_equal=True,unique_schedules=1,limitation='same x86_64 host on two logical CPUs, not cross-ISA or threaded concurrency'))
    fixtures=[json.loads(p.read_text()) for p in sorted((raw/'dev/sfs4/fixtures').glob('*.jsonl'))]
    assert any(r['net_reusable']>0 for r in fixtures)
    for name in ['cleaner-progress.json','cleaner-admission.json','cleaner-relocation.json']:
        write(OUT/name,dict(common,fixtures=fixtures,positive_net_reclaim=any(r['net_reusable']>0 for r in fixtures),all_requests_at_most_one_checkpoint=True,reserve='ordinary allocation keeps SB.cleaner_reserve; cleaner full plan rounded to segments fits reserve minus (2*metadata_bound+3); all retained roots excluded',no_progress='explicit finite refusal or caller-requested single deferred relocation',ownership=str((raw/'dev/sfs4/fixtures').relative_to(OUT))))
    performance=[]
    for f in fixtures:
        performance.append(dict(f,write_amplification_per_relocated_data_byte=f['bytes_written']/(f['data_blocks']*4096) if f['data_blocks'] else None,bytes_written_per_net_reclaimed_block=f['bytes_written']/f['net_reusable'] if f['net_reusable']>0 else None,metadata_bytes=f['metadata_blocks']*4096,bytes_relocated_per_net_reclaimed_block=f['relocated_blocks']*4096/f['net_reusable'] if f['net_reusable']>0 else None))
    write(OUT/'performance-baseline.json',dict(common,observations=performance,timing='command wall times only; no performance optimization or native device claim',no_net_fixture_requests=sum(r['net_reusable']<=0 for r in fixtures),reserve_high_water_blocks=max(r['reserve_used'] for r in fixtures)))
    write(OUT/'checker-agreement.json',dict(common,checker='independent codec-only closure walker and cross-image check_relocation',new_corruptions=profiles['dev']['sfs4']['corruption'],semantic_resealing='inode mutations reseal every map ancestor through checkpoint; metadata summaries store zero CRC by v0.2',runtime_pins='fixture manifests and real handle/dup/close reads; no cold-image open-count inference',quarantine='manifest forbids quarantined targets; frozen runtime quarantines entire volume',legacy_corpora_byte_identical=True))
    write(OUT/'orphan-cleaner-interaction.json',dict(common,ownership='serialized explicit reap removes nodes; cleaner only relocates mapped objects; durable CP retirement alone releases physical ownership',tested=['pinned open-unlinked read and shared offset','last close with zero I/O','eligible orphan relocated without implicit reap','distinct explicit reap exactly once','fallback orphan retained','orphan relocation crash/retry'],low_space_reap='legacy SFS-3 safe NoSpace test rerun; cleaner does not silently reap'))
    write(OUT/'modified-production-files.json',dict(common,files={p:sha(ROOT/p) for p in sorted(MODIFIED|{n for n in NEW if n.startswith(('sunlight-sunfs/src/','sunlight-sunfs-check/src/'))})},incoming_sfs3_preserved='diff against source-baseline, not HEAD',new_tests='sunlight-sunfs-lab/tests/cleaning.rs'))
    write(raw/'result.json',dict(common,tests_per_profile=counts,new_schedules=new,legacy_schedules=legacy))
    print('Qualification PASS',raw,flush=True)

def baseline():
    assert not (OUT/'source-baseline.json').exists()
    names = subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard'], cwd=ROOT).decode().split('\0')
    hashes = {n:sha(ROOT/n) for n in names if n and not n.startswith('docs/audit/sunfs/sfs-4/') and (ROOT/n).is_file() and not (ROOT/n).is_symlink()}
    raw = OUT/'raw/baseline'
    write(OUT/'source-baseline.json', dict(revision=subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT).decode().strip(), branch=subprocess.check_output(['git','branch','--show-current'], cwd=ROOT).decode().strip(), status=subprocess.check_output(['git','status','--porcelain=v1'], cwd=ROOT).decode(), sha256=hashes))
    before=sources()
    write(raw/'compiled-source.json',before)
    with tarfile.open(raw/'source-snapshot.tar.gz','w:gz') as a:
        for name in before:
            a.add(ROOT/name,arcname=name,recursive=False)
    run(cargo('check',['sunlight-sunfs'],['--target','x86_64-unknown-none']),raw,'no-std.log')
    for profile in ['dev','release']:
        host=['--target','x86_64-unknown-linux-gnu']+(['--release'] if profile=='release' else [])
        env=dict(os.environ)
        for sub, key in [('sfs1','SUNFS_EVIDENCE_DIR'),('sfs2','SUNFS_SFS2_EVIDENCE_DIR'),('sfs3','SUNFS_SFS3_EVIDENCE_DIR')]:
            p=raw/profile/sub
            p.mkdir(parents=True)
            env[key]=str(p)
        run(cargo('test',extra=[*host,'--','--nocapture']),raw,f'{profile}.log',env)
        run(cargo('build',['sunlight-sunfs-lab','sunlight-sunfs-check'],[*host,'--bins','--examples']),raw,f'build-{profile}.log')
    assert before==sources()
    write(raw/'result.json',dict(status='PASS',source_sha256=before))

if __name__ == '__main__':
    if sys.argv[1:]==['--baseline']:
        baseline()
    elif sys.argv[1:]==['--qualify']:
        qualify()
    elif sys.argv[1:]==['--seal']:
        assert sources()==json.loads((OUT/'regression-results.json').read_text())['compiled_source_sha256'],'final compiled source changed'
        integrity();integrity(True);print('Integrity sealed PASS')
    elif sys.argv[1:]==['--integrity-only']:
        integrity(True);print('Integrity verified PASS')
    else:
        raise SystemExit('use --baseline, --qualify, --seal or --integrity-only')
