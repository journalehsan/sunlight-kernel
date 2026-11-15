#!/usr/bin/env python3
"""Bounded SFS5B.5 native runner: genuine asynchronous access vs reset.

Never touches an existing image, a physical block device, or earlier audit
output.  Every invocation binds its source inventory, emulator, ELF, ISO,
disposable image, arguments and environment by hash.
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
OUT = ROOT / 'docs/audit/sunfs/sfs-5b5'
QEMU = Path('/tmp/sfs5b5-qemu-build/qemu-system-x86_64')
SEED = 0x5b5001
MARKER = b'SFS5A-ONLY-7b93143a-disposable-v1!'


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def put(p, value):
    Path(p).write_text(json.dumps(value, indent=2) + '\n')


def sources():
    names = subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
        cwd=ROOT).decode().split('\0')
    return {n: sha(ROOT / n) for n in names
            if n and (ROOT / n).is_file() and not (ROOT / n).is_symlink()
            and not n.startswith(('docs/audit/', 'docs/filesystems/'))}


def command(argv, raw, name, timeout=3600, env=None):
    start = time.monotonic_ns()
    with (raw / (name + '.log')).open('x') as log:
        try:
            code = subprocess.run(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                  timeout=timeout, env=env).returncode
        except subprocess.TimeoutExpired:
            code = 'TIMEOUT'
    row = dict(argv=argv, returncode=code, start_ns=start, end_ns=time.monotonic_ns(),
               log=str((raw / (name + '.log')).relative_to(OUT)))
    with (raw / 'commands.jsonl').open('a') as f:
        f.write(json.dumps(row) + '\n')
    print(name, code, flush=True)
    return row


def run_qemu(raw, tag, mode, cpu, cache, timeout_s=180, repeat=0):
    artifacts = raw / 'artifacts'
    artifacts.mkdir(exist_ok=True)
    image = artifacts / (tag + '.img')
    initial = bytearray(8192 * 512)
    initial[:len(MARKER)] = MARKER
    with image.open('xb') as f:
        f.write(initial)
        f.flush()
        os.fsync(f.fileno())
    serial = raw / (tag + '.serial.log')
    qlog = raw / (tag + '.qemu.log')
    env = dict(os.environ)
    for k in list(env):
        if k.startswith(('SFS5B3_', 'SFS5B5_')):
            del env[k]
    executable = str(QEMU)
    if mode == 'unmodified':
        executable = shutil.which('qemu-system-x86_64')
    else:
        env['SFS5B3_MODE'] = mode
    argv = [executable, '-machine', 'q35', '-accel', 'tcg', '-m', '1024M', '-smp', str(cpu),
            '-cdrom', str(raw / 'sfs5b5.iso'), '-boot', 'd', '-display', 'none',
            '-serial', f'file:{serial}', '-monitor', 'none',
            '-drive', f'id=sfs5b5,file={image},if=none,format=raw,cache={cache}',
            '-device', 'virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=sfs5b5,serial=SFS5B3-disposable',
            '-device', 'virtio-rng-pci,disable-modern=on', '-nic', 'none',
            '-no-reboot', '-no-shutdown']
    put(raw / (tag + '.invocation.json'), dict(
        argv=argv, seed=SEED,
        seed_use='deterministic fixture bytes and named barriers, no random fault scheduling',
        environment={k: v for k, v in env.items() if k.startswith('SFS5B')},
        elf_sha256=sha(raw / 'kernel.elf'), iso_sha256=sha(raw / 'sfs5b5.iso'),
        qemu_sha256=sha(Path(executable)),
        patch_sha256=sha(OUT / 'qemu-async-worker.patch'),
        initial_image_sha256=sha(image), repeat=repeat))
    start = time.monotonic()
    with qlog.open('xb') as out:
        proc = subprocess.Popen(argv, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT, env=env)
        ready = False
        while time.monotonic() - start < timeout_s:
            if proc.poll() is not None:
                break
            if serial.exists():
                try:
                    if b'[SFS5B4] READY' in serial.read_bytes():
                        ready = True
                        break
                except OSError:
                    pass
            time.sleep(0.2)
        proc.terminate()
        try:
            proc.wait(15)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(10)
    elapsed = time.monotonic() - start
    return dict(tag=tag, mode=mode, cpu=cpu, cache=cache, ready=ready,
                elapsed_s=round(elapsed, 3), serial=serial.name, qemu_log=qlog.name,
                image=image.name, exit=proc.returncode)


def native(raw, cpus, modes, caches, repeats):
    build = command(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                     '--features', 'sfs5b4-test'], raw, 'guest-build')
    if build['returncode']:
        put(raw / 'native-result.json', dict(status='BLOCKED', build=build))
        return
    shutil.copyfile(ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel', raw / 'kernel.elf')
    row = command(['bash', 'tools/make_hybrid_iso.sh', str(raw / 'kernel.elf'),
                   str(raw / 'sfs5b5.iso')], raw, 'iso-build')
    if row['returncode'] or not QEMU.exists():
        put(raw / 'native-result.json', dict(status='BLOCKED', iso=row, qemu=QEMU.exists()))
        return
    frozen = sources()
    put(raw / 'source-before.json', frozen)
    runs = []
    for cpu in cpus:
        for cache in caches:
            for mode in modes:
                for rep in range(repeats if mode == 'async' else 1):
                    tag = f'cpu-{cpu}-{cache}-{mode}-r{rep}'
                    runs.append(run_qemu(raw, tag, mode, cpu, cache, repeat=rep))
                    put(raw / 'native-runs.json', runs)
                    print(tag, runs[-1]['ready'], runs[-1]['elapsed_s'], flush=True)
    put(raw / 'native-result.json', dict(
        status='PASS' if all(r['ready'] for r in runs) else 'PARTIAL', runs=runs))


def stall(raw, repeats):
    """Replay the preserved SFS-5B.4 cpu-2 writeback unmodified-QEMU scenario."""
    build = command(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                     '--features', 'sfs5b4-test'], raw, 'guest-build')
    if build['returncode']:
        put(raw / 'stall-result.json', dict(status='BLOCKED', build=build))
        return
    shutil.copyfile(ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel', raw / 'kernel.elf')
    command(['bash', 'tools/make_hybrid_iso.sh', str(raw / 'kernel.elf'),
             str(raw / 'sfs5b5.iso')], raw, 'iso-build')
    runs = []
    for rep in range(repeats):
        runs.append(run_qemu(raw, f'stall-cpu-2-writeback-unmodified-r{rep}',
                             'unmodified', 2, 'writeback', repeat=rep))
        put(raw / 'stall-runs.json', runs)
        print(runs[-1]['tag'], runs[-1]['ready'], runs[-1]['elapsed_s'], flush=True)
    put(raw / 'stall-result.json', dict(
        status='PASS' if all(r['ready'] for r in runs) else 'PARTIAL', runs=runs,
        note='a pass does not establish the historical cause of the SFS-5B.4 stall'))


def regression(raw):
    spec = importlib.util.spec_from_file_location('sfs5b1', ROOT / 'tools/sunfs5b1_validate.py')
    base = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(base)
    base.OUT = OUT
    base.hosted(raw)
    for name, argv in [
        ('format', ['rustfmt', '--edition', '2021', '--check',
                    'sunlight-block/src/quiesce.rs', 'sunlight-block/src/lib.rs',
                    'sunlight-virtio/src/blk.rs']),
        ('clippy', ['cargo', 'clippy', '-p', 'sunlight-virtio', '-p', 'sunlight-block',
                    '--lib', '--offline', '--locked', '--target', 'x86_64-unknown-none']),
        ('whitespace', ['git', 'diff', '--check']),
        ('diagnostic', ['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                        '--features', 'sfs5b4-test']),
    ]:
        command(argv, raw, name)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('action', choices=['native', 'stall', 'regression'])
    p.add_argument('--cpus', default='1,2,4')
    p.add_argument('--modes', default='async,async-negative')
    p.add_argument('--caches', default='writeback,none')
    p.add_argument('--repeats', type=int, default=2)
    p.add_argument('--stall-repeats', type=int, default=5)
    args = p.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw = OUT / 'raw' / f'{args.action}-{stamp}'
    raw.mkdir(parents=True)
    put(raw / 'runner.json', dict(
        argv=os.sys.argv, runner_sha256=sha(Path(__file__)),
        environment={k: os.getenv(k) for k in
                     ['RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET']}))
    before = sources()
    if args.action == 'native':
        native(raw, [int(c) for c in args.cpus.split(',')], args.modes.split(','),
               args.caches.split(','), args.repeats)
    elif args.action == 'stall':
        stall(raw, args.stall_repeats)
    else:
        regression(raw)
    after = sources()
    changed = [n for n in set(before) | set(after) if before.get(n) != after.get(n)]
    put(raw / 'input-integrity.json', dict(
        status='PASS' if not changed else 'FAIL', changed=changed))
    print(raw, flush=True)
    assert not changed, changed


if __name__ == '__main__':
    main()
