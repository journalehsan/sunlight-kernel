#!/usr/bin/env bash
set -euo pipefail

# --- Configuration ---
TIMEOUT=30
KERNEL_ELF="target/x86_64-unknown-none/debug/sunlight-kernel"
ISO_PATH="target/sunlightos.iso"
LIMINE_BRANCH="v8.x"
LIMINE_DIR="target/limine"

EXPECTED=(
    "[SunlightOS] Kernel booting..."
    "[SunlightOS] Phase 0 OK"
)

# --- Step 1: Build kernel ---
echo "[test] Building kernel..."
cargo build --package sunlight-kernel

# --- Step 2: Ensure Limine is available ---
if [[ ! -d "$LIMINE_DIR" ]]; then
    echo "[test] Downloading Limine..."
    git clone --branch="$LIMINE_BRANCH" --depth=1 https://github.com/limine-bootloader/limine.git "$LIMINE_DIR"
    pushd "$LIMINE_DIR" >/dev/null
    ./bootstrap
    ./configure --enable-uefi-x86-64 --enable-bios-cd --enable-bios-pxe
    make -j"$(nproc)"
    popd >/dev/null
else
    echo "[test] Limine already cached."
fi

# --- Step 3: Create ISO layout ---
ISO_ROOT="target/iso_root"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine"
mkdir -p "$ISO_ROOT/boot"

cp "$KERNEL_ELF" "$ISO_ROOT/boot/sunlight-kernel.elf"
cp limine.cfg "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios.sys" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/BOOTX64.EFI" "$ISO_ROOT/boot/limine/"

# --- Step 4: Build ISO with xorriso ---
echo "[test] Building ISO..."
xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --efi-boot boot/limine/BOOTX64.EFI \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$ISO_PATH"

"$LIMINE_DIR/bin/limine" bios-install "$ISO_PATH"

# --- Step 5: Launch QEMU with timeout ---
echo "[test] Running QEMU boot test (timeout: ${TIMEOUT}s)..."

KVM_FLAGS=""
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    KVM_FLAGS="-enable-kvm"
    echo "[test] KVM acceleration enabled"
else
    echo "[test] KVM not available, using TCG"
fi

QEMU_OUTPUT=$(mktemp)
trap "rm -f $QEMU_OUTPUT" EXIT

set +e
qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial file:"$QEMU_OUTPUT" \
    -display none \
    -m 256M \
    -smp 2 \
    $KVM_FLAGS \
    -no-reboot \
    -no-shutdown &
QEMU_PID=$!

# Wait up to TIMEOUT seconds, checking if QEMU is still running
for ((i=0; i<TIMEOUT; i++)); do
    if ! kill -0 $QEMU_PID 2>/dev/null; then
        break
    fi
    # Check if expected output is already present (early exit on success)
    if grep -Fq "Phase 0 OK" "$QEMU_OUTPUT" 2>/dev/null; then
        sleep 1
        break
    fi
    sleep 1
done

# If still running, kill it
if kill -0 $QEMU_PID 2>/dev/null; then
    kill -TERM $QEMU_PID 2>/dev/null || true
    sleep 1
    kill -KILL $QEMU_PID 2>/dev/null || true
fi

wait $QEMU_PID 2>/dev/null
QEMU_EXIT=$?
set -e

# --- Step 6: Analyze output ---
echo ""
echo "[test] --- QEMU serial output ---"
cat "$QEMU_OUTPUT"
echo "[test] --------------------------"
echo ""

ALL_FOUND=true
for expected in "${EXPECTED[@]}"; do
    if grep -Fq "$expected" "$QEMU_OUTPUT"; then
        echo "[test] ✓ Found: $expected"
    else
        echo "[test] ✗ Missing: $expected"
        ALL_FOUND=false
    fi
done

if [[ "$ALL_FOUND" == true ]]; then
    echo "[test] ✓ Phase 0 gate PASSED"
    exit 0
else
    echo "[test] ✗ Phase 0 gate FAILED"
    exit 1
fi
