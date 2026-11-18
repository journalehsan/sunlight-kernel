#!/usr/bin/env bash
# Requires a normal build's other embedded services. Uses diskless guests only.
# Stop on any build failure so an older embedded binary cannot appear to pass.
set -euo pipefail
cd "$(dirname "$0")/.."
output="target/desktop-session-ux"
mkdir -p "$output"
service_flags='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
RUSTFLAGS="$service_flags" cargo build -p sunlight-kbd -p sunlight-tty-server \
    -p sunlight-display --release --no-default-features
bash tools/build_svg_apps.sh -p sunlight-vortex-shell
cargo build -p sunlight-kernel --no-default-features
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    "$output/production.iso" target/limine "$PWD"
python3 tools/desktop_tty_ux_gate.py "$output/production.iso" --out "$output/desktop"
python3 tools/desktop_tty_ux_gate.py "$output/production.iso" --failure --out "$output/recovery"
python3 tools/desktop_tty_ux_gate.py "$output/production.iso" --tty-first --vga std \
    --out "$output/limine"
