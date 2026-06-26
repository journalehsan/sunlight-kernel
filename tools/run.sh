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
  -m, --memory MB    Set RAM size (default: 2048)
  --cpus N           Set virtual CPU count (default: 1)
  --cpu MODEL        Set QEMU CPU model. Use x86_64-v3 to map to Haswell-v1
  --disk PATH        Disk image to attach (default: ~/vmware/sunlight.qcow2,
                     auto-created as 10G qcow2 if missing)
  --no-disk          Don't attach a disk
  --no-net           Disable NAT networking (virtio-net + user-mode NAT)
  --no-audio         Disable audio (intel-hda)
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
MEMORY="2048"
CPU_COUNT="4"
CPU_MODEL=""
DISK_PATH="$HOME/vmware/sunlight.qcow2"
DISK_SIZE="10G"
USE_DISK=true
USE_NET=true
USE_AUDIO=true
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
        --cpus)
            CPU_COUNT="$2"
            shift 2
            ;;
        --cpu)
            CPU_MODEL="$2"
            shift 2
            ;;
        --disk)
            DISK_PATH="$2"
            shift 2
            ;;
        --no-disk)
            USE_DISK=false
            shift
            ;;
        --no-net)
            USE_NET=false
            shift
            ;;
        --no-audio)
            USE_AUDIO=false
            shift
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

if [[ "$DISPLAY_TYPE" == "gtk" || "$DISPLAY_TYPE" == "sdl" ]]; then
    if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
        echo -e "${RED}✗ Error: graphical QEMU should not be run as root${NC}"
        echo -e "${YELLOW}  The GUI needs your user display session; running via sudo loses X11/Wayland auth and XDG_RUNTIME_DIR.${NC}"
        echo -e "${YELLOW}  Run as your normal user: ./tools/run.sh${NC}"
        echo -e "${YELLOW}  For root/headless runs, use: ./tools/run.sh --vnc  or  ./tools/run.sh --no-display${NC}"
        exit 1
    fi

    if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
        echo -e "${RED}✗ Error: no graphical display session detected${NC}"
        echo -e "${YELLOW}  Set DISPLAY/WAYLAND_DISPLAY or use --vnc/--no-display.${NC}"
        exit 1
    fi

    if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
        echo -e "${RED}✗ Error: XDG_RUNTIME_DIR is not set${NC}"
        echo -e "${YELLOW}  Launch from your desktop user session or use --vnc/--no-display.${NC}"
        exit 1
    fi
fi

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — QEMU Runner${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# Rebuild if requested
if [ "$BUILD_FIRST" = true ]; then
    echo -e "${YELLOW}Updating version...${NC}"
    python3 "$SCRIPT_DIR/version_manager.py" "$PROJECT_ROOT"

    echo -e "${YELLOW}Rebuilding kernel and services...${NC}"
    SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static"
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kbd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-mouse --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-deviced --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-networkd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-resolved --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-powerd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package pty_server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package timezone_service --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package rand_service --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-niced --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-gcd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightctl --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-uac --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sm --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-solar --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kv --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tls --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-utils --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-utils --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-top --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-fetch --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-display --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package eyes --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-runner --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-terminal --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tasks --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-bench --release
    # Sunshell MUST be compiled as user-space ELF with user-space linker script
    # Force x86_64-unknown-none target (override sunshell's Linux-only config)
    # This ensures it loads into 0x400000+ (user VAs), not kernel VAs (0xffffffff8...)
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --release --features sunlight --no-default-features --target x86_64-unknown-none
    # helios-note: Linux-compat musl binary embedded by the kernel. Must be a
    # non-PIE static ET_EXEC (-no-pie + crt-static) or the loader rejects it with
    # NotStaticExecutable; then stamp EI_OSABI (byte 7) to ELFOSABI_LINUX (3) so
    # is_linux_elf() routes it through the Helios compat layer. Mirrors build.sh.
    RUSTFLAGS="-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie" cargo build --package helios-note --release --target x86_64-unknown-linux-musl
    printf '\x03' | dd of="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release/helios-note" \
        bs=1 seek=7 conv=notrunc 2>/dev/null
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

case "$CPU_MODEL" in
    x86_64-v3|x86-64-v3)
        # QEMU does not expose an x86_64-v3 CPU name directly; Haswell-v1 is a
        # portable v3-class model that provides AVX2/FMA/BMI* under both TCG and KVM.
        CPU_MODEL="Haswell-v1"
        CPU_MODEL_LABEL="Haswell-v1 (x86-64-v3 class)"
        ;;
    "")
        CPU_MODEL_LABEL="QEMU default"
        ;;
    *)
        CPU_MODEL_LABEL="$CPU_MODEL"
        ;;
esac

CPU_ARGS=()
if [ -n "$CPU_MODEL" ]; then
    CPU_ARGS=(-cpu "$CPU_MODEL")
fi

# Create disk image if missing (qcow2 — QEMU's native format)
if [ "$USE_DISK" = true ] && [ ! -f "$DISK_PATH" ]; then
    echo -e "${YELLOW}Disk image not found, creating ${DISK_SIZE} qcow2 at $DISK_PATH${NC}"
    mkdir -p "$(dirname "$DISK_PATH")"
    qemu-img create -f qcow2 "$DISK_PATH" "$DISK_SIZE"
fi

# Build QEMU command
QEMU_CMD=(
    qemu-system-x86_64
    -cdrom "$ISO_PATH"
    -m "${MEMORY}M"
    -smp "$CPU_COUNT"
    "${CPU_ARGS[@]}"
    -vga std
    -serial stdio
    -no-reboot
)

# Attach disk (virtio-blk, legacy mode — matches the kernel's virtio driver)
if [ "$USE_DISK" = true ]; then
    QEMU_CMD+=(
        -drive "id=hd0,file=$DISK_PATH,if=none,format=qcow2"
        -device "virtio-blk-pci,disable-modern=on,drive=hd0"
    )
    echo -e "${BLUE}Disk:${NC}    $DISK_PATH"
fi

# NAT networking (virtio-net + user-mode NAT)
#   host 2222 -> guest 22 (SSH)
#   host 8080 -> guest 80 (Solar HTTP) — browse http://localhost:8080
if [ "$USE_NET" = true ]; then
    QEMU_CMD+=(
        -netdev "user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8080-:80"
        -device "virtio-net-pci,disable-modern=on,netdev=net0"
    )
    echo -e "${BLUE}Network:${NC} NAT (host 2222 -> guest 22, host 8080 -> guest 80)"
fi

# Audio (intel-hda; pa works with both PulseAudio and PipeWire)
if [ "$USE_AUDIO" = true ]; then
    QEMU_CMD+=(
        -audiodev "pa,id=snd0"
        -device intel-hda
        -device "hda-output,audiodev=snd0"
    )
    echo -e "${BLUE}Audio:${NC}   intel-hda"
fi

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
echo -e "${BLUE}vCPUs:${NC}   ${CPU_COUNT}"
echo -e "${BLUE}CPU:${NC}     ${CPU_MODEL_LABEL}"

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
        -smp "$CPU_COUNT" \
        "${CPU_ARGS[@]}" \
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
