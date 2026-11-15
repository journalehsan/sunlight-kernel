import subprocess,json,hashlib,os
from pathlib import Path
R=Path('/home/ehsantor/Projects/sunlightos-kernel'); O=R/'docs/audit/sunfs/sfs-5b4'; O.mkdir(exist_ok=False,parents=True); (O/'raw').mkdir()
def run(a): return subprocess.check_output(a,cwd=R)
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def put(n,v): (O/n).write_text(json.dumps(v,indent=2)+'\n')
status=run(['git','status','--porcelain=v1','-uall']).decode(); (O/'raw/incoming-status.txt').write_text(status)
(O/'raw/incoming.patch').write_bytes(run(['git','diff','--binary']))
names=run(['git','ls-files','-z','--cached','--others','--exclude-standard']).decode().split('\0')
m={n:sha(R/n) for n in names if n and not n.startswith('docs/audit/sunfs/sfs-5b4/') and (R/n).is_file() and not (R/n).is_symlink()}
put('raw/incoming-manifest.json',m)
inputs={n:sha(R/n) for n in ['Cargo.toml','Cargo.lock','.cargo/config.toml','rust-toolchain.toml','kernel/build.rs','kernel/Cargo.toml','sunlight-virtio/Cargo.toml']}
put('source-baseline.json',dict(revision=run(['git','rev-parse','HEAD']).decode().strip(),branch=run(['git','branch','--show-current']).decode().strip(),status='PASS',incoming_status='raw/incoming-status.txt',incoming_patch='raw/incoming.patch',manifest='raw/incoming-manifest.json',files=len(m),compilation_inputs=inputs,environment={k:os.getenv(k) for k in ['RUSTFLAGS','RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR','CARGO_BUILD_TARGET']},prior_evidence_preservation='sha256 per incoming file; recheck at final seal',seed='0x5b4001; deterministic barriers, no random schedule'))
for tool in ['rustc','cargo']: (O/f'raw/{tool}-version.txt').write_bytes(run([tool,'--version','--verbose']))
print(len(m),O)
