#!/usr/bin/env python3
"""Bounded SFS5B4 disposable native runner and final-source regression recorder.

Never touches an existing image, physical block device, or earlier audit output.
"""
import argparse
import datetime
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'docs/audit/sunfs/sfs-5b4'
QEMU = Path('/tmp/sfs5b4-qemu-build/qemu-system-x86_64')
SEED = 0x5b4001
MARKER = b'SFS5A-ONLY-7b93143a-disposable-v1!'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def put(p, value):
    p.write_text(json.dumps(value, indent=2)+'\n')


def sources():
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT).decode().split('\0')
    return {n: sha(ROOT/n) for n in names if n and (ROOT/n).is_file() and not (ROOT/n).is_symlink()
            and not n.startswith(('docs/audit/', 'docs/filesystems/'))}


def command(argv, raw, name, timeout=1800, env=None):
    start = time.monotonic_ns()
    with (raw/(name+'.log')).open('x') as log:
        try:
            code = subprocess.run(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=timeout, env=env).returncode
        except subprocess.TimeoutExpired:
            code = 'TIMEOUT'
    row = dict(argv=argv, returncode=code, start_ns=start, end_ns=time.monotonic_ns(), log=str((raw/(name+'.log')).relative_to(OUT)))
    with (raw/'commands.jsonl').open('a') as f:
        f.write(json.dumps(row)+'\n')
    print(name, code, flush=True)
    return row


def native(raw, cpus, modes, caches):
    build = command(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked', '--features', 'sfs5b4-test'], raw, 'guest-build')
    if build['returncode']:
        put(raw/'native-result.json', dict(status='BLOCKED', build=build))
        return
    artifacts = ROOT/'target/sfs5b4'/raw.name
    artifacts.mkdir(parents=True)
    elf = artifacts/'sunlight-kernel.elf'
    shutil.copyfile(ROOT/'target/x86_64-unknown-none/debug/sunlight-kernel', elf)
    iso = artifacts/'sfs5b4.iso'
    row = command(['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso)], raw, 'iso-build')
    if row['returncode'] or not QEMU.exists():
        put(raw/'native-result.json', dict(status='BLOCKED', iso=row, qemu_exists=QEMU.exists()))
        return
    rows = []
    frozen = sources()
    events = raw/'trace-events.conf'
    events.write_text('virtio_set_status\nvirtqueue_pop\nvirtqueue_fill\nvirtqueue_flush\nvirtio_blk_req_complete\nvirtio_blk_handle_write\n')
    for cpu in cpus:
        for cache in caches:
            for mode in modes:
                tag = f'cpu-{cpu}-{cache}-{mode}'
                image = artifacts/(tag+'.img')
                initial = bytearray(8192*512)
                initial[:len(MARKER)] = MARKER
                if mode == 'cancel':
                    initial[61*512] = 1
                with image.open('xb') as f:
                    f.write(initial)
                    f.flush()
                    os.fsync(f.fileno())
                serial, trace, device = [raw/(tag+suffix+'.log') for suffix in ['.serial', '.trace', '.qemu']]
                env = dict(os.environ)
                for k in list(env):
                    if k.startswith(('SFS5B3_', 'SFS5B4_')):
                        del env[k]
                executable = str(QEMU)
                if mode == 'unmodified':
                    executable = shutil.which('qemu-system-x86_64')
                else:
                    env['SFS5B3_MODE'] = 'hold' if mode == 'cancel' else mode
                if mode == 'release':
                    release = artifacts/(tag+'.release')
                    release.touch(exist_ok=False)
                    env['SFS5B3_RELEASE'] = str(release)
                argv = [executable, '-machine', 'q35', '-accel', 'tcg', '-m', '1024M', '-smp', str(cpu),
                        '-cdrom', str(iso), '-boot', 'd', '-display', 'none', '-serial', f'file:{serial}',
                        '-monitor', 'none', '-drive', f'id=sfs5b4,file={image},if=none,format=raw,cache={cache}',
                        '-device', 'virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=sfs5b4,serial=SFS5B3-disposable',
                        '-device', 'virtio-rng-pci,disable-modern=on', '-nic', 'none', '-no-reboot', '-no-shutdown',
                        '-trace', f'events={events},file={trace}']
                put(raw/(tag+'.invocation.json'), dict(argv=argv, seed=SEED, seed_use='deterministic bytes and named barriers, no random fault scheduling',
                    environment={k:v for k,v in env.items() if k.startswith('SFS5B')}, source=frozen,
                    elf_sha256=sha(elf), iso_sha256=sha(iso), qemu_sha256=sha(Path(executable)),
                    patch_sha256=sha(OUT/'qemu-async-access.patch'), initial_image_sha256=sha(image)))
                host_events = []
                ready = False
                with device.open('x') as log:
                    p = subprocess.Popen(argv, cwd=ROOT, env=env, stdout=log, stderr=log)
                    host_events.append(dict(event='PROCESS_STARTED', ns=time.monotonic_ns()))
                    try:
                        deadline = time.monotonic()+90
                        while p.poll() is None and time.monotonic() < deadline:
                            content = serial.read_text(errors='replace') if serial.exists() else ''
                            if '[SFS5B4] READY' in content:
                                ready = True
                                host_events.append(dict(event='READY_OBSERVED', ns=time.monotonic_ns()))
                                time.sleep(.25)
                                break
                            if 'panicked at' in content:
                                break
                            time.sleep(.05)
                    finally:
                        if p.poll() is None:
                            p.kill()
                        code = p.wait(timeout=10)
                        host_events.append(dict(event='PROCESS_REAPED', ns=time.monotonic_ns(), code=code))
                content = serial.read_text(errors='replace') if serial.exists() else ''
                dev = device.read_text(errors='replace')
                initial[62*512:63*512] = bytes([0x62])*512
                if mode in ['release', 'unmodified']:
                    initial[63*512:64*512] = bytes([0xb4])*512
                exact = image.read_bytes() == initial
                guest = [dict(seq=int(a), cpu=int(b), pid=int(c), task_generation=int(d), role=int(e), ready=f, stage=g)
                         for a,b,c,d,e,f,g in re.findall(r'\[SFS5B4\] event=(\d+) cpu=(\d+) pid=(\d+) task-generation=(\d+) role=(\d+) scheduler-ready=(\w+) stage=(\w+)', content)]
                device_events = [dict(seq=int(a), ns=int(b), epoch=int(c), queue=int(d), head=int(e), stage=f, bytes=int(g))
                                 for a,b,c,d,e,f,g in re.findall(r'SFS5B4 seq=(\d+) ns=(\d+) epoch=(\d+) queue=(\d+) head=(\d+) stage=(\w+) bytes=(\d+)', dev)]
                stages = [e['stage'] for e in device_events]
                illegal = 'ILLEGAL_POST_FENCE_ACCESS' in stages
                held = mode not in ['release', 'unmodified']
                canceled = mode == 'cancel'
                checks = dict(ready=ready, exact_image=exact, killed_after_window=code == -9,
                              post_scheduler=bool(guest) and all(e['ready']=='true' for e in guest),
                              contender=any(e['stage']=='RESET_OWNER_CONTESTED' for e in guest),
                              admission_closed=any(e['stage']=='ADMISSION_CLOSED' for e in guest),
                              terminal=any(e['stage']=='TERMINAL_QUARANTINE' for e in guest),
                              progress=any(e['stage']=='FORWARD_PROGRESS' for e in guest),
                              contention_progress=any(e['stage']=='CONTENTION_PROGRESS' for e in guest),
                              payload=not held or 'PAYLOAD_ACCESS_ENDED' in stages,
                              handshake=not held or any(e['stage']=='DEVICE_ACCESS_HANDSHAKE' for e in guest),
                              quiescence=not held or canceled or 'OLD_EPOCH_QUIESCED' in stages,
                              guard=not held or canceled or 'ORIGINAL_MAPPINGS_PROTECTED' in stages,
                              negative_detector=(illegal if mode=='negative' else not illegal),
                              reset_entry_access=mode!='reset-read' or ('RESET_ENTERED' in stages and any(e['stage']=='PAYLOAD_ACCESS_ENDED' for e in device_events[stages.index('RESET_ENTERED')+1:])))
                row = dict(cpu=cpu, cache=cache, mode=mode, status='PASS' if all(checks.values()) else 'FAIL', checks=checks,
                           guest=guest, device=device_events, host=host_events,
                           simultaneous_dma_reset='UNQUALIFIED', reset='none/cancellation' if canceled else 'real legacy whole-device',
                           released_pages=0, pinned_pages=5, serial=serial.name, trace=trace.name, device_log=device.name)
                put(raw/(tag+'.events.json'), row)
                rows.append(row)
                (raw/(tag+'.img.gz')).write_bytes(gzip.compress(image.read_bytes(), mtime=0))
                put(raw/'native-result.json', dict(status='PASS' if all(r['status']=='PASS' for r in rows) else 'PARTIAL', boots=rows))
                print(tag, row['status'], {k:v for k,v in checks.items() if not v}, flush=True)


def regression(raw):
    spec = importlib.util.spec_from_file_location('sfs5b1', ROOT/'tools/sunfs5b1_validate.py')
    base = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(base)
    base.OUT = OUT
    base.hosted(raw)
    for name, argv in [
        ('format', ['rustfmt', '--edition', '2021', '--check', 'sunlight-virtio/src/blk.rs', 'kernel/src/sfs_reset.rs', 'kernel/src/sfs5b4_test.rs', 'kernel/build.rs']),
        ('clippy', ['cargo', 'clippy', '-p', 'sunlight-virtio', '-p', 'sunlight-block', '--lib', '--offline', '--locked', '--target', 'x86_64-unknown-none']),
        ('whitespace', ['git', 'diff', '--check']),
        ('diagnostic', ['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked', '--features', 'sfs5b4-test']),
    ]:
        command(argv, raw, name)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('action', choices=['native', 'regression'])
    p.add_argument('--cpus', default='1,2,4')
    p.add_argument('--modes', default='hold,reset-read,error,negative,cancel,release,unmodified')
    p.add_argument('--caches', default='writeback,none')
    args = p.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw = OUT/'raw'/f'{args.action}-{stamp}'
    raw.mkdir()
    before = sources()
    put(raw/'source-before.json', before)
    put(raw/'runner.json', dict(argv=os.sys.argv, runner_sha256=sha(Path(__file__)), environment={k:os.getenv(k) for k in ['RUSTFLAGS','RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR','CARGO_BUILD_TARGET']}))
    if args.action == 'native':
        native(raw, [int(c) for c in args.cpus.split(',')], args.modes.split(','), args.caches.split(','))
    else:
        regression(raw)
    after = sources()
    put(raw/'source-after.json', after)
    changed = [n for n in set(before)|set(after) if before.get(n) != after.get(n)]
    put(raw/'input-integrity.json', dict(status='PASS' if not changed else 'FAIL', changed=changed))
    print(raw, flush=True)
    assert not changed, changed


if __name__ == '__main__':
    main()
