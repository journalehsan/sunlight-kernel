#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
env RUSTFLAGS="-C link-arg=-T$root/services/user-space.ld" \
    cargo build --manifest-path tools/native-std-probe/Cargo.toml \
    --target tools/targets/x86_64-sunlightos.json --bin native-fp-probe \
    --release -Z build-std=core,alloc -Z json-target-spec
env RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static' \
    cargo build -p sunlightd --release --no-default-features
env RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static' \
    cargo build -p sunshell --release --no-default-features --features sunlight
env SUNLIGHT_INJECT_PHASE=native-std cargo build -p sunlight-kernel \
    --bin sunlight-kernel --no-default-features --features key_inject,native_std_probe
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    target/native-std-foundation.iso target/limine "$root"
python3 tools/native_std_boot_gate.py
