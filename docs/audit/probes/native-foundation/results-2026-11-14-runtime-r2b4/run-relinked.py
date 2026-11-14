import pathlib,json,subprocess
E=pathlib.Path(__file__).resolve().parent;F=E/'final';R=E.parents[4];rows=[]
for cpu in [1,2,4]:
 cmd=['python3','tools/native-c/guest.py','--cases',str(F/'cases.json'),'--cpus',str(cpu),'--timeout','180','--out',str(F/('lifecycle-relinked-cpu-'+str(cpu)))]
 with (F/('lifecycle-relinked-cpu-'+str(cpu)+'.log')).open('w') as log:rc=subprocess.run(cmd,cwd=R,stdout=log,stderr=subprocess.STDOUT,timeout=900).returncode
 rows.append(dict(name='lifecycle-relinked-cpu-'+str(cpu),command=cmd,exit=rc));(F/'relinked-executions.json').write_text(json.dumps(rows,indent=2)+'\n');print(cpu,rc,flush=True)
