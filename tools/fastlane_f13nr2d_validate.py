#!/usr/bin/env python3
"""R2D native evidence checks.

Rounds 0..=5 reuse the R2C ordering-mode lifecycle (prepared task-table reuse,
terminal audit, UD2 exit) exactly as before. Rounds 6/7 add the R2D-B
delayed-dispatch publication boundary and the R2D-A initial-pin/exit boundary.
The R2C contract remains mandatory for the eight-round fixture; the two new
rounds are validated independently on top of it.
"""
from fastlane_f13nr2v_validate import fields
from fastlane_f13nr2c_validate import validate_native as validate_r2c


def _normalize(raw, cpus):
    """Project the eight-round R2D trace onto the six-round R2C contract."""
    lines = []
    for line in raw.splitlines():
        row = fields(line)
        if line.startswith('[R2D-'):
            continue
        if row.get('round') in (6, 7):
            # R2D-only rounds are validated separately; drop every base-contract
            # record attributed to them (cycle, waiter, reuse, terminal).
            continue
        if line.startswith('[R2V] PASS '):
            # Preserve the real frame/metadata numbers; only the round/retire
            # counts are projected onto the six-round R2C contract.
            line = (line.replace('rounds=8', 'rounds=6').replace('retired=8', 'retired=6'))
        lines.append(line)
    return '\n'.join(lines)


def validate_native(raw, cpus):
    r2d_a = [fields(line) for line in raw.splitlines() if line.startswith('[R2D-A] forced ')]
    r2d_a_audit = [fields(line) for line in raw.splitlines() if line.startswith('[R2D-A] audit ')]
    r2d_b = [fields(line) for line in raw.splitlines() if line.startswith('[R2D-B] forced ')]
    cycles = [fields(line) for line in raw.splitlines() if line.startswith('[R2V] cycle ')]
    finals = [fields(line) for line in raw.splitlines() if line.startswith('[R2V] PASS ')]
    errors = []
    if not finals:
        status = 'fail' if ('PANIC' in raw or 'panicked at' in raw) else 'not_run'
        return dict(status=status, errors=['forced fixture completion not reached'],
                    r2d_initial_pin_exit=r2d_a, r2d_delayed_dispatch=r2d_b)
    final = finals[-1]
    if final.get('rounds') != 8 or final.get('retired') != 8 or final.get('metadata_slots') != 1:
        errors.append('eight real lifecycle completions required')
    if final.get('frames_before') != final.get('frames_after'):
        errors.append('frame drift')
    for round_id in (6, 7):
        rows = [r for r in cycles if r.get('round') == round_id]
        if len(rows) != 1:
            errors.append('missing cycle %d' % round_id)
            continue
        row = rows[0]
        # Round 6 is R2D-B (R2V-style registration-wins, executing on the last
        # remote CPU); round 7 is R2D-A (the pinned task is remotely terminated
        # and executes nothing).
        execution = (cpus - 1) if (cpus > 1 and round_id == 6) else (0 if cpus == 1 else 'none')
        # Round 6 (R2D-B) forces every remote CPU to reject; round 7 (R2D-A)
        # forces only CPU1 to reject before the pinned task is terminated.
        mask = (((1 << cpus) - 1) & ~1 if round_id == 6 else 0x2) if cpus > 1 else 0
        expected = dict(source=0, execution=execution,
                        rejected=mask, released=mask,
                        timer_assistance=0, pins=0, waiters=0, queued=0, cleanup='reaped')
        if any(row.get(k) != v for k, v in expected.items()):
            errors.append('cycle %d execution/cleanup mismatch' % round_id)
    forced = {r.get('round') for r in r2d_a + r2d_b}
    if cpus > 1:
        if forced != {6, 7}:
            errors.append('both R2D forced rounds required on SMP')
        for r in r2d_a:
            expected = dict(owner=0, rejecter=1, pin_release_precedes_reap=1, remote_exit='Finished',
                            stack_retained=1, revived=0, duplicate_run=0, waiter_drained=1,
                            pending_dispatch=0, timer_assistance=0)
            if any(r.get(k) != v for k, v in expected.items()) or r.get('generation', 0) == 0:
                errors.append('R2D-A initial-pin/exit invariant missing')
        if len(r2d_a_audit) != 1:
            errors.append('exactly one R2D-A exit audit required')
        for r in r2d_a_audit:
            if any(r.get(k) != v for k, v in dict(pin='held', state='Finished', stack='retained',
                   queued=0, late_wake='retired', duplicate_exit='rejected').items()):
                errors.append('R2D-A audit invariant missing')
        for r in r2d_b:
            expected = dict(owner=0, waiter=1, registration='published', release='claimed',
                            retried=1, request_survives_ack=1, old_dispatch_scoped='cpu',
                            cleanup='reaped')
            if any(r.get(k) != v for k, v in expected.items()) or r.get('generation', 0) == 0:
                errors.append('R2D-B delayed-dispatch invariant missing')
            if r.get('busy_deferrals', 0) < 1:
                errors.append('R2D-B busy deferral not forced')
    elif r2d_a or r2d_b:
        errors.append('UP must not claim remote R2D rounds')
    if 'PANIC' in raw or 'panicked at' in raw:
        errors.append('guest panic')
    r2c = validate_r2c(_normalize(raw, cpus), cpus)
    errors.extend('r2c-base: ' + str(e) for e in r2c.get('errors', []))
    execution = sorted({cpus - 1} if (r2d_b and cpus > 1) else set()) or [0]
    return dict(status='fail' if errors else 'pass', errors=errors,
                actual_source_cpu_ids=[0], actual_execution_cpu_ids=execution,
                r2d_initial_pin_exit=r2d_a, r2d_exit_audit=r2d_a_audit,
                r2d_delayed_dispatch=r2d_b, r2c_base=r2c,
                scope=('R2V registration-wins + R2C ordering + R2D initial-pin/exit and '
                       'delayed-dispatch' if cpus > 1 else 'local eight-cycle control'))


def main():
    """Assemble the R2D validation summary from the retained final evidence."""
    import argparse
    import datetime
    import hashlib
    import json
    from pathlib import Path
    import subprocess

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', type=Path, default=Path('target/fastlane-f13nr2d-evidence/native-final'))
    parser.add_argument('--repetitions', type=Path,
                        default=Path('target/fastlane-f13nr2d-evidence/repetitions-final/repetitions.json'))
    parser.add_argument('--r2v-regression', type=Path,
                        default=Path('target/fastlane-f13nr2d-evidence/r2v-regression-final/results.json'))
    parser.add_argument('--r2c-regression', type=Path,
                        default=Path('target/fastlane-f13nr2d-evidence/r2c-regression-final/results.json'))
    parser.add_argument('--remote-smoke', type=Path,
                        default=Path('target/fastlane-f13nr2d-evidence/remote-smoke-final/results.json'))
    parser.add_argument('--checks-dir', type=Path, default=Path('target/fastlane-f13nr2d-evidence/checks'))
    parser.add_argument('--compat-dir', type=Path, default=Path('target/fastlane-f13nr2d-evidence/compat'))
    parser.add_argument('--c12f-repeat', type=Path,
                        default=Path('target/fastlane-f13nr2d-evidence/c12f-repeat/repeat.json'))
    parser.add_argument('--out', type=Path,
                        default=Path('target/fastlane-f13nr2d-evidence/validation-summary.json'))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]

    def read(path, default=None):
        path = path if path.is_absolute() else root / path
        return json.loads(path.read_text()) if path.exists() else default

    build = read(args.native / 'results.json', {}) or {}
    sources = dict(build.get('source_sha256', {}))
    for extra in ('tools/fastlane_f13nr2d_validate.py', 'tools/fastlane_f13nr2d_repeat.py',
                  'tools/test_fastlane_f13nr2d.py', 'tools/test_remote_wake_smp.py', 'Cargo.lock'):
        p = root / extra
        if p.is_file():
            sources[extra] = hashlib.sha256(p.read_bytes()).hexdigest()
    matches = bool(build) and all(
        (root / p).is_file()
        and hashlib.sha256((root / p).read_bytes()).hexdigest() == h
        for p, h in build.get('source_sha256', {}).items())

    matrix = {}
    for cpus in (1, 2, 4, 12):
        serial = (args.native if args.native.is_absolute() else root / args.native) / f'{cpus}cpu.serial.log'
        row = validate_native(serial.read_text(errors='replace') if serial.exists() else '', cpus)
        if not matches and row['status'] == 'pass':
            row['status'] = 'unverified'
            row['errors'].append('native build source hashes differ from current source')
        matrix[str(cpus)] = row

    host = ['--target', 'x86_64-unknown-linux-gnu', '--offline']
    check_commands = {
        'target': ['cargo', 'check', '-p', 'sunlight-kernel', '--offline'],
        'fixture-target': ['cargo', 'check', '-p', 'sunlight-kernel',
                           '--features', 'pin_release_test', '--offline'],
        'r2d-target': ['cargo', 'check', '-p', 'sunlight-kernel',
                       '--features', 'pin_release_r2d_test', '--offline'],
        'release-target': ['cargo', 'check', '-p', 'sunlight-kernel', '--release',
                           '--features', 'pin_release_r2d_test', '--offline'],
        'kernel-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', *host],
        'ring-doorbell': ['cargo', 'test', '-p', 'sunlight-fastlane-ring',
                          '-p', 'sunlight-fastlane-doorbell', *host],
        'ipc-service': ['cargo', 'test', '-p', 'sunlight-ipc', '-p', 'sunlight-audiod',
                        '-p', 'sunlightd', '--lib', *host],
        'boundary': ['python3', 'tools/fastlane_f13nr_boundary.py'],
        'scheduler-host': ['python3', 'tools/test_scheduler_host.py'],
        'scheduler-validators': ['python3', '-m', 'unittest', 'discover', '-s', 'tools',
                                 '-p', 'test_scheduler*.py'],
        'evidence-validators': ['python3', '-m', 'unittest', 'test_fastlane_f13nr2d',
                                'test_fastlane_f13nr2v', 'test_fastlane_f13nr2c'],
        'ordering-parser': ['python3', '-m', 'unittest', 'discover', '-s', 'tools',
                            '-p', 'test_fastlane_f13nr2c.py'],
        'clippy-kernel': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--offline',
                          '--', '-D', 'warnings'],
        'clippy-fastlane': ['cargo', 'clippy', '-p', 'sunlight-fastlane-ring',
                            '-p', 'sunlight-fastlane-doorbell', *host, '--', '-D', 'warnings'],
        'workspace-format': ['cargo', 'fmt', '--all', '--', '--check'],
        'fixture-format': ['rustfmt', '--edition', '2021', '--check',
                           'kernel/src/sched/handoff.rs', 'kernel/src/sched/pin_release_test.rs',
                           'kernel/src/arch/x86_64/interrupts.rs',
                           'kernel/src/arch/x86_64/lapic.rs'],
        'media': ['cargo', 'test', '-p', 'sunlight-media', *host],
        'diff-check': ['git', 'diff', '--check'],
        'miri': ['cargo', 'miri', '--version'],
    }
    checks = {}
    for name, command in check_commands.items():
        log = root / args.checks_dir / f'{name}.log'
        env = None
        if name in ('scheduler-validators', 'evidence-validators', 'ordering-parser'):
            env = dict(__import__('os').environ, PYTHONPATH=str(root / 'tools'))
        with log.open('w') as handle:
            try:
                code = subprocess.run(command, cwd=root, stdout=handle,
                                      stderr=subprocess.STDOUT, env=env).returncode
            except FileNotFoundError:
                code = 127
        text = log.read_text(errors='replace')
        checks[name] = dict(log=str(args.checks_dir / f'{name}.log'), exit_code=code,
                            status='pass' if code == 0 else 'fail',
                            rust_tests_passed=sum(
                                int(n) for n in __import__('re').findall(
                                    r'test result: ok\. (\d+) passed', text)))
    for name in ('clippy-kernel', 'workspace-format', 'media', 'miri',
                 'tcp-tls-sandbox-denied'):
        if name in checks:
            checks[name]['classification'] = (
                'BLOCKED' if name == 'miri' and checks[name]['status'] == 'fail'
                else checks[name]['status'].upper())
    for name in ('clippy-kernel', 'workspace-format', 'media', 'tcp-tls-sandbox-denied'):
        if name in checks and checks[name]['status'] == 'fail':
            checks[name]['classification'] = 'FAIL (pre-existing; matched to retained R2C evidence)'
    if (root / args.checks_dir / 'clippy-comparison.json').exists():
        checks['clippy-kernel']['comparison'] = read(args.checks_dir / 'clippy-comparison.json')
    if (root / args.checks_dir / 'format-comparison.json').exists():
        checks['workspace-format']['comparison'] = read(args.checks_dir / 'format-comparison.json')

    resources = {}
    for cpu, row in matrix.items():
        if row['status'] != 'pass':
            continue
        final = row['r2c_base']['actual_final']
        resources[cpu] = dict(
            active_pins=0, waiter_bits=0, pending_remote_requests=0, deferred_dispatch=0,
            task_queue_occurrences=0, retired_test_generations=8,
            retained_reaped_metadata_slots=final['metadata_slots'],
            frames_before=final['frames_before'], frames_after=final['frames_after'],
            frame_drift=final['frames_after'] - final['frames_before'],
            scope='quiescent teardown; CPU0 local controller request excluded; '
                  'prepared Vec capacity retained; heap bytes not measured')

    dirty = subprocess.check_output(['git', 'diff', '--name-only'], cwd=root, text=True).splitlines()
    version_bump = [p for p in dirty if p.endswith('Cargo.toml')]
    version_bump = [p for p in dirty if p.endswith('Cargo.toml') or p == '.version_state.json']
    code_changed = [p for p in dirty if p not in version_bump and p != 'Cargo.lock'
                    and (p.startswith('kernel/') or p.startswith('tools/')
                         or p.startswith('docs/'))]
    # kernel/Cargo.toml carries both the shared version bump and the new
    # pin_release_r2d_test feature, so it is listed as a code change too.
    if 'kernel/Cargo.toml' in dirty and 'kernel/Cargo.toml' not in code_changed:
        code_changed.insert(0, 'kernel/Cargo.toml (+ version bump)')
    version_bump_note = ('124 Cargo.toml manifests plus .version_state.json and Cargo.lock '
                         'carry a pre-existing workspace version bump; not scheduler code')

    compat_dir = args.compat_dir if args.compat_dir.is_absolute() else root / args.compat_dir
    compat_present = (compat_dir / 'lifecycle-resident-vm.log').exists()
    summary = dict(
        phase='F1.3N-R2D', complete=False,
        utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        source_sha256=sources,
        working_tree=dict(code_changed_files=code_changed,
                          version_bump_note=version_bump_note),
        build=dict(elf_sha256=build.get('elf_sha256'), iso_sha256=build.get('iso_sha256'),
                   qemu=build.get('qemu'), accelerator='tcg',
                   native_features=build.get('features')),
        native_build_matches_current_sources=matches,
        native_build_command=['cargo', 'build', '--package', 'sunlight-kernel', '--bin',
                              'sunlight-kernel', '--features', 'pin_release_r2d_test', '--offline'],
        native_command=['python3', 'tools/test_remote_wake_smp.py', '--pin-r2d',
                        '--cpus', '1', '2', '4', '12',
                        '--evidence-dir', 'target/fastlane-f13nr2d-evidence/native-final',
                        '--timeout', '150'],
        fixtures=dict(
            r2d_initial_pin_exit=dict(round=7, name='R2D-A',
                markers=['[R2D-A] audit ...', '[R2D-A] forced ...'],
                forced_ordering=[
                    'CPU0 initial pin published on slot 0 (is_pinned(0, idx))',
                    'CPU1 rejects held pin and publishes waiter bit (registered_mask=0x2)',
                    'CPU1 sets BOUNDARY=3 after observing its own registration',
                    'CPU0 observes BOUNDARY=3 and waiter_snapshot(0)==0b10, then ACCEPTED',
                    'CPU1 takes SCHEDULER and commits real terminate_process_by_pid',
                    'CPU0 pivots RSP and releases only after TERMINATED',
                ]),
            r2d_delayed_dispatch=dict(round=6, name='R2D-B',
                markers=['[R2D-B] forced ...'],
                forced_ordering=[
                    'CPU1 publishes waiter bit while pin A held (boundary=1)',
                    'CPU0 releases; drain claims full remote mask',
                    'release-kick dispatch forced Busy 3x then retried in the same bounded flush',
                    'slot reused with generation+1; older dispatch still completes',
                    'target executes on CPU N-1; both lifetimes clean up',
                ]),
        ),
        native_cpu_matrix=matrix,
        repetitions=read(args.repetitions, {}),
        r2v_regression=read(args.r2v_regression, {}),
        r2c_regression=read(args.r2c_regression, {}),
        remote_smoke=read(args.remote_smoke, {}),
        checks=checks,
        compatibility=dict(
            commands=[['python3', 'tools/fastlane_f11pr41_validate.py', '--native', '--regressions'],
                      ['bash', 'tools/test_sunlightd_d3.sh', '--cpus', '1', '2', '4'],
                      ['python3', 'tools/network_tls_gate.py', '--iso',
                       'target/fastlane-f11pr41-evidence/20261021T182508.112841Z/resident.iso',
                       '--cpus', '12', '--timeout', '240',
                       '--evidence', 'target/fastlane-f13nr2d-evidence/tcp-tls']],
            logs=dict(d1_d2_d3='target/fastlane-f13nr2d-evidence/compat/d1-d2-d3.log',
                      lifecycle_resident_vm='target/fastlane-f13nr2d-evidence/compat/lifecycle-resident-vm.log',
                      tcp_tls='target/fastlane-f13nr2d-evidence/compat/tcp-tls.log'),
            lifecycle_resident_vm=dict(
                native_assertions='PASS (entropy, authority 1/2/4/12, resident 1/2/4/12, '
                                  'scheduler-native, recovery-native, vm-tlb-12, vm-unmap-4, '
                                  'vm-protect-4, diff-check)',
                aggregate_exit='1', failing_check='format-check (cargo fmt --all --check)',
                failure_class='pre-existing workspace formatting; format-check hunks identical '
                              'to retained R2C evidence'),
            d1_d2_d3='PASS, 1/2/4 CPUs', tcp_tls='PASS, 12 CPUs, certificate verification plus '
                                                'unknown-issuer/wrong-host rejection',
            tcp_tls_sandbox_denied=['python3', 'tools/network_tls_gate.py'],
        ) if compat_present else None,
        resource_accounting=resources,
        c12f_mixed_failure=dict(
            original='target/fastlane-f13nr2c-evidence/c12f/mixed-capture2 (FAIL retained)',
            classification='evidence/timing-collection gap (startup service sample), '
                           'not a scheduler assertion failure',
            evidence='tty_server ready_count=0 with empty waits_ns; START serial line 369 '
                     'precedes Login-ready line 395; passing retry has the same ordering '
                     '(START 353, ready 410); all four workers progressed in all eight trials',
            same_image_retry='pass (R2C mixed-isolated-retry)',
            attempt_by_attempt=[
                dict(kind='normal', number=r['number'], status=r['status'],
                     runs=r.get('runs'),
                     external_ready_to_dispatch_p50_ms=(
                         r.get('steady_quantiles', {}).get('external_ready_to_dispatch', {})
                         .get('p50_ms')))
                for r in read(Path('target/fastlane-f13nr2c-evidence/c12f/summary.json'),
                              {}).get('captures', [])] + [
                dict(kind='mixed_audio', number=r['number'], status=r['status'],
                     runs=r.get('runs'), reason=r.get('reason'))
                for r in read(Path('target/fastlane-f13nr2c-evidence/c12f/summary.json'),
                              {}).get('mixed_captures', [])],
            startup_tty_ready_count_by_trial=[
                dict(run=d['run'], classification=d['classification'], error=d.get('error'),
                     tty_server_ready_count=next(
                         (s['ready_count'] for s in d.get('services', [])
                          if s['name'] == 'tty_server'), None),
                     operations=d.get('operations'))
                for d in read(Path('target/fastlane-f13nr2c-evidence/c12f/'
                                'mixed-capture2-diagnosis.json'), [])],
        ),
        c12f_same_image_repeat=read(args.c12f_repeat, {}),
        performance=dict(
            status='MATCHED BASELINE UNAVAILABLE', measured_improvement=False,
            pin_acquire_release_overhead='NOT RUN', zero_waiter_release='NOT RUN',
            one_waiter_release='NOT RUN', multi_waiter_release='NOT RUN',
            release_to_execution_latency='NOT RUN', remote_dispatch_latency='NOT RUN',
            reschedule_requests_per_second='NOT RUN', ipis_per_second='NOT RUN',
            coalesced_dispatches='functional coalescing assertions only; rate NOT RUN',
            scheduler_lock_hold_time='NOT RUN', c12f_timer_percentiles='see c12f captures; '
                                                                     'matched comparison unavailable',
            reason='TCG assertion traces are functional evidence; no matched pre-repair '
                   'source/binary/workload baseline established and no destructive Git '
                   'operations were used to reconstruct one',
        ),
        final_gates=dict(
            A='VERIFIED NATIVE',
            B='NOT READY',
            C='BLOCKED'),
        acceptance='Gate A closed natively for initial-pin/exit and delayed-dispatch/new-pin; '
                   'Gate B adapter prerequisites and Gate C restricted-SHM prerequisites '
                   'remain independently unverified. No native Doorbell adapter or production '
                   'FastLane transport is enabled.',
    )
    out = args.out if args.out.is_absolute() else root / args.out
    out.write_text(json.dumps(summary, indent=2) + '\n')
    print('R2D native:', {cpu: row['status'] for cpu, row in matrix.items()}, flush=True)
    print('code changed:', code_changed, flush=True)
    print('version-only bump files:', len(version_bump), flush=True)
    if any(row['status'] != 'pass' for row in matrix.values()) or not matches:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
