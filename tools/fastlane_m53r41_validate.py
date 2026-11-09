#!/usr/bin/env python3
"""Build and run the scheduled Account launch probe; retain incomplete gates."""
import argparse
from datetime import datetime, timezone
import json
import mmap
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from fastlane_f11b0e_validate import ROOT, digest, sources
from fastlane_f11b0c_validate import SERVICE_FLAGS
from fastlane_m53_quota_validate import stop


def assess(raw):
    errors = []
    for stage in ('entry', 'foreign-authority-denied', 'thread', 'child', 'exec', 'exec-live-thread-denied', 'process-exit-live-thread-reaped', 'suspended-creation-denied', 'resumed', 'dropped-credentials-child', 'dropped-credentials-parent', 'linux-clone-complete', 'child-image-rollback', 'complete'):
        if f'[SL41-USER] stage={stage}' not in raw:
            errors.append('missing scheduled ' + stage)
    rows = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines()
            if line.startswith('[SL41] event=userspace')]
    if not rows or any(r.get('live') != '1' for r in rows):
        errors.append('missing live registry observations')
    if len({(r.get('session'), r.get('epoch')) for r in rows}) != 1:
        errors.append('membership differs across actors')
    if '[SL41] event=prepublication' not in raw:
        errors.append('missing prepublication receipt')
    publications = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines()
                    if line.startswith('[SL41] event=prepublication')]
    for event in ('stale-generation-denied', 'bootstrap-replay-denied', 'retirement-fenced'):
        if raw.count('[SL41] event=' + event) != 1:
            errors.append('missing unique ' + event)
    if len(publications) != 2:
        errors.append('missing unique initial publication')
    if publications:
        initial = publications[0]
        if not any(all(r.get(k) == initial.get(k) for k in ('pid', 'incarnation', 'vm', 'session', 'epoch')) for r in rows):
            errors.append('first execution differs from publication identity')
        borrowers = [r for r in rows if r.get('pid') != r.get('owner')]
        if not borrowers or any(not any(leader.get('pid') == r.get('owner') and leader.get('incarnation') == r.get('owner_incarnation') and leader.get('vm') == r.get('vm') for leader in rows) for r in borrowers):
            errors.append('thread identity does not share initial process and VM')
    linux_threads = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines() if line.startswith('[SL41] event=linux-thread')]
    if not publications or not linux_threads or raw.count('THREAD_PROBE PASS') != 1 or any(
            r.get('pid') == r.get('owner') or (r.get('session'), r.get('epoch')) !=
            (publications[0].get('session'), publications[0].get('epoch')) for r in linux_threads):
        errors.append('missing Account-bound Linux clone qualification')
    # Exec must preserve a PID/incarnation while replacing the VM.
    identities = {}
    for row in rows:
        identities.setdefault((row.get('pid'), row.get('incarnation')), set()).add(row.get('vm'))
    if len(identities) != 8:
        errors.append('missing distinct initial, thread and child incarnations')
    if not any(len(vms) == 2 and (not publications or identity != (publications[0].get('pid'), publications[0].get('incarnation'))) for identity, vms in identities.items()):
        errors.append('missing exec VM replacement on same process incarnation')
    if not any(r.get('pid') != r.get('owner') for r in rows):
        errors.append('missing thread process quota owner')
    bad_images = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines() if line.startswith('[SL41] event=child-image-rollback')]
    if len(bad_images) != 1 or any(r.get('free_before') != r.get('free_after') for r in bad_images):
        errors.append('missing invalid ELF child rollback')
    rollback = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines() if line.startswith('[SL41] event=rollback')]
    if sorted(r.get('stage') for r in rollback) != ['1', '2', '3', '4'] or any(
            r.get('free_before') != r.get('free_after') or r.get('published') != '0' or r.get('runnable') != '0' for r in rollback):
        errors.append('missing exact per-stage rollback frame restoration')
    if any(marker in raw for marker in ('PANIC', 'panicked at', 'UNEXPECTED', 'checkpoint=FAIL', 'THREAD_PROBE FAIL')):
        errors.append('guest failure')
    return {'status': 'FAIL' if errors else 'PASS', 'errors': errors,
            'scope': 'scheduled initial launch, native child/thread, exec, foreign-authority denial',
            'rollback': 'PASS' if len(rollback) == 4 and not any('rollback' in e for e in errors) else 'FAIL', 'quota_prerequisites': 'NOT_RUN'}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence-root', type=Path, required=True)
    p.add_argument('--cpus', type=int, nargs='+', choices=(1, 2, 4, 12), default=[1, 2, 4, 12])
    p.add_argument('--timeout', type=int, default=180)
    args = p.parse_args()
    out = args.evidence_root.resolve() / ('native-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ'))
    out.mkdir(parents=True)
    result = {'status': 'RUNNING', 'source': sources(), 'checks': [], 'native': [],
              'production_fastlane': 'DISABLED', 'overall_verdict': 'BLOCKED'}
    def save():
        (out / 'validation-summary.json').write_text(json.dumps(result, indent=2) + '\n')
    def check(name, command, extra=None):
        env = {**os.environ, **(extra or {})}
        with (out / (name + '.log')).open('x') as log:
            r = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
        result['checks'].append({'name': name, 'command': command, 'environment': extra,
                                 'exit_code': r.returncode})
        save()
        print(name, r.returncode, flush=True)
        if r.returncode:
            raise RuntimeError(name + ' failed')
    save()
    try:
        services = {'sunlight-init': 'sunlight-init', 'sunlightd': 'sunlightd',
                    'sunlight-tty-server': 'sunlight-tty-server', 'sunlight-uac': 'uac_service',
                    'sunlight-sessiond': 'sunlight-sessiond', 'sunlight-vfs-server': 'sunlight-vfs-server'}
        for package in services:
            extra = {'RUSTFLAGS': SERVICE_FLAGS}
            if package == 'sunlight-sessiond':
                extra['SUNLIGHT_INJECT_PHASE'] = 'm53r41'
            check(package, ['cargo', 'build', '--locked', '--offline', '--release', '-p', package], extra)
        check('probe', ['cargo', 'build', '--locked', '--offline', '--release', '-p', 'sunlight-uac',
                        '--bin', 'session-race-probe', '--features', 'session-launch-test'], {'RUSTFLAGS': SERVICE_FLAGS})
        archive = out / 'embedded'
        archive.mkdir()
        payload_names = list(services.values()) + ['session-race-probe', 'linux-thread-probe']
        def payload_path(name):
            return ROOT / ('target/helios-probes' if name == 'linux-thread-probe' else 'target/x86_64-unknown-none/release') / name
        for name in payload_names:
            shutil.copy2(payload_path(name), archive / name)
        result['embedded'] = {name: digest(archive / name) for name in payload_names}
        check('kernel', ['cargo', 'build', '--locked', '--offline', '-p', 'sunlight-kernel',
                         '--bin', 'sunlight-kernel', '--features', 'key_inject,session_launch_test'],
              {'SUNLIGHT_INJECT_PHASE': 'desktop_login'})
        elf, iso = out / 'kernel.elf', out / 'fixture.iso'
        check('strip', ['objcopy', '--strip-debug', str(ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel'), str(elf)])
        scratch = out / 'iso-build'
        scratch.mkdir()
        shutil.copy2(ROOT / 'limine.conf', scratch / 'limine.conf')
        check('iso', ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso), str(ROOT / 'target/limine'), str(scratch)])
        with elf.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as raw:
            for name in payload_names:
                if digest(payload_path(name)) != result['embedded'][name] or raw.find((archive / name).read_bytes()) < 0:
                    raise RuntimeError('embedded payload changed or not found: ' + name)
        result['image'] = {'elf': str(elf), 'iso': str(iso), 'elf_sha256': digest(elf), 'iso_sha256': digest(iso)}
        for cpus in args.cpus:
            serial = out / f'{cpus}cpu.serial.log'
            serial.touch(exist_ok=False)
            cmd = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max', '-smp', str(cpus),
                   '-m', '2048', '-cdrom', str(iso), '-serial', 'file:' + str(serial), '-display', 'none',
                   '-nic', 'none', '-no-reboot', '-no-shutdown', '-device', 'virtio-rng-pci,disable-modern=on']
            start = time.monotonic()
            with (out / f'{cpus}cpu.qemu.log').open('x') as log:
                proc = subprocess.Popen(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    while time.monotonic() - start < args.timeout:
                        raw = serial.read_text(errors='replace')
                        if ('[SL41-USER] stage=complete' in raw and '[SL41] event=retirement-fenced\n' in raw) or 'PANIC' in raw or 'panicked at' in raw or 'checkpoint=FAIL' in raw or proc.poll() is not None:
                            break
                        time.sleep(.1)
                finally:
                    stop(proc)
            row = {'cpus': cpus, 'command': cmd, 'elapsed': time.monotonic() - start,
                   'serial_sha256': digest(serial), 'assessment': assess(serial.read_text(errors='replace'))}
            result['native'].append(row)
            save()
            print(cpus, row['assessment'], flush=True)
        result['status'] = 'PASS' if all(r['assessment']['status'] == 'PASS' for r in result['native']) else 'FAIL'
    except Exception as error:
        result.update(status='FAIL', failure=str(error))
    finally:
        result['source_matched_after'] = sources() == result['source']
        if not result['source_matched_after']:
            result['status'] = 'UNQUALIFIED'
        save()
    print(out, result['status'], flush=True)
    return 0 if result['status'] == 'PASS' else 1

if __name__ == '__main__':
    raise SystemExit(main())
