from pathlib import Path
import datetime
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import time

ROOT = Path('/home/ehsantor/Projects/sunlightos-kernel')
OUT = ROOT / 'docs/audit/sunfs/sfs-5b4/raw'
RUN = OUT / ('embedded-recovery-' + datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
RUN.mkdir()
os.sched_setaffinity(0, {min(os.sched_getaffinity(0))})
environment = dict(os.environ, CARGO_BUILD_JOBS='1', CARGO_NET_OFFLINE='true')
service_environment = dict(environment, CARGO_TARGET_DIR=str(ROOT / 'target/embedded-build'), RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static')
service_environment.pop('CARGO_ENCODED_RUSTFLAGS', None)
rows = []


def command(name, arguments, env):
    start = time.monotonic()
    with (RUN / (name + '.log')).open('x') as log:
        result = subprocess.run(arguments, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    rows.append(dict(name=name, arguments=arguments, returncode=result.returncode, seconds=time.monotonic()-start,
                     environment={key:env.get(key) for key in ['CARGO_BUILD_JOBS','CARGO_NET_OFFLINE','CARGO_TARGET_DIR','RUSTFLAGS']},
                     cpu_affinity=list(os.sched_getaffinity(0))))
    (RUN / 'commands.json').write_text(json.dumps(rows, indent=2)+'\n')
    print(name, result.returncode, flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)


packages = [
    ('pty_server', ['pty_server']),
    ('sunlight-resolved', ['resolved','resolvectl']),
    ('sunlight-audiod', ['audiod','audioctl']),
    ('sunlight-niced', ['nicectl']),
    ('sunlight-uac', ['capabilityctl','runas']),
    ('sunlight-display', ['sunlight-display']),
    ('eyes', ['eyes']),
    ('sunlight-silicon-echoes', ['silicon-echoes']),
    ('sunlight-edit', ['sunlight-edit']),
    ('sunlight-writer', ['sunlight-writer']),
    ('sunlight-clipd', ['sunlight-clipd','sunlight-clip']),
    ('wiseowl-memorydb', ['wiseowl-memorydb','wiseowl-memorydbctl']),
    ('wiseowl-index', ['wiseowl-indexd','wiseowl-indexctl']),
    ('wiseowl-console', ['wiseowl']),
    ('sunlight-clipman', ['sunlight-clipman']),
    ('sunlight-emoji-picker', ['sunlight-emoji-picker']),
]
for package, binaries in packages:
    arguments = ['cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none','-p',package]
    if package in ['wiseowl-memorydb','wiseowl-index']:
        arguments += ['--features','sunlightos','--no-default-features']
    for binary in binaries:
        arguments += ['--bin',binary]
    command(package, arguments, service_environment)
    for binary in binaries:
        shutil.copyfile(ROOT/'target/embedded-build/x86_64-unknown-none/release'/binary, ROOT/'target/x86_64-unknown-none/release'/binary)
note_environment = dict(environment, RUSTFLAGS='-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie')
note_environment.pop('CARGO_ENCODED_RUSTFLAGS', None)
command('helios-note', ['cargo','build','--offline','--locked','--release','--target','x86_64-unknown-linux-musl','-p','helios-note'], note_environment)
command('helios-note-stamp', ['bash','tools/stamp_helios_elf.sh',str(ROOT/'target/x86_64-unknown-linux-musl/release/helios-note')], environment)
command('diagnostic-kernel', ['cargo','build','--offline','--locked','-p','sunlight-kernel','--features','sfs5b4-test'], environment)
kernel = ROOT/'target/x86_64-unknown-none/debug/sunlight-kernel'
with kernel.open('rb') as source, gzip.GzipFile(filename=str(RUN/'recovered-guest.elf.gz'), mode='wb', mtime=0) as destination:
    shutil.copyfileobj(source, destination)
(OUT/'recovered-guest-identity.json').write_text(json.dumps(dict(sha256=hashlib.sha256(kernel.read_bytes()).hexdigest(),compressed_copy=str((RUN/'recovered-guest.elf.gz').relative_to(OUT)),build_load='one compiler job on one host CPU; native runner not affinity restricted'),indent=2)+'\n')
