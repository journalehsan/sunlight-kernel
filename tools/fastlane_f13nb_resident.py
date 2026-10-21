#!/usr/bin/env python3
"""Build normal embedded lifecycle binaries before resident compatibility.

Kernel include_bytes! consumes canonical release outputs. A kernel-only rebuild
after a dependency/recovery gate otherwise embeds those test-feature binaries.
Keep the flawed attempts; rebuild explicit normal inputs and reject test units.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import time

from fastlane_f13nb_validate import ROOT, digest, source_identity, write

TEST_MARKERS = ('ExecStart=/sbin/d1-driver', 'ExecStart=/sbin/d2-driver',
                '[D1-TEST]', '[D2-TEST]', '[D3-TEST]', '[D36R-TEST]',
                'capability profile d1-driver', 'capability profile d2-driver',
                'path=/sbin/d1-driver', 'path=/sbin/d2-driver')


def required(cpus):
    markers = [line for name in ('phase_sec.expected', 'phase_shm.expected')
               for line in (ROOT / 'tools/tests' / name).read_text().splitlines()
               if line and not line.startswith('#')]
    return markers + [
        '[FASTLANE-F1] capability issuance regression: adjacent token prediction REJECTED',
        '[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)',
        f'[FASTLANE-F1] remote SHM release: cpus={cpus} acknowledgements={cpus - 1} OK',
        '[SunlightOS] Phase 2.6 OK', '[SunlightOS] sunlightd OK',
        '[SUNLIGHTD] spawned sunlight-tls',
    ]


def validate_resident(raw, cpus):
    missing = [marker for marker in required(cpus) if marker not in raw]
    contamination = [marker for marker in TEST_MARKERS if marker in raw]
    faults = [marker for marker in ('KERNEL PANIC', '[SUNLIGHTD] PANIC', 'panicked at') if marker in raw]
    return dict(passed=not missing and not contamination and not faults,
                missing=missing, test_fixture_markers=contamination, faults=faults)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--cpus', type=int, nargs='+', default=[1, 2, 4, 12])
    parser.add_argument('--d3-image-only', action='store_true',
                        help='build dependency-test manager/fixtures with explicitly normal Init; do not boot')
    args = parser.parse_args()
    out = args.evidence.resolve()
    if out.exists() or any(c not in (1, 2, 4, 12) for c in args.cpus):
        parser.error('use a fresh evidence directory and CPUs 1/2/4/12')
    out.mkdir(parents=True)
    summary = dict(source=source_identity(), commands=[], results=[])
    write(out / 'results.json', summary)

    def build(name, command, flags=None):
        env = dict(os.environ)
        if flags is not None:
            env['RUSTFLAGS'] = flags
        with (out / f'{name}.log').open('w') as log:
            code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        summary['commands'].append(dict(command=command, RUSTFLAGS=flags, exit_code=code))
        write(out / 'results.json', summary)
        if code:
            raise SystemExit(code)

    packages = ['-p', 'sunlight-init'] if args.d3_image_only else ['-p', 'sunlight-init', '-p', 'sunlightd']
    flags = '-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
    build('normal-userspace', ['cargo', 'build', *packages,
          '--release', '--offline', '--no-default-features'],
          flags)
    if args.d3_image_only:
        build('d3-userspace', ['cargo', 'build', '-p', 'sunlightd', '-p', 'sunlight-startup-fixture',
              '--features', 'sunlightd/dependency-test,sunlight-startup-fixture/dependency-test',
              '--release', '--offline'], flags)
    summary['embedded_inputs'] = {}
    for name in ('sunlight-init', 'sunlightd'):
        source = ROOT / 'target/x86_64-unknown-none/release' / name
        destination = out / name
        shutil.copy2(source, destination)
        summary['embedded_inputs'][name] = digest(destination)
    manager = (out / 'sunlightd').read_bytes()
    assert b'[D36R-TEST]' not in (out / 'sunlight-init').read_bytes()
    for marker in (b'ExecStart=/sbin/d1-driver', b'ExecStart=/sbin/d2-driver'):
        assert (marker in manager) == args.d3_image_only
    feature = 'dependency_test' if args.d3_image_only else 'fastlane_f1_audit'
    summary['kernel_feature'] = feature
    build('kernel', ['cargo', 'build', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel',
                     '--features', feature, '--offline'])
    elf, iso = out / 'sunlight-kernel.elf', out / 'sunlightos.iso'
    shutil.copy2(ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel', elf)
    build('iso', ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso)])
    summary['elf_sha256'], summary['iso_sha256'] = digest(elf), digest(iso)
    # Check the actual included bytes, not merely Cargo's successful exit.
    embedded = elf.read_bytes()
    assert (out / 'sunlightd').read_bytes() in embedded
    assert (out / 'sunlight-init').read_bytes() in embedded
    del embedded
    if args.d3_image_only:
        summary['source_unchanged'] = source_identity() == summary['source']
        write(out / 'results.json', summary)
        print(f'D3 image with normal Init: {iso}', flush=True)
        if not summary['source_unchanged']:
            raise SystemExit(1)
        return
    for cpu in args.cpus:
        serial = out / f'{cpu}cpu.serial.log'
        serial.write_text('')
        command = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                   '-smp', str(cpu), '-m', '1024', '-cdrom', str(iso),
                   '-serial', f'file:{serial}', '-display', 'none', '-nic', 'none',
                   '-device', 'virtio-rng-pci,disable-modern=on', '-no-reboot', '-no-shutdown',
                   '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04']
        start = time.monotonic()
        timeout = False
        with (out / f'{cpu}cpu.qemu.log').open('w') as log:
            proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            try:
                while time.monotonic() - start < 180:
                    raw = serial.read_text(errors='replace')
                    row = validate_resident(raw, cpu)
                    if row['passed'] or row['test_fixture_markers'] or row['faults'] or proc.poll() is not None:
                        break
                    time.sleep(.2)
                else:
                    timeout = True
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                proc.wait()
        row = dict(cpus=cpu, command=command, timeout=timeout, exit_code=proc.returncode,
                   wall_seconds=time.monotonic() - start,
                   **validate_resident(serial.read_text(errors='replace'), cpu))
        if timeout:
            row['passed'] = False
        summary['results'].append(row)
        write(out / 'results.json', summary)
        print(f'normal resident {cpu} CPU: {row}', flush=True)
    summary['source_unchanged'] = source_identity() == summary['source']
    write(out / 'results.json', summary)
    if not summary['source_unchanged'] or not all(r['passed'] for r in summary['results']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
