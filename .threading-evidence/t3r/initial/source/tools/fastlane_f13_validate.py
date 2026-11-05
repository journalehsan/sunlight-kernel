#!/usr/bin/env python3
"""Bounded host-only F1.3 validation. Never builds or enables a native channel."""
import datetime
import hashlib
import json
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13-evidence'


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    results = {}
    host = ['--target', 'x86_64-unknown-linux-gnu']
    commands = {
        'host-tests': ['cargo', 'test', '-p', 'sunlight-fastlane-doorbell', *host, '--', '--nocapture'],
        'ring-tests': ['cargo', 'test', '-p', 'sunlight-fastlane-ring', *host],
        'clippy': ['cargo', 'clippy', '-p', 'sunlight-fastlane-doorbell', '--all-targets', *host, '--', '-D', 'warnings'],
        'target-build': ['cargo', 'build', '-p', 'sunlight-fastlane-doorbell', '--target', 'x86_64-unknown-none'],
        'kernel-check': ['cargo', 'check', '-p', 'sunlight-kernel'],
        'kernel-host': ['cargo', 'test', '-p', 'sunlight-kernel', '--lib', *host],
        'ipc-services-host': ['cargo', 'test', '-p', 'sunlight-ipc', '-p', 'sunlight-audiod', '-p', 'sunlightd', '--lib', *host],
        'scheduler-host': ['python3', 'tools/test_scheduler_host.py'],
        'changed-format': ['rustfmt', '--edition', '2021', '--check', 'fastlane-doorbell/src/lib.rs', 'fastlane-doorbell/src/tests.rs'],
        'diff-check': ['git', 'diff', '--check'],
        'workspace-format': ['cargo', 'fmt', '--all', '--', '--check'],
        'miri': ['cargo', 'miri', '--version'],
    }
    for name, command in commands.items():
        with (OUT / (name + '.log')).open('w') as log:
            try:
                r = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=120)
                code = r.returncode
            except subprocess.TimeoutExpired:
                code = 'timeout'
        results[name] = {'command': command, 'exit_code': code, 'log': name + '.log'}
        print(name, code, flush=True)
    with (OUT / 'environment.log').open('w') as log:
        log.write(platform.platform() + '\n')
        for cmd in [['rustc', '-Vv'], ['git', 'rev-parse', 'HEAD'], ['git', 'status', '--short'], ['lscpu']]:
            subprocess.run(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    sources = ['Cargo.toml', 'Cargo.lock', 'fastlane-doorbell/Cargo.toml', 'fastlane-doorbell/src/lib.rs', 'fastlane-doorbell/src/tests.rs', 'tools/fastlane_f13_validate.py', 'docs/ipc/fastlane-f13-doorbell.md']
    summary = {
        'phase': 'FastLane F1.3', 'status': 'partial: serialized host specification; native scheduler handoff unverified',
        'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'checks': results,
        'host_model': {'test_groups': 8, 'ring_messages': 20000, 'lifecycle_iterations': 10000, 'notify_close_races': 100, 'wake_samples': 1000, 'metadata_bytes_x86_64': 104},
        'native_kernel_fixture_validated': [], 'native_userspace_validated': [],
        'native_cpu_matrix': {str(n): 'not run: no kernel Doorbell adapter implemented' for n in [1, 2, 4, 12]},
        'blocked': ['verified atomic scheduler handoff and prompt remote wake delivery for Doorbell', 'production restricted SHM and R4.1/F1.1B-0 authority/lifecycle prerequisites'],
        'deferred': ['kernel adapter and native fixture', 'native resident/VM/TLB/SHM/D1/D2/D3/RecoveryBlocked/exit/exec/TCP/TLS reruns', 'native latency and scheduler-lock instrumentation', 'production FastLane transport'],
        'limitations': ['model identities are trusted fixture inputs, never bearer authority', 'no native scheduler correctness claim', 'host timing includes clock and host scheduler overhead', 'allocation-free model has no dynamic reservations; real kernel reservation failure not tested', '10k inline lifecycle transitions do not prove kernel allocator drift', 'baseline missing WAV remains unresolved; media not rerun', 'Miri unavailable if availability check fails'],
        'source_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sources},
    }
    (OUT / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    required = set(commands) - {'workspace-format', 'miri'}
    return 0 if all(results[n]['exit_code'] == 0 for n in required) else 1


if __name__ == '__main__':
    raise SystemExit(main())
