#!/usr/bin/env python3
"""Isolated SFS-5B.6 evidence capture; never authorizes guest DMA reuse."""
import datetime
import hashlib
import importlib.util
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-5b6'


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def put(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def capture():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw = OUT / 'raw' / stamp
    raw.mkdir(parents=True, exist_ok=False)
    for name, arguments in {
        'status': ['git', 'status', '--porcelain=v1', '-uall'],
        'revision': ['git', 'rev-parse', 'HEAD'],
        'branch': ['git', 'branch', '--show-current'],
        'disk': ['df', '-h', '.', '/tmp'],
        'diff': ['git', 'diff', '--binary'],
    }.items():
        with (raw / (name + '.txt')).open('xb') as output:
            subprocess.run(arguments, cwd=ROOT, stdout=output, check=True)
    names = subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
        cwd=ROOT).decode().split('\0')
    source = {}
    for name in names:
        path = ROOT / name
        if name and path.is_file() and not path.is_symlink() and not name.startswith('docs/audit/'):
            source[name] = digest(path)
    put(raw / 'source.json', source)
    prior = {}
    for directory in sorted((ROOT / 'docs/audit/sunfs').iterdir()):
        if directory.name == 'sfs-5b6':
            continue
        for path in sorted(directory.rglob('*')):
            if path.is_file() and not path.is_symlink():
                prior[str(path.relative_to(ROOT))] = digest(path)
    put(raw / 'prior-evidence.json', prior)
    put(OUT / 'source-baseline.json', dict(
        raw=str(raw.relative_to(OUT)), revision=(raw / 'revision.txt').read_text().strip(),
        branch=(raw / 'branch.txt').read_text().strip(), source_sha256=digest(raw / 'source.json'),
        prior_evidence_sha256=digest(raw / 'prior-evidence.json'),
        seed='0x5b5001 inherited unchanged', historical_pages=5, historical_bytes=20480,
        environment={key: os.getenv(key) for key in ['RUSTFLAGS', 'CARGO_TARGET_DIR', 'RUSTUP_TOOLCHAIN']}))
    print(raw, flush=True)
    return raw


def load_previous():
    spec = importlib.util.spec_from_file_location('previous', ROOT / 'tools/sunfs5b5_validate.py')
    previous = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(previous)
    previous.OUT = OUT
    return previous


if __name__ == '__main__':
    if sys.argv[1] == 'baseline':
        raw = capture()
        previous = load_previous()
        previous.command(['cargo', 'test', '-p', 'sunlight-block', '-p', 'sunlight-virtio',
                          '--lib', '--offline', '--locked', '--target', 'x86_64-unknown-linux-gnu'],
                         raw, 'baseline-storage', env=dict(os.environ, CARGO_INCREMENTAL='0'))
    elif sys.argv[1] == 'native':
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
        raw = OUT / 'raw' / ('native-' + stamp)
        raw.mkdir(parents=True, exist_ok=False)
        previous = load_previous()
        previous.QEMU = Path('/tmp/sfs5b6-qemu-build/qemu-system-x86_64')
        shutil.copy2(previous.QEMU, raw / 'qemu-system-x86_64.snapshot')
        original = Path(os.environ['SFS5B6_GUEST_DIR']) if os.getenv('SFS5B6_GUEST_DIR') else (
            ROOT / 'docs/audit/sunfs/sfs-5b5/raw/native-20261115T124316.387250Z')
        for name in ['kernel.elf', 'sfs5b5.iso']:
            shutil.copyfile(original / name, raw / name)
        put(raw / 'source-before.json', previous.sources())
        put(raw / 'qemu-source.json', {name: digest(Path('/tmp/sfs5b6-qemu-src') / name)
                                      for name in ['hw/block/virtio-blk.c', 'block/block-backend.c']})
        put(raw / 'fixture-context.json', dict(
            guest_source_directory=str(original),
            guest_source_sha256=digest(original / 'source.json') if (original / 'source.json').exists() else None,
            patch_sha256=digest(OUT / 'qemu-real-backend.patch'),
            inherited_patch_sha256=digest(OUT / 'qemu-async-worker.patch'),
            seed='0x5b5001', reset='terminal quarantine; no guest DMA release',
            execution_bound_seconds=45, terminate_grace_seconds=15, kill_grace_seconds=10))
        runs = []
        for cpu in [int(value) for value in os.getenv('SFS5B6_CPUS', '1,2,4').split(',')]:
            for cache in os.getenv('SFS5B6_CACHES', 'writeback,none').split(','):
                for test_case in os.getenv('SFS5B6_CASES', 'during,before,cancel,error,control').split(','):
                    mode = 'async' if test_case == 'control' else 'backend'
                    os.environ['SFS5B6_BACKEND'] = '1'
                    os.environ['SFS5B6_CASE'] = test_case
                    tag = f'cpu-{cpu}-{cache}-{test_case}'
                    row = previous.run_qemu(raw, tag, mode, cpu, cache, timeout_s=45)
                    with (raw / 'artifacts' / row['image']).open('rb') as image:
                        image.seek(63 * 512)
                        sector = image.read(512)
                    row['sector63_sha256'] = hashlib.sha256(sector).hexdigest()
                    row['sector63_all_b4'] = sector == bytes([0xb4]) * 512
                    row['sector63_all_zero'] = sector == bytes(512)
                    row['case'] = test_case
                    row['guest_build'] = str(original)
                    runs.append(row)
                    put(raw / 'runs.json', runs)
                    print(tag, row['ready'], row['elapsed_s'], flush=True)
        put(raw / 'source-after.json', previous.sources())
    elif sys.argv[1] == 'regression':
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
        raw = OUT / 'raw' / ('regression-' + stamp)
        raw.mkdir(parents=True, exist_ok=False)
        os.environ['CARGO_INCREMENTAL'] = '0'
        load_previous().regression(raw)
    elif sys.argv[1] == 'focused':
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
        raw = OUT / 'raw' / ('focused-' + stamp)
        raw.mkdir(parents=True, exist_ok=False)
        previous = load_previous()
        os.environ['CARGO_INCREMENTAL'] = '0'
        commands = [
            ('storage', ['cargo', 'test', '-p', 'sunlight-block', '-p', 'sunlight-fat', '-p', 'sunlight-fs',
                         '-p', 'sunlight-virtio', '--lib', '--offline', '--locked', '--target', 'x86_64-unknown-linux-gnu']),
            ('kernel-host', ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', '--offline', '--locked',
                             '--target', 'x86_64-unknown-linux-gnu']),
            ('no-std', ['cargo', 'check', '-p', 'sunlight-block', '-p', 'sunlight-virtio', '-p', 'sunlight-sunfs',
                        '--offline', '--locked', '--target', 'x86_64-unknown-none']),
            ('format', ['rustfmt', '--edition', '2021', '--check', 'sunlight-block/src/quiesce.rs',
                        'sunlight-block/src/lib.rs', 'sunlight-virtio/src/blk.rs']),
            ('clippy', ['cargo', 'clippy', '-p', 'sunlight-block', '-p', 'sunlight-virtio', '--lib',
                        '--offline', '--locked', '--target', 'x86_64-unknown-none']),
            ('default-build', ['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked']),
            ('diagnostic-build', ['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                                  '--features', 'sfs5b4-test']),
        ]
        results = [previous.command(arguments, raw, name) for name, arguments in commands]
        put(raw / 'results.json', results)
    elif sys.argv[1] == 'guest':
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
        raw = OUT / 'raw' / ('guest-' + stamp)
        raw.mkdir(parents=True, exist_ok=False)
        previous = load_previous()
        os.environ['CARGO_INCREMENTAL'] = '0'
        result = previous.command(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                                   '--features', 'sfs5b4-test'], raw, 'guest-build')
        if result['returncode']:
            sys.exit(1)
        target = Path(os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
        shutil.copyfile(target / 'x86_64-unknown-none/debug/sunlight-kernel', raw / 'kernel.elf')
        result = previous.command(['bash', 'tools/make_hybrid_iso.sh', str(raw / 'kernel.elf'),
                                   str(raw / 'sfs5b5.iso')], raw, 'iso-build')
        put(raw / 'source.json', previous.sources())
        print(raw, flush=True)
        if result['returncode']:
            sys.exit(1)
