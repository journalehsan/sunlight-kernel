"""Clippy the native runtime using its real target/features and baseline source."""
import collections
import difflib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
ROOT=Path.cwd();BASE=ROOT/'.threading-evidence/t4a/clippy/libc';BASE.mkdir()
log=ROOT/'.threading-evidence/t4a/clippy/users-third/cargo.log'
line=next(l for l in reversed(log.read_text().splitlines()) if 'Running `' in l and '--crate-name sunlight_libc ' in l and 'src/lib.rs' in l)
argv=shlex.split(line.split('Running `',1)[1].rsplit('`',1)[0]);env=os.environ.copy()
while '=' in argv[0]:
    n,v=argv.pop(0).split('=',1);env[n]=v
argv[0]=subprocess.check_output(['rustup','which','clippy-driver'],text=True).strip()
argv[1:1]=['--sysroot',str(ROOT/'target/native-std/sysroot')]
for i,a in enumerate(argv):
    if a.startswith('--emit='):argv[i]='--emit=metadata'
    if a=='--out-dir':argv[i+1]=str(BASE/'current')
(BASE/'current').mkdir()
with (BASE/'current/clippy.jsonl').open('w') as out:code=subprocess.run(argv,env=env,stdout=out,stderr=subprocess.STDOUT).returncode
(BASE/'current/result.json').write_text(json.dumps({'command':argv,'exit_code':code},indent=2)+'\n')
shutil.copytree(ROOT/'sunlight-libc',BASE/'initial-libc')
shutil.copy2(ROOT/'.threading-evidence/t4a/initial/source/sunlight-libc/src/std_runtime.rs',BASE/'initial-libc/src/std_runtime.rs')
idx=next(i for i,x in enumerate(argv) if x.endswith('sunlight-libc/src/lib.rs'));argv[idx]=str(BASE/'initial-libc/src/lib.rs')
argv[argv.index('--out-dir')+1]=str(BASE/'baseline');(BASE/'baseline').mkdir()
env['CARGO_MANIFEST_DIR']=str(BASE/'initial-libc');env['CARGO_MANIFEST_PATH']=str(BASE/'initial-libc/Cargo.toml')
with (BASE/'baseline/clippy.jsonl').open('w') as out:code=subprocess.run(argv,env=env,stdout=out,stderr=subprocess.STDOUT).returncode
(BASE/'baseline/result.json').write_text(json.dumps({'command':argv,'exit_code':code},indent=2)+'\n')
def rows(folder):
    result=[]
    for l in (BASE/folder/'clippy.jsonl').read_text().splitlines():
        try:d=json.loads(l)
        except ValueError:continue
        if 'level' in d:result.append((d['level'],d.get('code',{}).get('code') if d.get('code') else None,d['message']))
    return collections.Counter(result)
old,new=rows('baseline'),rows('current')
result={'baseline_exit':json.loads((BASE/'baseline/result.json').read_text())['exit_code'],'current_exit':json.loads((BASE/'current/result.json').read_text())['exit_code'],'baseline_records':sum(old.values()),'current_records':sum(new.values()),'added':list((new-old).elements()),'removed':list((old-new).elements())}
(BASE/'comparison.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
