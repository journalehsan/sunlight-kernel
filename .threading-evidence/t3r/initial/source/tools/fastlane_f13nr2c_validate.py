#!/usr/bin/env python3
"""R2C evidence checks; old R2V registration-wins assertions stay mandatory."""
from fastlane_f13nr2v_validate import fields, validate_native as validate_r2v


def validate_native(raw, cpus):
    lines = []
    extra = []
    ordering = []
    terminals = []
    reuse = []
    final = {}
    for line in raw.splitlines():
        row = fields(line)
        if line.startswith('[R2V] cycle ') and row.get('round', 0) >= 4:
            extra.append(row)
            continue
        if line.startswith('[R2C] ordering '):
            ordering.append(row)
        if line.startswith('[R2C] reuse '):
            reuse.append(row)
        if line.startswith('[R2C] terminal '):
            terminals.append(row)
        if line.startswith('[R2V] PASS '):
            final = row
            line = line.replace('rounds=6', 'rounds=4').replace('retired=6', 'retired=4')
        lines.append(line)
    result = validate_r2v('\n'.join(lines), cpus)
    errors = result['errors']
    if result['status'] != 'pass':
        return result
    if final.get('rounds') != 6 or final.get('retired') != 6 or final.get('metadata_slots') != 1:
        errors.append('six real lifecycle completions required')
    if len(terminals) != 6 or {r.get('round') for r in terminals} != set(range(6)):
        errors.append('six terminal boundary audits required')
    for r in terminals:
        expected = dict(pin='held', state='Finished', stack='retained', queued=0,
                        late_wake='retired', duplicate_exit='rejected',
                        path='terminate_current_user_process' if r.get('round') == 3 else 'process_exit',
                        owner=(1 if r.get('round', 0) >= 4 else cpus - 1) if cpus > 1 else 0, auditor=0)
        if any(r.get(k) != v for k, v in expected.items()) or r.get('generation', 0) == 0:
            errors.append('terminal identity/lifetime invariant missing')
    if len(extra) != 2 or {r.get('round') for r in extra} != {4, 5}:
        errors.append('missing ordering cycles')
    for r in extra:
        expected = dict(source=0, execution=1 if cpus > 1 else 0, rejected=0,
                        timer_assistance=0, pins=0, waiters=0, queued=0, cleanup='reaped')
        if any(r.get(k) != v for k, v in expected.items()):
            errors.append('ordering cycle execution/cleanup mismatch')
    if len(reuse) != 6 or {r.get('round') for r in reuse} != set(range(6)):
        errors.append('six task-table assignment markers required')
    generations = [r.get('generation') for r in reuse]
    if len(set(generations)) != 6 or any(not g for g in generations):
        errors.append('task-table reuse must have distinct live generations')
    for r in reuse:
        if any(r.get(k) != v for k, v in dict(task=0, old_wake='retired', pins=0, waiters=0, dispatch='quiescent').items()):
            errors.append('unsafe or missing quiescent table reuse')
    for r in terminals + ordering:
        assignments = [a for a in reuse if a.get('round') == r.get('round')]
        if len(assignments) != 1 or any(r.get(k) != assignments[0].get(k) for k in ['task', 'generation']):
            errors.append('lifecycle identity differs from assigned generation')
    if cpus > 1:
        if len(ordering) != 2 or {r.get('boundary') for r in ordering} != {1, 2}:
            errors.append('both forced ordering boundaries required')
        for r in ordering:
            expected = dict(owner=0, waiter=1, execution=1, recheck='released',
                            timer_assistance=0, pins=0, waiters=0, queued=0, cleanup='reaped')
            if any(r.get(k) != v for k, v in expected.items()):
                errors.append('missing release-wins progress chain')
            if r.get('round') != r.get('boundary', 0) + 3 or r.get('claimed') not in [0, 2]:
                errors.append('invalid boundary or claimed mask')
            if r.get('generation', 0) == 0 or 'task' not in r:
                errors.append('missing task identity')
    elif ordering:
        errors.append('UP must not claim remote ordering')
    result.update(status='fail' if errors else 'pass', ordering=ordering,
                  extra_cycles=extra, terminal_audits=terminals, task_table_reuse=reuse, actual_final=final,
                  actual_execution_cpu_ids=sorted({r['owner'] for r in terminals}),
                  target_execution_count=6, total_wait_release_calls=4 * (cpus - 1) + (2 if cpus > 1 else 0),
                  total_claimed_waiter_bit_events=result['release_notification_bits'] + sum(r.get('released', 0).bit_count() for r in extra),
                  counter_scope='R2V request/send/rejection counters cover first four registration-wins rounds; new boundaries separately recorded in ordering and extra_cycles', scope='R2V registration-wins plus forced release-before-publication and release-before-recheck' if cpus > 1 else 'local six-cycle control')
    return result


def main():
    import argparse
    import datetime
    import hashlib
    import json
    from pathlib import Path
    import subprocess
    import sys
    import fastlane_f13nr2v_validate as previous

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checks', action='store_true')
    parser.add_argument('--compatibility', action='store_true')
    parser.add_argument('--native', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    out = root / 'target/fastlane-f13nr2c-evidence'
    out.mkdir(parents=True, exist_ok=True)
    previous.OUT = out
    if args.checks or args.compatibility:
        saved = sys.argv
        sys.argv = [saved[0]] + (['--checks'] if args.checks else []) + (['--compatibility'] if args.compatibility else [])
        previous.main()
        sys.argv = saved
    def read(path, default=None):
        return json.loads(path.read_text()) if path.exists() else default
    checks = read(out / 'checks.json', {})
    checks.update(read(out / 'extra-checks.json', {}))
    if args.native:
        checks['native-r2c'] = previous.run('native-r2c', ['python3', 'tools/test_remote_wake_smp.py',
            '--pin-ordering', '--evidence-dir', str(out / 'native'), '--timeout', '120'], 900)
    if args.checks:
        for name, command in {
            'ordering-parser': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_fastlane_f13nr2c.py'],
            'ordering-target': ['cargo', 'check', '-p', 'sunlight-kernel', '--features', 'pin_release_ordering_test', '--offline'],
            'release-target': ['cargo', 'check', '-p', 'sunlight-kernel', '--release', '--features', 'pin_release_ordering_test', '--offline'],
            'clippy-ordering': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--features', 'pin_release_ordering_test', '--no-deps', '--offline', '--', '-D', 'warnings'],
            'r2c-format': ['rustfmt', '--edition', '2021', '--check', 'kernel/src/sched/handoff.rs', 'kernel/src/sched/pin_release_test.rs', 'kernel/src/arch/x86_64/interrupts.rs'],
            'miri': ['cargo', 'miri', '--version'],
        }.items():
            checks[name] = previous.run(name, command)
        (out / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
    (out / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
    build = read(out / 'native/results.json', {})
    matches = bool(build) and all((root / p).exists() and hashlib.sha256((root / p).read_bytes()).hexdigest() == h
                                 for p, h in build.get('source_sha256', {}).items())
    matrix = {}
    for cpus in [1, 2, 4, 12]:
        serial = out / f'native/{cpus}cpu.serial.log'
        matrix[str(cpus)] = validate_native(serial.read_text(errors='replace') if serial.exists() else '', cpus)
        if not matches and matrix[str(cpus)]['status'] == 'pass':
            matrix[str(cpus)]['status'] = 'unverified'
            matrix[str(cpus)]['errors'].append('build source differs from current source')
    sources = set(build.get('source_sha256', {})) | {'tools/fastlane_f13nr2c_validate.py',
        'tools/test_fastlane_f13nr2c.py', 'tools/test_remote_wake_smp.py',
        'docs/kernel/scheduler-remote-wake-f13nr2c.md', 'Cargo.lock', 'Cargo.toml',
        'rust-toolchain.toml', '.cargo/config.toml',
        'fastlane-ring/Cargo.toml', 'fastlane-ring/src/lib.rs',
        'fastlane-doorbell/Cargo.toml', 'fastlane-doorbell/src/lib.rs',
        'fastlane-doorbell/src/tests.rs'}
    hashes = {}
    for p in sorted(sources):
        if (root / p).exists():
            data = (root / p).read_bytes()
            hashes[p] = hashlib.sha256(data).hexdigest()
            dest = out / 'source' / p
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
    (out / 'worktree.patch').write_bytes(subprocess.check_output(['git', 'diff'], cwd=root))
    resources = {}
    for cpu, row in matrix.items():
        if row['status'] == 'pass':
            final = row['actual_final']
            resources[cpu] = dict(active_pins=0, waiter_bits=0, pending_remote_requests=0,
                deferred_dispatch=0, task_queue_occurrences=0, active_test_tasks=0,
                retired_test_generations=6, retained_reaped_metadata_slots=final['metadata_slots'],
                frames_before=final['frames_before'], frames_after=final['frames_after'],
                frame_drift=final['frames_after'] - final['frames_before'],
                scope='quiescent teardown; CPU0 local controller request excluded; prepared Vec capacity retained; heap bytes not measured')
    for row in matrix.values():
        row['classification'] = {'pass': 'PASS', 'fail': 'FAIL', 'not_run': 'NOT RUN'}.get(row['status'], 'BLOCKED')
    for name, row in checks.items():
        row['classification'] = 'BLOCKED' if name in {'miri', 'tcp-tls-sandbox-denied'} and row['status'] == 'fail' else row['status'].upper()
    compatibility = read(out / 'compatibility/validation-summary.json', {})
    c12f = read(out / 'c12f/summary.json', {})
    regression_matrix = {name: row['classification'] for name, row in checks.items()}
    regression_matrix.update({f'pin-ordering-{cpu}cpu': row['classification'] for cpu, row in matrix.items()})
    for row in compatibility.get('native', []):
        regression_matrix[row['name']] = 'PASS' if row['pass'] else 'FAIL'
    for row in compatibility.get('checks', []):
        regression_matrix['compat-' + row['name']] = 'PASS' if row['exit_code'] == 0 else 'FAIL'
    for kind in ['captures', 'mixed_captures']:
        for row in c12f.get(kind, []):
            regression_matrix[f'c12f-{kind}-{row["number"]}'] = row['status'].upper()
    retry = read(out / 'c12f/mixed-isolated-retry.json', {})
    regression_matrix['c12f-mixed-isolated-retry'] = retry.get('status', 'NOT RUN').upper()
    summary = dict(phase='F1.3N-R2C', complete=False,
        utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        source_sha256=hashes, build=build, native_build_matches_current_sources=matches,
        native_build_command=['cargo', 'build', '--package', 'sunlight-kernel', '--bin', 'sunlight-kernel', '--features', 'pin_release_ordering_test', '--offline'],
        native_command=['python3', 'tools/test_remote_wake_smp.py', '--pin-ordering', '--evidence-dir', str(out / 'native'), '--timeout', '120'],
        native_cpu_matrix=matrix, checks=checks, resource_accounting=resources,
        compatibility=compatibility, regression_matrix=regression_matrix,
        c12f=c12f, c12f_isolated_retry=read(out / 'c12f/mixed-isolated-retry.json'),
        performance=dict(status='MATCHED BASELINE UNAVAILABLE', measured_improvement=False,
            pin_release_overhead='NOT RUN', one_waiter_release_overhead='NOT RUN',
            multi_waiter_release_overhead='NOT RUN', release_to_scheduling_latency='NOT RUN',
            release_to_execution_latency='NOT RUN', waiter_release_latency='NOT RUN', remote_ipis_per_second='NOT RUN',
            coalesced_requests='functional coalescing assertions only; full rate NOT RUN',
            context_switches='pin-specific matched measurement NOT RUN',
            lock_hold_time='NOT RUN', smp_tail_regression='NOT RUN',
            reason='TCG assertion traces are functional evidence; no matched pre-repair source/binary/workload baseline established'),
        ordering=dict(registration_wins='see four R2V cycles per native row',
            release_wins='boundary=1 before publication; same selection pass executes task',
            publication_release_recheck='boundary=2; same selection pass executes task; drain may claim or waiter may withdraw',
            simultaneous_registration='selector registrations serialize under S; release proceeds without S',
            exact_exchange_timestamp='not instrumented; EMPTY observation bounds actual production xchg'),
        release_paths=dict(reschedule='forced native waiters and both ordering boundaries',
            timer='NOT RUN with forced waiters', process_exit='native terminal pin/reaper/late-wake audit',
            terminate_current_user_process='native UD2 terminal pin/reaper/late-wake audit'),
        limitations=['current mixed/audio capture 2 failed its startup TTY service-sample requirement',
            'exit during initial waiter registration not forced',
            'exit after rejection before the original pin release not forced',
            'delayed A dispatch overlapping C pin not forced; task table reuse is quiescent',
            'steal_work pinned rejection not forced', 'alternate-work/new-task wake overlap not forced',
            'exact simultaneous publication/extraction instruction boundary not timestamped',
            'AP timers masked only in isolated fixture; CPU0 control timer retained',
            'sparse/high APIC IDs unsupported by evidence; tested contiguous QEMU IDs only',
            'heap-byte/reference counters and matched performance unavailable',
            'atomic conditional Doorbell block/wake handoff remains independently unverified'],
        final_gates=dict(A='PARTIALLY VERIFIED', B='NOT READY', C='BLOCKED'),
        acceptance='INCOMPLETE: retain partial classification; no adapter, Restricted SHM, or production transport enabled')
    (out / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('R2C native:', {cpu: row['status'] for cpu, row in matrix.items()}, flush=True)
    if args.native and (not matches or any(r['status'] != 'pass' for r in matrix.values())):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
