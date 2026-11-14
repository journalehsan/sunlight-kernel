import pathlib,json,os,subprocess,shutil
E=pathlib.Path(__file__).resolve().parent;spec=json.loads((E/'configure-static-manifest.json').read_text());B=pathlib.Path(spec['build_directory']);env=os.environ.copy();env.update(spec['environment_overrides'])
(E/'configure-acceptance-manifest.json').write_text(json.dumps(spec,indent=2))
with (E/'configure-acceptance.log').open('w') as f:p=subprocess.run(spec['command'],cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
for n in ['pyconfig.h','Makefile','config.log']:shutil.copy2(B/n,E/('configure-acceptance-'+n))
(E/'configure-acceptance-result.json').write_text(json.dumps({'exit':p.returncode,'build_directory':str(B)}));print('configure-acceptance',p.returncode,flush=True)
assert p.returncode==0
cmd=['make','-k','-j4','-W',str(B.parent/'Python-3.13.0/Include/Python.h'),'python']
with (E/'core-compile-acceptance.log').open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=B,stdout=f,stderr=subprocess.STDOUT)
(E/'core-compile-acceptance-result.json').write_text(json.dumps({'command':cmd,'exit':p.returncode}));print('compile-acceptance',p.returncode,flush=True)
with (E/'core-compile-acceptance-serial.log').open('w') as f:p=subprocess.run(['make','-j1','python'],cwd=B,stdout=f,stderr=subprocess.STDOUT)
(E/'core-compile-acceptance-serial-result.json').write_text(json.dumps({'command':['make','-j1','python'],'exit':p.returncode}))
