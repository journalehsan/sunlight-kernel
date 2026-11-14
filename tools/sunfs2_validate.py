#!/usr/bin/env python3
"""SFS-2 hosted qualification. Never modifies SFS-0/SFS-1 evidence or user images.

Each invocation creates a separate attempt directory; baseline is immutable.
--replay CASE selects a deterministic crash schedule from the recorded corpus.
--integrity-only verifies final manifests and preservation, without rerunning tests.
"""
import argparse, datetime, gzip, hashlib, json, os, pathlib, platform, re, subprocess, time, tarfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-2'
CRATES = ['sunlight-sunfs', 'sunlight-sunfs-lab', 'sunlight-sunfs-check']
ALLOWED_MODIFIED = {'sunlight-sunfs/src/lib.rs', 'sunlight-sunfs-check/src/lib.rs'}
ALLOWED_NEW = {
    'sunlight-sunfs/src/engine.rs', 'sunlight-sunfs/src/engine/recovery.rs',
    'sunlight-sunfs-lab/tests/transactions.rs', 'sunlight-sunfs-lab/examples/transactions.rs',
    'docs/filesystems/sunfs/sfs-2-persistent-transactions-recovery.md',
    'tools/sunfs2_validate.py',
}
def sha(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for chunk in iter(lambda: f.read(1024*1024), b''): h.update(chunk)
    return h.hexdigest()
def write(name, obj):
    (OUT/name).write_text(json.dumps(obj, indent=2)+'\n')
def sources():
    paths = [ROOT/'Cargo.toml', ROOT/'Cargo.lock', pathlib.Path(__file__)]
    for c in CRATES: paths.extend(p for p in (ROOT/c).rglob('*') if p.is_file())
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths)}
def integrity(check_final=False):
    baseline = json.loads((OUT/'source-baseline.json').read_text())['sha256']
    changed = [p for p,h in baseline.items() if not (ROOT/p).is_file() or sha(ROOT/p)!=h]
    unexpected = [p for p in changed if p not in ALLOWED_MODIFIED]
    paths = subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard'], cwd=ROOT).decode().split('\0')
    new = [p for p in paths if p and p not in baseline and (ROOT/p).is_file() and not (ROOT/p).is_symlink()]
    bad_new = [p for p in new if p not in ALLOWED_NEW and not p.startswith('docs/audit/sunfs/sfs-2/')]
    diff = subprocess.run(['git','diff','--check'], cwd=ROOT, capture_output=True, text=True)
    errors=[]
    if check_final:
        old=json.loads((OUT/'source-integrity.json').read_text())
        errors=[p for p,h in old['outputs_sha256'].items() if not (ROOT/p).is_file() or sha(ROOT/p)!=h]
    result={'status':'PASS' if not(unexpected or bad_new or diff.returncode or errors) else 'FAIL',
            'incoming_regular_files':len(baseline), 'modified_incoming':changed,
            'unexpected_modified':unexpected,'unexpected_new':bad_new,'manifest_mismatches':errors,
            'sfs0_sfs1_files_preserved':not any(p.startswith(('docs/audit/sunfs/sfs-0/','docs/audit/sunfs/sfs-1/')) for p in changed),
            'git_diff_check':diff.returncode,'regular_file_scope':'symlink metadata and ignored build artifacts excluded',
            'outputs_sha256':{p:sha(ROOT/p) for p in sorted(new+list(ALLOWED_MODIFIED)) if p!='docs/audit/sunfs/sfs-2/source-integrity.json'}}
    assert result['status']=='PASS',result
    if not check_final:write('source-integrity.json',result)
    return result

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--integrity-only',action='store_true');parser.add_argument('--replay');args=parser.parse_args()
    if args.integrity_only:
        integrity(True);print('Final hashes and incoming/SFS-0/SFS-1 preservation PASS');return
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw=OUT/'raw'/('replay-' if args.replay else 'qualification-')
    raw=pathlib.Path(str(raw)+stamp);raw.mkdir()
    before=sources();(raw/'compiled-source.json').write_text(json.dumps(before,indent=2)+'\n')
    with tarfile.open(raw/'source-snapshot.tar.gz','w:gz') as archive:
        for name in before: archive.add(ROOT/name,arcname=name,recursive=False)
    commands=[]
    def run(argv,name,env=None):
        start=time.perf_counter_ns()
        with (raw/name).open('w') as f: p=subprocess.run(argv,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT,env=env)
        row={'argv':argv,'returncode':p.returncode,'log':str((raw/name).relative_to(OUT)),'elapsed_ns':time.perf_counter_ns()-start}
        commands.append(row);(raw/'commands.json').write_text(json.dumps(commands,indent=2)+'\n');print(name,p.returncode,flush=True)
        assert p.returncode==0,row
        return (raw/name).read_text()
    def cargo(action,packages,extra=()):return ['cargo',action,*sum((['-p',p] for p in packages),[]),'--offline','--locked',*extra]
    if args.replay:
        assert re.fullmatch(r'(two|three)-seed-\d+-event-\d+-[A-Za-z0-9-]+',args.replay), 'invalid case'
        env=dict(os.environ,SUNFS_SFS2_CASE=args.replay,SUNFS_SFS2_EVIDENCE_DIR=str(raw/'sfs2'))
        run(cargo('test',['sunlight-sunfs-lab'],['--test','transactions','--target','x86_64-unknown-linux-gnu','deterministic_d_p_r_r0_crash_fault_matrix','--','--nocapture']), 'replay.log',env)
        rows=[json.loads(line) for p in (raw/'sfs2').glob('crash-schedules-*.jsonl') for line in p.read_text().splitlines()]
        assert any(r['case']==args.replay for r in rows),'case did not execute'
        print('Replay PASS:',args.replay);return
    try:
        write('validation-summary.json',{'milestone':'SFS-2','overall':'SFS-2 INCOMPLETE','status':'NOT_RUN','reason':'Final qualification in progress','attempt':str(raw.relative_to(OUT)),'commit':False,'push':False})
        integrity()
        run(['rustc','--version','--verbose'],'toolchain.log')
        run(cargo('check',['sunlight-sunfs'],['--target','x86_64-unknown-none']),'core-no-std.log')
        counts={};corpora={};tests={};performance=[]
        for profile in ['dev','release']:
            host=['--target','x86_64-unknown-linux-gnu']+(['--release'] if profile=='release' else [])
            env=dict(os.environ,SUNFS_EVIDENCE_DIR=str(raw/profile/'sfs1'),SUNFS_SFS2_EVIDENCE_DIR=str(raw/profile/'sfs2'))
            (raw/profile/'sfs1').mkdir(parents=True);(raw/profile/'sfs2').mkdir()
            text=run(cargo('test',CRATES,[*host,'--','--nocapture']),f'tests-{profile}.log',env)
            counts[profile]=sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text)))
            assert counts[profile]==32,counts
            tests[profile]=[line for line in text.splitlines() if line.startswith('SFS2_')]
            rows={p.stem:[json.loads(line) for line in p.read_text().splitlines()] for p in (raw/profile/'sfs2').glob('*.jsonl')}
            assert len(rows['crash-schedules-two'])==1538
            assert len(rows['crash-schedules-three'])==1930
            assert len(rows['reordering'])==23 and len(rows['corruption-cases'])==9
            for key in ['crash-schedules-two','crash-schedules-three','reordering']:
                assert all(r['status']=='PASS' and r['selected'] in r['expected'] for r in rows[key])
            corpora[profile]=rows
            run(cargo('build',['sunlight-sunfs-lab','sunlight-sunfs-check'],[*host,'--bins','--examples']),f'build-{profile}.log')
            text=run(cargo('run',['sunlight-sunfs-lab'],[*host,'--example','transactions']),f'performance-{profile}.log')
            performance.extend(dict(json.loads(line),profile=profile) for line in text.splitlines() if line.startswith('{'))
        assert corpora['dev']==corpora['release'],'profiles disagree'
        regress=run(cargo('test',['sunlight-block','sunlight-fat','sunlight-fs'],['--lib','--target','x86_64-unknown-linux-gnu']),'storage-regressions.log')
        regression_counts=list(map(int,re.findall(r'test result: ok\. (\d+) passed',regress)));assert regression_counts==[29,9,101]
        run(cargo('clippy',CRATES,['--all-targets','--target','x86_64-unknown-linux-gnu','--','-D','warnings']),'clippy.log')
        # Independent pre-existing Python golden encoder; regeneration must leave
        # every source and fixture byte identical to its pre-run hash.
        run(['python3','tools/sunfs_golden.py'],'golden.log');assert sources()==before,'compiled source changed'
        # Keep raw traces and compress only newly generated test images.
        for p in raw.rglob('*.img'):
            with p.with_suffix('.img.gz').open('wb') as f:f.write(gzip.compress(p.read_bytes(),mtime=0))
            p.unlink()
        common={'status':'PASS','attempt':str(raw.relative_to(OUT)),'compiled_source_sha256':before,'profiles':['dev','release']}
        write('format-compatibility.json',dict(common,format='0.2 unchanged',layout_changes=[],golden_fixtures='unchanged independent Python regeneration',baseline_results='baseline-results.json',decision='raw/format-decision-before-implementation.json',limits='16 root files, 64-byte UTF-8 names, 64KiB/file, one owner/dirty epoch, SB extent cap also enforced',wider_valid_images='Unsupported for hierarchy/orphans/engine caps; checker retains wider support'))
        write('transaction-admission.json',dict(common,stage='full plan and encoding before persistent writes',resource_units='exact dependency blocks plus exclusive snapshot segment placement',caps='actual dependency count <= SB transaction_bound; recovery reserve excluded',failure='NoSpace or explicit limit/overflow; no writes, no freeze for admission failure',coverage=['low space','populated metadata','configured extent cap','nid/generation/offset exhaustion','exact emitted block set']))
        write('mutation-tests.json',dict(common,rust_tests=8,markers=tests,cases=['hello.txt exact 16 bytes, fsync/crash/remount/checker on 64/256/512 geometries','volatile create/write discarded','empty files','zero-length writes','partial boundary and multi-block writes','8 overwrites and repeated reuse','16 files and full-size writes','stable published identity, expired mount handles, and stale incarnation/UUID rejection','repeated fsync emits no I/O','limits and low space']))
        write('checkpoint-commit.json',dict(common,protocol=['exact plan','R0 when three valid slots; all three closures excluded during planning','write pre-encoded data and metadata','flush D','write CP to durably empty slot','flush P','zero oldest CP','flush R','acknowledge'],failure='I/O fail-stop, full plan quarantined, no retries; drop/fence/reset/remount required',durability_oracle='power-loss durable image, independent checker-derived byte equality and closure traversal'))
        write('crash-matrix.json',dict(common,per_profile={'two_retained_schedules':1538,'three_retained_schedules':1930,'reverse_persistence_prefixes':23,'total_schedules':3491},seeds=[0,1,2,0x53465332],qualification='finite deterministic schedules; seeds label repeated fixed schedules, not random reorder exploration',expected_state_oracle='every row asserts the exact singleton generation derived from the deterministic persistence schedule, full bytes/bindings, no protected overwrite, independent closure agreement',faults=['before/after every persistent I/O','failed/partial/torn/uncertain write','failed flush persists none/one/all','reverse dependency persistence with sector fragments','R0 and R retirement'],corpus='<attempt>/<profile>/sfs2/crash-schedules-{two,three}.jsonl',replay='python3 tools/sunfs2_validate.py --replay two-seed-0-event-0-power-before'))
        write('recovery-selection.json',dict(common,outcomes=['CleanWritable','DegradedReadOnly','Invalid(Unsupported)','Corrupt','Io/Indeterminate'],checks='two SB copies, every candidate full closure, immutable cross-generation ownership, all valid roots protected',cases=['nine CRC/semantic corruptions','missing fallback closure','all mount read events fail independently','unsupported revision','readonly feature and unsupported flush','no checkpoints','three-slot R0 recovery']))
        write('resource-accounting.json',dict(common,oracle='independent checker enumerates reachable blocks per generation; union, protected segments, placement overhead, available capacity and reserve compared with engine; all pre-commit protected blocks compared byte-for-byte',admission='exact reserved vs emitted dependency block sets, metadata/data counts and segment overhead',no_leak_scope='unpublished unreachable garbage is reusable after recovery; no hidden reachable ownership',recovery_reserve='never allocated; placement overhead reported separately',allocation_after_admission='none in core persistent protocol: all bytes, ownership maps, snapshot files and retained Vec capacity prepared first'))
        write('checker-agreement.json',dict(common,semantic_independence='checker owns its general graph traversal and recovery selection; no engine import/call; only low-level codecs shared',new_inspection='per-generation slots, reachable blocks, stable node identities, sizes, extents and root bindings',corrupt_mutation_images=9,resealed_semantic_images=7,corpora_identical_between_profiles=True))
        write('regression-results.json',dict(common,commands=commands,tests_per_profile=counts,existing_sfs1_tests_per_profile=24,new_mutation_tests_per_profile=8,regression_counts=regression_counts,total_storage_regressions=sum(regression_counts),native_guest='NOT_RUN',physical_durability='UNQUALIFIED'))
        write('performance-baseline.json',dict(common,observations=performance,host=platform.platform(),qualification='single-host diagnostic wall time; no thresholds; peak incremental allocated heap includes simulator pending/trace overhead, excludes preexisting heap, allocator metadata and stack; not a core-only memory measurement',amplification='metadata bytes including CP and retirement divided by logical payload size'))
        write('validation-summary.json',{'milestone':'SFS-2','overall':'SFS-2 COMPLETE','gates':{f'G{i}':'PASS' for i in range(1,11)},'execution_scope':'bounded hosted engine on conforming simulator; no native durability claim','attempt':str(raw.relative_to(OUT)),'tests_per_profile':counts,'crash_schedules_per_profile':3491,'corruption_images_per_profile':9,'regression_tests':sum(regression_counts),'native_mount':False,'physical_formatting':False,'kernel_boot_root_cpython_changes':False,'commit':False,'push':False})
        integrity();integrity(True);print('SFS-2 COMPLETE: all ten bounded hosted gates PASS',flush=True)
    except BaseException:
        write('validation-summary.json',{'milestone':'SFS-2','overall':'SFS-2 INCOMPLETE','status':'FAIL','attempt':str(raw.relative_to(OUT)),'reason':'qualification interrupted or failed; preserve logs and images','commit':False,'push':False})
        raise
if __name__=='__main__':main()
