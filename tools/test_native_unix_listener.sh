#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
out="$PWD/target/native-unix-listener"
mkdir -p "$out"
source_file=tools/tests/native-unix-listener.c
flags=(-Wall -Wextra -Werror -Os -static -nostdlib -fno-pie -no-pie -ffreestanding -fno-stack-protector -Wl,--build-id=none)
"${CC:-cc}" "${flags[@]}" -DNATIVE "$source_file" -o "$out/server.elf"
"${CC:-cc}" "${flags[@]}" "$source_file" -o "$out/client.elf"
bash tools/stamp_helios_elf.sh "$out/client.elf"

SUNLIGHT_NATIVE_UNIX_SERVER="$out/server.elf" SUNLIGHT_NATIVE_UNIX_CLIENT="$out/client.elf" \
    cargo build --package sunlight-kernel --features native_unix_listener_test 2>&1 | tee "$out/build.log"
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    "$out/guest.iso" 2>&1 | tee "$out/pack.log"

serial="$out/serial.log"
: > "$serial"
qemu-system-x86_64 -accel "${SUNLIGHT_TEST_ACCEL:-tcg}" -m 2048 -smp 2 \
    -device virtio-rng-pci,disable-modern=on \
    -cdrom "$out/guest.iso" -display none -monitor none -serial "file:$serial" \
    -no-reboot > "$out/qemu.log" 2>&1 &
guest=$!
trap 'kill "$guest" 2>/dev/null || true; wait "$guest" 2>/dev/null || true' EXIT
markers=(
    'nonroot native registration denied'
    'wrong UID Linux connect returned ECONNREFUSED'
    'denied client left queue empty'
    'post-close Linux connect returned ECONNREFUSED'
    'PASS exchange, close, refusal and rebind'
)
for ((attempt = 0; attempt < 120; ++attempt)); do
    if rg -qF '[UNIX-FIX] FAIL' "$serial" || ! kill -0 "$guest" 2>/dev/null; then break; fi
    found=0
    for marker in "${markers[@]}"; do
        if rg -qF "[UNIX-FIX] $marker" "$serial"; then found=$((found + 1)); fi
    done
    if ((found == ${#markers[@]})); then
        rg -F '[UNIX-FIX]' "$serial"
        exit 0
    fi
    sleep 1
done
rg -F '[UNIX-FIX]' "$serial" || true
echo "Guest fixture failed; see $serial and $out/qemu.log" >&2
exit 1
