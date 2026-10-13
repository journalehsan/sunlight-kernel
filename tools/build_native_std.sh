#!/usr/bin/env bash
# Build with the pinned native std overlay. Does not modify the rustup sysroot.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
python3 tools/native-std/prepare.py
env -u CARGO_ENCODED_RUSTFLAGS \
    RUSTC="$root/target/native-std/rustc" \
    RUSTFLAGS="-C link-arg=-T$root/services/user-space.ld --check-cfg=cfg(target_os,values(\"sunlightos\"))" \
    cargo "$@" --target "$root/tools/targets/x86_64-sunlightos.json" \
    -Z build-std=std,panic_abort -Z build-std-features= -Z json-target-spec
