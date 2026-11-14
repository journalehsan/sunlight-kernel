from pathlib import Path
import json,re,copy
E=Path(__file__).resolve().parent;old=E.parent/'results-2026-11-14-runtime-r2a';d=json.loads((old/'cpython-compatibility-inventory.json').read_text())
d['predecessor']='../results-2026-11-14-runtime-r2a/cpython-compatibility-inventory.json';d['scope']='R2B.1; current evidence supersedes only listed requirements. Other entries remain R2A historical classifications; no unsupported optional modules inferred.'
for row in d['requirements']:
 row['evidence_epoch']='R2A historical unless R2B1_evidence is present'
 row['R2A_classification']=row['classification']
 row['R2A_evidence']=row.get('evidence',[])
 row['evidence']=['../results-2026-11-14-runtime-r2a/'+p for p in row.get('evidence',[]) or []]

def update(name,classification,locations,header,evidence,scope,blocker='none within the bounded stated contract'):
 matches=[x for x in d['requirements'] if x['requirement']==name]
 if not matches:matches=[dict(requirement=name,kind='symbol or public type')];d['requirements']+=matches
 for x in matches:
  x.update(classification=classification,sunlight_implementation_locations=locations,header_availability=header,compile_result='PASS',link_result='PASS' if x.get('kind')!='type or macro' else 'NOT_APPLICABLE',link_availability=True,runtime_verification='PASS (bounded native tests)',native_execution_result='PASS (bounded native tests)',semantic_confidence=scope,remaining_blocker=blocker,evidence=evidence,R2B1_evidence=evidence,evidence_epoch='R2B.1 final source',archive_export=name in {'pthread_key_create','pthread_key_delete','pthread_getspecific','pthread_setspecific','pthread_once','pthread_exit','strerror','strerror_r'})
tsd_e=['final2/acceptance-focused-1/results.json','final2/acceptance-focused-2/results.json','final2/acceptance-focused-4/results.json','exit-race-guest-2/results.json','exit-race-guest-4/results.json']
for name in ['pthread_key_t','pthread_key_create','pthread_key_delete','pthread_getspecific','pthread_setspecific']:
 update(name,'VERIFIED',['sunlight-libc/include/pthread.h','sunlight-libc/src/pthread.rs','sunlight-libc/src/tls.rs'],True,tsd_e,'Generation-aware process keys and current-TCB values; callback selection/deletion serialized. VERIFIED is bounded functionality, not full lifecycle conformance.','initial-owner thread-only exit remains unsupported; full lifecycle separately PARTIAL; exact schedules UNQUALIFIED')
for name in ['pthread_once','pthread_once_t','PTHREAD_ONCE_INIT']:
 update(name,'VERIFIED',['sunlight-libc/include/pthread.h','sunlight-libc/src/pthread.rs','sunlight-libc/src/tls.rs'],True,tsd_e[:3]+['recursion-guest-2/results.json'],'Acquire/release state machine, futex wait/wake-all, explicit pthread_exit resets active initializer. Recursive same-control does not complete.','cancellation/exception/fork repair unsupported; exact registered-waiter race schedule UNQUALIFIED')
for name in ['strerror','strerror_r']:
 update(name,'VERIFIED',['sunlight-libc/include/string.h','sunlight-libc/src/string.rs','sunlight-libc/include/errno.h'],True,['final2/header-guest-c/results.json','final2/header-guest-cxx/results.json','strerror-threads-guest-2/results.json','strerror-threads-guest-4/results.json'],'Immutable C-locale messages, all 40 supported distinct numbers; selected int-returning strerror_r ABI in every feature profile; errno unchanged.','locale-dependent messages / GNU pointer-returning strerror_r not supplied')
update('pthread_exit','PARTIAL',['sunlight-libc/src/pthread.rs:305','sunlight-libc/src/pthread.rs:584','kernel/src/sched/mod.rs:4580'],True,tsd_e+['final2/owner-exit-observation/serial.log'],'Worker normal/explicit exit and detached TSD cleanup validated; initial-owner exit demonstrably terminates borrowers.','Initial-thread POSIX thread-only exit; bounded lazy detached mapping/control reaping retains overflow until process exit')
d['requirements'].append(dict(requirement='pthread destructor lifecycle / detached resource reclamation',classification='PARTIAL',header_availability=True,sunlight_implementation_locations=['sunlight-libc/src/pthread.rs','sunlight-libc/src/tls.rs'],compile_result='PASS',link_result='PASS',native_execution_result='PARTIAL',evidence=tsd_e+['final2/owner-exit-observation/serial.log'],remaining_blocker='initial-thread exit survival FAIL; pre-process-exit zero per-thread metadata retention UNQUALIFIED; detached queue pre-existing 64-entry bound'))
for name in ['pthread_cond_t','pthread_condattr_t','pthread_cond_init','pthread_cond_wait','pthread_cond_timedwait','pthread_cond_signal','pthread_cond_destroy']:
 found=[x for x in d['requirements'] if x['requirement']==name]
 if not found:found=[dict(requirement=name)];d['requirements']+=found
 for x in found:x.update(classification='MISSING',header_availability=False,sunlight_implementation_locations=[],compile_result='FAIL in actual CPython thread.c',link_result='UNKNOWN (not reached)',native_execution_result='NOT_RUN',remaining_blocker='out of R2B.1 scope; no fake types or functions',evidence=['final2-compile-Python-thread-o.log'])
for name in ['strncpy']:
 found=[x for x in d['requirements'] if x['requirement']==name]
 if not found:found=[dict(requirement=name)];d['requirements']+=found
 for x in found:x.update(classification='MISSING',header_availability=False,sunlight_implementation_locations=[],compile_result='FAIL (undeclared, actual Parser/pegen.c)',link_result='UNKNOWN (not reached)',native_execution_result='NOT_RUN',remaining_blocker='genuine next libc dependency outside error-string scope',evidence=['final2-compile-python.log'])
d['R2A_actual_compile_findings']=d['actual_compile_findings'];d['actual_compile_findings']=[]
for p in sorted(E.glob('final2-compile-*.log')):
 for line in p.read_text().splitlines():
  if 'error:' in line or 'warning:' in line:d['actual_compile_findings'].append(dict(diagnostic=line,evidence=p.name,classification='MISSING' if 'error:' in line else 'UNKNOWN'))
d['core_first_blocker']=dict(explicit_target='Python/thread.c',full_make_first_failed_translation_unit='Parser/pegen.c',diagnostic='unknown type pthread_condattr_t',additional_missing=['pthread_cond_t','condition-variable functions','strncpy'],evidence=['final2-compile-Python-thread-o.log','final2-compile-python.log'])
d['cpython_execution']='NOT_RUN (no interpreter link reached)'
(E/'cpython-compatibility-inventory.json').write_text(json.dumps(d,indent=2))
