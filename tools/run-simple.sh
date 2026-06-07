#!/usr/bin/env bash
# Simple QEMU runner - tries multiple display backends automatically

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — Boot with TUI${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ ! -f "$ISO_PATH" ]; then
    echo -e "${RED}✗ ISO not found. Run './tools/build.sh' first${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} ISO found"
echo ""
echo "Trying to launch QEMU with graphical display..."
echo "The TUI will show:"
echo "  • Header: SunlightOS branding with version"
echo "  • Main: Scrolling boot log with color-coded messages"
echo "  • Footer: System status, CPU, RAM info"
echo ""

# Try different display backends in order of preference
DISPLAY_OPTS=(
    "-display sdl"
    "-display gtk"
    "-display cocoa"
    "-display curses"
)

for DISPLAY in "${DISPLAY_OPTS[@]}"; do
    echo -e "${YELLOW}Attempting: qemu-system-x86_64 $DISPLAY${NC}"
    
    if timeout 15 qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -m 256M \
        -vga std \
        $DISPLAY \
        -serial stdio \
        -no-reboot \
        2>&1; then
        echo ""
        echo -e "${GREEN}QEMU exited successfully${NC}"
        exit 0
    fi
    
    echo -e "${YELLOW}Failed, trying next option...${NC}"
    echo ""
done

# Fallback to no display
echo -e "${YELLOW}All graphical displays failed. Running with serial output only...${NC}"
echo -e "${YELLOW}(The graphical TUI is still rendering, but not visible)${NC}"
echo ""

timeout 15 qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -m 256M \
    -vga std \
    -display none \
    -serial stdio \
    -no-reboot \
    2>&1 || true

echo ""
echo -e "${GREEN}Done${NC}"
