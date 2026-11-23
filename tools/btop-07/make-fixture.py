"""Create an exclusive regular-file FAT32 fixture with one MBR partition."""
import os
from pathlib import Path
import struct
import subprocess
import sys

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
volume = out / "volume.img"
disk = out / "disk.img"
size, start = 64 * 1024 * 1024, 2048
with volume.open("xb") as handle:
    handle.truncate(size)
subprocess.run(["mkfs.fat", "-F", "32", "-n", "BTOP07", "-i", "B7070001", str(volume)],
               check=True)
mbr = bytearray(start * 512)
struct.pack_into("<I", mbr, 440, 0xB7070001)
struct.pack_into("<B3xII", mbr, 450, 0x0C, start, size // 512)
mbr[510:512] = b"\x55\xaa"
with disk.open("xb") as handle:
    handle.write(mbr)
    with volume.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            handle.write(chunk)
    print(f"{disk}: file bytes={os.fstat(handle.fileno()).st_size}; partition="
          f"{start}+{size // 512} sectors; FAT label BTOP07 serial b7070001")
