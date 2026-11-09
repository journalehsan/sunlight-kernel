#!/usr/bin/env python3
"""Bounded, source-matched M5.3 quota kernel prerequisite evidence.

This runner never certifies scheduled process exit/kill/crash/exec or ring-3
mapping authority. Incomplete, duplicate or stale receipts fail assessment.
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
import uuid

from fastlane_f11b0e_validate import ROOT, digest, sources


def source_id(snapshot):
    return hashlib.sha256(json.dumps(snapshot, sort_keys=True).encode()).hexdigest()


def records(raw):
    found = []
    for line in raw.splitlines():
        if line.startswith('[SHM-M53] '):
            pairs = [word.split('=', 1) for word in line.split()[1:] if '=' in word]
            row = dict(pairs)
            if len(row) != len(pairs):
                raise ValueError('duplicate receipt field')
            found.append(row)
    return found


def assess(raw, cpus, source, run):
    errors = []
    try:
        rows = records(raw)
        events = [r.get('event') for r in rows]
        boundary_events = ['physical-boundary', 'small-aggregate'] if cpus == 1 else []
        if events != ['start', 'rollback', 'ledger-boundary'] + boundary_events + ['cycle'] * 16 + ['remote', 'complete']:
            raise ValueError('missing, duplicate or unordered receipts')
        for row in (rows[0], rows[-1]):
            if row['source'] != source or row['run'] != run or int(row['cpus']) != cpus:
                raise ValueError('stale source, invocation or CPU count')
        if rows[0]['scope'] != 'kernel-prerequisite':
            raise ValueError('wrong evidence scope')
        rollback = rows[1]
        if any(int(rollback[k]) != 0 for k in ('quota_before', 'quota_after', 'objects')):
            raise ValueError('rollback accounting')
        if rollback['free_before'] != rollback['free_after'] or rollback['partial_frames'] != '1':
            raise ValueError('rollback frame drift or missing backing failure')
        if rollback['overflow'] != 'denied' or rollback['over_budget'] != 'denied':
            raise ValueError('missing quota denials')
        if rows[2] != {'event': 'ledger-boundary', 'process_limit': str(512*1024*1024),
                       'quota_after': '0', 'scope': 'reservation-only'}:
            raise ValueError('missing reservation boundary')
        if cpus == 1:
            for row in rows[3:5]:
                if any(int(row[k]) != v for k,v in {'bytes':512*1024*1024, 'frames_allocated':131072,
                       'frames_released':131072, 'quota_after':0}.items()):
                    raise ValueError('physical quota boundary accounting')
                if row['over_budget'] != 'denied' or row['free_before'] != row['free_after']:
                    raise ValueError('physical quota boundary leak or missing denial')
            if int(rows[3]['quota_live']) != 512*1024*1024 or rows[4]['allocations'] != '64' or int(rows[4]['chunk']) != 8388608:
                raise ValueError('missing physical aggregate boundary')
        cycle_start = 5 if cpus == 1 else 3
        for index, row in enumerate(rows[cycle_start:cycle_start+16]):
            expected = {'cycle': index, 'quota_before': 0, 'quota_live': 8192,
                        'quota_owner_unmapped': 8192, 'quota_after': 0,
                        'frames_allocated': 2, 'frames_released': 2, 'views_retired': 2, 'objects': 0}
            if any(int(row[k]) != v for k, v in expected.items()):
                raise ValueError('cycle accounting')
            if row['free_before'] != row['free_after']:
                raise ValueError('cycle frame drift')
            if any(int(row[k]) <= 0 for k in ('pid','incarnation','vm','peer_pid','peer_incarnation','peer_vm')):
                raise ValueError('invalid lifecycle identity')
            if any(row[k] != rollback[k] for k in ('pid','incarnation','vm')):
                raise ValueError('owner changed during cycle')
            if any(row[k] != rows[cycle_start][k] for k in ('peer_pid','peer_incarnation','peer_vm')):
                raise ValueError('peer changed during cycle')
        remote = rows[-2]
        if int(remote['acknowledgements']) != cpus - 1 or int(remote['expected']) != cpus - 1:
            raise ValueError('missing remote acknowledgements')
        if cpus > 1 and (remote['scope'] != 'kernel-shm-prerequisite' or remote['free_before'] != remote['free_after']):
            raise ValueError('remote reclamation evidence')
        remote_rows = []
        for line in raw.splitlines():
            if line.startswith('[SHM-M53-REMOTE] '):
                pairs = [word.split('=', 1) for word in line.split()[1:] if '=' in word]
                record = dict(pairs)
                if len(record) != len(pairs):
                    raise ValueError('duplicate remote field')
                remote_rows.append(record)
        if cpus > 1:
            if [r.get('event') for r in remote_rows] != ['established', 'complete']:
                raise ValueError('missing remote VM and physical identity receipts')
            if not (raw.index('[SHM-M53-REMOTE] event=established ') <
                    raw.index('[SHM-M53-REMOTE] event=complete ') <
                    raw.index('[SHM-M53] event=remote ') <
                    raw.index('[SHM-M53] event=complete ')):
                raise ValueError('remote acknowledgements out of order')
            established, completed = remote_rows
            if any(int(established[k]) <= 0 for k in ('pid','incarnation','vm','root','frame')):
                raise ValueError('invalid remote identity')
            if any(completed[k] != established[k] for k in ('pid','incarnation','vm','frame')):
                raise ValueError('remote identity mismatch')
            if int(established['targets']) != (((1 << cpus)-1) & ~1):
                raise ValueError('remote target mask mismatch')
            if (completed['frame'] != completed['reused_after_ack'] or
                int(completed['acknowledgements']) != cpus-1 or
                completed['free_before'] != completed['free_after'] or
                any(completed[k] != v for k,v in {'quota_after':'0','views_retired':'1','frames_released':'1'}.items())):
                raise ValueError('remote release/reuse mismatch')
        elif remote_rows:
            raise ValueError('remote receipt without remote CPU')
        end = rows[-1]
        if end['cycles'] != '16' or end['quota_after'] != '0' or end['free_before'] != end['free_after']:
            raise ValueError('final reclamation drift')
        if any(marker in raw for marker in ('PANIC', 'panicked at', 'checkpoint=FAIL', '[FAULT]')):
            raise ValueError('guest failure')
    except (ValueError, KeyError, TypeError) as error:
        errors.append(str(error))
    return {'status': 'FAIL' if errors else 'PASS', 'scope': 'kernel-prerequisite-only',
            'errors': errors, 'receipts': rows if 'rows' in locals() else [],
            'scheduled_lifecycle': 'NOT_RUN', 'restricted_userspace_authorization': 'NOT_RUN'}


def stop(proc):
    if proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait(timeout=5)


def command(out, label, cmd, env=None, deadline=300):
    start = time.monotonic()
    reason = 'exited'
    with (out / (label + '.log')).open('x') as log:
        proc = subprocess.Popen(cmd, cwd=ROOT, env={**os.environ, **(env or {})},
                                stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            proc.wait(timeout=deadline)
        except subprocess.TimeoutExpired:
            reason = 'deadline'
            stop(proc)
    result = {'command': cmd, 'environment': env or {}, 'exit_code': proc.returncode,
              'timeout_seconds': deadline, 'reason': reason, 'elapsed_seconds': time.monotonic()-start}
    (out / (label + '.json')).write_text(json.dumps(result, indent=2)+'\n')
    if proc.returncode or reason != 'exited':
        raise RuntimeError(label + ': ' + reason + ', exit=' + str(proc.returncode))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--section', required=True, choices=('build', 'native'))
    parser.add_argument('--evidence-root', required=True, type=Path)
    parser.add_argument('--image-manifest', type=Path)
    parser.add_argument('--cpus', nargs='+', type=int, choices=(1,2,4,12), default=[1])
    parser.add_argument('--timeout', type=int, default=90)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 600:
        parser.error('timeout must be 1..600 seconds')
    out = args.evidence_root.resolve() / (args.section+'-'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True)
    snapshot = sources()
    identity = source_id(snapshot)
    summary = {'status': 'RUNNING', 'scope': 'kernel-prerequisite-only',
               'revision': subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
               'source_sha256': snapshot, 'source_id': identity, 'native': []}
    (out/'worktree.patch').write_bytes(subprocess.check_output(['git','diff','HEAD'],cwd=ROOT))
    (out/'working-tree.txt').write_bytes(subprocess.check_output(['git','status','--short'],cwd=ROOT))
    def save():
        (out/'validation-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    save()
    print(out, flush=True)
    try:
        if args.section == 'build':
            run = uuid.uuid4().hex
            build = ROOT/'target/fastlane-f11b0e3-build'
            env = {'CARGO_TARGET_DIR': str(build), 'CARGO_INCREMENTAL':'0', 'CARGO_PROFILE_DEV_DEBUG':'0',
                   'SUNLIGHT_M53_SOURCE': identity, 'SUNLIGHT_M53_RUN': run}
            command(out, 'kernel', ['cargo','build','--locked','--offline','-p','sunlight-kernel',
                                    '--bin','sunlight-kernel','--features','shared_quota_test'], env, 300)
            elf, iso = out/'kernel.elf', out/'fixture.iso'
            command(out, 'strip', ['objcopy','--strip-debug',str(build/'x86_64-unknown-none/debug/sunlight-kernel'),str(elf)])
            scratch = out/'iso-build'
            scratch.mkdir()
            (scratch/'limine.conf').write_bytes((ROOT/'limine.conf').read_bytes())
            command(out, 'iso', ['bash','tools/make_hybrid_iso.sh',str(elf),str(iso),str(ROOT/'target/limine'),str(scratch)])
            command(out, 'extract', ['xorriso','-osirrox','on','-indev',str(iso),'-extract','/boot/sunlight-kernel.elf',str(out/'iso-kernel.elf')])
            if digest(elf) != digest(out/'iso-kernel.elf'):
                raise RuntimeError('ISO ELF mismatch')
            summary['image'] = {'elf':str(elf),'iso':str(iso),'elf_sha256':digest(elf),
                                'iso_sha256':digest(iso),'source_id':identity,'run':run,
                                'features':['shared_quota_test']}
        else:
            if not args.image_manifest:
                raise RuntimeError('native requires fresh image manifest')
            manifest = json.loads(args.image_manifest.read_text())
            image = manifest['image']
            if manifest['status'] != 'PASS' or manifest['source_sha256'] != snapshot:
                raise RuntimeError('build failed or current source differs from tested image')
            if digest(Path(image['iso'])) != image['iso_sha256']:
                raise RuntimeError('image digest mismatch')
            summary['image'] = image
            for cpus in args.cpus:
                serial = out/f'{cpus}cpu.serial.log'
                serial.touch(exist_ok=False)
                cmd = ['qemu-system-x86_64','-accel','tcg','-cpu','max','-m','1536','-smp',str(cpus),
                       '-cdrom',image['iso'],'-display','none','-serial','file:'+str(serial),
                       '-nic','none','-no-reboot','-no-shutdown']
                row = {'cpus':cpus,'command':cmd,'timeout_seconds':args.timeout}
                summary['native'].append(row); save()
                start = time.monotonic()
                with (out/f'{cpus}cpu.qemu.log').open('x') as log:
                    proc = subprocess.Popen(cmd,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                    reason = 'deadline'
                    try:
                        while time.monotonic()-start < args.timeout:
                            raw = serial.read_text(errors='replace')
                            if '[SHM-M53] event=complete ' in raw:
                                reason = 'terminal-receipt'; break
                            if any(s in raw for s in ('PANIC','panicked at','[FAULT]')):
                                reason = 'guest-failure'; break
                            if proc.poll() is not None:
                                reason = 'qemu-exit'; break
                            time.sleep(.1)
                    finally:
                        stop(proc)
                raw = serial.read_text(errors='replace')
                row.update(exit_code=proc.returncode,reason=reason,elapsed_seconds=time.monotonic()-start,
                           serial_sha256=digest(serial), serial_bytes=serial.stat().st_size)
                row['assessment'] = assess(raw,cpus,identity,image['run'])
                if reason != 'terminal-receipt' or row['assessment']['status'] != 'PASS':
                    row['classification'] = (
                        'NO_SERIAL' if not raw else
                        'GUEST_TEST_FAILURE' if reason == 'guest-failure' else
                        'BOOT_OR_FIXTURE_ENTRY_FAILURE' if '[SHM-M53] event=start ' not in raw else
                        'INCOMPLETE_GUEST_TEST' if reason == 'deadline' else
                        'EVIDENCE_COLLECTION_FAILURE')
                    save()
                    raise RuntimeError('bounded smoke/prerequisite failed; stop matrix: '+str(cpus))
                row['classification'] = 'KERNEL_PREREQUISITE_PASS'
                save()
        if sources() != snapshot:
            raise RuntimeError('source changed during validation')
        summary['status'] = 'PASS'
    except Exception as error:
        summary['status'] = 'BLOCKED'
        summary['error'] = type(error).__name__+': '+str(error)
    finally:
        save()
    print(summary['status'], summary.get('error',''),flush=True)
    return 0 if summary['status']=='PASS' else 1

if __name__ == '__main__':
    raise SystemExit(main())
