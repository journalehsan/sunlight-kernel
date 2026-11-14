import pathlib,subprocess,json,time,os
R=pathlib.Path(__file__).resolve().parents[5];O=pathlib.Path(os.environ.get('R2B5_OUTPUT',str(pathlib.Path(__file__).resolve().parent)));rows=[];cases=[]
def run(name,cmd):
 start=time.monotonic()
 with (O/(name+'.log')).open('w') as f:
  f.write(json.dumps(cmd)+'\n');f.flush();rc=subprocess.run(cmd,cwd=R,stdout=f,stderr=subprocess.STDOUT,timeout=900).returncode
 rows.append(dict(name=name,command=cmd,exit=rc,seconds=time.monotonic()-start));(O/'tls-executions.json').write_text(json.dumps(rows,indent=2)+'\n');print(name,rc,flush=True);return rc
for name in ['pthread-tsd-once','pthread-tsd-exit-race','pthread-once-recursive','strerror-threads']:
 elf=R/('target/r2b5-'+name+'.elf')
 if run(name+'-build',['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(R/('tools/native-c/tests/'+name+'.c')),'-o',str(elf)]):raise SystemExit(1)
 cases.append(dict(name=name,elf=str(elf),expected_exit=0))
(O/'tls-cases.json').write_text(json.dumps(cases,indent=2)+'\n')
for cpu in [1,2,4]:run('tls-cpu-'+str(cpu),['python3','tools/native-c/guest.py','--cases',str(O/'tls-cases.json'),'--cpus',str(cpu),'--timeout','180','--out',str(O/('tls-cpu-'+str(cpu)))])
