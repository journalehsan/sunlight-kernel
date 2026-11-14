import pathlib,subprocess,json,hashlib,time
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;P=E.parent/'results-2026-11-14-py1';Q=E.parent/'results-2026-11-14-py2';rows=json.loads((E/'validated-executions.json').read_text())
hashes=json.loads((E/'validated-source.json').read_text())
def run(n,cmd):
 print('START',n,flush=True);t=time.monotonic()
 with (E/(n+'.log')).open('w') as f:f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 rows.append(dict(name=n,command=cmd,exit=p.returncode,seconds=time.monotonic()-t,source_matches=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())));(E/'validated-executions.json').write_text(json.dumps(rows,indent=2));print('END',n,p.returncode,flush=True);return p.returncode
for cpu in [2,4]:
 n='shell-cpu-'+str(cpu)
 run(n,['python3','tools/native-c/guest.py','--cases',str(E/'shell-cases.json'),'--cpus',str(cpu),'--timeout','90','--out',str(E/n)])
