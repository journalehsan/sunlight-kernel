#!/usr/bin/env python3
"""SFS-5B.2 final-source regression and disposable legacy PCI reset observer.

No existing image, host block device, filesystem mount, commit or push is used.
Hosted suites are reused unchanged from the SFS-5B.1 runner with OUT redirected.
"""
import argparse
import datetime
import gzip
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-5b2'
spec = importlib.util.spec_from_file_location('sfs5b1_runner', ROOT/'tools/sunfs5b1_validate.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.OUT = OUT


def write(p, v):
    p.write_text(json.dumps(v, indent=2)+'\n')


def run(argv, log, timeout=900):
    start = time.monotonic()
    with log.open('w') as f:
        try:
            p = subprocess.run(argv, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, timeout=timeout)
            code = p.returncode
        except subprocess.TimeoutExpired:
            code = 'TIMEOUT'
    row = dict(argv=argv, returncode=code, seconds=time.monotonic()-start, log=str(log.relative_to(OUT)))
    with (log.parent/'commands.jsonl').open('a') as f:
        f.write(json.dumps(row)+'\n')
    print(log.name, code, flush=True)
    return row


class QMP:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(5)
        self.sock.connect(str(path))
        self.stream = self.sock.makefile('rwb', buffering=0)
        self.greeting = self.read()
        self.command('qmp_capabilities')

    def read(self):
        while True:
            value = json.loads(self.stream.readline())
            if 'event' not in value:
                return value

    def command(self, name, args=None):
        msg = {'execute': name}
        if args is not None:
            msg['arguments'] = args
        self.stream.write((json.dumps(msg)+'\n').encode())
        return self.read()

    def close(self):
        self.stream.close()
        self.sock.close()


def native(raw):
    row = run(['cargo', 'build', '-p', 'sunlight-kernel', '--offline', '--locked', '--features', 'sfs5b2-test'], raw/'guest-build.log')
    if row['returncode'] != 0:
        write(raw/'native-result.json', dict(status='BLOCKED', build=row))
        return
    artifacts = ROOT/'target/sfs5b2'/raw.name
    artifacts.mkdir(parents=True)
    elf = artifacts/'sunlight-kernel.elf'
    shutil.copyfile(ROOT/'target/x86_64-unknown-none/debug/sunlight-kernel', elf)
    iso = artifacts/'sfs5b2.iso'
    row = run(['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso)], raw/'iso-build.log')
    if row['returncode'] != 0:
        write(raw/'native-result.json', dict(status='BLOCKED', build=row))
        return
    events = raw/'trace-events.conf'
    events.write_text('\n'.join(['virtio_set_status', 'virtio_queue_notify', 'virtqueue_pop', 'virtqueue_fill', 'virtqueue_flush', 'virtio_blk_req_complete', 'virtio_blk_handle_read', 'virtio_blk_handle_write'])+'\n')
    run(['qemu-system-x86_64', '--version'], raw/'qemu-version.log')
    rows = []
    for cpu in [1, 2, 4]:
        for cache in ['writeback', 'none']:
            tag = f'cpu-{cpu}-{cache}'
            image = artifacts/(tag+'.img')
            expected = bytearray(8192*512)
            expected[:len(base.MARKER)] = base.MARKER
            with image.open('xb') as f:
                f.write(expected)
                f.flush()
                os.fsync(f.fileno())
            serial = raw/(tag+'.serial.log')
            trace = raw/(tag+'.trace.log')
            qmp = Path('/tmp')/f'sfs5b2-{os.getpid()}-{tag}.qmp'
            argv = ['qemu-system-x86_64', '-machine', 'q35', '-accel', 'tcg', '-m', '1024M', '-smp', str(cpu),
                    '-cdrom', str(iso), '-boot', 'd', '-display', 'none', '-serial', f'file:{serial}', '-monitor', 'none',
                    '-qmp', f'unix:{qmp},server=on,wait=off',
                    '-drive', f'id=sfs5b2test,file={image},if=none,format=raw,cache={cache}',
                    '-device', 'virtio-blk-pci,disable-modern=on,drive=sfs5b2test,serial=SFS5B2-disposable',
                    '-device', 'virtio-rng-pci,disable-modern=on', '-nic', 'none', '-no-reboot', '-no-shutdown',
                    '-trace', f'events={events},file={trace}']
            write(raw/(tag+'.invocation.json'), dict(argv=argv, cpu=cpu, cache=cache, source_sha256=base.source(),
                  elf_sha256=base.sha(elf), iso_sha256=base.sha(iso), image_initial_sha256=base.sha(image), seed=None,
                  destructive_lbas=[61], test_scope='normal I/O, idle whole-device reset, retained DMA, no AP execution'))
            ready = False
            witness = {}
            with (raw/(tag+'.qemu.log')).open('w') as errors:
                p = subprocess.Popen(argv, cwd=ROOT, stdout=errors, stderr=errors)
                try:
                    end = time.monotonic()+90
                    while p.poll() is None and time.monotonic() < end:
                        content = serial.read_text(errors='replace') if serial.exists() else ''
                        if '[SFS5B2] READY-FOR-HOST-OBSERVER' in content:
                            ready = True
                            break
                        if 'panicked at' in content:
                            break
                        time.sleep(.1)
                    if ready:
                        client = QMP(qmp)
                        try:
                            witness['greeting'] = client.greeting
                            witness['devices'] = client.command('x-query-virtio')
                            # Experimental x-query-virtio-status aborts QEMU 11.1.2
                            # after reset; preserved first attempt demonstrates it.
                            # Inventory is independent of guest counters; reset ENTRY
                            # and request handling are observed by QEMU tracing.
                            witness['pci'] = client.command('query-pci')
                            witness['block'] = client.command('query-block')
                            witness['vm_status'] = client.command('query-status')
                        finally:
                            client.close()
                except Exception as e:
                    witness['error'] = repr(e)
                finally:
                    if p.poll() is None:
                        p.kill()
                    code = p.wait(timeout=10)
            write(raw/(tag+'.qmp.json'), witness)
            expected[61*512:62*512] = bytes([0xb2])*512
            exact = image.read_bytes() == expected
            (raw/(tag+'.img.gz')).write_bytes(gzip.compress(image.read_bytes(), mtime=0))
            row = dict(cpu=cpu, cache=cache, status='PASS' if ready and exact and 'error' not in witness and code == -9 else 'FAIL', ready=ready,
                       host_exact_image=exact, returncode=code, active_dma_at_reset=False, frames_released=0, observer_error=witness.get('error'),
                       ap_execution='NOT_RUN', serial=serial.name, trace=trace.name, qmp=tag+'.qmp.json')
            rows.append(row)
            write(raw/'native-result.json', dict(status='PASS' if all(r['status']=='PASS' for r in rows) else 'PARTIAL', boots=rows))
            print(tag, row['status'], flush=True)


def checks(raw):
    rows = []
    for name, argv in [
        ('format', ['rustfmt', '--edition', '2021', '--check', 'sunlight-virtio/src/blk.rs', 'kernel/src/sfs5b2_test.rs']),
        ('whitespace', ['git', 'diff', '--check']),
        ('clippy', ['cargo', 'clippy', '-p', 'sunlight-virtio', '-p', 'sunlight-block', '--lib', '--offline', '--locked', '--target', 'x86_64-unknown-none']),
    ]:
        rows.append(run(argv, raw/(name+'.log')))
    write(raw/'checks-result.json', rows)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['native', 'hosted', 'regression-native', 'checks'])
    mode = parser.parse_args().mode
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    raw = OUT/'raw'/f'{mode}-{stamp}'
    raw.mkdir()
    frozen = base.source()
    write(raw/'source-before.json', frozen)
    write(raw/'runner-identity.json', dict(argv=sys.argv, runner_sha256=base.sha(Path(__file__)),
          reused_runner_sha256=base.sha(ROOT/'tools/sunfs5b1_validate.py'), deterministic_seed='no random injection',
          hosted_seeds={'SFS1':[0,1,2,0x534653], 'SFS2':[0,1,2,0x53465332], 'SFS3':[0x53465333], 'SFS4':[0x53465334]},
          environment={k:os.environ.get(k) for k in ['RUSTFLAGS','RUSTUP_TOOLCHAIN','CARGO_BUILD_TARGET','CARGO_TARGET_DIR','SOURCE_DATE_EPOCH']}))
    for tool in ['rustc', 'cargo']:
        run([tool, '--version', '--verbose'], raw/(tool+'-version.log'))
    if mode == 'native':
        native(raw)
    elif mode == 'hosted':
        base.hosted(raw)
    elif mode == 'regression-native':
        base.native(raw)
    else:
        checks(raw)
    after = base.source()
    write(raw/'source-after.json', after)
    changed = [n for n in sorted(set(frozen)|set(after)) if frozen.get(n) != after.get(n)]
    write(raw/'input-integrity.json', dict(status='PASS' if not changed else 'FAIL', changed=changed))
    assert not changed, changed
    print(raw, flush=True)


if __name__ == '__main__':
    main()
