#!/usr/bin/env python3
"""Real init -> storagectl fstab -> KERNEL_VFS; public lsblk snapshots.
Temporarily edits the sole embedded fstab, restores it even on failure. Uses
only disposable target fixtures; no sudo. Run without concurrent builds.
"""
import json
import os
import re
from pathlib import Path
import subprocess
import struct
import time
import uuid
import zlib
import test_partitioned_storage as fixtures

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'target/fstab-inventory/native'
CONFIG = ROOT / 'sunlight-fs/etc/fstab'
PARTUUID = '12345678-1234-5678-9abc-123456789abc'
FLAGS = '-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'


def boot(name, image=None, usb=False):
    serial = OUT / f'{name}.serial.log'
    serial.unlink(missing_ok=True)
    args = ['qemu-system-x86_64', '-cdrom', str(OUT/'fstab.iso'), '-boot', 'd',
            '-serial', f'file:{serial}', '-display', 'none', '-m', '1024M', '-smp', '2',
            '-device', 'virtio-rng-pci,disable-modern=on',
            '-device', 'qemu-xhci,id=hid', '-device', 'usb-mouse,bus=hid.0',
            '-no-reboot', '-no-shutdown']
    if image:
        args += ['-drive', f'id=data,file={image},if=none,format=raw,cache=writeback']
        if usb:
            args += ['-device', 'qemu-xhci,id=storage,p2=1,p3=0',
                     '-device', 'usb-storage,bus=storage.0,port=1,drive=data,removable=on']
        else:
            args += ['-device', 'virtio-blk-pci,disable-modern=on,drive=data']
    if os.access('/dev/kvm', os.R_OK | os.W_OK):
        args += ['-enable-kvm']
    (OUT/f'{name}.command.json').write_text(json.dumps(args, indent=2)+'\n')
    with (OUT/f'{name}.qemu.log').open('w') as errors:
        process = subprocess.Popen(args, cwd=ROOT, stdout=errors, stderr=errors)
        try:
            deadline = time.monotonic()+120
            while time.monotonic() < deadline and process.poll() is None:
                text = serial.read_text(errors='replace') if serial.exists() else ''
                if '[TTY]  exit: echo FSTAB-INVENTORY-DONE -> 0' in text:
                    break
                if any(s in text for s in ['panicked at', '[FAULT]', '[STORAGE-RUNTIME] FAIL']):
                    raise RuntimeError(f'guest failure: {serial}')
                time.sleep(.2)
            else:
                raise RuntimeError(f'guest timeout/exit: {serial}')
        finally:
            process.terminate()
            process.wait(timeout=15)
    text = serial.read_text(errors='replace')
    assert '[FSTAB] boot apply complete' in text, serial
    assert '[FSTAB] REQUIRED FAILURE' not in text, serial
    assert '[STORAGE-INVENTORY] public queries and mutation denial PASS' in text, serial
    assert '[TTY]  exit: lsblk -> 0' in text and '[TTY]  exit: lsblk --json -> 0' in text, serial
    assert '[TTY]  exit: storagectl fstab -> 0' in text, serial
    assert 'PERSISTENT_ID MOUNTPOINT' in text, serial
    encoded = ''.join(re.findall(r'\[LSBLK-JSON-HEX\] ([0-9a-f]+)', text))
    data = json.loads(bytes.fromhex(encoded))
    assert set(data) == {'schema_version', 'devices'} and data['schema_version'] == 1
    schema = json.loads((ROOT/'docs/storage-inventory-v1.schema.json').read_text())
    fields = set(schema['properties']['devices']['items']['required'])
    assert all(set(device) == fields for device in data['devices'])
    table = bytes.fromhex(''.join(re.findall(r'\[LSBLK-TABLE-HEX\] ([0-9a-f]+)', text))).decode()
    (OUT/f'{name}.table.txt').write_text(table)
    assert 'PERSISTENT_ID MOUNTPOINT' in table

    (OUT/f'{name}.inventory.json').write_text(json.dumps(data, indent=2)+'\n')
    if image:
        assert f'[FSTAB] mounted PARTUUID={PARTUUID} /mnt runtime=' in text, serial
        assert '[VFS] FAT volume mounted at /mnt' not in text, 'must use privileged runtime mount'
        assert '[TTY]  exit: cat /mnt/HELLO.TXT -> 0' in text, serial
        devices = data['devices']
        assert len(devices) == 3, devices
        assert f'PARTUUID={PARTUUID} /mnt' in table
        mounted = next(d for d in devices if d['persistent_id'] == f'PARTUUID={PARTUUID}')
        assert mounted['mountpoint'] == '/mnt' and mounted['filesystem'] == 'fat32', mounted
        assert mounted['mount_protected'] is False and mounted['mount_read_only'] is False, mounted
        assert mounted['transport'] == ('usb' if usb else 'virtio'), mounted
        unmounted = next(d for d in devices if d['partition_ordinal'] == 2)
        assert unmounted['mountpoint'] is None and unmounted['filesystem'] is None, unmounted
        assert all(d['trusted_state'] is False and d['removable'] is None for d in devices)
    else:
        assert '[FSTAB] optional missing' in text and data['devices'] == [], serial
        assert '[TTY]  exit: cat /mnt/HELLO.TXT -> 1' in text, serial
    print(f'{name}: fstab, inventory JSON/table, native mutation denial PASS', flush=True)


def main():
    os.chdir(ROOT)
    OUT.mkdir(parents=True, exist_ok=True)
    original = CONFIG.read_bytes()
    env = {**os.environ, 'SUNLIGHT_RUNTIME_STORAGE_TEST': '0',
           'SUNLIGHT_STORAGE_TEST': '0', 'SUNLIGHT_TRUST_LEGACY_STATE': '0',
           'SUNLIGHT_INJECT_PHASE': 'storage-fstab'}
    try:
        # Fixed GPT UUID, never a session ID. The missing entry is optional.
        CONFIG.write_bytes(original + f'\nPARTUUID={PARTUUID} /mnt fat32 rw,nofail\n'.encode())
        with (OUT/'build.log').open('w') as log:
            fixtures.run('cargo', 'build', '-p', 'sunlight-utils', '--bin', 'storagectl', '--bin', 'lsblk',
                         '--release', '--features', 'storage-test', env={**env, 'RUSTFLAGS': FLAGS}, stdout=log, stderr=log)
            fixtures.run('cargo', 'build', '-p', 'sunlight-init', '-p', 'sunlight-vfs-server', '--release',
                         env={**env, 'RUSTFLAGS': FLAGS}, stdout=log, stderr=log)
            fixtures.run('cargo', 'build', '-p', 'sunlight-kernel', '--features', 'key_inject', env=env, stdout=log, stderr=log)
            fixtures.run('bash', 'tools/make_hybrid_iso.sh', 'target/x86_64-unknown-none/debug/sunlight-kernel',
                         str(OUT/'fstab.iso'), stdout=log, stderr=log)
        fixtures.OUT = OUT
        for layout, image, start, blocks in fixtures.fixtures():
            if layout != 'gpt':
                continue
            payload = bytearray(image.read_bytes())
            total = len(payload)//512
            # Add an unmounted non-FAT partition; update both existing GPT copies.
            entries = bytearray(payload[2*512:34*512])
            entries[128:144] = uuid.UUID('0fc63daf-8483-4772-8e79-3d69d8477de4').bytes_le
            entries[144:160] = uuid.UUID('abcdef01-1234-5678-9abc-123456789abc').bytes_le
            struct.pack_into('<QQ', entries, 160, start+blocks+1, start+blocks+128)
            for current, array in [(1,2), (total-1,total-33)]:
                payload[array*512:array*512+len(entries)] = entries
                h = bytearray(payload[current*512:(current+1)*512])
                struct.pack_into('<I', h, 88, zlib.crc32(entries))
                struct.pack_into('<I', h, 16, 0)
                struct.pack_into('<I', h, 16, zlib.crc32(h[:92]))
                payload[current*512:(current+1)*512] = h
            for name, usb in [('virtio-gpt',False), ('usb-gpt',True)]:
                disk = OUT/f'{name}.img'
                disk.write_bytes(payload)
                boot(name,disk,usb)
                assert disk.read_bytes() == payload, 'inventory/mount/read must leave bytes unchanged'
        boot('optional-absent')
    finally:
        CONFIG.write_bytes(original)
    print('Configuration restored. Rebuild normal embedded tools/kernel/ISO after regression gates.', flush=True)


if __name__ == '__main__':
    main()
