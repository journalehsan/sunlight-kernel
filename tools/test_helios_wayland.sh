#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
out="$PWD/target/helios-wayland"
mkdir -p "$out"
flags=(-Wall -Wextra -Werror -Os -static -nostdlib -fno-pie -no-pie -ffreestanding -fno-stack-protector -Wl,--build-id=none)
"${CC:-cc}" "${flags[@]}" tools/tests/helios-wayland-client.c -o "$out/client.elf"
bash tools/stamp_helios_elf.sh "$out/client.elf"
service_flags='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
RUSTFLAGS="$service_flags" cargo build -p helios-wayland-fixture -p sunlight-display \
    --release --features sunlight-display/helios_wayland_test >"$out/service-build.log" 2>&1
SUNLIGHT_WAYLAND_SERVER="$PWD/target/x86_64-unknown-none/release/helios-wayland-fixture" \
SUNLIGHT_WAYLAND_CLIENT="$out/client.elf" \
    cargo build -p sunlight-kernel --features helios_wayland_test >"$out/kernel-build.log" 2>&1
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    "$out/guest.iso" >"$out/pack.log" 2>&1

status=0
for accel in "${@:-tcg}"; do
    python3 tools/helios_wayland_gate.py "$out/guest.iso" "$out/$accel" "$accel" || status=1
done
exit "$status"
