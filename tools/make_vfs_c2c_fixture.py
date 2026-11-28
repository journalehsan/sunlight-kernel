#!/usr/bin/env python3
"""Create and verify a disposable GPT FAT32 image in a new directory."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
from uuid import UUID


BLOCK = 512
DISK_BLOCKS = 131072
START = 2048
BLOCKS = 128990
DISK_UUID = "d1f90f21-14aa-44c7-a2b0-1281e5b58df9"
FSTAB = Path(__file__).resolve().parents[1] / "sunlight-fs/etc/fstab.vfs-c2c"
MARKER = b"VFS-N1O-DISPOSABLE-ONLY!"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="new fixture directory")
    args = parser.parse_args()
    directory = args.directory.resolve()
    fields = FSTAB.read_text().splitlines()[-1].split()
    source, target, filesystem, options = fields
    guid = str(UUID(source.removeprefix("PARTUUID=")))
    if (source, target, filesystem, options) != (f"PARTUUID={guid}", "/mnt", "fat32", "rw"):
        raise ValueError("unexpected fixture fstab")
    directory.mkdir()
    image = directory / "disposable.raw"
    volume = directory / "initial.fat"
    seed = directory / "KEEP.TXT"
    seed.write_bytes(b"untouched")
    shutil.copyfile(FSTAB, directory / "fstab")
    with image.open("xb") as disk:
        disk.truncate(DISK_BLOCKS * BLOCK)
    subprocess.run(
        ["sfdisk", "--wipe", "never", str(image)],
        input=f"label: gpt\nlabel-id: {DISK_UUID}\nstart={START}, size={BLOCKS}, type=U, uuid={guid}\n",
        text=True, check=True,
    )
    subprocess.run(["mkfs.fat", "-F", "32", "-s", "1", "-C", str(volume), str(BLOCKS // 2)],
                   check=True)
    subprocess.run(["mcopy", "-i", str(volume), str(seed), "::KEEP.TXT"], check=True)
    with image.open("r+b") as disk, volume.open("rb") as fat:
        disk.write(MARKER)
        disk.seek(START * BLOCK)
        shutil.copyfileobj(fat, disk)
    subprocess.run(["sfdisk", "--verify", str(image)], check=True)
    table = json.loads(subprocess.check_output(["sfdisk", "--json", str(image)]))["partitiontable"]
    (partition,) = table["partitions"]
    if (table["label"], partition["start"], partition["size"], partition["uuid"].lower()) != (
        "gpt", START, BLOCKS, guid
    ):
        raise ValueError("unexpected GPT geometry or identity")
    with image.open("rb") as disk:
        prefix = disk.read(START * BLOCK)
        embedded = disk.read(BLOCKS * BLOCK)
        suffix = disk.read()
    if embedded != volume.read_bytes() or len(prefix) + len(embedded) + len(suffix) != DISK_BLOCKS * BLOCK:
        raise ValueError("unexpected image contents")
    subprocess.run(["fsck.fat", "-n", str(volume)], check=True)
    if subprocess.check_output(["mtype", "-i", str(volume), "::KEEP.TXT"]) != seed.read_bytes():
        raise ValueError("incorrect FAT seed payload")
    (directory / "fixture.json").write_text(json.dumps({
        "partuuid": guid,
        "partition_start": START,
        "partition_blocks": BLOCKS,
        "disk_blocks": DISK_BLOCKS,
        "prefix_sha256": hashlib.sha256(prefix).hexdigest(),
        "suffix_sha256": hashlib.sha256(suffix).hexdigest(),
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
