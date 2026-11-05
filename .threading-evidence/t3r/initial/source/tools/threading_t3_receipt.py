#!/usr/bin/env python3
"""Summarize retained T3 evidence, independently checking hardware observations."""
import hashlib,json,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
folder=ROOT/'.threading-evidence/t3'
from threading_t3_validate import assess, hardware_profile
implementation=json.loads((folder/'final-implementation-hashes.json').read_text())
result={'implementation_unchanged':all(hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==h for p,h in implementation.items()),'suites':{},'nonstandard':[],'tlb':[],'delivery':[],'affinity':[],'classification':'PARTIAL','remaining_native_proof':['nonzero BSP hardware identity','BSP later in firmware enumeration','independently forced per-thread migration trace']}
for suite in ('t1','admission','auth','linux','fp','std','production'):
    matches=sorted(folder.glob('*-suite-'+suite))
    if not matches: result['suites'][suite]={'status':'NOT_RUN'}; continue
    attempt=matches[-1]
    record=json.loads((attempt/'result.json').read_text()) if (attempt/'result.json').exists() else {}
    cells={}
    for guest in sorted(folder.iterdir()):
        if not (guest/'serial.log').exists() or not (guest/'result.json').exists():continue
        data=json.loads((guest/'result.json').read_text())
        argv=data.get('command',[])
        if data.get('topology','standard') != 'standard':continue
        if '-cdrom' not in argv or Path(argv[argv.index('-cdrom')+1]).resolve() != (attempt/'boot.iso').resolve():continue
        cpus=data['cpus']
        cells[str(cpus)]={'attempt':guest.name,'pass':bool(data.get('pass')) and assess((guest/'serial.log').read_text(errors='replace'),data['markers'],cpus)}
    passed=record.get('pass') and set(cells)=={'1','2','4','12'} and all(c['pass'] for c in cells.values())
    result['suites'][suite]={'path':str(attempt.relative_to(ROOT)),'record':record,'cells':cells,'status':'PASS' if passed else 'INCOMPLETE_OR_FAILED'}
for attempt in sorted(folder.iterdir()):
    if not (attempt/'result.json').exists() or not (attempt/'serial.log').exists():continue
    record=json.loads((attempt/'result.json').read_text())
    text=(attempt/'serial.log').read_text(errors='replace')
    if 'cpus' not in record: continue
    profile=record.get('topology',next((p for p in ('sparse','high','nonzero') if attempt.name.endswith('topology-'+p)),'standard'))
    observed=[int(a) for _,a in re.findall(r'^\[CPU-ID\] logical=(\d+) apic=(\d+) bsp=(?:true|false)$',text,re.M)]
    row={'attempt':attempt.name,'cpus':record['cpus'],'observed_apic_ids':observed,'profile':profile,
         'requested_hardware_observed':hardware_profile(profile,observed,record['cpus']),
         'functional_gate':assess(text,record['markers'],record['cpus'])}
    row['status']='NOT_RUN' if not row['requested_hardware_observed'] else 'PASS' if record.get('pass') and row['functional_gate'] else 'FAIL'
    if 'topology-' in attempt.name: result['nonstandard'].append(row)
    if any('-'+name+'-' in attempt.name for name in ('remote-wake','handoff','doorbell')) and profile != 'standard': result['delivery'].append(row)
    if '-affinity-' in attempt.name: result['affinity'].append(row)
    if '-tlb-' in attempt.name:
        row['counters']=re.findall(r'^\[MM-2B-DIAG\].*$',text,re.M)
        row['expected_logical_remote_mask']=hex(((1<<record['cpus'])-1)&~1)
        row['ack_set_evidence']='Gate completion checks each logical mailbox sequence; no individual ACK serial events are emitted by this existing gate.'
        result['tlb'].append(row)
for fixture in ('remote-wake','handoff','doorbell'):
    p=folder/fixture/'results.json'
    result[fixture+'_standard'] = json.loads(p.read_text()) if p.exists() else {'status':'NOT_RUN'}
(folder/'validation-summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k!='suites'},indent=2))
print({k:v['status'] for k,v in result['suites'].items()})
