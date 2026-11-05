import time,subprocess,json,hashlib
from pathlib import Path
base=Path('.threading-evidence/t3r')
while True:
 p=base/'finish-results.json'
 rows=json.loads(p.read_text()) if p.exists() else []
 if rows and rows[-1]['name']=='tlb-12cpu':break
 time.sleep(1)
if any(r['exit_code'] for r in rows): print('Finish contains failed attempts; audit retains them.',flush=True)
argv=['python3','tools/threading_t3r_validate.py','command','--timeout','600','fixture-clippy','--','python3',str(base/'clippy-capture.py'),str(base/'clippy')]
code=subprocess.run(argv).returncode
print('Clippy captured:',code,flush=True)
results=[]
production=sorted(base.glob('*-suite-production/kernel.elf'))[-1]
for path in (base/'ordinary-debug.elf',production,base/'release-feature.elf'):
 data=path.read_bytes()
 forbidden=[b'[T3R]',b'[T3R-FW]',b'T3R source',b't3r_fixture']
 record={'path':str(path),'sha256':hashlib.sha256(data).hexdigest(),'forbidden':{m.decode():m not in data for m in forbidden}}
 record['pass']=all(record['forbidden'].values());results.append(record)
(base/'production-exclusion.json').write_text(json.dumps({'images':results,'pass':all(r['pass'] for r in results)},indent=2)+'\n')
print('Production exclusion:',all(r['pass'] for r in results),flush=True)
