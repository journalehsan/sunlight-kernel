from pathlib import Path
import json,os,subprocess,shlex,hashlib
R=Path.cwd();E=Path(__file__).resolve().parent
spec=json.loads((E/'configure-environment.json').read_text());work=Path('/tmp/sunlight-r2b1-final-cpython-cross');work.mkdir(exist_ok=False);env=os.environ.copy();env.update(spec['environment_overrides']);spec['build_directory']=str(work)
p=subprocess.run(spec['command'],cwd=work,env=env,capture_output=True,text=True,timeout=240)
(E/'final-configure.log').write_text('$ '+shlex.join(spec['command'])+'\n'+p.stdout+p.stderr);(E/'final-configure-environment.json').write_text(json.dumps(spec,indent=2));(E/'final-configure-result.json').write_text(json.dumps(dict(exit=p.returncode,work=str(work))))
for f in ['config.log','pyconfig.h','Makefile','config.status']:
 if (work/f).exists():(E/('final-cpython-'+f)).write_bytes((work/f).read_bytes())
print('configure exit',p.returncode,flush=True)
if p.returncode:raise SystemExit(p.returncode)
results=[]
for target in ['Programs/python.o','Python/pyctype.o','Python/pymath.o','Python/thread.o','Objects/longobject.o','python']:
 p=subprocess.run(['make','-j1',target],cwd=work,env=env,capture_output=True,text=True,timeout=180)
 name=target.replace('/','-').replace('.','-');(E/('final-compile-'+name+'.log')).write_text(p.stdout+p.stderr);results.append(dict(target=target,exit=p.returncode,log='final-compile-'+name+'.log'))
 print(target,p.returncode,flush=True)
(E/'final-compile-results.json').write_text(json.dumps(results,indent=2))
# Retain actual emitted object evidence, with hashes and ELF headers, not host binaries.
objects={str(p.relative_to(work)):hashlib.sha256(p.read_bytes()).hexdigest() for p in work.rglob('*.o')}
(E/'final-cpython-object-hashes.json').write_text(json.dumps(objects,indent=2))
for obj in ['Programs/python.o','Python/pyctype.o','Python/pymath.o','Parser/token.o']:
 if (work/obj).exists():
  p=subprocess.run(['llvm-readelf','-h',work/obj],capture_output=True)
  (E/('final-cpython-'+obj.replace('/','-')+'.elf.log')).write_bytes(p.stdout+p.stderr)
