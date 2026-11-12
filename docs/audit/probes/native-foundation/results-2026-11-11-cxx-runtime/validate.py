import subprocess,json,pathlib,shlex,time
out=pathlib.Path('target/phase22'); results=[]
commands=[('final-2cpu',['python3','tools/native-c/guest.py','--suite','--cpus','2','--out',str(out/'final-2cpu')]),('final-4cpu',['python3','tools/native-c/guest.py','--suite','--cpus','4','--out',str(out/'final-4cpu')]),('output-abi',['python3','tools/native-c/tests/run_cxx_output.py','--out',str(out/'output-abi')]),('string-abi',['python3','tools/native-c/tests/run_cxx_string.py','--out',str(out/'string-abi')]),('discovery',['python3','tools/native-c/tests/run_cxx_exception_discovery.py','--out',str(out/'discovery')]),('historical',['python3','docs/audit/probes/run.py','--out',str(out/'historical')])]
for name in ['dirent','dlfcn','langinfo','locale','math','mman','poll','pthread','random','signal','socket','termios','time','unistd','wchar','stdio']:
 commands.append(('run_'+name,['python3','tools/native-c/tests/run_'+name+'.py','--out',str(out/('run_'+name))]))
commands += [('kernel',['cargo','check','-p','sunlight-kernel']),('overlay',['bash','tools/check_native_std.sh']),('workspace',['cargo','check','--workspace']),('foundation',['bash','tools/test_native_std_foundation.sh']),('mem-proof',['bash','tools/test-mem-string-proof.sh']),('alloc-proof',['bash','tools/test-alloc-proof.sh']),('alloc-base-build',['rustc','--test','tools/alloc-proof.rs','-o',str(out/'alloc-base')]),('alloc-base',[str(out/'alloc-base'),'--test-threads=1'])]
for name,cmd in commands:
 print(name,flush=True)
 with (out/(name+'-driver.log')).open('w') as f:
  f.write('$ '+shlex.join(cmd)+'\n');f.flush()
  try: code=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,timeout=1200).returncode
  except Exception as e: f.write(str(e));code=125
  f.write('\nexit='+str(code)+'\n')
 results.append(dict(name=name,command=cmd,exit=code));(out/'validation.json').write_text(json.dumps(results,indent=2))
