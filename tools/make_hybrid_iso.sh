#!/usr/bin/env bash
# Build a Limine hybrid ISO that boots on both legacy BIOS and x86_64 UEFI.
#
# Layout follows the bundled Limine USAGE.md (v8.x):
#   boot/sunlight-kernel.elf
#   boot/limine/{limine.conf,limine-bios.sys,limine-bios-cd.bin,limine-uefi-cd.bin}
#   EFI/BOOT/BOOTX64.EFI   (removable-media UEFI fallback path)
#
# Usage:
#   make_hybrid_iso.sh <kernel_elf> <iso_out> [limine_dir] [project_root]
#
# Environment:
#   LIMINE_BRANCH  — passed to setup_limine.sh (default: v8.x)
set -euo pipefail

if [[ $# -lt 2 || $# -gt 4 ]]; then
    echo "usage: $0 KERNEL_ELF ISO_OUT [LIMINE_DIR] [PROJECT_ROOT]" >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${4:-$(cd "$SCRIPT_DIR/.." && pwd)}"
KERNEL_ELF="$1"
ISO_OUT="$2"
LIMINE_DIR="${3:-$PROJECT_ROOT/target/limine}"
LIMINE_BRANCH="${LIMINE_BRANCH:-v8.x}"
ISO_ROOT="$PROJECT_ROOT/target/iso_root"
LIMINE_CONF="$PROJECT_ROOT/limine.conf"

if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "error: kernel ELF not found: $KERNEL_ELF" >&2
    exit 1
fi
if [[ ! -f "$LIMINE_CONF" ]]; then
    echo "error: limine.conf not found: $LIMINE_CONF" >&2
    exit 1
fi
if ! command -v xorriso >/dev/null 2>&1; then
    echo "error: xorriso is required to build the hybrid ISO" >&2
    exit 1
fi

"$SCRIPT_DIR/setup_limine.sh" "$LIMINE_DIR" "$LIMINE_BRANCH"

for f in limine limine-bios.sys limine-bios-cd.bin limine-uefi-cd.bin BOOTX64.EFI; do
    if [[ ! -e "$LIMINE_DIR/bin/$f" ]]; then
        echo "error: required Limine artifact missing: $LIMINE_DIR/bin/$f" >&2
        exit 1
    fi
done

# Fresh tree every time so stale EFI/BIOS files cannot hide a broken step.
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine"
mkdir -p "$ISO_ROOT/EFI/BOOT"

cp "$KERNEL_ELF" "$ISO_ROOT/boot/sunlight-kernel.elf"
cp "$LIMINE_CONF" "$ISO_ROOT/boot/limine/limine.conf"
cp "$LIMINE_DIR/bin/limine-bios.sys" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-uefi-cd.bin" "$ISO_ROOT/boot/limine/"
# Removable-media UEFI path (also present inside limine-uefi-cd.bin FAT image).
cp "$LIMINE_DIR/bin/BOOTX64.EFI" "$ISO_ROOT/EFI/BOOT/BOOTX64.EFI"

# Keep a copy under boot/limine for operators inspecting the ISO tree; not used
# as the El Torito EFI image (that must be limine-uefi-cd.bin per Limine USAGE).
cp "$LIMINE_DIR/bin/BOOTX64.EFI" "$ISO_ROOT/boot/limine/BOOTX64.EFI"

mkdir -p "$(dirname "$ISO_OUT")"
xorriso -as mkisofs -R -r -J \
    -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    -hfsplus -apm-block-size 2048 \
    --efi-boot boot/limine/limine-uefi-cd.bin \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$ISO_OUT"

"$LIMINE_DIR/bin/limine" bios-install "$ISO_OUT"

echo "hybrid ISO ready: $ISO_OUT"
