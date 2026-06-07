#!/usr/bin/env bash
# Run SunlightOS in QEMU with VNC display (works in headless environments)

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
echo -e "${GREEN}  SunlightOS — QEMU VNC Runner${NC}"
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
    exit 1
fi

echo -e "${GREEN}✓${NC} QEMU found"
echo ""

# Display options
echo "QEMU Configuration:"
echo "  • Memory: 256 MiB"
echo "  • VGA: std (VESA)"
echo "  • Display: VNC on :0 (port 5900)"
echo "  • Serial: stdio"
echo ""
echo -e "${YELLOW}Starting QEMU with VNC...${NC}"
echo -e "${YELLOW}Connect with: vncviewer localhost:5900${NC}"
echo ""

# Run QEMU with VNC
qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -m 256M \
    -vga std \
    -vnc :0 \
    -serial stdio \
    -no-reboot \
    "$@"

echo ""
echo -e "${GREEN}QEMU exited${NC}"
