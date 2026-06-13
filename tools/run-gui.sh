#!/usr/bin/env bash
# Run SunlightOS in QEMU with graphical display to view the boot TUI

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — QEMU GUI Runner${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# Check if ISO exists
if [ ! -f "$ISO_PATH" ]; then
    echo -e "${RED}Error: ISO not found at $ISO_PATH${NC}"
    echo -e "${YELLOW}Run './tools/build.sh' first to build the kernel${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} ISO found: $ISO_PATH"
echo ""

# Check for QEMU
if ! command -v qemu-system-x86_64 &> /dev/null; then
    echo -e "${RED}Error: qemu-system-x86_64 not found${NC}"
    echo -e "${YELLOW}Install QEMU: sudo pacman -S qemu-full${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} QEMU found"
echo ""

# Display options
echo "QEMU Configuration:"
echo "  • Memory: 256 MiB"
echo "  • VGA: std (VESA)"
echo "  • Display: GTK window"
echo "  • Serial: stdio (visible in terminal)"
echo ""
echo -e "${YELLOW}Starting QEMU...${NC}"
echo ""

# Run QEMU with GUI
# -vga std provides VESA framebuffer that Limine can use
# -display gtk gives us a nice graphical window
# -serial stdio shows serial output in the terminal
# -m 256M gives 256 MiB of RAM
qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -m 256M \
    -vga std \
    -display gtk \
    -serial stdio \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0,disable-modern=on \
    -no-reboot \
    -no-shutdown \
    "$@"

echo ""
echo -e "${GREEN}QEMU exited${NC}"
