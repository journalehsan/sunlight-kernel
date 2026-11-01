#!/usr/bin/env python3
"""M3.1B bounded page-table retention / VM teardown fixture.

The fixture is a boot-time opt-in kernel gate (feature fastlane_m31_retention_test)
that drives the ordinary SHM allocation/retirement entry points against dedicated
owner VMs. It needs no login or userspace actor, so this validator boots the
traced image directly and assesses the [FL-M31] serial records.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from fastlane_f11b0e_validate import ROOT, digest, sources

BASE = ROOT / "target/fastlane-m31b-evidence"
BUILD = ROOT / "target/fastlane-f11b0e3-build"
ENV = {"CARGO_TARGET_DIR": str(BUILD), "CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0"}


def rows(raw, prefix):
    found = []
    for line in raw.splitlines():
        if line.startswith(prefix):
            pairs = re.findall(r'(\w+)=(\S+)', line)
            record = dict(pairs)
            if len(record) != len(pairs):
                record['_duplicate'] = '1'
            bracket = re.search(r'\bids=\[([^\]]*)\]', line)
            if bracket:
                record['ids'] = bracket.group(1)
            found.append(record)
    return found


def assess(raw, cpus, prerepair=False):
    errors = []

    def need(ok, why):
        if not ok:
            errors.append(why)

    def num(record, key):
        try:
            return int(record[key])
        except (KeyError, ValueError, TypeError):
            errors.append('missing/invalid ' + key)
            return -1

    need(not any(x in raw for x in ('PANIC', 'panicked at')), 'guest assertion/panic')
    records = rows(raw, '[FL-M31] ')
    need(not any('_duplicate' in r for r in records), 'duplicate trace field')
    phases = [r for r in records if 'phase' in r]
    need(len(phases) == len(records), 'non-phase FL-M31 record')
    got = [r['phase'] for r in phases]
    for name in ('setup', 'initial', 'reuse', 'retention', 'teardown',
                 'teardown-retained', 'teardown-retained-complete', 'final', 'complete'):
        need(got.count(name) == 1, 'missing/duplicate phase ' + name)
    setup = [r for r in phases if r['phase'] == 'setup']
    if len(setup) == 1:
        need(num(setup[0], 'pre_repair') == int(prerepair), 'pre_repair mode flag mismatch')

    crossings = [r for r in phases if r['phase'] == 'crossing']
    need(len(crossings) == 8, 'expected eight 2 MiB boundary crossings')
    for index, row in enumerate(crossings):
        need(num(row, 'index') == index, 'crossing index out of order')
        if prerepair:
            # Retained empty chains from earlier crossings stay linked, so the
            # live table set is the one new chain plus all previously retained.
            need(num(row, 'tables_live') == 3 + index, 'pre-repair live table set not one chain per crossing')
            need(num(row, 'retained_empty_p1') == index + 1, 'pre-repair crossing did not retain one more empty P1')
            need(num(row, 'tables_after_free') == 3 + index, 'pre-repair table growth not one P1 chain per crossing')
        else:
            need(num(row, 'tables_live') == 3, 'live span did not own exactly three tables')
            need(num(row, 'retained_empty_p1') == 0, 'empty P1 retained after free')
            need(num(row, 'tables_after_free') == 0, 'table set did not drain after free')
        need(int(row['base'], 16) % (2 * 1024 * 1024) == 0, 'crossing base not 2 MiB aligned')

    reuse = [r for r in phases if r['phase'] == 'reuse']
    if len(reuse) == 1:
        need(num(reuse[0], 'shared_p1') == 1, 'same-span reuse did not share one P1')
        first = int(reuse[0]['first'], 16)
        second = int(reuse[0]['second'], 16)
        need(second == first + 4096, 'reuse witness not consecutive')
        need((first >> 21) == (second >> 21), 'reuse witness crossed a P1 window')

    retention = [r for r in phases if r['phase'] == 'retention']
    if len(retention) == 1:
        need(num(retention[0], 'crossings') == 8, 'retention phase crossing count mismatch')
        need(num(retention[0], 'retained_empty_p1') == (9 if prerepair else 0),
             'retained empty P1 total did not match the free-path policy')
        need(num(retention[0], 'total_tables') == (11 if prerepair else 0),
             'owner table total did not match the free-path policy')
        need(num(retention[0], 'pre_repair_analytic_p1_bound') == 2048, 'analytic bound changed')
        need(num(retention[0], 'max_live_group_tables') == (10 if prerepair else 3),
             'max live table group changed')

    teardown = [r for r in phases if r['phase'] == 'teardown']
    if len(teardown) == 1:
        need(num(teardown[0], 'reclaim_page_tables') == (12 if prerepair else 1),
             'owner teardown released tables other than reachable plus root')
        need(num(teardown[0], 'reclaim_user_frames') == 0, 'empty owner teardown released user frames')
        need(num(teardown[0], 'reachable_lower_tables') == (11 if prerepair else 0),
             'owner lower table set before teardown differed from the free-path policy')

    retained = [r for r in phases if r['phase'] == 'teardown-retained']
    ids = []
    if len(retained) == 1:
        spans = num(retained[0], 'spans')
        need(spans == 4, 'retention arm span count mismatch')
        need(num(retained[0], 'retained_empty_p1') == spans, 'retention arm did not retain one empty P1 per span')
        ids = [int(v) for v in retained[0].get('ids', '').strip('[]').split(',') if v.strip()]
        need(len(ids) == num(retained[0], 'reachable_lower_tables'), 'retained id list inconsistent')
        need(len(set(ids)) == len(ids), 'duplicate retained table identity')
    complete = [r for r in phases if r['phase'] == 'teardown-retained-complete']
    if len(complete) == 1 and ids:
        need(num(complete[0], 'released_tables') == len(ids), 'retention arm released wrong table count')
        need(num(complete[0], 'reclaim_page_tables') == len(ids) + 1, 'retention arm did not release each table plus root exactly once')
        need(num(complete[0], 'reclaim_user_frames') == 0, 'retention arm released user frames')

    final = [r for r in phases if r['phase'] == 'final']
    if len(final) == 1:
        need(num(final[0], 'free_delta') == 0, 'allocator drift after teardown')
        need(num(final[0], 'pt_delta') == 0, 'page-table class drift after teardown')
        need(num(final[0], 'shm_delta') == 0, 'shared-memory class drift after teardown')
    return {"status": "FAIL" if errors else "PASS", "errors": errors, "cpus": cpus,
            "prerepair": prerepair, "crossings": crossings, "retained_ids": ids,
            "records": len(records)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--section', choices=('build', 'native', 'prerepair-build', 'prerepair-native'),
                        required=True)
    parser.add_argument('--cpus', nargs='+', type=int, choices=(2, 4, 12), default=[2])
    parser.add_argument('--image-manifest', type=Path)
    parser.add_argument('--evidence-root', type=Path, default=BASE)
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    out = args.evidence_root.resolve() / (args.section + '-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True, exist_ok=False)
    prerepair = args.section.startswith('prerepair')
    build_section = args.section in ('build', 'prerepair-build')
    features = ['fastlane_mapping_test', 'key_inject', 'fastlane_census_trace', 'fastlane_m31_retention_test']
    if prerepair:
        features.append('fastlane_m31_prerepair')
    summary = {'milestone': 'M3.1B', 'status': 'RUNNING', 'section': args.section,
               'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
               'source_sha256': sources(), 'checks': [], 'native': [], 'production_fastlane': 'BLOCKED'}

    def save():
        (out / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')

    def check(name, cmd, env=None):
        with (out / (name + '.log')).open('w') as log:
            log.write(json.dumps({'command': cmd, 'environment': {**ENV, **(env or {})}}) + '\n')
            log.flush()
            result = subprocess.run(cmd, cwd=ROOT, env={**os.environ, **ENV, **(env or {})},
                                    stdout=log, stderr=subprocess.STDOUT)
        summary['checks'].append({'name': name, 'command': cmd, 'exit_code': result.returncode})
        save()
        print(name, result.returncode, flush=True)
        if result.returncode:
            raise RuntimeError(name + ' failed')

    save()
    print(out, flush=True)
    try:
        if build_section:
            check('kernel', ['cargo', 'build', '--locked', '--offline', '-p', 'sunlight-kernel',
                             '--bin', 'sunlight-kernel', '--features', ','.join(features)],
                  {'SUNLIGHT_INJECT_PHASE': 'desktop_login'})
            elf = out / 'kernel.elf'
            iso = out / 'fixture.iso'
            check('strip', ['objcopy', '--strip-debug',
                            str(BUILD / 'x86_64-unknown-none/debug/sunlight-kernel'), str(elf)])
            scratch = out / 'iso-build'
            scratch.mkdir()
            shutil.copy2(ROOT / 'limine.conf', scratch / 'limine.conf')
            check('iso', ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso),
                          str(ROOT / 'target/limine'), str(scratch)])
            check('extract', ['xorriso', '-osirrox', 'on', '-indev', str(iso), '-extract',
                              '/boot/sunlight-kernel.elf', str(out / 'iso-kernel.elf')])
            assert digest(elf) == digest(out / 'iso-kernel.elf')
            raw = elf.read_bytes()
            assert (b'[FL-M31]' in raw), 'm31 retention feature isolation mismatch'
            summary['image'] = {'elf': str(elf), 'iso': str(iso), 'elf_sha256': digest(elf),
                                'iso_sha256': digest(iso), 'features': features, 'prerepair': prerepair}
        else:
            if not args.image_manifest:
                raise RuntimeError('--image-manifest required')
            manifest = json.loads(args.image_manifest.read_text())
            image = manifest['image']
            assert manifest['status'] == 'PASS' and manifest['source_sha256'] == sources(), 'source mismatch'
            assert image['features'] == features
            identity = {'source': True,
                        'elf': digest(Path(image['elf'])) == image['elf_sha256'],
                        'iso': digest(Path(image['iso'])) == image['iso_sha256']}
            assert all(identity.values()), 'image mismatch'
            summary['image'] = image
            for cpus in args.cpus:
                serial = out / f'{cpus}cpu.serial.log'
                serial.write_text('')
                command = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                           '-smp', str(cpus), '-m', '2048', '-cdrom', image['iso'],
                           '-serial', 'file:' + str(serial), '-display', 'none', '-nic', 'none',
                           '-no-reboot', '-no-shutdown', '-device', 'virtio-rng-pci,disable-modern=on']
                start = time.monotonic()
                failure = None
                with (out / f'{cpus}cpu.qemu.log').open('w') as log:
                    proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                    try:
                        done = False
                        while time.monotonic() - start < args.timeout and proc.poll() is None:
                            raw = serial.read_text(errors='replace')
                            if '[FL-M31] phase=complete' in raw:
                                done = True
                                break
                            if any(m in raw for m in ('PANIC', 'panicked at', 'KERNEL PANIC')):
                                raise RuntimeError('guest assertion')
                            time.sleep(0.1)
                        if not done and '[FL-M31] phase=complete' not in serial.read_text(errors='replace'):
                            raise TimeoutError('m31 retention completion')
                    except Exception as error:
                        failure = {'type': type(error).__name__, 'message': str(error)}
                    finally:
                        if proc.poll() is None:
                            proc.terminate()
                            try:
                                proc.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                proc.kill()
                        proc.wait()
                assessment = assess(serial.read_text(errors='replace'), cpus, prerepair)
                row = {'cpus': cpus, 'command': command, 'failure': failure, 'assessment': assessment,
                       'serial': str(serial), 'serial_sha256': digest(serial)}
                summary['native'].append(row)
                save()
                print('native', cpus, assessment['status'], assessment['errors'], flush=True)
                if failure or assessment['status'] != 'PASS':
                    raise RuntimeError('native failed; retained original attempt')
        summary['status'] = 'PASS'
    except Exception as error:
        summary['status'] = 'FAIL'
        summary['failure'] = {'type': type(error).__name__, 'message': str(error)}
    finally:
        summary['source_sha256_after'] = sources()
        if summary['source_sha256_after'] != summary['source_sha256']:
            summary['status'] = 'FAIL'
            summary['source_changed'] = True
        save()
        archive = out / 'source'
        archive.mkdir(exist_ok=True)
        for name in summary['source_sha256']:
            src = ROOT / name
            dest = archive / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dest)
        (out / 'artifact-index.json').write_text(json.dumps(
            {str(p.relative_to(out)): digest(p) for p in out.rglob('*') if p.is_file() and p.name != 'artifact-index.json'},
            indent=2) + '\n')
    print('status', summary['status'], out, flush=True)
    return 0 if summary['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
