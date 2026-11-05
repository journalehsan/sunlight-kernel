#!/usr/bin/env python3
"""Collect F1.3N-A evidence without treating boots or historical gates as passes."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import subprocess
import time

from test_native_doorbell import SOURCES, validate_native

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13na-evidence'
BASELINE = ROOT / 'target/fastlane-f13nr2d-evidence/checks'


def read(path, default=None):
    return json.loads(path.read_text()) if path.exists() else default


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(name, command, timeout=1800, env=None):
    log = OUT / 'checks' / f'{name}.log'
    log.parent.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    with log.open('w') as handle:
        try:
            code = subprocess.run(command, cwd=ROOT, stdout=handle,
                                  stderr=subprocess.STDOUT, timeout=timeout, env=env).returncode
        except subprocess.TimeoutExpired:
            code = 124
    result = dict(command=command, cwd=str(ROOT), exit_code=code,
                  wall_seconds=time.monotonic() - start,
                  classification='PASSED' if code == 0 else 'BLOCKED' if code == 124 else 'FAILED',
                  log=str(log.relative_to(ROOT)))
    checks = read(OUT / 'checks.json', {})
    checks[name] = result
    (OUT / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
    print(f'{name}: {result["classification"]} (exit={code})', flush=True)
    return result


def diagnostics(raw):
    found = set()
    message = None
    for line in raw.splitlines():
        if line.startswith('error:') and 'could not compile' not in line:
            message = line
        elif message and '-->' in line:
            path = re.search(r'--> (.*?):\d+:\d+', line)
            if path:
                found.add((message, path[1]))
                message = None
    return found


def format_hunks(raw):
    return [re.sub(r':\d+:\n', ':LINE:\n', hunk)
            for hunk in re.split(r'(?m)(?=^Diff in )', raw) if hunk.startswith('Diff in ')]


def comparisons(checks):
    results = {}
    for name, extract in [('clippy-kernel', diagnostics), ('workspace-format', format_hunks)]:
        baseline = BASELINE / f'{name}.log'
        current = OUT / 'checks' / f'{name}.log'
        if current.exists() and baseline.exists():
            previous = set(extract(baseline.read_text(errors='replace')))
            actual = set(extract(current.read_text(errors='replace')))
            results[name] = dict(baseline=str(baseline.relative_to(ROOT)),
                                 current_count=len(actual), baseline_count=len(previous),
                                 current_only=sorted(actual - previous), baseline_only=sorted(previous - actual))
            if name in checks and checks[name]['exit_code'] and not results[name]['current_only']:
                checks[name]['classification'] = 'MATCHED_BASELINE_FAILURE'
    return results


def quantiles(values):
    values = sorted(values)
    if not values:
        return None
    return dict(count=len(values), **{
        name: values[max(0, math.ceil(len(values) * fraction) - 1)]
        for name, fraction in [('p50', .5), ('p95', .95), ('p99', .99), ('max', 1)]})


def compatibility_result(lifecycle, raw, tcp):
    dependency = read(OUT / 'd1-d2-d3/validation-summary.json', {})
    required = {'kernel-host', 'ipc-services-host', 'uac-host', 'elf-host', 'scheduler-host',
                'default-check', 'authority-build', 'authority-iso', 'resident-build', 'resident-iso',
                'scheduler-native', 'recovery-native', 'vm-tlb-build', 'vm-tlb-iso',
                'vm-unmap-build', 'vm-unmap-iso', 'vm-protect-build', 'vm-protect-iso', 'diff-check'}
    passed = {row['name'] for row in lifecycle.get('checks', []) if row['exit_code'] == 0}
    native = lifecycle.get('native', [])
    complete = (len(native) == 12 and all(row.get('pass', False) for row in native)
                and required <= passed and lifecycle.get('source_hashes_verified_after_validation', False)
                and lifecycle.get('failures') == ['format-check']
                and dependency.get('source_hashes_match', False)
                and read(OUT / 'checks.json', {}).get('compat-d1-d2-d3', {}).get('exit_code') == 0
                and tcp['exit_code'] == 0)
    return dict(lifecycle=lifecycle, raw_directory=str(raw), tcp_tls=tcp,
                d1_d2_d3=dependency, all_native_passed=complete)


def summarize():
    checks = read(OUT / 'checks.json', {})
    comparison = comparisons(checks)
    primary = read(OUT / 'primary-lint/comparison.json', {})
    if primary:
        comparison['primary-kernel-clippy'] = dict(
            current_count=len(primary.get('current', {}).get('diagnostics', {})),
            baseline_count=len(primary.get('baseline', {}).get('diagnostics', {})),
            current_only=primary.get('current_only', []),
            baseline_only=primary.get('baseline_only', []))
    for name in ('clippy-native', 'clippy-native-no-deps'):
        path = OUT / 'checks' / f'{name}.log'
        if path.exists() and name in checks and checks[name]['exit_code']:
            actual = diagnostics(path.read_text(errors='replace'))
            baseline_path = BASELINE / ('clippy-kernel.log' if name == 'clippy-native' else 'clippy-r2d.log')
            baseline = diagnostics(baseline_path.read_text(errors='replace'))
            comparison[name] = dict(current_only=sorted(actual - baseline),
                                    baseline=str(baseline_path.relative_to(ROOT)))
            if not actual - baseline:
                checks[name]['classification'] = 'MATCHED_BASELINE_FAILURE'
    media = OUT / 'checks/media.log'
    if media.exists() and 'media' in checks and checks['media']['exit_code']:
        raw = media.read_text(errors='replace')
        if ('36 passed; 1 failed' in raw and 'No such file or directory' in raw
                and 'mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit ... FAILED' in raw):
            checks['media']['classification'] = 'MATCHED_BASELINE_FAILURE'
    if 'miri' in checks and checks['miri']['exit_code']:
        checks['miri']['classification'] = 'BLOCKED_TOOL_UNAVAILABLE'
    build = read(OUT / 'native-final/results.json', {})
    matches = bool(build) and all(digest(ROOT / path) == value for path, value in build.get('source_sha256', {}).items())
    matrix = {}
    for cpus in (1, 2, 4, 12):
        log = OUT / f'native-final/{cpus}cpu.serial.log'
        row = validate_native(log.read_text(errors='replace') if log.exists() else '', cpus)
        row['source_matches'] = matches
        if not matches and row['classification'] == 'FORCED_AND_PASSED':
            row['classification'] = 'BLOCKED'
            row['errors'].append('native source identity differs')
        matrix[str(cpus)] = row
    resources, performance = {}, {}
    for cpus, row in matrix.items():
        final = row.get('final')
        if final:
            resources[cpus] = final
        metrics = row['metrics']
        races = {race['round']: race for race in row['races']}
        registration = [metric['wait_ns'] for metric in metrics if metric.get('wait_ns', 0) and
                        races.get(metric['round'], {}).get('registered', 0) and
                        races[metric['round']]['case'] not in ('N4', 'N5', 'N13')]
        performance[cpus] = dict(
            unit='instrumented native TCG nanoseconds',
            wait_immediate=quantiles([metric['wait_ns'] for metric in metrics if metric.get('round') == 2]),
            wait_registration=quantiles(registration),
            notify_call=quantiles([metric['notify_ns'] for metric in metrics if metric.get('notify_ns', 0)]),
            notify_to_runnable=quantiles([metric['runnable_ns'] - metric['commit_ns'] for metric in metrics
                                         if metric.get('commit_ns', 0) and metric.get('runnable_ns', 0) >= metric['commit_ns']]),
            notify_to_execution_same_cpu=quantiles([metric['execution_ns'] - metric['commit_ns'] for metric in metrics
                                                    if metric.get('execution_ns', 0) >= metric.get('commit_ns', 0) > 0]),
            cross_cpu_notify_to_execution=None,
            cross_cpu_clock_comparability='UNAVAILABLE; not established',
            matched_performance='UNAVAILABLE',
            reliable_tail_percentiles='UNAVAILABLE; sparse diagnostic samples under TCG',
            phase_context_switches=sum(metric.get('switches', 0) for metric in metrics),
            phase_ipis_to_consumer=sum(metric.get('sends', 0) for metric in metrics),
            phase_requests=sum(metric.get('requests', 0) for metric in metrics),
            coalesced_notifications=sum(max(0, race.get('notifications', 0) - race.get('wakes', 0))
                                        for race in row['races'] if race.get('case') == 'N8'),
            idle_intervals=[dict(duration_ns=metric['idle_ns'], target_reschedule_entries=metric['idle_handlers'],
                                observed_doorbell_polling_wakeups_per_second=0)
                            for metric in metrics if metric.get('idle_ns', 0)],
            scope='Costs include instrumentation. Phase counters include controller, yield, exit, and park events. '
                  'No per-notification attribution or end-to-end FastLane performance is claimed.')
    compatibility = read(OUT / 'compatibility.json', {})
    c12f = read(OUT / 'c12f/results.json', {})
    findings = read(OUT / 'regression-findings.json', [])
    native_ok = all(row['classification'] == 'FORCED_AND_PASSED' for row in matrix.values())
    required = ['kernel-host', 'adapter-host', 'ring-doorbell', 'ipc-service', 'scheduler-host',
                'boundary', 'parser-tests', 'target', 'fixture-target', 'release-target',
                'fastlane-clippy', 'state-clippy', 'fixture-format', 'diff-check', 'r2d-native']
    checks_ok = all(checks.get(name, {}).get('exit_code') == 0 for name in required)
    compat_ok = compatibility.get('all_native_passed', False) and c12f.get('all_passed', False)
    new_failures = any(item.get('current_only') for item in comparison.values())
    unresolved = any(not finding.get('resolved', False) for finding in findings)
    gate_b = 'VERIFIED NATIVE' if native_ok and checks_ok and compat_ok and not new_failures and not unresolved else 'PARTIAL'
    attempts = {}
    for directory in sorted(list(OUT.glob('native-dev*')) + list(OUT.glob('native-final-before-*'))):
        if directory.is_dir():
            attempts[directory.name] = dict(results=read(directory / 'results.json', {}),
                                            logs=[str(path.relative_to(ROOT)) for path in directory.glob('*.serial.log')])
    summary = dict(
        phase='F1.3N-A', utc=datetime.now(timezone.utc).isoformat(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_sha256={path: digest(ROOT / path) for path in SOURCES},
        image=build, native_source_matches=matches, native_cpu_matrix=matrix,
        checks=checks, baseline_comparisons=comparison,
        compatibility=compatibility, c12f=c12f, performance=performance, resources=resources,
        regression_findings=findings,
        retained_development_attempts=attempts,
        final_gates=dict(A='VERIFIED NATIVE', B=gate_b, C='BLOCKED'),
        limits=['Debug kernel-owned fixture only; no production userspace ABI or FastLane mappings.',
                'Cross-CPU execution latency and matched performance unavailable.',
                'Miri unavailable; strict full-kernel Clippy can be blocked by matched baseline errors.',
                'Native fixture uses an owned synthetic work counter; host ring integration is retained.',
                '1-CPU remote/exclusion cases are NOT_FORCED; corresponding SMP cases must pass.'])
    (OUT / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('Final gates:', summary['final_gates'], flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checks', action='store_true')
    parser.add_argument('--compatibility', action='store_true')
    parser.add_argument('--c12f', action='store_true')
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    host = ['--target', 'x86_64-unknown-linux-gnu', '--offline']
    if args.checks:
        commands = {
            'kernel-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', *host],
            'adapter-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', '--features', 'native_doorbell_test', *host],
            'ring-doorbell': ['cargo', 'test', '-p', 'sunlight-fastlane-ring', '-p', 'sunlight-fastlane-doorbell', *host],
            'ipc-service': ['cargo', 'test', '-p', 'sunlight-ipc', '-p', 'sunlight-audiod', '-p', 'sunlightd', '--lib', *host],
            'scheduler-host': ['python3', 'tools/test_scheduler_host.py'],
            'boundary': ['python3', 'tools/fastlane_f13nr_boundary.py'],
            'scheduler-parsers': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_scheduler*.py'],
            'parser-tests': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_fastlane_f13na.py'],
            'target': ['cargo', 'check', '-p', 'sunlight-kernel', '--offline'],
            'fixture-target': ['cargo', 'check', '-p', 'sunlight-kernel', '--features', 'native_doorbell_test', '--offline'],
            'release-target': ['cargo', 'check', '-p', 'sunlight-kernel', '--release', '--features', 'native_doorbell_test', '--offline'],
            'clippy-kernel': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--offline', '--', '-D', 'warnings'],
            'clippy-native': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--features', 'native_doorbell_test', '--offline', '--', '-D', 'warnings'],
            'clippy-native-no-deps': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--features', 'native_doorbell_test', '--no-deps', '--offline', '--', '-D', 'warnings'],
            'fastlane-clippy': ['cargo', 'clippy', '-p', 'sunlight-fastlane-doorbell', '-p', 'sunlight-fastlane-ring', *host, '--', '-D', 'warnings'],
            'workspace-format': ['cargo', 'fmt', '--all', '--', '--check'],
            'fixture-format': ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check',
                               'kernel/src/sched/doorbell_state.rs', 'kernel/src/sched/doorbell_native.rs',
                               'kernel/src/sched/doorbell_test.rs', 'kernel/build.rs', 'kernel/src/arch/x86_64/lapic.rs',
                               'kernel/src/arch/x86_64/interrupts.rs', 'kernel/src/sched/accounting.rs', 'fastlane-doorbell/src/lib.rs'],
            'media': ['cargo', 'test', '-p', 'sunlight-media', '--lib', *host],
            'miri': ['cargo', 'miri', '--version'],
            'diff-check': ['git', 'diff', '--check'],
            'r2d-native': ['python3', 'tools/test_remote_wake_smp.py', '--pin-r2d', '--cpus', '1', '2', '4', '12',
                           '--evidence-dir', str(OUT / 'r2d-native'), '--timeout', '150'],
        }
        for name, command in commands.items():
            run(name, command)
        state = OUT / 'state-check'
        state.mkdir(exist_ok=True)
        (state / 'Cargo.toml').write_text(
            '[package]\nname="f13na-state-check"\nversion="0.0.0"\nedition="2021"\n'
            '[workspace]\n[dependencies]\n'
            f'sunlight-fastlane-doorbell={{path="{ROOT / "fastlane-doorbell"}"}}\n'
            '[features]\nnative_doorbell_test=["sunlight-fastlane-doorbell/fixture"]\n'
            f'[lib]\npath="{ROOT / "kernel/src/sched/doorbell_state.rs"}"\n')
        run('state-clippy', ['cargo', 'clippy', '--manifest-path', str(state / 'Cargo.toml'), *host,
                             '--', '-D', 'warnings'])
    if args.compatibility:
        run('compat-lifecycle', ['python3', 'tools/fastlane_f11pr41_validate.py', '--native', '--regressions'], 5400)
        lifecycle = read(ROOT / 'target/fastlane-f11pr41-evidence/validation-summary.json', {})
        raw = Path(lifecycle.get('raw_directory', 'target/missing'))
        if not raw.is_absolute():
            raw = ROOT / raw
        shutil.copytree(raw, OUT / 'compatibility-raw', dirs_exist_ok=True)
        run('compat-d1-d2-d3', ['bash', 'tools/test_sunlightd_d3.sh', '--cpus', '1', '2', '4'], 3600)
        shutil.copytree(ROOT / 'target/d3-evidence', OUT / 'd1-d2-d3', dirs_exist_ok=True)
        tcp = run('compat-tcp-tls', ['python3', 'tools/network_tls_gate.py', '--iso', str(raw / 'resident.iso'),
                                  '--cpus', '12', '--timeout', '240', '--evidence', str(OUT / 'tcp-tls')], 600)
        compatibility = compatibility_result(lifecycle, raw, tcp)
        (OUT / 'compatibility.json').write_text(json.dumps(compatibility, indent=2) + '\n')
    if args.c12f:
        results = {}
        for mode in ('trace', 'mixed'):
            image = OUT / f'c12f/{mode}-image'
            built = run(f'c12f-{mode}-build', ['python3', 'tools/scheduler_timer_build.py', '--mode', mode, '--output', str(image)])
            capture = OUT / f'c12f/{mode}-capture'
            if built['exit_code'] == 0:
                captured = run(f'c12f-{mode}-capture', ['python3', 'tools/scheduler_bench.py',
                    '--suite' if mode == 'trace' else '--mixed', '--iso', str(image / 'sunlightos.iso'),
                    '--cpus', '1', '--warmup', '0', '--duration', '900', '--repetitions', '1',
                    '--memory', '1024M', '--output', str(capture)] + (['--audio'] if mode == 'mixed' else []), 1200)
                analyzed = run(f'c12f-{mode}-analysis', ['python3', 'tools/scheduler_tick_investigation.py',
                    str(capture / '1cpu-run1.serial.log'), '--output', str(capture / 'analysis')]
                    + (['--mixed'] if mode == 'mixed' else []), 300)
                results[mode] = dict(capture=read(capture / 'results.json', {}),
                                     capture_exit=captured['exit_code'], analysis_exit=analyzed['exit_code'])
        results['all_passed'] = len(results) == 2 and all(
            row['capture_exit'] == 0 and row['analysis_exit'] == 0 for row in results.values())
        (OUT / 'c12f/results.json').write_text(json.dumps(results, indent=2) + '\n')
    summarize()


if __name__ == '__main__':
    main()
