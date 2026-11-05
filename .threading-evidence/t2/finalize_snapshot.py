"""Preserve a T2-only patch against the exact initial dirty tree, not HEAD."""
import difflib,hashlib,json,subprocess,tempfile
from pathlib import Path
base=Path('.threading-evidence/t2');initial=json.loads((base/'initial/source.json').read_text())['files'];final=json.loads((base/'final-source.json').read_text())
changed=sorted(name for name,h in final['files'].items() if initial.get(name)!=h)
missing=[name for name in changed if name in initial and not (base/'initial/source'/name).exists()]
for name in missing:
 with tempfile.TemporaryDirectory(prefix='t2-original-') as temporary:
  p=Path(temporary)/name;p.parent.mkdir(parents=True,exist_ok=True)
  p.write_bytes(subprocess.check_output(['git','show','HEAD:'+name]))
  subprocess.run(['git','-C',temporary,'apply','--include='+name],input=(base/'initial/dirty.patch').read_bytes(),check=True)
  assert hashlib.sha256(p.read_bytes()).hexdigest()==initial[name],name
  q=base/'initial/source'/name;q.parent.mkdir(parents=True,exist_ok=True);q.write_bytes(p.read_bytes())
(base/'snapshot-attempts.json').write_text(json.dumps({'preserved_failure':{'error':'initial archive lacked kernel/build.rs baseline','phase':'packaging only; no test outcome inferred'},'reconstructed_and_verified':missing},indent=2)+'\n')
root=base/'final';source=root/'source';source.mkdir(exist_ok=True)
assert not [name for name in initial if name not in final['files']]
patch=[]
for name in changed:
 data=Path(name).read_bytes();assert hashlib.sha256(data).hexdigest()==final['files'][name],name
 saved=source/name;saved.parent.mkdir(parents=True,exist_ok=True)
 if saved.exists():assert saved.read_bytes()==data,name
 else:saved.write_bytes(data)
 if name in initial:
  original=(base/'initial/source'/name).read_bytes();assert hashlib.sha256(original).hexdigest()==initial[name],name
 else:original=b''
 patch.extend(difflib.unified_diff(original.decode().splitlines(True),data.decode().splitlines(True),fromfile='a/'+name if name in initial else '/dev/null',tofile='b/'+name))
(root/'t2.patch').write_text(''.join(patch))
(root/'manifest.json').write_text(json.dumps({'head':final['head'],'canonical_sha256':final['canonical_sha256'],'files':{n:final['files'][n] for n in changed}},indent=2)+'\n')
with tempfile.TemporaryDirectory(prefix='t2-patch-check-') as temporary:
 for name in changed:
  if name in initial:
   p=Path(temporary)/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes((base/'initial/source'/name).read_bytes())
 result=subprocess.run(['git','-C',temporary,'apply','--check',str((root/'t2.patch').resolve())],text=True,capture_output=True)
 (base/'t2-patch-check.json').write_text(json.dumps({'command':result.args,'exit_code':result.returncode,'stdout':result.stdout,'stderr':result.stderr},indent=2)+'\n')
 assert result.returncode==0,result.stderr
print('T2-only snapshot:',len(changed),'files; patch applies to initial dirty source.')
