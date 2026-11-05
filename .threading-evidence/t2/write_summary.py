"""Index real receipts and preserve final T2 source identity; never invent races."""
from pathlib import Path
import hashlib,json,re,subprocess,sys
sys.path.insert(0,'tools')
from threading_t2_validate import identity,assess
from threading_t2_acceptance import assess as acceptance
BASE=Path('.threading-evidence/t2')
RUN=Path((BASE/'release-validation-run.log').read_text().splitlines()[0])
RUNSTAMP=RUN.name.removeprefix('validation-run-')
def result_row(path):
 d=json.loads(path.read_text());return dict(result=str(path.relative_to(BASE)),pass_=d.get('pass',False),seconds=d.get('seconds'),iso_sha256=d.get('iso_sha256'),serial_sha256=d.get('serial_sha256'))
def row(path):
 d=result_row(path);d['pass']=d.pop('pass_');return d
def latest(suffix,after=''):
 choices=sorted(p for p in BASE.iterdir() if p.is_dir() and p.name.endswith('-'+suffix) and p.name>=after and (p/'result.json').exists())
 if not choices:raise RuntimeError('missing receipt '+suffix)
 return choices[-1]/'result.json'
summary={'status':'PARTIAL','head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
 'initial_canonical_sha256':json.loads((BASE/'initial/source.json').read_text())['canonical_sha256'],
 'validation_run':str(RUN.relative_to(BASE)), 'native_matrix':{str(c):{} for c in (1,2,4,12)},
 'forced_native_races':{},'source_compatibility':{},'scope':'Bounded native/Linux thread construction and cleanup; named native creation/lifecycle races remain unproven.'}
for suite in ('admission','t1','auth','linux','fp','std'):
 suite_path=latest('suite-'+suite,RUNSTAMP)
 suite_info=json.loads(suite_path.read_text())
 assert suite_info['pass'],str(suite_path)
 for cpu in (1,2,4,12):
  p=latest(suite+f'-{cpu}cpu',suite_path.parent.name[:23])
  r=json.loads(p.read_text());raw=(p.parent/'serial.log').read_text()
  assert r['pass'] and assess(raw,r['markers'],cpu),str(p)
  summary['native_matrix'][str(cpu)][suite]=row(p)
  if suite=='admission':
   recovery=re.findall(r'^\[T2\] recovery cycle=(\d+) heap=(\d+) drift=0 unreaped_workers=0$',raw,re.M)
   pressure=re.findall(r'^\[T2\] pressure-heap cycle=(\d+) allocated=(\d+) free=(\d+)$',raw,re.M)
   assert len(recovery)==3 and len({n for _,n in recovery})==1
   assert all(int(a)+int(f)==16*1024*1024 for _,a,f in pressure)
   summary['native_matrix'][str(cpu)][suite]['memory']={'recovery_heap_bytes':[int(n) for _,n in recovery], 'pressure_allocated_bytes':[int(a) for _,a,_ in pressure],'pressure_free_bytes':[int(f) for _,_,f in pressure]}
   summary['native_matrix'][str(cpu)][suite]['memory']['live_worker_delta_bytes']=[int(a)-int(recovery[0][1]) for _,a,_ in pressure]
   assert all(int(a)-int(recovery[0][1])==96*(32768+160+16+16) for _,a,_ in pressure)
for suite in ('pin-r2d','doorbell'):
 p=RUN/('final-'+suite)/'results.json';d=json.loads(p.read_text())
 rows=d if isinstance(d,list) else d.get('results',d.get('runs',[]))
 if not rows:raise RuntimeError('unrecognized external gate '+str(p))
 for r in rows:
  cpu=str(r.get('cpus',r.get('cpu')))
  if cpu not in summary['native_matrix']:continue
  classification=r.get('classification',r.get('validation',{}).get('classification'))
  passed=r.get('pass',r.get('passed',classification=='FORCED_AND_PASSED'))
  if not passed and r.get('exit_code')==33 and not r.get('errors'):passed=True
  summary['native_matrix'][cpu][suite]={'pass':bool(passed),'result':str(p.relative_to(BASE)), 'classification':classification,'details':r}
for key,suffix in {
 'production_release_build':'production-release-build','production_release_boot':'production-2cpu',
 'production_debug_build':'production-debug-restored','baremetal_check':'baremetal-final',
 'allocation_host':'allocation-host-property','admission_host':'host-rr-reservations',
 'descriptor_host':'descriptor-final','parser_controls':'final-parsers',
 'kernel_host':'host-final','scheduler_host':'scheduler-host-final',
 'native_thread_api_host':'native-thread-api-final','native_mapping_api_host':'native-mapping-api-final',
 'linux_host':'linux-host','format_t2':'t2-format-final-source','format_workspace':'workspace-format-final'}.items():
  summary[key]=row(latest(suffix))
summary['clippy']={'strict_cargo':row(latest('clippy-strict')),
 'direct_actual_kernel':row(latest('clippy-final-source')),
 'classification':json.loads((BASE/'release-direct-clippy-detail/classification.json').read_text())}
summary['bore_admission_host']=row(latest('host-bore-reservations'))
summary['ordinary_debug_abi']=row(latest('ordinary-debug-abi'))
summary['production_exclusion']=json.loads((BASE/'production-exclusion.json').read_text())
summary['allocation_inventory']='allocation-inventory.json'
summary['steady_worker_bytes']={'stack':32768,'environment':160,'capabilities':16,'cwd':16,'sum':32960,'fd_table_slot_cache_bytes':10240,'process_table_bytes':512*3152,'rr_queue_bytes':64*512*8}
summary['admission_limits']={'per_vm_workers':32,'global_workers':96,'task_slots':512,'inherited_bytes':16384,'env_entries':128,'capabilities':256,'heap_headroom_bytes':2097152}
summary['named_race_coverage']={'host_contention':'production reservation/publication functions, modeled hardware/context and lifecycle mutations; not native proof','missing':list(__import__('threading_t2_acceptance').RACES)}
summary['attempts']=[dict(attempt=p.parent.name,**json.loads(p.read_text())) for p in sorted(BASE.glob('*/result.json')) if p.parent.name[:8].isdigit()]
final_identity=identity();summary['canonical_source_sha256']=final_identity['canonical_sha256']
# Implementation files must match the validation run; only debug fixture source
# can differ before its final dedicated admission matrix, whose own receipt is checked.
current=final_identity['files']
for suite in ('admission','t1','auth','linux','fp','std'):
 p=latest('suite-'+suite,RUNSTAMP);old=json.loads((p.parent/'source.json').read_text())['files']
 paths=[name for name in current if name.startswith('kernel/') and name.endswith('.rs')]
 differences=[name for name in paths if current[name]!=old.get(name)]
 summary['source_compatibility'][suite]={'kernel_rs_differences':differences}
 if differences:raise RuntimeError(f'compiled kernel source drift for {suite}: {differences}')
summary['acceptance']=acceptance(summary);summary['status']=summary['acceptance']['status']
(BASE/'validation-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
(BASE/'final-source.json').write_text(json.dumps(final_identity,indent=2)+'\n')
print(json.dumps({'status':summary['status'],'canonical_sha256':summary['canonical_source_sha256'],'missing':summary['acceptance']['missing']},indent=2))
