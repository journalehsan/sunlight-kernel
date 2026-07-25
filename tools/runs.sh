#!/usr/bin/env bash
# Unified script to run SunlightOS in QEMU with various display options.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"

# shellcheck source=tools/vm_display_policy.sh
source "$SCRIPT_DIR/vm_display_policy.sh"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

show_usage() {
    cat <<USAGE
${GREEN}SunlightOS Runner${NC}

Usage: $0 [OPTIONS]

Build Options:
  -b, --build           Rebuild kernel + services before launching
                        (produces a hybrid BIOS+UEFI ISO)

Firmware (QEMU only):
  (default)             Legacy BIOS / SeaBIOS
  --uefi                Boot with OVMF (x86_64 UEFI). Paths via
                        SUNLIGHT_OVMF_CODE / SUNLIGHT_OVMF_VARS or common
                        distro locations; writable VARS at target/ovmf_vars.fd

Hypervisor:
  (default)             QEMU/KVM (all options below apply)
  --vmware              Start existing VMware VM (no ISO needed; uses vmrun)
                        Default VM path: ~/vmware/SunlightOS/SunlightOS.vmx
                        Override: --vmware-vm /path/to/vm.vmx
                            or set SUNLIGHT_VMWARE_VM env var
  --vmware-vm PATH      Path to .vmx file (implies --vmware)
  --vbox                Boot ISO in VirtualBox (creates/reuses VM 'SunlightOS')
                        Override VM name: --vbox-vm NAME

Display Options (QEMU only):
  --display MODE        Select display mode: gtk, sdl, vnc, curses, none
  -g, --gui             Launch with GTK window (default, requires X11/Wayland)
  -s, --sdl             Launch with SDL window
  -v, --vnc             Launch with VNC server on :0 (port 5900)
  -c, --curses          Launch with text-mode curses interface
  -n, --no-display      Launch without display (serial only)
  --screenshot          Capture screenshot and exit
  --dual-gpu            Use VGA std + explicit virtio-gpu-pci, like the old
                        hardware-cursor runner with a separate GPU output/tab
  --limine-only         Use only standard VGA; no VirtIO GPU is presented, so
                        the display backend must remain the Limine framebuffer
  --resolution WxH      Request an explicit QEMU desktop resolution. When unset,
                        SunlightOS prefers 1366x768, then 1280x800, 1280x720,
                        then 1024x768 if the QEMU video device exposes xres/yres

QEMU Options:
  -m, --memory MB       Set RAM size (default: 2048)
  --cpus N              Set virtual CPU count (default: 4)
  --cpu MODEL           Set QEMU CPU model. Use x86_64-v3 to map to Haswell-v1
  --disk PATH           Disk image to attach (default: ~/vmware/sunlight.qcow2,
                        auto-created as 10G qcow2 if missing)
  --no-disk             Don't attach a disk
  --no-net              Disable NAT networking (virtio-net + user-mode NAT)
  --no-audio            Disable audio (intel-hda)
  --debug               Enable QEMU debug output
  --gdb                 Wait for GDB connection on port 1234

Other:
  -h, --help            Show this help message

Examples:
  $0 --build
  $0 --build --uefi --no-display
  $0 --uefi --sdl
  $0 --vmware
  $0 --vmware --build
  $0 --vmware-vm ~/vms/SunlightOS.vmx
  $0 --vbox
  $0 --vbox --build
  $0 --display vnc
  $0 --dual-gpu --gui
  $0 --screenshot

USAGE
}

DISPLAY_TYPE="gtk"
MEMORY="4096"
CPU_COUNT="12"
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
DUAL_GPU_MODE=false
LIMINE_ONLY_MODE=false
QEMU_RESOLUTION_OVERRIDE="${SUNLIGHT_QEMU_RESOLUTION:-}"
QEMU_RESOLUTION_REQUESTED=false
if [[ -n "$QEMU_RESOLUTION_OVERRIDE" ]]; then
    QEMU_RESOLUTION_REQUESTED=true
fi
VMWARE_MODE=false
VMWARE_VM_PATH="${SUNLIGHT_VMWARE_VM:-$HOME/vmware/SunlightOS/SunlightOS.vmx}"
VBOX_MODE=false
VBOX_VM_NAME="SunlightOS"
UEFI_MODE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        -b|--build)
            BUILD_FIRST=true
            shift
            ;;
        --uefi)
            UEFI_MODE=true
            shift
            ;;
        --vmware)
            VMWARE_MODE=true
            shift
            ;;
        --vmware-vm)
            VMWARE_MODE=true
            VMWARE_VM_PATH="$2"
            shift 2
            ;;
        --vbox)
            VBOX_MODE=true
            shift
            ;;
        --vbox-vm)
            VBOX_MODE=true
            VBOX_VM_NAME="$2"
            shift 2
            ;;
        --display)
            case "${2:-}" in
                gtk|sdl|vnc|curses|none)
                    DISPLAY_TYPE="$2"
                    ;;
                *)
                    echo -e "${RED}Error: invalid display mode '${2:-}'${NC}"
                    show_usage
                    exit 1
                    ;;
            esac
            shift 2
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
        --dual-gpu)
            DUAL_GPU_MODE=true
            shift
            ;;
        --limine-only)
            LIMINE_ONLY_MODE=true
            shift
            ;;
        --resolution)
            QEMU_RESOLUTION_OVERRIDE="$2"
            QEMU_RESOLUTION_REQUESTED=true
            shift 2
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
        echo -e "${YELLOW}  Run as your normal user: ./tools/runs.sh${NC}"
        echo -e "${YELLOW}  For root/headless runs, use: ./tools/runs.sh --vnc  or  ./tools/runs.sh --no-display${NC}"
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

if { [ "$VMWARE_MODE" = true ] || [ "$VBOX_MODE" = true ]; } \
    && [ "$QEMU_RESOLUTION_REQUESTED" = true ]; then
    echo -e "${YELLOW}Warning:${NC} --resolution / SUNLIGHT_QEMU_RESOLUTION is QEMU-only and will be ignored for this hypervisor"
fi

echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  SunlightOS — QEMU Runner${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ "$BUILD_FIRST" = true ]; then
    echo -e "${YELLOW}Updating version...${NC}"
    python3 "$SCRIPT_DIR/version_manager.py" "$PROJECT_ROOT"

    if [[ -n "${PPC8086:-}" && -n "${FPC_I8086_RTL:-}" ]]; then
        echo -e "${YELLOW}Rebuilding Chronos DOS shell guest...${NC}"
        PPC8086="$PPC8086" FPC_I8086_RTL="$FPC_I8086_RTL" \
            "$PROJECT_ROOT/guest/sunshell/build.sh"
    else
        echo -e "${YELLOW}Chronos DOS shell:${NC} using checked-in SUNSH.EXE (set PPC8086 and FPC_I8086_RTL to rebuild)"
    fi

    echo -e "${YELLOW}Rebuilding kernel and services...${NC}"
    SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C no-redzone"
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-swapd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kbd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-mouse --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-usb-mouse --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-deviced --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-networkd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-resolved --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-powerd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-thermald --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package pty_server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package timezone_service --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timed --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tz --features tzutils --bin tzutils --release
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
    # Session lock policy (init-launched) + recovery CLI. Must use SERVICE_RUSTFLAGS
    # (userspace linker); a kernel-linked mezzo fails spawn with SegmentOutOfRange
    # and blocks desktop session establish after login.
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package mezzo --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package mezzoctl --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package eyes --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-runner --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sun-exec --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sun-open --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-terminal --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-chronos --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tasks --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vortex-shell --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-bench --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calculator --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-widget-gallery --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-silicon-echoes --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-files --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-light-lens --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-edit --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-writer --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calendar --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-reminders --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-devices --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package rappid-rabbit --features dom --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-api-lab --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-dialogd --release
    echo -e "${YELLOW}Building Control Panel (includes generated monochrome PNG icon assets)...${NC}"
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-control-panel --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-thumbd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-clipd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-clipman --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-emoji-picker --release
    # sunshell (includes localectl builtin + pulls in support libs e.g. sunlight-locale, sunlight-tz)
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --release --features sunlight --no-default-features --target x86_64-unknown-none
    RUSTFLAGS="-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie" cargo build --package helios-note --release --target x86_64-unknown-linux-musl
    printf '\x03' | dd of="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release/helios-note" \
        bs=1 seek=7 conv=notrunc 2>/dev/null
    # The kernel embeds the freshly built service ELFs with include_bytes!.
    # Force rustc to refresh those bytes even when Cargo misses artifact mtimes.
    touch "$PROJECT_ROOT/kernel/src/main.rs"
    cargo build --package sunlight-kernel

    KERNEL_ELF="$PROJECT_ROOT/target/x86_64-unknown-none/debug/sunlight-kernel"
    LIMINE_DIR="$PROJECT_ROOT/target/limine"
    echo -e "${YELLOW}Building hybrid ISO (BIOS + UEFI)...${NC}"
    LIMINE_BRANCH=v8.x "$SCRIPT_DIR/make_hybrid_iso.sh" \
        "$KERNEL_ELF" "$ISO_PATH" "$LIMINE_DIR" "$PROJECT_ROOT"
    echo -e "${GREEN}✓${NC} Build complete (hybrid BIOS+UEFI ISO)"
fi

if [ ! -f "$ISO_PATH" ]; then
    echo -e "${RED}✗ Error: ISO not found${NC}"
    echo -e "${YELLOW}  Run './tools/build.sh' first${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} ISO: $ISO_PATH"

if ! command -v qemu-system-x86_64 &>/dev/null; then
    echo -e "${RED}✗ Error: qemu-system-x86_64 not found${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} QEMU: $(qemu-system-x86_64 --version | head -1)"
echo ""

case "$CPU_MODEL" in
    x86_64-v3|x86-64-v3)
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

if [ "$USE_DISK" = true ] && [ ! -f "$DISK_PATH" ]; then
    echo -e "${YELLOW}Disk image not found, creating ${DISK_SIZE} qcow2 at $DISK_PATH${NC}"
    mkdir -p "$(dirname "$DISK_PATH")"
    qemu-img create -f qcow2 "$DISK_PATH" "$DISK_SIZE"
fi

VIDEO_DEVICE="virtio-vga"
if [ "$DUAL_GPU_MODE" = true ]; then
    VIDEO_DEVICE="virtio-gpu-pci"
elif [ "$LIMINE_ONLY_MODE" = true ]; then
    VIDEO_DEVICE="std"
fi

QEMU_RESOLUTION=""
QEMU_RESOLUTION_SOURCE="backend-default"
if sunlight_qemu_device_supports_resolution "$VIDEO_DEVICE"; then
    if [[ -n "$QEMU_RESOLUTION_OVERRIDE" ]]; then
        if sunlight_parse_resolution "$QEMU_RESOLUTION_OVERRIDE" >/dev/null; then
            QEMU_RESOLUTION="$QEMU_RESOLUTION_OVERRIDE"
            QEMU_RESOLUTION_SOURCE="override"
        else
            echo -e "${YELLOW}Warning:${NC} ignoring invalid resolution '$QEMU_RESOLUTION_OVERRIDE'"
        fi
    fi

    if [[ -z "$QEMU_RESOLUTION" ]]; then
        QEMU_RESOLUTION="$(sunlight_resolve_vm_display_mode \
            "" \
            "qemu" \
            "" \
            "${SUNLIGHT_VM_PREFERRED_MODES[@]}")"
        if [[ -n "$QEMU_RESOLUTION" ]]; then
            QEMU_RESOLUTION_SOURCE="vm-policy"
        fi
    fi
elif [ "$QEMU_RESOLUTION_REQUESTED" = true ]; then
    echo -e "${YELLOW}Warning:${NC} selected QEMU video device '$VIDEO_DEVICE' does not expose xres/yres; keeping backend default resolution"
fi

QEMU_RESOLUTION_X=""
QEMU_RESOLUTION_Y=""
if [[ -n "$QEMU_RESOLUTION" ]]; then
    read -r QEMU_RESOLUTION_X QEMU_RESOLUTION_Y \
        <<<"$(sunlight_parse_resolution "$QEMU_RESOLUTION")"
fi

resolve_ovmf_firmware() {
    # Prefer explicit env overrides, then common distro layouts.
    local code_candidates=(
        "${SUNLIGHT_OVMF_CODE:-}"
        /usr/share/OVMF/OVMF_CODE.fd
        /usr/share/OVMF/OVMF_CODE_4M.fd
        /usr/share/edk2/x64/OVMF_CODE.4m.fd
        /usr/share/edk2-ovmf/x64/OVMF_CODE.fd
        /usr/share/edk2-ovmf/OVMF_CODE.fd
        /usr/share/qemu/OVMF_CODE.fd
    )
    local vars_candidates=(
        "${SUNLIGHT_OVMF_VARS:-}"
        /usr/share/OVMF/OVMF_VARS.fd
        /usr/share/OVMF/OVMF_VARS_4M.fd
        /usr/share/edk2/x64/OVMF_VARS.4m.fd
        /usr/share/edk2-ovmf/x64/OVMF_VARS.fd
        /usr/share/edk2-ovmf/OVMF_VARS.fd
        /usr/share/qemu/OVMF_VARS.fd
    )
    local c v
    OVMF_CODE=""
    OVMF_VARS=""
    for c in "${code_candidates[@]}"; do
        [[ -n "$c" && -f "$c" ]] && OVMF_CODE="$c" && break
    done
    for v in "${vars_candidates[@]}"; do
        [[ -n "$v" && -f "$v" ]] && OVMF_VARS="$v" && break
    done
    if [[ -z "$OVMF_CODE" || -z "$OVMF_VARS" ]]; then
        echo -e "${RED}✗ Error: OVMF firmware not found for --uefi${NC}"
        echo -e "${YELLOW}  Install OVMF/edk2 firmware, or set:${NC}"
        echo -e "${YELLOW}    SUNLIGHT_OVMF_CODE=/path/to/OVMF_CODE.fd${NC}"
        echo -e "${YELLOW}    SUNLIGHT_OVMF_VARS=/path/to/OVMF_VARS.fd${NC}"
        echo -e "${YELLOW}  Searched under /usr/share/OVMF, /usr/share/edk2, /usr/share/edk2-ovmf, /usr/share/qemu${NC}"
        exit 1
    fi
}

if { [ "$VMWARE_MODE" = true ] || [ "$VBOX_MODE" = true ]; } && [ "$UEFI_MODE" = true ]; then
    echo -e "${YELLOW}Warning:${NC} --uefi is QEMU-only and is ignored for VMware/VirtualBox"
    UEFI_MODE=false
fi

QEMU_CMD=(
    qemu-system-x86_64
    -cdrom "$ISO_PATH"
    -m "${MEMORY}M"
    -smp "$CPU_COUNT"
    "${CPU_ARGS[@]}"
    -serial stdio
    -no-reboot
    -device virtio-rng-pci,disable-modern=on
    -device qemu-xhci,id=xhci
    -device usb-mouse,bus=xhci.0
)

if [ "$UEFI_MODE" = true ]; then
    resolve_ovmf_firmware
    OVMF_VARS_WORK="$PROJECT_ROOT/target/ovmf_vars.fd"
    # Writable private VARS store; never write the system OVMF VARS image.
    if [[ ! -f "$OVMF_VARS_WORK" ]] || [[ "$OVMF_VARS" -nt "$OVMF_VARS_WORK" ]]; then
        cp "$OVMF_VARS" "$OVMF_VARS_WORK"
    fi
    QEMU_CMD+=(
        -drive "if=pflash,format=raw,readonly=on,file=$OVMF_CODE"
        -drive "if=pflash,format=raw,file=$OVMF_VARS_WORK"
    )
    echo -e "${BLUE}Firmware:${NC} UEFI (OVMF)"
    echo -e "${BLUE}  CODE:${NC}  $OVMF_CODE"
    echo -e "${BLUE}  VARS:${NC}  $OVMF_VARS_WORK (writable copy of $OVMF_VARS)"
else
    echo -e "${BLUE}Firmware:${NC} Legacy BIOS (SeaBIOS)"
fi

if [ "$LIMINE_ONLY_MODE" = true ]; then
    QEMU_CMD+=(-vga std)
elif [ "$DUAL_GPU_MODE" = true ]; then
    if [[ -n "$QEMU_RESOLUTION" ]]; then
        QEMU_CMD+=(
            -vga std
            -device "virtio-gpu-pci,xres=${QEMU_RESOLUTION_X},yres=${QEMU_RESOLUTION_Y}"
        )
    else
        QEMU_CMD+=(
            -vga std
            -device virtio-gpu-pci
        )
    fi
else
    if [[ -n "$QEMU_RESOLUTION" ]]; then
        QEMU_CMD+=(
            -vga none
            -device "virtio-vga,xres=${QEMU_RESOLUTION_X},yres=${QEMU_RESOLUTION_Y}"
        )
    else
        QEMU_CMD+=(-vga virtio)
    fi
fi

if [ "$USE_DISK" = true ]; then
    QEMU_CMD+=(
        -drive "id=hd0,file=$DISK_PATH,if=none,format=qcow2"
        -device "virtio-blk-pci,disable-modern=on,drive=hd0"
    )
    echo -e "${BLUE}Disk:${NC}    $DISK_PATH"
fi

if [ "$USE_NET" = true ]; then
    QEMU_CMD+=(
        -netdev "user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8080-:80"
        -device "virtio-net-pci,disable-modern=on,netdev=net0"
    )
    echo -e "${BLUE}Network:${NC} NAT (host 2222 -> guest 22, host 8080 -> guest 80)"
fi

if [ "$USE_AUDIO" = true ]; then
    QEMU_CMD+=(
        -audiodev "pa,id=snd0"
        -device intel-hda
        -device "hda-output,audiodev=snd0"
    )
    echo -e "${BLUE}Audio:${NC}   intel-hda"
fi

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
if [ "$LIMINE_ONLY_MODE" = true ]; then
    echo -e "${BLUE}Video:${NC}   standard VGA (Limine-only backend)"
elif [ "$DUAL_GPU_MODE" = true ]; then
    echo -e "${BLUE}Video:${NC}   VGA std + virtio-gpu-pci"
    echo -e "${YELLOW}Note:${NC}    Switch to the virtio-gpu tab/output in QEMU for the desktop view"
else
    echo -e "${BLUE}Video:${NC}   virtio VGA"
fi
if [[ -n "$QEMU_RESOLUTION" ]]; then
    echo -e "${BLUE}Resolution:${NC} ${QEMU_RESOLUTION} (${QEMU_RESOLUTION_SOURCE})"
else
    echo -e "${BLUE}Resolution:${NC} backend default"
fi

if [ "$DEBUG_MODE" = true ]; then
    QEMU_CMD+=(-d int,cpu_reset)
    echo -e "${BLUE}Debug:${NC}   Enabled"
fi

if [ "$GDB_MODE" = true ]; then
    QEMU_CMD+=(-s -S)
    echo -e "${BLUE}GDB:${NC}     Waiting on port 1234"
fi

echo ""

if [ "$SCREENSHOT_MODE" = true ]; then
    SCREENSHOT_PATH="$PROJECT_ROOT/target/boot_screenshot.ppm"
    echo -e "${YELLOW}Screenshot mode - capturing display after 4s...${NC}"

    timeout 8 qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -m "${MEMORY}M" \
        -smp "$CPU_COUNT" \
        "${CPU_ARGS[@]}" \
        -vga vmvga \
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

        if command -v convert &>/dev/null; then
            PNG_PATH="${SCREENSHOT_PATH%.ppm}.png"
            convert "$SCREENSHOT_PATH" "$PNG_PATH" 2>/dev/null && \
                echo -e "${GREEN}✓ PNG version:${NC} $PNG_PATH"
        fi
    else
        echo -e "${RED}✗ Screenshot capture failed${NC}"
    fi
    exit 0
fi

# ── VMware ────────────────────────────────────────────────────────────────────
if [ "$VMWARE_MODE" = true ]; then
    if ! command -v vmrun &>/dev/null; then
        echo -e "${RED}✗ vmrun not found${NC}"
        echo -e "${YELLOW}  Install VMware Workstation or Fusion and ensure vmrun is on PATH.${NC}"
        exit 1
    fi
    if [ ! -f "$VMWARE_VM_PATH" ]; then
        echo -e "${RED}✗ VMware VM not found: $VMWARE_VM_PATH${NC}"
        echo -e "${YELLOW}  Set SUNLIGHT_VMWARE_VM, use --vmware-vm /path/to/vm.vmx,${NC}"
        echo -e "${YELLOW}  or place your VM at the default path above.${NC}"
        exit 1
    fi
    if ! rg -iq '^[[:space:]]*rtc\.diffFromUTC[[:space:]]*=[[:space:]]*"?0"?[[:space:]]*$' "$VMWARE_VM_PATH"; then
        echo -e "${RED}✗ VMware RTC is not explicitly configured as UTC${NC}"
        echo -e "${YELLOW}  Power off the VM and add this to its .vmx file:${NC}"
        echo -e "${YELLOW}  rtc.diffFromUTC = \"0\"${NC}"
        echo -e "${YELLOW}  SunlightOS keeps kernel wall time in UTC and applies timezone once.${NC}"
        exit 1
    fi
    echo -e "${BLUE}Hypervisor:${NC} VMware"
    echo -e "${BLUE}VM:${NC}         $VMWARE_VM_PATH"
    echo ""
    echo -e "${YELLOW}Starting VMware VM...${NC}"
    exec vmrun start "$VMWARE_VM_PATH" gui
fi

# ── VirtualBox ─────────────────────────────────────────────────────────────────
if [ "$VBOX_MODE" = true ]; then
    if ! command -v VBoxManage &>/dev/null; then
        echo -e "${RED}✗ VBoxManage not found${NC}"
        echo -e "${YELLOW}  Install VirtualBox and ensure VBoxManage is on PATH.${NC}"
        exit 1
    fi
    echo -e "${BLUE}Hypervisor:${NC} VirtualBox"
    echo -e "${BLUE}VM name:${NC}    $VBOX_VM_NAME"
    echo -e "${BLUE}ISO:${NC}        $ISO_PATH"
    echo ""

    if VBoxManage showvminfo "$VBOX_VM_NAME" &>/dev/null 2>&1; then
        echo -e "${YELLOW}Updating ISO on existing VM '$VBOX_VM_NAME'...${NC}"
        # Detach old medium (ignore errors if none attached)
        VBoxManage storageattach "$VBOX_VM_NAME" \
            --storagectl "IDE Controller" --port 0 --device 0 \
            --type dvddrive --medium emptydrive 2>/dev/null || true
        VBoxManage storageattach "$VBOX_VM_NAME" \
            --storagectl "IDE Controller" --port 0 --device 0 \
            --type dvddrive --medium "$ISO_PATH"
    else
        echo -e "${YELLOW}Creating VirtualBox VM '$VBOX_VM_NAME'...${NC}"
        VBoxManage createvm --name "$VBOX_VM_NAME" --register --ostype Linux_64
        VBoxManage modifyvm "$VBOX_VM_NAME" \
            --memory "$MEMORY" \
            --cpus "$CPU_COUNT" \
            --vram 128 \
            --graphicscontroller vmsvga \
            --audio-driver null \
            --boot1 dvd --boot2 disk --boot3 none
        VBoxManage storagectl "$VBOX_VM_NAME" --name "IDE Controller" --add ide
        VBoxManage storageattach "$VBOX_VM_NAME" \
            --storagectl "IDE Controller" --port 0 --device 0 \
            --type dvddrive --medium "$ISO_PATH"
        echo -e "${GREEN}✓${NC} VM created"
    fi

    echo -e "${YELLOW}Starting VirtualBox VM...${NC}"
    exec VBoxManage startvm "$VBOX_VM_NAME" --type gui
fi

# ── QEMU ───────────────────────────────────────────────────────────────────────
echo -e "${YELLOW}Starting QEMU...${NC}"
echo ""
exec "${QEMU_CMD[@]}"
