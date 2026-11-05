"""Direct Clippy of the actual native-std bin, with the preserved initial main."""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
ROOT=Path.cwd();BASE=ROOT/'.threading-evidence/t4a/clippy/users-second';BASE.mkdir()
argv=['bash','tools/build_native_std.sh','rustc','-p','sunlight-docserv','--bin','sunlight-docserv','--no-default-features','--features','native-hayro-smoke','--release','--offline','-vv']
os.utime(ROOT/'sunlight-docserv/src/main.rs',None)
with (BASE/'cargo.log').open('w') as log:
    code=subprocess.run(argv,stdout=log,stderr=subprocess.STDOUT).returncode
if code:raise SystemExit(code)
line=next(l for l in reversed((BASE/'cargo.log').read_text().splitlines()) if 'Running `' in l and '--crate-name sunlight_docserv --edition=' in l and 'src/main.rs' in l)
argv=shlex.split(line.split('Running `',1)[1].rsplit('`',1)[0]);env=os.environ.copy()
while '=' in argv[0]:
    name,value=argv.pop(0).split('=',1);env[name]=value
argv[0]=subprocess.check_output(['rustup','which','clippy-driver'],text=True).strip()
argv[1:1]=['--sysroot',str(ROOT/'target/native-std/sysroot')]
for i,arg in enumerate(argv):
    if arg.startswith('--emit='):argv[i]='--emit=metadata'
    if arg=='--out-dir':argv[i+1]=str(BASE/'current')
(BASE/'current').mkdir()
with (BASE/'current/clippy.jsonl').open('w') as log:code=subprocess.run(argv,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
(BASE/'current/result.json').write_text(json.dumps({'command':argv,'exit_code':code},indent=2)+'\n')
shutil.copytree(ROOT/'sunlight-docserv',BASE/'initial-docserv')
shutil.copy2(ROOT/'.threading-evidence/t4a/initial/source/sunlight-docserv/src/main.rs',BASE/'initial-docserv/src/main.rs')
idx=next(i for i,x in enumerate(argv) if x.endswith('sunlight-docserv/src/main.rs'))
argv[idx]=str(BASE/'initial-docserv/src/main.rs');argv[argv.index('--out-dir')+1]=str(BASE/'baseline')
env['CARGO_MANIFEST_DIR']=str(BASE/'initial-docserv');env['CARGO_MANIFEST_PATH']=str(BASE/'initial-docserv/Cargo.toml')
(BASE/'baseline').mkdir()
with (BASE/'baseline/clippy.jsonl').open('w') as log:code=subprocess.run(argv,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
(BASE/'baseline/result.json').write_text(json.dumps({'command':argv,'exit_code':code},indent=2)+'\n')
print('Native userspace Clippy results retained:',BASE)
