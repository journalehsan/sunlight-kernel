import subprocess,json,pathlib,shlex,time
out=pathlib.Path('target/phase22'); results=[]
commands=[('accepted-2cpu',['python3','tools/native-c/guest.py','--suite','--timeout','180','--cpus','2','--out',str(out/'accepted-2cpu')]),('accepted-4cpu',['python3','tools/native-c/guest.py','--suite','--timeout','180','--cpus','4','--out',str(out/'accepted-4cpu')]),('accepted-output-abi',['python3','tools/native-c/tests/run_cxx_output.py','--out',str(out/'accepted-output-abi')]),('accepted-string-abi',['python3','tools/native-c/tests/run_cxx_string.py','--out',str(out/'accepted-string-abi')]),('accepted-discovery',['python3','tools/native-c/tests/run_cxx_exception_discovery.py','--out',str(out/'accepted-discovery')]),('accepted-historical',['python3','docs/audit/probes/run.py','--out','docs/audit/probes/native-foundation/results-2026-11-11-cxx-runtime/accepted-historical'])]
for name in ['dirent','dlfcn','langinfo','locale','math','mman','poll','pthread','random','signal','socket','termios','time','unistd','wchar','stdio']:
 commands.append(('accepted-run_'+name,['python3','tools/native-c/tests/run_'+name+'.py','--out',str(out/('accepted-run_'+name))]))
commands += [('accepted-kernel',['cargo','check','-p','sunlight-kernel']),('accepted-overlay',['bash','tools/check_native_std.sh']),('accepted-workspace',['cargo','check','--workspace']),('accepted-foundation',['bash','tools/test_native_std_foundation.sh']),('accepted-mem-proof',['bash','tools/test-mem-string-proof.sh']),('accepted-alloc-proof',['bash','tools/test-alloc-proof.sh']),('accepted-alloc-base-build',['rustc','--test','tools/alloc-proof.rs','-o',str(out/'alloc-base')]),('accepted-alloc-base',[str(out/'alloc-base'),'--test-threads=1'])]
commands += [('accepted-class-abi',['python3','tools/native-c/tests/run_cxx_classes.py','--out',str(out/'accepted-class-abi')])]
for name,cmd in commands:
 print(name,flush=True)
 with (out/(name+'-driver.log')).open('w') as f:
  f.write('$ '+shlex.join(cmd)+'\n');f.flush()
  try: code=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,timeout=1200).returncode
  except Exception as e: f.write(str(e));code=125
  f.write('\nexit='+str(code)+'\n')
 results.append(dict(name=name,command=cmd,exit=code));(out/'accepted-validation.json').write_text(json.dumps(results,indent=2))
