#!/usr/bin/env python3
"""Assess actual retained native logs. Missing observations never become PASS."""
import pathlib,json,csv,re,hashlib,subprocess
R=pathlib.Path(__file__).resolve().parents[5];E=pathlib.Path(__file__).resolve().parent;F=E/'validated'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(name,v):(E/name).write_text(json.dumps(v,indent=2)+'\n')
def load(p):return json.loads(p.read_text())
def kv(line):return dict(re.findall(r'(\w+)=([^\s]+)',line))
baseline=load(E/'source-baseline.json')
source=load(F/'validated-source.json');assert all(sha(R/p)==h for p,h in source.items())
executions=load(F/'executions.json');assert executions[-1]['name']=='production-check','validation is unfinished'
more=load(F/'supplemental-executions.json')+load(F/'tls-executions.json')
assert more[-1]['name']=='tls-cpu-4'
cells=[];witnesses=[];retirement=[]
for cpu in [1,2,4]:
 folder=F/('lifecycle-cpu-'+str(cpu));data=load(folder/'results.json');serial=(folder/'serial.log').read_text(errors='replace');csvrows=list(csv.DictReader((folder/'results.csv').open()))
 resultlines=[kv(l) for l in serial.splitlines() if l.startswith('[NATIVE-C-RESULT]')]
 # Each launch is sequential. Correlate evidence with its actual owner PID,
 # not the numerically overlapping borrower case labels in the legacy runner.
 for index,(name,result) in enumerate(data.items()):
  pid=100+index;start=0 if index==0 else serial.index('[NATIVE-C-RESULT] case='+str(index-1)+' ')
  end=serial.index('[NATIVE-C-RESULT] case='+str(index)+' ')
  block=serial[start:end];handoffs=[kv(l) for l in block.splitlines() if l.startswith('[NATIVE-GROUP-WITNESS]') and kv(l).get('owner')==str(pid)]
  terminals=[kv(l) for l in block.splitlines() if l.startswith('[NATIVE-GROUP]') and kv(l).get('pid')==str(pid)]
  final=[kv(l) for l in block.splitlines() if l.startswith('[NATIVE-C-REAP-END]') and kv(l).get('case')==str(index)]
  output=result['stdout'];required=2 if name=='multiple' or name.startswith('churn-') else 1
  owner_exit=name in ['initial','multiple','forced-after-owner'] or name.startswith('churn-')
  alive=re.findall(r'W alive actor=(\d+) tid=(\d+) owner=(\d+)',output)
  progress=re.findall(r'W progress actor=(\d+) tid=(\d+) after-owner-query',output)
  checks={'runner_pass':result['pass_'],'expected_exit':str(result['exit'])==csvrows[index]['expected_exit'],
    'one_group_terminal':len(terminals)==1,'one_owner_reap':len(final)==1,
    'owner_resources_zero':len(final)==1 and all(final[0].get(k)=='0' for k in ['owned_frames','fds','stack']),
    'balanced_frames_fds_names':resultlines[index].get('clean')=='1'}
  if owner_exit:
   checks.update({'live_borrowers_before_exit':len(alive)==(1 if name=='initial' else 2),
      'initial_tls_cleanup':'W tls actor=3' in output,
      'quiescent_owner_observed':len({h['worker'] for h in handoffs if h.get('quiescent')=='1'})>=required,
      'independent_post_query_progress':len({t for a,t in progress})>=required,
      'observed_progress_workers_bound':all(any(h.get('worker')==t and h.get('quiescent')=='1' for h in handoffs) for a,t in progress)})
   if name!='forced-after-owner':checks['tls_cleanup']=all('W tls actor='+str(i)+' ' in output for i in range(1,required+1))
   if required==2:checks['first_reaped_before_final']='W first-borrower-reaped final-borrower-exiting' in output
  row=dict(case=name,cpus=cpu,status='PASS' if all(checks.values()) else 'FAIL',checks=checks,
      owner_pid=pid,terminal=terminals,handoff_observations=handoffs,thread_identities=alive,
      progress_identities=progress,resource_result=resultlines[index],owner_reap=final,
      kernel_sha256=(folder/'kernel-sha256.log').read_text().strip(),elf_sha256=csvrows[index]['sha256'],
      revision=baseline['revision'],source_manifest='validated/validated-source.json',
      evidence=str(folder.relative_to(E)),coordination='TLS exit announcement; actual same-VM worker syscall156 observation; snapshot must show original Finished and physical handoff complete before worker returns to emit progress; no sleep duration is accepted as proof',
      overlap='W6 overlap NOT_RUN; these are handoff/survival and sequential retirement witnesses')
  witnesses.append(row)
  retirement.append(dict(case=name,cpus=cpu,status=row['status'],resources=row['resource_result'],owner_reap=final,
    limitation='Physical frames, descriptor handles, stacks and task Reaped; no full heap/ledger-slot census or group-specific retirement fault'))
 regress=F/('regression-cpu-'+str(cpu));rows=list(csv.DictReader((regress/'results.csv').open()));assert len(rows)==94
 cxx=[r for r in rows if r['probe'].startswith(('cpp-','cxx-'))];c=[r for r in rows if r not in cxx];assert len(c)==73 and len(cxx)==21
 counts=lambda rows:dict(total=len(rows),passed=sum(x['acceptance']=='PASS' for x in rows))
 supp=load(F/('supplemental-cpu-'+str(cpu))/'results.json');tls=load(F/('tls-cpu-'+str(cpu))/'results.json')
 cells.append(dict(cpus=cpu,lifecycle=dict(total=len(data),passed=sum(x['status']=='PASS' for x in witnesses if x['cpus']==cpu)),C=counts(c),Cxx=counts(cxx),heap_abi='PASS' if supp['heap-abi']['pass_'] else 'FAIL',retained_owner_create_join='PASS' if supp['retained-admission']['pass_'] else 'FAIL',tls_once={k:'PASS' if v['pass_'] else 'FAIL' for k,v in tls.items()},regression_rows=rows,
 kernel_sha256=(regress/'kernel-sha256.log').read_text().strip()))
write('cpu-matrix.json',{'cells':cells,'cpus_12':{'status':'NOT_RUN','reason':'native runner accepts only 1/2/4; no criteria weakened'},'tcp_cpu_1':{'status':'NOT_RUN','reason':'unchanged TCP runner accepts only 2/4'},'tcp_executions':[x for x in executions if x['name'].startswith('tcp-')]})
write('lifecycle-witnesses.json',{'witnesses':witnesses,'coverage':{'W1':'PASS' if all(w['status']=='PASS' for w in witnesses if w['case']=='initial') else 'FAIL','W2':'PASS' if all(w['status']=='PASS' for w in witnesses if w['case']=='multiple') else 'FAIL','W3':'PASS' if all(w['status']=='PASS' for w in witnesses if w['case'] in ['initial','multiple']) else 'FAIL','W4':'PASS' if all(w['status']=='PASS' for w in witnesses if w['case'].startswith('exit-') and w['case']!='exit-3') else 'FAIL','W5':'PASS' if all(w['status']=='PASS' for w in witnesses if w['case'] in ['forced-after-owner','exit-3']) else 'FAIL','W6':'NOT_RUN','W7':'PARTIAL','W8':'PARTIAL','W9':'PARTIAL','W10':'PARTIAL'},'missing':{'W6':'No native external group terminator coordinated to overlap the final living thread exit before scheduler serialization; ordinary kill is not this race','W7':'Host exact-session scan only; live authenticated PTY/tab/session retained-owner witness NOT_RUN','W8':'Host broker/PID/VM identity checks only; native stale generation operations across group retirement/reuse NOT_RUN','W9':'Existing standalone retirement controls do not exercise retained native thread-group retirement failure','W10':'Ten native group rounds per CPU balance frames/FDs/names and show task/owner Reaped; full heap/ledger-slot census NOT_RUN'}})
fault=load(F/'retirement-controls/guest-results.json') if (F/'retirement-controls/guest-results.json').exists() else {'status':'NOT_RUN'}
write('resource-retirement.json',{'status':'PARTIAL','normal_and_forced':retirement,'existing_native_controls':fault,'unchanged_source_paths':['kernel/src/process/address_space.rs:reclaim_user_space','kernel/src/memory/shared.rs:cleanup_shared_pages','kernel/src/memory/tlb.rs','kernel/src/memory/retirement.rs','kernel/src/memory/shared_quota.rs'],'source_invariants':['all same-VM borrowers Reaped before owner reclaim','outgoing stack pin/current/core/queue guards unchanged','active VM mask zero before owner reclaim','persistent retirement obligations assert before reclaim','SHM withdrawal failure retains ownership and fails stop'],'unqualified':['TLB/retirement failure while final native borrower retires','interrupted group cleanup callback','group-specific exact mapping/backing/quota release census','borrower-created SHM view continuity: sys_shm_alloc/map/free still use current_process_mut and mapped_shared remains per-task']})
write('native-regressions.json',{'status':'PASS' if all(x['C']['passed']==73 and x['Cxx']['passed']==21 and x['heap_abi']=='PASS' and all(s=='PASS' for s in x['tls_once'].values()) for x in cells) and all(x['exit']==0 for x in executions+more if x['name'].startswith(('tcp-','pthread-abi','cxx-abi-vendor'))) else 'FAIL','matrix':cells,'executions':executions+more,'vendor_integrity':'C++ PROVENANCE and ABI checker fresh; all baseline vendor file hashes additionally checked in source-integrity.json','historical_failures_preserved':'Initial owner failure is fresh at initial-baseline, not canceled by later passes'})
write('thread-group-contract.json',{'ownership':'Retained original Process; no additional ThreadGroup object','identity':'AddressSpaceIdentity root+generation and original PID/incarnation; task identity remains separate','terminal':'native_group_exit: None -> Some(first code), serialized by SCHEDULER; Reaped remains physical retirement','membership':'Existing bounded task table; reserve through publish under one scheduler guard, exact VM identity. Finished owner allowed only with no terminal receipt. No new refcount','contracts':{'T1':{'status':'PASS','evidence':'W1/W2 and retained-admission on 1/2/4'},'T2':{'status':'PASS','evidence':'C pthread and TLS/once suites on 1/2/4'},'T3':{'status':'PASS','evidence':'W3 final surviving borrower and all group owner resources physically retired on 1/2/4'},'T4':{'status':'PARTIAL','evidence':'exit23/raw15729 and main17 with live borrowers pass; new group-parent wait schedule NOT_RUN'},'T5':{'status':'PARTIAL','evidence':'original live and retained-owner SIGKILL137 pass; group capability/mapping revocation under fault remains unqualified'},'T6':{'status':'PASS','evidence':'main return17 + one atexit callback kills live borrower on 1/2/4; Rust/IPC wrappers now157'},'T7':{'status':'PARTIAL','evidence':'group liveness and host exact-session scan; real retained-owner PTY witness NOT_RUN'},'T8':{'status':'PARTIAL','evidence':'exact VM group/descriptor lookup, existing generation-qualified privileged paths; native W8 NOT_RUN'}},'task_scoped_authority':'Task-owned IPC endpoints close at that task death; dead-task grants remain unavailable. No borrower owner impersonation or trusted-service transfer','process_helpers':'IPC ProcessExit::exit and Rust libc exit use existing157. Worker/pthread trampolines retain20. C exit/main and std sl_std_exit retain157','no_new_c_exports':True})
write('security-regressions.json',{'status':'PARTIAL','executions':[x for x in executions if x['name'] in ['kernel-host','ipc-host','scheduler-host','production-check']],'source_review':{'authority':'No trust flags, endpoint ownership or capability vector transfer added. is_exiting remains task authority guard; process_lifetime_live is only group liveness','forced_exit':'Retained original owner accepts authorized SIGKILL; existing foreground/manager/PTY/generation authorization precedes it','identity':'VM generation-qualified lookup and descriptor owner check. Numeric APIs remain numeric; W8 not inferred','session':'ProcessIsAlive keeps privileged incarnation check. Exact session teardown handles retained owner. Host adapter cannot qualify real PTY','forward_progress':'No new spin/wait/IPI acknowledgment loop; existing bounded table scans under scheduler guard; terminal receipt before nested callbacks; physical reap on existing scheduler path','failure':'Prepublication RAII remains; receipt owns terminal state through callback failure, existing persistent-retirement and SHM fail-stop assertions preserved'},'uncovered':['native authenticated IPC/capability operation after initial owner task exit','native stale-incarnation/group-generation rejection across reuse','native final-exit versus forced-kill overlap','live PTY/session/tab teardown after owner thread-only exit','group-specific TLB/quarantine/refund failure injection','borrower-created SHM view lifetime across that borrower exit remains a per-task authority boundary requiring native qualification']})
base=baseline;changes=[];missing=[];symlink_changes=[]
for p,h in base['files'].items():
 q=R/p
 if not q.is_file():missing.append(p)
 elif sha(q)!=h:changes.append(p)
for p,v in base['symlinks'].items():
 q=R/p
 if not q.is_symlink() or str(q.readlink())!=v:symlink_changes.append(p)
expected=['ipc/src/lib.rs','kernel/src/arch/x86_64/syscall.rs','kernel/src/process/foreground.rs','kernel/src/process/mod.rs','kernel/src/sched/mod.rs','kernel/src/sched/thread_admission.rs','sunlight-libc/src/lib.rs','sunlight-libc/src/sys.rs','tools/foreground-scope-proof.rs','tools/native-c/guest.py']
new_kernel='kernel/src/sched/native_group.rs'
# Invert the one authorized edit in the pre-existing dirty libc file to prove
# every earlier user byte is still present, not just a HEAD-relative diff.
lib=(R/'sunlight-libc/src/lib.rs').read_text().replace('sys::syscall1(sys::SYS_PROCESS_EXIT_GROUP, code);','sys::syscall1(sys::SYS_PROCESS_EXIT, code);',1)
user_lib_preserved=hashlib.sha256(lib.encode()).hexdigest()==base['files']['sunlight-libc/src/lib.rs']
status=subprocess.check_output(['git','status','--porcelain=v1','--untracked-files=all'],cwd=R,text=True);(E/'final-worktree.log').write_text(status)
sourceok=sorted(changes)==sorted(expected) and not missing and not symlink_changes and user_lib_preserved
integrity={'status':'PASS' if sourceok else 'FAIL','revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip(),'branch':subprocess.check_output(['git','branch','--show-current'],cwd=R,text=True).strip(),'baseline_file_count':len(base['files']),'symlink_count':len(base['symlinks']),'changed_from_baseline':changes,'expected_changed':expected,'new_kernel_file':{new_kernel:sha(R/new_kernel)},'missing':missing,'symlink_changes':symlink_changes,'earlier_dirty_libc_bytes_preserved_by_inverse_patch':user_lib_preserved,'validated_source_matches':all(sha(R/p)==h for p,h in source.items()),'all_execution_source_matches':all(x.get('source_matches',True) for x in executions),'current_source_sha256':{p:sha(R/p) for p in expected+[new_kernel]},'test_source_sha256':{str(p.relative_to(R)):sha(p) for p in E.glob('*') if p.suffix in ['.c','.py','.md']},'full_status':'final-worktree.log','no_commit_or_push':True,'earlier_pilot_matrix':'Intermediate source only; never substituted for validated/ guests','remaining_integrity_scope':'Content hashes and symlink targets; no clean-room or bit-identical rebuild claim'}
write('source-integrity.json',integrity)
gates={'G1':'PASS','G2':'PASS','G3':'PARTIAL','G4':'PARTIAL','G5':'PARTIAL','G6':'PARTIAL','G7':'PARTIAL','G8':load(E/'native-regressions.json')['status'],'G9':'PASS' if sourceok and all(x['exit']==0 for x in executions if x['name'] in ['production-check','kernel-host','ipc-host','scheduler-host']) else 'PARTIAL'}
write('validation-summary.json',{'verdict':'PARTIAL','complete':False,'gates':gates,'selected_model':'Option A: original Process retained as shared lifetime authority','revision':base['revision'],'source_manifest':'validated/validated-source.json','cpu_matrix':'cpu-matrix.json','witnesses':'lifecycle-witnesses.json','source_integrity':integrity['status'],'executions':executions+more,'required_uncovered_properties':load(E/'security-regressions.json')['uncovered'],'recommended_r2b5':'Qualify retained group authority with external final-exit/forced-kill coordination, authenticated IPC/capability and real PTY witnesses, stale generation reuse, group-integrated retirement failure and exact mapping/backing/quota/heap census; no Condvar, detached helper or CPython work','no_commit':True,'no_push':True})
print('Assessment complete',gates)
