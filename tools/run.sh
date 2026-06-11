#!/usr/bin/env bash
# Unified script to run SunlightOS in QEMU with various display options

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

show_usage() {
    cat << USAGE
${GREEN}SunlightOS QEMU Runner${NC}

Usage: $0 [OPTIONS]

Build Options:
  -b, --build        Rebuild kernel + services before launching

Display Options:
  -g, --gui          Launch with GTK window (default, requires X11)
  -s, --sdl          Launch with SDL window
  -v, --vnc          Launch with VNC server on :0 (port 5900)
  -c, --curses       Launch with text-mode curses interface
  -n, --no-display   Launch without display (serial only)
  --screenshot       Capture screenshot and exit

QEMU Options:
  -m, --memory MB    Set RAM size (default: 256)
  --debug            Enable QEMU debug output
  --gdb              Wait for GDB connection on port 1234

Other:
  -h, --help         Show this help message

Examples:
  $0 --build         # Rebuild and launch with GTK (most common)
  $0                 # Launch existing ISO with GTK (no rebuild)
  $0 --build --sdl   # Rebuild and launch with SDL
  $0 --vnc           # Launch with VNC on port 5900
  $0 --no-display    # Serial output only
  $0 --screenshot    # Capture boot screenshot

USAGE
}

# Default options
DISPLAY_TYPE="gtk"
MEMORY="256"
DEBUG_MODE=false
GDB_MODE=false
SCREENSHOT_MODE=false
BUILD_FIRST=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -b|--build)
            BUILD_FIRST=true
            shift
            ;;
        -g|--gui)
            DISPLAY_TYPE="gtk"
            shift
            ;;
        -s|--sdl)
            DISPLAY_TYPE="sdl"
            shift
            ;;
        -v|--vnc)
            DISPLAY_TYPE="vnc"
            shift
            ;;
        -c|--curses)
            DISPLAY_TYPE="curses"
            shift
            ;;
        -n|--no-display)
            DISPLAY_TYPE="none"
            shift
            ;;
        --screenshot)
            SCREENSHOT_MODE=true
            shift
            ;;
        -m|--memory)
            MEMORY="$2"
            shift 2
            ;;
        --debug)
            DEBUG_MODE=true
            shift
            ;;
        --gdb)
            GDB_MODE=true
            shift
            ;;
        -h|--help)
            show_usage
            exit 0
            ;;
        *)
            echo -e "${RED}Error: Unknown option: $1${NC}"
            show_usage
            exit 1
            ;;
    esac
done

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — QEMU Runner${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# Rebuild if requested
if [ "$BUILD_FIRST" = true ]; then
    echo -e "${YELLOW}Rebuilding kernel and services...${NC}"
    SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static"
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --release --features sunlight
    cargo build --package sunlight-kernel

    # Download and build Limine bootloader if needed
    LIMINE_DIR="$PROJECT_ROOT/target/limine"
    if [[ ! -d "$LIMINE_DIR" ]]; then
        echo -e "${YELLOW}Downloading and building Limine bootloader...${NC}"
        git clone --branch="v8.x" --depth=1 https://github.com/limine-bootloader/limine.git "$LIMINE_DIR"
        pushd "$LIMINE_DIR" >/dev/null
        ./bootstrap
        ./configure --enable-uefi-x86-64 --enable-bios-cd --enable-bios-pxe
        make -j"$(nproc)"
        popd >/dev/null
        echo -e "${GREEN}✓${NC} Limine built"
    else
        echo -e "${GREEN}✓${NC} Limine already cached"
    fi

    # Repack ISO
    LIMINE_DIR="$PROJECT_ROOT/target/limine"
    KERNEL_ELF="$PROJECT_ROOT/target/x86_64-unknown-none/debug/sunlight-kernel"
    ISO_ROOT="$PROJECT_ROOT/target/iso_root"
    rm -rf "$ISO_ROOT"
    mkdir -p "$ISO_ROOT/boot/limine"
    cp "$KERNEL_ELF" "$ISO_ROOT/boot/sunlight-kernel.elf"
    cp "$PROJECT_ROOT/limine.conf" "$ISO_ROOT/boot/limine/"
    cp "$LIMINE_DIR/bin/limine-bios.sys"    "$ISO_ROOT/boot/limine/"
    cp "$LIMINE_DIR/bin/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
    cp "$LIMINE_DIR/bin/BOOTX64.EFI"        "$ISO_ROOT/boot/limine/"
    xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
        -no-emul-boot -boot-load-size 4 -boot-info-table \
        --efi-boot boot/limine/BOOTX64.EFI \
        -efi-boot-part --efi-boot-image --protective-msdos-label \
        "$ISO_ROOT" -o "$ISO_PATH" 2>/dev/null
    "$LIMINE_DIR/bin/limine" bios-install "$ISO_PATH"
    echo -e "${GREEN}✓${NC} Build complete"
fi

# Check if ISO exists
if [ ! -f "$ISO_PATH" ]; then
    echo -e "${RED}✗ Error: ISO not found${NC}"
    echo -e "${YELLOW}  Run './tools/build.sh' first${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} ISO: $ISO_PATH"

# Check for QEMU
if ! command -v qemu-system-x86_64 &> /dev/null; then
    echo -e "${RED}✗ Error: qemu-system-x86_64 not found${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} QEMU: $(qemu-system-x86_64 --version | head -1)"
echo ""

# Build QEMU command
QEMU_CMD=(
    qemu-system-x86_64
    -cdrom "$ISO_PATH"
    -m "${MEMORY}M"
    -vga std
    -serial stdio
    -no-reboot
)

# Add display option
case $DISPLAY_TYPE in
    gtk)
        QEMU_CMD+=(-display gtk)
        echo -e "${BLUE}Display:${NC} GTK window"
        ;;
    sdl)
        QEMU_CMD+=(-display sdl)
        echo -e "${BLUE}Display:${NC} SDL window"
        ;;
    vnc)
        QEMU_CMD+=(-vnc :0)
        echo -e "${BLUE}Display:${NC} VNC on localhost:5900"
        echo -e "${YELLOW}Connect with:${NC} vncviewer localhost:5900"
        ;;
    curses)
        QEMU_CMD+=(-display curses)
        echo -e "${BLUE}Display:${NC} Text-mode curses"
        ;;
    none)
        QEMU_CMD+=(-display none)
        echo -e "${BLUE}Display:${NC} None (serial only)"
        ;;
esac

echo -e "${BLUE}Memory:${NC}  ${MEMORY} MiB"

# Add debug options
if [ "$DEBUG_MODE" = true ]; then
    QEMU_CMD+=(-d int,cpu_reset)
    echo -e "${BLUE}Debug:${NC}   Enabled"
fi

if [ "$GDB_MODE" = true ]; then
    QEMU_CMD+=(-s -S)
    echo -e "${BLUE}GDB:${NC}     Waiting on port 1234"
fi

echo ""

# Screenshot mode
if [ "$SCREENSHOT_MODE" = true ]; then
    SCREENSHOT_PATH="$PROJECT_ROOT/target/boot_screenshot.ppm"
    echo -e "${YELLOW}Screenshot mode - capturing display after 4s...${NC}"
    
    timeout 8 qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -m "${MEMORY}M" \
        -vga std \
        -display none \
        -serial stdio \
        -monitor telnet:127.0.0.1:55555,server,nowait \
        2>&1 &
    
    QEMU_PID=$!
    sleep 4
    
    (echo "screendump $SCREENSHOT_PATH"; sleep 1) | nc localhost 55555 2>/dev/null || true
    sleep 1
    kill $QEMU_PID 2>/dev/null || true
    wait $QEMU_PID 2>/dev/null || true
    
    if [ -f "$SCREENSHOT_PATH" ]; then
        echo -e "${GREEN}✓ Screenshot saved:${NC} $SCREENSHOT_PATH"
        
        if command -v convert &> /dev/null; then
            PNG_PATH="${SCREENSHOT_PATH%.ppm}.png"
            convert "$SCREENSHOT_PATH" "$PNG_PATH" 2>/dev/null && \
            echo -e "${GREEN}✓ PNG version:${NC} $PNG_PATH"
        fi
    else
        echo -e "${RED}✗ Screenshot capture failed${NC}"
    fi
    exit 0
fi

# Run QEMU
echo -e "${YELLOW}Starting QEMU...${NC}"
echo ""
exec "${QEMU_CMD[@]}"
