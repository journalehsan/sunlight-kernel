import pathlib,subprocess,json
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;cases=[]
for n in ['condvar-timeout','string-guard','exit-controls']:
 cmd=['python3','tools/native-c/cc.py','-O2','-fno-builtin','-Wall','-Wextra','-Werror',str(E/(n+'.c')),'-o',str(R/('target/py1-formatted-'+n+'.elf'))]
 with (E/(n+'-formatted-build.log')).open('w') as f:rc=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT).returncode
 assert rc==0
 if n=='exit-controls':
  for status in [0,37,255]:cases.append(dict(name='owner-exit-'+str(status),elf=str(R/('target/py1-formatted-'+n+'.elf')),expected_exit=status,args=[str(status)]))
 else:cases.append(dict(name=n,elf=str(R/('target/py1-formatted-'+n+'.elf')),expected_exit=0))
(E/'supplemental-formatted-cases.json').write_text(json.dumps(cases,indent=2))
for cpu in [1,2,4]:
 n='supplemental-formatted-cpu-'+str(cpu);print('START',n,flush=True)
 with (E/(n+'.log')).open('w') as f:rc=subprocess.run(['python3','tools/native-c/guest.py','--cases',str(E/'supplemental-formatted-cases.json'),'--cpus',str(cpu),'--timeout','180','--out',str(E/n)],stdout=f,stderr=subprocess.STDOUT).returncode
 print('END',n,rc,flush=True)
