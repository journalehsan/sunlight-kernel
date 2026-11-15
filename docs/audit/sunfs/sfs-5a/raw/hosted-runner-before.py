#!/usr/bin/env python3
"""SFS-5A bounded qualification; no existing evidence is rewritten.

Only creates exclusive regular image files beneath this milestone's attempt.
Never attaches a host device or an existing boot/root disk to QEMU.
"""
import argparse
import datetime
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-5a'
MARKER = b'SFS5A-ONLY-7b93143a-disposable-v1!'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def source():
    names = subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT
    ).decode().split('\0')
    return {n: sha(ROOT/n) for n in names if n and (ROOT/n).is_file()
            and not (ROOT/n).is_symlink() and not n.startswith('docs/audit/')}


def run(argv, path, env=None):
    start = time.monotonic()
    with path.open('w') as log:
        p = subprocess.run(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, env=env)
    row = dict(argv=argv, returncode=p.returncode, log=str(path.relative_to(OUT)),
               seconds=time.monotonic()-start)
    with (path.parent/'commands.jsonl').open('a') as f:
        f.write(json.dumps(row)+'\n')
    print(path.name, p.returncode, flush=True)
    return row


def boot(raw, iso, image, cpu, cache, restart):
    tag = f'cpu-{cpu}-{cache}-' + ('restart' if restart else 'initial')
    serial = raw/f'{tag}.serial.log'
    argv = ['qemu-system-x86_64', '-machine', 'q35', '-accel', 'tcg',
            '-m', '1024M', '-smp', str(cpu), '-cdrom', str(iso), '-boot', 'd',
            '-display', 'none', '-serial', f'file:{serial}', '-monitor', 'none',
            '-drive', f'id=sfs5atest,file={image},if=none,format=raw,cache={cache}',
            '-device', 'virtio-blk-pci,disable-modern=on,drive=sfs5atest,serial=SFS5A-7b93143a',
            '-no-reboot', '-no-shutdown', '-nic', 'none']
    write(raw/f'{tag}.invocation.json', dict(argv=argv, cache=cache, cpu=cpu,
          identity=MARKER.decode(), image=str(image), destructive_lbas=[1, 4, *range(8, 19), *range(32, 48)],
          source_sha256=source(), elf_sha256=sha(iso.parent/'sunlight-kernel.elf'), iso_sha256=sha(iso)))
    ready = False
    with (raw/f'{tag}.qemu.log').open('w') as errors:
        p = subprocess.Popen(argv, cwd=ROOT, stdout=errors, stderr=errors)
        try:
            deadline = time.monotonic()+90
            while p.poll() is None and time.monotonic() < deadline:
                content = serial.read_text(errors='replace') if serial.exists() else ''
                if '[SFS5A] READY-FOR-HOST-KILL' in content:
                    ready = True
                    break
                if 'panicked at' in content:
                    break
                time.sleep(.2)
        finally:
            # Abrupt VM death, with no guest shutdown or monitor flush.
            if p.poll() is None:
                p.kill()
            code = p.wait(timeout=10)
    content = serial.read_text(errors='replace') if serial.exists() else ''
    expected = '[SFS5A] B13 RESTART PASS' if restart else '[SFS5A] B14 serialized-barriers PASS'
    return dict(cpu=cpu, cache=cache, restart=restart,
                status='PASS' if ready and expected in content else 'FAIL',
                returncode=code, serial=str(serial.relative_to(OUT)),
                termination='SIGKILL after acknowledged cut' if ready else 'failed/timeout attempt')


def native(raw):
    import shutil
    build = run(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                 '--features', 'sfs5a-test'], raw/'build-guest.log')
    if build['returncode']:
        write(raw/'native-result.json', dict(status='BLOCKED', build=build)); return
    artifacts = ROOT/'target/sfs5a'/raw.name
    artifacts.mkdir(parents=True)
    shutil.copyfile(ROOT/'target/x86_64-unknown-none/debug/sunlight-kernel', artifacts/'sunlight-kernel.elf')
    iso = artifacts/'sfs5a.iso'
    build_iso = run(['bash', 'tools/make_hybrid_iso.sh', str(artifacts/'sunlight-kernel.elf'), str(iso)], raw/'build-iso.log')
    if build_iso['returncode']:
        write(raw/'native-result.json', dict(status='BLOCKED', build=build_iso)); return
    run(['qemu-system-x86_64', '--version'], raw/'qemu-version.log')
    rows = []
    for cache in ['writeback', 'none']:
        for cpu in [1, 2, 4]:
            image = artifacts/f'cpu-{cpu}-{cache}.img'
            expected = bytearray(8192*512)
            expected[:len(MARKER)] = MARKER
            expected[17*512:18*512] = bytes([0xa1])*512
            expected[18*512:19*512] = bytes([0xa2])*512
            with image.open('xb') as f:
                f.write(expected); f.flush(); os.fsync(f.fileno())
            before = sha(image)
            row = boot(raw, iso, image, cpu, cache, False)
            if row['status'] == 'PASS':
                for lba, value in [(1, 0x5a), (4, 0x42), *[(i, 0xd1) for i in range(8, 16)],
                                   (16, 0xc1), (17, 0), (18, 0), *[(32+i, i) for i in range(16)]]:
                    expected[lba*512:(lba+1)*512] = bytes([value])*512
                row['host_exact_image'] = image.read_bytes() == expected
                row['before_sha256'] = before
                row['after_sha256'] = sha(image)
                if not row['host_exact_image']:
                    row['status'] = 'FAIL'
            rows.append(row)
            write(raw/'native-result.json', dict(status='PARTIAL', boots=rows))
            print(cpu, cache, 'initial', row['status'], flush=True)
            if row['status'] == 'PASS':
                row = boot(raw, iso, image, cpu, cache, True)
                row['host_exact_image'] = image.read_bytes() == expected
                if not row['host_exact_image']:
                    row['status'] = 'FAIL'
                rows.append(row)
                print(cpu, cache, 'restart', row['status'], flush=True)
            # Lossless bytes retained under evidence; boot binaries remain ignored.
            (raw/(image.name+'.gz')).write_bytes(gzip.compress(image.read_bytes(), mtime=0))
            write(raw/'native-result.json', dict(status='PASS' if len(rows)==12 and all(x['status']=='PASS' for x in rows) else 'PARTIAL', boots=rows))


def hosted(raw):
    results = []
    packages = ['sunlight-sunfs', 'sunlight-sunfs-lab', 'sunlight-sunfs-check']
    for profile in ['debug', 'release']:
        env = dict(os.environ)
        for sub, var in [('sfs1', 'SUNFS_EVIDENCE_DIR'), ('sfs2', 'SUNFS_SFS2_EVIDENCE_DIR'),
                         ('sfs3', 'SUNFS_SFS3_EVIDENCE_DIR'), ('sfs4', 'SUNFS_SFS4_EVIDENCE_DIR')]:
            d = raw/profile/sub; d.mkdir(parents=True)
            env[var] = str(d)
        argv = ['cargo', 'test', *sum((['-p', p] for p in packages), []), '--offline', '--locked',
                '--target', 'x86_64-unknown-linux-gnu', *(['--release'] if profile=='release' else [])]
        row = run(argv, raw/f'hosted-{profile}.log', env)
        log = (raw/f'hosted-{profile}.log').read_text()
        row['tests_passed'] = sum(map(int, re.findall(r'test result: ok\. (\d+) passed', log)))
        results.append(row)
        for p in (raw/profile).rglob('*'):
            if p.is_file() and p.suffix in ('.img', '.trace', '.ownership'):
                p.with_suffix(p.suffix+'.gz').write_bytes(gzip.compress(p.read_bytes(), mtime=0))
                p.unlink()
    results.append(run(['cargo', 'test', '-p', 'sunlight-block', '-p', 'sunlight-fat', '-p', 'sunlight-fs',
                        '-p', 'sunlight-virtio', '--lib', '--offline', '--locked', '--target', 'x86_64-unknown-linux-gnu'], raw/'storage.log'))
    results.append(run(['cargo', 'check', '-p', 'sunlight-sunfs', '-p', 'sunlight-block', '-p', 'sunlight-virtio',
                        '--offline', '--locked', '--target', 'x86_64-unknown-none'], raw/'no-std.log'))
    write(raw/'hosted-result.json', results)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['native', 'hosted'])
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw = OUT/'raw'/f'{args.mode}-{stamp}'; raw.mkdir()
    write(raw/'source-before.json', source())
    if args.mode == 'native': native(raw)
    else: hosted(raw)
    write(raw/'source-after.json', source())
    assert json.loads((raw/'source-before.json').read_text()) == source(), 'sources changed during run'
    print(raw, flush=True)


if __name__ == '__main__':
    main()
