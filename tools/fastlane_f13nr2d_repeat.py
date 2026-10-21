#!/usr/bin/env python3
"""Bounded same-image R2D fixture repetitions.

Each repetition replays the exact final kernel image through the R2D fixture and
retains its own serial log and validator result. Nothing is retried until a pass
appears: every attempt (pass or fail) is preserved. One build produces the image;
all repetitions then run with --skip-build against it.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
from fastlane_f13nr2d_validate import validate_native  # noqa: E402


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--cpus', nargs='+', type=int, default=[2, 4, 12])
    p.add_argument('--repetitions', type=int, default=5)
    p.add_argument('--timeout', type=int, default=150)
    p.add_argument('--image-dir', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    image = args.image_dir.resolve()
    iso = image / 'sunlightos.iso'
    elf_dir = image / 'sunlight-kernel.elf'
    build = json.loads((image / 'results.json').read_text())
    result = dict(image_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),
                  elf_sha256=hashlib.sha256(elf_dir.read_bytes()).hexdigest(),
                  build_source_sha256=build.get('source_sha256', {}),
                  cpus=args.cpus, repetitions=args.repetitions, attempts=[])
    for cpus in args.cpus:
        for repetition in range(1, args.repetitions + 1):
            tag = f'{cpus}cpu-rep{repetition}'
            tag_dir = out / tag
            tag_dir.mkdir(parents=True, exist_ok=True)
            # --skip-build needs the exact image present in the evidence dir.
            shutil.copy2(iso, tag_dir / 'sunlightos.iso')
            shutil.copy2(elf_dir, tag_dir / 'sunlight-kernel.elf')
            command = ['python3', 'tools/test_remote_wake_smp.py', '--pin-r2d',
                       '--cpus', str(cpus), '--timeout', str(args.timeout),
                       '--evidence-dir', str(tag_dir), '--skip-build']
            start = time.monotonic()
            with (out / f'{tag}.log').open('w') as log:
                code = subprocess.run(command, cwd=ROOT, stdout=log,
                                      stderr=subprocess.STDOUT).returncode
            produced = out / tag / f'{cpus}cpu.serial.log'
            raw = produced.read_text(errors='replace') if produced.exists() else ''
            (out / f'{tag}.serial.log').write_text(raw)
            validated = validate_native(raw, cpus)
            result['attempts'].append(dict(
                cpus=cpus, repetition=repetition, exit_code=code,
                status=validated['status'], seconds=time.monotonic() - start,
                errors=validated.get('errors', []), command=command,
                serial=f'{tag}.serial.log'))
            (out / 'repetitions.json').write_text(json.dumps(result, indent=2) + '\n')
            print(f'{tag}: {validated["status"]} (exit={code})', flush=True)
    failed = [a for a in result['attempts'] if a['status'] != 'pass']
    result['status'] = 'pass' if not failed else 'fail'
    result['pass_count'] = len(result['attempts']) - len(failed)
    (out / 'repetitions.json').write_text(json.dumps(result, indent=2) + '\n')
    print('R2D repetitions:', result['status'],
          f'{result["pass_count"]}/{len(result["attempts"])}', flush=True)
    if failed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
