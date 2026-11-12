import subprocess,pathlib,shlex,json
out=pathlib.Path('target/phase21')
commands=[(p.stem,['python3',str(p),'--out',str(out/p.stem)]) for p in sorted(pathlib.Path('tools/native-c/tests').glob('run_*.py')) if p.stem not in ('run_cxx_string','run_cxx_output')]
commands += [('kernel',['cargo','check','-p','sunlight-kernel']),('overlay',['bash','tools/check_native_std.sh']),('workspace',['cargo','check','--workspace']),('alloc',['bash','tools/test-alloc-proof.sh']),('mem-string',['bash','tools/test-mem-string-proof.sh']),('foundation',['bash','tools/test_native_std_foundation.sh'])]
results={}
for name,cmd in commands:
    with (out/(name+'-driver.log')).open('w') as log:
        log.write('$ '+shlex.join(cmd)+'\n');log.flush()
        try: r=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,timeout=900); code=r.returncode
        except subprocess.TimeoutExpired: code='TIMEOUT'
    results[name]=code
    (out/'regressions.json').write_text(json.dumps(results,indent=2))
    print(name,code,flush=True)
