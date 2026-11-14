from pathlib import Path
import subprocess,json,hashlib,csv,re
R=Path.cwd();E=Path(__file__).resolve().parent;F=E/'final2'
def read(path):return json.loads((E/path).read_text())
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
initial=read('initial-source.json'); snapshot=read('final-runtime-inputs.json')
assert 'Finished `release`' in (E/'baseline-locked-build.log').read_text()
assert read('cpython-source-integrity.json')['changed_original_files']==[]
assert all((R/n).is_file() and sha(R/n)==h for n,h in snapshot['files'].items()),'source changed after final acceptance input snapshot'
allowed={'sunlight-libc/include/errno.h','sunlight-libc/include/limits.h','sunlight-libc/include/pthread.h','sunlight-libc/include/string.h','sunlight-libc/src/pthread.rs','sunlight-libc/src/string.rs','sunlight-libc/src/tls.rs'}
changed={};missing=[]
for n,h in initial['hashes'].items():
 p=R/n
 if not p.is_file():missing.append(n)
 elif sha(p)!=h:changed[n]=dict(before=h,after=sha(p))
assert not missing,missing
assert set(changed)==allowed,set(changed)-allowed
files=[n for n in subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z']).decode().split('\0') if n and (R/n).is_file()]
added=[n for n in files if n not in initial['hashes']]
report='docs/runtime/native-runtime-r2b1-pthread-tls-once.md'; eprefix=str(E.relative_to(R))+'/'
new_tests={'tools/native-c/tests/'+n for n in ['pthread-tsd-once.c','pthread-tsd-headers.c','pthread-tsd-exit-race.c','pthread-once-recursive.c','strerror-threads.c']}
unexpected=[n for n in added if n not in new_tests and n!=report and not n.startswith(eprefix)]
assert not unexpected,unexpected
revision=subprocess.check_output(['git','rev-parse','HEAD']).decode().strip();assert revision==initial['revision']
status=subprocess.check_output(['git','status','--short']).decode();diffcheck=subprocess.run(['git','diff','--check'],capture_output=True);assert diffcheck.returncode==0
(E/'git-diff-check.log').write_bytes(diffcheck.stdout+diffcheck.stderr+b'\nexit=0\n')
manifest=dict(revision=revision,branch=subprocess.check_output(['git','branch','--show-current']).decode().strip(),initial_status=initial['status'],final_status=status,original_file_changes=changed,missing_original_files=missing,unexpected_original_changes=[],unexpected_additions=unexpected,new_test_files=sorted(new_tests),source_input_fingerprint_sha256=snapshot['fingerprint_sha256'],runtime_input_snapshot_unchanged_since_final_validation=True,kernel_and_scheduler_unchanged=True,all_preexisting_R1_R2A_files_preserved=True,dependency_locks_and_vendors_unchanged=True,source_diff='source-diff-from-baseline.patch',baseline_production_snapshots='baseline-production/',commit='NOT_RUN per instruction',push='NOT_RUN per instruction',report=report,report_sha256=sha(R/report))
(E/'source-integrity-manifest.json').write_text(json.dumps(manifest,indent=2));(E/'final-git-status.log').write_text(status)
results=read('final2/run-results.json');assert all(x['exit']==0 for x in results),[(x['name'],x['exit']) for x in results if x['exit']]
supp=read('supplemental-results.json');assert len(supp)==12 and all(x['exit']==0 for x in supp),supp
rows=list(csv.DictReader((F/'acceptance-suite/results.csv').open()));assert len(rows)==94 and all(x['build']=='PASS' and x['guest_run']=='PASS' and x['acceptance']=='PASS' for x in rows)
r1=json.loads((E.parent/'results-2026-11-14-runtime-r1/test-result-summary.json').read_text());cpp={x['probe'] for x in r1['native_tests'] if any(s['path'].endswith(('.cpp','.cc','.cxx')) for s in x.get('fixture_sources',[]))};assert len(cpp)==21 and sum(x['probe'] in cpp for x in rows)==21
for row in rows:row.update(language='C++' if row['probe'] in cpp else 'C',source_revision=revision,runtime_input_fingerprint_sha256=snapshot['fingerprint_sha256'],cpus=2)
for cpu in [1,2,4]:assert read(f'final2/acceptance-focused-{cpu}/results.json')['elf']['pass_']
for name in ['header-guest-c','header-guest-cxx','heap-abi-guest']:assert read('final2/'+name+'/results.json')['elf']['pass_']
for name in ['exit-race-guest-2','exit-race-guest-4','strerror-threads-guest-2','strerror-threads-guest-4','recursion-guest-2','process-exit-guest-2']:assert read(name+'/results.json')['elf']['pass_']
assert read('process-exit-guest-2/results.json')['elf']['stdout']=='PROCESS EXIT WITHOUT TSD CLEANUP\n'
assert '3 passed; 0 failed' in (E/'host-tcb-final.log').read_text()
owner=read('final2/owner-exit-observation/results.json')['elf'];assert owner['pass_'] and owner['stdout']=='INITIAL PTHREAD_EXIT WITH LIVE BORROWER\n'
assert 'terminated reason=address-space-owner-exit' in (F/'owner-exit-observation/serial.log').read_text()
tcp=read('final2/tcp-loopback/poll-summary.json');assert tcp['result']=='PASS' and all(tcp['checks'].values())
assert 'ownership, upstream integrity, final ELF/frame inspection PASS' in (F/'cxx-abi-final.log').read_text()
assert '12 native C/C++ ABI checks' in (F/'pthread-abi.log').read_text()
assert 'R1 heap and SysV ABI PASS' in read('final2/heap-abi-guest/results.json')['elf']['stdout']
assert read('final2-configure-result.json')['exit']==0
compile=read('final2-compile-results.json');assert [x['exit'] for x in compile]==[0,0,0,2,2,2]
assert 'unknown type name \'pthread_condattr_t\'' in (E/'final2-compile-Python-thread-o.log').read_text()
assert 'Parser/pegen.c' in (E/'final2-compile-python.log').read_text() and "undeclared function 'strncpy'" in (E/'final2-compile-python.log').read_text()
archive=R/'target/native-c/sysroot/lib/libsunlight_native_c.a';nm=subprocess.check_output(['llvm-nm','-g','--defined-only',archive],stderr=subprocess.DEVNULL).decode(); exports={}
for name in ['pthread_key_create','pthread_key_delete','pthread_getspecific','pthread_setspecific','pthread_once','strerror','strerror_r']:
 lines=[line for line in nm.splitlines() if len(line.split())==3 and line.split()[-1]==name];assert len(lines)==1 and lines[0].split()[-2]=='T',(name,lines);exports[name]=lines
(E/'native-exports.json').write_text(json.dumps(dict(archive=str(archive.relative_to(R)),archive_sha256=sha(archive),symbols=exports),indent=2))
races=[]
for cpu in [1,2,4]:
 for behavior in ['per-thread values / NULL','delete-before-exit handshake','stale generation / reuse','concurrent create-delete churn','concurrent deletion with get/set','normal return destructors','explicit pthread_exit destructors','four destructor passes','destructor reentrant create/set/delete','detached destructor without join','80 create/join churn','128-key exhaustion','12-caller once publication / 120000 completed calls','nested distinct controls','initializer pthread_exit / surviving retry']:
  races.append(dict(behavior=behavior,cpus=cpu,status='PASS',evidence=f'final2/acceptance-focused-{cpu}/results.json',boundary='bounded assertion-based native functional test; stress is not exhaustive interleaving proof'))
for cpu in [2,4]:races.append(dict(behavior='already-running destructor / delete / reuse / release / join witness',cpus=cpu,status='PASS',evidence=f'exit-race-guest-{cpu}/results.json'))
races.extend([dict(behavior='initial-owner pthread_exit retains live borrowers',cpus=2,status='FAIL',gate='G2 PARTIAL',evidence='final2/owner-exit-observation/serial.log'),dict(behavior='recursive same-control once',cpus=2,status='PASS',boundary='bounded no-completion interval and independent thread progress; eternal noncompletion not experimentally proven',evidence='recursion-guest-2/results.json'),dict(behavior='exact already-registered futex waiter at initializer abandonment',status='UNQUALIFIED',reason='started flags and sleeps do not establish kernel waiter state'),dict(behavior='exact wake-before-registration and spurious-wake schedules',status='UNQUALIFIED',reason='source reasoning, no deterministic guest interleaving witness'),dict(behavior='zero per-thread metadata retention before process termination',status='UNQUALIFIED',reason='process reaping is observed; detached queue remains lazy/bounded'),dict(behavior='12-CPU native matrix',status='NOT_RUN',reason='existing runner supports only 1/2/4 CPUs')])
(E/'race-test-results.json').write_text(json.dumps(races,indent=2))
summary=dict(overall='PARTIAL (not R2B.1 COMPLETE)',gates=dict(G1='PASS',G2='PARTIAL',G3='PASS',G4='PASS',G5='PASS',G6='PASS',G7='PASS'),source_revision=revision,branch=manifest['branch'],runtime_input_fingerprint_sha256=snapshot['fingerprint_sha256'],legacy_suite=dict(C=73,Cxx=21,cpus=2,status='PASS',evidence='final2/acceptance-suite/results.csv'),legacy_cases=rows,new_tests=dict(main_fixture_cpus=[1,2,4],header_compile_profiles=6,header_guests=2,in_flight_destructor_guests=2,strerror_thread_guests=2,recursive_noncompletion_guests=1,process_exit_guests=1,positive_native_guest_executions=11,owner_exit_limitation_observations=1,host_tcb_tests=3,legacy_counts_exclude_new_tests=True),cpu_matrix={'1':'focused PASS','2':'focused, supplemental, regression PASS; initial-owner lifecycle FAIL','4':'focused and race/error-string supplements PASS','12':'NOT_RUN (runner unsupported)'},cpython=dict(version='3.13.0',configure='PASS',compile_results=compile,first_explicit_failure_translation_unit='Python/thread.c',full_make_first_failed_translation_unit='Parser/pegen.c',first_diagnostic='unknown type pthread_condattr_t',additional_blockers=['pthread_cond_t and condvar functions','strncpy','time/header integration'],interpreter_link='NOT_RUN',execution='NOT_RUN'),unqualified=[x for x in races if x['status']=='UNQUALIFIED'],remaining_blockers=['initial-thread pthread_exit owner lifecycle','lazy bounded detached metadata reclamation','missing condition variables','strncpy and time/header integration'],integrity='PASS',kernel_changes=False,condvar_implementation=False,commit=False,push=False,report=report)
(E/'validation-summary.json').write_text(json.dumps(summary,indent=2));checks={str(p.relative_to(E)):sha(p) for p in E.rglob('*') if p.is_file() and p.suffix in {'.json','.log','.txt','.csv','.py','.c','.h','.patch','.md','.sh'} and p.name!='evidence-checksums.json'};(E/'evidence-checksums.json').write_text(json.dumps(checks,indent=2));print(json.dumps({k:summary[k] for k in ['overall','gates','source_revision','runtime_input_fingerprint_sha256']},indent=2))
