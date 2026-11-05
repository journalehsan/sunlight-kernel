import json,difflib
from pathlib import Path
root=Path.cwd();base=root/'.threading-evidence/t3r';initial=json.loads((base/'initial/source.json').read_text())['files']
import sys;sys.path.insert(0,str(root/'tools'));from threading_t3_validate import identity
now=identity()['files'];changed={}
for f,h in now.items():
 if not f.endswith('.rs') or not f.startswith('kernel/') or initial.get(f)==h:continue
 old=base/'initial/source'/f;a=old.read_text().splitlines() if old.exists() else [];b=(root/f).read_text().splitlines()
 lines=set()
 for op,_,_,i,j in difflib.SequenceMatcher(a=a,b=b,autojunk=False).get_opcodes():
  if op!='equal':lines.update(range(i+1,max(i+1,j)+1))
 changed[f]=lines
records=[];new=[]
for line in (base/'clippy/clippy.jsonl').read_text().splitlines():
 try:d=json.loads(line)
 except json.JSONDecodeError:continue
 if d.get('$message_type')!='diagnostic' or d.get('level') not in ('error','warning'):continue
 primary=[{'file':s['file_name'],'line':s['line_start'],'changed':s['file_name'] in changed and bool(set(range(s['line_start'],s['line_end']+1))&changed[s['file_name']])} for s in d.get('spans',[]) if s.get('is_primary')]
 record={'level':d['level'],'message':d['message'],'code':d.get('code'),'primary':primary};records.append(record)
 if any(p['changed'] for p in primary):new.append(record)
report={'overall':'FAIL (existing unrelated diagnostics)','diagnostics':len(records),'changed_span_diagnostics':new,'changed_span_pass':not new,'records':records}
(base/'clippy/classification.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='records'},indent=2));raise SystemExit(bool(new))
