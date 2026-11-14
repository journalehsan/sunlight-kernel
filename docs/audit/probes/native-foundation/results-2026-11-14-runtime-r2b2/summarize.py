from pathlib import Path
import json,hashlib,subprocess,re,csv
E=Path(__file__).resolve().parent; R=Path.cwd()
def write(name,value): (E/name).write_text(json.dumps(value,indent=2)+'\n')
def read(name): return json.loads((E/name).read_text())
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def git(*args):return subprocess.check_output(['git',*args]).decode()
baseline=read('source-baseline.json')
original=baseline['sha256']
changes=[p for p,h in original.items() if not Path(p).is_file() or sha(Path(p))!=h]
allfiles=git('ls-files','--cached','--others','--exclude-standard').splitlines()
tracked=set(git('ls-files','--cached').splitlines())
symlink_rows=[]
for line in git('ls-files','--stage').splitlines():
 meta,path=line.split('\t',1);mode,oid,stage=meta.split()
 if mode=='120000':
  import os
  target=os.readlink(path).encode() if Path(path).is_symlink() else b''
  actual=hashlib.sha1(b'blob '+str(len(target)).encode()+b'\0'+target).hexdigest()
  symlink_rows.append(dict(path=path,index_blob=oid,actual_target_blob=actual,matches=actual==oid))
new=[p for p in allfiles if p not in original and p not in tracked and not p.startswith(str(E.relative_to(R))+'/')]
if any(not row['matches'] for row in symlink_rows): changes += [row['path'] for row in symlink_rows if not row['matches']]
allowed_report='docs/runtime/native-runtime-r2b2-lifecycle-race-closure.md'
unexpected_new=[p for p in new if p!=allowed_report]
fingerprint=hashlib.sha256(json.dumps({p:h for p,h in original.items() if not p.startswith('docs/')},sort_keys=True).encode()).hexdigest()
runs=read('run-results.json')+read('supplemental-results.json')
byname={r['name']:r for r in runs}
def status(name):return 'PASS' if name in byname and byname[name]['exit']==0 else 'FAIL' if name in byname else 'NOT_RUN'
def guest(stem):
 folder=E/stem; result=json.loads((folder/'results.json').read_text()) if (folder/'results.json').exists() else {}
 serial=(folder/'serial.log').read_text() if (folder/'serial.log').exists() else ''
 return {'run':stem,'runner_status':status(stem),'source_revision':baseline['revision'],'runtime_input_fingerprint_sha256':fingerprint,'kernel_sha256':sha(folder/'kernel.elf') if (folder/'kernel.elf').exists() else None,'serial_log':stem+'/serial.log','serial_sha256':sha(folder/'serial.log') if (folder/'serial.log').exists() else None,'guest_result':result},serial
initial=[]
for cpu in [1,2,4]:
 d,s=guest(f'initial-exit-cpu-{cpu}'); output=d['guest_result'].get('elf',{}).get('stdout','')
 killed=re.findall(r'borrower pid=(\d+) terminated reason=address-space-owner-exit',s)
 d.update(cpus=cpu,witness_id=f'IEXIT-{cpu}',worker_alive='I1 worker independently alive' in output,initial_pthread_exit_entered='I2 initial TLS cleanup entered' in output,worker_survived='I4 worker survival observed' in output,worker_cleanup='I5 worker TLS cleanup completed' in output,killed_borrower_tids=killed,contract_status='FAIL' if killed else 'UNQUALIFIED',exit_zero_is_only_observation_success=True)
 d['kernel_spawn_identities']=re.findall(r'\[SYSCALL\] thread_spawn: parent=(\d+) tid=(\d+)',s)
 initial.append(d)
write('initial-thread-exit-trace.json',{'status':'BLOCKED','root_cause':'native owner syscall 20 terminates address-space borrowers','kernel_patch_applied':False,'witnesses':initial,'coordination':'worker prints alive then releases initial-thread gate; initial TLS destructor proves pthread_exit was entered; kernel termination trace establishes borrower death. Worker observation delay is not claimed as post-exit overlap barrier.','proposed_patch_plan':'lifecycle-ownership.md','_Exit_api':read('exit-api-probe.json'),'_Exit_link':read('exit-link-probe.json')})
accounting=[]; failures=[]
for cpu in [1,2,4]:
 d,s=guest(f'detached-accounting-cpu-{cpu}');output=d['guest_result'].get('elf',{}).get('stdout','')
 d.update(cpus=cpu,witness_id=f'DETACH-{cpu}',snapshots=[dict(wave=int(w),phase=p,queued=int(q),control_bytes=int(c),stack_bytes=int(st),tls_bytes=int(t)) for w,p,q,c,st,t in re.findall(r'A wave=(\d+) phase=(\S+) queued=(\d+) control_bytes=(\d+) stack_bytes=(\d+) tls_bytes=(\d+)',output)],wave_counts=[dict(wave=int(w),created=int(c),callbacks=int(cb),kernel_reaped=int(k)) for w,c,cb,k in re.findall(r'A wave=(\d+) created=(\d+) callbacks=(\d+) kernel_reaped=(\d+)',output)])
 d['kernel_spawn_identities']=re.findall(r'\[SYSCALL\] thread_spawn: parent=(\d+) tid=(\d+)',s)
 accounting.append(d)
 d,s=guest(f'detached-churn-cpu-{cpu}');d.update(cpus=cpu,attempted_workers=72,observed_spawn_count=len(re.findall(r'\[SYSCALL\] thread_spawn:',s)),actual_spawn_tids=re.findall(r'\[SYSCALL\] thread_spawn: parent=\d+ tid=(\d+)',s),intended_overflow_overlap='NOT_RUN',reason='creation returned failure before release gate; exact admission cause not qualified; source and limits unchanged')
 failures.append(d)
write('detached-reclamation.json',{'status':'PARTIAL','method':'read-only white-box queue snapshots linked to unchanged runtime private symbol; bind address verified against final ELF; repr(C) Control size 72; no concurrent sweeper; query 156 confirms all workers Reaped before snapshot','build_binding':read('accounting-build.json'),'observations':accounting,'negative_attempts':failures,'proven_bound':'12 retained entries over four gated waves in each observed CPU configuration, then 0 after explicit create sweep; not an architecture-wide bound','concurrent_detach_sweep':{'status':'UNQUALIFIED','source_risk':'mark_detached CAS 0->1 and later state read lack queue lock/reference; worker can enqueue and exit while detacher is paused, third creator can free before detacher resumes','native_UAF_observed':False},'autonomous_reclamation':'FAIL: quiescence alone retains entries','global_steady_state_bound':'UNQUALIFIED','overflow':'source-confirmed silent reference loss at 64 queued controls; native overflow execution NOT_RUN due admission failure','TLS_values':'application-owned pointees; runtime cells discarded; callbacks observed before Reaped','futex_wait_state':'kernel clears on exit; registered-state snapshots NOT_RUN','stack_mapping_status':'stack/TLS allocation extents read from retained Control; no page-table enumeration; explicit sweep munmap source path plus zero queue observed, not independent per-map unmap acknowledgment'})
tls=[]
for cpu in [1,2,4]:
 d,s=guest(f'tls-controlled-cpu-{cpu}');output=d['guest_result'].get('elf',{}).get('stdout','');d.update(cpus=cpu,witness_id=f'TLS-CONTROLLED-{cpu}',coordination_states=output.splitlines(),overlap='callback held between T4 and T6 while initial thread deletes/reuses at T5; stale worker cell held between T1 and T3 across T2 deletion/reuse',terminal_outcome='two joins and one callback' if 'T7 joins=2 callbacks=1' in output else 'UNQUALIFIED',resource_counts={'joined_threads':2,'remaining_live_keys':0} if 'T7 joins=2 callbacks=1' in output else None)
 d['kernel_spawn_identities']=re.findall(r'\[SYSCALL\] thread_spawn: parent=(\d+) tid=(\d+)',s)
 handles=re.findall(r'(?:holder|callback)=(0x[0-9a-f]+)',output)
 d['public_pthread_handle_reuse_observed']=len(handles)==2 and handles[0]==handles[1]
 tls.append(d)
write('tls-race-witnesses.json',{'status':'PARTIAL','controlled_native_witnesses':tls,'additional_inflight_destructor_runs':[guest(f'tls-exit-race-cpu-{cpu}')[0] for cpu in [1,2,4]],'qualified_schedules':['key reuse while stale TCB value retained','destructor callback versus delete/reuse','destructor reentrancy through create/set/get/delete','explicit worker pthread_exit runs callback before Reaped/join'],'unqualified_schedules':[{'schedule':s,'status':'UNQUALIFIED','reason':'no controlled API-internal registry-lock overlap witness'} for s in ['key create versus delete','key delete versus getspecific','key delete versus setspecific']]+[{'schedule':'detached exit versus registry cleanup','status':'UNQUALIFIED','reason':'detached accounting proves callback and reap order, not concurrent registry-lock overlap'},{'schedule':'initial-thread exit versus live worker','status':'BLOCKED','reason':'native owner teardown kills borrower'},{'schedule':'final-thread exit versus process termination','status':'BLOCKED','reason':'initial thread cannot exit independently, so surviving final worker scenario is unavailable'}],'opaque_pthread_identity':'Control pointers are untagged and allocation reuse can alias stale handles; valid-handle tests do not qualify a stronger stale pthread_t guarantee','generation_design':'128 static slots, index low 7 bits, nonwrapping generation high bits; reentrant callbacks run after unlock; four destructor passes','functional_fixture_runs':[guest(f'focused-cpu-{cpu}')[0] for cpu in [1,2,4]]})
write('once-futex-race-witnesses.json',{'status':'UNQUALIFIED','source_audit':{'states':{'0':'uninitialized','1':'initializing','2':'complete'},'election':'Acquire CAS 0 to 1','publication':'Release store 2, wake-all; Acquire load/CAS on readers','wait':'existing native_futex WAIT(expected=1) then retry; compare/register and wake serialized by scheduler lock','abandonment':'TCB stack-linked frames reset to 0 Release and wake-all on supported pthread_exit before TLS callbacks','callback_lock':'no internal lock across initializer','recursion':'same control cannot complete; finite bounded observation only','unsupported':['cancellation','longjmp/exception escape','raw thread syscall bypassing cleanup']},'native_functional_runs':[guest(f'focused-cpu-{cpu}')[0] for cpu in [1,2,4]],'schedules':[{'id':f'ONCE-{n+1}','schedule':s,'status':'UNQUALIFIED' if n!=6 else 'NOT_RUN','functional_fixture':'focused' if n in [2,3,4,5] else None,'reason':'caller-start flags do not prove entry into futex registration; no once-integrated hook in this guest run' if n!=6 else 'no controlled concurrent termination witness'} for n,s in enumerate(['completion immediately before waiter registration','completion during waiter registration','multiple registered waiters on one initializer','initializer exit with registered waiting contenders','retry after abandonment with registered waiters','rapid repeated calls across completion transition','concurrent termination during contention'])],'completed_fast_path':'functional repeated 10000 calls per worker observed by focused test; not a race witness','existing_kernel_hooks':'atomic_futex_test t4a_fixture has controlled raw futex schedules for its own registered actor fixture; not used by native C runner and not substituted for pthread_once evidence','new_kernel_behavior':False})
legacy=list(csv.DictReader((E/'regression-suite/results.csv').open()))
old=json.loads((E.parent/'results-2026-11-14-runtime-r2b1/validation-summary.json').read_text())
languages={r['probe']:r['language'] for r in old['legacy_cases']}
for row in legacy:row.update(language=languages.get(row['probe'],'UNKNOWN'),cpus=2,source_revision=baseline['revision'],runtime_input_fingerprint_sha256=fingerprint)
counts={lang:dict(total=sum(r['language']==lang for r in legacy),passed=sum(r['language']==lang and r['acceptance']=='PASS' for r in legacy)) for lang in ['C','C++']}
write('regression-results.json',{'status':'PASS' if all(r['acceptance']=='PASS' for r in legacy) and all(status(n)=='PASS' for n in ['regression-suite','heap-abi','tcp-loopback','pthread-abi','cxx-abi','locked-final']) else 'FAIL','fresh_execution':True,'counts':counts,'individual_cases':legacy,'supplemental':{n:status(n) for n in ['heap-abi','tcp-loopback','pthread-abi','cxx-abi','locked-final']},'TLS_once_cpu_runs':{str(cpu):status(f'focused-cpu-{cpu}') for cpu in [1,2,4]},'static_initialization_exceptions':'individual cxx-local-static/cxx runtime and exception cells in suite; cxx ABI/vendor checker is distinct from guest runtime','libm_vendor_check':read('libm-vendor-check.json'),'historical_results_reused':False,'language_classification':'fixture language labels reused from R2B.1 inventory only; execution outcomes from fresh CSV'})
write('cpu-matrix.json',{'configurations':[{'cpus':cpu,'status':'PARTIAL','build_identity':fingerprint,'initial_exit':'BLOCKED','initial_exit_observation':status(f'initial-exit-cpu-{cpu}'),'detached_accounting':status(f'detached-accounting-cpu-{cpu}'),'controlled_TLS':status(f'tls-controlled-cpu-{cpu}'),'focused_TLS_once':status(f'focused-cpu-{cpu}'),'precise_once_futex':'UNQUALIFIED','regressions':'PASS' if cpu==2 and all(r['acceptance']=='PASS' for r in legacy) else 'NOT_RUN'} for cpu in [1,2,4]]+[{'cpus':12,'status':'NOT_RUN','reason':'unchanged tools/native-c/guest.py argparse choices=(1,2,4); no unsupported launch or runner modification'}]})
write('cpython-compatibility.json',{'version':'3.13.0','fresh_CPython_configure':'NOT_RUN','fresh_CPython_compile':'NOT_RUN','prior_observations':{'source':'results-2026-11-14-runtime-r2b1/validation-summary.json','configure':'PASS','compile':'FAIL: unknown pthread_condattr_t, pthread_cond_t and undeclared strncpy','interpreter_link':'NOT_RUN','execution':'NOT_RUN'},'production_source_delta':[],'current_source_assessment':{'initial_thread_pthread_exit':'BLOCKED: owner kills live workers','detached_reclamation':'PARTIAL: retained until next create; overflow remains source-confirmed','TLS':'generation-tagged isolation and controlled callback/delete/reuse supported; exact API-internal races UNQUALIFIED','once':'functional publication/abandonment and fast path pass; precise registration races UNQUALIFIED','condition_variables':'absent; no implementation or stubs added','libc_gaps':['strncpy','_Exit declaration/export','time/header integration from historical CPython diagnostics']},'impact':'no CPython capability advanced; lifecycle closure must precede condvars','next_milestone':'approved native group lifetime/thread-only exit work, deterministic reaper admission and once-integrated test hooks; then condvar design milestone'})
write('source-integrity.json',{'status':'PASS' if not changes and not unexpected_new else 'FAIL','initial_revision':baseline['revision'],'final_revision':git('rev-parse','HEAD').strip(),'final_branch':git('branch','--show-current').strip(),'final_git_status':git('status','--porcelain=v1'),'tracked_symlink_verification':{'method':'current target blob matches tracked index; initial Git status contains no symlink changes','count':len(symlink_rows),'mismatches':[r for r in symlink_rows if not r['matches']]},'initial_manifest_limit':'SHA-256 map includes regular files and symlinks resolving to files; directory/dangling symlinks checked separately against tracked index, not misclassified as new','changed_baseline_files':changes,'unexpected_new_files':unexpected_new,'production_files_modified':[],'runtime_input_fingerprint_sha256':fingerprint,'previous_evidence_preserved':not any(p.startswith(('docs/runtime/','docs/audit/')) for p in changes),'kernel_files_modified':[],'new_files':[p for p in allfiles if p not in original],'no_commit':git('rev-parse','HEAD').strip()==baseline['revision'],'no_push':True})
gates={'G1':'PASS' if not changes else 'FAIL','G2':'BLOCKED','G3':'PARTIAL','G4':'PARTIAL','G5':'UNQUALIFIED','G6':read('regression-results.json')['status'],'G7':'PASS' if not changes and not unexpected_new else 'FAIL'}
write('validation-summary.json',{'overall':'PARTIAL','complete':False,'gates':gates,'source_revision':baseline['revision'],'baseline_locked_build':'PASS','final_locked_build':status('locked-final'),'source_integrity':read('source-integrity.json')['status'],'production_files_modified':[],'kernel_changes':False,'condition_variables_implemented':False,'commit':False,'push':False,'report':allowed_report,'evidence_directory':str(E.relative_to(R)),'negative_findings':['initial-thread exit kills worker on 1/2/4 CPUs','72-thread detached test admission failure at 32 successful spawns; overflow schedule not reached','detached entries remain after bounded quiescence','queue overflow silently drops cleanup reference in source','_Exit public API compile probe FAIL','precise TLS registry and once/futex registration overlaps remain unqualified','public pthread_t Control pointer reuse aliases stale handles; no generation tag'],'runs':runs})
print(json.dumps({'gates':gates,'counts':counts,'baseline_changed':changes,'new_unexpected':unexpected_new},indent=2))
account_rows=[]
for d in accounting:
 for snap in d['snapshots']:
  if snap['wave'] in [0,1,4,5]:account_rows.append(f"| {d['cpus']} | {snap['wave']} | {snap['phase']} | {snap['queued']} | {snap['control_bytes']} | {snap['stack_bytes']} | {snap['tls_bytes']} |")
report=f'''# Native runtime R2B.2: lifecycle and race closure

Date: 2026-11-14. Revision `{baseline['revision']}`, branch `{baseline['branch']}`.

**Overall: PARTIAL; R2B.2 is not COMPLETE.** Initial-thread exit is BLOCKED by
native kernel owner/group semantics. No production repair was applied. Independent
userspace probes add actual detached resource accounting and controlled TLS
callback/delete/reuse witnesses. Missing precise registry and futex registration
schedules remain unqualified. No kernel modification, condvar implementation,
commit or push. CPython is not operational.

Evidence root (**E**): `docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b2/`.
Every result below is fresh unless explicitly described as historical.

| Gate | Status | Evidence and limit |
|---|---|---|
| G1 reproducible baseline / ownership | {gates['G1']} | `source-baseline.json`, `baseline-locked-build.log`, `lifecycle-ownership.md`; retained installed-toolchain locked build, not clean-room bit reproducibility |
| G2 initial pthread_exit preserves workers | BLOCKED | `initial-thread-exit-trace.json`; source-backed failure reproduced on 1/2/4 CPUs; kernel changes prohibited |
| G3 detached bounded steady state | PARTIAL | `detached-reclamation.json`; measured 12-entry cache over four waves and explicit sweep; no autonomous completion, global overflow bound unqualified |
| G4 TLS lifecycle race closure | PARTIAL | `tls-race-witnesses.json`; callback/delete/reuse and stale-cell witnesses qualified; exact create/delete/get/set internal overlap unqualified |
| G5 once/futex race closure | UNQUALIFIED | `once-futex-race-witnesses.json`; functional native pass does not establish registration-point overlaps |
| G6 native regressions | {gates['G6']} | `regression-results.json`, `regression-suite/results.csv`; individual outcomes preserved |
| G7 integrity / compatibility | {gates['G7']} | `source-integrity.json`, `cpython-compatibility.json`; every baseline file preserved |

## Baseline and source boundaries

Read R1, R2A and R2B.1 reports, R2B.1 validation summary and its validation commands.
The checkout already contained lockfile, libc header/module, vendor, fixture and
historical evidence changes. `source-baseline.json` records initial revision,
branch, status and hashes of regular tracked/nonignored untracked files.
Directory/dangling symlinks are verified separately against tracked Git blobs;
`integrity-classification-correction.md` preserves the initial classifier error
and its correction. Those changes
were preserved. The exact original `bash tools/build_native_c.sh` passed before
new probes and again after the regression runs. No build dependency or runner
change. Build/kernel/ISO/guest logs and kernel SHA-256 remain in each run folder.
The source fingerprint in JSON identifies the unchanged runtime input snapshot;
it is not a clean-room artifact reproducibility claim.

The source-backed ownership graph and detailed classification were written in
`lifecycle-ownership.md` before any proposed repair. No production edit followed.
The native C driver, static native archive, existing native kernel runner, QEMU
TCG `-cpu max`, 2048 MiB, and disposable guest images were used throughout.

## Thread and process termination

`sunlight-libc/src/std_runtime.rs:35` calls `main`, then `termination::exit`.
`termination.rs:81` runs atexit callbacks and `sl_std_exit` invokes syscall 157.
`kernel/src/arch/x86_64/syscall.rs:3152` implements native exit-group by matching
VM identity, terminating sibling tasks, then exiting the caller.

`sunlight-libc/src/pthread.rs:305` cleans thread-local once/TLS state and invokes
syscall 20. Worker return runs `finish`, then `thread_start` invokes the same
syscall (`thread.rs:65`). This exits a native borrower alone. The initial thread
is a static opaque sentinel without a heap Control and owns the address space.
`kernel/src/sched/mod.rs:4579` explicitly terminates borrowers for native owner
exit. It also closes owner IPC and publishes process completion; removing the
borrower-kill branch alone would leave other ownership obligations unresolved.

The initial-exit fixture proves worker creation, an independently emitted alive
marker, and entry into initial pthread_exit via that thread's TLS destructor.
All three CPU runs show borrower termination with
`reason=address-space-owner-exit`, no worker survival marker, and no worker TLS
cleanup marker. The observation runner exits successfully because exit zero was
expected; **the pthread lifecycle contract FAILS**. The post-cleanup observation
window is not asserted as a deterministic post-syscall barrier. The kernel trace,
not missing output alone, establishes the defect. Identities and traces are in
`initial-thread-exit-trace.json`.

This is a native ownership/process-group contract defect exposed by libc, not a
runner limitation. A userspace parked-owner shim would retain kernel task/session,
IPC and descriptor ownership and would not cover arbitrary Rust native borrowers.
There is no reviewed native thread-only owner-exit interface establishing the
required contract. The affected repair is therefore BLOCKED under the no-kernel-
changes rule. A final surviving-worker-only process cannot currently be qualified.

`_Exit` is actually absent from stdlib.h and native exports, an additional libc
gap rather than a demonstrated working API. `exit-api-probe.log` preserves the
fresh undeclared-function compile failure; `exit-link-probe.log` separately proves the undefined export after an explicit declaration. The callback-free `sl_std_exit` is
process-wide, but it is not an exported `_Exit`. No new API was silently substituted.
Returning from main and explicit exit retain group termination; worker pthread
exit/return retain per-borrower termination and supported cleanup.

## Blocked kernel patch proposal

A separately authorized milestone should introduce native thread-group lifetime
independent of the initial schedulable task. Preserve explicit syscall 157 group
termination. Thread-only exit must retain group-owned VM/resources until every
borrower is Reaped and outgoing execution pins are released; publish process
completion only after final-thread retirement. Reuse identity/generation and
existing deferred-retirement rules. Audit parent waiting/reaping, session/PTY and
foreground group ownership, IPC endpoint close, quota membership and capability
revocation. Native survivor progress, final-thread teardown and session/PTY/IPC
regressions must qualify that change. No such patch was applied here.

## Detached accounting and negative attempts

C Control owns native TID, return/state metadata, the 2 MiB stack extent and
static TLS/TCB extent. TLS values remain application-owned; callbacks run before
EXITED. The kernel owns task/futex state and outgoing-stack pins; query 156 is
the userspace exit acknowledgment. Joining waits for Reaped before release.
Detaching queues the Control after cleanup. Only subsequent pthread_create
sweeps the C queue; pthread_join does not, contrary to the old module comment.
The separate Rust std 256-slot deferred queue cannot reclaim C Control objects
without a resource-specific adapter and also needs later API activity.

The first attempt launched 72 gated detached workers to avoid intervening sweeps.
All CPU configurations admitted 32 before creation failed and main returned 2.
The gate was never released and no intended overflow schedule occurred. Exact
admission cause was not established. These failed runs and original source are
preserved as `detached-churn-cpu-*`; no quota, capacity or timeout was increased.
They do not validate overflow behavior or TLS completion.

The accounting fixture uses 12 detached workers in each of four waves. It binds
a test linker alias to the unchanged runtime's private DETACHED symbol; the final
ELF symbol address is verified by `build-accounting.py` and `accounting-build.json`.
The C mirror of repr(C) Control is 72 bytes. An initial incorrect 80-byte assertion
failed compilation; `accounting-layout-failed-attempt.log` preserves that negative
attempt. No library binary or source was patched. The ordinary static native
archive is linked. This is a deliberately white-box test, not a public libc API.

No queue writer runs during snapshots: workers are either held at the entry gate
or every TID has returned Reaped from syscall 156; only the initial thread can
create/sweep. Snapshots print actual queue cardinality and allocation extents.
Each wave records callbacks=12 and kernel_reaped=12, then waits another bounded
100 one-millisecond sleeps. Clock granularity means this is a requested quiescence
window, not a precisely measured 100 ms wall duration.

| CPUs | Wave | Snapshot | Queued controls | Queued control bytes | Queued stack bytes | Queued TLS bytes |
|---|---|---|---|---|---|---|
{chr(10).join(account_rows)}

During each gated snapshot, 12 successful creates and ready==12 establish 12 live
Control/stack/TCB allocations. Queue zero at that point describes only the deferred
queue, not all live resources. Full wave-by-wave counts and native logs are in
`detached-reclamation.json`.
The same 12-entry retention repeats; a final explicit create/sweep and joined
sentinel reaches queue zero. This proves bounded retention for the tested waves
and reclaimability when another create occurs. It does not prove autonomous
reclamation or a global architectural cache bound. Stack/TLS bytes are retained
allocation extents read from Control, not independently enumerated PTEs. The
sweep calls munmap, but no per-mapping unmap acknowledgment was instrumented.
Final process teardown is separately checked by the runner's owned-frame/FD/stack
resource checks.

`pthread.rs:92` silently drops the cleanup reference when 64 queue entries are
occupied; those resources survive until process exit. The 64 slots therefore do
not establish a bound on retained allocations. Runtime overflow remains source-
confirmed but its native schedule was not reached. A follow-up needs detach
admission before publication and an explicit cleanup owner with exit acknowledgment.
There is also a source-only observer risk: mark_detached changes 0->1 and then
reads Control without holding the queue lock. A worker could enqueue/exit while
that caller is descheduled, allowing a third creator to sweep before the caller
resumes. No native witness forced that interval; it remains UNQUALIFIED, not an
observed use-after-free. It requires a controlled test and protected observer
ownership before concurrent detach/reclamation can be qualified.
Do not enlarge the queue, free a running stack, or add unbounded polling.

## TLS witnesses

`tls-controlled.c` runs on 1/2/4 CPUs with bounded handshakes. T1 prints holder
pthread identity, key index/generation and populated stale cell, then blocks.
T2 deletes/reuses the same slot with a new generation. T3 proves old get is NULL,
old set returns EINVAL, and the reused key does not expose the retained cell.
This is a qualified stale-cell/reuse ordering witness; it is not an API-internal
get/delete overlap witness.

A second worker explicitly exits with a non-NULL TLS value. Its callback proves
the cell was cleared and reenters create/set/get/delete before printing T4 and
blocking. The initial thread deletes/reuses its key at T5 while the callback is
held; release permits T6 completion. T7 records two joins, one callback and zero
live test keys. This qualifies callback/delete/reuse overlap, reentrancy without
registry lock retention, and callback completion before worker Reaped/join.
The original in-flight destructor fixture is also rerun on every CPU count.
The two worker identities reuse the same public Control pointer in these logs
after the first join, while their kernel TIDs are distinct. Public pthread_t is
untagged; stale-handle alias rejection is not supplied. No stale-handle operation
was attempted. TLS key generations and kernel TID monotonicity must not be
misrepresented as generation protection for public pthread handles.

The functional TLS fixture revalidates 128 generation-tagged keys, exhaustion,
isolation, four destructor passes, stale-generation rejection, nested callback
operations, explicit exit, detach and churn. Static registry slots are never freed;
TCB values belong to their own thread. Generations never wrap. Exact concurrent
create/delete, delete/get, delete/set and detached-registry internal schedules
remain UNQUALIFIED. Initial/final-thread schedules are BLOCKED by G2. G4 remains
PARTIAL; starting competitors is not accepted as proof of overlap.

## Once and futex qualification

`pthread.rs:550` uses states 0/1/2, Acquire election CAS, Release completed store
and wake-all, and Acquire reads on return. It holds no lock during user init.
Stack-linked TCB frames reset abandoned nested initializers to 0 with Release
ordering and wake contenders during supported pthread_exit. Kernel futex compare,
registration and wake are serialized (`syscall.rs:3040-3135`); userspace consumes
WAIT results and retries. Recursion on the same control cannot complete itself;
this source conclusion is distinct from a precise kernel wait-state witness.

Fresh focused runs on 1/2/4 CPUs pass concurrent caller publication, initialization
count checks, nested controls, initializer pthread_exit/retry and repeated completed
fast paths. Their start flags and test sleeps do not show where each contender
was parked. Completion immediately before/during registration, registered multiple
waiters, abandonment with registered waiters, retry across registration and rapid
transition overlap remain UNQUALIFIED. Concurrent thread termination under once
contention is NOT_RUN. No exhaustive lost-wakeup or memory-safety claim follows.
Existing debug-only T4A kernel hooks control a separate registered raw-futex actor
fixture; they were inspected but not substituted for once-integrated guest evidence.
No new kernel futex behavior or production sleeps were introduced. G5 is UNQUALIFIED.

## CPU and regression results

1/2/4 CPUs: initial-exit observation reproduced the failure; focused TLS/once,
original callback/delete race, controlled TLS and detached accounting results are
recorded individually in `cpu-matrix.json`. Each guest folder preserves ELF/kernel
identity, serial log, exit/output checks and physical process-reaping observations.
12 CPUs: NOT_RUN because the unchanged native C runner accepts only 1/2/4.

Fresh legacy suite: C {counts['C']['passed']}/{counts['C']['total']},
C++ {counts['C++']['passed']}/{counts['C++']['total']}. `regression-results.json`
retains every case, including expected nonzero exits. Static initialization and
exception guest tests remain individual suite cells. Supplemental heap/ABI,
TCP loopback, pthread ABI and target C++ ABI/vendor checks: respectively
{status('heap-abi')}, {status('tcp-loopback')}, {status('pthread-abi')}, {status('cxx-abi')}.
Libm files were freshly compared with the immutable R2A manifest. Historical
execution results were not reused; only existing fixture language labels were
used to classify the fresh 94 CSV rows. No failure is hidden behind aggregate
counts. New probes are outside the 73 C/21 C++ legacy counts.

## Compatibility, integrity and next milestone

CPython 3.13.0 configure/compile was not rerun: no production input changed and
this milestone does not advance the port. R2B.1 configure PASS and core compile
FAIL are historical evidence explicitly labeled in `cpython-compatibility.json`.
Remaining condition-variable types/functions, strncpy and time/header integration
remain blockers; `_Exit` absence is freshly demonstrated here. Generation-tagged
TLS and supported once behavior remain available with the lifecycle/race limits
above. No fake symbols, artificial stubs or host substitution.

`source-integrity.json` compares every hashed baseline file, verifies tracked
symlink targets against Git blobs, and records final Git state.
Production files modified: **none**. All previous report/evidence and source files
remain unchanged. Added files are this report and E's probes, runners, logs and
JSON evidence. No commit or push occurred.

Recommended next step: separately authorize a native group-lifetime/thread-only
exit milestone, detached admission/reclamation ownership and deterministic
once-integrated registration witnesses. Only after G2-G5 close should the Condvar
milestone design wait-registration/unlock atomicity, mutex reacquisition,
signal/broadcast selection, deadline/clock semantics, destruction with waiters,
and cancellation boundaries. Reuse existing futex semantics and require controlled
native races on 1/2/4 CPUs before any CPython integration.
'''
(R/allowed_report).write_text(report)
