#!/usr/bin/env bash
set -euo pipefail

# --- Configuration ---
TIMEOUT=60
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
        NEED_DISK=false
        ;;
    phase3.0)
        EXPECTED_FILE="tools/tests/phase3_0.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.0 OK"
        PASS_LABEL="Phase 3.0"
        NEED_DISK=false
        ;;
    phase3.5)
        EXPECTED_FILE="tools/tests/phase3_5.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.5 OK"
        PASS_LABEL="Phase 3.5"
        NEED_DISK=true
        ;;
    phase3.6)
        EXPECTED_FILE="tools/tests/phase3_6.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.6 OK"
        PASS_LABEL="Phase 3.6"
        NEED_DISK=false
        ;;
    sunlightd)
        EXPECTED_FILE="tools/tests/sunlightd.expected"
        FINAL_MARKER="[SunlightOS] sunlightd OK"
        PASS_LABEL="sunlightd"
        NEED_DISK=false
        ;;
    phase3.7)
        EXPECTED_FILE="tools/tests/phase3_7.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.7 OK"
        PASS_LABEL="Phase 3.7"
        NEED_DISK=false
        ;;
    phase3.8)
        EXPECTED_FILE="tools/tests/phase3_8.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.8 OK"
        PASS_LABEL="Phase 3.8"
        NEED_DISK=false
        ;;
    phase3.9)
        EXPECTED_FILE="tools/tests/phase3_9.expected"
        FINAL_MARKER="[TTY]  hostnamectl invoked"
        PASS_LABEL="Phase 3.9"
        NEED_DISK=false
        ;;
    phase4.5)
        EXPECTED_FILE="tools/tests/phase4_5.expected"
        FINAL_MARKER="[SunlightOS] Phase 4.5 OK"
        PASS_LABEL="Phase 4.5"
        NEED_DISK=false
        ;;
    phase5.0)
        EXPECTED_FILE="tools/tests/phase5_0.expected"
        FINAL_MARKER="[NET]  virtio-net OK"
        PASS_LABEL="Phase 5.0"
        NEED_DISK=false
        ;;
    phase5.1)
        EXPECTED_FILE="tools/tests/phase5_1.expected"
        FINAL_MARKER="[NET]  Interface: eth0 MAC="
        PASS_LABEL="Phase 5.1"
        NEED_DISK=false
        ;;
    phase5.2)
        EXPECTED_FILE="tools/tests/phase5_2.expected"
        FINAL_MARKER="[DHCP] OK"
        PASS_LABEL="Phase 5.2"
        NEED_DISK=false
        ;;
    phase5.3)
        EXPECTED_FILE="tools/tests/phase5_3.expected"
        FINAL_MARKER="[NET]  NetOp handlers registered"
        PASS_LABEL="Phase 5.3"
        NEED_DISK=false
        ;;
    phase5.4)
        EXPECTED_FILE="tools/tests/phase5_4.expected"
        FINAL_MARKER="[NET]  Linux process socket syscalls ready"
        PASS_LABEL="Phase 5.4"
        NEED_DISK=false
        ;;
    phase5.5)
        EXPECTED_FILE="tools/tests/phase5_5.expected"
        FINAL_MARKER="[TLS]  Handshake OK: google.com"
        PASS_LABEL="Phase 5.5"
        NEED_DISK=false
        ;;
    phase5.6)
        EXPECTED_FILE="tools/tests/phase5_6.expected"
        FINAL_MARKER="[BTRFS] Mounted /data read-only"
        PASS_LABEL="Phase 5.6"
        NEED_DISK=true
        ;;
    phase5.7)
        EXPECTED_FILE="tools/tests/phase5_7.expected"
        FINAL_MARKER="[SunlightOS] Phase 5 OK"
        PASS_LABEL="Phase 5.7"
        NEED_DISK=true
        ;;
    phase5x.0)
        EXPECTED_FILE="tools/tests/phase5x_0.expected"
        FINAL_MARKER="[DHCP] OK"
        PASS_LABEL="Phase 5.x.0"
        NEED_DISK=false
        ;;
    phase5x.1)
        EXPECTED_FILE="tools/tests/phase5x_1.expected"
        FINAL_MARKER="[DNS]  OK"
        PASS_LABEL="Phase 5.x.1"
        NEED_DISK=false
        ;;
    phase5x.2)
        EXPECTED_FILE="tools/tests/phase5x_2.expected"
        FINAL_MARKER="[TCP]  OK"
        PASS_LABEL="Phase 5.x.2"
        NEED_DISK=false
        ;;
    phase5x.3)
        EXPECTED_FILE="tools/tests/phase5x_3.expected"
        FINAL_MARKER="[M3]   ping 8.8.8.8: SUCCESS"
        PASS_LABEL="Phase 5.x.3"
        NEED_DISK=false
        ;;
    phase5x.4)
        EXPECTED_FILE="tools/tests/phase5x_4.expected"
        FINAL_MARKER="[TLS]  Handshake OK"
        PASS_LABEL="Phase 5.x.4"
        NEED_DISK=false
        ;;
    phase5x.5)
        EXPECTED_FILE="tools/tests/phase5x_5.expected"
        FINAL_MARKER="[UTIL] OK"
        PASS_LABEL="Phase 5.x.5"
        NEED_DISK=false
        ;;
    phase5x.6)
        EXPECTED_FILE="tools/tests/phase5x_6.expected"
        FINAL_MARKER="[NET]  OK"
        PASS_LABEL="Phase 5.x.6"
        NEED_DISK=false
        ;;
    dns_hosts)
        EXPECTED_FILE="tools/tests/dns_hosts.expected"
        FINAL_MARKER="[DNS] /etc/hosts loaded (hosts + hardcoded resolver active)"
        PASS_LABEL="dns_hosts"
        NEED_DISK=false
        ;;
    phase6.5.1)
        EXPECTED_FILE="tools/tests/phase6_5_1.expected"
        FINAL_MARKER="[TTY]  sysfetch invoked"
        PASS_LABEL="Phase 6.5.1"
        NEED_DISK=false
        ;;
    phase6.5.3)
        EXPECTED_FILE="tools/tests/phase6_5_3.expected"
        FINAL_MARKER="[EXEC] ls exit=0"
        PASS_LABEL="Phase 6.5.3"
        NEED_DISK=true
        ;;
    phase_shm)
        EXPECTED_FILE="tools/tests/phase_shm.expected"
        FINAL_MARKER="[SHM]  Shared memory grant: PASSED"
        PASS_LABEL="Shared Memory Grant"
        NEED_DISK=false
        ;;
    phase_sec)
        EXPECTED_FILE="tools/tests/phase_sec.expected"
        FINAL_MARKER="[SEC]  Security hardening: PASSED"
        PASS_LABEL="Security Hardening"
        NEED_DISK=false
        ;;
    *)
        echo "[test] Unsupported gate '$PHASE'. Supported: phase2.6 phase3.0 phase3.5 phase3.6 phase3.7 phase3.8 phase3.9 phase4.5 phase5.0 phase5.1 phase5.2 phase5.3 phase5.4 phase5.5 phase5.6 phase5.7 phase5x.0 phase5x.1 phase5x.2 phase5x.3 phase5x.4 phase5x.5 phase5x.6 dns_hosts phase6.5.1 phase6.5.3 phase_shm phase_sec sunlightd"
        exit 2
        ;;
esac

mapfile -t EXPECTED < <(grep -Ev '^[[:space:]]*($|#)' "$EXPECTED_FILE")

# --- Step 1: Build service binaries first ---
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release >"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package timezone_service --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightctl --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --features sunlight --no-default-features --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-utils --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-utils --release >>"$BUILD_LOG" 2>&1

# --- Step 1b: Create FAT32 disk image (phase3.5+) ---
if [[ "$NEED_DISK" == "true" ]]; then
    bash tools/disk.sh >>"$BUILD_LOG" 2>&1
fi

# --- Step 2: Build kernel ---
KERNEL_FEATURES=""
if [[ "$PHASE" == "phase3.6" || "$PHASE" == "phase3.7" || "$PHASE" == "phase3.8" || "$PHASE" == "phase3.9" || "$PHASE" == "phase6.5.1" || "$PHASE" == "phase6.5.3" ]]; then
    KERNEL_FEATURES="--features key_inject"
fi
EXTRA_ENV=()
if [[ "$PHASE" == "phase3.9" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase3.9)
elif [[ "$PHASE" == "phase6.5.1" ]]; then
    # Reuse the phase3.9 key sequence — it logs in and types sysfetch
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase3.9)
elif [[ "$PHASE" == "phase6.5.3" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase6.5.3)
elif [[ "$PHASE" == "phase4.5" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase4.5)
elif [[ "$PHASE" == phase5* || "$PHASE" == phase5x* || "$PHASE" == "dns_hosts" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE="$PHASE")
fi
env "${EXTRA_ENV[@]}" cargo build --package sunlight-kernel $KERNEL_FEATURES >>"$BUILD_LOG" 2>&1

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

# Extra QEMU flags for phases that need a virtio-blk disk
DISK_FLAGS=""
if [[ "$NEED_DISK" == "true" && -f "target/test.img" ]]; then
    DISK_FLAGS="-drive id=hd0,file=target/test.img,if=none,format=raw -device virtio-blk-pci,disable-modern=on,drive=hd0"
fi

# Extra QEMU flags for Phase 5 networking (virtio-net). Always add for phase5* so PCI scan + driver init succeed.
NET_FLAGS=""
if [[ "$PHASE" == phase5* || "$PHASE" == phase5x* ]]; then
    NET_FLAGS="-netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-modern=on"
fi

set +e
qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial file:"$QEMU_OUTPUT" \
    -display none \
    -m 256M \
    -smp 2 \
    $KVM_FLAGS \
    $DISK_FLAGS \
    $NET_FLAGS \
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
