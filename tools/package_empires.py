#!/usr/bin/env python3
"""Build/validate a native SunApp staging tree, not an installer or loader.
The manifest uses the current native app convention; see bundle README.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tomllib
import wave
ROOT = Path(__file__).resolve().parents[1]
FLAGS = '-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--skip-build', action='store_true')
    args = p.parse_args()
    if not args.skip_build:
        import os
        subprocess.run(['cargo','build','-p','sunlight-empires','--features','native','--release'], cwd=ROOT,env=dict(os.environ,RUSTFLAGS=FLAGS),check=True)
    source=ROOT/'SunlightEmpires.sunapp'
    dest=ROOT/'target/empires-m3/package/SunlightEmpires.sunapp'
    if dest.exists(): shutil.rmtree(dest)
    shutil.copytree(source,dest)
    (dest/'Program').mkdir()
    executable=ROOT/'target/x86_64-unknown-none/release/sunlight-empires'
    assert executable.read_bytes().startswith(b'\x7fELF')
    shutil.copyfile(executable,dest/'Program/sunlight-empires')
    (dest/'Program/sunlight-empires').chmod(0o755)
    for name in ['content.rs','sound.rs']:
        shutil.copyfile(ROOT/'sunlight-empires/src'/name,dest/'Resources'/name)
    # Retain platform-supplied font notices alongside the static native ELF.
    notices=dest/'Licenses/fonts'; notices.mkdir(exist_ok=True)
    for path in sorted((ROOT/'sun-font').rglob('*')):
        if path.is_file() and any(word in path.name.lower() for word in ['license','copying','notice','ofl']):
            name=str(path.relative_to(ROOT/'sun-font')).replace('/','__')
            shutil.copyfile(path,notices/name)
    manifest=tomllib.loads((dest/'Manifest.toml').read_text())
    assert manifest['bundle']['format']==1 and manifest['runtime']['type']=='native'
    assert manifest['entry']['executable']=='/bin/sunlight-empires'
    assert manifest['application']['kind']=='windowed'
    assert (dest/manifest['application']['icon']).is_file()
    inventory=json.loads((dest/'Licenses/attribution.json').read_text())
    assert len(inventory['tracks'])==3
    for t in inventory['tracks']:
        assert t['composer'] and t['license']=='CC-BY-SA-3.0'
        for key in ['original','resource']:
            assert hashlib.sha256((dest/t[key]).read_bytes()).hexdigest()==t[key+'_sha256']
        with wave.open(str(dest/t['resource'])) as w:
            assert (w.getnchannels(),w.getsampwidth(),w.getframerate(),w.getnframes())==(2,2,48000,1440000)
    assert (dest/'Licenses/CC-BY-SA-3.0.txt').stat().st_size>15000
    files=[]
    for f in sorted(dest.rglob('*')):
        assert not f.is_symlink()
        if f.is_file():
            files.append(dict(path=f.relative_to(dest).as_posix(),bytes=f.stat().st_size,sha256=hashlib.sha256(f.read_bytes()).hexdigest()))
    report=dict(format_version=1,verdict='STAGING_ONLY',native_bundle_loader=False,installer=False,files=files)
    (dest.parent/'package-inventory.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'bundle':str(dest.relative_to(ROOT)),'files':len(files),'bytes':sum(f['bytes'] for f in files),'status':'STAGING_ONLY'}))
if __name__=='__main__': main()
