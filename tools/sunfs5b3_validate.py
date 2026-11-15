#!/usr/bin/env python3
"""SFS-5B.3 isolated QEMU witness and unchanged hosted regression runner.

QEMU must be built from the recorded source and patch. Only exclusively created
regular disposable images are attached. No host block device or existing image.
"""
import argparse
import datetime
import gzip
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-5b3'
spec = importlib.util.spec_from_file_location('sfs5b1', ROOT/'tools/sunfs5b1_validate.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.OUT = OUT
QEMU = Path('/tmp/sfs5b3-qemu-build/qemu-system-x86_64')


def write(path, value):
    path.write_text(json.dumps(value, indent=2)+'\n')


def command(argv, log, timeout=900):
    start = time.monotonic()
    with log.open('w') as output:
        try:
            code = subprocess.run(argv, cwd=ROOT, stdout=output,
                                  stderr=subprocess.STDOUT, timeout=timeout).returncode
        except subprocess.TimeoutExpired:
            code = 'TIMEOUT'
    row = dict(argv=argv, returncode=code, seconds=time.monotonic()-start,
               log=str(log.relative_to(OUT)))
    with (log.parent/'commands.jsonl').open('a') as output:
        output.write(json.dumps(row)+'\n')
    print(log.name, code, flush=True)
    return row


def native(raw):
    build = command(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked',
                     '--features', 'sfs5b3-test'], raw/'guest-build.log')
    if build['returncode']:
        write(raw/'native-result.json', dict(status='BLOCKED', build=build))
        return
    artifacts = ROOT/'target/sfs5b3'/raw.name
    artifacts.mkdir(parents=True)
    elf = artifacts/'sunlight-kernel.elf'
    shutil.copyfile(ROOT/'target/x86_64-unknown-none/debug/sunlight-kernel', elf)
    iso = artifacts/'sfs5b3.iso'
    build = command(['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso)], raw/'iso-build.log')
    if build['returncode'] or not QEMU.is_file():
        write(raw/'native-result.json', dict(status='BLOCKED', build=build, qemu_exists=QEMU.is_file()))
        return
    events = raw/'trace-events.conf'
    events.write_text('\n'.join(['virtio_set_status', 'virtqueue_pop', 'virtqueue_fill',
                                 'virtqueue_flush', 'virtio_blk_req_complete',
                                 'virtio_blk_handle_write', 'virtio_blk_handle_read'])+'\n')
    source = base.source()
    rows = []
    for cpu in [1, 2, 4]:
        for cache in ['writeback', 'none']:
            for mode in ['hold', 'release', 'unmodified']:
                tag = f'cpu-{cpu}-{cache}-{mode}'
                image = artifacts/(tag+'.img')
                initial = bytearray(8192*512)
                initial[:len(base.MARKER)] = base.MARKER
                with image.open('xb') as output:
                    output.write(initial)
                    output.flush()
                    os.fsync(output.fileno())
                serial = raw/(tag+'.serial.log')
                trace = raw/(tag+'.trace.log')
                device = raw/(tag+'.qemu.log')
                release = artifacts/(tag+'.release')
                if mode == 'release':
                    release.touch(exist_ok=False)
                env = dict(os.environ)
                # Avoid inherited test controls or an inherited trace target.
                env.pop('SFS5B3_MODE', None)
                env.pop('SFS5B3_RELEASE', None)
                if mode != 'unmodified':
                    env.update(SFS5B3_MODE=mode, SFS5B3_RELEASE=str(release))
                executable = str(QEMU) if mode != 'unmodified' else shutil.which('qemu-system-x86_64')
                argv = [executable, '-machine', 'q35', '-accel', 'tcg', '-m', '1024M', '-smp', str(cpu),
                        '-cdrom', str(iso), '-boot', 'd', '-display', 'none',
                        '-serial', f'file:{serial}', '-monitor', 'none',
                        '-drive', f'id=sfs5b3,file={image},if=none,format=raw,cache={cache}',
                        '-device', 'virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=sfs5b3,serial=SFS5B3-disposable',
                        '-device', 'virtio-rng-pci,disable-modern=on', '-nic', 'none',
                        '-no-reboot', '-no-shutdown', '-trace', f'events={events},file={trace}']
                write(raw/(tag+'.invocation.json'), dict(argv=argv, cpu=cpu, cache=cache, mode=mode,
                      source_sha256=source, elf_sha256=base.sha(elf), iso_sha256=base.sha(iso),
                      qemu_sha256=base.sha(Path(executable)), image_initial_sha256=base.sha(image),
                      patch_sha256=base.sha(OUT/'qemu-active-io.patch'),
                      environment={k:env.get(k) for k in ['SFS5B3_MODE', 'SFS5B3_RELEASE']}))
                ready = False
                with device.open('w') as errors:
                    process = subprocess.Popen(argv, cwd=ROOT, env=env, stdout=errors, stderr=errors)
                    try:
                        deadline = time.monotonic()+90
                        while process.poll() is None and time.monotonic() < deadline:
                            content = serial.read_text(errors='replace') if serial.exists() else ''
                            marker = '[SFS5B3] READY' if mode == 'hold' else '[SFS5B3] NORMAL-READY'
                            if marker in content:
                                ready = True
                                # Fixed, finite post-reset observation window. No DMA drain
                                # inferred merely from elapsed time or this observation.
                                time.sleep(0.5)
                                break
                            if 'panicked at' in content:
                                break
                            time.sleep(0.1)
                    finally:
                        if process.poll() is None:
                            process.kill()
                        code = process.wait(timeout=10)
                content = serial.read_text(errors='replace') if serial.exists() else ''
                witness = device.read_text(errors='replace')
                if mode != 'hold':
                    initial[63*512:64*512] = bytes([0xb3])*512
                exact = image.read_bytes() == initial
                progress = [list(map(int, p.split(', '))) for p in re.findall(r'progress=\[([0-9, ]+)\]', content)]
                advanced = bool(progress) and all(progress[-1][i] > progress[0][i] for i in range(cpu))
                dma = 'SFS5B3 dma-read head=0 offset=0 bytes=256 value=179' in witness
                outstanding = 'held head=0 consumed=256 remaining=256' in witness
                reset = 'reset-entry head=0 outstanding=1' in witness
                detached = 'detached head=0 timer=pending-cancelled' in witness
                fence = 'reset-complete status=0 queues-cleared=1' in witness[witness.find('SFS5B3 held'):]
                post = witness.split('SFS5B3 reset-entry head=0 outstanding=1')[-1] if reset else ''
                post_access = 'SFS5B3 dma-read' in post
                scoped = (dma and outstanding and reset and detached and fence and not post_access) if mode == 'hold' else True
                row = dict(cpu=cpu, cache=cache, mode=mode,
                           status='PASS' if ready and exact and scoped and code == -9 else 'FAIL',
                           ready=ready, returncode=code, exact_image=exact, device_dma_read_observed=dma,
                           real_request_held=outstanding, reset_with_outstanding=reset,
                           device_detach_observed=detached, reset_exit_observed=fence,
                           post_reset_instrumented_access=post_access, simultaneous_memory_instruction=False,
                           progress=progress, all_configured_cpu_timer_progress=advanced,
                           scheduler_progress='NOT_RUN', reset_other_cpu='NOT_RUN',
                           frames_released=0, queue_epoch_reinitialized=False,
                           serial=serial.name, trace=trace.name, device_log=device.name)
                (raw/(tag+'.img.gz')).write_bytes(gzip.compress(image.read_bytes(), mtime=0))
                rows.append(row)
                write(raw/'native-result.json', dict(status='PASS' if all(r['status']=='PASS' for r in rows) else 'PARTIAL', boots=rows))
                print(tag, row['status'], 'AP timer progress', advanced, flush=True)


def checks(raw):
    rows = []
    for name, argv in [
        ('format', ['rustfmt', '--edition', '2021', '--check', 'kernel/src/sfs5b3_test.rs']),
        ('whitespace', ['git', 'diff', '--check']),
        ('clippy', ['cargo', 'clippy', '-p', 'sunlight-virtio', '-p', 'sunlight-block', '--lib',
                    '--offline', '--locked', '--target', 'x86_64-unknown-none']),
    ]:
        rows.append(command(argv, raw/(name+'.log')))
    write(raw/'checks-result.json', rows)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['native', 'hosted', 'checks'])
    mode = parser.parse_args().mode
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw = OUT/'raw'/f'{mode}-{stamp}'
    raw.mkdir()
    before = base.source()
    write(raw/'source-before.json', before)
    write(raw/'runner-identity.json', dict(argv=os.sys.argv, runner_sha256=base.sha(Path(__file__)),
          reused_runner_sha256=base.sha(ROOT/'tools/sunfs5b1_validate.py'),
          environment={k:os.environ.get(k) for k in ['RUSTFLAGS','RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR','CARGO_BUILD_TARGET']}))
    if mode == 'hosted':
        base.hosted(raw)
    elif mode == 'native':
        native(raw)
    else:
        checks(raw)
    after = base.source()
    write(raw/'source-after.json', after)
    changed = [n for n in sorted(set(before)|set(after)) if before.get(n) != after.get(n)]
    write(raw/'input-integrity.json', dict(status='PASS' if not changed else 'FAIL', changed=changed))
    assert not changed, changed
    print(raw, flush=True)


if __name__ == '__main__':
    main()
