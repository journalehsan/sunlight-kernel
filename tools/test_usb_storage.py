#!/usr/bin/env python3
"""Real xHCI -> USB BOT/SCSI -> runtime storagectl -> FAT/VFS, no virtio disk.
Only generated target/usb-storage images are modified. No root required.
"""
import json
import os
import re
from pathlib import Path
import subprocess
import socket
import time

import test_partitioned_storage as fixtures
import test_runtime_storage as runtime

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'target/usb-storage'


def mouse_report(content):
    # DebugLog appends a newline to each existing HID diagnostic fragment;
    # concurrent service log lines may interleave those fragments.
    fragments = re.sub(r'(?m)^\[.*(?:\n|$)', '', content)
    return re.search(r'emit_dx=\s*37\s*emit_dy=\s*-\s*11(?:\s|$)', fragments) is not None


def boot(name, image, negative=False, sector_size=512):
    serial = OUT / f'{name}.serial.log'
    serial.unlink(missing_ok=True)
    qmp_path = OUT / f'{name}.qmp'
    qmp_path.unlink(missing_ok=True)
    args = ['qemu-system-x86_64', '-cdrom', str(OUT/'usb.iso'), '-boot', 'd',
            '-serial', f'file:{serial}', '-display', 'none', '-m', '1024M', '-smp', '2',
            '-device', 'virtio-rng-pci,disable-modern=on',
            '-device', 'qemu-xhci,id=hid', '-device', 'usb-mouse,bus=hid.0',
            '-device', 'qemu-xhci,id=storage,p2=1,p3=0',
            '-drive', f'id=usb0,file={image},if=none,format=raw,cache=writeback',
            '-device', f'usb-storage,bus=storage.0,port=1,drive=usb0,removable=on,logical_block_size={sector_size},physical_block_size={sector_size}',
            '-qmp', f'unix:{qmp_path},server=on,wait=off',
            '-no-reboot', '-no-shutdown']
    if os.access('/dev/kvm', os.R_OK | os.W_OK):
        args += ['-enable-kvm']
    (OUT/f'{name}.command.json').write_text(json.dumps(args, indent=2)+'\n')
    marker = ('[USB-MSC] SCSI/sector profile rejected: SectorSize(4096)' if sector_size != 512 else
              '[STORAGE-RUNTIME] USB malformed media rejected PASS' if negative else runtime.PASS)
    with (OUT/f'{name}.qemu.log').open('w') as errors:
        process = subprocess.Popen(args, cwd=ROOT, stdout=errors, stderr=errors)
        try:
            injected = False
            deadline = time.monotonic()+120
            while time.monotonic() < deadline and process.poll() is None:
                content = serial.read_text(errors='replace') if serial.exists() else ''
                # HID diagnostics use separate DebugLog calls for numeric fragments.
                # Wait until display startup and the foreground storage probe have
                # completed so unrelated numeric startup logs cannot split the report.
                if (not injected and '[USB-MOUSE] tty input router ready' in content
                        and '[TTY]  exit: storagectl probe -> 0' in content):
                    with socket.socket(socket.AF_UNIX) as sock:
                        sock.settimeout(3)
                        sock.connect(str(qmp_path))
                        stream = sock.makefile('rwb', buffering=0)
                        json.loads(stream.readline())
                        def execute(command, arguments=None):
                            request = {'execute': command}
                            if arguments is not None:
                                request['arguments'] = arguments
                            stream.write(json.dumps(request).encode()+b'\n')
                            while True:
                                response = json.loads(stream.readline())
                                if 'error' in response:
                                    raise RuntimeError(response)
                                if 'return' in response:
                                    return response
                        execute('qmp_capabilities')
                        execute('input-send-event', {'events': [
                            {'type': 'rel', 'data': {'axis': 'x', 'value': 37}},
                            {'type': 'rel', 'data': {'axis': 'y', 'value': -11}}]})
                    injected = True
                exited = ("name='storagectl' reason=exit code=0" in content
                          and '[TTY]  exit: storagectl probe -> 0' in content)
                hid_report = mouse_report(content)
                if marker in content and (sector_size != 512 or (exited and hid_report)):
                    break
                if 'panicked at' in content or '[FAULT]' in content or '[STORAGE-RUNTIME] FAIL' in content or "name='storagectl' reason=exit code=1" in content:
                    raise RuntimeError(f'guest failed: {serial}')
                time.sleep(.2)
            else:
                raise RuntimeError(f'guest timeout/exit: {serial}')
        finally:
            process.terminate()
            process.wait(timeout=15)
    content = serial.read_text(errors='replace')
    assert '[BLK]  Found virtio-blk' not in content, serial
    assert 'class=08 subclass=06 protocol=50' in content, serial
    assert '[VFS] FAT volume mounted at' not in content, serial
    if sector_size == 512:
        assert '[USB-MSC] registered USB-backed disk' in content, serial
        assert '[STORAGE-RUNTIME] unauthorized PASS' in content, serial
        assert '[USB-MOUSE] boot mouse ready' in content, serial
        assert mouse_report(content), serial
        assert '[STORAGE-RUNTIME] USB-backed registry device observed' in content, serial
    else:
        assert '[USB-MSC] registered USB-backed disk' not in content, serial
    print(f'{name}: {marker} PASS', flush=True)


def main():
    os.chdir(ROOT)
    OUT.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, 'SUNLIGHT_RUNTIME_STORAGE_TEST': '1',
           'SUNLIGHT_STORAGE_TEST': '0', 'SUNLIGHT_TRUST_LEGACY_STATE': '0',
           'SUNLIGHT_INJECT_PHASE': 'storage-runtime'}
    flags = '-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
    with (OUT/'build.log').open('w') as log:
        fixtures.run('cargo', 'build', '-p', 'sunlight-usb-mouse', '--release',
                     '--features', 'usb_mouse_debug', env={**env, 'RUSTFLAGS': flags},
                     stdout=log, stderr=log)
        fixtures.run('cargo', 'build', '-p', 'sunlight-utils', '--bin', 'storagectl',
                     '--release', '--features', 'storage-test',
                     env={**env, 'RUSTFLAGS': flags}, stdout=log, stderr=log)
        fixtures.run('cargo', 'build', '-p', 'sunlight-kernel', '--features', 'key_inject',
                     env=env, stdout=log, stderr=log)
        fixtures.run('bash', 'tools/make_hybrid_iso.sh',
                     'target/x86_64-unknown-none/debug/sunlight-kernel', str(OUT/'usb.iso'),
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
    malformed = OUT/'malformed.img'
    malformed.write_bytes(bytes(4*1024*1024))
    boot('malformed', malformed, negative=True)
    assert malformed.read_bytes() == bytes(4*1024*1024)
    boot('4kn', malformed, sector_size=4096)
    assert malformed.read_bytes() == bytes(4*1024*1024)
    print('USB storage: three layouts, native lifecycle, exact host contents, bounds, malformed/4Kn rejection PASS')


if __name__ == '__main__':
    main()
