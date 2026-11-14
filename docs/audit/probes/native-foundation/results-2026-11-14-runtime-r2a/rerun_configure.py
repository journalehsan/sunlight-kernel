import os,pathlib,subprocess,json,shlex,sys
r=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent;label=sys.argv[1];spec=json.loads((e/'configure-environment.json').read_text());env=os.environ.copy();env.update(spec['environment_overrides']);env.update(ac_cv_file__dev_ptmx='no',ac_cv_file__dev_ptc='no')
spec['environment_overrides'].update(ac_cv_file__dev_ptmx='no',ac_cv_file__dev_ptc='no');spec['cache_justifications'].update(ac_cv_file__dev_ptmx='devices-guest: native fopen ENOENT',ac_cv_file__dev_ptc='devices-guest: native fopen ENOENT')
work=pathlib.Path('/tmp/sunlight-r2a-cpython')/label;work.mkdir(exist_ok=False);spec['build_directory']=str(work)
p=subprocess.run(spec['command'],cwd=work,env=env,capture_output=True,text=True)
(e/(label+'-environment.json')).write_text(json.dumps(spec,indent=2));(e/(label+'.log')).write_text('$ '+shlex.join(spec['command'])+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n');(e/(label+'-config.log')).write_bytes((work/'config.log').read_bytes())
for f in ['pyconfig.h','Makefile','config.status']:
 if (work/f).exists():(e/(label+'-'+f)).write_bytes((work/f).read_bytes())
(e/(label+'-result.json')).write_text(json.dumps(dict(exit=p.returncode,build_directory=str(work)),indent=2));print(p.stdout[-2000:]+p.stderr)
