#!/usr/bin/env python3
"""Repeatable QEMU capture harness for a scheduler benchmark ISO.

The ISO must contain and start its own deterministic benchmark workload. This
tool records the complete serial stream and exact VM command; it does not infer
throughput from elapsed guest time or boot success.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time


def sha256(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--iso', type=Path, required=True)
    p.add_argument('--cpus', nargs='+', type=int, default=[1, 2, 4, 8, 12])
    p.add_argument('--accel', choices=['tcg', 'kvm'], default='tcg')
    p.add_argument('--warmup', type=int, default=30)
    p.add_argument('--duration', type=int, default=60)
    p.add_argument('--repetitions', type=int, default=5)
    p.add_argument('--seed', type=int, default=1)
    p.add_argument('--memory', default='1536M')
    p.add_argument('--cpu-model', default='max')
    p.add_argument('--output', type=Path, default=Path('target/scheduler-perf'))
    args = p.parse_args()
    if not args.iso.is_file() or not args.cpus or any(c not in (1, 2, 4, 8, 12) for c in args.cpus) or args.warmup < 0 or args.duration < 1 or args.repetitions < 1:
        p.error('ISO must exist; CPUs must be selected from 1, 2, 4, 8, 12; durations and repetitions must be positive')
    qemu = shutil.which('qemu-system-x86_64')
    if not qemu:
        p.error('qemu-system-x86_64 not found')
    args.output.mkdir(parents=True, exist_ok=True)
    base = {
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'worktree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], text=True).strip()),
        'iso': str(args.iso.resolve()), 'iso_sha256': sha256(args.iso),
        'qemu_version': subprocess.check_output([qemu, '--version'], text=True).splitlines()[0],
        'accelerator': args.accel, 'host': platform.platform(),
        'host_logical_cpus': os.cpu_count(), 'cpus': args.cpus,
        'warmup_seconds': args.warmup, 'measurement_seconds': args.duration,
        'repetitions': args.repetitions, 'seed': args.seed,
        'memory': args.memory, 'cpu_model': args.cpu_model,
        'results': [],
    }
    for ncpu in args.cpus:
        for rep in range(args.repetitions):
            stem = f'{ncpu}cpu-run{rep + 1}'
            serial = args.output / f'{stem}.serial.log'
            qlog = args.output / f'{stem}.qemu.log'
            command = [qemu, '-accel', 'tcg,thread=multi' if args.accel == 'tcg' else 'kvm',
                       '-cpu', args.cpu_model, '-smp', str(ncpu), '-m', args.memory,
                       '-cdrom', str(args.iso.resolve()), '-serial', f'file:{serial}',
                       '-display', 'none', '-nic', 'none', '-no-reboot', '-no-shutdown']
            started = time.time()
            with qlog.open('w') as log:
                proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                try:
                    time.sleep(args.warmup + args.duration)
                finally:
                    if proc.poll() is None:
                        proc.terminate()
                    try:
                        code = proc.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        code = proc.wait()
            raw = serial.read_text(errors='replace') if serial.exists() else ''
            base['results'].append({
                'cpus': ncpu, 'repetition': rep + 1, 'seed': args.seed + rep,
                'command': command, 'start_unix': started,
                'elapsed_seconds': time.time() - started, 'qemu_exit': code,
                'serial_file': str(serial), 'serial_sha256': sha256(serial) if serial.exists() else None,
                'serial_bytes': len(raw), 'qemu_log': str(qlog),
                'guest_report_markers': [line for line in raw.splitlines() if '[SCHED-PERF]' in line],
            })
            (args.output / 'results.json').write_text(json.dumps(base, indent=2) + '\n')
            print(f'{ncpu} CPUs repetition {rep + 1}: captured {serial}', flush=True)


if __name__ == '__main__':
    main()
