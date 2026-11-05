#!/usr/bin/env python3
"""Run the F1.3N-A kernel-only Doorbell fixture and retain every attempt."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

from fastlane_f13nr2v_validate import fields

ROOT = Path(__file__).resolve().parents[1]
ROUNDS = 42
SOURCES = [
    'kernel/Cargo.toml', 'kernel/build.rs', 'Cargo.lock',
    'kernel/src/main.rs', 'kernel/src/process/mod.rs',
    'kernel/src/sched/mod.rs', 'kernel/src/sched/accounting.rs',
    'kernel/src/sched/doorbell_state.rs', 'kernel/src/sched/doorbell_native.rs',
    'kernel/src/sched/doorbell_test.rs', 'kernel/src/sched/requests.rs',
    'kernel/src/sched/handoff.rs', 'kernel/src/process/foreground.rs',
    'kernel/src/arch/x86_64/syscall.rs', 'kernel/src/arch/x86_64/interrupts.rs',
    'kernel/src/arch/x86_64/lapic.rs', 'fastlane-doorbell/Cargo.toml',
    'fastlane-doorbell/src/lib.rs', 'tools/helios-probes/doorbell-probe.S',
]


def case_number(round_id):
    if round_id <= 11:
        return round_id + 1
    if round_id == 12:
        return 12
    if round_id <= 16:
        return round_id
    return 10 if round_id == 17 else 17


def validate_native(raw, cpus):
    def rows(tag):
        return [fields(line) for line in raw.splitlines()
                if line.startswith(f'[F13NA] {tag} ')]
    races, metrics, cleanups = rows('race'), rows('metrics'), rows('cleanup')
    finals, identities, workloads = rows('PASS'), rows('cpu'), rows('workload')
    errors = []
    result = dict(classification='NOT_RUN', errors=errors, races=races,
                  metrics=metrics, cleanup=cleanups, actual_cpus=identities,
                  workloads=workloads, final=finals[-1] if finals else None)
    if any(marker in raw for marker in ('PANIC', 'panicked at', '[FAULT]', 'UNEXPECTED')):
        errors.append('guest panic/fault')
    if len(finals) != 1:
        errors.append('one fixture completion required')
        result['classification'] = 'FORCED_AND_FAILED' if errors[0] == 'guest panic/fault' else 'NOT_RUN'
        return result
    final = finals[0]
    expected = dict(cpus=cpus, rounds=ROUNDS, created=ROUNDS + 1,
                    retired=ROUNDS + 1, active=0, waiters=0, task_refs=0,
                    obligations=0, deferred=0, pending=0, queued=0, pins=0,
                    metadata_heap_allocations=0, polling_wakes=0)
    for key, value in expected.items():
        if final.get(key) != value:
            errors.append(f'final {key}: expected {value}')
    if not final.get('frames_before') or final.get('frames_after') != final.get('frames_before'):
        errors.append('frame accounting missing or drifted')
    if len(identities) != cpus or {row.get('cpu') for row in identities} != set(range(cpus)):
        errors.append('missing actual CPU identity')
    if any(row.get('cpu') != row.get('apic') or row.get('timer') != 'masked' for row in identities):
        errors.append('APIC/timer evidence missing')
    if len(workloads) != cpus or {row.get('cpu') for row in workloads} != set(range(cpus)):
        errors.append('missing unrelated workload CPU')
    if any(row.get('user_executions', 0) <= 0 for row in workloads):
        errors.append('unrelated workload did not execute')
    for label, records in [('race', races), ('metrics', metrics), ('cleanup', cleanups)]:
        if len(records) != ROUNDS or {row.get('round') for row in records} != set(range(ROUNDS)):
            errors.append(f'missing or duplicate {label} record')
    generation = 0
    incarnation = 0
    for row in races:
        round_id = row.get('round', -1)
        if round_id not in range(ROUNDS):
            errors.append('invalid race round')
            continue
        case = case_number(round_id)
        if row.get('case') != f'N{case}':
            errors.append(f'round {round_id}: wrong case')
        allowed_not_forced = cpus == 1 and case in (4, 7, 13)
        expected_result = 'NOT_FORCED' if allowed_not_forced else 'FORCED_AND_PASSED'
        if row.get('result') != expected_result:
            errors.append(f'round {round_id}: critical order not forced')
        if row.get('generation', 0) <= generation or row.get('incarnation', 0) <= incarnation:
            errors.append(f'round {round_id}: generation/incarnation reused')
        generation, incarnation = row.get('generation', 0), row.get('incarnation', 0)
        if round_id and row.get('stale_rejected') != 1:
            errors.append(f'round {round_id}: stale identity rejection absent')
        immediate = case in (1, 2, 3, 11)
        killed = case in (13, 14)
        registrations = 0 if immediate else (2 if case == 9 else 1)
        blocks = 0 if immediate or case in (4, 5, 13) else (2 if case == 9 else 1)
        resumes = 0 if killed else (2 if case == 9 else 1)
        wakes = 0 if immediate or killed else (2 if case == 9 else 1)
        notifications = (8 * max(1, cpus - 1) if case == 8 else
                         0 if killed or case == 10 else 1)
        source_mask = (1 if cpus == 1 or immediate or case == 6 else
                       ((1 << cpus) - 1) & ~1 if case == 8 else
                       1 << (1 + round_id % (cpus - 1)))
        required = dict(registered=registrations, blocked=blocks,
                        wakes=wakes, resumes=resumes, notifications=notifications,
                        sources=source_mask, consumer=0, cleanup='retired', ready=wakes,
                        spurious=1 if case == 9 else 0)
        required['outcome'] = ('work' if case in (1, 2) else 'changed' if case == 3 else
                               'consumer_exit' if killed else 'exhausted' if case == 15 else
                               'producer_exit' if round_id == 17 else 'closed' if case in (10, 11) else
                               'cancelled' if round_id == 11 else 'notified')
        if case in (4, 5, 13):
            required['prepark'] = 1
            if cpus > 1:
                required['excluded'] = 1
        for key, value in required.items():
            if row.get(key) != value:
                errors.append(f'round {round_id}: {key} expected {value}, got {row.get(key)}')
        if row.get('selections', 0) < 1:
            errors.append(f'round {round_id}: actual scheduler selection absent')
        if case == 15:
            if row.get('snapshot') != (1 << 64) - 1 or row.get('sequence') != (1 << 64) - 1:
                errors.append('exhaustion boundary not reached')
        elif row.get('sequence') != notifications:
            errors.append(f'round {round_id}: incorrect notification sequence')
    for row in cleanups:
        if any(row.get(key) != value for key, value in dict(
                waiter=0, task_refs=0, queued=0, state='Reaped', retired=1).items()):
            errors.append('unsafe teardown')
    idle = [row for row in metrics if row.get('round') == 6]
    if len(idle) != 1 or idle[0].get('idle_ns', 0) < 100_000_000 or idle[0].get('idle_handlers') != 0:
        errors.append('blocked idle interval missing or awakened')
    if any(row.get('ticks') != 0 for row in metrics if row.get('round') != 0):
        errors.append('timer assistance during blocking rounds')
    result['bootstrap_tick_mirror_delta'] = next((row.get('ticks') for row in metrics if row.get('round') == 0), None)
    if not sum(row.get('sends', 0) for row in metrics) or not sum(row.get('handlers', 0) for row in metrics):
        errors.append('IPI acceptance/handler evidence absent')
    result['classification'] = 'FORCED_AND_FAILED' if errors else 'FORCED_AND_PASSED'
    return result


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cpus', nargs='+', type=int, default=[1, 2, 4, 12])
    parser.add_argument('--timeout', type=int, default=150)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--evidence-dir', type=Path, default=Path('target/fastlane-f13na-evidence/native-final'))
    args = parser.parse_args()
    if any(cpu not in (1, 2, 4, 12) for cpu in args.cpus) or args.timeout <= 0:
        parser.error('use CPUs 1/2/4/12 and a positive timeout')
    evidence = args.evidence_dir.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    if (evidence / 'results.json').exists():
        parser.error('use a fresh evidence directory to preserve prior attempts')
    elf, iso = evidence / 'sunlight-kernel.elf', evidence / 'sunlightos.iso'
    build_command = ['cargo', 'build', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel',
                     '--features', 'native_doorbell_test', '--offline']
    iso_command = ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso),
                   str(ROOT / 'target/limine'), str(ROOT)]
    source_hashes = {path: sha256(ROOT / path) for path in SOURCES}
    if not args.skip_build:
        with (evidence / 'build.log').open('w') as log:
            subprocess.run(build_command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(ROOT / 'target/x86_64-unknown-none/debug/sunlight-kernel', elf)
            subprocess.run(iso_command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    summary = dict(phase='F1.3N-A', source_sha256=source_hashes,
                   revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                   elf_sha256=sha256(elf), iso_sha256=sha256(iso),
                   build_command=build_command, iso_command=iso_command,
                   qemu=subprocess.check_output(['qemu-system-x86_64', '--version'], text=True).splitlines()[0],
                   results=[])
    for cpus in args.cpus:
        serial = evidence / f'{cpus}cpu.serial.log'
        serial.write_text('')
        command = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max',
                   '-smp', str(cpus), '-m', '1024', '-cdrom', str(iso), '-serial',
                   f'file:{serial}', '-display', 'none', '-nic', 'none',
                   '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04', '-no-reboot', '-no-shutdown']
        start = time.monotonic()
        timed_out = False
        with (evidence / f'{cpus}cpu.qemu.log').open('w') as log:
            process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            try:
                while time.monotonic() - start < args.timeout:
                    raw = serial.read_text(errors='replace')
                    if '[F13NA] PASS ' in raw or 'PANIC' in raw or 'panicked at' in raw or process.poll() is not None:
                        break
                    time.sleep(0.1)
                else:
                    timed_out = True
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                exit_code = process.wait()
        raw = serial.read_text(errors='replace')
        validation = validate_native(raw, cpus)
        if timed_out:
            validation['classification'] = 'BLOCKED'
            validation['errors'].append('external QEMU timeout')
        summary['results'].append(dict(cpus=cpus, command=command, timeout=timed_out,
                                       exit_code=exit_code, wall_seconds=time.monotonic() - start,
                                       validation=validation))
        (evidence / 'results.json').write_text(json.dumps(summary, indent=2) + '\n')
        print(f'{cpus} CPUs: {validation["classification"]}: {validation["errors"][:6]}', flush=True)
    if any(row['validation']['classification'] != 'FORCED_AND_PASSED' for row in summary['results']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
