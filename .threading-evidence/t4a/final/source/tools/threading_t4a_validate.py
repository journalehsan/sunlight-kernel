#!/usr/bin/env python3
"""Durable, source-bound T4A commands and native guests; never reuse attempts."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
from threading_t3r_validate import assess as topology_assess, identity

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / '.threading-evidence/t4a'


def assess_races(text, cpus):
    marker = f'[T4A] PASS cpus={cpus} cases=34 waiter_records=0 resources=stable'
    if not topology_assess(text, [marker], cpus):
        return False
    if text.count("[T4A] unsupported SHM aliases=2 rejected=true backing_released=true\n") != 1:
        return False
    rows = re.findall(r'^\[T4A\] case=(\d+) registered=(\d+) completed=(\d+) wakes=(\d+) live_wakes=(\d+) dispatch0=0x([0-9a-f]+) heap=(\d+) frames=(\d+) stale=0 pins=0 queues=0 ownership=ok$', text, re.M)
    if [int(r[0]) for r in rows] != list(range(34)):
        return False
    for case, reg, done, wakes, live, mask, heap, frames in rows:
        case, reg, done, wakes, live = map(int, (case, reg, done, wakes, live))
        epoch=case
        case %= 17
        expected = 0 if case in (0, 1) or (case == 8 and epoch >= 17) else 2 if case == 7 else 128 if case == 14 else 32 if case >= 15 else 1
        if reg != expected or done != expected:
            return False
        if case == 5 and (wakes != 1 or live != 1):
            return False
        if case == 14 and (int(mask, 16) != 3 or wakes != 128):
            return False
    if len({r[7] for r in rows}) != 1:
        return False
    ordering = re.findall(r'^\[T4A\] ordering case=(\d+) point=(\d+) waiter_cpu=(\d+) waker_cpu=(\d+) entered=(\d+) completed=(\d+)$', text, re.M)
    if [(int(r[0]),int(r[1])) for r in ordering] != [(offset+c,c-1) for offset in (0,17) for c in range(1,6)]:
        return False
    if any((int(r[2]),int(r[3]),int(r[4]),int(r[5])) != (0,1,1,1) for r in ordering):
        return False
    cardinal = re.findall(r'^\[T4A\] cardinality case=(\d+) step=(\d+) limit=(\d+) actual=(\d+) selected_total=(\d+) resumed=(\d+) remaining=(\d+) unique=true$', text, re.M)
    expected = []
    for case, broadcast in ((15,2**32-1),(32,2**31-1)):
        for step,(limit,actual,total) in enumerate(((0,0,0),(1,1,1),(4,4,5),(broadcast,27,32),(1,0,32))):
            expected.append((case,step,limit,actual,total,32-total))
    if len(cardinal) != len(expected):
        return False
    for row,want in zip(cardinal,expected):
        case,step,limit,actual,total,resumed,remaining = map(int,row)
        if (case,step,limit,actual,total,remaining) != want or resumed > total:
            return False
    for case in range(34):
        children = re.findall(rf'^\[T4A\] created case={case} role=(\d+) tid=(\d+) inc=(\d+) stack=(0x[0-9a-f]+) tls=(0x[0-9a-f]+)$',text,re.M)
        count = 32 if case%17>=15 else 3 if case%17==7 else 2
        if len(children)!=count or {int(r[0]) for r in children} != (set(range(32)) if count==32 else {0,1,2} if count==3 else {0,1}):
            return False
        if any(len({r[col] for r in children}) != count for col in (1,2,3,4)):
            return False
    return True


def run(name, argv, timeout=600, iso=None, cpus=None, markers=(), races=False):
    if Path(name).name != name or timeout <= 0:
        raise ValueError('plain attempt name and positive timeout required')
    folder = BASE / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ') + '-' + name)
    folder.mkdir(parents=True)
    source = identity()
    (folder / 'source.json').write_text(json.dumps(source, indent=2) + '\n')
    record = dict(pass_=False, timeout=False)
    if iso:
        iso = Path(iso).resolve()
        record.update(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(), cpus=cpus, markers=markers)
        argv = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max', '-smp', str(cpus), '-m', '2048',
                '-cdrom', str(iso), '-serial', f'file:{folder / "serial.log"}', '-display', 'none', '-nic', 'none',
                '-device', 'virtio-rng-pci,disable-modern=on', '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04',
                '-no-reboot', '-no-shutdown']
    record['command'] = argv
    print(folder, flush=True)
    started = time.monotonic()
    proc = None
    try:
        with (folder / 'output.log').open('w') as log:
            proc = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            while True:
                serial = folder / 'serial.log'
                text = serial.read_text(errors='replace') if serial.exists() else ''
                if iso and (assess_races(text, cpus) if races else topology_assess(text, markers, cpus)):
                    record['pass_'] = True
                    break
                if iso and ('KERNEL PANIC' in text or '[EXCEPTION]' in text or 'THREAD_PROBE FAIL' in text):
                    break
                if proc.poll() is not None:
                    record['pass_'] = not iso and proc.returncode == 0
                    break
                if time.monotonic() - started >= timeout:
                    record['timeout'] = True
                    break
                time.sleep(.25)
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        if proc:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
            record['exit_code'] = proc.returncode
        if iso:
            serial = folder / 'serial.log'
            text = serial.read_text(errors='replace') if serial.exists() else ''
            record['pass_'] = not record['timeout'] and (assess_races(text, cpus) if races else topology_assess(text, markers, cpus))
            record['serial_sha256'] = hashlib.sha256(text.encode()).hexdigest()
        record['source_unchanged'] = identity() == source
        record['pass'] = record.pop('pass_') and record['source_unchanged']
        record['seconds'] = time.monotonic() - started
        (folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print('PASS' if record['pass'] else 'FAIL', flush=True)
    return record['pass'], folder


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='mode', required=True)
    command = sub.add_parser('command')
    command.add_argument('--timeout', type=int, default=600)
    command.add_argument('name')
    command.add_argument('argv', nargs=argparse.REMAINDER)
    guest = sub.add_parser('guest')
    guest.add_argument('name')
    guest.add_argument('--iso', required=True)
    guest.add_argument('--cpus', type=int, choices=(1, 2, 4, 12), required=True)
    guest.add_argument('--timeout', type=int, default=180)
    guest.add_argument('--marker', action='append', default=[])
    guest.add_argument('--races', action='store_true')
    a = p.parse_args()
    if a.mode == 'command':
        argv = a.argv[1:] if a.argv[:1] == ['--'] else a.argv
        if not argv:
            p.error('command required')
        ok, _ = run(a.name, argv, a.timeout)
    else:
        if not a.races and not a.marker:
            p.error('guest needs --races or --marker')
        ok, _ = run(a.name, [], a.timeout, a.iso, a.cpus, a.marker, a.races)
    raise SystemExit(0 if ok else 1)


if __name__ == '__main__':
    main()
