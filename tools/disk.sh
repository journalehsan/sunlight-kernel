#!/usr/bin/env bash
set -euo pipefail

DISK_IMAGE="target/sunlightos_disk.img"
DISK_SIZE_MB=64

if [[ -f "$DISK_IMAGE" ]]; then
    echo "[disk] Disk image already exists: $DISK_IMAGE"
    exit 0
fi

echo "[disk] Creating ${DISK_SIZE_MB}MB FAT32 disk image..."

# Create raw disk image
dd if=/dev/zero of="$DISK_IMAGE" bs=1M count=$DISK_SIZE_MB

# Create partition table and FAT32 partition
parted "$DISK_IMAGE" --script -- \
    mklabel msdos \
    mkpart primary fat32 1MiB 100% \
    set 1 boot on

# Setup loop device
LOOP_DEV=$(sudo losetup --find --show --partscan "$DISK_IMAGE")
trap "sudo losetup -d $LOOP_DEV || true" EXIT

# Wait for partition to appear
sleep 1
PART_DEV="${LOOP_DEV}p1"
if [[ ! -b "$PART_DEV" ]]; then
    PART_DEV="${LOOP_DEV//dev/dev\/}1"
fi

# Format FAT32
sudo mkfs.fat -F 32 "$PART_DEV"

# Mount and add files
MOUNT_DIR=$(mktemp -d)
trap "sudo umount $MOUNT_DIR 2>/dev/null || true; sudo losetup -d $LOOP_DEV 2>/dev/null || true; rm -rf $MOUNT_DIR" EXIT

sudo mount "$PART_DEV" "$MOUNT_DIR"

# Copy kernel and limine files if they exist
if [[ -f "target/x86_64-unknown-none/debug/sunlight-kernel" ]]; then
    sudo mkdir -p "$MOUNT_DIR/boot"
    sudo cp "target/x86_64-unknown-none/debug/sunlight-kernel" "$MOUNT_DIR/boot/"
fi

if [[ -f "limine.cfg" ]]; then
    sudo mkdir -p "$MOUNT_DIR/boot/limine"
    sudo cp "limine.cfg" "$MOUNT_DIR/boot/limine/"
fi

sudo umount "$MOUNT_DIR"

echo "[disk] Disk image created: $DISK_IMAGE"
