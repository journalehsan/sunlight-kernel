#!/usr/bin/env python3
"""Build and run the scheduled Account lifecycle probe; retain incomplete gates."""
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
import uuid
from fastlane_f11b0e_validate import ROOT, digest, sources
from fastlane_f11b0c_validate import SERVICE_FLAGS
from fastlane_m53_quota_validate import stop, source_id


from fastlane_m53r5_assess import assess
from fastlane_m53r53_assess import assess as assess_session_orders, peer_reaped
from fastlane_m53r52_assess import assess as assess_exit_unmap


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence-root', type=Path, required=True)
    p.add_argument('--cpus', type=int, nargs='+', choices=(1, 2, 4, 12), default=[1, 2, 4, 12])
    p.add_argument('--timeout', type=int, default=180)
    p.add_argument('--image-manifest', type=Path)
    p.add_argument('--build-only', action='store_true')
    p.add_argument('--remote-witness', action='store_true')
    p.add_argument('--restricted-retirement', action='store_true',
                   help='measure restricted retirement; does not qualify retained mappings')
    p.add_argument('--restricted-order', choices=('cleanup-first', 'retirement-first'))
    args = p.parse_args()
    if args.restricted_order and not args.restricted_retirement:
        p.error('--restricted-order requires --restricted-retirement')
    if args.remote_witness and 1 in args.cpus: p.error('remote witness requires SMP')
    out = args.evidence_root.resolve() / ('native-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ'))
    out.mkdir(parents=True)
    result = {'status': 'RUNNING', 'source': sources(), 'checks': [], 'native': [],
              'production_fastlane': 'DISABLED', 'overall_verdict': 'PARTIAL', 'remote_witness': args.remote_witness,
              'restricted_retirement': args.restricted_retirement, 'restricted_order': args.restricted_order,
              'head': subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()}
    result['build_identity'] = {'source': source_id(result['source']), 'run': uuid.uuid4().hex}
    def save():
        (out / 'validation-summary.json').write_text(json.dumps(result, indent=2) + '\n')
    def check(name, command, extra=None):
        env = {**os.environ, **(extra or {})}
        if args.restricted_order:
            env['SUNLIGHT_R55_ORDER'] = args.restricted_order
            extra = {**(extra or {}), 'SUNLIGHT_R55_ORDER': args.restricted_order}
        with (out / (name + '.log')).open('x') as log:
            r = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=600)
        result['checks'].append({'name': name, 'command': command, 'environment': extra,
                                 'exit_code': r.returncode})
        save()
        print(name, r.returncode, flush=True)
        if r.returncode:
            raise RuntimeError(name + ' failed')
    save()
    try:
        if args.image_manifest:
            manifest = json.loads(args.image_manifest.read_text())
            if manifest['source'] != result['source'] or manifest['status'] != 'PASS':
                raise RuntimeError('source-matched passing build required')
            if (manifest.get('remote_witness', False) != args.remote_witness
                    or manifest.get('restricted_retirement', False) != args.restricted_retirement
                    or manifest.get('restricted_order') != args.restricted_order):
                raise RuntimeError('fixture configuration mismatch')
            result['build_identity'] = manifest['build_identity']
            result['embedded'] = manifest['embedded']
            result['image'] = manifest['image']
            iso = Path(manifest['image']['iso'])
            if digest(iso) != manifest['image']['iso_sha256']:
                raise RuntimeError('image digest mismatch')
        else:
            services = {'sunlight-init': 'sunlight-init', 'sunlightd': 'sunlightd',
                        'sunlight-tty-server': 'sunlight-tty-server', 'sunlight-uac': 'uac_service',
                        'sunlight-sessiond': 'sunlight-sessiond', 'sunlight-vfs-server': 'sunlight-vfs-server'}
            for package in services:
                extra = {'RUSTFLAGS': SERVICE_FLAGS}
                if package == 'sunlight-sessiond':
                    extra['SUNLIGHT_INJECT_PHASE'] = 'm53r41'
                command = ['cargo', 'rustc', '--locked', '--offline', '--release', '-p', package, '--bin', services[package]]
                if package == 'sunlight-uac' and (args.remote_witness or args.restricted_retirement):
                    command += ['--features', 'quota-lifecycle-test'
                                + (',quota-remote-test' if args.remote_witness else '')
                                + (',restricted-retirement-test' if args.restricted_retirement else '')]
                command += ['--', '-C', 'metadata=' + result['build_identity']['run']]
                check(package, command, extra)
            check('probe', ['cargo', 'rustc', '--locked', '--offline', '--release', '-p', 'sunlight-uac',
                            '--bin', 'session-race-probe', '--features', 'quota-lifecycle-test' + (',quota-remote-test' if args.remote_witness else '') + (',restricted-retirement-test' if args.restricted_retirement else ''), '--', '-C', 'metadata=' + result['build_identity']['run']], {'RUSTFLAGS': SERVICE_FLAGS})
            archive = out / 'embedded'
            archive.mkdir()
            payload_names = list(services.values()) + ['session-race-probe', 'linux-thread-probe']
            def payload_path(name):
                return ROOT / ('target/helios-probes' if name == 'linux-thread-probe' else 'target/x86_64-unknown-none/release') / name
            for name in payload_names:
                shutil.copy2(payload_path(name), archive / name)
            result['embedded'] = {name: digest(archive / name) for name in payload_names}
            check('kernel', ['cargo', 'build', '--locked', '--offline', '-p', 'sunlight-kernel',
                             '--bin', 'sunlight-kernel', '--features', 'key_inject,quota_lifecycle_test' + (',restricted_remote_test' if args.remote_witness else '') + (',fastlane_mapping_test,fastlane_census_trace' if args.restricted_retirement else '')],
                  {'SUNLIGHT_INJECT_PHASE': 'desktop_login', 'SUNLIGHT_R5_SOURCE': result['build_identity']['source'], 'SUNLIGHT_R5_RUN': result['build_identity']['run']})
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
        for cpus in ([] if args.build_only else args.cpus):
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
                        if ('[SL41-USER] stage=complete' in raw and '[SL41] event=retirement-fenced\n' in raw and '[R51-SESSION-USER] event=complete ' in raw and peer_reaped(raw)) or 'PANIC' in raw or 'panicked at' in raw or 'checkpoint=FAIL' in raw or proc.poll() is not None:
                            break
                        time.sleep(.1)
                finally:
                    stop(proc)
            row = {'cpus': cpus, 'command': cmd, 'elapsed': time.monotonic() - start, 'timeout_seconds': args.timeout,
                   'serial_sha256': digest(serial), 'assessment': assess(serial.read_text(errors='replace'), cpus, result['build_identity']['source'], result['build_identity']['run'])}
            if args.remote_witness:
                from fastlane_m53r5_remote import assess_remote
                from fastlane_mapping_assess import assess_mapping
                row['remote_witness'] = assess_remote(serial.read_text(errors='replace'), cpus)
                raw = serial.read_text(errors='replace')
                start = raw.find('[FL-M2] case=start ')
                row['restricted_mapping'] = assess_mapping(raw[start:] if start >= 0 else '', cpus)
                for gate in ('remote_witness', 'restricted_mapping'):
                    if row[gate]['status'] != 'PASS':
                        row['assessment']['status'] = 'FAIL'
                        row['assessment']['errors'] += row[gate]['errors']
            from fastlane_m53r41_validate import assess as assess_launch
            row['launch_regression'] = assess_launch(serial.read_text(errors='replace'))
            if row['launch_regression']['status'] != 'PASS':
                row['assessment']['status'] = 'FAIL'
                row['assessment']['errors'] += row['launch_regression']['errors']
            row['session_retirement'] = {'status': 'PASS' if '[R51-SESSION-USER] event=complete ' in serial.read_text(errors='replace') else 'NOT_RUN',
                                         'scope': 'retired authority, both independent resource-owner cleanup orders, final surviving-peer reap'}
            if row['session_retirement']['status'] != 'PASS':
                row['assessment']['status'] = 'FAIL'
                row['assessment']['errors'].append('missing retirement survivor completion')
            row['exit_unmap_regression'] = assess_exit_unmap(serial.read_text(errors='replace'))
            if row['exit_unmap_regression']['status'] != 'PASS':
                row['assessment']['status'] = 'FAIL'
                row['assessment']['errors'] += row['exit_unmap_regression'].get('errors', ['missing exit/unmap orders'])
            row['session_cleanup_orders'] = assess_session_orders(serial.read_text(errors='replace'), restricted_extra=args.restricted_retirement and args.restricted_order != 'cleanup-first')
            if row['session_cleanup_orders']['status'] != 'PASS':
                row['assessment']['status'] = 'FAIL'
                row['assessment']['errors'] += row['session_cleanup_orders'].get('errors', ['missing session cleanup orders'])
            if args.restricted_retirement:
                from fastlane_m53r54_assess import assess as assess_restricted
                if args.restricted_order:
                    from fastlane_m53r55_assess import assess as assess_order
                    row['restricted_retirement'] = assess_order(serial.read_text(errors='replace'), args.restricted_order)
                else:
                    row['restricted_retirement'] = assess_restricted(serial.read_text(errors='replace'))
                if row['restricted_retirement']['diagnostic_status'] != 'PASS':
                    row['assessment']['status'] = 'FAIL'
                    row['assessment']['errors'] += row['restricted_retirement']['errors']
            result['native'].append(row)
            save()
            print(cpus, row['assessment'], flush=True)
            if row['assessment']['status'] != 'PASS' and not args.restricted_retirement:
                raise RuntimeError('native failed; original attempt retained')
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
