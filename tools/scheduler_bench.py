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
import re


RECORD = re.compile(r'^\[SCHED-BENCH\] (START|SAMPLE|END) (\{.*\})$')


def validate_guest_report(raw, seed, cpus, repetition):
    """Require a complete framed report; boot output alone is never a sample."""
    records = []
    for line in raw.splitlines():
        match = RECORD.match(line.strip())
        if match:
            try:
                records.append((match.group(1), json.loads(match.group(2))))
            except json.JSONDecodeError:
                return False, 'malformed_record'
    starts = [v for k, v in records if k == 'START']
    ends = [v for k, v in records if k == 'END']
    samples = [v for k, v in records if k == 'SAMPLE']
    if len(starts) != 1 or len(ends) != 1:
        return False, 'missing_or_duplicate_boundary'
    if records[0][0] != 'START' or records[-1][0] != 'END':
        return False, 'record_order_mismatch'
    start, end = starts[0], ends[0]
    expected_id = f'{cpus}-{repetition}-{seed}'
    if start.get('run') != expected_id or end.get('run') != expected_id:
        return False, 'run_id_mismatch'
    if start.get('seed') != seed or start.get('cpus') != cpus:
        return False, 'configuration_mismatch'
    count = start.get('samples')
    if not isinstance(count, int) or count < 1 or len(samples) != count:
        return False, 'incomplete_samples'
    if [v.get('index') for v in samples] != list(range(count)):
        return False, 'sample_index_mismatch'
    if any(v.get('run') != expected_id for v in samples):
        return False, 'sample_run_id_mismatch'
    if end.get('sample_count') != count or end.get('status') != 'ok':
        return False, 'invalid_end_record'
    return True, None


def sha256(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--iso', type=Path, required=True)
    p.add_argument('--suite', action='store_true', help='capture the 22-run C1.2B suite; duration is host timeout')
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
    if args.suite and (args.cpus != [1] or args.repetitions != 1 or args.seed != 1):
        p.error('C1.2B suite requires --cpus 1 --repetitions 1 --seed 1; it contains its own repetitions')
    args.output.mkdir(parents=True, exist_ok=True)
    base = {
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'worktree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], text=True).strip()),
        'iso': str(args.iso.resolve()), 'iso_sha256': sha256(args.iso),
        'qemu_version': subprocess.check_output([qemu, '--version'], text=True).splitlines()[0],
        'accelerator': args.accel, 'host': platform.platform(),
        'host_logical_cpus': os.cpu_count(), 'host_affinity': sorted(os.sched_getaffinity(0)),
        'cpus': args.cpus,
        'warmup_seconds': args.warmup, 'measurement_seconds': args.duration,
        'repetitions': args.repetitions, 'seed': args.seed,
        'memory': args.memory, 'cpu_model': args.cpu_model,
        'suite': args.suite, 'host_load_start': os.getloadavg(),
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
            serial.write_text('')
            with qlog.open('w') as log:
                proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                try:
                    if args.suite:
                        deadline = time.monotonic() + args.warmup + args.duration
                        while time.monotonic() < deadline and proc.poll() is None:
                            raw = serial.read_text(errors='replace') if serial.exists() else ''
                            if any(marker in raw for marker in ('[SCHED-BENCH] SUITE complete runs=22', '[SCHED-BENCH] suite failed', '[FAULT]')):
                                break
                            time.sleep(0.25)
                    else:
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
            run_seed = args.seed + rep
            if args.suite:
                from scheduler_fairness import validate_suite
                try:
                    measured = validate_suite(raw)
                    (args.output / f'{stem}.fairness.json').write_text(json.dumps(measured, indent=2) + '\n')
                    valid, invalid_reason = True, None
                except (ValueError, KeyError, TypeError, ZeroDivisionError) as error:
                    valid, invalid_reason = False, str(error)
            else:
                valid, invalid_reason = validate_guest_report(raw, run_seed, ncpu, rep + 1)
            base['results'].append({
                'cpus': ncpu, 'repetition': rep + 1, 'seed': run_seed,
                'command': command, 'start_unix': started,
                'elapsed_seconds': time.time() - started, 'qemu_exit': code,
                'serial_file': str(serial), 'serial_sha256': sha256(serial) if serial.exists() else None,
                'serial_bytes': len(raw), 'qemu_log': str(qlog),
                'guest_report_markers': [line for line in raw.splitlines() if '[SCHED-BENCH]' in line],
                'guest_report_complete': valid,
                'guest_report_invalid_reason': invalid_reason,
            })
            base['host_load_end'] = os.getloadavg()
            (args.output / 'results.json').write_text(json.dumps(base, indent=2) + '\n')
            print(f'{ncpu} CPUs repetition {rep + 1}: valid={valid} reason={invalid_reason}, captured {serial}', flush=True)
            if not valid:
                raise SystemExit(1)


if __name__ == '__main__':
    main()
