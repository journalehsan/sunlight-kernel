#!/usr/bin/env python3
"""Bounded T1 commands/guest gates with fresh, durable attempt directories.

Examples:
  python3 tools/threading_t1_validate.py command host -- cargo test ...
  python3 tools/threading_t1_validate.py guest pilot --iso image.iso --cpus 1 \
      --marker '[T1] PASS'
Every attempt records source hashes, exact argv, logs and a verdict, including
exceptions/timeouts. No existing attempt is overwritten. TCG is functional proof.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]


def assess(text, markers):
    if not markers or any(not m for m in markers):
        return False
    if any(m in text for m in ('KERNEL PANIC', '[EXCEPTION]', 'THREAD_PROBE FAIL',
                               'FP isolation FAILED', 'probe panicked', 'parser/raster FAILED')):
        return False
    return all(text.count(m) == 1 and '\n' in text.split(m, 1)[1] for m in markers)


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
    guest.add_argument('--timeout', type=int, default=180)
    guest.add_argument('--marker', action='append', required=True)
    args = parser.parse_args()
    if args.timeout <= 0 or Path(args.name).name != args.name:
        parser.error('positive timeout and plain attempt name required')
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    folder = ROOT / '.threading-evidence/t1' / f'{stamp}-{args.name}'
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
        record.update(cpus=args.cpus, markers=args.marker)
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
                    if assess(text, args.marker):
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
            record['pass'] = not record['timeout'] and assess(final, args.marker)
            record['serial_sha256'] = hashlib.sha256(final.encode()).hexdigest()
        record['seconds'] = time.monotonic() - started
        (folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print('PASS' if record['pass'] else 'FAIL', flush=True)
    raise SystemExit(0 if record['pass'] else 1)


if __name__ == '__main__':
    main()
