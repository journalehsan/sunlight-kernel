import subprocess,json,pathlib,shlex
out=pathlib.Path('target/phase22');results=[]
commands=[('runtime-2cpu',['python3','tools/native-c/guest.py','--suite','--cpus','2','--timeout','180','--out',str(out/'runtime-2cpu')]),('runtime-4cpu',['python3','tools/native-c/guest.py','--suite','--cpus','4','--timeout','180','--out',str(out/'runtime-4cpu')])]
for name in ['classes','output','string','exception_discovery']:
 commands.append(('runtime-'+name+'-abi',['python3','tools/native-c/tests/run_cxx_'+name+'.py','--out',str(out/('runtime-'+name+'-abi'))]))
for name,cmd in commands:
 print(name,flush=True)
 with (out/(name+'-driver.log')).open('w') as f:
  f.write('$ '+shlex.join(cmd)+'\n');f.flush()
  try: code=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,timeout=900).returncode
  except Exception as e: f.write(str(e));code=125
  f.write('\nexit='+str(code)+'\n')
 results.append(dict(name=name,command=cmd,exit=code));(out/'runtime-validation.json').write_text(json.dumps(results,indent=2))
