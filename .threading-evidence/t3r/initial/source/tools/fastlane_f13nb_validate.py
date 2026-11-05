#!/usr/bin/env python3
"""Bounded D3 acceptance replay; a fresh directory retains every failed attempt.

The isolated unit of replay is the established D1/D2/D3 guest, not D3 alone.
Doorbell is an exclusive boot fixture, so host ordering never implies shared
guest state. This runner does not infer a root cause from passing repetitions.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def source_identity():
    # Enumerate source inputs without traversing generated target trees or Git
    # metadata. Include all services, not only the native Doorbell source list.
    paths = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard'],
        cwd=ROOT, text=True).splitlines()
    hashes = {p: digest(ROOT / p) for p in paths
              if (ROOT / p).is_file() and (p.endswith(('.rs', '.toml', '.ld', '.py', '.sh', '.S'))
                                         or p == 'Cargo.lock')}
    return dict(revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'],
                cwd=ROOT, text=True).strip(), source_sha256=hashes,
                manifest_sha256=hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest())


def d3_checkpoints(raw, controller='', exit_code=None):
    """Classify raw current-attempt evidence, never a copied old summary."""
    lines = raw.splitlines()
    failures = [line for line in lines if re.search(
        r'\[D[123]-TEST\].*FAIL|KERNEL PANIC|\[SUNLIGHTD\] PANIC|panicked at', line)]
    samples = [{k: int(v) for k, v in re.findall(r'(\w+)=(\d+)', line)}
               for line in lines if line.startswith('[D3-MEM] ')]
    passes = [line for line in lines if re.match(r'\[D[123]-TEST\].* PASS$', line)]
    complete = all(f'[D{i}-TEST] COMPLETE PASS' in raw for i in (1, 2, 3))
    if failures:
        status = 'GUEST_ASSERTION_FAILED'
    elif 'timed out' in controller or exit_code == 124:
        status = 'HOST_WATCHDOG_TIMEOUT'
    elif complete and len(samples) == 6 and [s.get('cycle') for s in samples] == list(range(6)):
        status = 'PASSED' if exit_code in (0, None) else 'HARNESS_FAILED_AFTER_COMPLETION'
    elif exit_code == 0:
        status = 'INCOMPLETE_EVIDENCE'
    else:
        status = 'D3_INCOMPLETE' if '[D3-TEST]' in raw else 'D3_NOT_REACHED'
    ready = [line for line in lines if line.startswith('[D1] ready unit=d3-')]
    traces = [line for line in lines if line.startswith('[D3-TRACE]')]
    steady = [s['after_free'] for s in samples if s.get('cycle', 0) > 0 and 'after_free' in s]
    return dict(classification=status, d3_reached='[D3-TEST]' in raw,
                assertion_failures=failures, last_assertion_checkpoint=passes[-1] if passes else None,
                last_ready_checkpoint=ready[-1] if ready else None,
                completed_graph_cycles=len(samples), graph_samples=samples,
                lifecycle_traces=traces,
                steady_frame_range=max(steady) - min(steady) if steady else None,
                completion_captured=complete,
                cleanup='six graph receipts captured' if len(samples) == 6 else 'incomplete; guest terminated by harness',
                scheduler_waiter_state='not instrumented in D3 image; no Doorbell fixture enabled')


def replay(iso, out, cpus, repetitions, order):
    if out.exists():
        raise SystemExit(f'use a fresh evidence directory: {out}')
    out.mkdir(parents=True)
    identity = source_identity()
    write(out / 'source.json', identity)
    summary = dict(utc=datetime.now(timezone.utc).isoformat(),
                   image=str(iso), iso_sha256=digest(iso),
                   source_manifest_sha256=identity['manifest_sha256'],
                   order=order, guest_order='D1 concurrent with D2 startup; D2 then D3',
                   d3_only_guest=False, timeout_seconds=180, attempts=[])
    write(out / 'results.json', summary)
    for repetition in range(1, repetitions + 1):
        for cpu in cpus:
            attempt = out / f'run-{repetition}-{cpu}cpu'
            attempt.mkdir()
            command = ['python3', 'tools/sunlightd_d1_gate.py', '--d2', '--d3',
                       '--iso', str(iso), '--evidence', str(attempt), '--cpus', str(cpu)]
            started = time.monotonic()
            load = os.getloadavg()
            with (attempt / 'controller.log').open('w') as log:
                try:
                    code = subprocess.run(command, cwd=ROOT, stdout=log,
                                          stderr=subprocess.STDOUT, timeout=210).returncode
                except subprocess.TimeoutExpired:
                    # The native harness owns QEMU teardown inside its 180 s
                    # budget. This outer timeout is separately classified.
                    code = 124
            raw = (attempt / f'serial-{cpu}cpu.log').read_text(errors='replace') if (
                attempt / f'serial-{cpu}cpu.log').exists() else ''
            controller = (attempt / 'controller.log').read_text(errors='replace')
            qemu = ['qemu-system-x86_64', '-cdrom', str(iso), '-serial',
                    f'file:{attempt}/serial-{cpu}cpu.log', '-display', 'none', '-m', '1024M',
                    '-smp', str(cpu), '-device', 'virtio-rng-pci,disable-modern=on',
                    '-no-reboot', '-no-shutdown']
            if Path('/dev/kvm').exists():
                qemu += ['-enable-kvm', '-cpu', 'host']
            row = dict(repetition=repetition, cpus=cpu, command=command, qemu_command=qemu,
                       exit_code=code, wall_seconds=time.monotonic() - started,
                       host_load_start=load, checkpoints=d3_checkpoints(raw, controller, code),
                       serial_sha256=hashlib.sha256(raw.encode()).hexdigest())
            summary['attempts'].append(row)
            write(out / 'results.json', summary)
            print(f"{out.name}/{attempt.name}: {row['checkpoints']['classification']}", flush=True)
    summary['image_unchanged'] = digest(iso) == summary['iso_sha256']
    summary['source_unchanged'] = source_identity()['manifest_sha256'] == identity['manifest_sha256']
    write(out / 'results.json', summary)
    return (summary['image_unchanged'] and summary['source_unchanged']
            and all(r['checkpoints']['classification'] == 'PASSED' for r in summary['attempts']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--iso', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--cpus', type=int, nargs='+', default=[1, 2, 4])
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--order', nargs='+', default=['isolated-orchestration'])
    args = parser.parse_args()
    if any(cpu not in (1, 2, 4) for cpu in args.cpus) or not 1 <= args.repetitions <= 5:
        parser.error('D3 supports 1/2/4 CPUs and 1..5 bounded repetitions')
    if not replay(args.iso.resolve(), args.evidence.resolve(), args.cpus,
                  args.repetitions, args.order):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
