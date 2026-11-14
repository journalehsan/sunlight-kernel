import pathlib,json,re
E=pathlib.Path(__file__).resolve().parent;P=E.parent/'results-2026-11-14-py2'
def write(n,d):(E/n).write_text(json.dumps(d,indent=2)+'\n')
x=json.loads((E/'object-inventory.json').read_text());before=json.loads((P/'cpython-compilation-matrix.json').read_text());old={u['object']:u for u in before['units']}
fs={'Modules/_io/fileio.o','Modules/_stat.o','Modules/getpath.o','Modules/main.o','Modules/posixmodule.o','Objects/obmalloc.o','Python/fileutils.o','Python/marshal.o'}
sig={'Modules/_threadmodule.o','Modules/faulthandler.o','Modules/signalmodule.o'}
for u in x['units']:
 o=u['object'];b=old[o];u['PY2_result']=b['status'];u['PY2_first_diagnostic']=b['first_error'];u['mandatory']=b.get('mandatory',True)
 if u['compile_result']=='PASS':
  u['root_cause_classification']=None;u['ownership_layer']=None;u['planned_repair']='None needed for this translation unit';continue
 if o in fs:
  u['root_cause_classification']='Filesystem/platform';u['ownership_layer']='Native VFS/kernel metadata/path contracts and libc adapter ABI';u['planned_repair']='Design authoritative native metadata/canonical path contracts in separately authorized owning-layer work; do not synthesize missing fields or symlink resolution.';u['required_api_header']+=['sys/stat.h (truthful metadata definition)'] if o!='Objects/obmalloc.o' else []
 elif o in sig:
  u['root_cause_classification']='Signals';u['ownership_layer']='Native kernel signal delivery/dispositions/masks';u['planned_repair']='Separately authorize required signal semantics, then libc adapters. No constants-only compatibility.'
 elif o=='Python/bootstrap_hash.o':
  u['root_cause_classification']='Entropy';u['ownership_layer']='Native secure entropy service/IPC transport';u['planned_repair']='Establish secure nonblocking entropy and readiness/error contract before declaring GRND_NONBLOCK.'
 elif o=='Python/sysmodule.o':
  u['root_cause_classification']='Unsupported kernel contract';u['ownership_layer']='Native worker/group process identity';u['planned_repair']='Authoritative process-group ID contract before C getpid export.'
 elif o=='Modules/_localemodule.o':
  u['root_cause_classification']='libc/header';u['ownership_layer']='libc locale metadata';u['planned_repair']='Implement real consistent C-locale DAY/ABDAY/MON/ABMON and date/time-format metadata; no kernel dependency asserted.'
 else:
  u['root_cause_classification']='Time/environment';u['ownership_layer']='libc time/calendar or environment ownership; additional native signal/stat requirements in pylifecycle';u['planned_repair']='Implement calendar/time interfaces over actual native clocks; design mutable environment without invalidating existing static Rust references; preserve independent signal/stat blockers.'
 u['required_api_header']=sorted(set(u['required_api_header']));u['diagnostic_completeness']='All errors permitted up to a fatal missing include; later dependencies may be masked. No claim of complete unresolved-symbol inventory.'
counts=dict(configured=x['configured'],attempted=sum(u['compile_result']!='NOT_RUN' for u in x['units']),passed=sum(u['compile_result']=='PASS' for u in x['units']),failed=sum(u['compile_result']=='FAIL' for u in x['units']),unattempted=sum(u['compile_result']=='NOT_RUN' for u in x['units']))
assert counts['configured']==counts['passed']+counts['failed']+counts['unattempted']==180
assert counts['attempted']==counts['passed']+counts['failed']==180
x.update(counts);write('object-inventory.json',x)
counts.update(status='FAIL',previous_configured=180,object_set_unchanged=True,first_remaining_failure=next(u['first_failing_diagnostic'] for u in x['units'] if u['object']=='Objects/obmalloc.o'),newly_successful=[u['object'] for u in x['units'] if u['compile_result']=='PASS' and u['PY2_result']!='PASS'],mandatory_failed=[u['object'] for u in x['units'] if u['mandatory'] and u['compile_result']=='FAIL'],notes=['Modules/getbuildinfo.o independently compiled with exact upstream shell recipe; make target itself remains blocked by dependencies.','Core-make log is interleaved; first failing diagnostic for each object is taken from its independent log.','No mandatory object removed and no optional module newly reclassified.'])
write('compile-results.json',counts)
a=json.loads((E/'configure-results.json').read_text());a.update(manifest='configure-manifest.json',configured_object_count=180,unchanged_object_set=True,matching_build_python='/tmp/sunlight-r1-cpython/build-host/python',source_version='CPython 3.13.0 final',recognition_patch='inherited-platform-recognition.patch',new_source_changes=False,setup_local='Retained PY-2 Modules/Setup.local without changes',feature_changes=['Native setvbuf and clock_getres now detected with implemented contracts.'],cache_policy='Inherited evidence-backed no-semaphore and no-/dev/ptmx,/dev/ptc cache; no unsupported function forced to yes.')
write('configure-results.json',a)
print(counts)
