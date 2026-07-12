#!/usr/bin/env bash
# Write SunlightOS to a USB flash drive for bare-metal boot (e.g. ThinkPad T440p)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE="$PROJECT_ROOT/target/sunlightos.iso"
TARGET_DISK=""
CONFIRM=0
YES=0
DO_BUILD=1

usage() {
    echo "Usage:"
    echo "  ./tools/write.sh --disk /dev/sdX --i-understand-this-will-erase-data [--yes] [--no-build]"
    echo
    echo "Options:"
    echo "  --disk /dev/sdX     Target block device (whole disk, e.g. /dev/sdb)"
    echo "  --i-understand-this-will-erase-data"
    echo "                      Required safety acknowledgment"
    echo "  --yes               Skip the interactive 'YES' confirmation prompt"
    echo "  --no-build          Do not rebuild; use existing image at $IMAGE"
    echo
    echo "Example:"
    echo "  ./tools/write.sh --disk /dev/sdb --i-understand-this-will-erase-data"
    echo "  ./tools/write.sh --disk /dev/sdb --i-understand-this-will-erase-data --yes"
    echo
    echo "This will destroy all data on the target disk and make it bootable."
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "error: required command not found: $1"
        exit 1
    }
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --disk)
                TARGET_DISK="$2"
                shift 2
                ;;
            --i-understand-this-will-erase-data)
                CONFIRM=1
                shift
                ;;
            --yes)
                YES=1
                shift
                ;;
            --no-build)
                DO_BUILD=0
                shift
                ;;
            *)
                usage
                ;;
        esac
    done

    if [[ -z "$TARGET_DISK" ]]; then
        echo "error: --disk required"
        usage
    fi

    if [[ "$CONFIRM" -ne 1 ]]; then
        echo "error: missing --i-understand-this-will-erase-data"
        exit 1
    fi
}

check_block_device() {
    if [[ ! -b "$TARGET_DISK" ]]; then
        echo "error: not a block device: $TARGET_DISK"
        exit 1
    fi

    TYPE=$(lsblk -dn -o TYPE "$TARGET_DISK")
    if [[ "$TYPE" != "disk" ]]; then
        echo "error: target must be whole disk, not partition"
        lsblk "$TARGET_DISK"
        exit 1
    fi
}

check_root_disk() {
    ROOT_SRC=$(findmnt -no SOURCE / || true)
    # Strip btrfs subvolume suffix, e.g. /dev/sda2[/@] -> /dev/sda2
    ROOT_SRC="${ROOT_SRC%%\[*}"

    if [[ -n "$ROOT_SRC" && -b "$ROOT_SRC" ]]; then
        TARGET_NAME=$(basename "$TARGET_DISK")

        # Walk the full device tree underneath root (-s = inverse/parents) so
        # LVM/LUKS/mapper/btrfs-multi roots resolve down to their physical
        # disks, not just the immediate parent (PKNAME misses those).
        while read -r dev; do
            [[ -z "$dev" ]] && continue
            if [[ "$dev" == "$TARGET_NAME" ]]; then
                echo "error: refusing to write to disk backing root: $TARGET_DISK"
                exit 1
            fi
        done < <(lsblk -rnso NAME "$ROOT_SRC" 2>/dev/null)
    fi
}

check_mounted() {
    # MOUNTPOINT (singular) exists on old and new util-linux; MOUNTPOINTS does not
    if lsblk -nr -o MOUNTPOINT "$TARGET_DISK" | grep -q .; then
        echo "error: disk appears mounted"
        lsblk "$TARGET_DISK"
        exit 1
    fi
}

print_disk_info() {
    echo
    echo "Target disk:"
    lsblk -o NAME,SIZE,MODEL,SERIAL,TRAN,FSTYPE,MOUNTPOINT "$TARGET_DISK"
    echo
}

confirm_prompt() {
    if [[ "$YES" -eq 1 ]]; then
        return
    fi

    echo "DANGER: all data on $TARGET_DISK will be erased."
    echo "Type YES to continue:"
    read -r answer

    if [[ "$answer" != "YES" ]]; then
        echo "aborted."
        exit 1
    fi
}

build_image() {
    if [[ "$DO_BUILD" -eq 0 ]]; then
        echo "Skipping build (--no-build)."
        return
    fi

    echo "Building SunlightOS image..."

    # NOTE: unlike build.sh/run.sh we deliberately do NOT run version_manager.py
    # here — flashing a USB should not mutate the source tree. The embedded
    # version is whatever the last real build set. Run ./tools/build.sh first
    # if you want a fresh version stamp.

    SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C no-redzone"
    TLS_RUSTFLAGS="$SERVICE_RUSTFLAGS --cfg aes_force_soft --cfg polyval_force_soft --cfg poly1305_force_soft --cfg chacha20_force_soft --cfg curve25519_dalek_backend=\"serial\""

    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kbd --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-mouse --release
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
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kv --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kvctl --features sunlightos --no-default-features --release
    RUSTFLAGS="$TLS_RUSTFLAGS" cargo build --package sunlight-tls --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package certificatectl --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --features sunlight --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-utils --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-utils --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-top --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-fetch --features sunlightos --no-default-features --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-display --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package eyes --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-runner --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sun-exec --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-terminal --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-chronos --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tasks --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calculator --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-writer --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-reminders --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calendar --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package rappid-rabbit --features dom --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-api-lab --release
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-emoji-picker --release

    # sunshell must use the kernel target (user-space VA range)
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --release --features sunlight --no-default-features --target x86_64-unknown-none

    # The kernel embeds the service binaries built above via include_bytes!, but
    # there is no kernel/build.rs emitting rerun-if-changed for them — so cargo
    # can skip the kernel rebuild and embed STALE services. Force a fresh compile
    # so a flashed USB always reflects the services we just built.
    touch "$PROJECT_ROOT/kernel/src/main.rs"
    cargo build --package sunlight-kernel

    # Ensure Limine is available
    LIMINE_DIR="$PROJECT_ROOT/target/limine"
    if [[ ! -d "$LIMINE_DIR" ]]; then
        echo "Downloading and building Limine bootloader..."
        git clone --branch="v8.x" --depth=1 https://github.com/limine-bootloader/limine.git "$LIMINE_DIR"
        pushd "$LIMINE_DIR" >/dev/null
        ./bootstrap
        ./configure --enable-uefi-x86-64 --enable-bios-cd --enable-bios-pxe
        make -j"$(nproc)"
        popd >/dev/null
    fi

    # Build the hybrid ISO (Limine BIOS + UEFI)
    KERNEL_ELF="$PROJECT_ROOT/target/x86_64-unknown-none/debug/sunlight-kernel"
    ISO_ROOT="$PROJECT_ROOT/target/iso_root"
    rm -rf "$ISO_ROOT"
    mkdir -p "$ISO_ROOT/boot/limine"

    cp "$KERNEL_ELF" "$ISO_ROOT/boot/sunlight-kernel.elf"
    cp "$PROJECT_ROOT/limine.conf" "$ISO_ROOT/boot/limine/"
    cp "$LIMINE_DIR/bin/limine-bios.sys"    "$ISO_ROOT/boot/limine/"
    cp "$LIMINE_DIR/bin/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
    cp "$LIMINE_DIR/bin/BOOTX64.EFI"        "$ISO_ROOT/boot/limine/"

    echo "Creating ISO..."
    xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
        -no-emul-boot -boot-load-size 4 -boot-info-table \
        --efi-boot boot/limine/BOOTX64.EFI \
        -efi-boot-part --efi-boot-image --protective-msdos-label \
        "$ISO_ROOT" -o "$IMAGE"

    # Embed Limine BIOS boot sector into the ISO
    "$LIMINE_DIR/bin/limine" bios-install "$IMAGE"

    echo "Build complete: $IMAGE"
}

resolve_limine() {
    # Prefer the locally built Limine, fall back to PATH
    local local_limine="$PROJECT_ROOT/target/limine/bin/limine"
    if [[ -x "$local_limine" ]]; then
        echo "$local_limine"
    elif command -v limine >/dev/null 2>&1; then
        echo "limine"
    else
        echo "error: limine not found (neither in target/limine nor in PATH)" >&2
        exit 1
    fi
}

install_limine() {
    # build_image() already runs `limine bios-install` on the freshly built ISO,
    # so when we just built there is nothing to do here. With --no-build the ISO
    # is already a bootable hybrid image, and the limine binary may not be
    # present (e.g. target/ was cleaned) — re-running it is both unnecessary and
    # a hard failure via resolve_limine, so skip it.
    if [[ "$DO_BUILD" -eq 0 ]]; then
        echo "Skipping Limine install (--no-build); ISO is already bootable."
        return
    fi

    local LIMINE_BIN
    LIMINE_BIN=$(resolve_limine) || return 0

    echo "Re-installing Limine BIOS bootloader into image (idempotent)..."
    "$LIMINE_BIN" bios-install "$IMAGE" || true
}

write_image() {
    # Refuse to write an image bigger than the target device (avoids a
    # half-written, unbootable USB). lsblk -b reports bytes without needing root.
    local img_size disk_size
    img_size=$(stat -c %s "$IMAGE" 2>/dev/null || echo 0)
    disk_size=$(lsblk -bdno SIZE "$TARGET_DISK" 2>/dev/null || echo 0)
    if [[ "$img_size" -gt 0 && "$disk_size" -gt 0 && "$img_size" -gt "$disk_size" ]]; then
        echo "error: image ($img_size bytes) is larger than $TARGET_DISK ($disk_size bytes)"
        exit 1
    fi

    echo
    echo "Writing image to $TARGET_DISK"
    echo "Command:"
    echo "sudo dd if=\"$IMAGE\" of=\"$TARGET_DISK\" bs=4M status=progress conv=fsync"
    echo

    sudo dd if="$IMAGE" of="$TARGET_DISK" bs=4M status=progress conv=fsync

    sync
    sudo blockdev --rereadpt "$TARGET_DISK" || true
    sudo udevadm settle || true
}

success_message() {
    echo
    echo "✅ SunlightOS written successfully to $TARGET_DISK"
    echo
    echo "Boot tips for ThinkPad T440p:"
    echo " - Enter BIOS setup with F1 during boot"
    echo " - Disable Secure Boot (Security tab)"
    echo " - Use F12 boot menu to select the USB"
    echo " - Try both UEFI and Legacy/CSM modes if it does not boot"
    echo " - If using Legacy, ensure 'UEFI/Legacy Boot' is set to Both or Legacy First"
    echo
    echo "After first boot you can remove the USB."
}

main() {
    require_cmd lsblk
    require_cmd findmnt
    require_cmd dd

    parse_args "$@"

    if [[ "$DO_BUILD" -eq 1 ]]; then
        require_cmd cargo
        require_cmd xorriso
    fi

    check_block_device
    check_root_disk
    check_mounted
    print_disk_info
    confirm_prompt

    build_image

    if [[ ! -f "$IMAGE" ]]; then
        echo "error: image not found after build: $IMAGE"
        exit 1
    fi

    install_limine
    write_image

    success_message
}

main "$@"
