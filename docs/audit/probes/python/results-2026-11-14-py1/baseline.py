import pathlib,subprocess,hashlib,json,os
R=pathlib.Path.cwd(); E=pathlib.Path(__file__).resolve().parent
files=subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard']).split(b'\0')
def inventory():
 out={}
 for f in files:
  if not f:continue
  p=R/os.fsdecode(f)
  if p.is_symlink():out[os.fsdecode(f)]={'symlink':os.readlink(p)}
  elif p.is_file():out[os.fsdecode(f)]=hashlib.sha256(p.read_bytes()).hexdigest()
 return out
(E/'source-baseline.json').write_text(json.dumps(inventory(),indent=2))
for name,cmd,cwd in [('worktree',['git','status','--porcelain=v1'],R),('revision',['git','rev-parse','HEAD'],R),('branch',['git','branch','--show-current'],R),('locked-build',['bash','tools/build_native_c.sh'],R),('build-python',['/tmp/sunlight-r1-cpython/build-host/python','-VV'],R),('first-compile',['make','-j1','python'],pathlib.Path('/tmp/sunlight-r2a-cpython/cross-acceptance'))]:
 with (E/(name+'.log')).open('w') as f:
  f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=cwd,stdout=f,stderr=subprocess.STDOUT);f.write('\nexit='+str(p.returncode)+'\n')
 print(name,p.returncode,flush=True)
