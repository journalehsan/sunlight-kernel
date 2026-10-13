#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."
# Default image: viewer present, document renderer disabled.
env SUNLIGHT_INJECT_PHASE=desktop_login \
    cargo build -p sunlight-kernel --no-default-features --features key_inject
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    target/docview-launch-test.iso target/limine "$PWD"
python3 tools/docview_launch_gate.py
