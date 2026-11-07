#!/usr/bin/env python3
"""Destructive only to generated target/partition-storage fixtures. No root needed.
Builds the explicit kernel fixture gate, boots each disk twice over virtio-blk,
and checks guest and host file bytes plus unchanged sectors outside the volume.
"""
import os
from pathlib import Path
import struct
import subprocess
import time
import uuid
import zlib

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'target/partition-storage'


def run(*args, **kwargs):
    subprocess.run(args, check=True, cwd=ROOT, **kwargs)


def fixtures():
    volume = OUT / 'volume.img'
    with volume.open('wb') as f:
        f.truncate(64 * 1024 * 1024)
    run('mkfs.fat', '-F32', '-n', 'SUNSTATE', str(volume), stdout=subprocess.DEVNULL)
    hello = OUT / 'HELLO.TXT'
    hello.write_bytes(b'partition fixture\n')
    run('mcopy', '-i', str(volume), str(hello), '::HELLO.TXT')
    data = volume.read_bytes()
    blocks = len(data) // 512
    for layout in ['superfloppy', 'mbr', 'gpt']:
        start = 0 if layout == 'superfloppy' else 2048
        # All fixtures carry SUNSTATE to prove that a label cannot opt into
        # /state without explicit trust; partitions always select /mnt.
        payload = bytearray(data)
        if layout == 'superfloppy':
            payload[510:512] = b'\0\0'
        total = start + blocks + (2048 if start else 0)
        image = bytearray(total * 512)
        image[start*512:(start+blocks)*512] = payload
        if start:
            image[510:512] = b'\x55\xaa'
            image[450] = 0x0c if layout == 'mbr' else 0xee
            struct.pack_into('<II', image, 454, start if layout == 'mbr' else 1,
                             blocks if layout == 'mbr' else total-1)
        if layout == 'gpt':
            entries = bytearray(128*128)
            entries[:16] = uuid.UUID('ebd0a0a2-b9e5-4433-87c0-68b6b72699c7').bytes_le
            entries[16:32] = uuid.UUID('12345678-1234-5678-9abc-123456789abc').bytes_le
            struct.pack_into('<QQ', entries, 32, start, start+blocks-1)
            for current, alternate, array in [(1,total-1,2), (total-1,1,total-33)]:
                h = bytearray(512)
                struct.pack_into('<8sIIIIQQQQ16sQIII', h, 0, b'EFI PART', 0x10000,92,0,0,
                                 current,alternate,34,total-34,
                                 uuid.UUID('87654321-4321-8765-abcd-cba987654321').bytes_le,
                                 array,128,128,zlib.crc32(entries))
                struct.pack_into('<I', h,16,zlib.crc32(h[:92]))
                image[current*512:(current+1)*512] = h
                image[array*512:array*512+len(entries)] = entries
        path = OUT / f'{layout}.img'
        path.write_bytes(image)
        yield layout, path, start, blocks


def boot(layout, image, number):
    log = OUT / f'{layout}-boot-{number}.log'
    err = OUT / f'{layout}-boot-{number}.qemu.log'
    # QEMU opens the serial file asynchronously; never accept an old PASS.
    log.unlink(missing_ok=True)
    args = ['qemu-system-x86_64','-cdrom',str(OUT/'storage.iso'),'-boot','d',
            '-serial',f'file:{log}','-display','none','-m','1024M','-smp','2',
            '-drive',f'id=hd0,file={image},if=none,format=raw,cache=writeback',
            '-device','virtio-blk-pci,disable-modern=on,drive=hd0',
            '-device','virtio-rng-pci,disable-modern=on','-no-reboot','-no-shutdown']
    if os.access('/dev/kvm', os.R_OK | os.W_OK):
        args += ['-enable-kvm']
    with err.open('w') as errors:
        p = subprocess.Popen(args,cwd=ROOT,stdout=errors,stderr=errors)
        try:
            deadline = time.monotonic()+120
            while time.monotonic() < deadline and p.poll() is None:
                content = log.read_text(errors='replace') if log.exists() else ''
                if '[STORAGE] PASS' in content:
                    break
                if '[STORAGE] FAIL' in content or 'panicked at' in content:
                    raise RuntimeError(f'guest failure: {log}')
                time.sleep(0.2)
            else:
                raise RuntimeError(f'guest timeout/exit: {log}; {err}')
        finally:
            p.terminate()
            p.wait(timeout=15)
    content = log.read_text(errors='replace')
    expected = ['[STORAGE] fixture read OK','[STORAGE] remount contents OK',
                '[STORAGE] write sync OK' if number == 1 else '[STORAGE] restart contents OK']
    for marker in expected:
        assert marker in content, (log,marker)
    expected_mount = '/boot' if layout == 'superfloppy' else '/mnt'
    assert f'[VFS] FAT volume mounted at {expected_mount}' in content
    assert '[VFS] FAT volume mounted at /state' not in content
    print(f'{layout} boot {number}: PASS', flush=True)


def main():
    os.chdir(ROOT)
    OUT.mkdir(parents=True,exist_ok=True)
    with (OUT/'build.log').open('w') as log:
        run('cargo','build','-p','sunlight-kernel',env={**os.environ,'SUNLIGHT_STORAGE_TEST':'1','SUNLIGHT_TRUST_LEGACY_STATE':'0'},stdout=log,stderr=log)
        run('bash','tools/make_hybrid_iso.sh','target/x86_64-unknown-none/debug/sunlight-kernel',
            str(OUT/'storage.iso'),stdout=log,stderr=log)
    for layout,image,start,blocks in fixtures():
        before = image.read_bytes()
        for number in [1,2]:
            boot(layout,image,number)
        after = image.read_bytes()
        assert before[:start*512] == after[:start*512]
        assert before[(start+blocks)*512:] == after[(start+blocks)*512:]
        host = subprocess.check_output(['mtype','-i',f'{image}@@{start*512}','::PROBE.TXT'])
        assert host == b'guest durable contents\n', host
    print('All three layouts: guest restart, remount, sync, host contents and partition guards PASS')


if __name__ == '__main__':
    main()
