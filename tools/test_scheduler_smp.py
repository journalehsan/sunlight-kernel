#!/usr/bin/env python3
"""Build and run the isolated Phase B lifecycle gate, with retained evidence.

Requires Rust kernel toolchain, GNU as/ld, Limine/xorriso and QEMU. TCG results
are functional evidence, not performance measurements. --skip-build requires
the exact feature-set ELF/ISO produced by a previous invocation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cpus', nargs='+', type=int, default=[1, 2, 4, 12])
    parser.add_argument('--timeout', type=int, default=360)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--perf-diag', action='store_true',
                        help='also compile scheduler_perf_diag for functional coverage')
    parser.add_argument('--evidence-dir', type=Path,
                        help='retain this run in a separate directory')
    args = parser.parse_args()
    if any(c < 1 or c > 12 for c in args.cpus) or args.timeout < 1:
        parser.error('CPUs must be 1..12 and timeout must be positive')
    root = Path(__file__).resolve().parents[1]
    evidence = args.evidence_dir or Path('target/scheduler-b-evidence')
    if not evidence.is_absolute():
        evidence = root / evidence
    evidence.mkdir(parents=True, exist_ok=True)
    elf = evidence / 'sunlight-kernel.elf'
    iso = evidence / 'sunlightos.iso'
    if not args.skip_build:
        with (evidence / 'build.log').open('w') as log:
            features = ['scheduler_b_test']
            if args.perf_diag:
                features.append('scheduler_perf_diag')
            subprocess.run([
                'cargo', 'build', '--package', 'sunlight-kernel', '--bin',
                'sunlight-kernel', '--features', ','.join(features), '--offline',
            ], cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(root / 'target/x86_64-unknown-none/debug/sunlight-kernel', elf)
            subprocess.run([
                'bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso),
                str(root / 'target/limine'), str(root),
            ], cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    metadata = {
        'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'elf_sha256': hashlib.sha256(elf.read_bytes()).hexdigest(),
        'iso_sha256': hashlib.sha256(iso.read_bytes()).hexdigest(),
        'qemu': subprocess.check_output(['qemu-system-x86_64', '--version'], text=True).splitlines()[0],
        'accelerator': 'tcg', 'cpus': args.cpus,
        'features': ['scheduler_b_test'] + (['scheduler_perf_diag'] if args.perf_diag else []),
        'results': [],
    }
    try:
        failures = []
        for cpus in args.cpus:
            serial = evidence / f'{cpus}cpu.serial.log'
            # Popen can return before QEMU opens/truncates its serial backend.
            # Clear the previous run synchronously so an old PASS cannot win.
            serial.write_text('')
            with (evidence / f'{cpus}cpu.qemu.log').open('w') as log:
                command = [
                    'qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                    '-smp', str(cpus), '-m', '1024', '-cdrom', str(iso),
                    '-serial', f'file:{serial}', '-display', 'none', '-nic', 'none',
                    '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04',
                    '-no-reboot', '-no-shutdown',
                ]
                proc = subprocess.Popen(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
                timed_out = False
                marker = f'[SCHED-B] PASS cpus={cpus} '
                deadline = time.monotonic() + args.timeout
                completed = False
                try:
                    while time.monotonic() < deadline:
                        output = serial.read_text(errors='replace') if serial.exists() else ''
                        if marker in output and '\n' in output.split(marker, 1)[1]:
                            completed = True
                            break
                        if proc.poll() is not None:
                            break
                        time.sleep(0.25)
                    timed_out = not completed and proc.poll() is None
                finally:
                    if proc.poll() is None:
                        proc.terminate()
                        try:
                            proc.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            proc.kill()
                    code = proc.wait()
            output = serial.read_text(errors='replace') if serial.exists() else ''
            passed = (marker in output and '\n' in output.split(marker, 1)[1]
                      and not timed_out)
            metadata['results'].append({
                'cpus': cpus, 'pass': passed, 'exit_code': code,
                'timeout': timed_out, 'command': command,
                'completion': 'serial-marker' if passed else 'incomplete',
            })
            print(f'{cpus} CPUs: {"PASS" if passed else "FAIL"} ({serial})', flush=True)
            if not passed:
                failures.append(cpus)
        if failures:
            raise RuntimeError(f'Guest failures on CPUs {failures}; inspect retained serial/QEMU logs')
    finally:
        (evidence / 'results.json').write_text(json.dumps(metadata, indent=2) + '\n')


if __name__ == '__main__':
    main()
