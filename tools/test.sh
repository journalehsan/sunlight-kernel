#!/usr/bin/env bash
set -euo pipefail

# --- Configuration ---
TIMEOUT=30
KERNEL_ELF="target/x86_64-unknown-none/debug/sunlight-kernel"
ISO_PATH="target/sunlightos.iso"
LIMINE_BRANCH="v8.x"
LIMINE_DIR="target/limine"
SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static"
BUILD_LOG=$(mktemp)
PHASE="${1:-phase3.0}"

case "$PHASE" in
    phase2.6)
        EXPECTED_FILE="tools/tests/phase2_6.expected"
        FINAL_MARKER="[SunlightOS] Phase 2.6 OK"
        PASS_LABEL="Phase 2.6"
        ;;
    phase3.0)
        EXPECTED_FILE="tools/tests/phase3_0.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.0 OK"
        PASS_LABEL="Phase 3.0"
        ;;
    *)
        echo "[test] Unsupported gate '$PHASE'. Supported: phase2.6 phase3.0"
        exit 2
        ;;
esac

mapfile -t EXPECTED < <(grep -Ev '^[[:space:]]*($|#)' "$EXPECTED_FILE")

# --- Step 1: Build service binaries first ---
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release >"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release >>"$BUILD_LOG" 2>&1

# --- Step 2: Build kernel ---
cargo build --package sunlight-kernel >>"$BUILD_LOG" 2>&1

# --- Step 3: Ensure Limine is available ---
if [[ ! -d "$LIMINE_DIR" ]]; then
    git clone --branch="$LIMINE_BRANCH" --depth=1 https://github.com/limine-bootloader/limine.git "$LIMINE_DIR" >>"$BUILD_LOG" 2>&1
    pushd "$LIMINE_DIR" >>"$BUILD_LOG" 2>&1
    ./bootstrap >>"$BUILD_LOG" 2>&1
    ./configure --enable-uefi-x86-64 --enable-bios-cd --enable-bios-pxe >>"$BUILD_LOG" 2>&1
    make -j"$(nproc)" >>"$BUILD_LOG" 2>&1
    popd >>"$BUILD_LOG" 2>&1
fi

# --- Step 4: Create ISO layout ---
ISO_ROOT="target/iso_root"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine"
mkdir -p "$ISO_ROOT/boot"

cp "$KERNEL_ELF" "$ISO_ROOT/boot/sunlight-kernel.elf"
cp limine.cfg "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios.sys" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/bin/BOOTX64.EFI" "$ISO_ROOT/boot/limine/"

# --- Step 5: Build ISO with xorriso ---
xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --efi-boot boot/limine/BOOTX64.EFI \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$ISO_PATH" >>"$BUILD_LOG" 2>&1

"$LIMINE_DIR/bin/limine" bios-install "$ISO_PATH" >>"$BUILD_LOG" 2>&1

# --- Step 6: Launch QEMU with timeout ---
KVM_FLAGS=""
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    KVM_FLAGS="-enable-kvm"
fi

QEMU_OUTPUT=$(mktemp)
trap "rm -f $QEMU_OUTPUT $BUILD_LOG" EXIT

set +e
qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial file:"$QEMU_OUTPUT" \
    -display none \
    -m 256M \
    -smp 2 \
    $KVM_FLAGS \
    -no-reboot \
    -no-shutdown >>"$BUILD_LOG" 2>&1 &
QEMU_PID=$!

# Wait up to TIMEOUT seconds, checking if QEMU is still running
for ((i=0; i<TIMEOUT; i++)); do
    if ! kill -0 $QEMU_PID 2>/dev/null; then
        break
    fi
    # Check if the final runtime milestone is present (early exit on success).
    if grep -Fq "[timer] 100 ticks elapsed" "$QEMU_OUTPUT" 2>/dev/null \
        && grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT" 2>/dev/null; then
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

ALL_FOUND=true
PMM_LINE=$(grep -E '^\[PMM\] [0-9]+/[0-9]+ MiB free$' "$QEMU_OUTPUT" | head -n1 || true)
if [[ -n "$PMM_LINE" ]]; then
    :
else
    ALL_FOUND=false
fi

for expected in "${EXPECTED[@]}"; do
    if ! grep -Fq "$expected" "$QEMU_OUTPUT"; then
        ALL_FOUND=false
    fi
done

if [[ "$ALL_FOUND" == true ]]; then
    echo "══════════════════════════════════════"
    echo "  SunlightOS — ${PASS_LABEL} Boot Gate"
    echo "══════════════════════════════════════"
    if [[ -n "$PMM_LINE" ]]; then
        echo "$PMM_LINE"
    fi
    for expected in "${EXPECTED[@]}"; do
        echo "$expected"
    done
    echo "══════════════════════════════════════"
    echo "✓ ${PASS_LABEL} gate PASSED"
    exit 0
else
    echo "[test] --- build and tool output ---"
    cat "$BUILD_LOG"
    echo "[test] -----------------------------"
    echo ""
    echo "[test] --- QEMU serial output ---"
    cat "$QEMU_OUTPUT"
    echo "[test] --------------------------"
    echo ""

    if [[ -n "$PMM_LINE" ]]; then
        echo "[test] ✓ Found: [PMM] .../... MiB free"
    else
        echo "[test] ✗ Missing: [PMM] .../... MiB free"
    fi

    for expected in "${EXPECTED[@]}"; do
        if grep -Fq "$expected" "$QEMU_OUTPUT"; then
            echo "[test] ✓ Found: $expected"
        else
            echo "[test] ✗ Missing: $expected"
        fi
    done
    echo "[test] ✗ ${PASS_LABEL} gate FAILED"
    exit 1
fi
