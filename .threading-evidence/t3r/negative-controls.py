"""Mutated native transcripts are HOST PARSER CONTROLS, never guest evidence."""
import sys,json,hashlib
from pathlib import Path
sys.path.insert(0,'tools')
from threading_t3r_receipt import assess_migration
base=Path('.threading-evidence/t3r')
source=sorted(base.glob('*-migration-2cpu/serial.log'))[-1]
text=source.read_text();assert assess_migration(text,2)
def remove(pred):return '\n'.join(l for l in text.splitlines() if not pred(l))+'\n'
controls={
 'request_only':remove(lambda l:l.startswith('[T3R] execution')),
 'wrong_process_incarnation':text.replace('owner_inc=1','owner_inc=999',1),
 'wrong_thread_incarnation':text.replace('[T3R] execution tid=58113 inc=2','[T3R] execution tid=58113 inc=999',1),
 'identical_physical_cpu':text.replace('cpu=1 apic=1','cpu=1 apic=0'),
 'missing_destination_execution':remove(lambda l:l.startswith('[T3R] execution') and 'cpu=1' in l),
 'invalid_topology_mapping':text.replace('logical=1 apic=1 bsp=false','logical=1 apic=9 bsp=false'),
 'wrong_tss_owner':text.replace('rsp0=0xffffffff901e0520','rsp0=0xffffffff901ead20'),
 'missing_tls_validation':text.replace('tls=true ',''),
 'corrupt_fp_validation':text.replace('x87=true','x87=false'),
 'missing_retirement':remove(lambda l:l.startswith('[T3R] retire')),
 'incomplete_reaping':remove(lambda l:l.startswith('[T3R] reaped')),
 'abandoned_handoff_waiter':text.replace('handoff_waiters=0','handoff_waiters=1'),
}
# Derive address replacement from receipt, rather than depending on link layout.
import re
rsp=re.search(r'\[T3R\] execution .*?rsp0=(0x[0-9a-f]+)',text)[1]
controls['wrong_tss_owner']=text.replace('rsp0='+rsp,'rsp0=0xfffffffffffffff0')
results=[];out=base/'negative-controls';out.mkdir(exist_ok=False)
for name,mutated in controls.items():
 assert mutated!=text,name
 accepted=assess_migration(mutated,2)
 (out/(name+'.HOST_ONLY.txt')).write_text(mutated)
 results.append({'name':name,'classification':'HOST PARSER NEGATIVE CONTROL','accepted':accepted,'pass':not accepted,'sha256':hashlib.sha256(mutated.encode()).hexdigest()})
record={'native_input':str(source),'native_input_sha256':hashlib.sha256(text.encode()).hexdigest(),'native_positive_accepted':True,'controls':results,'pass':all(r['pass'] for r in results)}
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(record,indent=2));raise SystemExit(0 if record['pass'] else 1)
