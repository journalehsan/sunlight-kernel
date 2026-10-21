#!/usr/bin/env python3
"""Record F1.3N's audited boundary without claiming native implementation."""
import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13n-evidence'


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    host = ['--target', 'x86_64-unknown-linux-gnu']
    commands = {
        'delivery-boundary': ['python3', 'tools/fastlane_f13n_boundary.py'],
        'doorbell-host': ['cargo', 'test', '-p', 'sunlight-fastlane-doorbell', *host, '--', '--nocapture'],
        'ring-host': ['cargo', 'test', '-p', 'sunlight-fastlane-ring', *host],
        'kernel-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', *host],
        'ipc-services-host': ['cargo', 'test', '-p', 'sunlight-ipc', '-p', 'sunlight-audiod', '-p', 'sunlightd', '--lib', *host],
        'scheduler-host': ['python3', 'tools/test_scheduler_host.py'],
        'scheduler-validators': ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_scheduler*.py'],
        'target-check': ['cargo', 'check', '-p', 'sunlight-kernel'],
        'foundation-target': ['cargo', 'build', '-p', 'sunlight-fastlane-doorbell', '-p', 'sunlight-fastlane-ring'],
        'foundation-clippy': ['cargo', 'clippy', '-p', 'sunlight-fastlane-doorbell', '-p', 'sunlight-fastlane-ring', '--all-targets', *host, '--', '-D', 'warnings'],
        'kernel-clippy': ['cargo', 'clippy', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel', '--', '-D', 'warnings'],
        'foundation-format': ['rustfmt', '--edition', '2021', '--check', 'fastlane-doorbell/src/lib.rs', 'fastlane-doorbell/src/tests.rs', 'fastlane-ring/src/lib.rs'],
        'diff-check': ['git', 'diff', '--check'],
        'workspace-format': ['cargo', 'fmt', '--all', '--', '--check'],
        'media-host': ['cargo', 'test', '-p', 'sunlight-media', '--lib', *host],
        'miri-availability': ['cargo', 'miri', '--version'],
    }
    results = {}
    for name, command in commands.items():
        with (OUT / f'{name}.log').open('w') as log:
            try:
                code = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                      timeout=180).returncode
            except subprocess.TimeoutExpired:
                code = 'timeout'
        results[name] = dict(command=command, exit_code=code, log=f'{name}.log',
                             status='pass' if code == 0 else 'unavailable' if name == 'miri-availability' else 'fail')
        results[name]['passed_rust_tests'] = sum(map(int, re.findall(
            r'test result: ok\. (\d+) passed', (OUT / f'{name}.log').read_text(errors='replace'))))
        if name == 'scheduler-host':
            shutil.copyfile(ROOT / 'target/scheduler-b-evidence/focused-host.log',
                            OUT / 'scheduler-focused-host.log')
        print(name, code, flush=True)
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=ROOT).decode()
    (OUT / 'worktree.patch').write_text(git('diff', '--binary'))
    (OUT / 'worktree-status.log').write_text(git('status', '--short'))
    with (OUT / 'toolchain.log').open('w') as log:
        subprocess.run(['rustc', '-Vv'], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    sources = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', 'kernel/Cargo.toml',
               'kernel/src/sched/mod.rs', 'kernel/src/sched/requests.rs', 'kernel/src/sched/handoff.rs',
               'kernel/src/arch/x86_64/interrupts.rs', 'kernel/src/arch/x86_64/lapic.rs',
               'kernel/src/arch/x86_64/syscall.rs', 'kernel/src/ipc/mod.rs',
               'kernel/src/process/mod.rs', 'kernel/src/process/foreground.rs',
               'fastlane-doorbell/src/lib.rs', 'fastlane-doorbell/src/tests.rs',
               'fastlane-ring/src/lib.rs', 'tools/fastlane_f13n_boundary.py',
               'tools/fastlane_f13n_validate.py', 'docs/ipc/fastlane-f13n-native-doorbell.md']
    transitions = [
        'notify-before-registration', 'notify-before-wait', 'notify-during-registration',
        'notify-during-block-commit', 'notify-after-block', 'remote-notify',
        'multiple-notify-runnable', 'multiple-notify-blocked', 'close-before-wait',
        'close-blocked', 'cancel-before-notify', 'cancel-concurrent-notify',
        'consumer-exit-waiting', 'producer-exit-waiting', 'stale-generation',
        'sequence-exhaustion', 'repeated-lifecycle', 'resource-exhaustion',
    ]
    summary = {
        'phase': 'FastLane F1.3N', 'status': 'partial: N1 audit and host delivery counterexamples only',
        'complete': False, 'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'source_revision': git('rev-parse', 'HEAD').strip(), 'worktree_dirty': True,
        'source_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sources},
        'build_identity': {'native_fixture_elf': None, 'native_fixture_iso': None,
                           'reason': 'No native adapter or fixture built; target-check is compilation only'},
        'checks': results,
        'baseline_failures': {
            'kernel-clippy': 'Existing strict lints in dependencies and build scripts; see full log',
            'workspace-format': 'Existing formatting differences outside changed files',
            'media-host': '36 pass; missing docs/songs/onaldin_music-catch-the-sunlight-333176.wav',
            'miri-availability': 'cargo-miri not installed for current nightly',
        },
        'host_boundary_assertions': ['request-only:no-interrupt', 'busy-icr:retained-without-autonomous-retry',
                                    'external-retry:send-accepted-execution-unobserved', 'coalescing:per-cpu-retention'],
        'host_boundary_classification': 'Extracted production functions with simulated LAPIC, not kernel execution',
        'native_kernel_state': [], 'native_scheduler_block_wake': [], 'native_remote_execution': [],
        'native_userspace': [],
        'native_cpu_matrix': {str(n): {'status': 'skip', 'actual_cpu_ids': [],
                                     'reason': 'N1 delivery invariant unresolved; no native Doorbell fixture'} for n in [1, 2, 4, 12]},
        'native_transitions': {t: {'status': 'skip', 'assertion_reached': False,
                                  'expected_scheduler_state': None, 'actual_scheduler_state': None,
                                  'sequence_before': None, 'sequence_after': None,
                                  'wake_outcome': 'unmeasured', 'cleanup_outcome': 'unmeasured'} for t in transitions},
        'native_measurements': {'latency_percentiles': None, 'clock_comparability': 'not measured',
                                'idle_wakeups': None, 'frame_drift': None, 'metadata_allocations': None},
        'blockers': ['Request-only wake is not interrupt delivery',
                     'Busy-ICR deferred mask has no autonomous event-driven retry guarantee',
                     'Exact-incarnation adapter and all exit/cancellation paths remain unimplemented'],
        'deferred_regressions': ['resident boot', 'C1.2F native timing', 'VM/TLB/SHM retirement',
                                 'exec lifecycle', 'D1/D2/D3', 'RecoveryBlocked', 'TCP/TLS'],
        'security_boundary': 'No scheduler, syscall, production IPC, SHM or authority changes',
    }
    (OUT / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    optional = {'workspace-format', 'media-host', 'miri-availability', 'kernel-clippy'}
    return int(any(r['exit_code'] != 0 for n, r in results.items() if n not in optional))


if __name__ == '__main__':
    raise SystemExit(main())
