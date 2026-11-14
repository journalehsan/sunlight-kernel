import pathlib,json,hashlib,os,subprocess,re
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent
read=lambda p:json.loads((E/p).read_text())
def write(p,x):(E/p).write_text(json.dumps(x,indent=2)+'\n')
rows=read('verified-executions.json');assert rows[-1]['name']=='verified-locked-repeat','Validation still running'
assert all(x['exit']==0 and x['source_matches'] for x in rows),[x for x in rows if x['exit']!=0 or not x['source_matches']]
matrix=[];semantics=[]
for cpu in [2,4]:
 tcp=read(f'verified-tcp-cpu-{cpu}/poll-summary.json');assert tcp['result']=='PASS' and all(tcp['checks'].values())
assert all(x['exit']==0 for x in read('clock-header-order-verified.json'))
for cpu in [1,2,4]:
 all_cases=read(f'verified-functional-cpu-{cpu}/results.json')
 assert len(all_cases)==32 and all_cases['header-abi']['pass_']
 focused_names=['condvar','libc','exit','heap','admission','pthread-tsd-once','pthread-tsd-exit-race','pthread-once-recursive','strerror-threads']
 supplemental_names=['condvar-timeout','string-guard','owner-exit-0','owner-exit-37','owner-exit-255']
 f={n:all_cases[n] for n in focused_names};s={n:all_cases[n] for n in supplemental_names};l={n:x for n,x in all_cases.items() if n.startswith('lifecycle-')};r=read(f'verified-regression-cpu-{cpu}/results.json')
 assert len(l)==17 and len(r)==94

 assert all(x['pass_'] for group in [f,s,l,r] for x in group.values())
 cxx={n:x for n,x in r.items() if n.startswith(('cxx-','cpp-'))};c={n:x for n,x in r.items() if n not in cxx};assert len(c)==73 and len(cxx)==21
 for n,x in {**f,**s}.items():
  assert 'FAIL' not in x['stdout'] and not x['stderr'],(cpu,n,x)
 assert 'PY1 condvar PASS:' in f['condvar']['stdout']
 assert '81 boundary cases' in f['libc']['stdout'] and 'strings and strtol PASS' in f['libc']['stdout']
 assert f['exit']['stdout']=='PY1 worker _Exit(37)\n' and f['exit']['exit']==37
 assert s['string-guard']['stdout']=='PY1 strncpy guard-page PASS\n'
 t=re.search(r'wakes=(\d+) timeouts=(\d+)',s['condvar-timeout']['stdout']);assert t and int(t[1])>0 and int(t[2])>0 and int(t[1])+int(t[2])==60
 for code in [0,37,255]:assert s['owner-exit-'+str(code)]['exit']==code and s['owner-exit-'+str(code)]['stdout']=='PY1 initial owner immediate group exit\n'
 matrix.append(dict(cpus=cpu,C='73/73',CXX='21/21',lifecycle='17/17',TLS_once='4/4',heap_ABI='PASS',retained_admission='PASS',condvar='PASS (bounded functional)',signal_timeout_stress=dict(wakes=int(t[1]),timeouts=int(t[2]),exact_race='UNQUALIFIED'),strncpy_strings='PASS',immediate_exit='PASS',TCP='PASS' if cpu in [2,4] else 'NOT_RUN (unsupported)'))
 semantics.append(dict(cpus=cpu,exact_exit_output_checked=True,guard_page_checked=True,timeout_outcomes_checked=True,evidence=[f'verified-functional-cpu-{cpu}/results.json']))
write('regression-matrix.json',matrix);write('focused-semantic-checks.json',semantics)
base=read('source-baseline.json');changed=[];missing=[];links=[]
for n,h in base.items():
 p=R/n
 if isinstance(h,dict):
  if not p.is_symlink() or os.readlink(p)!=h['symlink']:links.append(n)
 elif not p.is_file():missing.append(n)
 elif hashlib.sha256(p.read_bytes()).hexdigest()!=h:changed.append(n)
preserve=read('incoming-content-preservation.json');assert set(changed)==set(preserve) and all(x['matches'] for x in preserve.values()) and not missing and not links
assert all(hashlib.sha256((R/n).read_bytes()).hexdigest()==h for n,h in read('verified-source.json').items())
write('source-integrity.json',dict(status='PASS',original_entries=len(base),changed_original_files=sorted(changed),missing_original_files=missing,changed_symlinks=links,incoming_content_reconstructed_exactly=True,new_production_files=['sunlight-libc/src/condvar.rs','sunlight-libc/include/bits/time.h'],all_other_incoming_files_unchanged=True,final_source=read('verified-source.json')))
for name,cmd in [('final-worktree',['git','status','--short']),('final-revision',['git','rev-parse','HEAD']),('diff-check',['git','diff','--check'])]:
 p=subprocess.run(cmd,capture_output=True,text=True);(E/(name+'.log')).write_text(repr(cmd)+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n');assert p.returncode==0
assert read('cpython-source-integrity.json')['matches']
x=read('compile-summary.json')
write('validation-summary.json',dict(verdict='PY-1 PARTIAL',gates={'G1':'PARTIAL sequencing: worktree/build baseline pre-edit; independent upstream archive verification and pristine manifest captured subsequently','G2':'PARTIAL: native functional API passes; exact race schedules UNQUALIFIED','G3':'PARTIAL: implemented bootstrap additions pass; mandatory libc gaps remain','G4':'PASS attempt; core compilation FAIL','G5':'NOT_RUN: compilation prerequisite fails','G6':'NOT_RUN: no interpreter executable','G7':'PASS','G8':'PASS'},readiness={'cross_configure':'PASS','core_compilation':'FAIL','static_interpreter_link':'NOT_RUN','loader_startup':'NOT_RUN','first_python_statement':'NOT_RUN','first_python_output':None,'imports_basic_runtime':'NOT_RUN','threaded_python':'NOT_READY','production':'NOT_READY'},compile={k:x[k] for k in ['configured_object_targets','attempted_translation_units','successful_translation_units','failed_translation_units','not_attempted_targets','first_serial_error']},regressions=matrix,inherited_limits=['R2B.4 and R2B.5 retirement/security remain PARTIAL or UNQUALIFIED','Authenticated IPC continuity not upgraded','Autonomous detached reclamation not added'],production_changes=[*sorted(changed),'sunlight-libc/src/condvar.rs','sunlight-libc/include/bits/time.h'],commit=False,push=False,remaining_bootstrap_blockers='libc-gap-inventory.json',recommended_next='PY-2 mandatory libc and static core closure',evidence_root=str(E.relative_to(R)),report='docs/runtime/python-py1-minimal-static-bootstrap.md'))
p=R/'docs/runtime/python-py1-minimal-static-bootstrap.md';text=p.read_text().replace('MATRIX_PENDING','PASS').replace('INTEGRITY_PENDING','PASS')
table='| CPUs | C | C++ | Lifecycle | TLS/once | Heap/ABI | Condvar + timeout | Strings + guard | _Exit | TCP |\n|---|---|---|---|---|---|---|---|---|---|\n'
for m in matrix:table+=f"| {m['cpus']} | 73/73 | 21/21 | 17/17 | 4/4 | PASS | PASS, bounded | PASS | PASS | {m['TCP']} |\n"
text=text.replace('MATRIX_TABLE_PENDING',table).replace('INTEGRITY_TEXT_PENDING','Final integrity checks pass for all 71,182 original entries: exactly the nine authorized original production files differ, no original file is missing, and no symlink target changed. All final regression commands report matching production hashes. No kernel or prior evidence change is attributed to PY-1.')
p.write_text(text);print('PY-1 PARTIAL; final regressions and integrity PASS')
