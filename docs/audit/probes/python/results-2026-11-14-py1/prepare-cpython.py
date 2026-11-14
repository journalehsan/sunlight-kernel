import pathlib,hashlib,tarfile,subprocess,json,os,shutil
E=pathlib.Path(__file__).resolve().parent; W=pathlib.Path('/tmp/sunlight-py1-cpython'); W.mkdir(exist_ok=False)
a=pathlib.Path('/tmp/sunlight-r1-Python-3.13.0.tar.xz'); sha=hashlib.sha256(a.read_bytes()).hexdigest();assert sha=='086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d'
with tarfile.open(a) as t:t.extractall(W,filter='data')
S=W/'Python-3.13.0'; before={str(p.relative_to(S)):hashlib.sha256(p.read_bytes()).hexdigest() for p in S.rglob('*') if p.is_file()};(E/'cpython-pristine.json').write_text(json.dumps(before,indent=2))
patch=pathlib.Path('docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/cpython-platform-experimental.patch');shutil.copy2(patch,E/'cpython-platform.patch')
p=subprocess.run(['patch','-p1','-i',str((E/'cpython-platform.patch').resolve())],cwd=S,capture_output=True,text=True);(E/'platform-patch.log').write_text(p.stdout+p.stderr);assert p.returncode==0
spec=json.loads(pathlib.Path('docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2a/configure-environment.json').read_text());spec['command'][0]=str(S/'configure');spec['environment_overrides'].update(ac_cv_file__dev_ptmx='no',ac_cv_file__dev_ptc='no');B=W/'cross';B.mkdir();env=os.environ.copy();env.update(spec['environment_overrides']);(E/'configure-manifest.json').write_text(json.dumps(spec,indent=2))
with (E/'configure.log').open('w') as f:p=subprocess.run(spec['command'],cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
for n in ['pyconfig.h','Makefile','config.log']:shutil.copy2(B/n,E/('configure-'+n))
(E/'cpython-identity.json').write_text(json.dumps({'archive':str(a),'sha256':sha,'source':str(S),'build':str(B),'configure_exit':p.returncode,'build_python':'/tmp/sunlight-r1-cpython/build-host/python'},indent=2));print('configure',p.returncode)
