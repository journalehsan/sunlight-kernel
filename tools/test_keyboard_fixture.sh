#!/usr/bin/env bash
# Isolated optical image; no disk/loop devices or privileged operations.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/qemu-keyboard-smoke
service_flags='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
for package in sunlight-kbd sunlight-tty-server sunlight-display; do
    RUSTFLAGS="$service_flags" cargo build -p "$package" --release --no-default-features
done
SUNLIGHT_ACCOUNT_FIXTURE=1 SUNLIGHT_INJECT_PHASE=desktop_login \
    cargo build -p sunlight-kernel --no-default-features --features key_inject
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    target/qemu-keyboard-smoke/account-fixture.iso target/limine "$PWD"
python3 tools/keyboard_fixture_gate.py
