#!/usr/bin/env python3
"""Retain fresh deferred-IPC prerequisite evidence; never enables FastLane."""
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

BASE = ROOT / 'target/fastlane-native-boundary-evidence'
BUILD = ROOT / 'target/fastlane-f11b0e3-build'
ENV = {'CARGO_TARGET_DIR': str(BUILD), 'CARGO_INCREMENTAL': '0', 'CARGO_PROFILE_DEV_DEBUG': '0'}
SERVICES = {
    'sunlight-init': 'sunlight-init', 'sunlightd': 'sunlightd',
    'sunlight-tty-server': 'sunlight-tty-server', 'sunlight-uac': 'uac_service',
    'sunlight-sessiond': 'sunlight-sessiond', 'sunlight-vfs-server': 'sunlight-vfs-server',
    'sunlightctl': 'sunlightctl',
}
CASES = ('invalid-request-cap', 'owner-isolation-cancel', 'stale-token-reply-first',
         'close-first', 'stale-incarnation', 'exiting-caller', 'capacity-rollback',
         'exhaustion-rollback', 'call-generation-exhaustion')


def required(cpus):
    markers = [line for name in ('phase2_6.expected', 'phase_sec.expected', 'phase_shm.expected')
               for line in (ROOT / 'tools/tests' / name).read_text().splitlines()
               if line and not line.startswith(('#', '[SMP]'))]
    return list(dict.fromkeys(markers)) + [
        f'[SMP] Phase-1 bring-up complete: {cpus - 1} AP(s) online ({cpus} total logical CPUs)',
        '[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)',
        f'[FASTLANE-F1] remote SHM release: cpus={cpus} acknowledgements={cpus - 1} OK',
        '[SunlightOS] sunlightd OK',
    ]


def assess(raw, cpus):
    errors = [f'missing marker: {m}' for m in required(cpus) if m not in raw]
    lines = [line for line in raw.splitlines() if line.startswith('[IPC-DEFER] ')]
    rows = [dict(re.findall(r'(\w+)=([^\s]+)', line)) for line in lines]
    for case in CASES:
        matched = [r for r in rows if r.get('case') == case]
        if len(matched) != 1 or matched[0].get('result') != 'PASS':
            errors.append('missing/duplicate/failed case: ' + case)
    finals = [r for line, r in zip(lines, rows) if line.startswith('[IPC-DEFER] PASS ')]
    if len(finals) != 1:
        errors.append('one final fixture receipt required')
    elif any(finals[0].get(k) != v for k, v in {
            'cases': '9', 'pending': '0', 'deferred': '0', 'queued': '0',
            'scope': 'boot-method', 'concurrent_race': 'NOT_RUN'}.items()):
        errors.append('incorrect final fixture scope or inventory')
    elif not all(finals[0].get(k, '').isdigit() and int(finals[0][k]) > 0
                 for k in ('caller_inc', 'caller_vm')):
        errors.append('missing exact caller identity')
    if len(lines) != 10:
        errors.append('unexpected fixture records')
    if any(m in raw for m in ('PANIC', 'panicked at', '[FAULT]', 'UNEXPECTED',
                             '[D1-TEST]', '[D2-TEST]', '[D3-TEST]', '[D36R-TEST]',
                             '[SESSION-E3]', '[REAP-TEST]')):
        errors.append('fault or unrelated test feature contamination')
    return {'status': 'FAIL' if errors else 'PASS', 'errors': errors,
            'scope': 'native boot-method prerequisite; concurrent userspace races NOT_RUN'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--section', choices=('build', 'native', 'production', 'checks'), required=True)
    parser.add_argument('--cpus', nargs='+', type=int, choices=(2, 4, 12), default=[2])
    parser.add_argument('--image-manifest', type=Path)
    parser.add_argument('--reply-race', action='store_true', help='Additional SMP kernel-method fixture; no Account authentication claim')
    parser.add_argument('--evidence-root', type=Path, default=BASE)
    args = parser.parse_args()
    if args.section == 'native' and not args.image_manifest:
        parser.error('--image-manifest is required')
    out = args.evidence_root.resolve() / (args.section + '-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True)
    summary = {'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
               'source_sha256': sources(), 'checks': [], 'native': [],
               'production_fastlane': 'BLOCKED', 'performance': {'status': 'BLOCKED', 'samples': 0},
               'status': 'NOT_RUN'}
    features = ['ipc_reply_race_test' if args.reply_race else 'ipc_deferred_test', 'fastlane_f1_audit']
    def assessment(raw, cpus):
        result = assess(raw, cpus)
        if args.reply_race:
            from fastlane_reply_race_validate import assess as race_assess
            race = race_assess(raw, cpus)
            result['errors'] += race['errors']
            result['race'] = race
            result['status'] = 'FAIL' if result['errors'] else 'PASS'
            result['scope'] = race['scope']
        return result

    def save():
        (out / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')

    def check(name, command, extra=None, timeout=1200):
        start = time.monotonic()
        with (out / (name + '.log')).open('w') as log:
            log.write(json.dumps({'command': command, 'environment': {**ENV, **(extra or {})}}) + '\n')
            log.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, env={**os.environ, **ENV, **(extra or {})},
                                      stdout=log, stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary['checks'].append({'name': name, 'command': command, 'exit_code': code,
                                  'status': 'PASS' if code == 0 else 'FAIL',
                                  'seconds': time.monotonic() - start})
        save()
        print(name, summary['checks'][-1]['status'], flush=True)
        if code:
            raise RuntimeError(name + ' failed; see ' + str(out))

    save()
    try:
        if args.section == 'checks':
            host = ['cargo', 'test', '--locked', '--offline', '--target', 'x86_64-unknown-linux-gnu']
            check('kernel-host', host + ['-p', 'sunlight-kernel', '--lib', '--', '--test-threads=1'])
            packages = ['sunlight-ipc', 'sunlight-uac', 'sunlight-sessiond', 'sunlight-fs',
                        'sunlightd', 'sunlight-audiod', 'sunlight-elf',
                        'sunlight-fastlane-ring', 'sunlight-fastlane-doorbell']
            check('services-host', host + sum((['-p', p] for p in packages), []) + ['--lib'])
            check('lease-docs', host + ['-p', 'sunlight-fastlane-ring', '--doc'])
            check('manager-runtime', host + ['-p', 'sunlightd', '--bin', 'sunlightd', '--features', 'dependency-test'])
            check('scheduler-host', ['python3', 'tools/test_scheduler_host.py'])
            check('parser-tests', ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_fastlane_native_boundary.py'])
            if args.reply_race:
                check('race-parser-tests', ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_fastlane_reply_race.py'])
            check('diff-check', ['git', 'diff', '--check'])
        elif args.section in ('build', 'production'):
            for package, binary in SERVICES.items():
                check(package, ['cargo', 'build', '--locked', '--offline', '--release', '-p', package],
                      {'RUSTFLAGS': SERVICE_FLAGS})
                shutil.copy2(BUILD / 'x86_64-unknown-none/release' / binary,
                             ROOT / 'target/x86_64-unknown-none/release' / binary)
            command = ['cargo', 'build', '--locked', '--offline', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel']
            if args.section == 'build':
                command += ['--features', ','.join(features)]
            check('kernel', command)
            elf, iso = out / 'kernel.elf', out / 'fixture.iso'
            check('strip', ['objcopy', '--strip-debug', str(BUILD / 'x86_64-unknown-none/debug/sunlight-kernel'), str(elf)])
            scratch = out / 'iso-build'
            scratch.mkdir()
            shutil.copy2(ROOT / 'limine.conf', scratch / 'limine.conf')
            check('iso', ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso), str(ROOT / 'target/limine'), str(scratch)])
            check('iso-extract', ['xorriso', '-osirrox', 'on', '-indev', str(iso), '-extract',
                                 '/boot/sunlight-kernel.elf', str(out / 'iso-kernel.elf')])
            if digest(elf) != digest(out / 'iso-kernel.elf'):
                raise RuntimeError('ISO kernel differs from ELF')
            payloads = {}
            archive = out / 'embedded'
            archive.mkdir()
            with elf.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as raw:
                for binary in SERVICES.values():
                    path = ROOT / 'target/x86_64-unknown-none/release' / binary
                    data = path.read_bytes()
                    offset = raw.find(data)
                    if offset < 0:
                        raise RuntimeError('ordinary service not embedded: ' + binary)
                    shutil.copy2(path, archive / binary)
                    payloads[binary] = {'sha256': digest(path), 'bytes': len(data), 'offset': offset}
                marker = raw.find(b'[IPC-DEFER]') >= 0
                if marker != (args.section == 'build'):
                    raise RuntimeError('fixture marker inclusion differs from requested feature set')
                if (raw.find(b'[IPC-RACE]') >= 0) != (args.section == 'build' and args.reply_race):
                    raise RuntimeError('race marker inclusion differs from requested feature set')
                for marker in (b'[D36R-TEST]', b'[SESSION-E3]', b'[REAP-TEST]', b'ExecStart=/sbin/d1-driver'):
                    if raw.find(marker) >= 0:
                        raise RuntimeError('ordinary services contain unrelated fixture')
            summary['image'] = {'elf': str(elf), 'iso': str(iso), 'elf_sha256': digest(elf),
                                'iso_sha256': digest(iso), 'embedded_services': payloads,
                                'features': features if args.section == 'build' else []}
        else:
            manifest = json.loads(args.image_manifest.read_text())
            image = manifest['image']
            elf, iso = Path(image['elf']), Path(image['iso'])
            if manifest['status'] != 'PASS' or manifest['source_sha256'] != sources():
                raise RuntimeError('build source identity differs from current source')
            if image['features'] != features:
                raise RuntimeError('wrong feature set')
            if digest(elf) != image['elf_sha256'] or digest(iso) != image['iso_sha256']:
                raise RuntimeError('image identity mismatch')
            for binary, payload in image['embedded_services'].items():
                if digest(elf.parent / 'embedded' / binary) != payload['sha256']:
                    raise RuntimeError('archived service identity mismatch: ' + binary)
            summary['image'] = image
            for cpus in args.cpus:
                serial = out / f'{cpus}cpu.serial.log'
                serial.write_text('')
                command = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                           '-smp', str(cpus), '-m', '1024', '-cdrom', str(iso), '-serial', f'file:{serial}',
                           '-display', 'none', '-nic', 'none', '-device', 'virtio-rng-pci,disable-modern=on',
                           '-no-reboot', '-no-shutdown']
                start = time.monotonic()
                timed_out = False
                with (out / f'{cpus}cpu.qemu.log').open('w') as log:
                    process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                    try:
                        while time.monotonic() - start < 180:
                            raw = serial.read_text(errors='replace')
                            if assessment(raw, cpus)['status'] == 'PASS' or any(m in raw for m in ('PANIC', 'panicked at', 'UNEXPECTED')) or process.poll() is not None:
                                break
                            time.sleep(.2)
                        else:
                            timed_out = True
                    finally:
                        if process.poll() is None:
                            process.terminate()
                            try:
                                process.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                process.kill()
                        process.wait()
                row = {'cpus': cpus, 'command': command, 'qemu_exit_code': process.returncode,
                       'timed_out': timed_out, 'seconds': time.monotonic() - start,
                       'serial_sha256': digest(serial), **assessment(serial.read_text(errors='replace'), cpus)}
                if timed_out or sources() != summary['source_sha256']:
                    row['status'] = 'FAIL'
                    row['errors'].append('timeout or source changed during execution')
                if row['status'] == 'PASS':
                    ack = {'status': 'PASS', 'cpus': cpus, 'serial_sha256': digest(serial),
                           'elf_sha256': digest(elf), 'iso_sha256': digest(iso),
                           'scope': row['scope'], 'source_matches': True}
                    ack_path = out / f'{cpus}cpu.host-ack.json'
                    ack_path.write_text(json.dumps(ack, indent=2) + '\n')
                    if json.loads(ack_path.read_text()) != ack:
                        raise RuntimeError('host acknowledgement readback mismatch')
                    row['host_ack_sha256'] = digest(ack_path)
                summary['native'].append(row)
                save()
                print('native', cpus, row['status'], flush=True)
                if row['status'] != 'PASS':
                    raise RuntimeError('native prerequisite failed; larger CPU cases NOT_RUN')
        summary['source_unchanged'] = sources() == summary['source_sha256']
        if not summary['source_unchanged']:
            raise RuntimeError('source changed during validation')
        summary['status'] = 'PASS'
    except Exception as error:
        summary['status'] = 'FAIL'
        summary['failure'] = str(error)
        raise
    finally:
        save()
        print(out / 'validation-summary.json', flush=True)


if __name__ == '__main__':
    main()
