import pathlib,subprocess,json,os,shutil,shlex,re,concurrent.futures
E=pathlib.Path(__file__).resolve().parent;B=pathlib.Path('/tmp/sunlight-py2-cpython/cross-static');spec=json.loads((E/'configure-manifest.json').read_text());env=os.environ.copy();env.update(spec['environment_overrides'])
def run(n,cmd):
 with (E/(n+'.log')).open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
 (E/(n+'-result.json')).write_text(json.dumps({'command':cmd,'cwd':str(B),'exit':p.returncode},indent=2));return p.returncode
assert run('configure-final',spec['command'])==0
for n in ['Makefile','config.log','pyconfig.h']:shutil.copy2(B/n,E/('configure-final-'+n))
run('core-final',['make','-k','-j4','-W','/tmp/sunlight-py1-cpython/Python-3.13.0/Include/Python.h','python'])
run('core-final-serial',['make','-j1','python'])
log=(E/'core-final.log').read_text().replace('\\\n',' ');units={}
for l in log.splitlines():
 if 'tools/native-c/cc.py ' in l and '-c ' in l and '-o ' in l:
  a=shlex.split(l[l.index('python3 '):]);o=a[a.index('-o')+1];units[o]={'object':o,'source':[x for x in a if x.endswith('.c')][-1],'command':a}
# Independently retain per-unit diagnostics; no parallel-log attribution guesses.
def one(u):
 p=subprocess.run(u['command']+['-ferror-limit=0'],cwd=B,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);(E/('final-'+u['object'].replace('/','_')+'.log')).write_text(p.stdout);u.update(attempted=True,status='PASS' if p.returncode==0 else 'FAIL',first_error=next((x for x in p.stdout.splitlines() if 'error:' in x),None),diagnostics=p.stdout,missing_declarations=sorted(set(re.findall(r"call to undeclared function '([^']+)'",p.stdout))),missing_headers=sorted(set(re.findall(r"fatal error: '([^']+)' file not found",p.stdout))),missing_types_or_constants=sorted(set(re.findall(r"(?:use of undeclared identifier|unknown type name) '([^']+)'",p.stdout))),mandatory=not u['source'].endswith('/unicodedata.c'));return u
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:rows=list(pool.map(one,units.values()))
# Extract this configuration's denominator, rather than assume the old set.
v={}
for l in (B/'Makefile').read_text().replace('\\\n',' ').splitlines():
 m=re.match(r'^(\w+)\s*=\s*(.*)',l)
 if m:v[m[1]]=m[2]
def expand(s,depth=0):
 if depth>30:return s
 return re.sub(r'\$\((\w+)\)|\$\{(\w+)\}|\$(\w)',lambda m:expand(v.get(next(x for x in m.groups() if x),''),depth+1),s)
expected=sorted(set(expand(v['LIBRARY_OBJS']).split()+['Programs/python.o']));(E/'core-final-object-manifest.txt').write_text('\n'.join(expected)+'\n')
for o in expected:
 if o not in units:rows.append(dict(object=o,source='Modules/getbuildinfo.c' if o=='Modules/getbuildinfo.o' else None,attempted=False,status='NOT_RUN',mandatory=True,reason='make prerequisites incomplete'))
result={'configured':len(expected),'before_configured':180,'attempted':len(units),'passed':sum(u['status']=='PASS' for u in rows),'failed':sum(u['status']=='FAIL' for u in rows),'unattempted':[u['object'] for u in rows if not u['attempted']],'first_serial_error':next((x for x in (E/'core-final-serial.log').read_text().splitlines() if 'error:' in x),None),'units':rows,'denominator_change_reason':None if len(expected)==180 else 'See before/final generated Makefiles and configure logs'}
(E/'cpython-compilation-matrix.json').write_text(json.dumps(result,indent=2));print({k:result[k] for k in result if k!='units'},flush=True)
