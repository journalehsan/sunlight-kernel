#!/usr/bin/env python3
"""Source-matched, bounded wider IPC boot replay. Does not run scheduled races."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import time

from fastlane_f11b0e_validate import ROOT, digest, sources
from fastlane_m53_quota_validate import source_id, stop
from fastlane_m53r2_assess import assess_ipc


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image-manifest', required=True, type=Path)
    parser.add_argument('--evidence-root', required=True, type=Path)
    parser.add_argument('--cpus', nargs='+', type=int, choices=(1, 2, 4, 12), default=[2])
    parser.add_argument('--repetitions', type=int, choices=(1, 2, 3), default=2)
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 180:
        parser.error('timeout must be 1..180 seconds')
    out = args.evidence_root.resolve() / ('ipc-native-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True)
    snapshot = sources()
    result = {'status': 'RUNNING', 'scope': 'boot-method', 'source_sha256': snapshot,
              'source_id': source_id(snapshot), 'native': [],
              'scheduled_lifecycle': 'NOT_RUN', 'production_fastlane': 'DISABLED'}

    def save():
        (out / 'validation-summary.json').write_text(json.dumps(result, indent=2) + '\n')

    save()
    try:
        manifest = json.loads(args.image_manifest.read_text())
        if manifest['status'] != 'PASS' or manifest['source_sha256'] != snapshot:
            raise ValueError('failed build or source mismatch')
        image = manifest['image']
        for kind in ('elf', 'iso'):
            if digest(Path(image[kind])) != image[kind + '_sha256']:
                raise ValueError(kind + ' hash mismatch')
        for name, entry in image.get('services', {}).items():
            if digest(Path(image['elf']).parent / 'embedded' / name) != entry['sha256']:
                raise ValueError('embedded input hash mismatch: ' + name)
        result['image'] = image
        for cpus in args.cpus:
            for repetition in range(1, args.repetitions + 1):
                name = f'{cpus}cpu-{repetition}'
                serial = out / (name + '.serial.log')
                serial.touch(exist_ok=False)
                cmd = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                       '-smp', str(cpus), '-m', '2048', '-cdrom', image['iso'],
                       '-serial', 'file:' + str(serial), '-display', 'none', '-nic', 'none',
                       '-no-reboot', '-no-shutdown', '-device', 'virtio-rng-pci,disable-modern=on']
                row = {'cpus': cpus, 'repetition': repetition, 'command': cmd,
                       'timeout_seconds': args.timeout, 'serial': str(serial)}
                result['native'].append(row)
                save()
                start = time.monotonic()
                reason = 'deadline'
                with (out / (name + '.qemu.log')).open('x') as log:
                    proc = subprocess.Popen(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                            start_new_session=True)
                    try:
                        while time.monotonic() - start < args.timeout:
                            raw = serial.read_text(errors='replace')
                            if '[SEC]  dead process cap:' in raw:
                                reason = 'terminal-boot-receipt'
                                break
                            if 'PANIC' in raw or 'panicked at' in raw:
                                reason = 'guest-panic'
                                break
                            if proc.poll() is not None:
                                reason = 'qemu-exit'
                                break
                            time.sleep(.05)
                    finally:
                        stop(proc)
                row.update(reason=reason, qemu_exit_code=proc.returncode,
                           elapsed_seconds=time.monotonic() - start,
                           serial_sha256=digest(serial),
                           assessment=assess_ipc(serial.read_text(errors='replace')))
                row['status'] = ('PASS' if reason == 'terminal-boot-receipt'
                                 and row['assessment']['status'] == 'PASS' else 'FAIL')
                save()
                print(name, row['status'], row['assessment']['errors'], flush=True)
        result['status'] = 'PASS' if all(r['status'] == 'PASS' for r in result['native']) else 'FAIL'
    except Exception as error:
        result['status'] = 'FAIL'
        result['failure'] = str(error)
    finally:
        result['source_matched_after'] = sources() == snapshot
        if not result['source_matched_after']:
            result['status'] = 'UNQUALIFIED'
        save()
    print(result['status'], out, flush=True)
    return 0 if result['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
