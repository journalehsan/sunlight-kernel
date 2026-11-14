import pathlib,subprocess,json,hashlib,tarfile
R=pathlib.Path.cwd(); E=pathlib.Path(__file__).resolve().parent
def git(*a):return subprocess.check_output(['git',*a],text=True)
def hashes(root):return {str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in root.rglob('*') if p.is_file() and not p.is_symlink() and '.git' not in p.parts and 'target' not in p.parts and '__pycache__' not in p.parts and E not in p.parents}
x={'revision':git('rev-parse','HEAD').strip(),'branch':git('branch','--show-current').strip(),'status':git('status','--porcelain=v1','-uall'),'files':{n:hashlib.sha256((R/n).read_bytes()).hexdigest() for n in git('ls-files','--cached','--others','--exclude-standard').splitlines() if (R/n).is_file() and not (R/n).is_symlink() and E not in (R/n).parents}}
(E/'source-baseline.json').write_text(json.dumps(x,indent=2))
S=pathlib.Path('/tmp/sunlight-py1-cpython/Python-3.13.0'); a=pathlib.Path('/tmp/sunlight-r1-Python-3.13.0.tar.xz'); changes=[]
with tarfile.open(a) as t:
 for m in t:
  if m.isfile():
   p=S/pathlib.Path(m.name).relative_to('Python-3.13.0')
   if not p.exists() or hashlib.sha256(p.read_bytes()).digest()!=hashlib.sha256(t.extractfile(m).read()).digest(): changes.append(str(p.relative_to(S)))
y={'archive':str(a),'sha256':hashlib.sha256(a.read_bytes()).hexdigest(),'upstream_differences':changes,'source_files':hashes(S),'build_python':subprocess.check_output(['/tmp/sunlight-r1-cpython/build-host/python','-c','import sys,json;print(json.dumps(dict(version=sys.version,version_info=list(sys.version_info))))'],text=True)}
(E/'source-identity.json').write_text(json.dumps(y,indent=2));print('baseline',len(x['files']),'upstream differences',changes)
