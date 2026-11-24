"""Replace existing test-only payloads in a fresh copy of the archived ISO."""
import hashlib
import mmap
from pathlib import Path
import shutil
import sys

target = Path("target")
launcher, leader, output = map(Path, sys.argv[1:])
source = target / "btop-22/final/guest-ramfs-trace-config.iso"
assert hashlib.file_digest(source.open("rb"), "sha256").hexdigest() == (
    "e7575a42b56655c41b578157b8cf4fa9c2e575dd25b8bffff5d90a8bb79eb148")
btop = (target / "btop-01/btop").read_bytes()
assert hashlib.sha256(btop).hexdigest() == (
    "bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702")
with source.open("rb") as original, output.open("xb+") as copy:
    shutil.copyfileobj(original, copy)
    copy.flush()
    with mmap.mmap(copy.fileno(), 0) as image:
        for path, binary in (
            ("btop-03/tty-select.elf", launcher),
            ("btop-06/masked-select-guest.elf", leader),
            ("btop-06/masked-select-original.elf", leader),
            ("btop-02/procfs.elf", leader),
            ("btop-04/route-scope.elf", leader),
        ):
            old = (target / path).read_bytes()
            new = binary.read_bytes()
            assert len(new) <= len(old), path
            first = image.find(old)
            second = image.find(old, first + len(old))
            assert first >= 0 and second > first
            assert image.find(old, second + len(old)) == -1
            for offset in (first, second):
                image[offset:offset + len(old)] = new.ljust(len(old), b"\0")
                print(f"{path}: offset={offset} size={len(old)}")
        assert image.find(btop) == 33436780
