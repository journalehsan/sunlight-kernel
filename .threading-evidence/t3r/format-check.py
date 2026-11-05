import difflib,json,subprocess,tempfile
from pathlib import Path
root=Path.cwd();base=root/'.threading-evidence/t3r/initial/source'
import sys;sys.path.insert(0,str(root/'tools'));from threading_t3_validate import identity
old=json.loads((base.parent/'source.json').read_text());now=identity()
results=[];bad=False
for f,h in now['files'].items():
 if not f.startswith('kernel/') or not f.endswith('.rs') or old['files'].get(f)==h:continue
 p=root/f;s=p.read_text().splitlines();initial=(base/f).read_text().splitlines() if (base/f).exists() else []
 changed=set()
 for op,_,_,a,b in difflib.SequenceMatcher(a=initial,b=s,autojunk=False).get_opcodes():
  if op!='equal':changed.update(range(a,max(a+1,b)))
 with tempfile.TemporaryDirectory() as tmp:
  q=Path(tmp)/p.name;q.write_text(p.read_text());subprocess.run(['rustfmt','--edition','2021','--config','skip_children=true',str(q)],check=True)
  formatted=q.read_text().splitlines()
 discrepancies=[]
 for op,a,b,_,_ in difflib.SequenceMatcher(a=s,b=formatted,autojunk=False).get_opcodes():
  if op!='equal':discrepancies.append({'line':a+1,'changed':bool(set(range(a,max(a+1,b)))&changed)})
 results.append({'file':f,'discrepancies':discrepancies})
 bad |= any(d['changed'] for d in discrepancies)
(root/'.threading-evidence/t3r/formatting.json').write_text(json.dumps({'changed_spans_pass':not bad,'files':results},indent=2)+'\n')
print(json.dumps(results,indent=2));raise SystemExit(int(bad))
