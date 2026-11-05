"""Compile preserved initial kernel sources with the captured real dependencies."""
from pathlib import Path
import hashlib
import json
import os
import shutil
import subprocess
ROOT=Path.cwd()
BASE=ROOT/'.threading-evidence/t4a/clippy'
TREE=BASE/'initial-tree-completion'
TREE.mkdir(exist_ok=False)
shutil.copytree(ROOT/'kernel',TREE/'kernel')
for path in ROOT.iterdir():
    if path.name!='kernel' and not path.name.startswith('.'):
        (TREE/path.name).symlink_to(path.resolve(),target_is_directory=path.is_dir())
initial=ROOT/'.threading-evidence/t4a/initial/source/kernel'
for path in initial.rglob('*'):
    if path.is_file():shutil.copy2(path,TREE/'kernel'/path.relative_to(initial))
hashes=json.loads((ROOT/'.threading-evidence/t4a/initial/sha256.json').read_text())
verified={name:hashlib.sha256((TREE/name).read_bytes()).hexdigest()==digest for name,digest in hashes.items() if name.startswith('kernel/')}
assert all(verified.values())
(BASE/'baseline-completion-hash-verification.json').write_text(json.dumps(verified,indent=2)+'\n')
record=json.loads((BASE/'current-completion/result.json').read_text())
argv=record['command'];env=os.environ.copy();env.update(record['environment'])
env['CARGO_MANIFEST_DIR']=str(TREE/'kernel');env['CARGO_MANIFEST_PATH']=str(TREE/'kernel/Cargo.toml')
argv[argv.index('kernel/src/main.rs')]=str(TREE/'kernel/src/main.rs')
argv[argv.index('--out-dir')+1]=str(BASE/'baseline-completion')
(BASE/'baseline-completion').mkdir()
with (BASE/'baseline-completion/clippy.jsonl').open('w') as log:
    code=subprocess.run(argv,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
(BASE/'baseline-completion/result.json').write_text(json.dumps({'command':argv,'environment':record['environment'],'exit_code':code},indent=2)+'\n')
print('Initial kernel source hashes verified:',len(verified),'Clippy exit:',code)
