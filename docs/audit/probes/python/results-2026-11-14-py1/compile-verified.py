import pathlib,json,os,subprocess,shutil
E=pathlib.Path(__file__).resolve().parent;spec=json.loads((E/'configure-static-manifest.json').read_text());B=pathlib.Path(spec['build_directory']);env=os.environ.copy();env.update(spec['environment_overrides'])
(E/'configure-verified-manifest.json').write_text(json.dumps(spec,indent=2))
with (E/'configure-verified.log').open('w') as f:p=subprocess.run(spec['command'],cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
for n in ['pyconfig.h','Makefile','config.log']:shutil.copy2(B/n,E/('configure-verified-'+n))
(E/'configure-verified-result.json').write_text(json.dumps({'exit':p.returncode,'build_directory':str(B)}));print('configure-verified',p.returncode,flush=True)
assert p.returncode==0
cmd=['make','-k','-j4','-W',str(B.parent/'Python-3.13.0/Include/Python.h'),'python']
with (E/'core-compile-verified.log').open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=B,stdout=f,stderr=subprocess.STDOUT)
(E/'core-compile-verified-result.json').write_text(json.dumps({'command':cmd,'exit':p.returncode}));print('compile-verified',p.returncode,flush=True)
with (E/'core-compile-verified-serial.log').open('w') as f:p=subprocess.run(['make','-j1','python'],cwd=B,stdout=f,stderr=subprocess.STDOUT)
(E/'core-compile-verified-serial-result.json').write_text(json.dumps({'command':['make','-j1','python'],'exit':p.returncode}))
