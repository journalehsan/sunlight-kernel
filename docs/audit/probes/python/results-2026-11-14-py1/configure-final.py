import pathlib,json,shutil,tarfile,subprocess,os,re
E=pathlib.Path(__file__).resolve().parent;W=pathlib.Path('/tmp/sunlight-py1-cpython');S=W/'Python-3.13.0';B=W/'cross-final';B.mkdir()
shutil.copy2(E/'cpython-platform.patch',E/'cpython-platform-initial.patch')
with tarfile.open('/tmp/sunlight-r1-Python-3.13.0.tar.xz') as t:
 for n in ['configure','config.sub']:(S/n).write_bytes(t.extractfile('Python-3.13.0/'+n).read())
shutil.copy2('docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2a/cpython-platform.patch',E/'cpython-platform.patch')
p=subprocess.run(['patch','-p1','-i',str(E/'cpython-platform.patch')],cwd=S,capture_output=True,text=True);(E/'platform-final-patch.log').write_text(p.stdout+p.stderr);assert p.returncode==0
spec=json.loads((E/'configure-manifest.json').read_text());spec['command'].append('--disable-test-modules');spec['build_directory']=str(B)
(E/'configure-final-manifest.json').write_text(json.dumps(spec,indent=2));env=os.environ.copy();env.update(spec['environment_overrides'])
# Keep upstream mandatory bootstrap modules, plus the Unicode database. Optional
# extensions are excluded explicitly; no mandatory module is stubbed out.
stdlib=(W/'cross/Modules/Setup.stdlib').read_text();mods=[]
for l in stdlib.splitlines():
 if not l or l.startswith(('#','*',' ')):continue
 n=l.split()[0]
 if n.isidentifier() and len(l.split())>1 and any(x.endswith('.c') for x in l.split()):mods.append(n)
mods += ['_ssl','_hashlib','_sqlite3','_ctypes','_multiprocessing','_tkinter','_testcapi','_testinternalcapi']
(B/'Modules').mkdir();setup='*static*\nunicodedata unicodedata.c\n*disabled*\n'+'\n'.join(sorted(set(mods)-{'unicodedata'}))+'\n';(B/'Modules/Setup.local').write_text(setup);(E/'Setup.local').write_text(setup)
with (E/'configure-final.log').open('w') as f:p=subprocess.run(spec['command'],cwd=B,env=env,stdout=f,stderr=subprocess.STDOUT)
for n in ['pyconfig.h','Makefile','config.log']:shutil.copy2(B/n,E/('configure-final-'+n))
(E/'configure-final-result.json').write_text(json.dumps({'exit':p.returncode,'build_directory':str(B)}));print('configure-final',p.returncode,flush=True)
