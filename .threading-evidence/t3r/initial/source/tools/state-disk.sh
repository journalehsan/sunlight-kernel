#!/usr/bin/env bash
# Create the dedicated writable FAT32 volume mounted by SunlightOS at /state.
set -euo pipefail

STATE_IMAGE="${1:-target/state.img}"
STATE_SIZE_MIB="${SUNLIGHT_STATE_SIZE_MIB:-256}"

for tool in dd mkfs.fat; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "[state-disk] ERROR: '$tool' is required" >&2
        exit 1
    fi
done

mkdir -p "$(dirname "$STATE_IMAGE")"
dd if=/dev/zero of="$STATE_IMAGE" bs=1M count="$STATE_SIZE_MIB" status=none
mkfs.fat -F32 -n SUNSTATE "$STATE_IMAGE" >/dev/null

# Avoid making the data volume a BIOS boot candidate. The FAT BPB remains
# readable by SunlightOS, which does not require the trailing boot signature.
dd if=/dev/zero of="$STATE_IMAGE" bs=1 seek=510 count=2 conv=notrunc status=none

echo "[state-disk] $STATE_IMAGE created (${STATE_SIZE_MIB} MiB FAT32, label SUNSTATE)"
