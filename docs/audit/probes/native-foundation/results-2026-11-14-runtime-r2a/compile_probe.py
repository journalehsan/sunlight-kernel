import pathlib,subprocess,json,shlex
r=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent;s=pathlib.Path('/tmp/sunlight-r2a-cpython/Python-3.13.0')
probe=e/'compile-partial';probe.mkdir(exist_ok=False)
log=(e/'cpython-config.log').read_text();conf=log.rsplit('/* confdefs.h */',1)[1].split('configure: exit',1)[0]
# These are only definitions genuinely emitted before configure stopped.
(probe/'pyconfig.h').write_text('/* INCOMPLETE configure definitions: isolated diagnostics ONLY. */\n'+conf)
rows=[]
for unit in ['Python/pyctype.c','Python/pymath.c','Python/thread.c','Objects/longobject.c']:
 cmd=['python3','tools/native-c/cc.py','-std=c11','-O2','-DPy_BUILD_CORE','-I'+str(probe),'-I'+str(s/'Include'),'-I'+str(s/'Include/internal'),'-c',str(s/unit),'-o',str(probe/(pathlib.Path(unit).stem+'.o'))]
 p=subprocess.run(cmd,cwd=r,capture_output=True,text=True);name=pathlib.Path(unit).stem
 (probe/(name+'.log')).write_text('$ '+shlex.join(cmd)+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n')
 rows.append(dict(translation_unit=unit,exit=p.returncode,evidence=str((probe/(name+'.log')).relative_to(e)),configuration='INCOMPLETE actual configure confdefs, NOT host pyconfig; isolated compile only'))
(e/'compile-results.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows,indent=2))
