#!/usr/bin/env python3
"""Native privileged storage lifecycle on disposable virtio fixtures, no sudo.

The disk starts unmounted in KERNEL_VFS. Real ring-3 callers exercise the
credential gate, mount API, ordinary file syscalls, busy/dup rejection and
unmount/remount. The older boot-selection fixture remains a separate gate.
"""
import os
from pathlib import Path
import subprocess
import time

import test_partitioned_storage as fixtures

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'target/runtime-storage'
PASS = '[STORAGE-RUNTIME] mount write read sync busy close unmount remount exact PASS'


def boot(layout, image):
    serial = OUT / f'{layout}.serial.log'
    serial.unlink(missing_ok=True)
    args = ['qemu-system-x86_64', '-cdrom', str(OUT/'runtime.iso'), '-boot', 'd',
            '-serial', f'file:{serial}', '-display', 'none', '-m', '1024M', '-smp', '2',
            '-drive', f'id=hd0,file={image},if=none,format=raw,cache=writeback',
            '-device', 'virtio-blk-pci,disable-modern=on,drive=hd0',
            '-device', 'virtio-rng-pci,disable-modern=on',
            '-device', 'qemu-xhci,id=xhci', '-device', 'usb-mouse,bus=xhci.0',
            '-no-reboot', '-no-shutdown']
    if os.access('/dev/kvm', os.R_OK | os.W_OK):
        args += ['-enable-kvm']
    with (OUT/f'{layout}.qemu.log').open('w') as errors:
        process = subprocess.Popen(args, cwd=ROOT, stdout=errors, stderr=errors)
        try:
            deadline = time.monotonic()+120
            while time.monotonic() < deadline and process.poll() is None:
                content = serial.read_text(errors='replace') if serial.exists() else ''
                if PASS in content and "name='storagectl' reason=exit code=0" in content:
                    break
                if 'panicked at' in content or "name='storagectl' reason=exit code=1" in content:
                    raise RuntimeError(f'guest failed: {serial}')
                time.sleep(.2)
            else:
                raise RuntimeError(f'guest timeout/exit: {serial}')
        finally:
            process.terminate()
            process.wait(timeout=15)
    content = serial.read_text(errors='replace')
    assert '[STORAGE-RUNTIME] unauthorized PASS' in content, serial
    assert '[VFS] FAT volume mounted at' not in content, serial
    print(f'{layout}: native credential gate and mount lifecycle PASS', flush=True)


def main():
    os.chdir(ROOT)
    OUT.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, 'SUNLIGHT_RUNTIME_STORAGE_TEST': '1',
           'SUNLIGHT_STORAGE_TEST': '0', 'SUNLIGHT_TRUST_LEGACY_STATE': '0',
           'SUNLIGHT_INJECT_PHASE': 'storage-runtime'}
    flags = '-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
    with (OUT/'build.log').open('w') as log:
        fixtures.run('cargo', 'build', '-p', 'sunlight-utils', '--bin', 'storagectl',
                     '--release', '--features', 'storage-test',
                     env={**env, 'RUSTFLAGS': flags}, stdout=log, stderr=log)
        fixtures.run('cargo', 'build', '-p', 'sunlight-kernel', '--features', 'key_inject',
                     env=env, stdout=log, stderr=log)
        fixtures.run('bash', 'tools/make_hybrid_iso.sh',
                     'target/x86_64-unknown-none/debug/sunlight-kernel', str(OUT/'runtime.iso'),
                     stdout=log, stderr=log)
    fixtures.OUT = OUT
    for layout, image, start, blocks in fixtures.fixtures():
        before = image.read_bytes()
        boot(layout, image)
        after = image.read_bytes()
        assert before[:start*512] == after[:start*512]
        assert before[(start+blocks)*512:] == after[(start+blocks)*512:]
        contents = subprocess.check_output(['mtype', '-i', f'{image}@@{start*512}', '::PROBE.TXT'])
        assert contents == b'guest durable contents\n', contents
    print('Runtime storage: all layouts, exact host contents and partition guards PASS')


if __name__ == '__main__':
    main()
