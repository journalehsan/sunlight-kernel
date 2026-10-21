#!/usr/bin/env python3
"""Retain F1.3N-R host evidence and optionally native scheduler wake evidence.

Historical vulnerable behavior is extracted from the audit revision. Functional
QEMU success does not establish physical-hardware timing or Busy reproduction.
"""
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
OUT = ROOT / 'target/fastlane-f13nr-evidence'
BASELINE = 'f7a285955cef31578983e0163ecfca558affc261'


def run(name, command, timeout=240):
    start = time.monotonic()
    with (OUT / f'{name}.log').open('w') as log:
        try:
            code = subprocess.run(command, cwd=ROOT, stdout=log,
                                  stderr=subprocess.STDOUT, timeout=timeout).returncode
        except subprocess.TimeoutExpired:
            code = 124
    raw = (OUT / f'{name}.log').read_text(errors='replace')
    return dict(command=command, exit_code=code, seconds=time.monotonic() - start,
                log=f'{name}.log', passed_tests=sum(map(int, re.findall(
                    r'test result: ok\. (\d+) passed', raw))))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', action='store_true')
    parser.add_argument('--collect', action='store_true', help='collect existing logs without rerunning checks')
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    checks = json.loads((OUT / 'host-results.json').read_text()) if (OUT / 'host-results.json').exists() else {}
    if not args.collect:
        for filename in ['sched/mod.rs', 'sched/requests.rs']:
            destination = OUT / ('baseline-sched.rs' if filename.endswith('mod.rs') else 'baseline-requests.rs')
            destination.write_bytes(subprocess.check_output(['git', 'show', f'{BASELINE}:kernel/src/{filename}'], cwd=ROOT))
        # Keep the original extracted production counterexamples executable.
        script = (ROOT / 'tools/fastlane_f13n_boundary.py').read_text()
        script = script.replace("OUT = ROOT / 'target/fastlane-f13n-evidence'", "OUT = ROOT / 'target/fastlane-f13nr-evidence/baseline'")
        script = script.replace("(ROOT / 'kernel/src/sched/mod.rs').read_text()", "(OUT.parent / 'baseline-sched.rs').read_text()")
        script = script.replace("ROOT / 'kernel/src/sched/requests.rs'", "OUT.parent / 'baseline-requests.rs'")
        script = script.replace('ROOT = Path(__file__).resolve().parents[1]', f'ROOT = Path({str(ROOT)!r})')
        (OUT / 'baseline-boundary.py').write_text(script)
        host = ['--target', 'x86_64-unknown-linux-gnu']
        commands = {
            'baseline-boundary': ['python3', str(OUT / 'baseline-boundary.py')],
            'delivery-boundary': ['python3', 'tools/fastlane_f13nr_boundary.py'],
            'kernel-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', *host],
            'ipc-services-host': ['cargo', 'test', '-p', 'sunlight-ipc', '-p', 'sunlight-audiod', '-p', 'sunlightd', '--lib', *host],
            'ring-host': ['cargo', 'test', '-p', 'sunlight-fastlane-ring', *host],
            'doorbell-host': ['cargo', 'test', '-p', 'sunlight-fastlane-doorbell', *host],
            'scheduler-host': ['python3', 'tools/test_scheduler_host.py'],
            'scheduler-validators': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_scheduler*.py'],
            'target-check': ['cargo', 'check', '-p', 'sunlight-kernel', '--offline'],
            'fixture-check': ['cargo', 'check', '-p', 'sunlight-kernel', '--features', 'remote_wake_test', '--offline'],
            'diff-check': ['git', 'diff', '--check'],
        }
        for name, command in commands.items():
            checks[name] = run(name, command)
            print(name, checks[name]['exit_code'], flush=True)
        (OUT / 'host-results.json').write_text(json.dumps(checks, indent=2) + '\n')
    if args.native:
        checks['native-final'] = run('native-final-run', ['python3', 'tools/test_remote_wake_smp.py',
            '--cpus', '1', '2', '4', '12', '--timeout', '120', '--evidence-dir', str(OUT / 'native-frozen')], timeout=900)
    matrix = {}
    for cpu in [1, 2, 4, 12]:
        serial = OUT / 'native-frozen' / f'{cpu}cpu.serial.log'
        raw = serial.read_text(errors='replace') if serial.exists() else ''
        lines = [line for line in raw.splitlines() if line.startswith('[F13NR] PASS ')]
        row = dict(status='pass' if lines else 'unverified', log=str(serial.relative_to(OUT)),
                   physical_remote=cpu > 1 and bool(lines), apic_mode='xAPIC MMIO',
                   source_cpu_ids=sorted(set(map(int, re.findall(r'\[F13NR\] publish source=(\d+)', raw)))),
                   execution_cpu_ids=sorted(set(map(int, re.findall(r'\[F13NR\] executed .*?target=(\d+)', raw)))))
        if lines:
            row['counters'] = {k: int(v, 0) if re.fullmatch(r'(0x[0-9a-f]+|[0-9]+)', v) else v
                               for k, v in re.findall(r'(\w+)=(\S+)', lines[-1])}
            row['zero_frame_drift'] = row['counters']['frames_before'] == row['counters']['frames_after']
        msrs = {}
        for line in raw.splitlines():
            base = re.search(r'apic_base=(0x[0-9a-f]+)', line)
            identity = re.search(r'(?:source|cpu)=(\d+)', line)
            if base and identity:
                msrs[int(identity[1])] = int(base[1], 16)
        row['apic_base_msr_by_cpu'] = msrs
        row['xapic_mode_verified'] = (set(msrs) == set(range(cpu)) and
                                     all(v & (1 << 11) and not v & (1 << 10) for v in msrs.values()))
        matrix[str(cpu)] = row
    sources = subprocess.check_output(['git', 'diff', '--name-only'], cwd=ROOT, text=True).splitlines()
    sources += ['kernel/src/sched/remote_wake_test.rs', 'tools/fastlane_f13nr_boundary.py',
                'tools/fastlane_f13nr_validate.py', 'tools/test_remote_wake_smp.py',
                'kernel/src/sched/handoff.rs', 'kernel/src/sched/quantum.rs',
                'rust-toolchain.toml', '.cargo/config.toml',
                'docs/kernel/scheduler-remote-wake-f13nr.md']
    source_hashes = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sources if (ROOT / p).is_file()}
    for path in source_hashes:
        destination = OUT / 'source' / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / path, destination)
    (OUT / 'source-sha256.json').write_text(json.dumps(source_hashes, indent=2) + '\n')
    (OUT / 'worktree.patch').write_bytes(subprocess.check_output(['git', 'diff'], cwd=ROOT))
    summary = dict(phase='FastLane F1.3N-R', complete=False, status='partial: delivery repair native validated; pin-release eligibility rearm and performance acceptance unresolved',
        utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), baseline_revision=BASELINE,
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        checks=checks, native_cpu_matrix=matrix, source_sha256=source_hashes,
        audited=['request-only callers', 'xAPIC ICR', 'IRQ/syscall/exit tails', 'idle entry', 'CPU lifecycle limits'],
        implemented=['coalesced remote delivery ownership', 'bounded immediate Busy continuation', 'missing flush tails', 'exit handoff kick', 'debug-only native fixture'],
        host_validated=['extracted production dispatch with controlled Busy and acknowledgement hooks', 'atomic request state', 'legacy vulnerable-model counterexamples'],
        native_scheduler_validated=all(r['status'] == 'pass' for r in matrix.values()),
        native_remote_ipi_validated=all(matrix[str(c)]['physical_remote'] for c in [2, 4, 12]),
        native_validation_scope='Final QEMU xAPIC idle/entry, many-to-one, injected Busy and four cycles; post-pin-release eligibility remains unverified. Only formatting and safety documentation changed in kernel files after the final native build.',
        measured=['functional counters', 'task execution identities', 'per-CPU APIC_BASE mode bits', 'free frames before allocation and after reap', 'same-CPU C1.2F normal and mixed capture percentiles; unmatched baseline'],
        critical_liveness_guarantee='Unproven when Ready work is rejected while its stack is pinned remotely, then the pin is released without a new scheduling event',
        unverified=['full compatibility and C1.2F native replay after final local-delivery coalescing delta', 'native pin-release eligibility race', 'physical hardware Busy reproduction', 'matched remote wake latency percentiles and rates', 'cross-CPU clock comparability', 'exhaustive native idle-entry interleavings', 'production-hardware retry budget suitability'],
        blocked=['post-pin-release eligibility has no proven independent rearm; see known_limit_pin_release_does_not_rearm_consumed_request', 'full acceptance pending matched performance and exhaustive native race evidence'],
        compatibility_index='compatibility-results.json', performance_index='performance-results.json',
        adapter_ready=False, production_doorbell=False,
        failure_policy='At most 4096 attempts per target outside locks with IF clear; retain all remaining bits then panic on exhaustion or offline target. A count bound is not a wall-clock bound.',
        counter_scope='Native requests count fixture task wakes; attempts/accepted count bounded-dispatch sends to target (including exit handoffs); handled/opportunities are target-only. Source-control IPIs excluded. No rates or latency inferred.')
    (OUT / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')


if __name__ == '__main__':
    main()
