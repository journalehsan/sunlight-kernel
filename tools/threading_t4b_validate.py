#!/usr/bin/env python3
"""Durable, source-bound T4B commands and native guests; never reuse attempts."""
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
def guest_assess(text, markers, cpus):
    # A restarted service must never hide a failed synchronization attempt.
    if 'native futex wait failed' in text or '[T4B-DIAG]' in text:
        return False
    # Only the explicit worker-panic containment subprocess may panic.
    normalized=text.replace('\\n','\n')
    for match in re.finditer(r"^thread .* panicked at [^\n]*\n([^\n]*)",normalized,re.M):
        if match[1]!='native worker panic containment probe':return False
    return topology_assess(text, markers, cpus)


BASE = ROOT / '.threading-evidence/t4b'


def assess_races(text, cpus):
    marker=f'[T4B] PASS cpus={cpus} cases=11 resources=stable'
    if not topology_assess(text,[marker],cpus): return False
    rows=re.findall(r'^\[T4B\] case=(\d+) registered=(\d+) completed=(\d+) selected=(\d+) result=(\d+) dispatch0=0x([0-9a-f]+) heap=(\d+) frames=(\d+) active=0 deadlines=0 receipts=0 queues=0 pins=0 refs=3$',text,re.M)
    if [int(r[0]) for r in rows]!=list(range(11)): return False
    if len({r[7] for r in rows})!=1: return False
    for row in rows:
        case,reg,done,selected,result,mask,heap,frames=row
        case,reg,done,selected,result=map(int,(case,reg,done,selected,result))
        expected=33 if case==8 else 16 if case==9 else 1
        if reg!=expected or done!=expected: return False
        if result not in (0,2):return False
        if case==2 and selected!=int(result==0):return False
        if case not in (1,2,5,9) and result!=0:return False
        if case in (1,9) and (result!=2 or selected): return False
        if case in (0,4,7) and (result!=0 or selected!=1): return False
        if case in (3,5,6,10) and selected: return False
        if case==8 and selected!=33: return False
        if case==7 and int(mask,16)!=(3 if cpus>1 else 1): return False
        children=re.findall(rf'^\[T4B\] created case={case} role=(\d+) tid=(\d+) inc=(\d+) vm=(\d+) stack=(0x[0-9a-f]+) tls=(0x[0-9a-f]+)$',text,re.M)
        count=32 if case==8 else 3
        if len(children)!=count or {int(r[0]) for r in children}!=set(range(count)): return False
        if any(len({r[col] for r in children})!=count for col in (1,2,4,5)): return False
        identities={int(r[0]):(r[1],r[2]) for r in children}
        regs=re.findall(rf'^\[T4B\] registered case={case} role=(\d+) seq=(\d+) tid=(\d+) inc=(\d+)$',text,re.M)
        terms=re.findall(rf'^\[T4B\] terminal case={case} role=(\d+) seq=(\d+) tid=(\d+) inc=(\d+) result=(\d+)$',text,re.M)
        expected_keys={(r,seq) for r in (list(range(32)) if case==8 else [0]) for seq in range(1,17 if case==9 else 2)}
        if case==8:
            retries=[int(r[0]) for r in regs if r[1]=='2']
            if len(retries)!=1:return False
            expected_keys.add((retries[0],2))
        if len(regs)!=expected or len(terms)!=expected: return False
        for events in (regs,terms):
            if {(int(r[0]),int(r[1])) for r in events}!=expected_keys: return False
            if any((r[2],r[3])!=identities[int(r[0])] for r in events): return False
        if any(int(r[4]) not in (0,2) for r in terms): return False
        if case!=8 and any(int(r[4])!=result for r in terms):return False
        if case==8 and any(r[4]!='0' for r in terms):return False
        if case in (1,9) and any(r[4]!='2' for r in terms): return False
        progress=re.findall(rf'^\[T4B\] progress case={case} ticks=(\d+) userspace_selected=(\d+)$',text,re.M)
        if len(progress)!=1 or int(progress[0][0])<=0 or int(progress[0][1])!=(32 if case==8 else 0): return False
    contender=re.findall(r'^\[T4B\] contender role=(\d+) tid=(\d+) inc=(\d+) worker_cpu=(\d+) owner_cpu=0$',text,re.M)
    if len(contender)!=1 or int(contender[0][3])!=int(cpus>1):return False
    case8=re.findall(r'^\[T4B\] created case=8 role=(\d+) tid=(\d+) inc=(\d+) vm=',text,re.M)
    if tuple(contender[0][:3]) not in case8:return False
    calls=re.findall(r'^\[T4B\] wake case=8 role=(\d+) limit=(\d+) selected=(\d+)$',text,re.M)
    if len(calls)!=16 or sum(int(r[2]) for r in calls)!=30: return False
    roles={r[0] for r in calls}
    if roles!={'64',contender[0][0]}:return False
    for role in roles:
        actor=[r for r in calls if r[0]==role]
        if [int(r[1]) for r in actor]!=[0,1,4,1,4,1,32,1]:return False
        if any(int(r[2])>int(r[1]) for r in actor):return False
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
                if iso and (assess_races(text, cpus) if races else guest_assess(text, markers, cpus)):
                    record['pass_'] = True
                    break
                if iso and ('KERNEL PANIC' in text or '[EXCEPTION]' in text or 'THREAD_PROBE FAIL' in text or 'native futex wait failed' in text or '[T4B-DIAG]' in text):
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
            record['pass_'] = not record['timeout'] and (assess_races(text, cpus) if races else guest_assess(text, markers, cpus))
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
