"""Read the MBR and allocation map of the disposable regular file."""
import argparse
from pathlib import Path
import struct
cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument("image", type=Path)
cli.add_argument("--empty", action="store_true")
args = cli.parse_args()
path = args.image
if not path.is_file() or path.is_symlink():
    raise ValueError("expected a disposable regular-file image")
with path.open("rb") as image:
    mbr = image.read(512)
    assert mbr[510:] == b"\x55\xaa"
    assert struct.unpack_from("<I", mbr, 440)[0] == 0xB7070001
    assert mbr[450] == 0x0C and not any(mbr[462:510])
    start, length = struct.unpack_from("<II", mbr, 454)
    assert (start + length) * 512 == path.stat().st_size
    image.seek(start * 512)
    bpb = image.read(512)
    unit, spc, reserved, fats = struct.unpack_from("<HBHB", bpb, 11)
    sectors, fat_len = struct.unpack_from("<II", bpb, 32)
    assert sectors == length and unit == 512 and fats == 2
    assert bpb[71:82].rstrip() == b"BTOP07"
    assert struct.unpack_from("<I", bpb, 67)[0] == 0xB7070001
    blocks = (sectors - reserved - fats * fat_len) // spc
    image.seek((start + reserved) * 512)
    fat = image.read(fat_len * 512)
    assert fat == image.read(fat_len * 512)
    free = sum((entry & 0x0FFFFFFF) == 0
               for (entry,) in struct.iter_unpack("<I", fat[8:4 * (blocks + 2)]))
    if args.empty:
        root = struct.unpack_from("<I", bpb, 44)[0]
        assert root == 2 and free == blocks - 1
        image.seek((start + reserved + fats * fat_len) * unit)
        directory = image.read(spc * unit)
        assert directory[:11] == b"BTOP07     " and directory[11] == 0x08
        assert directory[32] == 0, "fixture root must contain only the volume label"
    print(f"FAT32 BTOP07 b7070001 start={start} sectors={length} "
          f"frsize={unit * spc} blocks={blocks} bfree={free} bavail={free}")
