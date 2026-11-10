#!/usr/bin/env python3
"""Source-bound original M1--M5 clients through an Account-inherited real shell.

Only launch infrastructure changes. Original security assessors are imported
unchanged. Each attempt has exclusive logs, including separate stdout/stderr.
"""
import argparse
from datetime import datetime, timezone
import json
import mmap
import os
from pathlib import Path
import shutil
import subprocess
import time

from fastlane_f11b0c_validate import SERVICE_FLAGS
from fastlane_f11b0e_validate import ROOT, digest


def sources():
    paths = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=ROOT,
        text=True).splitlines()
    return {p: digest(ROOT / p) for p in sorted(set(paths))
            if not p.startswith('target/') and (not p.startswith('.') or p.startswith('.cargo/'))
            and (ROOT / p).is_file()
            and (p.endswith(('.rs', '.toml', '.ld', '.py', '.sh', '.S'))
                 or p in ('Cargo.lock', 'limine.cfg', 'limine.conf'))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--suite', required=True, choices=('m2', 'm4', 'm5'))
    parser.add_argument('--evidence-root', type=Path, default=ROOT / '.kfp-2a4-evidence/native')
    parser.add_argument('--timeout', type=int, default=180)
    parser.add_argument('--cpus', type=int, choices=(1, 2, 4, 12), default=2)
    parser.add_argument('--kernel-fixture', choices=('mapping', 'doorbell', 'transfer'))
    args = parser.parse_args()
    out = args.evidence_root.resolve() / (
        args.suite + '-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True)
    result = {'suite': args.suite, 'status': 'RUNNING', 'stage': 'build',
              'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'source_sha256': sources(), 'checks': [], 'native': [],
              'production_fastlane': 'DISABLED', 'cpus': args.cpus}

    def save():
        (out / 'results.json').write_text(json.dumps(result, indent=2) + '\n')

    def integrity():
        if sources() != result['source_sha256']:
            result['stage'] = 'source-integrity'
            raise RuntimeError('source changed during attempt')

    def check(name, command, extra=None):
        save()
        with (out / (name + '.stdout')).open('x') as stdout, (out / (name + '.stderr')).open('x') as stderr:
            row = {'name': name, 'command': command, 'cwd': str(ROOT),
                   'environment': extra or {}, 'stdout': str(stdout.name), 'stderr': str(stderr.name)}
            result['checks'].append(row)
            save()
            completed = subprocess.run(command, cwd=ROOT, env={**os.environ, **(extra or {})},
                                       stdout=stdout, stderr=stderr, timeout=600)
            row['exit_status'] = completed.returncode
        save()
        print(name, completed.returncode, flush=True)
        completed.check_returncode()
        integrity()

    save()
    print(out, flush=True)
    try:
        # Reuse the existing session fixture's registration, initial launch,
        # rollback/foreign/replay checks, and component HELLO/READY exchange.
        services = {'sunlight-init': 'sunlight-init', 'sunlightd': 'sunlightd',
                    'sunlight-tty-server': 'sunlight-tty-server', 'sunlight-uac': 'uac_service',
                    'sunlight-sessiond': 'sunlight-sessiond', 'sunlight-vfs-server': 'sunlight-vfs-server',
                    'sunlightctl': 'sunlightctl'}
        for package, binary in services.items():
            cmd = ['cargo', 'build', '--locked', '--offline', '--release', '-p', package]
            if package == 'sunlight-uac':
                service_feature = {'m2': 'fastlane-mapping-test',
                                   'm4': 'fastlane-doorbell-test',
                                   'm5': 'fastlane-transfer-test'}[args.suite]
                cmd += ['--features', 'kfp-security-runner,' + service_feature]
            extra = {'RUSTFLAGS': SERVICE_FLAGS, 'CARGO_INCREMENTAL': '0', 'KFP2A4_SUITE': args.suite}
            if package == 'sunlight-sessiond':
                extra['SUNLIGHT_INJECT_PHASE'] = 'm53r41'
            check(package, cmd, extra)
        names = list(services.values()) + ['session-race-probe', 'sshl']
        archive = out / 'embedded'
        archive.mkdir()
        for name in names:
            shutil.copy2(ROOT / 'target/x86_64-unknown-none/release' / name, archive / name)
        fixture_feature = {'m2': 'fastlane_mapping_test', 'm4': 'fastlane_doorbell_test',
                           'm5': 'fastlane_transfer_test'}[args.suite]
        if args.kernel_fixture:
            fixture_feature = 'fastlane_' + args.kernel_fixture + '_test'
        features = 'key_inject,session_launch_test,' + fixture_feature
        check('kernel', ['cargo', 'build', '--locked', '--offline', '-p', 'sunlight-kernel',
                         '--bin', 'sunlight-kernel', '--features', features],
              {'SUNLIGHT_INJECT_PHASE': 'desktop_login', 'CARGO_INCREMENTAL': '0'})
        elf, iso = out / 'kernel.elf', out / 'fixture.iso'
        check('strip', ['objcopy', '--strip-debug', str(ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel'), str(elf)])
        scratch = out / 'iso-work'
        scratch.mkdir()
        shutil.copy2(ROOT / 'limine.conf', scratch / 'limine.conf')
        check('iso', ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso), str(ROOT / 'target/limine'), str(scratch)])
        check('extract', ['xorriso', '-osirrox', 'on', '-indev', str(iso), '-extract',
                          '/boot/sunlight-kernel.elf', str(out / 'iso-kernel.elf')])
        result['stage'] = 'source-integrity'
        assert digest(elf) == digest(out / 'iso-kernel.elf'), 'ISO kernel differs'
        with elf.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as raw:
            result['embedded'] = {}
            for name in names:
                data = (archive / name).read_bytes()
                assert data == (ROOT / 'target/x86_64-unknown-none/release' / name).read_bytes(), name
                offset = raw.find(data)
                assert offset >= 0, ('payload not embedded', name)
                result['embedded'][name] = {'sha256': digest(archive / name), 'offset': offset, 'bytes': len(data)}
        result['image'] = {'elf': str(elf), 'iso': str(iso), 'elf_sha256': digest(elf),
                           'iso_sha256': digest(iso), 'features': features}
        integrity()
        result['stage'] = 'guest-launch'
        serial = out / (str(args.cpus) + 'cpu.serial.log')
        command = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max', '-smp', str(args.cpus),
                   '-m', '2048', '-cdrom', str(iso), '-serial', 'file:' + str(serial), '-display', 'none',
                   '-nic', 'none', '-no-reboot', '-no-shutdown', '-device', 'virtio-rng-pci,disable-modern=on']
        row = {'command': command, 'cwd': str(ROOT), 'serial': str(serial)}
        result['native'].append(row)
        save()
        with (out / 'qemu.stdout').open('x') as stdout, (out / 'qemu.stderr').open('x') as stderr:
            proc = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            start = time.monotonic()
            reason = 'timeout'
            try:
                while time.monotonic() - start < args.timeout:
                    raw = serial.read_text(errors='replace') if serial.exists() else ''
                    if '[KFP-SEC] event=complete suite=' + args.suite in raw:
                        reason = 'complete'
                        break
                    if 'PANIC' in raw or 'panicked at' in raw or 'checkpoint=FAIL' in raw:
                        reason = 'guest-assertion'
                        break
                    if proc.poll() is not None:
                        reason = 'qemu-exit'
                        break
                    time.sleep(.1)
            finally:
                row['host_terminated'] = proc.poll() is None
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                row['exit_status'] = proc.wait()
        row['stop_reason'] = reason
        row['elapsed_seconds'] = time.monotonic() - start
        row['serial_sha256'] = digest(serial)
        raw = serial.read_text(errors='replace')
        result['stage'] = 'security-tests' if '[FL-' in raw else 'shell-prerequisite'
        failure = None if reason == 'complete' else {'type': reason, 'message': 'native driver incomplete'}
        identity = dict(source=True, elf=True, iso=True, services=True)
        if args.suite == 'm2':
            from fastlane_mapping_validate import assess
            from fastlane_m3_validate import assess_words
            row['assessment'] = assess(raw, args.cpus, identity, failure)
            row['m3'] = assess_words(raw)
        elif args.suite == 'm4':
            from fastlane_m4_assess import assess
            row['assessment'] = assess(raw, args.cpus, identity, failure)
        else:
            from fastlane_m5_assess import assess
            row['assessment'] = assess(raw, args.cpus, identity, failure)
        integrity()
        result['status'] = 'PASS' if row['assessment']['status'] == 'PASS' and reason == 'complete' else 'FAIL'
        result['classification'] = ('PASS' if result['status'] == 'PASS' else
                                    'TEST_FAILURE' if result['stage'] == 'security-tests' else 'INFRASTRUCTURE_FAILURE')
    except Exception as error:
        result['status'] = 'FAIL'
        result['failure'] = {'type': type(error).__name__, 'message': str(error)}
        result['classification'] = {'build': 'BUILD_FAILURE', 'source-integrity': 'SOURCE_INTEGRITY_FAILURE',
                                    'security-tests': 'TEST_FAILURE'}.get(result['stage'], 'INFRASTRUCTURE_FAILURE')
    finally:
        result['source_sha256_after'] = sources()
        if result['source_sha256_after'] != result['source_sha256']:
            result['status'] = 'FAIL'
            result['classification'] = 'SOURCE_INTEGRITY_FAILURE'
        save()
        (out / 'artifact-index.json').write_text(json.dumps({str(p.relative_to(out)): digest(p)
            for p in out.rglob('*') if p.is_file() and p.name != 'artifact-index.json'}, indent=2) + '\n')
    print(result['status'], result.get('classification'), flush=True)
    return int(result['status'] != 'PASS')


if __name__ == '__main__':
    raise SystemExit(main())
