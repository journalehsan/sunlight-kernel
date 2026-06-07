#!/usr/bin/env bash
set -euo pipefail

# --- Configuration ---
QEMU_MEMORY="256M"
QEMU_CPUS="2"
KERNEL_ELF="target/x86_64-unknown-none/debug/sunlight-kernel"
ISO_PATH="target/sunlightos.iso"
LIMINE_BRANCH="v8.x"
LIMINE_DIR="target/limine"
SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static"

# --- Step 1: Build service binaries first (embedded via include_bytes!) ---
echo "[build] Building user-space services..."
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release

# --- Step 2: Build the kernel ELF ---
echo "[build] Building kernel..."
cargo build --package sunlight-kernel

# --- Step 3: Download Limine if not cached ---
if [[ ! -d "$LIMINE_DIR" ]]; then
    echo "[build] Downloading Limine..."
    git clone --branch="$LIMINE_BRANCH" --depth=1 https://github.com/limine-bootloader/limine.git "$LIMINE_DIR"
    pushd "$LIMINE_DIR" >/dev/null
    ./bootstrap
    ./configure --enable-uefi-x86-64 --enable-bios-cd --enable-bios-pxe
    make -j"$(nproc)"
    popd >/dev/null
else
    echo "[build] Limine already cached."
fi

# --- Step 4: Create ISO layout ---
ISO_ROOT="target/iso_root"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine"
mkdir -p "$ISO_ROOT/boot"

cp "$KERNEL_ELF" "$ISO_ROOT/boot/sunlight-kernel.elf"
cp limine.conf "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios.sys" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/BOOTX64.EFI" "$ISO_ROOT/boot/limine/"

# --- Step 5: Build ISO with xorriso ---
echo "[build] Building ISO..."
xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --efi-boot boot/limine/BOOTX64.EFI \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$ISO_PATH"

# --- Step 6: Install Limine bootloader into ISO ---
"$LIMINE_DIR/bin/limine" bios-install "$ISO_PATH"

# --- Step 7: Launch QEMU ---
echo "[build] Launching QEMU..."

KVM_FLAGS=""
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    KVM_FLAGS="-enable-kvm"
    echo "[build] KVM acceleration enabled"
else
    echo "[build] KVM not available, falling back to TCG"
fi

qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial stdio \
    -display none \
    -m "$QEMU_MEMORY" \
    -smp "$QEMU_CPUS" \
    $KVM_FLAGS \
    -no-reboot \
    -no-shutdown
