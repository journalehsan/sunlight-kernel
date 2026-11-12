import subprocess,pathlib,json,shlex
out=pathlib.Path('target/phase21'); results={}
commands=[('string-abi-final',['python3','tools/native-c/tests/run_cxx_string.py','--out',str(out/'string-abi-final')]),('output-abi-final',['python3','tools/native-c/tests/run_cxx_output.py','--out',str(out/'output-abi-final')])]
commands += [('accepted-'+str(c)+'cpu',['python3','tools/native-c/guest.py','--suite','--cpus',str(c),'--out',str(out/('accepted-'+str(c)+'cpu'))]) for c in (2,4)]
for name,cmd in commands:
 with (out/(name+'-driver.log')).open('w') as log:
  log.write('$ '+shlex.join(cmd)+'\n');log.flush()
  try: code=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,timeout=600).returncode
  except subprocess.TimeoutExpired: code='TIMEOUT'
 results[name]=code; (out/'final-validation.json').write_text(json.dumps(results,indent=2));print(name,code,flush=True)
