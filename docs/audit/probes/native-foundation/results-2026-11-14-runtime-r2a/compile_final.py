import pathlib,subprocess,json,shlex
r=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent;b=pathlib.Path('/tmp/sunlight-r2a-cpython/cross-acceptance');rows=[]
assert (b/'Makefile').exists(),'final configure still running'
for target in ['Programs/python.o','Python/pyctype.o','Python/pymath.o','Python/thread.o','Objects/longobject.o','python']:
 cmd=['make','-j1',target];p=subprocess.run(cmd,cwd=b,capture_output=True,text=True)
 name='acceptance-compile-'+target.replace('/','-').replace('.','-');(e/(name+'.log')).write_text('$ '+shlex.join(cmd)+'\ncwd='+str(b)+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n')
 rows.append(dict(target=target,exit=p.returncode,evidence=name+'.log',classification='POSIX threading gap' if 'pthread_key_t' in p.stderr else 'Unknown'))
(e/'acceptance-compile-results.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows,indent=2))
