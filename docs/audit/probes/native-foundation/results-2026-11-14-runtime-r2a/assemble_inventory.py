import pathlib,json,copy,re
r=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent;r1=e.parent/'results-2026-11-14-runtime-r1';old=json.loads((r1/'cpython-compatibility-inventory.json').read_text());items=copy.deepcopy(old['requirements']);probes={x['requirement']:x for x in json.loads((e/'api-probes.json').read_text())}
verified_headers={'assert.h','inttypes.h','sys/types.h','ctype.h'}
math={'atanh','atanhf','frexp','frexpf','lgamma','lgammaf','lgamma_r','lgammaf_r','modf','modff','signgam'}
integer={'imaxabs','imaxdiv','strtoimax','strtoumax','wcstoimax','wcstoumax'}
ctype={'isalnum','isalpha','isblank','iscntrl','isdigit','isgraph','islower','isprint','ispunct','isspace','isupper','isxdigit','tolower','toupper'}
known={x['requirement'] for x in items}
for name in (math|integer|ctype|{'pthread_key_t','pthread_cond_t','pthread_condattr_t','pthread_once_t','atanhl','frexpl','lgammal','modfl','SSIZE_MAX','EBUSY','ldexp','log','strerror'} )-known:
 items.append(dict(requirement=name,kind='symbol' if name not in {'pthread_key_t','pthread_cond_t','pthread_condattr_t','pthread_once_t','SSIZE_MAX','EBUSY'} else 'type or macro',classification='UNKNOWN',cpython_subsystem='core/runtime (float variants ancillary; long-double not selected by observed core probes)',sunlight_implementation_locations=[]))
for x in items:
 name=x['requirement'];x['R1_classification']=x.get('classification','UNKNOWN');x['historical_evidence']=x.pop('evidence',None)
 x['evidence']=['../results-2026-11-14-runtime-r1/cpython-compatibility-inventory.json']
 x['compile_result']='UNKNOWN';x['link_result']='UNKNOWN';x['native_execution_result']='UNKNOWN';x['remaining_blocker']=x.get('proposed_next_action','not fully qualified')
 if x['classification']=='VERIFIED':x['classification']='PARTIAL' # R1 header-only parsing is not native semantics.
 if name in probes:
  p=probes[name];x['compile_result']='PASS' if p['header_available'] else 'FAIL';x['link_result']='PASS' if p.get('link_available') is True else 'FAIL' if p.get('link_available') is False else 'UNKNOWN';x['evidence']=[p['evidence']] if isinstance(p['evidence'],str) else p['evidence'];x['header_availability']=p['header_available']
  if 'archive_export' in p:x['archive_export']=p['archive_export']
  if not p['header_available'] and not p.get('archive_export',False):x['classification']='MISSING'
 if name in verified_headers|integer|ctype|math|{'SSIZE_MAX','EBUSY'}:
  x.update(classification='VERIFIED',header_availability=True,link_availability=True if name not in verified_headers|{'SSIZE_MAX','EBUSY'} else 'NOT_APPLICABLE',compile_result='PASS',link_result='PASS' if name not in verified_headers|{'SSIZE_MAX','EBUSY'} else 'NOT_APPLICABLE',native_execution_result='PASS',remaining_blocker='none within the bounded tested contract',semantic_confidence='bounded native cases; not entire C/POSIX/libm conformance',evidence=['acceptance-focused/focused-results.json','acceptance-focused/final-math-guest/results.json','acceptance-focused/final-headers-c-guest/results.json','acceptance-focused/final-headers-cxx-guest/results.json','acceptance-focused/final-ctype-guest/results.json'])
  if name in math:x['sunlight_implementation_locations']=['sunlight-libc/src/math.rs','sunlight-libc/vendor/libm/src/math'];x['cpython_subsystem']='configure mandatory math / core numeric helpers; float variants ancillary'
  elif name in integer or name=='assert.h':x['sunlight_implementation_locations']=['sunlight-libc/src/inttypes.rs','sunlight-libc/include/'+('assert.h' if name=='assert.h' else 'inttypes.h')]
  elif name in ctype or name=='ctype.h':x['sunlight_implementation_locations']=['sunlight-libc/include/ctype.h','sunlight-libc/src/ctype.rs']
  elif name in verified_headers:x['sunlight_implementation_locations']=['sunlight-libc/include/'+name]
  else:x['sunlight_implementation_locations']=['sunlight-libc/include/'+('limits.h' if name=='SSIZE_MAX' else 'errno.h')]
 if name in {'ldexp','log'}:
  x.update(classification='PARTIAL',compile_result='PASS',link_result='PASS',native_execution_result='PASS (bounded smoke)',semantic_confidence='existing correctly typed compiler-builtins export, newly declared; errno/full numerical qualification absent',sunlight_implementation_locations=['pinned compiler-builtins/src/math/mod.rs','sunlight-libc/include/math.h'],remaining_blocker='broader error/accuracy qualification',evidence=['acceptance-focused/final-headers-c-guest/results.json','acceptance-focused/final-headers-cxx-guest/results.json'])
 if name in {'pthread_key_t','pthread_cond_t','pthread_condattr_t','pthread_once_t'}:
  x.update(classification='MISSING',compile_result='FAIL',link_result='NOT_APPLICABLE',native_execution_result='NOT_RUN',remaining_blocker='requires real threading implementation outside R2A; no fake types',cpython_subsystem='native TSS / GIL / pthread core backend',evidence=['acceptance-compile-Programs-python-o.log','acceptance-compile-Python-thread-o.log','pthread-type-probes.json'])
 if name in {'atanhl','frexpl','lgammal','modfl'}:
  x.update(classification='MISSING',compile_result='NOT_RUN',link_result='FAIL (actual synthetic-declaration native link)' ,native_execution_result='NOT_RUN',remaining_blocker='x87 80-bit implementation required if selected; none selected by observed CPython core',evidence=['archive-missing-target-symbols.json',name+'-link.log'],semantic_confidence='not aliased to double; target long double mantissa 64/storage 16/alignment 16')
 if name=='lgammaf_r':
  x.update(classification='PARTIAL',native_execution_result='indirect algorithm path via lgammaf PASS; direct C ABI NOT_RUN',remaining_blocker='direct reentrant float C ABI call qualification')
 if name=='strerror':
  x.update(classification='MISSING',compile_result='FAIL (undeclared in actual thread.c)',link_result='FAIL',native_execution_result='NOT_RUN',remaining_blocker='libc error-string API outside R2A',evidence=['acceptance-compile-Python-thread-o.log','strerror-link.log'])
 x['runtime_verification']=x['native_execution_result']
 if x.get('remaining_blocker')=='none within the bounded tested contract':x['proposed_next_action']='retain bounded regression coverage'
 if name in {'atanhl','frexpl','lgammal','modfl','strerror'}:x['header_availability']=False; x['link_availability']=False
# Exact observed diagnostics, separated from source-inferred dependencies.
findings=[]
for p in e.glob('acceptance-compile-*.log'):
 for line in p.read_text().splitlines():
  if 'error:' in line:
   category='POSIX threading gap' if 'pthread_' in line else 'libc API gap' if 'strerror' in line else 'CPython platform integration' if 'timespec' in line else 'Unknown'
   findings.append(dict(diagnostic=line,classification=category,evidence=p.name))
result=dict(reference='CPython 3.13.0',predecessor='../results-2026-11-14-runtime-r1/cpython-compatibility-inventory.json',scope='Bounded native header/math qualification; optional modules remain separate; no completeness percentage',requirements=items,actual_compile_findings=findings,core_first_blocker='pthread_key_t in Include/cpython/pythread.h:37',optional_extensions='See final configure module detection; absent optional external libraries are not the first core blocker',cpython_execution='NOT_RUN')
(e/'cpython-compatibility-inventory.json').write_text(json.dumps(result,indent=2))
