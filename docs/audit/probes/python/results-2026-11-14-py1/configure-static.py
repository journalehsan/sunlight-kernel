import pathlib,subprocess,os,json,shutil
E=pathlib.Path(__file__).resolve().parent;B=pathlib.Path('/tmp/sunlight-py1-cpython/cross-static');B.mkdir();(B/'Modules').mkdir();shutil.copy2(E/'Setup.local',B/'Modules/Setup.local')
spec=json.loads((E/'configure-final-manifest.json').read_text());spec['environment_overrides']['DYNLOADFILE']='dynload_stub.o';spec['build_directory']=str(B);spec['override_justifications']={'DYNLOADFILE':'Native dlopen supports only the fixed main-program namespace; shared-object relocation/loading unavailable. Select the upstream supported no-dynamic-loading adapter.'}
(E/'configure-static-manifest.json').write_text(json.dumps(spec,indent=2));env=os.environ.copy();env.update(spec['environment_overrides'])
with (E/'configure-static.log').open('w') as f:p=subprocess.run(spec['command'],cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
for n in ['pyconfig.h','Makefile','config.log']:shutil.copy2(B/n,E/('configure-static-'+n))
(E/'configure-static-result.json').write_text(json.dumps({'exit':p.returncode,'build_directory':str(B)}));print('configure-static',p.returncode,flush=True)
if p.returncode:raise SystemExit(p.returncode)
with (E/'core-compile-static.log').open('w') as f:
 cmd=['make','-k','-j4','python'];f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=B,stdout=f,stderr=subprocess.STDOUT)
(E/'core-compile-static-result.json').write_text(json.dumps({'command':cmd,'exit':p.returncode}));print('compile-static',p.returncode,flush=True)
