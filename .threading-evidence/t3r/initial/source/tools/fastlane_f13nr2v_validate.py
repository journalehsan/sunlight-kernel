#!/usr/bin/env python3
"""Collect current-build native pin-release assertions and compatibility evidence."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13nr2v-evidence'


def fields(line):
    return {key: int(value, 0) if re.fullmatch(r'0x[0-9a-f]+|[0-9]+', value) else value
            for key, value in re.findall(r'(\w+)=(\S+)', line)}


def validate_native(raw, cpus):
    identities = [fields(line) for line in raw.splitlines() if line.startswith('[R2V] cpu ')]
    cycles = [fields(line) for line in raw.splitlines() if line.startswith('[R2V] cycle ')]
    waiters = [fields(line) for line in raw.splitlines() if line.startswith('[R2V] waiter ')]
    finals = [fields(line) for line in raw.splitlines() if line.startswith('[R2V] PASS ')]
    result = dict(status='not_run', actual_source_cpu_ids=[], actual_execution_cpu_ids=[],
                  cycles=cycles, waiters=waiters, errors=[])
    if not finals:
        result['errors'].append('forced fixture completion not reached')
        if 'PANIC' in raw or 'panicked at' in raw:
            result['status'] = 'fail'
        return result
    expected = ((1 << cpus) - 1) & ~1
    errors = result['errors']
    final = finals[-1]
    if len(identities) != cpus or {row.get('cpu') for row in identities} != set(range(cpus)):
        errors.append('missing actual CPU identities')
    if any(row.get('cpu') != row.get('apic') for row in identities):
        errors.append('APIC identity mismatch')
    if len(finals) != 1 or final.get('cpus') != cpus or final.get('rounds') != 4:
        errors.append('invalid completion identity')
    if len(cycles) != 4 or {row.get('round') for row in cycles} != set(range(4)):
        errors.append('missing or duplicate cycles')
    for row in cycles:
        for key, value in dict(source=0, execution=cpus - 1, rejected=expected,
                               released=expected, timer_assistance=0, pins=0, waiters=0,
                               queued=0, cleanup='reaped').items():
            if row.get(key) != value:
                errors.append(f'cycle {row.get("round")} {key} mismatch')
    if len(waiters) != 4 * (cpus - 1):
        errors.append('wrong waiter count')
    for round_id in range(4):
        rows = [row for row in waiters if row.get('round') == round_id]
        if {row.get('cpu') for row in rows} != set(range(1, cpus)):
            errors.append(f'cycle {round_id} waiter identity mismatch')
        for row in rows:
            if any(row.get(key) != 1 for key in ['rejected', 'ack', 'accepted', 'handled', 'pass']):
                errors.append(f'cycle {round_id} incomplete progress chain')
            if row.get('registered_mask', 0) & (1 << row['cpu']) == 0:
                errors.append(f'cycle {round_id} missing waiter bit')
            if not all(1 <= row.get(key, 0) <= 4 for key in ['sends', 'requests']):
                errors.append(f'cycle {round_id} missing or excessive dispatch')
    if final.get('frames_before') != final.get('frames_after'):
        errors.append('frame drift')
    if final.get('forced') != 4 * (cpus - 1) or final.get('retired') != 4:
        errors.append('wrong execution/rejection totals')
    if final.get('timer_assistance') != 0:
        errors.append('timer assistance')
    if 'PANIC' in raw or 'panicked at' in raw:
        errors.append('guest panic')
    result.update(status='fail' if errors else 'pass', final=final,
                  actual_source_cpu_ids=[0], actual_execution_cpu_ids=[cpus - 1],
                  rejection_count=len(waiters), waiter_registration_count=len(waiters),
                  forced_release_count=len(cycles) if cpus > 1 else 0,
                  scheduler_pass_count_lower_bound=len(waiters),
                  ipi_handler_count_lower_bound=len(waiters),
                  release_notification_bits=sum(row['released'].bit_count() for row in cycles),
                  reschedule_request_count=sum(row.get('requests', 0) for row in waiters),
                  accepted_ipi_count=sum(row.get('sends', 0) for row in waiters),
                  target_execution_count=len(cycles), timer_assistance=0,
                  counter_scope='unique per-cycle waiter rejection bits; post-release sends and requests to registered waiter CPUs, including final exit self-IPI; excludes control and local no-waiter IPIs',
                  scope='local cleanup control' if cpus == 1 else 'forced registration-wins RR migration')
    return result


def run(name, command, timeout=600):
    start = time.monotonic()
    with (OUT / f'{name}.log').open('w') as log:
        try:
            code = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                  timeout=timeout).returncode
        except subprocess.TimeoutExpired:
            code = 124
    return dict(command=command, exit_code=code, seconds=time.monotonic() - start,
                status='pass' if code == 0 else 'fail', log=f'{name}.log',
                rust_tests_passed=sum(map(int, re.findall(r'test result: ok\. (\d+) passed',
                    (OUT / f'{name}.log').read_text(errors='replace')))))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', action='store_true')
    parser.add_argument('--checks', action='store_true')
    parser.add_argument('--compatibility', action='store_true')
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    checks_path = OUT / 'checks.json'
    checks = json.loads(checks_path.read_text()) if checks_path.exists() else {}
    if args.checks:
        host = ['--target', 'x86_64-unknown-linux-gnu', '--offline']
        commands = {
            'boundary': ['python3', 'tools/fastlane_f13nr_boundary.py'],
            'scheduler-host': ['python3', 'tools/test_scheduler_host.py'],
            'kernel-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', *host],
            'ring-doorbell': ['cargo', 'test', '-p', 'sunlight-fastlane-ring', '-p', 'sunlight-fastlane-doorbell', *host],
            'ipc-service': ['cargo', 'test', '-p', 'sunlight-ipc', '-p', 'sunlight-audiod', '-p', 'sunlightd', '--lib', *host],
            'scheduler-validators': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_scheduler*.py'],
            'evidence-validators': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_fastlane_f13nr2v.py'],
            'target': ['cargo', 'check', '-p', 'sunlight-kernel', '--offline'],
            'fixture-target': ['cargo', 'check', '-p', 'sunlight-kernel', '--features', 'pin_release_test', '--offline'],
            'clippy-kernel': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--offline', '--', '-D', 'warnings'],
            'clippy-fastlane': ['cargo', 'clippy', '-p', 'sunlight-fastlane-ring', '-p', 'sunlight-fastlane-doorbell', *host, '--', '-D', 'warnings'],
            'media': ['cargo', 'test', '-p', 'sunlight-media', *host],
            'workspace-format': ['cargo', 'fmt', '--all', '--', '--check'],
            'fixture-format': ['rustfmt', '--edition', '2021', '--check', 'kernel/src/sched/pin_release_test.rs', 'kernel/src/sched/handoff.rs'],
            'diff-check': ['git', 'diff', '--check'],
        }
        for name, command in commands.items():
            checks[name] = run(name, command)
            checks_path.write_text(json.dumps(checks, indent=2) + '\n')
            print(name, checks[name]['status'], flush=True)
    if args.native:
        checks['native'] = run('native', ['python3', 'tools/test_remote_wake_smp.py', '--pin-release',
            '--evidence-dir', str(OUT / 'native'), '--timeout', '120'], timeout=900)
        checks_path.write_text(json.dumps(checks, indent=2) + '\n')
    if args.compatibility:
        commands = {
            'remote-smoke': (['python3', 'tools/test_remote_wake_smp.py', '--evidence-dir', str(OUT / 'remote-smoke')], 900),
            'lifecycle-resident-vm': (['python3', 'tools/fastlane_f11pr41_validate.py', '--native', '--regressions'], 7200),
            'd1-d2-d3': (['bash', 'tools/test_sunlightd_d3.sh', '--cpus', '1', '2', '4'], 1800),
        }
        for name, (command, timeout) in commands.items():
            checks[name] = run(name, command, timeout)
            checks_path.write_text(json.dumps(checks, indent=2) + '\n')
            print(name, checks[name]['status'], flush=True)
        prior = ROOT / 'target/fastlane-f11pr41-evidence/validation-summary.json'
        if prior.exists():
            compat = json.loads(prior.read_text())
            original = ROOT / compat['raw_directory']
            shutil.copytree(original, OUT / 'compatibility', dirs_exist_ok=True)
            resident = OUT / 'compatibility/resident.iso'
            if resident.exists():
                checks['tcp-tls'] = run('tcp-tls', ['python3', 'tools/network_tls_gate.py',
                    '--iso', str(resident), '--cpus', '12', '--timeout', '240',
                    '--evidence', str(OUT / 'tcp-tls')], 900)
                checks_path.write_text(json.dumps(checks, indent=2) + '\n')
        for name in ['d3-evidence', 'd36r-evidence']:
            origin = ROOT / 'target' / name
            if origin.exists():
                shutil.copytree(origin, OUT / name, dirs_exist_ok=True)
    matrix = {}
    for cpu in [1, 2, 4, 12]:
        serial = OUT / 'native' / f'{cpu}cpu.serial.log'
        matrix[str(cpu)] = validate_native(serial.read_text(errors='replace') if serial.exists() else '', cpu)
    sources = sorted(set(subprocess.check_output(['git', 'diff', '--name-only'], cwd=ROOT,
                         text=True).splitlines()) | {
        'kernel/src/sched/pin_release_test.rs', 'kernel/src/sched/remote_wake_test.rs',
        'tools/fastlane_f13nr2v_validate.py', 'tools/test_fastlane_f13nr2v.py',
        'tools/test_remote_wake_smp.py', 'docs/kernel/scheduler-remote-wake-f13nr2v.md',
        'tools/fastlane_f13n_boundary.py', 'tools/fastlane_f13nr_boundary.py',
        'fastlane-ring/Cargo.toml', 'fastlane-ring/src/lib.rs',
        'fastlane-doorbell/Cargo.toml', 'fastlane-doorbell/src/lib.rs',
        'fastlane-doorbell/src/tests.rs', 'rust-toolchain.toml', '.cargo/config.toml',
        'kernel/src/sched/quantum.rs'})
    hashes = {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
              for path in sources if (ROOT / path).is_file()}
    (OUT / 'worktree.patch').write_bytes(subprocess.check_output(['git', 'diff'], cwd=ROOT))
    for path in hashes:
        dest = OUT / 'source' / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes((ROOT / path).read_bytes())
    manifest = OUT / 'native/results.json'
    build = json.loads(manifest.read_text()) if manifest.exists() else None
    build_matches = bool(build) and all(
        (ROOT / path).is_file() and hashlib.sha256((ROOT / path).read_bytes()).hexdigest() == digest
        for path, digest in build.get('source_sha256', {}).items())
    if not build_matches:
        for row in matrix.values():
            if row['status'] == 'pass':
                row['status'] = 'unverified'
                row['errors'].append('native build source hashes differ from current source')
    compatibility = OUT / 'compatibility/validation-summary.json'
    c12f = OUT / 'c12f/summary.json'
    resources = {cpu: dict(frame_drift=row['final']['frames_after'] - row['final']['frames_before'],
        metadata_slots=row['final']['metadata_slots'], retired=row['final']['retired'],
        active_pins=0, active_waiter_bits=0, pending_remote_requests=0, pending_dispatch=0,
        final_task_queue_occurrences=0) for cpu, row in matrix.items() if row['status'] == 'pass'}
    summary = dict(phase='F1.3N-R2V', complete=False,
        status='partial: forced native registration-wins progress verified; remaining acceptance gates blocked',
        utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        revision=subprocess.check_output(['git', 'show', '-s', '--format=%H', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_sha256=hashes, checks=checks, native_cpu_matrix=matrix,
        build=build, native_build_matches_current_sources=build_matches,
        performance=dict(status='MATCHED BASELINE UNAVAILABLE',
                         c12f_native_latency='current-only capture; matched regression unverified' if c12f.exists() else 'unverified',
                         matched_timing_claims=False, pin_release_latency='unmeasured',
                         current_c12f=json.loads(c12f.read_text()) if c12f.exists() else None),
        compatibility=json.loads(compatibility.read_text()) if compatibility.exists() else None,
        resource_accounting=resources,
        resource_limits=['heap allocation byte accounting unverified',
                         'CPU0 ordinary local timer control request excluded from remote teardown',
                         'no pending-notification overlap with process-table slot reuse forced'],
        release_paths=dict(timer='unverified with waiters', reschedule='see native matrix',
                           terminate_current_user_process='unverified', process_exit='cleanup only; no waiter forced'),
        unverified=['native release-wins registration boundary', 'concurrent registration boundary',
                    'exit concurrent with rejection', 'process table slot reuse', 'steal_work rejection',
                    'matched C1.2F latency and performance', 'strict complete kernel Clippy'],
        compatibility_pass=({'native', 'remote-smoke', 'lifecycle-resident-vm', 'd1-d2-d3', 'tcp-tls'} <= checks.keys()
                            and all(row['status'] == 'pass' for row in checks.values())),
        final_gates=dict(pin_release_repair='PARTIALLY VERIFIED', remote_wake_scheduler='NOT READY',
                         native_fastlane_transport='BLOCKED pending restricted SHM authorization and lifecycle verification'))
    (OUT / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    if args.native and (not build_matches or any(row['status'] != 'pass' for row in matrix.values())):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
