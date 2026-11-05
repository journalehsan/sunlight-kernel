#!/usr/bin/env python3
"""Build a C1.2E guest image and retain its exact input sources and hashes."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


@contextmanager
def frozen_workload(bench, frozen):
    # include_bytes! uses the canonical output path. Restore its original
    # contents so a later fresh Cargo build never silently reuses our fixture.
    previous = bench.read_bytes() if frozen else None
    try:
        if frozen:
            shutil.copyfile(frozen, bench)
        yield
    finally:
        if previous is not None:
            bench.write_bytes(previous)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--mode', choices=['trace', 'mixed', 'performance'], required=True)
    p.add_argument('--bench-elf', type=Path, help='use a frozen workload binary for matched policy comparisons')
    args = p.parse_args()
    root = Path(__file__).resolve().parents[1]
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    bench = root / 'target/x86_64-unknown-none/release/schedbench'
    features = dict(trace='scheduler_timer_trace', mixed='scheduler_mixed_bench', performance='scheduler_guest_bench')[args.mode]
    commands = []
    with frozen_workload(bench, args.bench_elf), (out / 'build.log').open('w') as log:
        if not args.bench_elf:
            env = dict(os.environ, RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone')
            cmd = ['cargo', 'build', '--package', 'sunlight-bench', '--bin', 'schedbench', '--release', '--target', 'x86_64-unknown-none', '--offline']
            commands.append(cmd)
            subprocess.run(cmd, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copyfile(bench, out / 'schedbench')
        cmd = ['cargo', 'build', '--package', 'sunlight-kernel', '--bin', 'sunlight-kernel', '--features', features, '--offline']
        commands.append(cmd)
        subprocess.run(cmd, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copyfile(root / 'target/x86_64-unknown-none/debug/sunlight-kernel', out / 'sunlight-kernel.elf')
        cmd = ['bash', 'tools/make_hybrid_iso.sh', str(out / 'sunlight-kernel.elf'), str(out / 'sunlightos.iso'), str(root / 'target/limine'), str(root)]
        commands.append(cmd)
        subprocess.run(cmd, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    sources = ['kernel/src/sched/mod.rs', 'kernel/src/sched/quantum.rs', 'kernel/src/sched/requests.rs',
               'kernel/src/scheduler_bench_diag.rs', 'kernel/src/scheduler_timer_trace.rs',
               'kernel/src/arch/x86_64/interrupts.rs', 'kernel/src/ipc/mod.rs', 'kernel/src/main.rs',
               'kernel/src/process/spawn.rs', 'kernel/src/process/mod.rs', 'kernel/src/arch/x86_64/syscall.rs',
               'kernel/src/memory/pmm.rs',
               'kernel/Cargo.toml', 'sunlight-bench/src/scheduler_guest.rs', 'sunlight-bench/src/thread.rs',
               'services/timer_server/src/main.rs', 'services/sunlight-niced/src/config.rs',
               'tools/scheduler_timer.py', 'tools/scheduler_bench.py', 'tools/scheduler_timer_build.py']
    for name in sources:
        dst = out / 'source' / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / name, dst)
    (out / 'worktree.patch').write_bytes(subprocess.check_output(['git', 'diff'], cwd=root))
    paths = [out / 'schedbench', out / 'sunlight-kernel.elf', out / 'sunlightos.iso'] + [out / 'source' / s for s in sources]
    manifest = dict(features=features, mode=args.mode, commands=commands,
        workload_source='frozen ELF supplied by --bench-elf' if args.bench_elf else 'source/sunlight-bench/src/scheduler_guest.rs',
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        sha256={str(path.relative_to(out)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths})
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(out, flush=True)


if __name__ == '__main__':
    main()
