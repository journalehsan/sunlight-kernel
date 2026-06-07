#!/usr/bin/env bash
# Run SunlightOS in QEMU and capture a screenshot of the TUI

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"
SCREENSHOT_PATH="$PROJECT_ROOT/target/boot_tui_screenshot.ppm"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — Screenshot Capture${NC}"
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

echo "Running QEMU with screenshot capture..."
echo "Will capture screen after 3 seconds of boot"
echo ""

# Run QEMU in the background with monitor on stdio
# We'll send a screendump command after boot
timeout 8 qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -m 256M \
    -vga std \
    -display none \
    -serial stdio \
    -monitor telnet:127.0.0.1:55555,server,nowait \
    2>&1 &

QEMU_PID=$!
echo "QEMU started (PID: $QEMU_PID)"

# Wait for boot to complete
sleep 4

# Try to capture screenshot via monitor
echo "Capturing screenshot..."
(echo "screendump $SCREENSHOT_PATH"; sleep 1) | nc localhost 55555 2>/dev/null || true

# Wait a bit more
sleep 2

# Check if screenshot was created
if [ -f "$SCREENSHOT_PATH" ]; then
    echo -e "${GREEN}✓${NC} Screenshot saved to: $SCREENSHOT_PATH"
    
    # Convert to PNG if ImageMagick is available
    if command -v convert &> /dev/null; then
        PNG_PATH="${SCREENSHOT_PATH%.ppm}.png"
        convert "$SCREENSHOT_PATH" "$PNG_PATH" 2>/dev/null && \
        echo -e "${GREEN}✓${NC} PNG version: $PNG_PATH" || true
    fi
else
    echo -e "${YELLOW}Warning: Screenshot not captured${NC}"
    echo "Serial output from boot:"
    echo ""
fi

# Kill QEMU
kill $QEMU_PID 2>/dev/null || true
wait $QEMU_PID 2>/dev/null || true

echo ""
echo -e "${GREEN}Done${NC}"
