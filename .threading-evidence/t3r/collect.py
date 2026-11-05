"""Reassess durable T3R evidence. Evidence files are never native receipts by assertion alone."""
import sys,json,hashlib,difflib,subprocess,shutil
from pathlib import Path
sys.path.insert(0,'tools')
from threading_t3_validate import identity,assess
from threading_t3r_receipt import assess_migration
root=Path.cwd();base=root/'.threading-evidence/t3r';now=identity()
initial=json.loads((base/'initial/source.json').read_text())
implementation=[f for f in now['files'] if f.startswith('kernel/') and initial['files'].get(f)!=now['files'][f]]
all_kernel=[f for f in now['files'] if f.startswith('kernel/')]
summary={'classification':'T3R PARTIAL — migration verified; nonzero-BSP hardware proof unavailable','implementation':'IMPLEMENTED (debug-only fixtures; production scheduling policy unchanged)','initial_source_exactly_t3':json.loads((base/'initial/t3-comparison.json').read_text())['canonical_equal'],'head':now['head'],'native_migration':[],'compatibility':[],'hardware_topology':[],'host_checks':[],'historical_failures_preserved':[]}
matrix=sorted(base.glob('*-migration/result.json'))[-1]
model=json.loads(matrix.read_text());assert model['pass']
frozen_source=json.loads((matrix.parent/'source.json').read_text())
assert all(frozen_source['files'][f]==now['files'][f] for f in all_kernel)
assert frozen_source['files']['tools/helios-probes/thread-migration-probe.S']==now['files']['tools/helios-probes/thread-migration-probe.S']
assert hashlib.sha256((matrix.parent/'kernel.elf').read_bytes()).hexdigest()==model['elf_sha256']
assert hashlib.sha256((matrix.parent/'boot.iso').read_bytes()).hexdigest()==model['iso_sha256']
summary['final_migration_matrix']=str(matrix)
summary['kernel_source_matches_final_migration']=True
for cpu in (2,4,12):
 p=sorted(base.glob(f'*-migration-{cpu}cpu/result.json'))[-1]
 d=json.loads(p.read_text());serial=(p.parent/'serial.log').read_text()
 assert d['pass'] and assess_migration(serial,cpu) and d['iso_sha256']==model['iso_sha256']
 assert d['serial_sha256']==hashlib.sha256(serial.encode()).hexdigest()
 summary['native_migration'].append({'cpus':cpu,'status':'PASS','result':str(p),'serial_sha256':d['serial_sha256'],'iso_sha256':d['iso_sha256'],'apic_ids':d['observed_apic_ids']})
for p in sorted(base.glob('*-suite-*/result.json')):
 d=json.loads(p.read_text());assert d['pass']
 src=json.loads((p.parent/'source.json').read_text())
 assert all(src['files'][f]==now['files'][f] for f in all_kernel)
 summary['compatibility'].append({'suite':d['suite'],'status':'PASS','cpus':list(map(int,d['guests'])),'result':str(p),'kernel_source_matches_final':True})
for name in ('remote-wake','handoff'):
 p=base/name/'results.json';d=json.loads(p.read_text())
 for r in d['results']:
  cpu=r['cpus'];serial=(base/name/f'{cpu}cpu.serial.log').read_text()
  marker=f'[{"R2V" if name=="handoff" else "F13NR"}] PASS cpus={cpu} '
  assert r['pass'] and assess(serial,[marker],cpu)
 summary[name]={'status':'PASS','cpus':[r['cpus'] for r in d['results']],'result':str(p)}
for p in base.glob('*-nonzero-bsp-attempt/result.json'):
 d=json.loads(p.read_text());assert d['classification']=='BLOCKED / NOT_RUN' and not d['hardware_proof']
 summary['hardware_topology'].append({'status':'BLOCKED / NOT_RUN','result':str(p),'functional_migration':d['functional_migration'],'reason':d['reason'],'firmware':d['firmware'],'cpu_identity':d['cpu_identity']})
summary['late_bsp']={'status':'BLOCKED / NOT_RUN','reason':'Observed firmware puts actual BSP APIC 0 in slot 0; no available tested configuration produced a later BSP.'}
for p in sorted(base.glob('*/result.json')):
 d=json.loads(p.read_text())
 if 'command' not in d:continue
 if not d.get('pass') and 'clippy' not in p.parent.name:
  summary['historical_failures_preserved'].append(str(p))
 if any(p.parent.name.endswith('-'+suffix) for suffix in ('topology-host','kernel-host','compat-host','parser-controls','handoff-host','requests-host','admission-host','scheduler-host')):
  summary['host_checks'].append({'status':'PASS' if d['pass'] else 'FAIL','result':str(p)})
summary['negative_controls']=json.loads((base/'negative-controls/result.json').read_text());assert summary['negative_controls']['pass']
summary['formatting']=json.loads((base/'formatting.json').read_text());assert summary['formatting']['changed_spans_pass']
c=json.loads((base/'clippy/classification.json').read_text());assert c['changed_span_pass'] and c['same_message_and_primary_file_multiset_as_t3']
summary['clippy']={k:v for k,v in c.items() if k!='records'}
summary['production_exclusion']=json.loads((base/'production-exclusion.json').read_text());assert summary['production_exclusion']['pass']
summary['tlb_12cpu']=[str(p) for p in base.glob('*-tlb-12cpu/result.json') if json.loads(p.read_text()).get('cpus')==12]
assert len(summary['tlb_12cpu'])==1 and json.loads(Path(summary['tlb_12cpu'][0]).read_text())['pass']
summary['not_run']=['Genuine nonzero BSP initialization/AP/IPI/NMI/TLB gate','BSP later in actual firmware enumeration','T2R deferred parent-exit, VM-teardown and exec races','T4–T7']
summary['retained_prior_evidence']=['.threading-evidence/t1','.threading-evidence/t2','.threading-evidence/t2r','.threading-evidence/t3']
(base/'validation-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
final=base/'final';final.mkdir(exist_ok=False)
(final/'source.json').write_text(json.dumps(now,indent=2)+'\n')
(final/'status.txt').write_bytes(subprocess.check_output(['git','status','--short']))
changed=[f for f in sorted(set(initial['files'])|set(now['files'])) if initial['files'].get(f)!=now['files'].get(f)]
(final/'delta.json').write_text(json.dumps(changed,indent=2)+'\n')
with (base/'implementation.patch').open('w') as out:
 for f in changed:
  old=base/'initial/source'/f;new=root/f
  assert new.exists(),f
  out.writelines(difflib.unified_diff(old.read_text().splitlines(True) if old.exists() else [],new.read_text().splitlines(True),fromfile='a/'+f,tofile='b/'+f))
  dest=final/'source'/f;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(new,dest)
print(summary['classification']);print('Final source',now['canonical_sha256']);print('T3R paths:',len(changed));print('Kernel/assembly identity: matches accepted native matrix')
