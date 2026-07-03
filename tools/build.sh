#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Step 0: Auto-version all workspace crates ---
echo "[build] Updating version..."
python3 "$SCRIPT_DIR/version_manager.py" "$PROJECT_ROOT"

# --- Configuration ---
QEMU_MEMORY="256M"
QEMU_CPUS="2"
KERNEL_ELF="target/x86_64-unknown-none/debug/sunlight-kernel"
ISO_PATH="target/sunlightos.iso"
LIMINE_BRANCH="v8.x"
LIMINE_DIR="target/limine"

# Kernel flags: conservative, no SIMD assumptions (x86-64 baseline only).
# The kernel target (x86_64-unknown-none) already disables SSE/AVX via soft-float.
KERNEL_RUSTFLAGS="-C link-arg=-Tkernel/src/arch/x86_64/linker.ld -C relocation-model=static"

# Userspace baseline: x86-64-v2 (SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B).
# v2 enables better code generation for userspace services without requiring AVX.
# v3 (AVX/AVX2) is runtime-only for selected apps until kernel adds XSAVE/YMM switching.
SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone"

# sunlight-tls links a pure-Rust rustls stack. x86_64-unknown-none disables SSE,
# but x86-64-v2 re-enables SSE/SSE2. The RustCrypto crates still need soft backend
# forcing because their auto-detection triggers AVX2/AES-NI backends that require
# runtime feature checks or OS state the kernel doesn't provide yet (XSAVE/YMM).
# See the Phase-0 TLS build recipe.
# Phase-0 TLS build recipe.
TLS_RUSTFLAGS="$SERVICE_RUSTFLAGS --cfg aes_force_soft --cfg polyval_force_soft --cfg poly1305_force_soft --cfg chacha20_force_soft --cfg curve25519_dalek_backend=\"serial\""

# --- Step 1: Build service binaries first (embedded via include_bytes!) ---
echo "[build] Building user-space services..."
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
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightd --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-niced --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-gcd --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightctl --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-uac --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sm --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-solar --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kv --features sunlightos --no-default-features --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kvctl --features sunlightos --no-default-features --release
RUSTFLAGS="$TLS_RUSTFLAGS" cargo build --package sunlight-tls --features sunlightos --no-default-features --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package certificatectl --features sunlightos --no-default-features --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --features sunlight --no-default-features --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-utils --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-utils --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-top --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-fetch --features sunlightos --no-default-features --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sunsay --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-zoxide --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-dict --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-hangman --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package cpu-utils --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-display --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package eyes --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-runner --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sun-exec --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-terminal --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tasks --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-bench --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calculator --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-files --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-control-panel --release
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-thumbd --release
# Force a non-PIE static link so e_type is ET_EXEC (not ET_DYN). The kernel ELF
# loader (sunlight-elf parse_elf_header) only accepts ET_EXEC; -no-pie + crt-static
# is required because musl otherwise emits a static-PIE that loads as DYN.
RUSTFLAGS="-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie" cargo build --package helios-note --release --target x86_64-unknown-linux-musl
# Patch EI_OSABI (byte 7) to ELFOSABI_LINUX (3) so the kernel's is_linux_elf()
# recognizes this musl binary as a Linux-compat process. Rust/musl outputs
# ELFOSABI_NONE (0) by default; we stamp it to 3 post-link, matching the
# treatment applied to hello-linux.elf.
printf '\x03' | dd of="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release/helios-note" \
    bs=1 seek=7 conv=notrunc 2>/dev/null

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
    -netdev user,id=net0,hostfwd=tcp::8080-:80 -device virtio-net-pci,netdev=net0,disable-modern=on \
    -no-reboot \
    -no-shutdown
