#!/usr/bin/env python3
"""Bounded T3 commands/guest gates with fresh, durable attempt directories.

Examples:
  python3 tools/threading_t3_validate.py command host -- cargo test ...
  python3 tools/threading_t3_validate.py guest pilot --iso image.iso --cpus 1 \
      --marker '[T2] PASS'
Every attempt records source hashes, exact argv, logs and a verdict, including
exceptions/timeouts. No existing attempt is overwritten. TCG is functional proof.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
import re
from pathlib import Path
import signal
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]


def marker_assess(text, markers, cpus=None):
    from threading_t2r_receipt import assess_pilot
    pilots = [re.fullmatch(r"\[T2R\] PILOT PASS case=(\d+) cpus=(\d+)", marker) for marker in markers]
    for pilot in pilots:
        if pilot and (int(pilot[2]) != cpus or not assess_pilot(text, int(pilot[1]), cpus)):
            return False

    if not markers or any(not m for m in markers):
        return False
    if any(m in text for m in ('KERNEL PANIC', '[EXCEPTION]', 'THREAD_PROBE FAIL',
                               'FP isolation FAILED', 'probe panicked', 'parser/raster FAILED')):
        return False
    if not all(text.count(m) == 1 and '\n' in text.split(m, 1)[1] for m in markers):
        return False
    if '[T2] PASS' in markers:
        begin = re.findall(r'^\[T2\] begin cpus=(\d+) limits=32,96,512 cycles=3$', text, re.M)
        if len(begin) != 1 or (cpus is not None and int(begin[0]) != cpus):
            return False
        for abi in (0, 1):
            faults = re.findall(rf'^\[T2-FAULT\] abi={abi} ordinal=(\d+) stage=([a-z-]+) rejected=ENOMEM$', text, re.M)
            if [(int(n), stage) for n, stage in faults] != list(enumerate(
                    ['task-table'] + ['run-queue'] * 64 + ['environment-entries']
                    + ['string'] * 4 + ['capabilities', 'string', 'box', 'box', 'publication'], 1)):
                return False
            if text.count(f'[T2] faults abi={abi} points=75 cleanup=exact\n') != 1:
                return False
        pressure = re.findall(r'^\[T2\] pressure cycle=(\d+) workers=96 vm_workers=32 observer=(\d+) useful_cpus=0x([0-9a-f]+)$', text, re.M)
        if [int(row[0]) for row in pressure] != [0, 1, 2]:
            return False
        if any(int(n) == 0 or int(mask, 16).bit_count() < min(2, int(begin[0]))
               for _, n, mask in pressure):
            return False
        recovery = re.findall(r'^\[T2\] recovery cycle=(\d+) heap=(\d+) drift=0 unreaped_workers=0$', text, re.M)
        if [int(n) for n, _ in recovery] != [1, 2, 3] or len({h for _, h in recovery}) != 1:
            return False
        if text.count('[T2] PASS faults_native=75 faults_linux=75 cycles=3 pressure=96 recovery=exact queues=unique T1_pins=preserved\n') != 1:
            return False
    return True


def assess(text, markers, cpus=None):
    if '[T3R] PASS' in markers:
        from threading_t3r_receipt import assess_migration
        if not assess_migration(text, cpus):
            return False
    if not marker_assess(text, markers, cpus):
        return False
    rows = re.findall(r'^\[CPU-ID\] logical=(\d+) apic=(\d+) bsp=(true|false)$', text, re.M)
    if cpus is None or len(rows) != cpus:
        return False
    if [int(r[0]) for r in rows] != list(range(cpus)) or len({r[1] for r in rows}) != cpus:
        return False
    if [r[2] for r in rows] != ['true'] + ['false'] * (cpus - 1):
        return False
    if any(int(r[1]) >= 255 for r in rows):
        return False
    online = re.findall(r'^\[CPU-ID\] online logical=(\d+) apic=(\d+) tss=(0x[0-9a-f]+) ist_df=(0x[0-9a-f]+) ist_nmi=(0x[0-9a-f]+)$', text, re.M)
    if len(online) != cpus - 1 or {int(r[0]) for r in online} != set(range(1, cpus)):
        return False
    if any(rows[int(r[0])][1] != r[1] for r in online):
        return False
    bsp = re.findall(r'^\[CPU-ID\] bsp logical=(0) apic=(\d+) tss=(0x[0-9a-f]+) ist_df=(0x[0-9a-f]+) ist_nmi=(0x[0-9a-f]+)$', text, re.M)
    if len(bsp) != 1 or bsp[0][1] != rows[0][1]:
        return False
    online += bsp
    for column in (2,3,4):
        if len({r[column] for r in online}) != cpus or any(int(r[column],16) == 0 for r in online):
            return False
    stacks = [r[col] for r in online for col in (3,4)]
    return len(stacks) == len(set(stacks))


def hardware_profile(profile, observed, cpus):
    if len(observed) != cpus or len(set(observed)) != cpus:
        return False
    return (profile == 'standard' or
            profile == 'sparse' and observed != list(range(cpus)) or
            profile == 'high' and any(apic > 63 for apic in observed) or
            profile == 'nonzero' and observed[0] != 0)


def identity():
    paths = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT
    ).decode().split('\0')
    files = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest()
             for p in paths if p and not p.startswith('.threading-evidence/')
             and (ROOT / p).is_file()}
    return {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                          text=True).strip(),
            'files': files,
            'canonical_sha256': hashlib.sha256(
                json.dumps(files, sort_keys=True).encode()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='mode', required=True)
    command = sub.add_parser('command')
    command.add_argument('name')
    command.add_argument('--timeout', type=int, default=600)
    command.add_argument('argv', nargs=argparse.REMAINDER)
    guest = sub.add_parser('guest')
    guest.add_argument('name')
    guest.add_argument('--iso', type=Path, required=True)
    guest.add_argument('--cpus', type=int, choices=range(1, 13), required=True)
    guest.add_argument('--topology', choices=('standard', 'sparse', 'nonzero', 'high'), default='standard')
    guest.add_argument('--timeout', type=int, default=180)
    guest.add_argument('--marker', action='append', required=True)
    args = parser.parse_args()
    if args.timeout <= 0 or Path(args.name).name != args.name:
        parser.error('positive timeout and plain attempt name required')
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    folder = ROOT / '.threading-evidence/t3r' / f'{stamp}-{args.name}'
    folder.mkdir(parents=True)
    (folder / 'source.json').write_text(json.dumps(identity(), indent=2) + '\n')
    record = {'pass': False, 'timeout': False}
    if args.mode == 'command':
        argv = args.argv[1:] if args.argv[:1] == ['--'] else args.argv
        if not argv:
            parser.error('command required')
    else:
        iso = args.iso.resolve()
        record['iso_sha256'] = hashlib.sha256(iso.read_bytes()).hexdigest()
        argv = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                '-smp', str(args.cpus), '-m', '2048', '-cdrom', str(iso),
                '-serial', f'file:{folder / "serial.log"}', '-display', 'none',
                '-nic', 'none', '-device', 'virtio-rng-pci,disable-modern=on',
                '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04',
                '-no-reboot', '-no-shutdown']
        if args.topology == 'sparse':
            if args.cpus not in (4, 12): parser.error('sparse profile requires 4 or 12 CPUs')
            argv[argv.index('-smp') + 1] = ('4,maxcpus=6,sockets=2,cores=3,threads=1' if args.cpus == 4 else '12,maxcpus=14,sockets=2,cores=7,threads=1')
        elif args.topology == 'nonzero':
            if args.cpus != 1: parser.error('nonzero profile requires 1 CPU')
            argv[argv.index('-cpu') + 1] = 'max,apic-id=42'
        elif args.topology == 'high':
            if args.cpus not in (2, 12): parser.error('high profile requires 2 or 12 CPUs')
            argv[argv.index('-smp') + 1] = f'{args.cpus-1},maxcpus=128,sockets=2,cores=64,threads=1'
            argv += ['-device', 'max-x86_64-cpu,socket-id=1,core-id=0,thread-id=0']
        record.update(cpus=args.cpus, markers=args.marker, topology=args.topology)
    record['command'] = argv
    print(folder, flush=True)
    started = time.monotonic()
    proc = None
    try:
        with (folder / 'output.log').open('w') as log:
            proc = subprocess.Popen(argv, cwd=ROOT, stdout=log,
                                    stderr=subprocess.STDOUT, start_new_session=True)
            deadline = started + args.timeout
            while True:
                if args.mode == 'guest':
                    serial = folder / 'serial.log'
                    text = serial.read_text(errors='replace') if serial.exists() else ''
                    if any(m in text for m in ('KERNEL PANIC', '[EXCEPTION]',
                                               'FP isolation FAILED', 'probe panicked')):
                        break
                    if assess(text, args.marker, args.cpus):
                        record['pass'] = True
                        break
                code = proc.poll()
                if code is not None:
                    if args.mode == 'command':
                        record['pass'] = code == 0
                    break
                if time.monotonic() >= deadline:
                    record['timeout'] = True
                    break
                time.sleep(0.25)
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        if proc is not None:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
            record['exit_code'] = proc.returncode
        if args.mode == 'guest':
            serial = folder / 'serial.log'
            final = serial.read_text(errors='replace') if serial.exists() else ''
            record['pass'] = not record['timeout'] and assess(final, args.marker, args.cpus)
            observed = [int(apic) for _, apic in re.findall(r'^\[CPU-ID\] logical=(\d+) apic=(\d+) bsp=(?:true|false)$', final, re.M)]
            record['observed_apic_ids'] = observed
            supported = hardware_profile(args.topology, observed, args.cpus)
            record['requested_hardware_observed'] = supported
            if not supported:
                record['pass'] = False
                record['classification'] = 'NOT_RUN: requested hardware topology not observed'
            record['serial_sha256'] = hashlib.sha256(final.encode()).hexdigest()
        record['seconds'] = time.monotonic() - started
        (folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print('PASS' if record['pass'] else 'FAIL', flush=True)
    raise SystemExit(0 if record['pass'] else 1)


if __name__ == '__main__':
    main()
