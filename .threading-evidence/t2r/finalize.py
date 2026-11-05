import difflib
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
sys.path.insert(0, 'tools')
from threading_t2_validate import identity
from threading_t2_acceptance import RACES

base = Path('.threading-evidence/t2r')
subprocess.run([sys.executable, str(base/'audit-receipts.py')], check=True, stdout=subprocess.DEVNULL)
all_rows = json.loads((base/'pilot-receipt-audit.json').read_text())
proofs = [r for r in all_rows if r['eligible_final_pilot']]
assert {(r['case'],r['winner']) for r in proofs} == {(0,'0'),(0,'1'),(1,'0'),(1,'1'),(2,'0')}
summary = {'status':'PARTIAL', 'head':identity()['head'],
           'initial_canonical_sha256':json.loads((base/'initial/source.json').read_text())['canonical_sha256'],
           'original_control_ids':list(RACES), 'scope':'Safer-test subset after user steering; original acceptance criteria retained.',
           'controls':{}, 'pilot_proofs':proofs, 'checks':{},
           'smp_matrix':{'1':'NOT_RUN','2':'quota pilots and one immediate-exit order only','4':'NOT_RUN','12':'NOT_RUN'},
           'acceptance':{'status':'PARTIAL','missing_controls':[r for r in RACES if r not in ('final_global_slot','final_vm_slot','immediate_child_exit')],
                         'missing_orders':['immediate_child_exit/child_exit_before_creator_return'],
                         'missing_regressions':['final-source full race matrix on 1/2/4/12 CPUs'],
                         'smallest_next_executable_proof':'Two-CPU creation versus parent exit, with exact creator/owner/child generations and each legal serialization order.'}}
for name in RACES:
    case={'final_global_slot':0,'final_vm_slot':1,'immediate_child_exit':2}.get(name)
    rows=[r for r in proofs if r['case']==case] if case is not None else []
    summary['controls'][name]={'native_2cpu':'PILOT_PASSED_BOTH_WINNERS' if case in (0,1) else 'ONE_ORDER_PILOT_PASSED' if case==2 else 'NOT_EXERCISED',
                               'classification':'NATIVE_WITNESS' if rows else 'MISSING_NATIVE_WITNESS',
                               'proofs':[r['attempt'] for r in rows], 'original_scope_complete':False}

def latest(pattern):
    return sorted(base.glob(pattern))[-1]
for key,pattern in {
 'admission_host':'*admission-host-final','allocation_host':'*allocation-host-final',
 'admission_bore_host':'*admission-bore-host','kernel_host':'*kernel-host',
 'scheduler_host':'*scheduler-host','descriptor_host':'*descriptor-host','linux_host':'*linux-host',
 'native_thread_api_host':'*native-thread-api-host','native_mapping_api_host':'*native-mapping-api-host',
 'parser_controls':'*parsers-final-source','baremetal_check':'*baremetal-final',
 'format_changed':'*format-final-source','format_workspace':'*format-workspace-final',
 'production_debug_build':'*production-debug-final','production_release_gate':'*suite-production',
 't1_native_prior_to_scope_change':'*suite-t1','linux_native':'*suite-linux','std_hayro_native':'*suite-std',
 'strict_cargo_clippy':'*Z-clippy','fixture_clippy_final':'*fixture-clippy-final',
}.items():
    p=latest(pattern)/'result.json';r=json.loads(p.read_text());summary['checks'][key]={'result':str(p.relative_to(base)), 'pass':r['pass']}
summary['clippy_classification']=json.loads((base/'fixture-clippy-final-detail/classification.json').read_text())
summary['resources']=[]
for row in proofs:
    text=(base/row['attempt']/'serial.log').read_text()
    def one(pattern):return re.findall(pattern,text,re.M)[0]
    b,boxes,cache,reaped=map(int,one(r'^\[T2R\] heap baseline=(\d+) cached_fd_boxes=(\d+) cached_fd_bytes=(\d+) reaped_stacks=(\d+) drift=0$'))
    tid,inc,slot,progress=map(int,one(r'^\[T2R\] retired tid=(\d+) incarnation=(\d+) slot=(\d+) state=Reaped stack=none pinned=false progress=(\d+)$'))
    summary['resources'].append({'attempt':row['attempt'],'baseline_heap_bytes':b,'cached_fd_boxes':boxes,'cached_fd_bytes':cache,
      'final_heap_bytes':b+cache,'reaped_worker_stacks':reaped,'live_owner_stacks':4,'workers_global':0,'workers_per_vm':[0]*4,
      'vm_owners':4,'borrowers_per_vm':[0]*4,'occupied_task_slots':4,'retained_task_slots':boxes+4,
      'fd_references':12,'capability_entries':4,'reserved_admissions':0,'pending_construction':0,
      'reservation_measurement':'No persistent ticket registry exists; local reservation lifetime is under SCHEDULER. Quiescent zero follows the source contract.',
      'target_child':{'tid':tid,'incarnation':inc,'slot':slot,'state':'Reaped','stack':None,'pinned':False},'unrelated_owner_progress':progress})

p=Path('docs/kernel/threading-t2r-lifecycle-race-closure.md');doc=p.read_text()
doc=doc.replace('| Pilot pending |','| Pilot pending |')
doc=doc.replace('| Pilot pending |','| Pilot pending |')
doc=doc.replace('last global worker slot | Pilot pending','last global worker slot | 2 CPUs: both winners pass')
doc=doc.replace('last per-VM worker slot | Pilot pending','last per-VM worker slot | 2 CPUs: both winners pass')
doc=doc.replace('Immediate exit after publication | Pilot pending','Immediate exit after publication | 2 CPUs: creator-return-first order passes')
section='''## Recorded native results and reconciliation

The final strict parser and compiled-source audit accept five pilot guests:

| Control | Winner/order | Exact guest receipt |
| --- | --- | --- |
'''
for r in proofs:
 section+=f"| {('final_global_slot','final_vm_slot','immediate_child_exit')[r['case']]} | {'actor '+r['winner'] if r['case']!=2 else 'creator return before child exit'} | `{r['attempt']}` |\n"
section+='''
All are 2-CPU QEMU TCG guests. Each quota guest records two distinct CPU
entrants before taking the lifecycle guard, exactly one complete publication,
exactly one recoverable raw -2 rejection, and the generation-qualified child's
Reaped/no-stack/no-pin receipt. Independent VM owners report progress; actor
progress is excluded from the final progress count. The immediate-exit guest's
creator success report precedes child exit entry; the reverse order is missing.

| Guest kind | Warm baseline | Reaped worker stacks | Cached FD bytes | Final heap |
| --- | ---: | ---: | ---: | ---: |
| Global, each winner | 2,051,968 | 96 | 983,040 | 3,035,008 |
| Per-VM, each winner | 2,051,968 | 32 | 327,680 | 2,379,648 |
| Immediate exit | 2,051,968 | 1 | 10,240 | 2,062,208 |

The baseline is measured after four owners and bounded task/queue storage are
prepared, before any worker creation. Every remaining byte is attributed to
one retained 10,240-byte FD-table box per Reaped worker slot. The final heap
must equal that baseline plus those boxes. Owners remain live until the single
terminal receipt: four occupied tasks/stacks, 12 FD references and four original
capability entries. All four per-VM worker/borrower counts and the global worker
count are zero. Every Reaped slot has no stack or physical pin, and every queue
membership is unique. Inherited worker state is freed, not counted as cache.
Reserved/pending zero is a source-contract statement: there is no persistent
reservation registry; local transaction state cannot survive the guard scope.
It is not an independently sampled ticket counter. No aggregate balance alone
is promoted to a missing lifecycle or stale-slot reuse proof.

`pilot-receipt-audit.json` and `validation-summary.json` retain the exact ISO,
serial and source bindings and each guest's full child identity. Each runner
also saves the complete source hash map; final compiled inputs match all five
accepted pilot records. This does not establish hardware/topology certification.

## Host, compatibility and production checks

Admission RR/BORE, allocator/unmap failure properties, 171 kernel library tests,
scheduler host tests, descriptor tests, Linux host tests, focused native thread
and mapping APIs, and all 11 T1/T2/T2R parser tests pass. Changed Rust spans and
the complete new fixture pass rustfmt. Full-workspace formatting still reports
baseline diagnostics. Strict Cargo Clippy stops at the unchanged IPC baseline;
direct final-fixture kernel Clippy reports 155 existing diagnostics and zero
new/changed-line diagnostics. These Clippy runs are recorded FAIL with baseline
classification; they are not labelled PASS.

The 2-CPU native std/Hayro quota/join/detach/TLS and Linux clone/futex/FD/TLS gates
pass. A 2-CPU T1 hostile-stack/lifecycle gate passed before the user narrowed the
scope; no further hostile-stack run was made after that request. Later changes
are debug-fixture coordination/receipt changes, a fixture-gated placement hook,
and formatting; T1 assembly, VM retirement and production handoff logic are
unchanged. Every final pilot and the ordinary release image pass the emitted
trusted-stack ABI checker. This is not a new full final-source T1 SMP matrix.

Ordinary production release build and 2-CPU boot pass on the final implementation;
ordinary debug build is restored and passes. The release gate restores the
ordinary supervisor before building. Production fixture-exclusion results are
retained separately. No FastLane benchmark or old exhaustion matrix was rerun.
All command paths, stdout, return codes and timeouts are indexed in the ledger.
The descriptor host test automatically updated its package-version lockfile;
that unrelated generated change was restored to its exact initial bytes.

| New focused race matrix | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- | --- |
| Two quota controls | Not run | Both winners pass | Not run | Not run |
| Immediate exit | Not run | One order passes | Not run | Not run |
| Five lifecycle controls | Not run | Missing | Not run | Not run |

T2 remains **PARTIAL for the original defined security scope**. The smallest
next executable proof is a bounded two-CPU creator-versus-parent-exit fixture
with exact owner/creator/child generations and each legal serialization order.
It was deferred under this run's safer-test scope. Four other lifecycle rows,
the reverse immediate-exit order and the final SMP expansion also remain.

'''
if '## Recorded native results and reconciliation' not in doc:
 doc=doc.replace('## Scope adjustment and limitations',section+'## Scope adjustment and limitations')
p.write_text(doc)
current=identity();summary['canonical_source_sha256']=current['canonical_sha256'];summary['source_snapshot']='final/source.json'
(base/'validation-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
final=base/'final';final.mkdir(exist_ok=True)
(final/'source.json').write_text(json.dumps(current,indent=2)+'\n')
initial=json.loads((base/'initial/source.json').read_text())
changed=[f for f,h in current['files'].items() if initial['files'].get(f)!=h]
assert set(f for f in changed if f in initial['files'])=={'kernel/Cargo.toml','kernel/build.rs','kernel/src/arch/x86_64/syscall.rs','kernel/src/main.rs','kernel/src/process/fd_table.rs','kernel/src/sched/mod.rs'}
patch=[]
for f in changed:
 target=final/'source'/f;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(Path(f).read_bytes())
 old=(base/'initial/source'/f).read_text().splitlines(True) if f in initial['files'] else []
 patch.extend(difflib.unified_diff(old,Path(f).read_text().splitlines(True),fromfile='a/'+f if old else '/dev/null',tofile='b/'+f))
(final/'t2r.patch').write_text(''.join(patch))
(final/'manifest.json').write_text(json.dumps({'canonical_sha256':current['canonical_sha256'],'changed_files':changed,'sha256':{f:current['files'][f] for f in changed}},indent=2)+'\n')
print('PARTIAL; final eligible pilots:',len(proofs),'source:',current['canonical_sha256'])
