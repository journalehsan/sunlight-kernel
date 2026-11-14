import pathlib,subprocess,json,hashlib,time
R=pathlib.Path(__file__).resolve().parents[5];E=pathlib.Path(__file__).resolve().parent;rows=[]
for cpu in [1,4]:
 args=['python3','tools/native-c/guest.py','--cases',str(E/'ipc-cases.json'),'--cpus',str(cpu),'--timeout','150','--out',str(E/('ipc-cpu-'+str(cpu)))];t=time.monotonic()
 with (E/('ipc-cpu-'+str(cpu)+'.log')).open('w') as f:rc=subprocess.run(args,cwd=R,stdout=f,stderr=subprocess.STDOUT,timeout=1200).returncode
 rows.append(dict(cpu=cpu,command=args,exit=rc,seconds=time.monotonic()-t));(E/'ipc-executions.json').write_text(json.dumps(rows,indent=2)+'\n');print(cpu,rc,flush=True)
