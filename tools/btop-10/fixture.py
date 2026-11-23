"""Populate a fresh copy of the known disposable FAT image and map its data."""
import hashlib
from pathlib import Path
import shutil
import struct
import subprocess

out = Path("target/btop-10")
source = Path("target/btop-07/fixture/disk.img")
assert source.is_file() and not source.is_symlink()
assert hashlib.sha256(source.read_bytes()).hexdigest() == (
    "5b4392a7619f561871d3e1a3fa31cf4e564b708c2162972898789a1409ce4c59"
)
payload = bytes((offset * 37 + (offset >> 8) * 17 + 11) & 255
                for offset in range(65536))
file = out / "READ10.BIN"
image = out / "disk.img"
with file.open("xb") as stream:
    stream.write(payload)
with image.open("xb") as stream, source.open("rb") as original:
    shutil.copyfileobj(original, stream)
subprocess.run(["mcopy", "-i", f"{image}@@1048576", file, "::READ10.BIN"], check=True)
with image.open("rb") as stream:
    def read_at(offset, size):
        stream.seek(offset)
        return stream.read(size)

    mbr = read_at(0, 512)
    start, length = struct.unpack_from("<II", mbr, 454)
    bpb = read_at(start * 512, 512)
    unit, spc, reserved, fats = struct.unpack_from("<HBHB", bpb, 11)
    fat_size = struct.unpack_from("<I", bpb, 36)[0]
    root = struct.unpack_from("<I", bpb, 44)[0]
    assert (start, length, unit, spc, root) == (2048, 131072, 512, 1, 2)
    data = start + reserved + fats * fat_size
    directory = read_at(data * 512, 512)
    entry, = (directory[index:index+32] for index in range(0, 512, 32)
              if directory[index:index+11] == b"READ10  BIN")
    cluster = struct.unpack_from("<H", entry, 26)[0] | (
        struct.unpack_from("<H", entry, 20)[0] << 16)
    assert struct.unpack_from("<I", entry, 28)[0] == len(payload)
    sectors = []
    for offset in range(0, len(payload), 512):
        sector = data + cluster - 2
        assert start <= sector < start + length and sector not in sectors
        assert read_at(sector * 512, 512) == payload[offset:offset+512]
        sectors.append(sector)
        cluster = struct.unpack("<I", read_at((start + reserved) * 512 + cluster * 4, 4))[0]
        cluster &= 0x0fffffff
    assert cluster >= 0x0ffffff8
(out / "payload-lbas.txt").write_text("".join(f"{sector}\n" for sector in sectors))
print(f"partition={start}:{start + length} root={data} "
      f"payload={min(sectors)}:{max(sectors) + 1} sectors={len(sectors)}")
