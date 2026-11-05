#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
env SUNLIGHT_INJECT_PHASE=desktop_login cargo build -p sunlight-kernel \
    --bin sunlight-kernel --no-default-features --features key_inject
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    target/svg-icons.iso target/limine "$root"
python3 tools/svg_icons_boot_gate.py
