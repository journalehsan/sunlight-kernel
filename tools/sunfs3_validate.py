#!/usr/bin/env python3
"""Hosted SFS-3 qualification; never writes SFS-0/SFS-1/SFS-2 evidence.

The filesystem and checker remain Rust. This driver records commands, hashes,
source snapshots, raw test evidence and machine-readable qualification results.
"""
import argparse
import datetime
import gzip
import hashlib
import json
import os
import pathlib
import re
import subprocess
import tarfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-3'
CRATES = ['sunlight-sunfs', 'sunlight-sunfs-lab', 'sunlight-sunfs-check']
MODIFIED = {'sunlight-sunfs/src/engine.rs', 'sunlight-sunfs/src/engine/recovery.rs',
            'sunlight-sunfs-check/src/lib.rs'}
NEW = {'sunlight-sunfs/src/engine/namespace.rs', 'sunlight-sunfs/src/engine/handles.rs',
       'sunlight-sunfs-lab/tests/namespace.rs', 'tools/sunfs3_validate.py',
       'docs/filesystems/sunfs/adr/0003-sfs3-bounded-namespace-lifetime.md',
       'docs/filesystems/sunfs/sfs-3-namespace-orphan-lifetime.md'}

def sha(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for chunk in iter(lambda: f.read(1024*1024), b''):
            h.update(chunk)
    return h.hexdigest()

def write(name, value):
    (OUT/name).write_text(json.dumps(value, indent=2)+'\n')

def sources():
    paths = [ROOT/'Cargo.toml', ROOT/'Cargo.lock', pathlib.Path(__file__)]
    for crate in CRATES:
        paths.extend(p for p in (ROOT/crate).rglob('*') if p.is_file())
    paths.append(ROOT/'docs/filesystems/sunfs/adr/0003-sfs3-bounded-namespace-lifetime.md')
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths)}

def integrity(verify=False):
    baseline = json.loads((OUT/'source-baseline.json').read_text())['sha256']
    changed = [p for p,h in baseline.items() if not (ROOT/p).is_file() or sha(ROOT/p)!=h]
    unexpected = sorted(set(changed)-MODIFIED)
    names = subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard'],cwd=ROOT).decode().split('\0')
    new = [p for p in names if p and p not in baseline and (ROOT/p).is_file() and not (ROOT/p).is_symlink()]
    bad_new = [p for p in new if p not in NEW and not p.startswith('docs/audit/sunfs/sfs-3/')]
    diff = subprocess.run(['git','diff','--check'],cwd=ROOT,capture_output=True,text=True)
    mismatches = []
    if verify:
        old = json.loads((OUT/'source-integrity.json').read_text())
        mismatches = [p for p,h in old['outputs_sha256'].items() if not (ROOT/p).is_file() or sha(ROOT/p)!=h]
        assert set(new+changed)-{'docs/audit/sunfs/sfs-3/source-integrity.json'} == set(old['outputs_sha256']), 'output file set changed'
    result = dict(status='PASS' if not(unexpected or bad_new or diff.returncode or mismatches) else 'FAIL',
                  incoming_regular_files=len(baseline), modified_incoming=changed,
                  unexpected_modified=unexpected, unexpected_new=bad_new, manifest_mismatches=mismatches,
                  prior_evidence_preserved=not any(p.startswith('docs/audit/sunfs/') for p in changed),
                  git_diff_check=diff.returncode,
                  scope='all incoming tracked/unignored regular files; symlink metadata and ignored build artifacts excluded',
                  outputs_sha256={p:sha(ROOT/p) for p in sorted(set(new+changed)) if p!='docs/audit/sunfs/sfs-3/source-integrity.json'})
    assert result['status']=='PASS', result
    if not verify:
        write('source-integrity.json', result)
    return result

def cargo(action, packages=CRATES, extra=()):
    return ['cargo',action,*sum((['-p',p] for p in packages),[]),'--offline','--locked',*extra]

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--integrity-only', action='store_true')
    parser.add_argument('--seal', action='store_true', help='seal final report/evidence edits after qualification')
    parser.add_argument('--replay')
    args=parser.parse_args()
    if args.integrity_only:
        integrity(True); print('Final source/evidence and incoming preservation PASS'); return
    if args.seal:
        assert sources()==json.loads((OUT/'regression-results.json').read_text())['compiled_source_sha256'], 'compiled source changed'
        integrity(); integrity(True); print('Final report/evidence sealed PASS'); return
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw=OUT/'raw'/f'{"replay" if args.replay else "qualification"}-{stamp}'
    raw.mkdir()
    before=sources()
    (raw/'compiled-source.json').write_text(json.dumps(before,indent=2)+'\n')
    with tarfile.open(raw/'source-snapshot.tar.gz','w:gz') as archive:
        for name in before:
            archive.add(ROOT/name,arcname=name,recursive=False)
    commands=[]
    def run(argv,name,env=None):
        started=time.perf_counter_ns()
        with (raw/name).open('w') as f:
            p=subprocess.run(argv,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT,env=env)
        commands.append(dict(argv=argv,returncode=p.returncode,log=str((raw/name).relative_to(OUT)),elapsed_ns=time.perf_counter_ns()-started))
        (raw/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
        print(name,p.returncode,flush=True)
        assert p.returncode==0,commands[-1]
        return (raw/name).read_text()
    if args.replay:
        assert re.fullmatch(r'[a-z-]+-seed-1397117747-event-\d+-[A-Za-z0-9-]+',args.replay)
        env=dict(os.environ,SUNFS_SFS3_CASE=args.replay,SUNFS_SFS3_EVIDENCE_DIR=str(raw/'sfs3'))
        run(cargo('test',['sunlight-sunfs-lab'],['--test','namespace','--target','x86_64-unknown-linux-gnu','namespace_d_p_r_r0_crash_fault_matrix','--','--nocapture']),'replay.log',env)
        rows=[json.loads(line) for p in (raw/'sfs3').glob('crash-*.jsonl') for line in p.read_text().splitlines()]
        assert any(r['case']==args.replay for r in rows),'case did not execute'
        compress(raw)
        print('Replay PASS',args.replay); return
    try:
        write('validation-summary.json',dict(milestone='SFS-3',overall='SFS-3 INCOMPLETE',status='NOT_RUN',attempt=str(raw.relative_to(OUT)),reason='qualification in progress',commit=False,push=False))
        integrity()
        run(['rustc','--version','--verbose'],'toolchain.log')
        run(cargo('check',['sunlight-sunfs'],['--target','x86_64-unknown-none']),'core-no-std.log')
        counts={}; corpora={}; performance=[]; all_cases={}
        for profile in ['dev','release']:
            host=['--target','x86_64-unknown-linux-gnu']+(['--release'] if profile=='release' else [])
            for sub in ['sfs1','sfs2','sfs3']:
                (raw/profile/sub).mkdir(parents=True)
            env=dict(os.environ,SUNFS_EVIDENCE_DIR=str(raw/profile/'sfs1'),SUNFS_SFS2_EVIDENCE_DIR=str(raw/profile/'sfs2'),SUNFS_SFS3_EVIDENCE_DIR=str(raw/profile/'sfs3'))
            log=run(cargo('test',extra=[*host,'--','--nocapture']),f'tests-{profile}.log',env)
            counts[profile]=sum(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))
            assert counts[profile]==43, counts
            rows={f'{sub}/{p.stem}':[json.loads(line) for line in p.read_text().splitlines()] for sub in ['sfs2','sfs3'] for p in (raw/profile/sub).glob('*.jsonl') if p.stem!='performance'}
            for key,corpus in rows.items():
                assert all(row['status']=='PASS' for row in corpus), key
                if 'crash' in key or 'reordering' in key:
                    assert all(row['selected'] in row['expected'] for row in corpus),key
            assert len(rows['sfs2/crash-schedules-two'])==1538
            assert len(rows['sfs2/crash-schedules-three'])==1930
            assert len(rows['sfs2/reordering'])==23
            assert len(rows['sfs2/corruption-cases'])==9
            assert len(rows['sfs3/reordering'])==75
            assert len(rows['sfs3/corruption'])==7
            cases={key:len(value) for key,value in rows.items()}
            assert sum(count for key,count in cases.items() if key.startswith('sfs3/crash-'))==6616, cases
            all_cases[profile]=cases;corpora[profile]=rows
            performance.extend(dict(json.loads(line),profile=profile) for line in (raw/profile/'sfs3/performance.jsonl').read_text().splitlines())
            run(cargo('build',['sunlight-sunfs-lab','sunlight-sunfs-check'],[*host,'--bins','--examples']),f'build-{profile}.log')
        assert corpora['dev']==corpora['release'],'profiles disagree'
        log=run(cargo('test',['sunlight-block','sunlight-fat','sunlight-fs'],['--lib','--target','x86_64-unknown-linux-gnu']),'storage-regressions.log')
        regressions=list(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))
        assert regressions==[29,9,101],regressions
        run(cargo('clippy',extra=['--all-targets','--target','x86_64-unknown-linux-gnu','--','-D','warnings']),'clippy.log')
        run(['cargo','fmt','-p','sunlight-sunfs','-p','sunlight-sunfs-lab','-p','sunlight-sunfs-check','--','--check'],'format.log')
        run(['python3','tools/sunfs_golden.py'],'golden.log')
        assert sources()==before,'compiled source changed'
        compress(raw)
        common=dict(status='PASS',attempt=str(raw.relative_to(OUT)),profiles=['dev','release'],compiled_source_sha256=before)
        write('format-compatibility.json',dict(common,format='experimental 0.2 unchanged',layout_changes=[],codecs_formatter_geometry_golden='byte-identical to incoming baseline',unsupported_versions='legacy rejection test rerun in both profiles',decision='docs/filesystems/sunfs/adr/0003-sfs3-bounded-namespace-lifetime.md',old_engine='SFS-2 explicitly rejects hierarchies/orphans; existing checker format semantics already support them'))
        write('namespace-model.json',dict(common,identity=['volume UUID','nid','incarnation'],binding='single-link (parent NodeId, name bytes); no independent dirent number',limits=dict(regular_objects_including_orphans=16,nonroot_directories_including_orphans=16,entries_per_directory=16,depth=8,input_components=64,component_bytes=64,file_bytes=65536,orphans='min(16, SB.maximum_orphans)',descriptions=32,handles=64),checks=['existing-component dot/dot-dot type checking','root dot-dot clamps','UTF-8, slash/NUL/dot entry rejection','duplicate names','cycles','parent relationships','subtree resulting depth','empty rmdir','SB and transaction limits'],directory_pins='UNSUPPORTED; selectors/enumeration snapshots are not openat capabilities',symlinks='UNSUPPORTED'))
        write('handle-lifetime.json',dict(common,token='opaque mount-session plus monotonic nonreused registration; copied token is not dup',description='NodeId, mode, shared offset and checked descriptor reference count',dup='new registered token shares description',independent_open='new description and offset',identity_does_not_grant_authority='legacy selectors are trusted internal APIs; VFS owns user rights',runtime_pins='one object pin per independent description, released on final descriptor close',stale='closed/remounted/copied-close tokens reject, new names/NIDs never alias',tests=['read/write modes','independent/shared offsets','64 descriptor cap','32 description cap','close orders and duplicate close','remount expiry','open destination replacement']))
        write('rename-replacement.json',dict(common,atomicity='one working namespace epoch and one full-volume checkpoint for both parents, replacement and nlinks',supported=['same-directory','cross-directory','open file','open destination','directory subtree move','empty directory replacement','self rename no-op'],rejections=['cycle','resulting depth','type mismatch','nonempty destination directory','entry/orphan cap'],durable_oracle='exact handcrafted paths, node identities, bytes and orphan records; old/new singleton generation per schedule'))
        write('orphan-state-machine.json',dict(common,states=['Linked: one binding, nlink file=1 or dir=2+subdirs','Unlinked: no binding, nlink=0, persistent map/orphan record in next checkpoint','Pinned: zero-link object has independent description references','Pending reap: no descriptions, same complete persistent backing','Reaped from current map: ordinary admitted checkpoint omits node/record','Physical reusable: no retained closure or quarantine owns segment'],last_close='consumes one token, no I/O or physical free; duplicate close returns Stale',unique_final_owner='exclusive reap transaction removes each candidate once; allocator is the only physical placement path',io_failure='freeze and quarantine; no retry; reset/remount rebuilds candidates',admission_failure='working objects/dirty flag restored, no persistent I/O',orphan_table_exhaustion='safe Limit before binding mutation; lower SB cap and 16-record full-table tested'))
        write('orphan-recovery.json',dict(common,mount='read-only full validation, no writes, zero volatile handles restored',reaping='explicit reap_orphans transaction; writable only; all cold orphans eligible',old_fallback='remains fully validated/protected after current map drops orphan',crash_scenarios=['before/during unlink commit','after unlink while open','before last-close retirement commit','during orphan reap','after publication','during R/R0'],no_leak='eligible orphans removed from selected map after successful explicit reap and remount; fallback-only backing expires by normal checkpoint rotation',low_space='explicit NoSpace with objects intact; no progress guarantee when full-snapshot admission cannot fit'))
        new_schedules=sum(v for k,v in all_cases['dev'].items() if k.startswith('sfs3/crash-'))
        write('crash-matrix.json',dict(common,per_profile=all_cases,new_namespace_fault_schedules=new_schedules,new_reverse_prefixes=75,legacy_sfs2_schedules=3491,total_mutation_schedules_per_profile=new_schedules+75+3491,legacy_sfs1='112 formatter cuts, 480 fault cases, 4096 malformed inputs retained in separate per-profile evidence',seed=0x53465333,seed_scope='one fixed deterministic namespace schedule seed; labels are not random exploration',faults=['before/after every write and flush','failed writes','prefixes 0,1,511,512,2048,4095,4096','torn byte/sector fragments','uncertain writes none/all persisted','flush failure none/one/all persisted','reverse dependency prefixes including sector fragments'],assertions=['D acknowledged complete generation','P exact independently derived old/new generation','R newest full recoverable closure and checker/engine agreement','R0 no protected overwrite, duplicate plan block or early reuse','cold eligible-orphan reaping when writable'],thread_races='NOT_RUN; exclusive synchronous owner; sequential reader/dup/close interleavings only',future_concurrency_plan=['retain in-flight operation pins before releasing outer VFS locks','barrier-controlled reader vs unlink/replace','concurrent dup-close and last-close/reap with one final owner','independent progress and native I/O fencing witnesses'],replay='python3 tools/sunfs3_validate.py --replay replace-seed-1397117747-event-0-power-before'))
        write('resource-accounting.json',dict(common,physical_oracle='checker independently enumerates every complete generation; union/segments/current/fallback-only/reserve/free compared to engine; all initial protected bytes unchanged',runtime_oracle='handcrafted open/dup/close operation counts; checker does not invent volatile reference counts',runtime_backing='owned in-memory bytes, not pointers into retired disk blocks; full snapshots copy every remaining object including pinned orphans',reservation='exact complete snapshot encoding before I/O, disjoint from every retained closure, quarantine held on failure',separate_sets=['namespace nodes','open descriptions and descriptor references','orphan objects/pins/candidates','current CP blocks','retained-only blocks','pending plan blocks','placement overhead','cleaner reserve','available/reclaimable capacity'],reclaimable_definition='available whole-segment capacity minus separate reserve, saturating at zero; not cleaner progress',limits='no block sharing across current objects, no direct close/free path, no cleaner'))
        write('checker-agreement.json',dict(common,independence='semantic checker imports codecs only, never engine recovery/namespace/lifetime traversal',extension='exposes parent/nlink/all-directory entries/orphan records for independent test oracles',existing_validation='full graph, cycle/hardlink, parent, links, orphan separation, extents, segment summaries, immutable retained closure identity and fallback',new_crc_resealed_corruptions=7,legacy_sfs2_corruptions=9,profile_corpora_equal=True,volatile_handles='not observable from a cold image'))
        write('regression-results.json',dict(common,commands=commands,tests_per_profile=counts,legacy_sfs1_tests=24,legacy_sfs2_tests=8,new_sfs3_tests=11,storage_regression_counts=regressions,total_storage_regressions=sum(regressions),core_no_std='PASS',clippy='PASS',formatting='PASS',golden='PASS unchanged',security_review='bounded input validation, modes/opaque tokens, stale references, cycle/orphan accounting and fail-stop review plus CRC-valid malformed-image tests; not external penetration testing',native_guest='NOT_RUN',physical_durability='UNQUALIFIED'))
        write('performance-baseline.json',dict(common,observations=performance,qualification='single-host simulator timings and exact emitted bytes; no optimization or hardware prediction',checkpoint_model='full-volume rewrite; metadata includes CP publication and old slot invalidation',last_close_extra_io=0,orphan_retirement='one additional admitted full-volume commit when eligible; no commit if no candidates'))
        write('validation-summary.json',dict(milestone='SFS-3',overall='SFS-3 COMPLETE',gates={f'G{i}':'PASS' for i in range(1,11)},execution_scope='bounded hosted exclusive-owner engine; finite deterministic schedules',attempt=str(raw.relative_to(OUT)),tests_per_profile=counts,new_sfs3_tests_per_profile=11,new_namespace_schedules_per_profile=new_schedules+75,legacy_sfs2_schedules_per_profile=3491,all_mutation_schedules_per_profile=new_schedules+75+3491,storage_regressions=139,thread_races='NOT_RUN',native_guest='NOT_RUN',physical_durability='UNQUALIFIED',directory_pins_openat='UNSUPPORTED',commit=False,push=False))
        report(raw,counts,new_schedules,performance)
        integrity();integrity(True)
        print('SFS-3 COMPLETE: G1-G10 PASS for bounded hosted scope',flush=True)
    except BaseException:
        write('validation-summary.json',dict(milestone='SFS-3',overall='SFS-3 INCOMPLETE',status='FAIL',attempt=str(raw.relative_to(OUT)),reason='qualification interrupted or failed; preserve logs, images and sources',commit=False,push=False))
        raise

def compress(raw):
    manifest={}
    for p in sorted(raw.rglob('*')):
        if p.suffix not in ('.img','.trace'):
            continue
        original=p.read_bytes(); destination=p.with_suffix(p.suffix+'.gz')
        assert not destination.exists(),destination
        compressed=gzip.compress(original,mtime=0)
        assert gzip.decompress(compressed)==original
        destination.write_bytes(compressed)
        manifest[str(destination.relative_to(OUT))]=dict(original_sha256=hashlib.sha256(original).hexdigest(),compressed_sha256=sha(destination),original_bytes=len(original),compressed_bytes=len(compressed))
        p.unlink()
    (raw/'lossless-compression.json').write_text(json.dumps(manifest,indent=2)+'\n')

def report(raw,counts,schedules,performance):
    release=[r for r in performance if r['profile']=='release']
    table='\n'.join(f"| {r['operation']} | {r['metadata_bytes_including_cp_retirement']} | {r['data_bytes']} | {r['flushes']} | {r['elapsed_ns']} | {r['protected_blocks']} |" for r in release)
    (ROOT/'docs/filesystems/sunfs/sfs-3-namespace-orphan-lifetime.md').write_text(f'''# SunFS SFS-3 namespace and orphan lifetime

2026-11-14. **SFS-3 COMPLETE: G1–G10 PASS in the bounded hosted scope.**
Machine-readable verdict: [validation-summary.json](../../audit/sunfs/sfs-3/validation-summary.json).
No native mount, physical disk formatting, guest validation, boot/root migration,
Python filesystem port, SunApp packaging, commit or push occurred.

## Baseline and design decision

Incoming branch `main`, revision `24aa8186b8abd55099727d37d3da53e04d83c6be`, clean worktree.
[source-baseline.json](../../audit/sunfs/sfs-3/source-baseline.json) hashes 103,418
incoming regular files. All SFS-0/SFS-1/SFS-2 evidence stays byte-identical. Read the
current architecture, format, recovery/integration contracts, ADRs 0001/0002, prior
reports, codecs, formatter, transaction admission, retained closures, checker and
simulator before editing engine code. The unchanged baseline passed core no_std,
32 tests in each profile and hosted bins/examples. Raw baseline logs/corpora reside
in `raw/baseline-retry/`. An earlier runner omitted legacy evidence directories;
its four output-file errors are retained under `raw/baseline/`, not erased.

[ADR 0003](adr/0003-sfs3-bounded-namespace-lifetime.md) records the preimplementation
decision. **No on-disk layout/version/feature change.** Existing v0.2 explicitly
specifies hierarchical directories, directory parents/nlink, orphan records and
mapped zero-link nodes. The engine now supports that existing representation.
Codecs, formatter, geometry and four independently regenerated golden fixtures
remain byte-identical. Unsupported version rejection still passes the legacy tests.
An SFS-2 engine explicitly rejects hierarchies/orphans; no compatibility upgrade or
silent old-engine reinterpretation is claimed.

## Identity and API

| Concept | Owner and identity |
|---|---|
| Persistent node | `(volume UUID, nid, incarnation)`, monotonic NID, no recycling |
| Directory binding | `(parent NodeId, name bytes)` names one node, no dirent number |
| Open description | Private registration: NodeId, mode, offset, descriptor references |
| Public handle | Opaque mount-session/token, monotonically allocated, never reused |
| Physical extent | Owner/logical block/content generation/segment epoch, separate from name |
| Checkpoint backing | Independently protected union of every complete retained closure |
| Orphan | No binding, nlink=0, mapped complete backing and persistent orphan record |

`root`, `mkdir(parent,name)`, `create_at`, `lookup_at`, `lookup_components`,
`entries_at`, `rename`, `unlink`, `rmdir`, `sync_all`, `reap_orphans` extend the
existing create/lookup/read/write/fsync APIs. Directory listing is a fresh bounded
snapshot, not a retained cookie. Rename changes identity neither for source files
nor directories. Internal objects carry binding fields separately from NodeId;
file parent associations generate directory entries, while the file inode parent
field stays zero as v0.2 requires. Directories serialize their actual parent.

Legacy `FileId` is a trusted linked-node selector, not a capability or lifetime pin.
It remains usable through rename, becomes Stale after unlink, and expires on remount.
Its public identity fields do not grant user authority. Offset-based reads/writes
explicitly address a linked object without changing an open description's position.
The eventual VFS adapter owns credentials, ancestor search/mount checks, public
process descriptor tables and in-flight operation pins. It must map authorized
pins to this single engine lifetime policy, with no parallel reclamation policy.

`open(selector, mode)`, `read_handle`, `write_handle`, `seek`, `position`, `dup`,
`close`, `fsync_handle` implement minimal regular-file descriptions. Independent
opens have independent offsets. Dup creates a registered token sharing mode and
offset; cloning an opaque token does not register another reference. A copied token
can be consumed only once. Duplicate close and closed/remounted tokens return Stale.
Read/write access modes are enforced; they do not implement user authentication.

## Namespace bounds and rename

Keep 16 regular objects total, including orphans; 64-byte UTF-8 names; 65,536 bytes
per file and the configured extent limit. Add 16 nonroot directory objects including
orphans, 16 entries per directory, depth 8 (root depth 0; files count), 64 supplied
components, min(16, SB orphan cap), 32 descriptions and 64 registered handles.
Superblock node, orphan and exact transaction bounds still apply. All 33 nodes,
16 full orphan records, and 64-byte names are exercised together. A maximum-size
combination can exceed the configured transaction bound and safely return NoSpace.
Nothing inflates that bound or consumes cleaner reserve to make a test pass.

Each component walk checks the current directory before interpreting dot/dot-dot;
missing/.. and regular-file/.. fail. Root .. clamps to root. Stored names cannot
be dot, dot-dot, slash, NUL, invalid UTF-8 or empty. No lexical symlink emulation,
symlinks, hardlinks, traversal outside the root or recursive graph walk exists.
Creation checks resulting depth. Rename checks every moved descendant's resulting
depth and rejects moving a directory below itself. Duplicates and bounds are checked
before binding changes. Empty rmdir and same-kind empty-directory replacement work;
nonempty destination replacement and file/directory type mismatch fail.

Rename changes source/destination bindings, replacement orphan state, parent and
link counts together under the exclusive mutable owner. One full-volume checkpoint
publishes that epoch. It never executes two independently durable directory updates.
An open source retains its identity; an open overwritten destination retains its
original node while new lookup resolves to the source node. Same-binding rename is
a no-op. Path selectors do not retain removed directories: directory opens, pinned
relative lookup/openat and retained dot-dot parent contracts are explicitly unsupported.

## Open-unlink, last-close and reaping

Unlink removes the binding and queues a zero-link orphan, after reserving orphan
record capacity. The record stores unlink generation/reason=1 and all backing stays
in the node map/segment ownership. Even a closed unlinked object first becomes a
pending orphan; no special direct-free path exists. Capacity exhaustion returns
Limit before namespace mutation. Existing descriptions retain read/write access.
Creating a new object under the same name allocates a new NID; old handles cannot
reach it. Data writes through an unlinked handle participate in full-volume fsync.

Closing a token consumes one descriptor reference. Only the final reference to a
description removes its object pin; other independent descriptions still pin it.
Last-close queues eligibility, emits zero I/O, and never frees a physical block.
`reap_orphans` removes all eligible objects and their records in one normal admitted
checkpoint. It skips pinned orphans, returns zero without committing if none qualify,
and safely rejects duplicate/stale close. NoSpace restores working objects/dirty
state and emits no writes. Uncertain I/O freezes the owner and retains the complete
reservation/quarantine. Close may still consume references in that frozen owner.

Reaping removes current-map ownership only. Older published generations retain their
complete backing independently; allocation excludes every retained closure until
its slot is durably invalidated. The single allocator is the only physical reuse
owner. Runtime reads use owned memory, not disk pointers, and every snapshot copies
all remaining objects including pinned orphans. Thus handles do not require another
physical retention allocator or hidden backing outside the checkpoint union.

Mount is read-only inspection. It validates every persistent orphan with all other
closure invariants, restores zero volatile descriptions, and exposes every cold
orphan as a reap candidate. **Recovery reaping is explicit**, via `reap_orphans` on
a writable owner; mount does not perform a hidden cleanup write. After that commit
and remount, selected-map orphans are gone; fallback generations still validate.
Read-only/degraded mounts never reap. No cleaner or guaranteed progress at full
snapshot/reserve exhaustion is claimed; safe NoSpace can defer retirement.

## Persistence and independent resource proof

The SFS-2 protocol is unchanged: complete plan/encoding, optional oldest-slot R0,
dependency writes, flush D, checkpoint write, flush P, oldest-slot zero, flush R,
then acknowledgment. All planning excludes the initial union even if R0 later
retires one root. No publication-time allocation or direct orphan free is added.
Failures freeze mutation rather than retrying uncertain I/O.

Engine accounting separately reports current checkpoint blocks, fallback-only union,
pending reserved blocks, protected segments, placement overhead, available capacity,
cleaner reserve and reclaimable capacity after reserve. Runtime accounting separates
namespace nodes, orphans, pinned orphans, eligible candidates, descriptions and tokens.
Reclaimable capacity describes whole-segment allocation eligibility, not GC progress.

The independent checker continues its own semantic traversal, importing codecs only.
It already validates tree reachability/parents/cycles, names, identities, links,
orphan separation, extents/data, exact summaries/bitmaps, duplicate ownership and
fallback eligibility. New inspection fields expose every directory's entries,
parent/nlink and persistent orphans. Test oracles reconstruct paths and file bytes
from those physical references, compare handcrafted expected identities/data/orphans,
and recompute union/current/fallback/segment/free/reserve counts. They compare every
initially protected block byte-for-byte and reject duplicate planned allocation.
Cold images cannot reveal runtime handles; live pin assertions use separate known
open/dup/close sequences. No cold checker invents open-reference counts.

## Final qualification

Final attempt: `{raw.relative_to(OUT)}`. Compiled source SHA256 and a source archive
bind the tested bytes. Debug/release each passed **{counts['dev']} tests**: all 24 SFS-1,
all eight SFS-2, and eleven SFS-3 tests. Existing storage regressions: **139 PASS**
(block 29, FAT 9, VFS 101). Core x86_64-unknown-none check, both hosted builds,
Clippy with warnings denied, scoped rustfmt and unchanged golden regeneration pass.
Only the existing unrelated duplicate startup-fixture build-target warning remains.

Each profile executes **{schedules} new namespace fault schedules**, plus **75** new
reverse-dependency/sector-prefix schedules, and all **3,491** legacy SFS-2 schedules:
**{schedules+75+3491} mutation schedules per profile**. The legacy SFS-1 formatter
112 crash cuts/480 faults and 4096 malformed-input campaign also rerun; these are
separate evidence, not added to the mutation count. Profile corpora match exactly.
Seven new CRC-resealed malformed namespace/orphan images and all nine SFS-2 corrupt
images pass expected degraded fallback checks; no repair writes occur.

Fifteen new scenarios cover mkdir/create/write, same/cross rename, open replacement,
directory move/replacement, unlink/read, orphan write, rmdir, last-close retirement,
cold reaping and three-retained replacement/reaping. Every persistent write/flush
has before/after cuts, failed/prefix/torn/uncertain writes, and failed barriers with
none/one/all pending writes durable. D/P/R/R0 assertions require exact independently
derived generation, complete namespace/bytes/orphans, engine/checker recovery
agreement and protected backing safety. Writable recovered orphans are actually
reaped and rechecked after another crash. This is finite deterministic coverage,
not exhaustive arbitrary hardware reorder exploration. Thread races are NOT_RUN;
sequential active-reader/dup-close schedules are not labeled thread concurrency.
A future barrier-controlled race plan is retained in crash-matrix.json.

Development observations remain separate: omitted baseline output directories,
8 MiB fixtures correctly rejected by snapshot/reserve admission, a Clippy test
Drop warning, and an interrupted qualification while correcting the transient peak
retained-backing observation. Each is preserved separately before final qualification. Earlier passing snapshots do not
validate later source. Raw traces/images are losslessly gzip-compressed with
verified original/compressed hashes; commands, fault rows and source hashes remain.

## Metadata and performance observations

The 256-block-segment fixture has two small regular files and three nonroot
directories. These release simulator observations are diagnostic, not hardware
latency forecasts or optimized results. Metadata includes checkpoint/retirement.

| Operation | Metadata bytes | Data bytes copied | Flushes | Elapsed ns | Protected blocks |
|---|---:|---:|---:|---:|---:|
{table}

Each nonempty orphan table adds one 4 KiB record page plus its mandatory 4 KiB root;
records are 32 bytes, bounded to 16 (512 bytes of records). Last-close adds no writes.
Eligible retirement costs another whole-volume checkpoint. Traversal timing is
captured for 10,000 two-component lookups, alongside peak retained block counts,
in [performance-baseline.json](../../audit/sunfs/sfs-3/performance-baseline.json).
Peak retained backing is independently reconstructed from pre-commit closures plus
the new private closure that coexisted at flush P, before retirement. It includes
the temporary three-generation union, not just the two post-commit roots.
No memory allocator or cleaner optimization is included.

## Security, SunApp and remaining scope

Review checked bounded names/counts/walks, checked generation/token arithmetic,
opaque mode-restricted registrations, stale identity rejection, transactional
replacement/orphan capacity, input typing and fail-stop ownership. CRC is accidental
corruption detection, not authentication. Global allocator OOM retains the inherited
fail-stop contract; recoverable heap exhaustion is unqualified. The hosted mode
model supplies no multiuser credentials, process FD table, POSIX locking or native
VFS integration. Runtime device/session exclusivity and reset fencing remain adapter
obligations; actual driver honesty/DMA quiescence are unqualified.

[SunApp contract](../../audit/sunfs/sfs-3/sunapp-installation-contract.md) reviews
write/verify/fsync/rename/directory-fsync activation and old-open backing. Atomic
regular-file replacement is a useful foundation. Nonempty directory replacement
is unsupported, so directory package activation needs a future indirection or
exchange contract. Filesystem rename does not authenticate packages or isolate apps.

Remaining limits are full-snapshot memory/write amplification and whole-segment
tail waste, bounded objects, explicit reaping, no full-disk progress guarantee,
no directory pins/openat, no symlinks/hardlinks/truncate, no thread race or native
physical durability qualification. Recommend **SFS-4: bounded foreground cleaner,
relocation and reserve/progress proof**, preserving identities, runtime ownership
and every retained closure, with independent crash/resource oracles before tuning.
Directory pin/VFS integration needs its own retained-relative-operation contract.

## Deliverables, gates and reproduction

Implementation: `sunlight-sunfs/src/engine.rs`, `engine/namespace.rs`,
`engine/handles.rs`, `engine/recovery.rs`; checker inspection:
`sunlight-sunfs-check/src/lib.rs`; hosted tests:
`sunlight-sunfs-lab/tests/namespace.rs`; driver: `tools/sunfs3_validate.py`.
All requested JSON artifacts, raw evidence and the SunApp review are in
[docs/audit/sunfs/sfs-3](../../audit/sunfs/sfs-3/). The final source integrity manifest
checks all incoming files and hashes every changed/new output except itself.

| Gate | Result | Basis |
|---|---|---|
| G1 | PASS | unchanged baseline, format/fixture preservation |
| G2 | PASS | bounded tree/dot/type/depth/entry and maximum-object tests |
| G3 | PASS | one-checkpoint rename/replacement and exact crash namespace oracles |
| G4 | PASS | modes, shared/independent offsets, stale/cap tests |
| G5 | PASS | old-open/new-name identities, close order, retained backing |
| G6 | PASS | persistent orphan states, explicit cold reap, no early reuse |
| G7 | PASS | finite hosted D/P/R/R0 matrix; thread races NOT_RUN |
| G8 | PASS | independent full-closure traversal and reconstructed accounting |
| G9 | PASS | final builds, 43 tests/profile, 139 regressions, lint and scoped review |
| G10 | PASS | compiled-source/output hashes, earlier evidence preservation |

Run `python3 tools/sunfs3_validate.py` for a fresh isolated qualification.
Replay: `python3 tools/sunfs3_validate.py --replay replace-seed-1397117747-event-0-power-before`.
Verify: `python3 tools/sunfs3_validate.py --integrity-only`.
''')

if __name__=='__main__':
    main()
