#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
bash tools/build_sunjs.sh
# Separate invocations avoid unifying sunlight-libc allocator features.
for package in sunlightd sunlight-tty-server sunlight-kbd; do
    env RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
        cargo build -p "$package" --release --no-default-features
done
env RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
    cargo build -p sunshell --release --no-default-features --features sunlight
cargo build -p sunlight-kernel --bin sunlight-kernel \
    --no-default-features --features sunjs_probe
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    target/sunjs-test.iso target/limine "$root"
python3 tools/test_sunjs_guest.py
