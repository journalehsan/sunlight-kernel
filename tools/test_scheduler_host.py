#!/usr/bin/env python3
"""Run production scheduler state tests and optional isolated host libraries.

The workspace-wide feature union is not host compatible. --workspace-libraries
tests each library separately with a timeout; --resume skips previous passes.
All evidence stays under target/scheduler-b-evidence. No guest image is changed.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace-libraries', action='store_true')
    parser.add_argument('--resume', action='store_true')
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    evidence = root / 'target/scheduler-b-evidence'
    evidence.mkdir(exist_ok=True, parents=True)
    modules = [
        'kernel/src/sched/handoff.rs', 'kernel/src/sched/requests.rs',
        'kernel/src/sched/quantum.rs', 'tools/foreground-scope-proof.rs',
        'ipc/src/cpu_accounting.rs',
    ]
    with tempfile.TemporaryDirectory(prefix='scheduler-host-') as temporary:
        work = Path(temporary)
        wrapper = work / 'timekeeping.rs'
        wrapper.write_text(
            '#[macro_export]\nmacro_rules! serial_println { ($($a:tt)*) => '
            '{ let _ = format_args!($($a)*); }; }\n'
            f'#[path = "{root / "kernel/src/timekeeping.rs"}"] mod timekeeping;\n')
        with (evidence / 'focused-host.log').open('w') as log:
            for i, source in enumerate([root / p for p in modules] + [wrapper]):
                binary = work / f'test-{i}'
                subprocess.run(['rustc', '--edition=2021', '--test', str(source), '-o', str(binary)],
                               cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
                subprocess.run([str(binary), '--test-threads=1'], cwd=root, stdout=log,
                               stderr=subprocess.STDOUT, check=True, timeout=args.timeout)
            subprocess.run([sys.executable, 'tools/scheduler_policy_probe.py'], cwd=root,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=args.timeout)
            subprocess.run([sys.executable, 'tools/scheduler_queue_probe.py'], cwd=root,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=args.timeout)
    print('Focused production state/lifecycle/accounting/policy tests: PASS', flush=True)
    if not args.workspace_libraries:
        return
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1', '--offline'], cwd=root))
    folder = evidence / 'host-libraries'
    folder.mkdir(exist_ok=True)
    result_path = folder / 'results.json'
    results = json.loads(result_path.read_text()) if args.resume and result_path.exists() else []
    done = {r['package'] for r in results if r['exit_code'] == 0}
    for p in metadata['packages']:
        name = p['name']
        if p['id'] not in metadata['workspace_members'] or not any(t['kind'] == ['lib'] for t in p['targets']):
            continue
        if name in done:
            continue
        with (folder / f'{name}.log').open('w') as log:
            command = ['cargo', 'test', '--package', name, '--lib', '--target',
                       'x86_64-unknown-linux-gnu', '--offline', '--', '--test-threads=1']
            try:
                proc = subprocess.Popen(command, cwd=root, stdout=log, stderr=subprocess.STDOUT,
                                        start_new_session=True)
                code = proc.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
                code = 124
        text = (folder / f'{name}.log').read_text(errors='replace')
        counts = re.findall(r'test result: ok\. (\d+) passed', text)
        results = [r for r in results if r['package'] != name]
        results.append({'package': name, 'exit_code': code,
                        'passed_tests': sum(map(int, counts)), 'command': command})
        result_path.write_text(json.dumps(results, indent=2) + '\n')
        print(name, 'PASS' if code == 0 else f'FAIL ({code})', flush=True)
    failures = [r['package'] for r in results if r['exit_code']]
    print(f'Host library packages: {len(results)}, failures/timeouts: {failures}', flush=True)
    if failures:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
