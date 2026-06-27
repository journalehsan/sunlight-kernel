#!/usr/bin/env bash
# Run SunlightOS with VirtIO GPU 2D for hardware cursor support.
# QEMU shows two display outputs:
#   • virtio-gpu-pci — desktop with smooth hardware cursor (primary)
#   • VGA std        — Limine boot output / fallback framebuffer
#
# To see only the VirtIO GPU window, switch to QEMU's GPU tab in the GTK toolbar.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — QEMU VirtIO GPU Runner (hardware cursor)${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ ! -f "$ISO_PATH" ]; then
    echo -e "${RED}Error: ISO not found at $ISO_PATH${NC}"
    echo -e "${YELLOW}Run './tools/build.sh' first${NC}"
    exit 1
fi

if ! command -v qemu-system-x86_64 &>/dev/null; then
    echo -e "${RED}Error: qemu-system-x86_64 not found${NC}"
    exit 1
fi

echo "QEMU Configuration:"
echo "  • Memory: 256 MiB"
echo "  • Display devices: virtio-gpu-pci + vga std (BIOS boot)"
echo "  • Cursor: hardware (MOVE_CURSOR via VirtIO GPU)"
echo "  • Network: virtio-net-pci (legacy mode)"
echo ""
echo -e "${YELLOW}Starting QEMU with VirtIO GPU...${NC}"
echo -e "${YELLOW}Switch to the 'virtio-gpu-pci' tab in the QEMU GTK window to see the desktop.${NC}"
echo ""

qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -m 256M \
    -vga std \
    -device virtio-gpu-pci \
    -display gtk \
    -serial stdio \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0,disable-modern=on \
    -no-reboot \
    -no-shutdown \
    "$@"

echo ""
echo -e "${GREEN}QEMU exited${NC}"
