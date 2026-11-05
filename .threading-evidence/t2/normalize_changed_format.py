"""Apply whole-rustfmt hunks intersecting T2 edits; retain baseline-only style.
Both old/new files must have byte-identical complete rustfmt output, proving
this edit is formatting only (including string literals and inline assembly).
"""
import ast,difflib,hashlib,json,subprocess
from pathlib import Path
root=Path.cwd();base=root/'.threading-evidence/t2';initial=base/'initial/source'
module=ast.parse((base/'format_changed.py').read_text())
files=next(ast.literal_eval(node.value) for node in module.body if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='files' for t in node.targets))
folder=base/'format-normalization';folder.mkdir(exist_ok=False)
def formatted(path):
 result=subprocess.check_output(['rustfmt','--edition','2021','--config','skip_children=true','--emit','stdout',str(path)],text=True)
 prefix=str(path)+':\n\n'
 return result[len(prefix):] if result.startswith(prefix) else result
record={}
for name in files:
 path=root/name;before=path.read_text();target=formatted(path)
 a=before.splitlines(keepends=True);b=target.splitlines(keepends=True)
 if (initial/name).exists():
  old=(initial/name).read_text().splitlines(keepends=True)
  edited=[(max(0,j-2),k+2) for op,_,_,j,k in difflib.SequenceMatcher(a=old,b=a,autojunk=False).get_opcodes() if op!='equal']
 else: edited=[(0,len(a))]
 output=[]
 for op,j,k,l,m in difflib.SequenceMatcher(a=a,b=b,autojunk=False).get_opcodes():
  use=op!='equal' and any(j<=end and k>=start for start,end in edited)
  output.extend(b[l:m] if use else a[j:k])
 after=''.join(output)
 if after!=before:
  saved=folder/'source'/name;saved.parent.mkdir(parents=True,exist_ok=True);saved.write_text(before)
  path.write_text(after)
  assert formatted(path)==target,name
  record[name]={'before':hashlib.sha256(before.encode()).hexdigest(),'after':hashlib.sha256(after.encode()).hexdigest(),'complete_rustfmt_output_sha256':hashlib.sha256(target.encode()).hexdigest()}
(folder/'equivalence.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({'format_only_files':list(record)},indent=2))
