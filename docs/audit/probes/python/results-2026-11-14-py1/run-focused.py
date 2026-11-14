import pathlib,json,subprocess,time
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent
cases=[{'name':'py1-condvar','elf':str(R/'target/py1-condvar.elf'),'expected_exit':0},{'name':'py1-strncpy','elf':str(R/'target/py1-libc.elf'),'expected_exit':0},{'name':'py1-immediate-exit','elf':str(R/'target/py1-exit.elf'),'expected_exit':37}]
(E/'focused-cases.json').write_text(json.dumps(cases,indent=2))
for cpu in [1,2,4]:
 cmd=['python3','tools/native-c/guest.py','--cases',str(E/'focused-cases.json'),'--cpus',str(cpu),'--timeout','180','--out',str(E/('focused-v2-cpu-'+str(cpu)))];print('START',cpu,flush=True)
 with (E/('focused-v2-cpu-'+str(cpu)+'.log')).open('w') as f:p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 print('END',cpu,p.returncode,flush=True)
