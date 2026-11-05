import difflib,hashlib,json,sys
from pathlib import Path
root=Path.cwd();base=root/'.threading-evidence/t2';initial=base/'initial/source'
identity=json.loads((base/'initial/source.json').read_text())['files']
folder=Path(sys.argv[1]);rows=[];cache={}
def classify(name,start,end):
 p=Path(name)
 if p.is_absolute():
  try:name=str(p.relative_to(root))
  except ValueError:return 'outside-repository'
 p=root/name
 if name not in identity:return 'new-file' if not name.startswith('target/') else 'generated-input'
 if name not in cache:
  if p.exists() and hashlib.sha256(p.read_bytes()).hexdigest()==identity[name]:cache[name]=None
  elif (initial/name).exists():
   a=(initial/name).read_text().splitlines();b=p.read_text().splitlines()
   cache[name]=[(j+1,k) for op,_,_,j,k in difflib.SequenceMatcher(a=a,b=b,autojunk=False).get_opcodes() if op=='equal']
  else:cache[name]=[]
 equal=cache[name]
 if equal is None:return 'unchanged-file'
 return 'unchanged-lines' if any(start>=j and end<=k for j,k in equal) else 'new-or-changed-lines'
for line in (folder/'clippy.jsonl').read_text().splitlines():
 try:d=json.loads(line)
 except ValueError:continue
 spans=[s for s in d.get('spans',[]) if s.get('is_primary')]
 if not spans:continue
 rows.append({'message':d['message'],'code':d.get('code'),'level':d['level'],
 'spans':[{'file':s['file_name'],'line':s['line_start'],'classification':classify(s['file_name'],s['line_start'],s['line_end'])} for s in spans]})
new=[r for r in rows if any(s['classification'] in ('new-file','new-or-changed-lines') for s in r['spans'])]
result={'diagnostics_with_primary_spans':len(rows),'new_or_changed_line_diagnostics':new,'existing_diagnostics':len(rows)-len(new),'rows':rows}
(folder/'classification.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'total':len(rows),'new':new},indent=2))
