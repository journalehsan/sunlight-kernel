import pathlib,subprocess,json,os,shutil,re,concurrent.futures,shlex
E=pathlib.Path(__file__).resolve().parent;B=pathlib.Path('/tmp/sunlight-py4-final-cpython/cross-static');B.mkdir(parents=True,exist_ok=False)
P=E.parent/'results-2026-11-14-py2';spec=json.loads((P/'configure-manifest.json').read_text());env=os.environ.copy();env.update(spec['environment_overrides']);spec['build_directory']=str(B)
(B/'Modules').mkdir();shutil.copy2('/tmp/sunlight-py2-cpython/cross-static/Modules/Setup.local',B/'Modules/Setup.local');(E/'configure-manifest.json').write_text(json.dumps(spec,indent=2))
def run(name,cmd):
 with (E/(name+'.log')).open('w') as f:
  f.write(shlex.join(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
 return dict(command=cmd,cwd=str(B),exit=p.returncode,status='PASS' if p.returncode==0 else 'FAIL')
r=run('configure',spec['command']);(E/'configure-results.json').write_text(json.dumps(r,indent=2));assert r['exit']==0
for n in ['Makefile','config.log','pyconfig.h']:shutil.copy2(B/n,E/('configured-'+n))
# Get all invocations without allowing a failed prerequisite to hide getbuildinfo.
r=run('make-dry-run',['make','-n','python']);lines=(E/'make-dry-run.log').read_text().replace('\\\n',' ').splitlines();commands={};recipes={}
for l in lines:
 if 'tools/native-c/cc.py ' in l and '-c ' in l and '-o ' in l:
  a=shlex.split(l[l.index('python3 '):]);commands[a[a.index('-o')+1]]=a;recipes[a[a.index('-o')+1]]=l[l.index('python3 '):]
v={}
for l in (B/'Makefile').read_text().replace('\\\n',' ').splitlines():
 m=re.match(r'^(\w+)\s*=\s*(.*)',l)
 if m:v[m[1]]=m[2]
def expand(s,d=0):
 if d>40:raise ValueError('recursion')
 return re.sub(r'\$\((\w+)\)|\$\{(\w+)\}|\$(\w)',lambda m:expand(v.get(next(x for x in m.groups() if x),''),d+1),s)
expected=sorted(set(expand(v['LIBRARY_OBJS']).split()+['Programs/python.o']));(E/'object-manifest.json').write_text(json.dumps(expected,indent=2));(E/'compile-commands.json').write_text(json.dumps(commands,indent=2))
# Generate upstream config/header prerequisites through make, retaining all failures.
run('core-make',['make','-k','-j4','python'])
def one(o):
 a=commands.get(o)
 if not a:return dict(object=o,translation_unit=None,compiler_invocation=None,compile_result='NOT_RUN',first_failing_diagnostic=None,root_cause_classification='Unknown',required_api_header=[],ownership_layer='Unknown',planned_repair='Recover missing build command',final_outcome='NOT_RUN')
 (B/o).parent.mkdir(parents=True,exist_ok=True);p=subprocess.run(recipes[o] if o=='Modules/getbuildinfo.o' else a+['-ferror-limit=0'],shell=o=='Modules/getbuildinfo.o',cwd=B,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);log='compile-'+o.replace('/','_')+'.log';(E/log).write_text(shlex.join(a)+'\n'+p.stdout)
 err=next((l for l in p.stdout.splitlines() if 'error:' in l),None)
 apis=sorted(set(re.findall(r"(?:call to undeclared function|use of undeclared identifier|unknown type name) '([^']+)'",p.stdout)))
 headers=sorted(set(re.findall(r"fatal error: '([^']+)' file not found",p.stdout)))
 return dict(object=o,translation_unit=next(x for x in reversed(a) if x.endswith('.c')),compiler_invocation=recipes[o] if o=='Modules/getbuildinfo.o' else a,compile_result='PASS' if p.returncode==0 else 'FAIL',first_failing_diagnostic=err,root_cause_classification='Unknown' if err else None,required_api_header=apis+headers,ownership_layer='Pending source review' if err else None,planned_repair='See contract reports' if err else None,final_outcome='PASS' if p.returncode==0 else 'FAIL',log=log)
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:rows=list(pool.map(one,expected))
x=dict(configured=len(expected),attempted=sum(r['compile_result']!='NOT_RUN' for r in rows),passed=sum(r['compile_result']=='PASS' for r in rows),failed=sum(r['compile_result']=='FAIL' for r in rows),unattempted=sum(r['compile_result']=='NOT_RUN' for r in rows),units=rows)
(E/'object-inventory.json').write_text(json.dumps(x,indent=2));(E/'compile-results.json').write_text(json.dumps({k:v for k,v in x.items() if k!='units'},indent=2));print({k:v for k,v in x.items() if k!='units'},flush=True)
