import pathlib,subprocess,json,hashlib,os,shutil
R=pathlib.Path.cwd(); E=R/'docs/audit/probes/python/results-2026-11-14-py2'; P=E.parent/'results-2026-11-14-py1'
def run(name,cmd,cwd=R,env=None):
 with (E/(name+'.log')).open('w') as f:
  f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=cwd,env=env,stdout=f,stderr=subprocess.STDOUT)
 (E/(name+'-result.json')).write_text(json.dumps({'command':cmd,'cwd':str(cwd),'exit':p.returncode},indent=2));return p.returncode
files=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z']).decode().split('\0'); hashes={}
for n in files:
 p=R/n
 if p.is_file() and not n.startswith(str(E.relative_to(R))):hashes[n]=hashlib.sha256(p.read_bytes()).hexdigest()
(E/'source-baseline.json').write_text(json.dumps({'revision':subprocess.check_output(['git','rev-parse','HEAD']).decode().strip(),'branch':subprocess.check_output(['git','branch','--show-current']).decode().strip(),'status':subprocess.check_output(['git','status','--porcelain=v1','--untracked-files=all']).decode(),'files':hashes},indent=2))
run('locked-build',['bash','tools/build_native_std.sh','build','--manifest-path','tools/native-c/Cargo.toml','--target-dir',str(R/'target/native-c'),'--release','--locked'])
run('build-python',['/tmp/sunlight-r1-cpython/build-host/python','-VV'])
spec=json.loads((P/'configure-verified-manifest.json').read_text());B=pathlib.Path('/tmp/sunlight-py2-cpython/cross-static');B.mkdir(parents=True,exist_ok=True);shutil.copy2(P/'Setup.local',B/'Modules.Setup.local') if False else None
(B/'Modules').mkdir(exist_ok=True);shutil.copy2(P/'Setup.local',B/'Modules/Setup.local');spec['build_directory']=str(B);env=os.environ.copy();env.update(spec['environment_overrides']);(E/'configure-manifest.json').write_text(json.dumps(spec,indent=2))
assert run('configure',spec['command'],B,env)==0
shutil.copy2(B/'Makefile',E/'configure-Makefile');shutil.copy2(B/'pyconfig.h',E/'configure-pyconfig.h')
run('core-before',['make','-k','-j4','python'],B)
run('core-before-serial',['make','-j1','python'],B)
