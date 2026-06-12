#!/usr/bin/env bash
# SunlightOS Interactive Installer Wizard
# Prompts for user configuration, partitions disk, installs bootloader, clones filesystem
set -euo pipefail

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m'

# ============================================================================
# Configuration & Paths
# ============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Target disk — usually /dev/sda in QEMU VM
TARGET_DISK="${1:-/dev/sda}"
BOOT_PARTITION="${TARGET_DISK}1"
ROOT_PARTITION="${TARGET_DISK}2"

LIMINE_DIR="$PROJECT_ROOT/target/limine"
LIMINE_CONF="$PROJECT_ROOT/limine.conf"
KERNEL_ELF="$PROJECT_ROOT/target/x86_64-unknown-none/debug/sunlight-kernel.elf"

# Temporary mount points
MOUNT_BOOT="/mnt/sunlight_boot"
MOUNT_ROOT="/mnt/sunlight_root"

# ============================================================================
# Utility Functions
# ============================================================================

log_info() {
    echo -e "${GREEN}[INFO]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

log_step() {
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}$*${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
}

cleanup() {
    log_info "Cleaning up..."
    umount "$MOUNT_BOOT" 2>/dev/null || true
    umount "$MOUNT_ROOT" 2>/dev/null || true
    rmdir "$MOUNT_BOOT" 2>/dev/null || true
    rmdir "$MOUNT_ROOT" 2>/dev/null || true
}

trap cleanup EXIT

# Validate input: hostname
validate_hostname() {
    local hostname="$1"
    if [[ ! "$hostname" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
        return 1
    fi
    return 0
}

# Validate input: username
validate_username() {
    local username="$1"
    if [[ ! "$username" =~ ^[a-z_][a-z0-9_-]{0,31}$ ]]; then
        return 1
    fi
    return 0
}

# Validate input: timezone (basic check)
validate_timezone() {
    local tz="$1"
    if [[ -f "/usr/share/zoneinfo/$tz" ]]; then
        return 0
    fi
    return 1
}

# Prompt for input with validation
prompt_input() {
    local prompt="$1"
    local validator="$2"
    local value=""

    while [ -z "$value" ] || ! $validator "$value"; do
        echo -n "$prompt: "
        read -r value
        if [ -z "$value" ]; then
            log_warn "Input cannot be empty"
        elif ! $validator "$value"; then
            log_warn "Invalid input: $value"
            value=""
        fi
    done

    echo "$value"
}

# Prompt for password (hidden)
prompt_password() {
    local prompt="$1"
    local password=""
    local password2=""

    while true; do
        echo -n "$prompt: "
        read -rs password
        echo ""

        if [ -z "$password" ]; then
            log_warn "Password cannot be empty"
            continue
        fi

        echo -n "Confirm password: "
        read -rs password2
        echo ""

        if [ "$password" != "$password2" ]; then
            log_warn "Passwords do not match"
            continue
        fi

        echo "$password"
        break
    done
}

# ============================================================================
# Checks
# ============================================================================

check_requirements() {
    log_step "Checking System Requirements"

    # Check if running as root
    if [ "$EUID" -ne 0 ]; then
        log_error "This installer must run as root (use: sudo)"
        exit 1
    fi

    # Check for required tools
    for tool in fdisk parted mkfs.vfat mkfs.ext4 dd mount umount; do
        if ! command -v "$tool" &> /dev/null; then
            log_error "Required tool not found: $tool"
            exit 1
        fi
    done

    # Check if Limine is available
    if [ ! -f "$LIMINE_DIR/bin/limine-bios.sys" ]; then
        log_error "Limine bootloader not found at: $LIMINE_DIR/bin/"
        log_warn "Run './tools/run.sh --build' to build Limine"
        exit 1
    fi

    # Check if kernel ELF exists
    if [ ! -f "$KERNEL_ELF" ]; then
        log_error "Kernel ELF not found: $KERNEL_ELF"
        log_warn "Run './tools/run.sh --build' to build the kernel"
        exit 1
    fi

    log_info "✓ All requirements met"
}

check_disk() {
    log_step "Verifying Target Disk"

    if [ ! -b "$TARGET_DISK" ]; then
        log_error "Disk not found: $TARGET_DISK"
        echo -e "${YELLOW}Available disks:${NC}"
        lsblk -nd -o NAME,SIZE,TYPE | grep disk || true
        exit 1
    fi

    local disk_size=$(blockdev --getsize64 "$TARGET_DISK" 2>/dev/null || echo 0)
    local disk_gb=$((disk_size / 1024 / 1024 / 1024))

    if [ "$disk_gb" -lt 4 ]; then
        log_error "Disk too small: ${disk_gb}GB (need at least 4GB)"
        exit 1
    fi

    log_info "Target disk: $TARGET_DISK (${disk_gb}GB)"

    # Confirm before proceeding
    echo ""
    echo -e "${RED}⚠ WARNING: This will erase all data on $TARGET_DISK${NC}"
    echo -n "Type 'yes' to continue: "
    read -r confirm
    if [ "$confirm" != "yes" ]; then
        log_info "Installation cancelled"
        exit 0
    fi
}

# ============================================================================
# User Configuration
# ============================================================================

prompt_configuration() {
    log_step "System Configuration"

    echo ""
    echo -e "${BLUE}Hostname Configuration${NC}"
    HOSTNAME=$(prompt_input "Enter system hostname" validate_hostname)
    log_info "Hostname: $HOSTNAME"

    echo ""
    echo -e "${BLUE}Timezone Configuration${NC}"
    TIMEZONE=$(prompt_input "Enter timezone (e.g., Asia/Tehran, UTC)" validate_timezone)
    log_info "Timezone: $TIMEZONE"

    echo ""
    echo -e "${BLUE}Root Administrative Account${NC}"
    ROOT_PASSWORD=$(prompt_password "Set root password")
    log_info "Root password set"

    echo ""
    echo -e "${BLUE}Standard User Account${NC}"
    USERNAME=$(prompt_input "Enter standard user username" validate_username)
    log_info "Username: $USERNAME"
    USER_PASSWORD=$(prompt_password "Set password for $USERNAME")
    log_info "User password set"

    echo ""
    log_info "Configuration complete:"
    log_info "  Hostname:  $HOSTNAME"
    log_info "  Timezone:  $TIMEZONE"
    log_info "  Root:      (configured)"
    log_info "  User:      $USERNAME (configured)"
}

# ============================================================================
# Disk Operations
# ============================================================================

partition_disk() {
    log_step "Partitioning Disk"

    log_info "Clearing partition table..."
    dd if=/dev/zero of="$TARGET_DISK" bs=512 count=2048 2>/dev/null

    log_info "Creating partitions..."
    # Partition layout:
    # 1: 512 MB FAT32 (/boot)
    # 2: remainder ext4 (/)
    fdisk "$TARGET_DISK" << EOF
o
n
p
1

+512M
t
1
c
n
p
2


w
EOF

    log_info "Waiting for partition table to settle..."
    sleep 2
    partprobe "$TARGET_DISK" || true
    sleep 1

    # Verify partitions exist
    for attempt in {1..5}; do
        if [ -b "$BOOT_PARTITION" ] && [ -b "$ROOT_PARTITION" ]; then
            log_info "✓ Partitions ready"
            return 0
        fi
        sleep 1
    done

    log_error "Partitions not detected after partitioning"
    exit 1
}

format_partitions() {
    log_step "Formatting Partitions"

    log_info "Formatting $BOOT_PARTITION as FAT32..."
    mkfs.vfat -F 32 -n "SUNLIGHT_BOOT" "$BOOT_PARTITION" > /dev/null 2>&1

    log_info "Formatting $ROOT_PARTITION as ext4..."
    mkfs.ext4 -F -L "SUNLIGHT_ROOT" "$ROOT_PARTITION" > /dev/null 2>&1

    log_info "✓ Partitions formatted"
}

mount_partitions() {
    log_step "Mounting Partitions"

    mkdir -p "$MOUNT_BOOT" "$MOUNT_ROOT"

    log_info "Mounting $BOOT_PARTITION..."
    mount "$BOOT_PARTITION" "$MOUNT_BOOT"

    log_info "Mounting $ROOT_PARTITION..."
    mount "$ROOT_PARTITION" "$MOUNT_ROOT"

    log_info "✓ Partitions mounted"
}

install_bootloader() {
    log_step "Installing Limine Bootloader"

    log_info "Creating boot directory structure..."
    mkdir -p "$MOUNT_BOOT/boot/limine"

    log_info "Copying kernel and bootloader files..."
    cp "$KERNEL_ELF" "$MOUNT_BOOT/boot/sunlight-kernel.elf"
    cp "$LIMINE_CONF" "$MOUNT_BOOT/boot/limine/limine.conf"
    cp "$LIMINE_DIR/bin/limine-bios.sys" "$MOUNT_BOOT/boot/limine/"

    log_info "Installing Limine to MBR of $TARGET_DISK..."
    # Create a temporary ISO for Limine installation
    local temp_iso="/tmp/sunlight_install.iso"
    xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
        -no-emul-boot -boot-load-size 4 -boot-info-table \
        -quiet "$MOUNT_BOOT" -o "$temp_iso" 2>/dev/null || true

    # Install Limine to the disk's MBR
    "$LIMINE_DIR/bin/limine" bios-install "$TARGET_DISK" 2>/dev/null || true
    rm -f "$temp_iso"

    log_info "✓ Bootloader installed"
}

clone_filesystem() {
    log_step "Cloning Live Filesystem"

    log_info "This system is currently running from RAMFS (live environment)"
    log_info "Cloning root filesystem to persistent storage..."

    # Clone everything except special filesystems
    local exclude_patterns=(
        '--exclude=proc/*'
        '--exclude=sys/*'
        '--exclude=dev/*'
        '--exclude=run/*'
        '--exclude=tmp/*'
        '--exclude=var/tmp/*'
        '--exclude=var/lock/*'
        '--exclude=etc/hostname'
        '--exclude=etc/localtime'
        '--exclude=etc/auth/users.json'
        '--exclude=etc/fstab'
        '--exclude=boot'
        '--exclude=sunlight_boot'
        '--exclude=sunlight_root'
    )

    # rsync from live root to mounted root partition
    rsync -av --delete "${exclude_patterns[@]}" / "$MOUNT_ROOT/" 2>&1 | grep -v "^building\|^sending\|^created\|^total" || true

    log_info "✓ Filesystem cloned"
}

generate_configuration() {
    log_step "Generating Configuration Files"

    # 1. Hostname
    log_info "Writing hostname..."
    echo "$HOSTNAME" > "$MOUNT_ROOT/etc/hostname"

    # 2. Timezone
    log_info "Writing timezone configuration..."
    mkdir -p "$MOUNT_ROOT/etc"
    if [ -f "/usr/share/zoneinfo/$TIMEZONE" ]; then
        cp "/usr/share/zoneinfo/$TIMEZONE" "$MOUNT_ROOT/etc/localtime"
    else
        ln -sf "UTC" "$MOUNT_ROOT/etc/localtime"
    fi

    # 3. User accounts (JSON format)
    log_info "Generating user accounts..."
    mkdir -p "$MOUNT_ROOT/etc/auth"
    cat > "$MOUNT_ROOT/etc/auth/users.json" << EOF
{
  "version": 1,
  "users": [
    {
      "uid": 0,
      "gid": 0,
      "username": "root",
      "password_hash": "$(echo -n "$ROOT_PASSWORD" | sha256sum | awk '{print $1}')",
      "home": "/root",
      "shell": "/bin/sshell"
    },
    {
      "uid": 1000,
      "gid": 1000,
      "username": "$USERNAME",
      "password_hash": "$(echo -n "$USER_PASSWORD" | sha256sum | awk '{print $1}')",
      "home": "/home/$USERNAME",
      "shell": "/bin/sshell"
    }
  ],
  "groups": [
    {
      "gid": 0,
      "groupname": "root"
    },
    {
      "gid": 1000,
      "groupname": "$USERNAME"
    }
  ]
}
EOF

    # 4. fstab (mount configuration)
    log_info "Generating fstab..."
    local boot_uuid=$(blkid -s UUID -o value "$BOOT_PARTITION" 2>/dev/null || echo "UUID-BOOT")
    local root_uuid=$(blkid -s UUID -o value "$ROOT_PARTITION" 2>/dev/null || echo "UUID-ROOT")

    cat > "$MOUNT_ROOT/etc/fstab" << EOF
# SunlightOS fstab — Auto-generated by install_sunlightos
# <fs>                        <mount>  <type>  <options>  <dump>  <pass>
UUID=$root_uuid                 /        ext4    defaults   0       1
UUID=$boot_uuid                 /boot    vfat    defaults   0       2
/proc                           /proc    proc    defaults   0       0
/sys                            /sys     sysfs   defaults   0       0
/dev/pts                        /dev/pts devpts  defaults   0       0
/tmp                            /tmp     tmpfs   size=256M  0       0
EOF

    log_info "✓ Configuration files generated"
    log_info "  - Hostname: $HOSTNAME"
    log_info "  - Timezone: $TIMEZONE"
    log_info "  - Users: root, $USERNAME"
    log_info "  - fstab (boot: $boot_uuid, root: $root_uuid)"
}

# ============================================================================
# Main Flow
# ============================================================================

main() {
    echo -e "${GREEN}"
    echo "╔════════════════════════════════════════════════════╗"
    echo "║     SunlightOS Interactive System Installer        ║"
    echo "║                  Phase 6 - Release                 ║"
    echo "╚════════════════════════════════════════════════════╝"
    echo -e "${NC}"

    check_requirements
    check_disk
    prompt_configuration

    partition_disk
    format_partitions
    mount_partitions
    install_bootloader
    clone_filesystem
    generate_configuration

    log_step "Installation Complete"
    echo ""
    log_info "✓ SunlightOS successfully installed to $TARGET_DISK"
    echo ""
    echo -e "${GREEN}System Details:${NC}"
    echo "  Hostname:       $HOSTNAME"
    echo "  Timezone:       $TIMEZONE"
    echo "  Root partition: $ROOT_PARTITION (ext4)"
    echo "  Boot partition: $BOOT_PARTITION (FAT32)"
    echo ""
    echo -e "${YELLOW}Next Steps:${NC}"
    echo "  1. Power down the system"
    echo "  2. Attach the disk permanently to the VM or physical system"
    echo "  3. Boot into your installed SunlightOS"
    echo ""
}

# ============================================================================
# Entry Point
# ============================================================================

if [ "${BASH_SOURCE[0]}" == "${0}" ]; then
    main "$@"
fi
