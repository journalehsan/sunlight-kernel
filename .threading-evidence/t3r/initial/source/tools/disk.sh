#!/usr/bin/env bash
# tools/disk.sh — create a deterministic FAT32 test image for Phase 3.5
set -euo pipefail

DISK_IMG="target/test.img"
DISK_ROOT="target/disk-root"

# Check for required tools
for tool in dd mkfs.fat mmd mcopy; do
    if ! command -v "$tool" &>/dev/null; then
        echo "[disk] ERROR: '$tool' not found. Install mtools and dosfstools."
        exit 1
    fi
done

mkdir -p target

# Create a fresh 64 MiB raw disk image
dd if=/dev/zero of="$DISK_IMG" bs=1M count=64 status=none

# Format as FAT32 (no partition table — raw image for virtio-blk-pci)
mkfs.fat -F32 -n SUNLIGHTOS "$DISK_IMG" >/dev/null

# Build the directory tree with known test content
rm -rf "$DISK_ROOT"
mkdir -p "$DISK_ROOT/BOOT"

# Content matches Phase 3.5 gate expectations in tools/tests/phase3_5.expected
printf 'SunlightOS FAT32 boot volume\n' > "$DISK_ROOT/HELLO.TXT"
printf 'Phase 3.5 FAT32 OK\n'           > "$DISK_ROOT/BOOT/PHASE35.TXT"

# Copy files onto the FAT32 image with mtools (no root required)
mcopy -i "$DISK_IMG"  "$DISK_ROOT/HELLO.TXT"         ::HELLO.TXT
mmd   -i "$DISK_IMG"  ::BOOT
mcopy -i "$DISK_IMG"  "$DISK_ROOT/BOOT/PHASE35.TXT"  ::BOOT/PHASE35.TXT

# Clear the BIOS boot signature so SeaBIOS won't try to boot the data disk
# before the CD-ROM. The FAT32 BPB at offset 510-511 normally has 0x55 0xAA,
# which SeaBIOS interprets as a bootable disk. Zeroing it prevents that while
# leaving the FAT32 filesystem intact (our kernel reads FAT32 directly via the
# virtio-blk driver without needing the BIOS boot sector).
python3 -c "
import sys
with open('$DISK_IMG', 'r+b') as f:
    f.seek(510)
    f.write(b'\x00\x00')
"

echo "[disk] $DISK_IMG created (64 MiB FAT32 with HELLO.TXT and BOOT/PHASE35.TXT)"
