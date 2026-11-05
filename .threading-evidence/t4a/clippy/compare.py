from collections import Counter
import difflib
import json
from pathlib import Path
ROOT=Path.cwd();BASE=ROOT/'.threading-evidence/t4a/clippy'
def diagnostics(folder):
    rows=[]
    for line in (BASE/folder/'clippy.jsonl').read_text().splitlines():
        try:d=json.loads(line)
        except ValueError:continue
        if 'level' in d:
            primary=[s for s in d.get('spans',[]) if s.get('is_primary')]
            paths=[s['file_name'].split('/initial-tree/')[-1] for s in primary]
            rows.append((d['level'],d.get('code',{}).get('code') if d.get('code') else None,d['message'],tuple(paths)))
    return rows
old=Counter(diagnostics('baseline'));new=Counter(diagnostics('current'))
changed={}
for path in (ROOT/'.threading-evidence/t4a/initial/source/kernel').rglob('*.rs'):
    name=str(Path('kernel')/path.relative_to(ROOT/'.threading-evidence/t4a/initial/source/kernel'))
    spans=[]
    for op,_,_,j,k in difflib.SequenceMatcher(a=path.read_text().splitlines(),b=(ROOT/name).read_text().splitlines(),autojunk=False).get_opcodes():
        if op!='equal':spans.extend(range(j+1,max(j+1,k)+1))
    changed[name]=set(spans)
intersections=[]
for line in (BASE/'current/clippy.jsonl').read_text().splitlines():
    try:d=json.loads(line)
    except ValueError:continue
    for span in d.get('spans',[]):
        if not span.get('is_primary'):continue
        name=span['file_name']
        if name.endswith('t4a_fixture.rs') or set(range(span['line_start'],span['line_end']+1)) & changed.get(name,set()):
            intersections.append({'message':d['message'],'span':span})
result={'baseline_records':sum(old.values()),'current_records':sum(new.values()),'added':list((new-old).elements()),'removed':list((old-new).elements()),'changed_span_intersections':intersections}
(BASE/'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
