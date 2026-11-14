import pathlib,subprocess,json,time
E=pathlib.Path(__file__).resolve().parent;B=pathlib.Path('/tmp/sunlight-py1-cpython/cross')
with (E/'core-compile-01.log').open('w') as f:
 cmd=['make','-k','-j4','python'];f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,cwd=B,stdout=f,stderr=subprocess.STDOUT)
(E/'core-compile-01-result.json').write_text(json.dumps({'command':cmd,'exit':p.returncode}));print(p.returncode)
